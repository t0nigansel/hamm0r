use runner::session::SessionStrategy;
use runner::{execute_run, execute_scenario_run, Payload, RunConfig, ScenarioRunConfig, ScenarioStep};
use serde::{Deserialize, Serialize};
use storage::runs::{read_all, RunRecord};
use storage::types::{Request, SessionConfig, Target};
use storage::{requests, scenarios, targets};
use tauri::{AppHandle, Emitter as _, State};

use super::AppPaths;
use crate::error::CommandError;

/// Payload descriptor sent from the UI for a single fire.
#[derive(Debug, Deserialize)]
pub struct PayloadSpec {
    pub prompt_id: String,
    pub payload_id: String,
    pub text: String,
}

/// Progress event emitted to the UI after each attempt.
#[derive(Debug, Clone, Serialize)]
pub struct RunProgressEvent {
    pub run_id: String,
    pub seq: u32,
    pub total: u32,
    pub status: u16,
    pub error: Option<String>,
    pub finished: bool,
}

/// Start a run for the given engagement + request + payload list.
///
/// Returns the run_id immediately and fires progress events (`run-progress`)
/// via Tauri as each attempt completes. The JSONL file is written to
/// `<engagements_dir>/<engagement_slug>/runs/<run_id>.jsonl`.
#[tauri::command]
pub async fn start_run(
    app: AppHandle,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    request_id: String,
    payloads: Vec<PayloadSpec>,
    parallelism: Option<usize>,
) -> Result<String, CommandError> {
    let all_requests = requests::load_all(&paths.0.requests_dir())?;
    let request = all_requests
        .get(&request_id)
        .ok_or_else(|| anyhow::anyhow!("request '{}' not found", request_id))?
        .clone();

    let run_id = next_run_id(&paths.0.engagement_dir(&engagement_slug))?;
    let engagement_dir = paths.0.engagement_dir(&engagement_slug);

    let runner_payloads: Vec<Payload> = payloads
        .into_iter()
        .map(|p| Payload {
            prompt_id: p.prompt_id,
            payload_id: p.payload_id,
            text: p.text,
            session: "default".into(),
        })
        .collect();

    let config = RunConfig {
        engagement_dir,
        run_id: run_id.clone(),
        request,
        payloads: runner_payloads,
        parallelism: parallelism.unwrap_or(4),
        runner_version: env!("CARGO_PKG_VERSION").to_owned(),
    };

    let run_id_ret = run_id.clone();

    tokio::spawn(async move {
        let result = execute_run(config, move |progress| {
            let event = RunProgressEvent {
                run_id: progress.run_id,
                seq: progress.seq,
                total: progress.total,
                status: progress.status,
                error: progress.error,
                finished: progress.finished,
            };
            let _ = app.emit("run-progress", event);
        })
        .await;

        if let Err(e) = result {
            eprintln!("run {run_id} failed: {e}");
        }
    });

    Ok(run_id_ret)
}

/// Start a run from a persisted scenario definition.
#[tauri::command]
pub async fn start_scenario_run(
    app: AppHandle,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    scenario_id: String,
) -> Result<String, CommandError> {
    let scenario = scenarios::load(&paths.0.scenarios_dir(), &scenario_id)?
        .ok_or_else(|| anyhow::anyhow!("scenario '{}' not found", scenario_id))?;

    if scenario.target_id.trim().is_empty() {
        return Err(anyhow::anyhow!("scenario has no target selected").into());
    }
    if scenario.steps.is_empty() {
        return Err(anyhow::anyhow!("scenario has no steps").into());
    }

    let (target, request) = load_target_and_request(&paths.0, &scenario.target_id)?;
    let run_id = next_run_id(&paths.0.engagement_dir(&engagement_slug))?;
    let engagement_dir = paths.0.engagement_dir(&engagement_slug);
    let session_strategy = session_strategy_from_target(&target.session_config);
    let steps: Vec<ScenarioStep> = scenario
        .steps
        .iter()
        .enumerate()
        .map(|(idx, step)| ScenarioStep {
            id: if step.id.trim().is_empty() {
                format!("step-{:03}", idx + 1)
            } else {
                step.id.clone()
            },
            prompt_id: step.prompt_id.clone(),
            prompt_text: step.prompt_text.clone(),
            session: if step.session.trim().is_empty() {
                "A".to_owned()
            } else {
                step.session.clone()
            },
        })
        .collect();

    let config = ScenarioRunConfig {
        engagement_dir,
        run_id: run_id.clone(),
        request,
        session_strategy,
        steps,
        repeat: scenario.repeat.max(1),
        runner_version: env!("CARGO_PKG_VERSION").to_owned(),
    };

    let run_id_ret = run_id.clone();
    tokio::spawn(async move {
        let result = execute_scenario_run(config, move |progress| {
            let event = RunProgressEvent {
                run_id: progress.run_id,
                seq: progress.seq,
                total: progress.total,
                status: progress.status,
                error: progress.error,
                finished: progress.finished,
            };
            let _ = app.emit("run-progress", event);
        })
        .await;

        if let Err(e) = result {
            eprintln!("scenario run {run_id} failed: {e}");
        }
    });

    Ok(run_id_ret)
}

