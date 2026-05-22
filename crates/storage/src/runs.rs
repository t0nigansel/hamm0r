use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

// â”€â”€ Run record types â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// Schema defined in docs/Datamodel.md Â§"Run log".
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
    /// Set on replay run files. References the (run_id, seq) of the
    /// original attempt this replay was derived from. Replay files use
    /// the naming `<original_run>-replay-<n>.jsonl`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_of: Option<ReplaySource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplaySource {
    /// Original run id (e.g. `run-003`).
    pub run_id: String,
    /// Seq of the attempt being replayed.
    pub seq: u32,
    /// True when the user supplied a custom prompt for the replay.
    /// `false` means the original prompt text was re-fired verbatim.
    #[serde(default)]
    pub prompt_overridden: bool,
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
    /// Stable id of the Request template this attempt was fired against.
    /// Recorded so replay can resolve the Request without URL/method
    /// heuristics. `None` for legacy run logs written before this field
    /// existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Mutator id that produced this prompt variant, or `"seed"` for the
    /// unmutated seed prompt. `None` for legacy run logs written before
    /// this field existed (Section 2.9 of docs/ToDo.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation_id: Option<String>,
    /// Section 1.6 of `docs/ToDo.md`. Short session label (e.g. `s0`,
    /// `s1`) when the attempt was fired as part of a multi-session run.
    /// Absent on single-session and legacy logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Section 1.6 of `docs/ToDo.md`. Phase the prompt was fired in
    /// when multi-session was active: `plant`, `probe`, or `any`.
    /// Absent on single-session and legacy logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
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

/// Section 1.5 of `docs/ToDo.md`. Emitted by the post-run scanner when
/// a canary planted in one session surfaces in another session's
/// probe response. Append-only, like every other run record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeakDetected {
    /// `seq` of the probe attempt where the canary surfaced.
    pub probe_seq: u32,
    /// Session that fired the probe (e.g. `s1`).
    pub probe_session: String,
    /// Session that planted the canary (e.g. `s0`).
    pub planted_session: String,
    /// The canary string that matched.
    pub canary: String,
}

/// A single line in a run JSONL file.
///
/// The `type` field selects the variant. Readers must tolerate unknown types
/// (forward compatibility).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunRecord {
    Header(RunHeader),
    Attempt(Box<RunAttempt>),
    Footer(RunFooter),
    /// Multi-session cross-session leak (Section 1.5 of `docs/ToDo.md`).
    LeakDetected(LeakDetected),
}

// â”€â”€ Paths â”€â”€â”€

/// Return the response-body directory for `run_id` inside `engagement_dir`,
/// creating it (and parents) if needed. This is the single enforcement point
/// for response-dir layout — callers outside `storage/` must not assemble or
/// create the path themselves (CLAUDE.md invariant #6).
pub fn ensure_response_dir(engagement_dir: &Path, run_id: &str) -> anyhow::Result<PathBuf> {
    let dir = engagement_dir.join("responses").join(run_id);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("cannot create response dir: {}", dir.display()))?;
    Ok(dir)
}

// â”€â”€ Writer â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

