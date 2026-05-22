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
    /// Request id of the attempt this event describes. `None` for the
    /// terminal "run finished" event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Prompt id (library entry) used. `None` for the terminal event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
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
            mutation_id: Some("seed".to_owned()),
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
        replay_of: None,
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
        logger.info(
            "runner",
            Some(&run_id),
            &format!("Run task spawned for url={request_url}"),
        );
        let result = execute_run(config, run_progress_emitter(app.clone())).await;
        finalize_run_task(
            result,
            &app,
            &logger,
            &active_runs_map,
            &engagement_dir_for_error,
            &run_id,
            &format!("run execution failed (url: {request_url})"),
            "run-execution",
        );
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

    // Persist the scenario binding into engagement.yaml so the Run button
    // works on subsequent visits (even before the run header is readable).
    // Failure here is non-fatal — the run can still fire; only the
    // "default scenario" hint on the engagement is missed.
    if let Ok(Some(mut meta)) =
        storage::engagements::load(&paths.0.engagements_dir(), &engagement_slug)
    {
        if meta.target.scenario_id != scenario.id {
            meta.target.scenario_id = scenario.id.clone();
            if let Err(e) = storage::engagements::save_meta(&paths.0.engagements_dir(), &meta) {
                logger.0.error(
                    "runner",
                    None,
                    &format!("could not persist scenario binding: {e}"),
                );
            }
        }
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
    let mut seed_payloads: Vec<Payload> = Vec::new();
    // Section 1.4 of docs/ToDo.md: keep each prompt's phase tag
    // alongside the payload list so the multi-session dispatch below
    // can schedule plant before probe. Single-session runs ignore this.
    let mut phase_by_payload_id: std::collections::HashMap<String, storage::types::Phase> =
        std::collections::HashMap::new();
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
            let payload_id = format!("{category}:{}", entry.id);
            phase_by_payload_id.insert(payload_id.clone(), entry.phase);
            seed_payloads.push(Payload {
                prompt_id: entry.id.clone(),
                payload_id,
                text: entry.text.clone(),
                session: "default".to_owned(),
                mutation_id: Some("seed".to_owned()),
            });
        }
    }
    if seed_payloads.is_empty() {
        return Err(anyhow::anyhow!(
            "library subset matched no prompts (categories={:?}, owasp_refs={:?})",
            library.categories,
            library.owasp_refs,
        )
        .into());
    }

    // Section 2.8: expand each seed prompt through the configured mutation
    // engine. The seed itself is always included; mutators contribute extra
    // variants in registry order. The final attempt list is seed_count ×
    // (1 + mutations) × requests × repeat (per docs/ToDo.md §2.8).
    let payloads: Vec<Payload> = if let Some(mutations) = scenario.mutations.as_ref() {
        let enabled = &mutations.enabled_mutators;
        let cap = mutations.max_variants_per_seed;
        if enabled.is_empty() {
            seed_payloads
        } else {
            let mut expanded: Vec<Payload> = Vec::with_capacity(seed_payloads.len());
            for seed in &seed_payloads {
                for variant in runner::mutation::expand_seed(&seed.text, enabled, cap) {
                    let mutation_suffix = if variant.mutation_id == "seed" {
                        String::new()
                    } else {
                        format!(":{}", variant.mutation_id)
                    };
                    expanded.push(Payload {
                        prompt_id: seed.prompt_id.clone(),
                        payload_id: format!("{}{}", seed.payload_id, mutation_suffix),
                        text: variant.text,
                        session: seed.session.clone(),
                        mutation_id: Some(variant.mutation_id),
                    });
                }
            }
            expanded
        }
    } else {
        seed_payloads
    };

    let run_id = next_run_id(&paths.0.engagement_dir(&engagement_slug))?;
    let engagement_dir = paths.0.engagement_dir(&engagement_slug);

    // Section 1 of docs/ToDo.md: if the scenario opted into multi-
    // session (session_count > 1) fire through the multi-session runner.
    // The multi-session path schedules plants → probes across N
    // isolated clients and runs the leak scanner. Single-session
    // scenarios (session_count absent or 1) fall through to the
    // existing matrix flow below — backward compatible.
    if scenario.session_count.unwrap_or(1) > 1 {
        let session_count = scenario.session_count.unwrap_or(1);
        let identity =
            scenario
                .session_identity
                .clone()
                .unwrap_or(storage::types::SessionIdentityConfig {
                    kind: storage::types::SessionIdentityKind::CookieJar,
                });
        let phased_prompts: Vec<runner::PhasedPayload> = payloads
            .iter()
            .map(|p| runner::PhasedPayload {
                prompt_id: p.prompt_id.clone(),
                payload_id: p.payload_id.clone(),
                text: p.text.clone(),
                phase: phase_by_payload_id
                    .get(&p.payload_id)
                    .copied()
                    .unwrap_or(storage::types::Phase::Any),
            })
            .collect();

        logger.0.info(
            "runner",
            Some(&run_id),
            &format!(
                "multi-session scenario fired: scenario_id={} session_count={} prompts={} request_ids={}",
                scenario.id,
                session_count,
                phased_prompts.len(),
                scenario.request_ids.len(),
            ),
        );

        let cancellation = runner::run::RunCancellation::new();
        let config = runner::MultiSessionRunConfig {
            engagement_dir: engagement_dir.clone(),
            run_id: run_id.clone(),
            scenario_id: scenario.id.clone(),
            registry,
            request_ids: scenario.request_ids.iter().map(|e| e.id.clone()).collect(),
            per_request_repeat: scenario
                .request_ids
                .iter()
                .filter_map(|e| e.repeat.map(|r| (e.id.clone(), r)))
                .collect(),
            prompts: phased_prompts,
            repeat: scenario.repeat.max(1),
            session_count,
            session_identity: identity,
            runner_version: env!("CARGO_PKG_VERSION").to_owned(),
            body_logging_enabled: config_state.0.logging.body_logging_enabled,
            on_attempt_log: None,
            cancellation: Some(cancellation.clone()),
        };

        let run_id_ret = run_id.clone();
        let logger = logger.0.clone();
        let active_runs_map = active_runs.0.clone();
        active_runs_map
            .lock()
            .map_err(|_| anyhow::anyhow!("active run registry poisoned"))?
            .insert(run_id.clone(), cancellation);

        let engagement_dir_for_error = engagement_dir.clone();
        tokio::spawn(async move {
            let result =
                runner::execute_multi_session_run(config, run_progress_emitter(app.clone())).await;
            finalize_run_task(
                result,
                &app,
                &logger,
                &active_runs_map,
                &engagement_dir_for_error,
                &run_id,
                "multi-session run execution failed",
                "multi-session-run-execution",
            );
        });

        return Ok(run_id_ret);
    }

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
        let result = execute_matrix_run(config, run_progress_emitter(app.clone())).await;
        finalize_run_task(
            result,
            &app,
            &logger,
            &active_runs_map,
            &engagement_dir_for_error,
            &run_id,
            "matrix run execution failed",
            "matrix-run-execution",
        );
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