/// Start a transient one-step scenario (used by Quick Run / Workbench).
#[tauri::command]
pub async fn start_transient_scenario_run(
    app: AppHandle,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    target_id: String,
    prompt_text: String,
    prompt_id: Option<String>,
) -> Result<String, CommandError> {
    if prompt_text.trim().is_empty() {
        return Err(anyhow::anyhow!("prompt text is empty").into());
    }

    let (target, request) = load_target_and_request(&paths.0, &target_id)?;
    let run_id = next_run_id(&paths.0.engagement_dir(&engagement_slug))?;
    let engagement_dir = paths.0.engagement_dir(&engagement_slug);
    let session_strategy = session_strategy_from_target(&target.session_config);

    let config = ScenarioRunConfig {
        engagement_dir,
        run_id: run_id.clone(),
        request,
        session_strategy,
        steps: vec![ScenarioStep {
            id: "step-001".to_owned(),
            prompt_id,
            prompt_text,
            session: "A".to_owned(),
        }],
        repeat: 1,
        runner_version: env!("CARGO_PKG_VERSION").to_owned(),
    };

    let run_id_ret = run_id.clone();
    tokio::spawn(async move {
        let result = execute_scenario_run(config, move |progress| {
            let event = RunProgressEvent {
                run_id: progress.run_id,
                seq: progress.seq,
                total: progress.total,
                status: progress.status,
                error: progress.error,
                finished: progress.finished,
            };
            let _ = app.emit("run-progress", event);
        })
        .await;

        if let Err(e) = result {
            eprintln!("transient run {run_id} failed: {e}");
        }
    });

    Ok(run_id_ret)
}

/// Read attempt records from a run's JSONL file. Returns a JSON array of
/// attempt objects (headers and footers are omitted).
#[tauri::command]
pub fn read_run_attempts(
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
) -> Result<Vec<serde_json::Value>, CommandError> {
    let run_path = paths
        .0
        .engagement_dir(&engagement_slug)
        .join("runs")
        .join(format!("{run_id}.jsonl"));

    if !run_path.exists() {
        return Ok(vec![]);
    }

    let records = read_all(&run_path)?;
    let attempts = records
        .into_iter()
        .filter_map(|r| match r {
            RunRecord::Attempt(a) => serde_json::to_value(*a).ok(),
            _ => None,
        })
        .collect();
    Ok(attempts)
}

/// Read the raw text of one response body file.
#[tauri::command]
pub fn read_response_body(
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
    seq: u32,
) -> Result<Option<String>, CommandError> {
    let engagement_dir = paths.0.engagement_dir(&engagement_slug);
    storage::runs::read_response_body(&engagement_dir, &run_id, seq).map_err(Into::into)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn load_target_and_request(
    paths: &storage::HammorPaths,
    target_id: &str,
) -> Result<(Target, Request), CommandError> {
    let all_targets = targets::load_all(&paths.targets_dir())?;
    let target = all_targets
        .get(target_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("target '{}' not found", target_id))?;

    let all_requests = requests::load_all(&paths.requests_dir())?;
    let request = all_requests
        .get(&target.request_id)
        .or_else(|| all_requests.get(&target.id))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("request '{}' not found", target.request_id))?;

    Ok((target, request))
}

fn session_strategy_from_target(config: &SessionConfig) -> SessionStrategy {
    match config {
        SessionConfig::None => SessionStrategy::None,
        SessionConfig::Cookie => SessionStrategy::Cookie,
        SessionConfig::Header { header_name } => SessionStrategy::Header {
            header_name: header_name.clone(),
        },
        SessionConfig::BodyField { field_name } => SessionStrategy::BodyField {
            field_name: field_name.clone(),
        },
    }
}

fn next_run_id(engagement_dir: &std::path::Path) -> anyhow::Result<String> {
    let runs_dir = engagement_dir.join("runs");
    if !runs_dir.exists() {
        return Ok("run-001".to_owned());
    }

    let max_seq = std::fs::read_dir(&runs_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            s.strip_prefix("run-")
                .and_then(|r| r.strip_suffix(".jsonl"))
                .and_then(|n| n.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);

    Ok(format!("run-{:03}", max_seq + 1))
}
