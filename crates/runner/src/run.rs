use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use bytes::Bytes;
use storage::runs::{
    RequestEnvelope, ResponseEnvelope, RunAttempt, RunFooter, RunHeader, RunRecord, RunStatus,
    Timing,
};
#[cfg(test)]
use storage::types::AuthConfig;
use storage::types::{BodyFormat, Request, SessionConfig};
use storage::{atomic_write, runs};
use tokio::sync::Notify;
use tokio::sync::Semaphore;

use crate::adapter::{self, AdapterResponse};
use crate::error::RunnerError;
use crate::session::{SessionManager, SessionStrategy};

// ── Public API ────────────────────────────────────────────────────────────────

/// One payload to fire in a run.
#[derive(Debug, Clone)]
pub struct Payload {
    pub prompt_id: String,
    pub payload_id: String,
    pub text: String,
    /// Session label — steps sharing a label share auth/cookie state.
    pub session: String,
}

/// Configuration for a single run.
pub struct RunConfig {
    pub engagement_dir: PathBuf,
    pub run_id: String,
    pub request: Request,
    pub payloads: Vec<Payload>,
    pub parallelism: usize,
    pub runner_version: String,
    pub body_logging_enabled: bool,
    pub on_attempt_log: Option<Arc<dyn Fn(AttemptLog) + Send + Sync>>,
    pub cancellation: Option<RunCancellation>,
}

/// Configuration for matrix execution (Phase 2C of `docs/RefactorPlan.md`).
///
/// Fires `request_ids` × `payloads` as a Cartesian product: each prompt
/// against each Request, with auth-chain prerequisites resolved via the
/// `deps` module. This is the only Scenario execution path — the legacy
/// step-based runner was retired in Phase 2 (see `RefactorPlan.md`).
pub struct MatrixRunConfig {
    pub engagement_dir: PathBuf,
    pub run_id: String,
    /// Source Scenario id, recorded in the run header so the UI can map
    /// run → scenario name later.
    pub scenario_id: String,
    /// Every Request reachable from the matrix — targets and any
    /// prerequisites referenced via `{{<id>.<bind>}}`. The DAG resolver
    /// walks this map.
    pub registry: HashMap<String, Request>,
    /// Target Request ids to fire (each one against every payload).
    pub request_ids: Vec<String>,
    /// Per-request repeat multipliers, keyed by request id. A request whose
    /// id is absent here defaults to 1. Applied on top of `repeat`:
    /// total firings for a request = `repeat` × `per_request_repeat[id]`.
    pub per_request_repeat: HashMap<String, u32>,
    /// Prompts (already resolved from a library subset upstream).
    pub payloads: Vec<Payload>,
    /// Number of independent passes over the request × payload matrix.
    pub repeat: u32,
    /// True: one bind cache shared across the whole run, so auth-chain
    /// prerequisites fire once per run. False: fresh cache per cell.
    pub shared_session: bool,
    pub session_strategy: SessionStrategy,
    pub runner_version: String,
    pub body_logging_enabled: bool,
    pub on_attempt_log: Option<Arc<dyn Fn(AttemptLog) + Send + Sync>>,
    pub cancellation: Option<RunCancellation>,
}

/// Progress notification emitted after each attempt completes.
#[derive(Debug, Clone)]
pub struct RunProgress {
    pub run_id: String,
    pub seq: u32,
    pub total: u32,
    pub status: u16,
    pub error: Option<String>,
    pub finished: bool,
}

#[derive(Debug, Clone)]
pub struct AttemptLog {
    pub run_id: String,
    pub seq: u32,
    pub request_method: String,
    pub request_url: String,
    pub request_headers: HashMap<String, String>,
    pub request_body_size: u64,
    pub request_body: Option<String>,
    pub response_status: u16,
    pub response_headers: HashMap<String, String>,
    pub response_body_size: u64,
    pub response_body: Option<String>,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub is_timeout: bool,
}

#[derive(Clone, Debug, Default)]
pub struct RunCancellation {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl RunCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    async fn until_cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        self.notify.notified().await;
    }
}

// ── Internal types ────────────────────────────────────────────────────────────

/// Result of one HTTP exchange, decoded from the adapter response.
struct RequestOutcome {
    status: u16,
    error: Option<String>,
    is_timeout: bool,
    response_headers: std::collections::HashMap<String, String>,
    body_bytes: Option<Bytes>,
    first_byte_ms: Option<u64>,
    duration_ms: u64,
}

