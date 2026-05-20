use runner::session::SessionStrategy;
use runner::{execute_matrix_run, execute_run, AttemptLog, MatrixRunConfig, Payload, RunConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use storage::prompts;
use storage::runs::{self, read_all, RunRecord};
use storage::types::Request;
use storage::{requests, scenarios};
use tauri::{AppHandle, Emitter as _, State};

use super::{
    emit_user_relevant_error, report_user_relevant_error, ActiveRunsState, AppConfigState,
    AppPaths, LoggerState,
};
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

#[derive(Debug, Clone, Serialize)]
pub struct RunDiagnostics {
    pub run_id: String,
    pub status: String,
    pub request_id: Option<String>,
    pub request_url: Option<String>,
    pub attempts: u32,
    pub has_footer: bool,
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StopRunResult {
    pub stopped: bool,
}

/// Start a run for the given engagement + request + payload list.
///
/// Returns the run_id immediately and fires progress events (`run-progress`)
/// via Tauri as each attempt completes. The JSONL file is written to
/// `<engagements_dir>/<engagement_slug>/runs/<run_id>.jsonl`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_run(
    app: AppHandle,
    active_runs: State<'_, ActiveRunsState>,
    config_state: State<'_, AppConfigState>,
    paths: State<'_, AppPaths>,
    logger: State<'_, LoggerState>,
    engagement_slug: String,
    request_id: String,
    payloads: Vec<PayloadSpec>,
    parallelism: Option<usize>,
) -> Result<String, CommandError> {
    logger.0.info(
        "runner",
        None,
        &format!(
            "start_run requested for engagement={} request_id={} payloads={}",
            engagement_slug,
            request_id,
            payloads.len()
        ),
    );

    let all_requests = requests::load_all(&paths.0.requests_dir())?;
    let request = match all_requests.get(&request_id) {
        Some(request) => request.clone(),
        None => {
            let message = format!("request '{}' not found", request_id);
            logger.0.error("runner", None, &message);
            return Err(anyhow::anyhow!(message).into());
        }
    };

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

    let cancellation = runner::run::RunCancellation::new();
    let config = RunConfig {
        engagement_dir,
        run_id: run_id.clone(),
        request,
        payloads: runner_payloads,
        parallelism: parallelism.unwrap_or(4),
        runner_version: env!("CARGO_PKG_VERSION").to_owned(),
        body_logging_enabled: config_state.0.logging.body_logging_enabled,
        cancellation: Some(cancellation.clone()),
        on_attempt_log: Some(Arc::new({
            let logger = logger.0.clone();
            let app = app.clone();
            let first_error_reported = Arc::new(AtomicBool::new(false));
            move |attempt| {
                log_attempt(&logger, attempt.clone());
                if let Some(error) = &attempt.error {
                    if !first_error_reported.swap(true, Ordering::SeqCst) {
                        let (scope, message) =
                            build_attempt_error_event("run-attempt", "Run", &attempt, error);
                        emit_user_relevant_error(&app, &scope, Some(&attempt.run_id), &message);
                    }
                }
            }
        })),
    };
    let request_url = config.request.url.clone();
    let engagement_dir_for_error = paths.0.engagement_dir(&engagement_slug);

    let run_id_ret = run_id.clone();
    let logger = logger.0.clone();
    let active_runs_map = active_runs.0.clone();
    active_runs_map
        .lock()
        .map_err(|_| anyhow::anyhow!("active run registry poisoned"))?
        .insert(run_id.clone(), cancellation);

    tokio::spawn(async move {
        let app_for_progress = app.clone();
        let app_for_error = app;
        logger.info(
            "runner",
            Some(&run_id),
            &format!("Run task spawned for url={request_url}"),
        );
        let result = execute_run(config, move |progress| {
            let event = RunProgressEvent {
                run_id: progress.run_id,
                seq: progress.seq,
                total: progress.total,
                status: progress.status,
                error: progress.error,
                finished: progress.finished,
            };
            let _ = app_for_progress.emit("run-progress", event);
        })
        .await;

        if let Err(e) = result {
            let startup_error = format!("run execution failed (url: {request_url}): {e}");
            report_user_relevant_error(
                &app_for_error,
                &logger,
                "runner",
                "run-execution",
                Some(&run_id),
                &startup_error,
            );
            let _ = write_run_startup_error(&engagement_dir_for_error, &run_id, &startup_error);
            let _ = app_for_error.emit(
                "run-progress",
                RunProgressEvent {
                    run_id: run_id.clone(),
                    seq: 0,
                    total: 0,
                    status: 0,
                    error: Some(startup_error),
                    finished: true,
                },
            );
        }
        if let Ok(mut runs) = active_runs_map.lock() {
            runs.remove(&run_id);
        }
    });

    Ok(run_id_ret)
}

/// Start a run from a persisted scenario definition.
#[tauri::command]
pub async fn start_scenario_run(
    app: AppHandle,
    active_runs: State<'_, ActiveRunsState>,
    config_state: State<'_, AppConfigState>,
    paths: State<'_, AppPaths>,
    logger: State<'_, LoggerState>,
    engagement_slug: String,
    scenario_id: String,
) -> Result<String, CommandError> {
    logger.0.info(
        "runner",
        None,
        &format!(
            "start_scenario_run requested for engagement={} scenario_id={}",
            engagement_slug, scenario_id
        ),
    );

    let scenario = match scenarios::load(&paths.0.scenarios_dir(), &scenario_id)? {
        Some(scenario) => scenario,
        None => {
            let message = format!("scenario '{}' not found", scenario_id);
            logger.0.error("runner", None, &message);
            return Err(anyhow::anyhow!(message).into());
        }
    };

    // A Scenario is always a matrix: `request_ids` × library subset fired
    // as a Cartesian product. The legacy step-based runner was retired in
    // Phase 2 of `docs/RefactorPlan.md`.
    if scenario.request_ids.is_empty() {
        let message =
            "scenario has no Requests configured — open it in the Scenarios view".to_owned();
        logger.0.error("runner", None, &message);
        return Err(anyhow::anyhow!(message).into());
    }
    if scenario.library.is_none() {
        let message =
            "scenario has no library subset configured — open it in the Scenarios view".to_owned();
        logger.0.error("runner", None, &message);
        return Err(anyhow::anyhow!(message).into());
    }

    dispatch_matrix_scenario(
        app,
        active_runs,
        config_state,
        paths,
        logger,
        engagement_slug,
        scenario,
    )
    .await
}

// ── Phase 2 matrix execution ──────────────────────────────────────────────────
//
// Dispatcher for matrix-mode scenarios (Phase 2 of `docs/RefactorPlan.md`).
// Loads the global Request registry, resolves the Scenario's library subset
// against `~/hamm0r/prompts/`, and fires `execute_matrix_run`. Called from
// `start_scenario_run` when the Scenario has `request_ids` and `library`
// populated.
async fn dispatch_matrix_scenario(
    app: AppHandle,
    active_runs: State<'_, ActiveRunsState>,
    config_state: State<'_, AppConfigState>,
    paths: State<'_, AppPaths>,
    logger: State<'_, LoggerState>,
    engagement_slug: String,
    scenario: storage::types::Scenario,
) -> Result<String, CommandError> {
    if scenario.request_ids.is_empty() {
        return Err(anyhow::anyhow!("matrix scenario has no request_ids").into());
    }
    let library = scenario
        .library
        .clone()
        .ok_or_else(|| anyhow::anyhow!("matrix scenario has no library subset configured"))?;

    // Build the registry from every Request on disk. The deps resolver
    // walks only what's reachable, so loading the whole set is fine.
    let registry: HashMap<String, Request> = requests::load_all(&paths.0.requests_dir())?;

    // Validate every target Request id exists.
    for entry in &scenario.request_ids {
        if !registry.contains_key(&entry.id) {
            return Err(anyhow::anyhow!(
                "matrix scenario references unknown Request '{}'",
                entry.id
            )
            .into());
        }
    }

    // Resolve the library subset to a list of payloads. A prompt entry
    // matches if its category (filename stem) is listed OR its
    // `owasp_ref` is in `owasp_refs`.
    let prompt_map = prompts::load_all(&paths.0.prompts_dir())?;
    let mut payloads: Vec<Payload> = Vec::new();
    for (category, entries) in &prompt_map {
        let category_match = library.categories.iter().any(|c| c == category);
        for entry in entries {
            let owasp_match = entry
                .owasp_ref
                .as_ref()
                .map(|r| library.owasp_refs.iter().any(|w| w == r))
                .unwrap_or(false);
            if !(category_match || owasp_match) {
                continue;
            }
            payloads.push(Payload {
                prompt_id: entry.id.clone(),
                payload_id: format!("{category}:{}", entry.id),
                text: entry.text.clone(),
                session: "default".to_owned(),
            });
        }
    }
    if payloads.is_empty() {
        return Err(anyhow::anyhow!(
            "library subset matched no prompts (categories={:?}, owasp_refs={:?})",
            library.categories,
            library.owasp_refs,
        )
        .into());
    }

    let run_id = next_run_id(&paths.0.engagement_dir(&engagement_slug))?;
    let engagement_dir = paths.0.engagement_dir(&engagement_slug);
    // Matrix runs are stateless at the session-injection layer for now;
    // bind sharing is the only inter-attempt state.
    let session_strategy = SessionStrategy::None;

    logger.0.info(
        "runner",
        Some(&run_id),
        &format!(
            "matrix scenario fired: scenario_id={} request_ids={} payload_count={} shared_session={}",
            scenario.id,
            scenario.request_ids.len(),
            payloads.len(),
            scenario.shared_session,
        ),
    );

    let cancellation = runner::run::RunCancellation::new();
    let config = MatrixRunConfig {
        engagement_dir,
        run_id: run_id.clone(),
        scenario_id: scenario.id.clone(),
        registry,
        request_ids: scenario.request_ids.iter().map(|e| e.id.clone()).collect(),
        per_request_repeat: scenario
            .request_ids
            .iter()
            .filter_map(|e| e.repeat.map(|r| (e.id.clone(), r)))
            .collect(),
        payloads,
        repeat: scenario.repeat.max(1),
        shared_session: scenario.shared_session,
        session_strategy,
        runner_version: env!("CARGO_PKG_VERSION").to_owned(),
        body_logging_enabled: config_state.0.logging.body_logging_enabled,
        cancellation: Some(cancellation.clone()),
        on_attempt_log: Some(Arc::new({
            let logger = logger.0.clone();
            let app = app.clone();
            let first_error_reported = Arc::new(AtomicBool::new(false));
            move |attempt| {
                log_attempt(&logger, attempt.clone());
                if let Some(error) = &attempt.error {
                    if !first_error_reported.swap(true, Ordering::SeqCst) {
                        let (scope, message) = build_attempt_error_event(
                            "matrix-run-attempt",
                            "Matrix run",
                            &attempt,
                            error,
                        );
                        emit_user_relevant_error(&app, &scope, Some(&attempt.run_id), &message);
                    }
                }
            }
        })),
    };

    let run_id_ret = run_id.clone();
    let logger = logger.0.clone();
    let active_runs_map = active_runs.0.clone();
    active_runs_map
        .lock()
        .map_err(|_| anyhow::anyhow!("active run registry poisoned"))?
        .insert(run_id.clone(), cancellation);

    let engagement_dir_for_error = paths.0.engagement_dir(&engagement_slug);
    tokio::spawn(async move {
        let app_for_progress = app.clone();
        let app_for_error = app;
        let result = execute_matrix_run(config, move |progress| {
            let event = RunProgressEvent {
                run_id: progress.run_id,
                seq: progress.seq,
                total: progress.total,
                status: progress.status,
                error: progress.error,
                finished: progress.finished,
            };
            let _ = app_for_progress.emit("run-progress", event);
        })
        .await;

        if let Err(e) = result {
            let startup_error = format!("matrix run execution failed: {e}");
            report_user_relevant_error(
                &app_for_error,
                &logger,
                "runner",
                "matrix-run-execution",
                Some(&run_id),
                &startup_error,
            );
            let _ = write_run_startup_error(&engagement_dir_for_error, &run_id, &startup_error);
            let _ = app_for_error.emit(
                "run-progress",
                RunProgressEvent {
                    run_id: run_id.clone(),
                    seq: 0,
                    total: 0,
                    status: 0,
                    error: Some(startup_error),
                    finished: true,
                },
            );
        }
        if let Ok(mut runs) = active_runs_map.lock() {
            runs.remove(&run_id);
        }
    });

    Ok(run_id_ret)
}

#[tauri::command]
pub fn stop_run(
    logger: State<'_, LoggerState>,
    active_runs: State<'_, ActiveRunsState>,
    engagement_slug: Option<String>,
    run_id: String,
) -> Result<StopRunResult, CommandError> {
    logger.0.info(
        "runner",
        Some(&run_id),
        &format!(
            "stop_run requested for engagement={}",
            engagement_slug.as_deref().unwrap_or("unknown")
        ),
    );

    let runs = active_runs
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("active run registry poisoned"))?;
    if let Some(cancellation) = runs.get(&run_id) {
        cancellation.cancel();
        logger
            .0
            .info("runner", Some(&run_id), "Run cancellation requested");
        Ok(StopRunResult { stopped: true })
    } else {
        logger.0.debug(
            "runner",
            Some(&run_id),
            "stop_run ignored because the run is no longer active",
        );
        Ok(StopRunResult { stopped: false })
    }
}