/// Re-fire a single past attempt with an optional tweaked prompt.
///
/// Writes the result to a sibling run file `<run_id>-replay-<n>.jsonl`
/// so the original run remains immutable (CLAUDE.md invariant #12).
/// See `docs/Datamodel.md §"Replay run files"`. Returns the replay
/// run_id immediately; progress events fire via `run-progress` as the
/// single attempt completes.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn replay_attempt(
    app: AppHandle,
    active_runs: State<'_, ActiveRunsState>,
    config_state: State<'_, AppConfigState>,
    paths: State<'_, AppPaths>,
    logger: State<'_, LoggerState>,
    engagement_slug: String,
    run_id: String,
    seq: u32,
    prompt_override: Option<String>,
) -> Result<String, CommandError> {
    let engagement_dir = paths.0.engagement_dir(&engagement_slug);
    let original_path = engagement_dir.join("runs").join(format!("{run_id}.jsonl"));
    if !original_path.exists() {
        return Err(anyhow::anyhow!("run '{run_id}' not found").into());
    }

    // Locate the source attempt.
    let records = read_all(&original_path)?;
    let original = records
        .iter()
        .find_map(|r| match r {
            RunRecord::Attempt(a) if a.seq == seq => Some(a.clone()),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("seq {seq} not found in {run_id}"))?;

    // Resolve which Request template fired this attempt. Prefer the
    // explicit field; fall back to URL+method match (legacy logs).
    let registry = requests::load_all(&paths.0.requests_dir())?;
    let request: Request = match &original.request_id {
        Some(id) => registry.get(id).cloned().ok_or_else(|| {
            anyhow::anyhow!("Request '{id}' referenced by original attempt no longer exists")
        })?,
        None => {
            let m = original.request.method.to_ascii_uppercase();
            let u = original.request.url.trim_end_matches('/').to_owned();
            registry
                .values()
                .find(|r| {
                    r.method.to_ascii_uppercase() == m
                        && r.url.trim_end_matches('/') == u
                })
                .cloned()
                .ok_or_else(|| anyhow::anyhow!(
                    "no Request matches the original attempt's url+method (legacy log without request_id)"
                ))?
        }
    };

    let prompt_overridden = prompt_override.is_some();
    let prompt_text = prompt_override
        .or_else(|| original.prompt_text.clone())
        .unwrap_or_default();

    let replay_run_id = next_replay_run_id(&engagement_dir, &run_id)?;
    let cancellation = runner::run::RunCancellation::new();
    let active_runs_map = active_runs.0.clone();
    active_runs_map
        .lock()
        .map_err(|_| anyhow::anyhow!("active run registry poisoned"))?
        .insert(replay_run_id.clone(), cancellation.clone());

    let config = RunConfig {
        engagement_dir: engagement_dir.clone(),
        run_id: replay_run_id.clone(),
        request,
        payloads: vec![Payload {
            prompt_id: original.prompt_id.clone(),
            payload_id: format!("replay:{run_id}:{seq}"),
            text: prompt_text,
            session: original.session.clone().unwrap_or_else(|| "default".into()),
            mutation_id: original.mutation_id.clone(),
        }],
        parallelism: 1,
        runner_version: env!("CARGO_PKG_VERSION").to_owned(),
        body_logging_enabled: config_state.0.logging.body_logging_enabled,
        cancellation: Some(cancellation),
        replay_of: Some(storage::runs::ReplaySource {
            run_id: run_id.clone(),
            seq,
            prompt_overridden,
        }),
        on_attempt_log: None,
    };

    let engagement_dir_for_error = engagement_dir.clone();
    let replay_run_id_ret = replay_run_id.clone();
    let logger = logger.0.clone();

    logger.info(
        "runner",
        Some(&replay_run_id),
        &format!("replay_attempt source=run={run_id} seq={seq} override={prompt_overridden}"),
    );

    tokio::spawn(async move {
        let result = execute_run(config, run_progress_emitter(app.clone())).await;
        finalize_run_task(
            result,
            &app,
            &logger,
            &active_runs_map,
            &engagement_dir_for_error,
            &replay_run_id,
            "replay execution failed",
            "replay-execution",
        );
    });

    Ok(replay_run_id_ret)
}

/// List replay files for a given original run. Returns each replay's
/// run_id (e.g. `run-003-replay-1`) so the UI can load them on demand.
#[tauri::command]
pub fn list_replays(
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
) -> Result<Vec<String>, CommandError> {
    let runs_dir = paths.0.engagement_dir(&engagement_slug).join("runs");
    if !runs_dir.exists() {
        return Ok(vec![]);
    }
    let prefix = format!("{run_id}-replay-");
    let mut found: Vec<String> = std::fs::read_dir(&runs_dir)
        .map_err(|e| anyhow::anyhow!(e))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy().into_owned();
            if let Some(rest) = s.strip_suffix(".jsonl") {
                if rest.starts_with(&prefix) {
                    return Some(rest.to_owned());
                }
            }
            None
        })
        .collect();
    // Sort by trailing index numerically so replay-2 follows replay-1 etc.
    found.sort_by_key(|id| {
        id.rsplit('-')
            .next()
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0)
    });
    Ok(found)
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

