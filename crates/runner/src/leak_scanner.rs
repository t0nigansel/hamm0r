// Section 1.5 of docs/ToDo.md and Phase 4 of multiSessionPlan.md.
//
// After a multi-session run finishes, this module walks every probe /
// any-phase attempt in the JSONL, loads the corresponding response
// file, and checks whether *any other session's* canary surfaces in
// the body. Hits are appended to the same run JSONL as
// `RunRecord::LeakDetected` records — append-only, like every other
// run record (CLAUDE.md invariant #12).
//
// The scanner is pure I/O over files that the runner has already
// fsynced. It does not touch the network, does not touch the analyzer,
// and runs in the same task as the multi-session runner so the
// JSONL is single-writer.

use std::collections::HashMap;
use std::path::Path;

use storage::runs::{LeakDetected, RunRecord};

use crate::canary::CANARY_PREFIX;
use crate::error::RunnerError;

/// Scan `run_path`'s probe/any attempts for cross-session canary leaks.
/// `canaries_by_session` maps session labels (`s0`, `s1`, ...) to their
/// canary strings. Writes one `LeakDetected` record per (probe seq,
/// planted session) pair where the planted session's canary appears in
/// the probe response body.
pub fn run(
    engagement_dir: &Path,
    run_id: &str,
    canaries_by_session: &HashMap<String, String>,
    run_path: &Path,
) -> Result<u32, RunnerError> {
    let records = storage::runs::read_all(run_path).map_err(|e| anyhow::anyhow!(e))?;

    let mut leaks_emitted = 0u32;
    for record in &records {
        let attempt = match record {
            RunRecord::Attempt(a) => a.as_ref(),
            _ => continue,
        };

        // Only probe and any-phase attempts can leak. Plant prompts
        // are the ones that planted the canary, by definition.
        let phase = attempt.phase.as_deref().unwrap_or("any");
        if phase == "plant" {
            continue;
        }

        let Some(probe_session) = attempt.session_id.clone() else {
            continue;
        };
        let Some(body_file) = &attempt.response.body_file else {
            continue;
        };

        // body_file is relative to the engagement dir.
        let body_path = engagement_dir.join(body_file);
        let body = match std::fs::read_to_string(&body_path) {
            Ok(b) => b,
            // Best-effort: a missing or non-UTF-8 response file just
            // means we can't scan that one. Don't fail the whole pass.
            Err(_) => continue,
        };

        // Fast filter: if the prefix isn't anywhere in the body, no
        // canary can possibly be present. Avoids N substring scans
        // per attempt when no session leaked.
        if !body.contains(CANARY_PREFIX) {
            continue;
        }

        for (planted_session, canary) in canaries_by_session {
            if planted_session == &probe_session {
                continue;
            }
            if body.contains(canary.as_str()) {
                let _ = run_id; // run_id is encoded in the path; not duplicated in the record.
                let leak = LeakDetected {
                    probe_seq: attempt.seq,
                    probe_session: probe_session.clone(),
                    planted_session: planted_session.clone(),
                    canary: canary.clone(),
                };
                storage::runs::append(run_path, &RunRecord::LeakDetected(leak))
                    .map_err(|e| anyhow::anyhow!(e))?;
                leaks_emitted += 1;
            }
        }
    }
    Ok(leaks_emitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use storage::runs::{
        RequestEnvelope, ResponseEnvelope, RunAttempt, RunFooter, RunHeader, RunStatus, Timing,
    };
    use tempfile::TempDir;

    fn make_attempt(
        seq: u32,
        session: &str,
        phase: &str,
        body_file: Option<&str>,
    ) -> RunRecord {
        RunRecord::Attempt(Box::new(RunAttempt {
            seq,
            ts: "t".into(),
            prompt_id: "p".into(),
            payload_id: "pl".into(),
            step_id: None,
            iteration: None,
            session: Some(session.into()),
            prompt_text: None,
            kind: None,
            request_id: None,
            mutation_id: None,
            session_id: Some(session.into()),
            phase: Some(phase.into()),
            request: RequestEnvelope {
                method: "POST".into(),
                url: "http://x".into(),
                headers: HashMap::new(),
                headers_hash: None,
                body_size: 0,
            },
            response: ResponseEnvelope {
                status: 200,
                headers: HashMap::new(),
                body_size: 0,
                body_file: body_file.map(str::to_owned),
                error: None,
            },
            timing: Timing {
                sent_at: "t".into(),
                first_byte_at: None,
                received_at: "t".into(),
                duration_ms: 0,
            },
            indicators_matched: Vec::new(),
        }))
    }

    fn write_jsonl(path: &Path, records: &[RunRecord]) {
        for r in records {
            storage::runs::append(path, r).unwrap();
        }
    }

    fn setup_engagement(records: &[RunRecord], responses: &[(&str, &str)]) -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let runs_dir = dir.path().join("runs");
        fs::create_dir_all(&runs_dir).unwrap();
        let responses_dir = dir.path().join("responses").join("run-001");
        fs::create_dir_all(&responses_dir).unwrap();
        for (rel, body) in responses {
            // rel is "responses/run-001/0001.txt" — strip the
            // engagement-relative prefix and write the file.
            let p = dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, body).unwrap();
        }
        let run_path = runs_dir.join("run-001.jsonl");
        write_jsonl(&run_path, records);
        (dir, run_path)
    }

    #[test]
    fn detects_canary_from_other_session_in_probe() {
        let header = RunRecord::Header(RunHeader {
            run_id: "run-001".into(),
            engagement: "e".into(),
            request_id: "r".into(),
            started_at: "t".into(),
            runner_version: "test".into(),
            prompt_files: vec![],
            scenario_id: None,
            replay_of: None,
        });
        let s0_plant = make_attempt(1, "s0", "plant", Some("responses/run-001/0001.txt"));
        let s1_probe = make_attempt(2, "s1", "probe", Some("responses/run-001/0002.txt"));
        let footer = RunRecord::Footer(RunFooter {
            run_id: "run-001".into(),
            finished_at: "t".into(),
            attempts_total: 2,
            attempts_failed: 0,
            status: RunStatus::Completed,
        });

        let (dir, run_path) = setup_engagement(
            &[header, s0_plant, s1_probe, footer],
            &[
                ("responses/run-001/0001.txt", "planted ok"),
                (
                    "responses/run-001/0002.txt",
                    "model says: I once heard HAMM0R-abcdef01234 from another user",
                ),
            ],
        );

        let mut canaries = HashMap::new();
        canaries.insert("s0".into(), "HAMM0R-abcdef01234".to_owned());
        canaries.insert("s1".into(), "HAMM0R-zzzzzzzzzzz".to_owned());

        let n = run(dir.path(), "run-001", &canaries, &run_path).unwrap();
        assert_eq!(n, 1);

        let records = storage::runs::read_all(&run_path).unwrap();
        let leaks: Vec<_> = records
            .iter()
            .filter_map(|r| match r {
                RunRecord::LeakDetected(l) => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(leaks.len(), 1);
        assert_eq!(leaks[0].probe_seq, 2);
        assert_eq!(leaks[0].probe_session, "s1");
        assert_eq!(leaks[0].planted_session, "s0");
        assert_eq!(leaks[0].canary, "HAMM0R-abcdef01234");
    }

    #[test]
    fn no_leak_when_probe_body_does_not_contain_canary() {
        let header = RunRecord::Header(RunHeader {
            run_id: "run-001".into(),
            engagement: "e".into(),
            request_id: "r".into(),
            started_at: "t".into(),
            runner_version: "test".into(),
            prompt_files: vec![],
            scenario_id: None,
            replay_of: None,
        });
        let s1_probe = make_attempt(1, "s1", "probe", Some("responses/run-001/0001.txt"));

        let (dir, run_path) = setup_engagement(
            &[header, s1_probe],
            &[("responses/run-001/0001.txt", "the model said nothing leaky")],
        );

        let mut canaries = HashMap::new();
        canaries.insert("s0".into(), "HAMM0R-aaaaaaaaaaa".to_owned());
        canaries.insert("s1".into(), "HAMM0R-bbbbbbbbbbb".to_owned());

        let n = run(dir.path(), "run-001", &canaries, &run_path).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn same_session_canary_in_own_response_is_not_a_leak() {
        // s0 planted the canary; if s0's *own* probe response echoes
        // it back, that's not cross-session leakage — it's the same
        // session reading what it wrote.
        let header = RunRecord::Header(RunHeader {
            run_id: "run-001".into(),
            engagement: "e".into(),
            request_id: "r".into(),
            started_at: "t".into(),
            runner_version: "test".into(),
            prompt_files: vec![],
            scenario_id: None,
            replay_of: None,
        });
        let s0_probe = make_attempt(1, "s0", "probe", Some("responses/run-001/0001.txt"));

        let (dir, run_path) = setup_engagement(
            &[header, s0_probe],
            &[("responses/run-001/0001.txt", "echo HAMM0R-aaaaaaaaaaa")],
        );

        let mut canaries = HashMap::new();
        canaries.insert("s0".into(), "HAMM0R-aaaaaaaaaaa".to_owned());
        canaries.insert("s1".into(), "HAMM0R-bbbbbbbbbbb".to_owned());

        let n = run(dir.path(), "run-001", &canaries, &run_path).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn plant_phase_attempts_are_skipped() {
        // Plants don't probe for leaks (they create them); even if a
        // plant response happens to echo the canary, that's not a leak.
        let header = RunRecord::Header(RunHeader {
            run_id: "run-001".into(),
            engagement: "e".into(),
            request_id: "r".into(),
            started_at: "t".into(),
            runner_version: "test".into(),
            prompt_files: vec![],
            scenario_id: None,
            replay_of: None,
        });
        let s1_plant = make_attempt(1, "s1", "plant", Some("responses/run-001/0001.txt"));

        let (dir, run_path) = setup_engagement(
            &[header, s1_plant],
            &[(
                "responses/run-001/0001.txt",
                "the plant echoed HAMM0R-aaaaaaaaaaa from s0",
            )],
        );

        let mut canaries = HashMap::new();
        canaries.insert("s0".into(), "HAMM0R-aaaaaaaaaaa".to_owned());
        canaries.insert("s1".into(), "HAMM0R-bbbbbbbbbbb".to_owned());

        let n = run(dir.path(), "run-001", &canaries, &run_path).unwrap();
        assert_eq!(n, 0);
    }
}
