use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

// ── Run record types ───────────────────────────────────────────────────────────
// Schema defined in docs/Datamodel.md §"Run log".
// These are the types the runner writes; the analyzer reads them back.

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunHeader {
    pub run_id: String,
    pub engagement: String,
    pub request_id: String,
    pub started_at: String,
    pub runner_version: String,
    pub prompt_files: Vec<String>,
    /// Scenario the run was launched from, if any. Set by matrix runs;
    /// absent on flat ad-hoc runs (e.g. the per-result rerun path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub method: String,
    pub url: String,
    /// Request headers captured for debugging (sensitive values should be masked).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    /// Legacy field kept for backward compatibility with old run logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers_hash: Option<String>,
    pub body_size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    /// HTTP status code, or 0 if the request failed.
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body_size: u64,
    /// Relative path to the raw body file, or null if no body was received.
    pub body_file: Option<String>,
    /// Non-null when the request failed before a response was received.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timing {
    pub sent_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_byte_at: Option<String>,
    pub received_at: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunAttempt {
    pub seq: u32,
    pub ts: String,
    pub prompt_id: String,
    pub payload_id: String,
    /// Scenario step identifier when the run came from a scenario.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    /// 1-based iteration index when scenario repeat > 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    /// Session label used for this attempt (for example A/B/C).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Prompt snapshot text used for this attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_text: Option<String>,
    /// Phase 2 of docs/RefactorPlan.md: tags non-user-facing attempts.
    /// Currently `Some("prerequisite")` for auth-chain prerequisite
    /// firings, `None` for ordinary attempts. Readers must tolerate
    /// unknown values (forward compatibility).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub request: RequestEnvelope,
    pub response: ResponseEnvelope,
    pub timing: Timing,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub indicators_matched: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    AbortedByUser,
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunFooter {
    pub run_id: String,
    pub finished_at: String,
    pub attempts_total: u32,
    pub attempts_failed: u32,
    pub status: RunStatus,
}

/// A single line in a run JSONL file.
///
/// The `type` field selects the variant. Readers must tolerate unknown types
/// (forward compatibility).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RunRecord {
    Header(RunHeader),
    Attempt(Box<RunAttempt>),
    Footer(RunFooter),
}

// ── Writer ────────────────────────────────────────────────────────────────────

/// Append one record to a run JSONL file with one `fsync` per call.
///
/// The file is created if it does not exist. Never truncates or rewrites
/// existing lines (CLAUDE.md invariant 12).
pub fn append(run_path: &Path, record: &RunRecord) -> anyhow::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_path)
        .with_context(|| format!("cannot open run file: {}", run_path.display()))?;

    serde_json::to_writer(&mut file, record)
        .with_context(|| format!("cannot serialise record to {}", run_path.display()))?;

    file.write_all(b"\n")
        .with_context(|| format!("cannot write newline to {}", run_path.display()))?;

    file.flush()
        .with_context(|| format!("cannot flush {}", run_path.display()))?;

    file.sync_data()
        .with_context(|| format!("cannot fsync {}", run_path.display()))?;

    Ok(())
}

// ── Reader ────────────────────────────────────────────────────────────────────

/// Read all well-formed records from a run JSONL file.
///
/// A malformed line stops iteration — everything after it is treated as absent
/// (crash-recovery semantics per Datamodel.md). The caller can inspect how many
/// records were read vs the file's line count to detect truncation.
pub fn read_all(run_path: &Path) -> anyhow::Result<Vec<RunRecord>> {
    let raw = std::fs::read_to_string(run_path)
        .with_context(|| format!("cannot read run file: {}", run_path.display()))?;

    let mut records = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<RunRecord>(line) {
            Ok(r) => records.push(r),
            Err(_) => break, // stop at first malformed line
        }
    }
    Ok(records)
}