impl RequestOutcome {
    fn from_adapter(result: Result<AdapterResponse, RunnerError>, elapsed_ms: u64) -> Self {
        match result {
            Ok(r) => Self {
                status: r.status,
                error: None,
                is_timeout: false,
                response_headers: r.response_headers,
                body_bytes: Some(r.body_bytes),
                first_byte_ms: r.first_byte_ms,
                duration_ms: r.duration_ms,
            },
            Err(e) => Self {
                status: 0,
                error: Some(e.to_string()),
                is_timeout: matches!(&e, RunnerError::Http(inner) if inner.is_timeout()),
                response_headers: std::collections::HashMap::new(),
                body_bytes: None,
                first_byte_ms: None,
                duration_ms: elapsed_ms,
            },
        }
    }
}

/// Identity fields for one attempt record, separate from HTTP outcome and run context.
struct AttemptMeta {
    seq: u32,
    prompt_id: String,
    payload_id: String,
    step_id: Option<String>,
    iteration: Option<u32>,
    session: Option<String>,
    prompt_text: Option<String>,
    /// Phase 2 of docs/RefactorPlan.md: tags non-user-facing attempts.
    /// `Some("prerequisite")` for auth-chain prerequisite firings.
    kind: Option<String>,
}

struct RequestLogPayload {
    headers: HashMap<String, String>,
    body_size: u64,
    body_text: Option<String>,
}

enum AttemptTaskResult {
    Written { failed: bool },
    Cancelled,
}

// ── Execution ─────────────────────────────────────────────────────────────────

/// Execute a full run: fire all payloads, write a JSONL line per attempt,
/// emit progress after each one. Returns when all payloads are done.
///
/// Payloads execute in parallel up to `config.parallelism`. The JSONL file
/// is append-only with one fsync per line.
pub async fn execute_run<F>(config: RunConfig, on_progress: F) -> Result<(), RunnerError>
where
    F: Fn(RunProgress) + Send + Sync + 'static,
{
    let total = config.payloads.len() as u32;
    let run_path = config
        .engagement_dir
        .join("runs")
        .join(format!("{}.jsonl", config.run_id));
    let responses_dir = config.engagement_dir.join("responses").join(&config.run_id);

    std::fs::create_dir_all(&responses_dir).map_err(|e| anyhow::anyhow!(e))?;
    write_header(
        &run_path,
        &config.run_id,
        &config.engagement_dir,
        &config.request.id,
        &config.runner_version,
        config.payloads.iter().map(|p| p.prompt_id.clone()),
        None,
    )?;

    let sem = Arc::new(Semaphore::new(config.parallelism));
    let on_progress = Arc::new(on_progress);
    let on_attempt_log = config.on_attempt_log.clone();
    let request = Arc::new(config.request);
    let run_id = Arc::new(config.run_id.clone());
    let run_path = Arc::new(run_path);
    let responses_dir = Arc::new(responses_dir);
    let body_logging_enabled = config.body_logging_enabled;
    let cancellation = config.cancellation.clone();

    // Build the HTTP client once per run and share it across all attempts.
    // reqwest::Client is cheap to clone and owns the connection pool + DNS
    // cache + TLS session cache, so reusing it across N payloads avoids N
    // TLS handshakes and N pool warm-ups.
    let http = crate::client::build_http_client(request.timeout_seconds)?;

    let mut handles = Vec::new();

    for (seq_0, payload) in config.payloads.into_iter().enumerate() {
        let seq = (seq_0 + 1) as u32;
        let permit = sem.clone().acquire_owned().await.unwrap();
        let request = Arc::clone(&request);
        let run_id = Arc::clone(&run_id);
        let run_path = Arc::clone(&run_path);
        let responses_dir = Arc::clone(&responses_dir);
        let on_progress = Arc::clone(&on_progress);
        let on_attempt_log = on_attempt_log.clone();
        let cancellation = cancellation.clone();
        let http = http.clone();

        let handle = tokio::spawn(async move {
            let _permit = permit;

            if cancellation
                .as_ref()
                .is_some_and(RunCancellation::is_cancelled)
            {
                return Ok::<_, RunnerError>(AttemptTaskResult::Cancelled);
            }

            let sent_at = iso_now();
            let send_time = Instant::now();

            let result = if let Some(cancel) = &cancellation {
                tokio::select! {
                    _ = cancel.until_cancelled() => {
                        return Ok(AttemptTaskResult::Cancelled);
                    }
                    result = adapter::execute_with_session(
                        &http,
                        &request,
                        &payload.text,
                        &SessionStrategy::None,
                        &payload.session,
                    ) => result
                }
            } else {
                adapter::execute_with_session(
                    &http,
                    &request,
                    &payload.text,
                    &SessionStrategy::None,
                    &payload.session,
                )
                .await
            };

            let received_at = iso_now();
            let outcome =
                RequestOutcome::from_adapter(result, send_time.elapsed().as_millis() as u64);
            let request_log = render_request_for_log(
                &request,
                &payload.text,
                &SessionStrategy::None,
                &payload.session,
                body_logging_enabled,
            );
            let progress_status = outcome.status;
            let progress_error = outcome.error.clone();

            let meta = AttemptMeta {
                seq,
                prompt_id: payload.prompt_id,
                payload_id: payload.payload_id,
                step_id: None,
                iteration: None,
                session: Some(payload.session),
                prompt_text: Some(payload.text),
                kind: None,
            };

            let response_status = outcome.status;
            let response_headers = outcome.response_headers.clone();
            let response_body_size = outcome
                .body_bytes
                .as_ref()
                .map(|b| b.len() as u64)
                .unwrap_or(0);
            let response_body =
                render_response_body_for_log(outcome.body_bytes.as_ref(), body_logging_enabled);

            let attempt = build_attempt(
                meta,
                &outcome,
                &run_id,
                &request,
                sent_at,
                received_at,
                &responses_dir,
            )?;
            runs::append(&run_path, &attempt).map_err(|e| anyhow::anyhow!(e))?;

            if let Some(on_attempt_log) = &on_attempt_log {
                let (request_headers, request_body_size, request_body) = match &request_log {
                    Ok(log) => (log.headers.clone(), log.body_size, log.body_text.clone()),
                    Err(_) => (HashMap::new(), 0, None),
                };
                on_attempt_log(AttemptLog {
                    run_id: run_id.as_ref().clone(),
                    seq,
                    request_method: request.method.clone(),
                    request_url: request.url.clone(),
                    request_headers,
                    request_body_size,
                    request_body,
                    response_status,
                    response_headers,
                    response_body_size,
                    response_body,
                    duration_ms: outcome.duration_ms,
                    error: progress_error.clone(),
                    is_timeout: outcome.is_timeout,
                });
            }

            on_progress(RunProgress {
                run_id: run_id.as_ref().clone(),
                seq,
                total,
                status: progress_status,
                error: progress_error,
                finished: false,
            });

            Ok::<_, RunnerError>(AttemptTaskResult::Written {
                failed: outcome.error.is_some(),
            })
        });

        handles.push(handle);
    }

    let mut attempts_failed = 0u32;
    let mut attempts_written = 0u32;
    for handle in handles {
        match handle.await {
            Ok(Ok(AttemptTaskResult::Written { failed })) => {
                attempts_written += 1;
                if failed {
                    attempts_failed += 1;
                }
            }
            Ok(Ok(AttemptTaskResult::Cancelled)) => {}
            Ok(Err(_)) | Err(_) => attempts_failed += 1,
        }
    }

    let cancelled = cancellation
        .as_ref()
        .is_some_and(RunCancellation::is_cancelled);
    let status = if cancelled {
        RunStatus::AbortedByUser
    } else {
        RunStatus::Completed
    };
    let attempts_total = if cancelled { attempts_written } else { total };

    write_footer(
        &run_path,
        &config.run_id,
        attempts_total,
        attempts_failed,
        status,
    )?;

    on_progress(RunProgress {
        run_id: config.run_id,
        seq: attempts_written,
        total: attempts_total,
        status: 0,
        error: None,
        finished: true,
    });

    Ok(())
}