#[derive(Debug, Serialize)]
pub struct DeleteRunResult {
    pub deleted: bool,
    /// Number of filesystem entries (files + responses dir) actually removed.
    pub removed: usize,
}

/// Permanently delete a run's artifacts: run JSONL, verdicts JSONL, the
/// responses directory, and any generated report HTML. Refuses if the run
/// is currently active.
#[tauri::command]
pub fn delete_run(
    logger: State<'_, LoggerState>,
    paths: State<'_, AppPaths>,
    active_runs: State<'_, ActiveRunsState>,
    engagement_slug: String,
    run_id: String,
) -> Result<DeleteRunResult, CommandError> {
    logger.0.info(
        "runner",
        Some(&run_id),
        &format!("delete_run requested for engagement={engagement_slug}"),
    );

    {
        let active = active_runs
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("active run registry poisoned"))?;
        if active.contains_key(&run_id) {
            return Err(anyhow::anyhow!("Run is still active. Stop it first, then delete.").into());
        }
    }

    let engagement_dir = paths.0.engagement_dir(&engagement_slug);
    let removed = runs::delete_run(&engagement_dir, &run_id)?;

    logger.0.info(
        "runner",
        Some(&run_id),
        &format!("delete_run removed {removed} entries"),
    );

    Ok(DeleteRunResult {
        deleted: removed > 0,
        removed,
    })
}