/// Read the raw text of one response body file by run ID and sequence number.
pub fn read_response_body(
    engagement_dir: &Path,
    run_id: &str,
    seq: u32,
) -> anyhow::Result<Option<String>> {
    read_body_at(
        &engagement_dir
            .join("responses")
            .join(run_id)
            .join(format!("{seq:04}.txt")),
    )
}

/// Read a response body file using its relative path as stored in `RunAttempt.body_file`.
pub fn read_body_by_relative_path(
    engagement_dir: &Path,
    relative_path: &str,
) -> anyhow::Result<Option<String>> {
    read_body_at(&engagement_dir.join(relative_path))
}

/// Permanently remove every artifact tied to a run inside `engagement_dir`:
/// the run JSONL, its verdicts JSONL, the responses directory, and any
/// generated report HTML. Missing files are tolerated. Returns the number
/// of filesystem entries removed (for diagnostics in the UI toast).
pub fn delete_run(engagement_dir: &Path, run_id: &str) -> anyhow::Result<usize> {
    if run_id.trim().is_empty() {
        anyhow::bail!("run_id must not be empty");
    }

    let mut removed = 0usize;

    let run_log = engagement_dir.join("runs").join(format!("{run_id}.jsonl"));
    if run_log.exists() {
        std::fs::remove_file(&run_log)
            .with_context(|| format!("cannot delete {}", run_log.display()))?;
        removed += 1;
    }

    let verdicts = engagement_dir
        .join("runs")
        .join(format!("{run_id}.verdicts.jsonl"));
    if verdicts.exists() {
        std::fs::remove_file(&verdicts)
            .with_context(|| format!("cannot delete {}", verdicts.display()))?;
        removed += 1;
    }

    let responses_dir = engagement_dir.join("responses").join(run_id);
    if responses_dir.exists() {
        std::fs::remove_dir_all(&responses_dir)
            .with_context(|| format!("cannot delete {}", responses_dir.display()))?;
        removed += 1;
    }

    let report = engagement_dir
        .join("reports")
        .join(format!("report-{run_id}.html"));
    if report.exists() {
        std::fs::remove_file(&report)
            .with_context(|| format!("cannot delete {}", report.display()))?;
        removed += 1;
    }

    Ok(removed)
}