// ── Matrix execution (Phase 2C) ───────────────────────────────────────────────

/// Execute a matrix run: every payload × every Request, sequentially,
/// with auth-chain prerequisites resolved via `deps::fire_chain`.
///
/// Iteration order is **prompts outer × requests inner** so a single
/// prompt's behavior across all Request shapes is contiguous in the
/// run log — that's the way most users want to read results
/// ("did this attack break any of these endpoints?").
pub async fn execute_matrix_run<F>(
    config: MatrixRunConfig,
    on_progress: F,
) -> Result<(), RunnerError>
where
    F: Fn(RunProgress) + Send + Sync + 'static,
{
    let run_path = config
        .engagement_dir
        .join("runs")
        .join(format!("{}.jsonl", config.run_id));
    let responses_dir = config.engagement_dir.join("responses").join(&config.run_id);
    std::fs::create_dir_all(&responses_dir).map_err(|e| anyhow::anyhow!(e))?;

    // Header references the first target Request id by convention. Matrix
    // runs cover multiple Requests, but the JSONL header schema requires a
    // single string — picking the first target gives the right hint.
    let primary_request_id = config
        .request_ids
        .first()
        .cloned()
        .unwrap_or_else(|| "matrix".to_owned());

    let prompt_files: Vec<String> = {
        let mut seen = HashSet::new();
        config
            .payloads
            .iter()
            .filter(|p| seen.insert(p.prompt_id.clone()))
            .map(|p| p.prompt_id.clone())
            .collect()
    };

    let scenario_id = if config.scenario_id.trim().is_empty() {
        None
    } else {
        Some(config.scenario_id.clone())
    };
    write_header(
        &run_path,
        &config.run_id,
        &config.engagement_dir,
        &primary_request_id,
        &config.runner_version,
        prompt_files,
        scenario_id,
    )?;

    // User-facing total counts only target attempts (one per cell). The
    // prerequisite firings are bookkeeping — they get their own RunAttempt
    // records but don't count toward "X of N done".
    let repeat = config.repeat.max(1);
    // Sum of per-request repeats across all target requests. A request absent
    // from the map contributes 1 (the global repeat handles the rest).
    let request_repeat_sum: u32 = config
        .request_ids
        .iter()
        .map(|id| {
            config
                .per_request_repeat
                .get(id)
                .copied()
                .unwrap_or(1)
                .max(1)
        })
        .sum();
    let target_total = (config.payloads.len() as u32)
        .saturating_mul(request_repeat_sum)
        .saturating_mul(repeat);

    let mut session_manager = SessionManager::new(
        config.session_strategy.clone(),
        // Use the longest timeout among targets so we can support per-Request
        // overrides. Matrix-mode shares one client across cells.
        config
            .request_ids
            .iter()
            .filter_map(|id| config.registry.get(id).map(|r| r.timeout_seconds))
            .max()
            .unwrap_or(30),
    );

    // Bind cache strategy: shared across all cells when shared_session,
    // freshly created per cell otherwise. Held on the stack; sequential
    // execution means no synchronization.
    let mut shared_cache = if config.shared_session {
        Some(crate::template::BindCache::new())
    } else {
        None
    };

    let mut seq: u32 = 0;
    let mut target_seq: u32 = 0;
    let mut attempts_failed: u32 = 0;
    let on_attempt_log = config.on_attempt_log.clone();
    let cancellation = config.cancellation.clone();

    'outer: for iteration in 1..=repeat {
        for payload in &config.payloads {
            for request_id in &config.request_ids {
                let local_repeat = config
                    .per_request_repeat
                    .get(request_id)
                    .copied()
                    .unwrap_or(1)
                    .max(1);
                for _local_iter in 0..local_repeat {
                    if cancellation
                        .as_ref()
                        .is_some_and(RunCancellation::is_cancelled)
                    {
                        write_footer(
                            &run_path,
                            &config.run_id,
                            seq,
                            attempts_failed,
                            RunStatus::AbortedByUser,
                        )?;
                        on_progress(RunProgress {
                            run_id: config.run_id.clone(),
                            seq: target_seq,
                            total: target_seq,
                            status: 0,
                            error: None,
                            finished: true,
                        });
                        return Ok(());
                    }

                    let target_req = match config.registry.get(request_id) {
                        Some(r) => r,
                        None => {
                            seq += 1;
                            target_seq += 1;
                            attempts_failed += 1;
                            let synthetic_err = RunnerError::Extraction {
                                reason: format!("matrix references unknown Request '{request_id}'"),
                            };
                            write_synthetic_failed_attempt(
                                &run_path,
                                &config.run_id,
                                seq,
                                request_id,
                                payload,
                                Some(iteration),
                                &synthetic_err,
                            )?;
                            on_progress(RunProgress {
                                run_id: config.run_id.clone(),
                                seq: target_seq,
                                total: target_total,
                                status: 0,
                                error: Some(synthetic_err.to_string()),
                                finished: false,
                            });
                            continue;
                        }
                    };

                    let session_client = session_manager
                        .client_for_with_timeout(&payload.session, target_req.timeout_seconds)?;

                    // Per-cell cache when not shared.
                    let mut local_cache = crate::template::BindCache::new();
                    let cache: &mut crate::template::BindCache =
                        shared_cache.as_mut().unwrap_or(&mut local_cache);

                    let send_time = Instant::now();
                    let chain_result = if let Some(cancel) = &cancellation {
                        tokio::select! {
                            _ = cancel.until_cancelled() => {
                                write_footer(
                                    &run_path,
                                    &config.run_id,
                                    seq,
                                    attempts_failed,
                                    RunStatus::AbortedByUser,
                                )?;
                                on_progress(RunProgress {
                                    run_id: config.run_id.clone(),
                                    seq: target_seq,
                                    total: target_seq,
                                    status: 0,
                                    error: None,
                                    finished: true,
                                });
                                return Ok(());
                            }
                            result = crate::deps::fire_chain(
                                session_client,
                                &config.registry,
                                request_id,
                                &payload.text,
                                &config.session_strategy,
                                &payload.session,
                                cache,
                            ) => result
                        }
                    } else {
                        crate::deps::fire_chain(
                            session_client,
                            &config.registry,
                            request_id,
                            &payload.text,
                            &config.session_strategy,
                            &payload.session,
                            cache,
                        )
                        .await
                    };

                    match chain_result {
                        Ok(outcome) => {
                            // Persist each prerequisite as its own attempt with
                            // kind=prerequisite, in topological order.
                            for (prereq_id, resp) in outcome.prerequisites {
                                let prereq_req = config
                                    .registry
                                    .get(&prereq_id)
                                    .expect("prereq exists in registry");
                                seq += 1;
                                let outcome_log = RequestOutcome::from_adapter(
                                    Ok(resp),
                                    send_time.elapsed().as_millis() as u64,
                                );
                                let prereq_meta = AttemptMeta {
                                    seq,
                                    prompt_id: payload.prompt_id.clone(),
                                    payload_id: format!("prereq:{prereq_id}"),
                                    step_id: None,
                                    iteration: Some(iteration),
                                    session: Some(payload.session.clone()),
                                    prompt_text: None,
                                    kind: Some("prerequisite".to_owned()),
                                };
                                if outcome_log.error.is_some() {
                                    attempts_failed += 1;
                                }
                                let attempt = build_attempt(
                                    prereq_meta,
                                    &outcome_log,
                                    &config.run_id,
                                    prereq_req,
                                    iso_now(),
                                    iso_now(),
                                    &responses_dir,
                                )?;
                                runs::append(&run_path, &attempt)
                                    .map_err(|e| anyhow::anyhow!(e))?;
                            }

                            // Now the target attempt.
                            seq += 1;
                            target_seq += 1;
                            let target_outcome = RequestOutcome::from_adapter(
                                Ok(outcome.target),
                                send_time.elapsed().as_millis() as u64,
                            );
                            let progress_status = target_outcome.status;
                            let progress_error = target_outcome.error.clone();
                            if target_outcome.error.is_some() {
                                attempts_failed += 1;
                            }
                            let target_meta = AttemptMeta {
                                seq,
                                prompt_id: payload.prompt_id.clone(),
                                payload_id: payload.payload_id.clone(),
                                step_id: None,
                                iteration: Some(iteration),
                                session: Some(payload.session.clone()),
                                prompt_text: Some(payload.text.clone()),
                                kind: None,
                            };
                            let response_status = progress_status;
                            let response_headers = target_outcome.response_headers.clone();
                            let response_body_size = target_outcome
                                .body_bytes
                                .as_ref()
                                .map(|b| b.len() as u64)
                                .unwrap_or(0);
                            let response_body = render_response_body_for_log(
                                target_outcome.body_bytes.as_ref(),
                                config.body_logging_enabled,
                            );
                            let request_log = render_request_for_log(
                                target_req,
                                &payload.text,
                                &config.session_strategy,
                                &payload.session,
                                config.body_logging_enabled,
                            );

                            let attempt = build_attempt(
                                target_meta,
                                &target_outcome,
                                &config.run_id,
                                target_req,
                                iso_now(),
                                iso_now(),
                                &responses_dir,
                            )?;
                            runs::append(&run_path, &attempt).map_err(|e| anyhow::anyhow!(e))?;

                            if let Some(on_attempt_log) = &on_attempt_log {
                                let (request_headers, request_body_size, request_body) =
                                    match &request_log {
                                        Ok(log) => (
                                            log.headers.clone(),
                                            log.body_size,
                                            log.body_text.clone(),
                                        ),
                                        Err(_) => (HashMap::new(), 0, None),
                                    };
                                on_attempt_log(AttemptLog {
                                    run_id: config.run_id.clone(),
                                    seq,
                                    request_method: target_req.method.clone(),
                                    request_url: target_req.url.clone(),
                                    request_headers,
                                    request_body_size,
                                    request_body,
                                    response_status,
                                    response_headers,
                                    response_body_size,
                                    response_body,
                                    duration_ms: target_outcome.duration_ms,
                                    error: progress_error.clone(),
                                    is_timeout: target_outcome.is_timeout,
                                });
                            }

                            on_progress(RunProgress {
                                run_id: config.run_id.clone(),
                                seq: target_seq,
                                total: target_total,
                                status: progress_status,
                                error: progress_error,
                                finished: false,
                            });
                        }
                        Err(err) => {
                            // Couldn't even resolve / fire prereqs. Synthesize one
                            // failed target attempt so the row exists in the log.
                            seq += 1;
                            target_seq += 1;
                            attempts_failed += 1;
                            write_synthetic_failed_attempt(
                                &run_path,
                                &config.run_id,
                                seq,
                                request_id,
                                payload,
                                Some(iteration),
                                &err,
                            )?;
                            on_progress(RunProgress {
                                run_id: config.run_id.clone(),
                                seq: target_seq,
                                total: target_total,
                                status: 0,
                                error: Some(err.to_string()),
                                finished: false,
                            });
                            if cancellation
                                .as_ref()
                                .is_some_and(RunCancellation::is_cancelled)
                            {
                                break 'outer;
                            }
                        }
                    }
                } // end for _local_iter
            }
        }
    }

    write_footer(
        &run_path,
        &config.run_id,
        seq,
        attempts_failed,
        RunStatus::Completed,
    )?;
    on_progress(RunProgress {
        run_id: config.run_id,
        seq: target_seq,
        total: target_total,
        status: 0,
        error: None,
        finished: true,
    });
    Ok(())
}