// â”€â”€ Reader â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Read all well-formed records from a run JSONL file.
///
/// A malformed line stops iteration â€” everything after it is treated as absent
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

    // Also delete any replay run files derived from this run (sibling
    // pattern `<run_id>-replay-<n>.jsonl` plus their responses dirs and
    // verdict logs). Replay files only make sense in the context of
    // their original; orphaning them would leave dead artifacts.
    let runs_dir = engagement_dir.join("runs");
    if runs_dir.exists() {
        let replay_prefix = format!("{run_id}-replay-");
        for entry in std::fs::read_dir(&runs_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(s) = name.to_str() else { continue };
            if !s.starts_with(&replay_prefix) {
                continue;
            }
            // Resolve the replay run_id by stripping a `.jsonl` or
            // `.verdicts.jsonl` suffix; skip anything else.
            let replay_id = s
                .strip_suffix(".verdicts.jsonl")
                .or_else(|| s.strip_suffix(".jsonl"));
            let Some(replay_id) = replay_id else { continue };
            let path = entry.path();
            std::fs::remove_file(&path)
                .with_context(|| format!("cannot delete {}", path.display()))?;
            removed += 1;
            let replay_responses = engagement_dir.join("responses").join(replay_id);
            if replay_responses.exists() {
                std::fs::remove_dir_all(&replay_responses)
                    .with_context(|| format!("cannot delete {}", replay_responses.display()))?;
                removed += 1;
            }
        }
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
            replay_of: None,
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
            request_id: None,
            mutation_id: None,
            session_id: None,
            phase: None,
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
    fn replay_header_round_trips_with_replay_of() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("run-003-replay-1.jsonl");
        let header = RunRecord::Header(RunHeader {
            run_id: "run-003-replay-1".into(),
            engagement: "2026-04-25-acme".into(),
            request_id: "openai-chat".into(),
            started_at: "2026-04-25T09:00:00Z".into(),
            runner_version: "test".into(),
            prompt_files: vec![],
            scenario_id: None,
            replay_of: Some(ReplaySource {
                run_id: "run-003".into(),
                seq: 42,
                prompt_overridden: true,
            }),
        });
        append(&path, &header).unwrap();

        let records = read_all(&path).unwrap();
        let RunRecord::Header(h) = &records[0] else {
            panic!("expected header")
        };
        let replay_of = h.replay_of.clone().expect("replay_of populated");
        assert_eq!(replay_of.run_id, "run-003");
        assert_eq!(replay_of.seq, 42);
        assert!(replay_of.prompt_overridden);
    }

    #[test]
    fn run_attempt_round_trips_request_id() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("run-001.jsonl");
        let mut record = attempt(1);
        if let RunRecord::Attempt(a) = &mut record {
            a.request_id = Some("openai-chat".into());
        }
        append(&path, &record).unwrap();

        let records = read_all(&path).unwrap();
        let RunRecord::Attempt(a) = &records[0] else {
            panic!("expected attempt")
        };
        assert_eq!(a.request_id.as_deref(), Some("openai-chat"));
    }

    #[test]
    fn delete_run_also_removes_replay_artifacts() {
        let engagement = TempDir::new().unwrap();
        let runs_dir = engagement.path().join("runs");
        let responses_dir = engagement.path().join("responses");
        std::fs::create_dir_all(&runs_dir).unwrap();
        std::fs::create_dir_all(responses_dir.join("run-003")).unwrap();
        std::fs::create_dir_all(responses_dir.join("run-003-replay-1")).unwrap();
        std::fs::create_dir_all(responses_dir.join("run-003-replay-2")).unwrap();

        std::fs::write(runs_dir.join("run-003.jsonl"), "{}\n").unwrap();
        std::fs::write(runs_dir.join("run-003-replay-1.jsonl"), "{}\n").unwrap();
        std::fs::write(runs_dir.join("run-003-replay-2.jsonl"), "{}\n").unwrap();
        std::fs::write(runs_dir.join("run-004.jsonl"), "{}\n").unwrap();
        std::fs::write(runs_dir.join("run-004-replay-1.jsonl"), "{}\n").unwrap();

        let removed = delete_run(engagement.path(), "run-003").unwrap();
        // 1 (original) + 1 (orig responses) + 2 (replay jsonl files) + 2 (replay responses dirs) = 6
        assert_eq!(removed, 6);

        assert!(!runs_dir.join("run-003.jsonl").exists());
        assert!(!runs_dir.join("run-003-replay-1.jsonl").exists());
        assert!(!runs_dir.join("run-003-replay-2.jsonl").exists());
        // Sibling run + its replays untouched.
        assert!(runs_dir.join("run-004.jsonl").exists());
        assert!(runs_dir.join("run-004-replay-1.jsonl").exists());
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

    // ── Section 1.5 (LeakDetected) round-trip ────────────────────────────

    #[test]
    fn leak_detected_record_serializes_as_snake_case_tag() {
        let record = RunRecord::LeakDetected(LeakDetected {
            probe_seq: 17,
            probe_session: "s1".into(),
            planted_session: "s0".into(),
            canary: "HAMM0R-abcdef01234".into(),
        });
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains("\"type\":\"leak_detected\""));
        assert!(json.contains("\"probe_seq\":17"));
        let back: RunRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, record);
    }

    #[test]
    fn run_attempt_with_session_id_and_phase_round_trips() {
        let attempt = RunRecord::Attempt(Box::new(RunAttempt {
            seq: 1,
            ts: "2026-04-25T09:00:01Z".into(),
            prompt_id: "p".into(),
            payload_id: "pl".into(),
            step_id: None,
            iteration: None,
            session: None,
            prompt_text: None,
            kind: None,
            request_id: None,
            mutation_id: None,
            session_id: Some("s2".into()),
            phase: Some("probe".into()),
            request: RequestEnvelope {
                method: "POST".into(),
                url: "https://example.test".into(),
                headers: HashMap::new(),
                headers_hash: None,
                body_size: 0,
            },
            response: ResponseEnvelope {
                status: 200,
                headers: HashMap::new(),
                body_size: 0,
                body_file: None,
                error: None,
            },
            timing: Timing {
                sent_at: "...".into(),
                first_byte_at: None,
                received_at: "...".into(),
                duration_ms: 0,
            },
            indicators_matched: Vec::new(),
        }));
        let json = serde_json::to_string(&attempt).unwrap();
        assert!(json.contains("\"session_id\":\"s2\""));
        assert!(json.contains("\"phase\":\"probe\""));
        let back: RunRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, attempt);
    }

    #[test]
    fn legacy_run_attempt_without_session_id_loads() {
        let line = r#"{"type":"attempt","seq":1,"ts":"t","prompt_id":"p","payload_id":"pl","request":{"method":"POST","url":"u","body_size":0},"response":{"status":200,"headers":{},"body_size":0,"body_file":null},"timing":{"sent_at":"t","received_at":"t","duration_ms":0}}"#;
        let record: RunRecord = serde_json::from_str(line).unwrap();
        let RunRecord::Attempt(a) = record else {
            panic!("expected attempt");
        };
        assert!(a.session_id.is_none());
        assert!(a.phase.is_none());
    }
}