fn read_body_at(path: &Path) -> anyhow::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    std::fs::read_to_string(path)
        .map(Some)
        .with_context(|| format!("cannot read response body: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn header() -> RunRecord {
        RunRecord::Header(RunHeader {
            run_id: "run-001".into(),
            engagement: "2026-04-25-acme".into(),
            request_id: "openai-chat".into(),
            started_at: "2026-04-25T09:00:00Z".into(),
            runner_version: "0.1.0".into(),
            prompt_files: vec!["injection-classics".into()],
            scenario_id: None,
        })
    }

    fn attempt(seq: u32) -> RunRecord {
        RunRecord::Attempt(Box::new(RunAttempt {
            seq,
            ts: "2026-04-25T09:00:01Z".into(),
            prompt_id: "injection-classics".into(),
            payload_id: "inj-001".into(),
            step_id: None,
            iteration: None,
            session: None,
            prompt_text: None,
            kind: None,
            request: RequestEnvelope {
                method: "POST".into(),
                url: "https://api.example.com".into(),
                headers: HashMap::from([("content-type".into(), "application/json".into())]),
                headers_hash: None,
                body_size: 100,
            },
            response: ResponseEnvelope {
                status: 200,
                headers: HashMap::new(),
                body_size: 500,
                body_file: Some(format!("responses/run-001/{seq:04}.txt")),
                error: None,
            },
            timing: Timing {
                sent_at: "2026-04-25T09:00:01.000Z".into(),
                first_byte_at: Some("2026-04-25T09:00:01.100Z".into()),
                received_at: "2026-04-25T09:00:01.200Z".into(),
                duration_ms: 200,
            },
            indicators_matched: vec![],
        }))
    }

    fn footer() -> RunRecord {
        RunRecord::Footer(RunFooter {
            run_id: "run-001".into(),
            finished_at: "2026-04-25T09:01:00Z".into(),
            attempts_total: 2,
            attempts_failed: 0,
            status: RunStatus::Completed,
        })
    }

    #[test]
    fn append_and_read_all() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("run-001.jsonl");

        append(&path, &header()).unwrap();
        append(&path, &attempt(1)).unwrap();
        append(&path, &attempt(2)).unwrap();
        append(&path, &footer()).unwrap();

        let records = read_all(&path).unwrap();
        assert_eq!(records.len(), 4);
        assert!(matches!(records[0], RunRecord::Header(_)));
        assert!(matches!(records[1], RunRecord::Attempt(_)));
        assert!(matches!(records[3], RunRecord::Footer(_)));
    }

    #[test]
    fn append_is_additive() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("run-001.jsonl");

        append(&path, &header()).unwrap();
        append(&path, &attempt(1)).unwrap();

        // Second open must not truncate.
        append(&path, &attempt(2)).unwrap();

        let records = read_all(&path).unwrap();
        assert_eq!(records.len(), 3);
    }

    #[test]
    fn read_all_stops_at_malformed_line() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("run-001.jsonl");

        // Write two valid records, then a corrupted line, then another valid one.
        append(&path, &header()).unwrap();
        append(&path, &attempt(1)).unwrap();
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n{{CORRUPTED\n{}\n",
                serde_json::to_string(&header()).unwrap(),
                serde_json::to_string(&attempt(1)).unwrap(),
                serde_json::to_string(&footer()).unwrap(),
            ),
        )
        .unwrap();

        let records = read_all(&path).unwrap();
        // Only the two records before the corrupt line should be present.
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn each_record_is_one_line() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("run-001.jsonl");
        append(&path, &header()).unwrap();
        append(&path, &attempt(1)).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 2);
    }

    #[test]
    fn delete_run_removes_all_artifacts() {
        let engagement = TempDir::new().unwrap();
        let runs_dir = engagement.path().join("runs");
        let responses_dir = engagement.path().join("responses").join("run-001");
        let reports_dir = engagement.path().join("reports");
        std::fs::create_dir_all(&runs_dir).unwrap();
        std::fs::create_dir_all(&responses_dir).unwrap();
        std::fs::create_dir_all(&reports_dir).unwrap();

        std::fs::write(runs_dir.join("run-001.jsonl"), "{}\n").unwrap();
        std::fs::write(runs_dir.join("run-001.verdicts.jsonl"), "{}\n").unwrap();
        std::fs::write(responses_dir.join("0001.txt"), "body").unwrap();
        std::fs::write(reports_dir.join("report-run-001.html"), "<html/>").unwrap();
        // Adjacent run that must NOT be touched.
        std::fs::write(runs_dir.join("run-002.jsonl"), "{}\n").unwrap();

        let removed = delete_run(engagement.path(), "run-001").unwrap();
        assert_eq!(removed, 4);

        assert!(!runs_dir.join("run-001.jsonl").exists());
        assert!(!runs_dir.join("run-001.verdicts.jsonl").exists());
        assert!(!responses_dir.exists());
        assert!(!reports_dir.join("report-run-001.html").exists());
        // Sibling run untouched.
        assert!(runs_dir.join("run-002.jsonl").exists());
    }

    #[test]
    fn delete_run_is_idempotent_when_artifacts_missing() {
        let engagement = TempDir::new().unwrap();
        // Only the runs directory exists; no files inside.
        std::fs::create_dir_all(engagement.path().join("runs")).unwrap();

        let removed = delete_run(engagement.path(), "run-missing").unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn delete_run_rejects_empty_id() {
        let engagement = TempDir::new().unwrap();
        assert!(delete_run(engagement.path(), "").is_err());
        assert!(delete_run(engagement.path(), "   ").is_err());
    }
}
