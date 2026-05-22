use std::path::{Path, PathBuf};

use analyzer::report::{build_report_data, render_html_report, ReportAttempt, ReportBuildInput};
use insta::assert_snapshot;
use storage::runs::{self, RunRecord};
use storage::verdicts;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read_fixture_response(fixtures: &Path, seq: u32) -> String {
    let path = fixtures.join(format!("response-{seq:04}.txt"));
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn fixture_run_renders_stable_report_html() {
    let fixtures = fixtures_dir();
    let run_path = fixtures.join("run-001.jsonl");
    let verdict_path = fixtures.join("run-001.verdicts.jsonl");

    let run_records = runs::read_all(&run_path).expect("fixture run JSONL should parse");
    let verdict_records =
        verdicts::read_all(&verdict_path).expect("fixture verdict JSONL should parse");

    let mut engagement_slug = String::from("fixture-engagement");
    let mut run_id = String::from("run-001");
    let mut started_at = None;
    let mut finished_at = None;
    let mut attempts = Vec::new();

    for record in run_records {
        match record {
            RunRecord::Header(header) => {
                engagement_slug = header.engagement;
                run_id = header.run_id;
                started_at = Some(header.started_at);
            }
            RunRecord::Attempt(attempt) => attempts.push(*attempt),
            RunRecord::Footer(footer) => {
                finished_at = Some(footer.finished_at);
            }
            RunRecord::LeakDetected(_) => {}
        }
    }

    let mut latest_verdicts = verdicts::latest_by_seq(&verdict_records)
        .into_values()
        .collect::<Vec<_>>();
    latest_verdicts.sort_by_key(|v| v.seq);

    let report_attempts = attempts
        .into_iter()
        .map(|attempt| ReportAttempt {
            seq: attempt.seq,
            prompt_id: attempt.prompt_id,
            step_id: attempt.step_id,
            iteration: attempt.iteration,
            session: attempt.session,
            http_status: attempt.response.status,
            latency_ms: Some(attempt.timing.duration_ms),
            response_excerpt: read_fixture_response(&fixtures, attempt.seq),
        })
        .collect::<Vec<_>>();

    let report_data = build_report_data(ReportBuildInput {
        engagement_slug,
        run_id,
        generated_at: "2026-04-26T10:02:00Z".to_owned(),
        started_at,
        finished_at,
        judge_model: verdict_records.iter().find_map(|record| match record {
            verdicts::VerdictRecord::Header(header) => Some(header.model.clone()),
            _ => None,
        }),
        attempts: report_attempts,
        verdicts: latest_verdicts,
        triage: Vec::new(),
    });

    let html = render_html_report(&report_data).expect("fixture report should render");
    assert_snapshot!("fixture_run_report_html", html);
}