/// Read attempt records from a run's JSONL file. Returns a JSON array of
/// attempt objects (headers and footers are omitted).
#[tauri::command]
pub fn read_run_attempts(
    logger: State<'_, LoggerState>,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
) -> Result<Vec<serde_json::Value>, CommandError> {
    logger.0.debug(
        "runner",
        Some(&run_id),
        &format!("read_run_attempts invoked for engagement={engagement_slug}"),
    );
    let run_path = paths
        .0
        .engagement_dir(&engagement_slug)
        .join("runs")
        .join(format!("{run_id}.jsonl"));

    if !run_path.exists() {
        return Ok(vec![]);
    }

    let records = read_all(&run_path)?;
    let attempts: Vec<serde_json::Value> = records
        .into_iter()
        .filter_map(|r| match r {
            RunRecord::Attempt(a) => serde_json::to_value(*a).ok(),
            _ => None,
        })
        .collect();
    logger.0.debug(
        "runner",
        Some(&run_id),
        &format!("read_run_attempts completed count={}", attempts.len()),
    );
    Ok(attempts)
}

/// Read the raw text of one response body file.
#[tauri::command]
pub fn read_response_body(
    logger: State<'_, LoggerState>,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
    seq: u32,
) -> Result<Option<String>, CommandError> {
    let engagement_dir = paths.0.engagement_dir(&engagement_slug);
    logger.0.debug(
        "runner",
        Some(&run_id),
        &format!("read_response_body invoked for seq={seq} engagement={engagement_slug}"),
    );
    storage::runs::read_response_body(&engagement_dir, &run_id, seq)
        .inspect(|body| {
            logger.0.debug(
                "runner",
                Some(&run_id),
                &format!("read_response_body completed has_body={}", body.is_some()),
            );
        })
        .map_err(|err| {
            logger.0.error(
                "runner",
                Some(&run_id),
                &format!("read_response_body failed for seq={seq}: {err}"),
            );
            CommandError::from(err)
        })
}

