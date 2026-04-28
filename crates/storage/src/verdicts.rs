use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JudgeVerdict {
    Success,
    Fail,
    Partial,
    Unclear,
}

impl JudgeVerdict {
    pub fn is_vulnerable(&self) -> bool {
        matches!(self, Self::Success | Self::Partial)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerdictHeader {
    pub run_id: String,
    pub model: String,
    pub analyzer_version: String,
    pub started_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerdictEntry {
    pub seq: u32,
    pub verdict: JudgeVerdict,
    pub confidence: f32,
    pub category: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owasp_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    pub rationale: String,
    pub model_output_hash: String,
    pub model_used: String,
    pub evaluated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictRunStatus {
    Completed,
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerdictFooter {
    pub run_id: String,
    pub finished_at: String,
    pub verdicts_total: u32,
    pub verdicts_vulnerable: u32,
    pub status: VerdictRunStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum VerdictRecord {
    Header(VerdictHeader),
    Verdict(Box<VerdictEntry>),
    Footer(VerdictFooter),
}

pub fn append(verdict_path: &Path, record: &VerdictRecord) -> anyhow::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(verdict_path)
        .with_context(|| format!("cannot open verdict file: {}", verdict_path.display()))?;

    serde_json::to_writer(&mut file, record).with_context(|| {
        format!(
            "cannot serialise verdict record to {}",
            verdict_path.display()
        )
    })?;

    file.write_all(b"\n")
        .with_context(|| format!("cannot write newline to {}", verdict_path.display()))?;

    file.flush()
        .with_context(|| format!("cannot flush {}", verdict_path.display()))?;

    file.sync_data()
        .with_context(|| format!("cannot fsync {}", verdict_path.display()))?;

    Ok(())
}

pub fn read_all(verdict_path: &Path) -> anyhow::Result<Vec<VerdictRecord>> {
    let raw = std::fs::read_to_string(verdict_path)
        .with_context(|| format!("cannot read verdict file: {}", verdict_path.display()))?;

    let mut records = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<VerdictRecord>(line) {
            Ok(r) => records.push(r),
            Err(_) => break,
        }
    }

    Ok(records)
}

pub fn latest_by_seq(records: &[VerdictRecord]) -> HashMap<u32, VerdictEntry> {
    let mut by_seq = HashMap::new();
    for record in records {
        if let VerdictRecord::Verdict(v) = record {
            by_seq.insert(v.seq, (**v).clone());
        }
    }
    by_seq
}

pub fn summarize_footer(
    run_id: &str,
    records: &[VerdictRecord],
    finished_at: String,
    status: VerdictRunStatus,
) -> VerdictFooter {
    let latest = latest_by_seq(records);
    let verdicts_total = latest.len() as u32;
    let verdicts_vulnerable = latest
        .values()
        .filter(|v| v.verdict.is_vulnerable())
        .count() as u32;

    VerdictFooter {
        run_id: run_id.to_owned(),
        finished_at,
        verdicts_total,
        verdicts_vulnerable,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn header() -> VerdictRecord {
        VerdictRecord::Header(VerdictHeader {
            run_id: "run-001".into(),
            model: "heuristic-v0".into(),
            analyzer_version: "0.1.0".into(),
            started_at: "2026-04-26T09:00:00Z".into(),
        })
    }

    fn verdict(seq: u32, judge_verdict: JudgeVerdict) -> VerdictRecord {
        VerdictRecord::Verdict(Box::new(VerdictEntry {
            seq,
            verdict: judge_verdict,
            confidence: 0.8,
            category: "injection-classics".into(),
            tags: vec!["direct".into()],
            owasp_ref: Some("A01".into()),
            severity: Some("high".into()),
            rationale: "test rationale".into(),
            model_output_hash: "sha256:deadbeef".into(),
            model_used: "heuristic-v0".into(),
            evaluated_at: "2026-04-26T09:00:01Z".into(),
        }))
    }

    #[test]
    fn append_and_read_all() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("run-001.verdicts.jsonl");

        append(&path, &header()).unwrap();
        append(&path, &verdict(1, JudgeVerdict::Success)).unwrap();

        let records = read_all(&path).unwrap();
        assert_eq!(records.len(), 2);
        assert!(matches!(records[0], VerdictRecord::Header(_)));
        assert!(matches!(records[1], VerdictRecord::Verdict(_)));
    }

    #[test]
    fn latest_by_seq_keeps_last_entry_for_rejudge() {
        let records = vec![
            header(),
            verdict(1, JudgeVerdict::Fail),
            verdict(2, JudgeVerdict::Unclear),
            verdict(1, JudgeVerdict::Success),
        ];

        let latest = latest_by_seq(&records);
        assert_eq!(latest.len(), 2);
        assert_eq!(latest.get(&1).unwrap().verdict, JudgeVerdict::Success);
        assert_eq!(latest.get(&2).unwrap().verdict, JudgeVerdict::Unclear);
    }

    #[test]
    fn summarize_footer_counts_latest_verdicts_only() {
        let records = vec![
            header(),
            verdict(1, JudgeVerdict::Fail),
            verdict(2, JudgeVerdict::Partial),
            verdict(1, JudgeVerdict::Success),
        ];

        let footer = summarize_footer(
            "run-001",
            &records,
            "2026-04-26T09:00:10Z".into(),
            VerdictRunStatus::Completed,
        );

        assert_eq!(footer.verdicts_total, 2);
        assert_eq!(footer.verdicts_vulnerable, 2);
        assert_eq!(footer.run_id, "run-001");
    }

    #[test]
    fn read_all_stops_at_malformed_line() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("run-001.verdicts.jsonl");

        std::fs::write(
            &path,
            format!(
                "{}\n{}\n{{CORRUPTED\n",
                serde_json::to_string(&header()).unwrap(),
                serde_json::to_string(&verdict(1, JudgeVerdict::Fail)).unwrap(),
            ),
        )
        .unwrap();

        let records = read_all(&path).unwrap();
        assert_eq!(records.len(), 2);
    }
}