/// Append a synthetic failed attempt to the run log. Used when the chain
/// fails before producing any AdapterResponse — typically a missing
/// Request, an unresolved env var, or a cycle detected by the resolver.
fn write_synthetic_failed_attempt(
    run_path: &std::path::Path,
    run_id: &str,
    seq: u32,
    request_id: &str,
    payload: &Payload,
    iteration: Option<u32>,
    err: &RunnerError,
) -> Result<(), RunnerError> {
    let now = iso_now();
    let attempt = RunRecord::Attempt(Box::new(RunAttempt {
        seq,
        ts: now.clone(),
        prompt_id: payload.prompt_id.clone(),
        payload_id: payload.payload_id.clone(),
        step_id: None,
        iteration,
        session: Some(payload.session.clone()),
        prompt_text: Some(payload.text.clone()),
        kind: None,
        request: RequestEnvelope {
            method: "POST".to_owned(),
            url: format!("matrix:{request_id}"),
            headers: HashMap::new(),
            headers_hash: None,
            body_size: 0,
        },
        response: ResponseEnvelope {
            status: 0,
            headers: HashMap::new(),
            body_size: 0,
            body_file: None,
            error: Some(err.to_string()),
        },
        timing: Timing {
            sent_at: now.clone(),
            first_byte_at: None,
            received_at: now,
            duration_ms: 0,
        },
        indicators_matched: Vec::new(),
    }));
    let _ = run_id; // silence unused: the attempt itself doesn't carry run_id
    runs::append(run_path, &attempt).map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn write_header(
    run_path: &std::path::Path,
    run_id: &str,
    engagement_dir: &std::path::Path,
    request_id: &str,
    runner_version: &str,
    prompt_ids: impl IntoIterator<Item = String>,
    scenario_id: Option<String>,
) -> Result<(), RunnerError> {
    let prompt_files = prompt_ids
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let header = RunRecord::Header(RunHeader {
        run_id: run_id.to_owned(),
        engagement: engagement_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned(),
        request_id: request_id.to_owned(),
        started_at: iso_now(),
        runner_version: runner_version.to_owned(),
        prompt_files,
        scenario_id,
    });
    runs::append(run_path, &header).map_err(|e| anyhow::anyhow!(e).into())
}

fn write_footer(
    run_path: &std::path::Path,
    run_id: &str,
    attempts_total: u32,
    attempts_failed: u32,
    status: RunStatus,
) -> Result<(), RunnerError> {
    let footer = RunRecord::Footer(RunFooter {
        run_id: run_id.to_owned(),
        finished_at: iso_now(),
        attempts_total,
        attempts_failed,
        status,
    });
    runs::append(run_path, &footer).map_err(|e| anyhow::anyhow!(e).into())
}

fn build_attempt(
    meta: AttemptMeta,
    outcome: &RequestOutcome,
    run_id: &str,
    request: &Request,
    sent_at: String,
    received_at: String,
    responses_dir: &std::path::Path,
) -> Result<RunRecord, RunnerError> {
    let body_file = if let Some(bytes) = outcome.body_bytes.as_ref() {
        let path = responses_dir.join(format!("{:04}.txt", meta.seq));
        let rel = format!("responses/{run_id}/{:04}.txt", meta.seq);
        atomic_write(&path, bytes).map_err(|e| anyhow::anyhow!(e))?;
        Some(rel)
    } else {
        None
    };

    let body_size = outcome
        .body_bytes
        .as_ref()
        .map(|b| b.len() as u64)
        .unwrap_or(0);
    let request_body_size = estimate_request_body_size(
        request,
        meta.prompt_text.as_deref().unwrap_or_default(),
        meta.session.as_deref().unwrap_or("default"),
    );
    let request_headers = crate::redact::request_headers_for_log(request);

    Ok(RunRecord::Attempt(Box::new(RunAttempt {
        seq: meta.seq,
        ts: sent_at.clone(),
        prompt_id: meta.prompt_id,
        payload_id: meta.payload_id,
        step_id: meta.step_id,
        iteration: meta.iteration,
        session: meta.session,
        prompt_text: meta.prompt_text,
        kind: meta.kind,
        request: RequestEnvelope {
            method: request.method.clone(),
            url: request.url.clone(),
            headers: request_headers,
            headers_hash: None,
            body_size: request_body_size,
        },
        response: ResponseEnvelope {
            status: outcome.status,
            headers: outcome.response_headers.clone(),
            body_size,
            body_file,
            error: outcome.error.clone(),
        },
        timing: Timing {
            sent_at,
            first_byte_at: outcome.first_byte_ms.map(|_| received_at.clone()),
            received_at,
            duration_ms: outcome.duration_ms,
        },
        indicators_matched: vec![],
    })))
}

fn render_request_for_log(
    request: &Request,
    prompt: &str,
    session_strategy: &SessionStrategy,
    session_value: &str,
    body_logging_enabled: bool,
) -> Result<RequestLogPayload, RunnerError> {
    fn inject_session_body_field(
        body: serde_json::Value,
        session_strategy: &SessionStrategy,
        session_value: &str,
    ) -> serde_json::Value {
        if let SessionStrategy::BodyField { field_name } = session_strategy {
            match body {
                serde_json::Value::Object(mut map) => {
                    map.insert(
                        field_name.clone(),
                        serde_json::Value::String(session_value.to_owned()),
                    );
                    serde_json::Value::Object(map)
                }
                other => other,
            }
        } else {
            body
        }
    }

    let mut headers = crate::redact::request_headers_for_log(request);
    if let SessionStrategy::Header { header_name } = session_strategy {
        headers.insert(header_name.clone(), session_value.to_owned());
    }
    let headers = crate::template::render_headers(&headers, prompt)?;

    let (body_size, body_text) = match request.adapter {
        storage::types::AdapterType::CustomRest => match request.body.format {
            BodyFormat::Json => {
                let body = inject_session_body_field(
                    request.body.content.clone(),
                    session_strategy,
                    session_value,
                );
                let json = crate::template::render_json_value(body, prompt)?;
                let text =
                    serde_json::to_string_pretty(&json).map_err(|e| RunnerError::Extraction {
                        reason: e.to_string(),
                    })?;
                (
                    text.len() as u64,
                    body_logging_enabled.then_some(limit_body_for_log(text.into_bytes())),
                )
            }
            BodyFormat::Form => {
                let body_str = serde_json::to_string(&request.body.content).map_err(|e| {
                    RunnerError::Extraction {
                        reason: e.to_string(),
                    }
                })?;
                let rendered = crate::template::render(&body_str, prompt)?;
                (
                    rendered.len() as u64,
                    body_logging_enabled.then_some(limit_body_for_log(rendered.into_bytes())),
                )
            }
            BodyFormat::Text | BodyFormat::Raw => {
                let text = request.body.content.as_str().unwrap_or_default().to_owned();
                let rendered = crate::template::render(&text, prompt)?;
                (
                    rendered.len() as u64,
                    body_logging_enabled.then_some(limit_body_for_log(rendered.into_bytes())),
                )
            }
        },
        storage::types::AdapterType::OpenAiCompat => {
            let mut body = request.body.content.clone();
            if let Some(messages) = body.get_mut("messages") {
                let msgs_str =
                    serde_json::to_string(messages).map_err(|e| RunnerError::Extraction {
                        reason: e.to_string(),
                    })?;
                let rendered = crate::template::render(&msgs_str, prompt)?;
                *messages =
                    serde_json::from_str(&rendered).map_err(|e| RunnerError::Extraction {
                        reason: e.to_string(),
                    })?;
            } else {
                body["messages"] = serde_json::json!([{"role":"user","content":prompt}]);
            }
            body = inject_session_body_field(body, session_strategy, session_value);
            let text =
                serde_json::to_string_pretty(&body).map_err(|e| RunnerError::Extraction {
                    reason: e.to_string(),
                })?;
            (
                text.len() as u64,
                body_logging_enabled.then_some(limit_body_for_log(text.into_bytes())),
            )
        }
        storage::types::AdapterType::RawHttp => {
            let text = match &request.body.content {
                serde_json::Value::String(s) => crate::template::render(s, prompt)?,
                other => crate::template::render(&other.to_string(), prompt)?,
            };
            (
                text.len() as u64,
                body_logging_enabled.then_some(limit_body_for_log(text.into_bytes())),
            )
        }
    };

    Ok(RequestLogPayload {
        headers,
        body_size,
        body_text,
    })
}

fn render_response_body_for_log(
    body: Option<&Bytes>,
    body_logging_enabled: bool,
) -> Option<String> {
    if !body_logging_enabled {
        return None;
    }
    body.map(|bytes| limit_body_for_log(bytes.to_vec()))
}

fn limit_body_for_log(body: Vec<u8>) -> String {
    const BODY_LOG_LIMIT: usize = 500 * 1024;
    if body.len() > BODY_LOG_LIMIT {
        return format!(
            "Won't log the payload since the size is {:.1} KB",
            body.len() as f64 / 1024.0
        );
    }
    String::from_utf8_lossy(&body).into_owned()
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use serde_json::json;
    use storage::types::{BodyConfig, ExtractConfig, ResponseConfig};

    fn sample_request() -> Request {
        Request {
            version: 1,
            id: "req-1".to_owned(),
            name: "Request".to_owned(),
            method: "POST".to_owned(),
            url: "https://example.test".to_owned(),
            auth: AuthConfig::None,
            headers: HashMap::new(),
            body: BodyConfig {
                format: BodyFormat::Json,
                content: json!({"prompt":"{{prompt}}"}),
            },
            response: ResponseConfig {
                extract: ExtractConfig::Raw,
                result_columns: Vec::new(),
                bind: None,
            },
            timeout_seconds: 50,
            adapter: storage::types::AdapterType::CustomRest,
            tag: None,
        }
    }

    #[test]
    fn request_headers_mask_known_secret_headers_case_insensitively() {
        let mut request = sample_request();
        request
            .headers
            .insert("Authorization".to_owned(), "Bearer secret".to_owned());
        request
            .headers
            .insert("x-api-key".to_owned(), "super-secret".to_owned());
        request
            .headers
            .insert("Content-Type".to_owned(), "application/json".to_owned());

        let masked = crate::redact::request_headers_for_log(&request);

        assert_eq!(masked.get("Authorization"), Some(&"<redacted>".to_owned()));
        assert_eq!(masked.get("x-api-key"), Some(&"<redacted>".to_owned()));
        assert_eq!(
            masked.get("Content-Type"),
            Some(&"application/json".to_owned())
        );
    }

    #[test]
    fn request_headers_mask_custom_auth_header() {
        let mut request = sample_request();
        request.auth = AuthConfig::CustomHeader {
            header_name: "X-Custom-Key".to_owned(),
            value_env: "CUSTOM_KEY".to_owned(),
        };
        request
            .headers
            .insert("X-Custom-Key".to_owned(), "secret-value".to_owned());

        let masked = crate::redact::request_headers_for_log(&request);

        assert_eq!(masked.get("X-Custom-Key"), Some(&"<redacted>".to_owned()));
    }

    #[test]
    fn limit_body_for_log_keeps_small_payloads() {
        let logged = limit_body_for_log(b"hello".to_vec());
        assert_eq!(logged, "hello");
    }

    #[test]
    fn limit_body_for_log_replaces_large_payloads() {
        let body = vec![b'a'; 500 * 1024 + 1];
        let logged = limit_body_for_log(body);

        assert!(logged.starts_with("Won't log the payload since the size is "));
        assert!(logged.ends_with(" KB"));
    }
}

fn estimate_request_body_size(request: &Request, prompt: &str, session_value: &str) -> u64 {
    fn inject_session_body_field(
        mut body: serde_json::Value,
        session_config: &SessionConfig,
        session_value: &str,
    ) -> serde_json::Value {
        if let SessionConfig::BodyField { field_name } = session_config {
            if let Some(map) = body.as_object_mut() {
                map.insert(
                    field_name.clone(),
                    serde_json::Value::String(session_value.to_owned()),
                );
            }
        }
        body
    }

    let render_len = |text: String| -> u64 {
        crate::template::render(&text, prompt)
            .map(|s| s.len() as u64)
            .unwrap_or(text.len() as u64)
    };

    match request.body.format {
        BodyFormat::Json => {
            let body = inject_session_body_field(
                request.body.content.clone(),
                &SessionConfig::None,
                session_value,
            );
            crate::template::render_json_value(body, prompt)
                .and_then(|body| {
                    serde_json::to_string(&body).map_err(|e| RunnerError::Extraction {
                        reason: e.to_string(),
                    })
                })
                .map(|text| text.len() as u64)
                .unwrap_or_else(|_| {
                    serde_json::to_string(&request.body.content)
                        .map(|text| text.len() as u64)
                        .unwrap_or_default()
                })
        }
        BodyFormat::Form => {
            let text = serde_json::to_string(&request.body.content).unwrap_or_default();
            render_len(text)
        }
        BodyFormat::Text | BodyFormat::Raw => {
            if let Some(text) = request.body.content.as_str() {
                render_len(text.to_owned())
            } else {
                render_len(request.body.content.to_string())
            }
        }
    }
}

pub fn iso_now() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // Produce a simple ISO-8601 UTC string without external deps.
    let secs = now.as_secs();
    let (h, rem) = (secs / 3600 % 24, secs % 3600);
    let (m, s) = (rem / 60, rem % 60);
    let days = secs / 86400;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Convert days-since-epoch to (year, month, day).
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let leap = is_leap(year);
        let days_in_year = if leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let months = [
        31u64,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &dim in &months {
        if days < dim {
            break;
        }
        days -= dim;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}