/// Section 1.8 of docs/ToDo.md. Return every `LeakDetected` record
/// from a run's JSONL so the engagement results UI can badge probe
/// rows whose seq was leaked into.
#[tauri::command]
pub fn read_run_leaks(
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
    let leaks: Vec<serde_json::Value> = records
        .into_iter()
        .filter_map(|r| match r {
            RunRecord::LeakDetected(l) => serde_json::to_value(l).ok(),
            _ => None,
        })
        .collect();
    Ok(leaks)
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
            RunRecord::LeakDetected(_) => {}
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

/// Build a `Fn(RunProgress)` closure that wraps each runner-side progress
/// notification in a `RunProgressEvent` and emits it to the UI via the
/// `run-progress` channel. Used by every run-dispatch path.
fn run_progress_emitter(
    app: AppHandle,
) -> impl Fn(runner::run::RunProgress) + Send + Sync + 'static {
    move |progress| {
        let event = RunProgressEvent {
            run_id: progress.run_id,
            seq: progress.seq,
            total: progress.total,
            status: progress.status,
            error: progress.error,
            finished: progress.finished,
            request_id: progress.request_id,
            prompt_id: progress.prompt_id,
        };
        let _ = app.emit("run-progress", event);
    }
}

/// Common tail for every spawned run task: on failure, surface the error to
/// the user (log + toast + persisted startup-error file + terminal progress
/// event); always remove the run from the active-runs registry.
#[allow(clippy::too_many_arguments)]
fn finalize_run_task(
    result: Result<(), runner::RunnerError>,
    app: &AppHandle,
    logger: &crate::logger::AppLogger,
    active_runs_map: &std::sync::Arc<
        std::sync::Mutex<HashMap<String, runner::run::RunCancellation>>,
    >,
    engagement_dir_for_error: &std::path::Path,
    run_id: &str,
    error_label: &str,
    error_scope: &str,
) {
    if let Err(e) = result {
        let startup_error = format!("{error_label}: {e}");
        report_user_relevant_error(
            app,
            logger,
            "runner",
            error_scope,
            Some(run_id),
            &startup_error,
        );
        let _ = write_run_startup_error(engagement_dir_for_error, run_id, &startup_error);
        let _ = app.emit(
            "run-progress",
            RunProgressEvent {
                run_id: run_id.to_owned(),
                seq: 0,
                total: 0,
                status: 0,
                error: Some(startup_error),
                finished: true,
                request_id: None,
                prompt_id: None,
            },
        );
    }
    if let Ok(mut runs) = active_runs_map.lock() {
        runs.remove(run_id);
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

/// Find the next available `<original>-replay-N` id. N starts at 1 and
/// increments past any existing replay files for the same original run.
fn next_replay_run_id(
    engagement_dir: &std::path::Path,
    original_run_id: &str,
) -> anyhow::Result<String> {
    let runs_dir = engagement_dir.join("runs");
    if !runs_dir.exists() {
        return Ok(format!("{original_run_id}-replay-1"));
    }
    let prefix = format!("{original_run_id}-replay-");
    let max_n = std::fs::read_dir(&runs_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy().into_owned();
            s.strip_prefix(&prefix)
                .and_then(|r| r.strip_suffix(".jsonl"))
                .and_then(|n| n.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);
    Ok(format!("{original_run_id}-replay-{}", max_n + 1))
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