#[tauri::command]
pub fn get_run_diagnostics(
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
) -> Result<Option<RunDiagnostics>, CommandError> {
    let run_path = paths
        .0
        .engagement_dir(&engagement_slug)
        .join("runs")
        .join(format!("{run_id}.jsonl"));
    if !run_path.exists() {
        return Ok(None);
    }

    let records = read_all(&run_path)?;
    let mut attempts = 0u32;
    let mut has_footer = false;
    let mut started_at: Option<String> = None;
    let mut request_id: Option<String> = None;
    let mut notes: Vec<String> = Vec::new();

    for record in &records {
        match record {
            RunRecord::Header(h) => {
                started_at = Some(h.started_at.clone());
                request_id = Some(h.request_id.clone());
            }
            RunRecord::Attempt(_) => attempts += 1,
            RunRecord::Footer(_) => has_footer = true,
        }
    }

    let mut request_url: Option<String> = None;
    if let Some(req_id) = &request_id {
        let all_requests = requests::load_all(&paths.0.requests_dir())?;
        if let Some(request) = all_requests.get(req_id).cloned() {
            request_url = Some(request.url.clone());
            match request.auth {
                storage::types::AuthConfig::Bearer { token_env } => {
                    if storage::secrets::resolve_token(&token_env)?
                        .map(|v| v.trim().is_empty())
                        .unwrap_or(true)
                    {
                        notes.push(format!("Missing env var for bearer auth: {}", token_env));
                    }
                }
                storage::types::AuthConfig::CustomHeader { value_env, .. } => {
                    if storage::secrets::resolve_token(&value_env)?
                        .map(|v| v.trim().is_empty())
                        .unwrap_or(true)
                    {
                        notes.push(format!(
                            "Missing env var for API key/header auth: {}",
                            value_env
                        ));
                    }
                }
                storage::types::AuthConfig::Basic {
                    user_env,
                    password_env,
                } => {
                    if storage::secrets::resolve_token(&user_env)?
                        .map(|v| v.trim().is_empty())
                        .unwrap_or(true)
                    {
                        notes.push(format!("Missing env var for basic auth user: {}", user_env));
                    }
                    if storage::secrets::resolve_token(&password_env)?
                        .map(|v| v.trim().is_empty())
                        .unwrap_or(true)
                    {
                        notes.push(format!(
                            "Missing env var for basic auth password: {}",
                            password_env
                        ));
                    }
                }
                storage::types::AuthConfig::None => {}
            }
        } else {
            notes.push(format!(
                "Request mapping not found for request_id '{}'.",
                req_id
            ));
        }
    }

    if attempts == 0 && !has_footer {
        notes.push(
            "No attempt record written yet. The run likely failed before first response."
                .to_owned(),
        );
    }
    if let Some(startup_err) =
        read_run_startup_error(&paths.0.engagement_dir(&engagement_slug), &run_id)
    {
        notes.insert(0, startup_err);
    }

    let updated_at = std::fs::metadata(&run_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|ts| ts.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
        .map(|d| format!("unix:{}", d.as_secs()));

    let status = if has_footer {
        "finished"
    } else if attempts > 0 {
        "active"
    } else {
        "starting"
    }
    .to_owned();

    Ok(Some(RunDiagnostics {
        run_id,
        status,
        request_id,
        request_url,
        attempts,
        has_footer,
        started_at,
        updated_at,
        notes,
    }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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

fn log_attempt(logger: &crate::logger::AppLogger, attempt: AttemptLog) {
    let mut lines = vec![
        format!("Attempt completed seq={}", attempt.seq),
        format!("request.method={}", attempt.request_method),
        format!("request.url={}", attempt.request_url),
        format!(
            "request.headers={}",
            format_headers(&attempt.request_headers)
        ),
        format!("request.body_size={}", attempt.request_body_size),
        format!("response.status={}", attempt.response_status),
        format!(
            "response.headers={}",
            format_headers(&attempt.response_headers)
        ),
        format!("response.body_size={}", attempt.response_body_size),
        format!("duration_ms={}", attempt.duration_ms),
    ];

    if let Some(error) = &attempt.error {
        lines.push(format!("error={error}"));
    }
    if let Some(body) = &attempt.request_body {
        lines.push("request.body:".to_owned());
        lines.push(body.clone());
    }
    if let Some(body) = &attempt.response_body {
        lines.push("response.body:".to_owned());
        lines.push(body.clone());
    }

    logger.info("runner", Some(&attempt.run_id), &lines.join("\n"));
}

fn build_attempt_error_event(
    scope_prefix: &str,
    label: &str,
    attempt: &AttemptLog,
    error: &str,
) -> (String, String) {
    if attempt.is_timeout {
        (
            format!("{scope_prefix}-timeout"),
            format!(
                "{label} {} timed out on attempt {}: {}",
                attempt.run_id, attempt.seq, error
            ),
        )
    } else {
        (
            scope_prefix.to_owned(),
            format!(
                "{label} {} hit a request error on attempt {}: {}",
                attempt.run_id, attempt.seq, error
            ),
        )
    }
}

fn format_headers(headers: &std::collections::HashMap<String, String>) -> String {
    let mut pairs = headers
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>();
    pairs.sort();
    pairs.join(", ")
}

fn run_startup_error_path(engagement_dir: &std::path::Path, run_id: &str) -> std::path::PathBuf {
    engagement_dir
        .join("runs")
        .join(format!("{run_id}.startup-error.txt"))
}

fn write_run_startup_error(
    engagement_dir: &std::path::Path,
    run_id: &str,
    message: &str,
) -> anyhow::Result<()> {
    let path = run_startup_error_path(engagement_dir, run_id);
    std::fs::write(path, message).map_err(|e| anyhow::anyhow!(e))
}

fn read_run_startup_error(engagement_dir: &std::path::Path, run_id: &str) -> Option<String> {
    let path = run_startup_error_path(engagement_dir, run_id);
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_attempt(timeout: bool) -> AttemptLog {
        AttemptLog {
            run_id: "run-001".to_owned(),
            seq: 7,
            request_method: "POST".to_owned(),
            request_url: "http://example.test".to_owned(),
            request_headers: std::collections::HashMap::new(),
            request_body_size: 0,
            request_body: None,
            response_status: 0,
            response_headers: std::collections::HashMap::new(),
            response_body_size: 0,
            response_body: None,
            duration_ms: 1000,
            error: Some("boom".to_owned()),
            is_timeout: timeout,
        }
    }

    #[test]
    fn build_attempt_error_event_marks_timeout_scope() {
        let attempt = sample_attempt(true);
        let (scope, message) = build_attempt_error_event("run-attempt", "Run", &attempt, "boom");
        assert_eq!(scope, "run-attempt-timeout");
        assert!(message.contains("timed out on attempt 7"));
    }

    #[test]
    fn build_attempt_error_event_marks_regular_scope() {
        let attempt = sample_attempt(false);
        let (scope, message) = build_attempt_error_event("run-attempt", "Run", &attempt, "boom");
        assert_eq!(scope, "run-attempt");
        assert!(message.contains("hit a request error on attempt 7"));
    }
}
