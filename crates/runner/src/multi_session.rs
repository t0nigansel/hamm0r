// Section 1 of docs/ToDo.md and docs/plans/multiSessionPlan.md.
//
// Multi-session runner. Fires a scenario across N sessions, each with
// an isolated HTTP client (cookie jar + optional session identity
// header), schedules prompts in plant → barrier → probe phases, and
// substitutes a per-session canary token into prompts that use the
// `{{canary}}` template marker. The post-run leak scanner (Phase 4) and
// analyzer integration (Phase 5) build on top of this module.
//
// V1 fires sessions sequentially within each phase. The plan acknowledges
// parallel-across-sessions as a future refinement; sequential v1 keeps
// per-session cookie state coherent without any extra synchronization
// and matches the existing `execute_matrix_run` model.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use storage::runs::{
    LeakDetected, RequestEnvelope, ResponseEnvelope, RunAttempt, RunHeader, RunRecord, RunStatus,
    Timing,
};
use storage::types::{Phase, Request, SessionIdentityConfig, SessionIdentityKind};
use storage::{atomic_write, runs};

use crate::adapter;
use crate::canary;
use crate::error::RunnerError;
use crate::run::{AttemptLog, RunCancellation, RunProgress};
use crate::session::SessionStrategy;

// ── Public API ────────────────────────────────────────────────────────────────

/// One prompt with its multi-session phase tag.
#[derive(Debug, Clone)]
pub struct PhasedPayload {
    pub prompt_id: String,
    pub payload_id: String,
    pub text: String,
    pub phase: Phase,
}

/// Configuration for one multi-session run. Modelled on
/// `MatrixRunConfig` but specialized: sessions are built here from
/// `session_count` + `session_identity`, prompts carry an explicit
/// phase tag, and the post-run leak scanner is part of the same call.
pub struct MultiSessionRunConfig {
    pub engagement_dir: PathBuf,
    pub run_id: String,
    pub scenario_id: String,
    pub registry: HashMap<String, Request>,
    /// Target Request ids to fire (each one against every prompt).
    pub request_ids: Vec<String>,
    /// Per-request repeat multipliers, multiplied on top of `repeat`.
    pub per_request_repeat: HashMap<String, u32>,
    /// Prompts already resolved from the library subset, tagged with phase.
    pub prompts: Vec<PhasedPayload>,
    pub repeat: u32,
    pub session_count: u32,
    pub session_identity: SessionIdentityConfig,
    pub runner_version: String,
    pub body_logging_enabled: bool,
    pub on_attempt_log: Option<Arc<dyn Fn(AttemptLog) + Send + Sync>>,
    pub cancellation: Option<RunCancellation>,
}

// ── Internal ──────────────────────────────────────────────────────────────────

struct SessionContext {
    label: String,
    canary: String,
    http: reqwest::Client,
}

/// Build the per-session HTTP clients. Each session always gets its own
/// cookie jar (so cross-session leakage via cookies is impossible by
/// construction). When the identity kind is a header variant the
/// session label is also installed as a default request header on the
/// session's client, so every outbound request from that session
/// carries the right identifier without per-attempt plumbing.
fn build_sessions(
    run_id: &str,
    scenario_id: &str,
    session_count: u32,
    identity: &SessionIdentityConfig,
    timeout_seconds: u32,
) -> Result<Vec<SessionContext>, RunnerError> {
    let mut out = Vec::with_capacity(session_count as usize);
    for idx in 0..session_count {
        let label = format!("s{idx}");
        let canary_value = canary::generate(run_id, idx, scenario_id);

        let mut builder = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(u64::from(timeout_seconds)))
            .cookie_store(true)
            .use_rustls_tls();

        // Apply session identity header (when applicable). The session
        // identifier is the short label (e.g. "s0"), not the canary;
        // the canary lives in prompt text.
        if let Some((header_name, value)) = identity_header_for(identity, &label) {
            let mut headers = HeaderMap::new();
            let name = HeaderName::from_bytes(header_name.as_bytes()).map_err(|err| {
                RunnerError::Extraction {
                    reason: format!("invalid session identity header name '{header_name}': {err}"),
                }
            })?;
            let value = HeaderValue::from_str(&value).map_err(|err| RunnerError::Extraction {
                reason: format!("invalid session identity header value: {err}"),
            })?;
            headers.insert(name, value);
            builder = builder.default_headers(headers);
        }

        let http = builder.build().map_err(RunnerError::Http)?;
        out.push(SessionContext {
            label,
            canary: canary_value,
            http,
        });
    }
    Ok(out)
}

fn identity_header_for(identity: &SessionIdentityConfig, label: &str) -> Option<(String, String)> {
    match &identity.kind {
        SessionIdentityKind::CookieJar => None,
        SessionIdentityKind::ConversationHeader { header_name }
        | SessionIdentityKind::CustomHeader { header_name } => {
            Some((header_name.clone(), label.to_owned()))
        }
    }
}

// ── Execution ─────────────────────────────────────────────────────────────────

/// Fire a scenario across `session_count` parallel sessions, plant-phase
/// before probe-phase. Returns when the leak scanner has finished writing
/// `leak_detected` records.
pub async fn execute_multi_session_run<F>(
    config: MultiSessionRunConfig,
    on_progress: F,
) -> Result<(), RunnerError>
where
    F: Fn(RunProgress) + Send + Sync + 'static,
{
    let run_path = config
        .engagement_dir
        .join("runs")
        .join(format!("{}.jsonl", config.run_id));
    let responses_dir = runs::ensure_response_dir(&config.engagement_dir, &config.run_id)
        .map_err(|e| anyhow::anyhow!(e))?;

    let primary_request_id = config
        .request_ids
        .first()
        .cloned()
        .unwrap_or_else(|| "multi-session".to_owned());

    // Deduplicate prompt ids for the header `prompt_files` field, in
    // declaration order (which is the order multi-session prompts come
    // from the library subset upstream).
    let mut seen = std::collections::HashSet::new();
    let prompt_files: Vec<String> = config
        .prompts
        .iter()
        .filter(|p| seen.insert(p.prompt_id.clone()))
        .map(|p| p.prompt_id.clone())
        .collect();

    write_header(
        &run_path,
        &config.run_id,
        &config.engagement_dir,
        &primary_request_id,
        &config.runner_version,
        prompt_files,
        Some(config.scenario_id.clone()),
    )?;

    let timeout_seconds = config
        .request_ids
        .iter()
        .filter_map(|id| config.registry.get(id).map(|r| r.timeout_seconds))
        .max()
        .unwrap_or(30);

    let sessions = build_sessions(
        &config.run_id,
        &config.scenario_id,
        config.session_count.max(1),
        &config.session_identity,
        timeout_seconds,
    )?;

    let plants: Vec<&PhasedPayload> = config
        .prompts
        .iter()
        .filter(|p| p.phase == Phase::Plant)
        .collect();
    let probes_and_any: Vec<&PhasedPayload> = config
        .prompts
        .iter()
        .filter(|p| p.phase == Phase::Probe || p.phase == Phase::Any)
        .collect();

    let repeat = config.repeat.max(1);

    // Total counts only target attempts (one per cell) for progress UX.
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
    let total_per_session =
        ((plants.len() + probes_and_any.len()) as u32).saturating_mul(request_repeat_sum);
    let target_total = total_per_session
        .saturating_mul(repeat)
        .saturating_mul(config.session_count.max(1));

    let mut seq: u32 = 0;
    let mut target_seq: u32 = 0;
    let mut attempts_failed: u32 = 0;
    // Box into a trait object so the per-cell helper has a single
    // monomorphized type to take. The generic `F` only matters at the
    // public boundary.
    let on_progress: Arc<dyn Fn(RunProgress) + Send + Sync> = Arc::new(on_progress);
    let on_attempt_log = config.on_attempt_log.clone();
    let cancellation = config.cancellation.clone();

    // Helper closure that fires one cell. Async closures aren't stable;
    // inline the loop instead.
    for iteration in 1..=repeat {
        // ── Plant phase ───────────────────────────────────────────────
        for prompt in &plants {
            if fire_session_round(
                &mut seq,
                &mut target_seq,
                &mut attempts_failed,
                &sessions,
                prompt,
                Phase::Plant,
                iteration,
                &config,
                target_total,
                &run_path,
                &responses_dir,
                &on_progress,
                on_attempt_log.as_ref(),
                cancellation.as_ref(),
            )
            .await?
            {
                // Cancelled — write footer and bail.
                on_progress(crate::run::finalize_run(
                    &run_path,
                    &config.run_id,
                    seq,
                    attempts_failed,
                    RunStatus::AbortedByUser,
                    target_seq,
                    target_seq,
                )?);
                return Ok(());
            }
        }

        // Phase barrier: all plants for this iteration are now written
        // to disk and their responses are on the filesystem. Probes can
        // safely fire knowing every canary has had its chance to land.

        // ── Probe + any phase ─────────────────────────────────────────
        for prompt in &probes_and_any {
            let phase = prompt.phase;
            if fire_session_round(
                &mut seq,
                &mut target_seq,
                &mut attempts_failed,
                &sessions,
                prompt,
                phase,
                iteration,
                &config,
                target_total,
                &run_path,
                &responses_dir,
                &on_progress,
                on_attempt_log.as_ref(),
                cancellation.as_ref(),
            )
            .await?
            {
                on_progress(crate::run::finalize_run(
                    &run_path,
                    &config.run_id,
                    seq,
                    attempts_failed,
                    RunStatus::AbortedByUser,
                    target_seq,
                    target_seq,
                )?);
                return Ok(());
            }
        }
    }

    // ── Leak scanner (Phase 4) ────────────────────────────────────────
    let scanner_started = crate::leak_scanner::run(
        &config.engagement_dir,
        &config.run_id,
        &sessions
            .iter()
            .map(|s| (s.label.clone(), s.canary.clone()))
            .collect::<HashMap<_, _>>(),
        &run_path,
    );
    if let Err(err) = scanner_started {
        // Scanner failure is non-fatal: log to stderr and continue
        // writing the footer. The run itself is still valid; just
        // missing leak verdicts.
        eprintln!("warning: leak scanner failed for {}: {err}", config.run_id);
    }

    on_progress(crate::run::finalize_run(
        &run_path,
        &config.run_id,
        seq,
        attempts_failed,
        RunStatus::Completed,
        target_seq,
        target_total,
    )?);
    Ok(())
}

/// Fire `prompt` across every session × every request × per-request
/// repeat. Returns `Ok(true)` when the run was cancelled mid-iteration
/// (caller must write the footer); `Ok(false)` when the round
/// completed normally.
#[allow(clippy::too_many_arguments)]
async fn fire_session_round(
    seq: &mut u32,
    target_seq: &mut u32,
    attempts_failed: &mut u32,
    sessions: &[SessionContext],
    prompt: &PhasedPayload,
    phase: Phase,
    iteration: u32,
    config: &MultiSessionRunConfig,
    target_total: u32,
    run_path: &std::path::Path,
    responses_dir: &std::path::Path,
    on_progress: &Arc<dyn Fn(RunProgress) + Send + Sync>,
    on_attempt_log: Option<&Arc<dyn Fn(AttemptLog) + Send + Sync>>,
    cancellation: Option<&RunCancellation>,
) -> Result<bool, RunnerError> {
    for session in sessions {
        // Substitute the canary into the prompt text. Plant prompts that
        // contain `{{canary}}` get the per-session canary planted here;
        // probe / any prompts without the marker pass through unchanged.
        let rendered_prompt = canary::inject(&prompt.text, &session.canary);

        for request_id in &config.request_ids {
            let local_repeat = config
                .per_request_repeat
                .get(request_id)
                .copied()
                .unwrap_or(1)
                .max(1);
            for _ in 0..local_repeat {
                if cancellation.is_some_and(|c| c.is_cancelled()) {
                    return Ok(true);
                }

                let target_req = match config.registry.get(request_id) {
                    Some(r) => r,
                    None => {
                        *seq += 1;
                        *target_seq += 1;
                        *attempts_failed += 1;
                        let synthetic_err = RunnerError::Extraction {
                            reason: format!(
                                "multi-session references unknown Request '{request_id}'"
                            ),
                        };
                        write_synthetic_failed_attempt(
                            run_path,
                            *seq,
                            request_id,
                            prompt,
                            &session.label,
                            phase,
                            iteration,
                            &synthetic_err,
                        )?;
                        (on_progress)(RunProgress {
                            run_id: config.run_id.clone(),
                            seq: *target_seq,
                            total: target_total,
                            status: 0,
                            error: Some(synthetic_err.to_string()),
                            finished: false,
                            request_id: Some(request_id.clone()),
                            prompt_id: Some(prompt.prompt_id.clone()),
                        });
                        continue;
                    }
                };

                let send_time = Instant::now();
                let outcome = adapter::execute_with_session(
                    &session.http,
                    target_req,
                    &rendered_prompt,
                    &SessionStrategy::None,
                    &session.label,
                )
                .await;

                let (status, error, response_headers, body_bytes, duration_ms, is_timeout) =
                    match outcome {
                        Ok(r) => (
                            r.status,
                            None,
                            r.response_headers,
                            Some(r.body_bytes),
                            r.duration_ms,
                            false,
                        ),
                        Err(e) => {
                            let timeout =
                                matches!(&e, RunnerError::Http(inner) if inner.is_timeout());
                            (
                                0u16,
                                Some(e.to_string()),
                                HashMap::new(),
                                None,
                                send_time.elapsed().as_millis() as u64,
                                timeout,
                            )
                        }
                    };

                *seq += 1;
                *target_seq += 1;
                if error.is_some() {
                    *attempts_failed += 1;
                }

                let body_size = body_bytes.as_ref().map(|b| b.len() as u64).unwrap_or(0);
                let body_file = if let Some(bytes) = body_bytes.as_ref() {
                    let path = responses_dir.join(format!("{:04}.txt", *seq));
                    let rel = format!("responses/{}/{:04}.txt", config.run_id, *seq);
                    atomic_write(&path, bytes).map_err(|e| anyhow::anyhow!(e))?;
                    Some(rel)
                } else {
                    None
                };

                let now = iso_now();
                let attempt = RunRecord::Attempt(Box::new(RunAttempt {
                    seq: *seq,
                    ts: now.clone(),
                    prompt_id: prompt.prompt_id.clone(),
                    payload_id: prompt.payload_id.clone(),
                    step_id: None,
                    iteration: Some(iteration),
                    session: Some(session.label.clone()),
                    prompt_text: Some(rendered_prompt.clone()),
                    kind: None,
                    request_id: Some(request_id.clone()),
                    mutation_id: None,
                    session_id: Some(session.label.clone()),
                    phase: Some(phase.as_str().to_owned()),
                    request: RequestEnvelope {
                        method: target_req.method.clone(),
                        url: target_req.url.clone(),
                        headers: crate::redact::request_headers_for_log(target_req),
                        headers_hash: None,
                        body_size: rendered_prompt.len() as u64,
                    },
                    response: ResponseEnvelope {
                        status,
                        headers: response_headers.clone(),
                        body_size,
                        body_file,
                        error: error.clone(),
                    },
                    timing: Timing {
                        sent_at: now.clone(),
                        first_byte_at: None,
                        received_at: now,
                        duration_ms,
                    },
                    indicators_matched: Vec::new(),
                }));
                runs::append(run_path, &attempt).map_err(|e| anyhow::anyhow!(e))?;

                if let Some(cb) = on_attempt_log {
                    let body_text = body_bytes
                        .as_ref()
                        .filter(|_| config.body_logging_enabled)
                        .map(|b| String::from_utf8_lossy(b).into_owned());
                    cb(AttemptLog {
                        run_id: config.run_id.clone(),
                        seq: *seq,
                        request_method: target_req.method.clone(),
                        request_url: target_req.url.clone(),
                        request_headers: crate::redact::request_headers_for_log(target_req),
                        request_body_size: rendered_prompt.len() as u64,
                        request_body: config.body_logging_enabled.then(|| rendered_prompt.clone()),
                        response_status: status,
                        response_headers,
                        response_body_size: body_size,
                        response_body: body_text,
                        duration_ms,
                        error: error.clone(),
                        is_timeout,
                    });
                }

                (on_progress)(RunProgress {
                    run_id: config.run_id.clone(),
                    seq: *target_seq,
                    total: target_total,
                    status,
                    error,
                    finished: false,
                    request_id: Some(request_id.clone()),
                    prompt_id: Some(prompt.prompt_id.clone()),
                });
            }
        }
    }
    Ok(false)
}

#[allow(clippy::too_many_arguments)]
fn write_synthetic_failed_attempt(
    run_path: &std::path::Path,
    seq: u32,
    request_id: &str,
    prompt: &PhasedPayload,
    session_label: &str,
    phase: Phase,
    iteration: u32,
    err: &RunnerError,
) -> Result<(), RunnerError> {
    let now = iso_now();
    let attempt = RunRecord::Attempt(Box::new(RunAttempt {
        seq,
        ts: now.clone(),
        prompt_id: prompt.prompt_id.clone(),
        payload_id: prompt.payload_id.clone(),
        step_id: None,
        iteration: Some(iteration),
        session: Some(session_label.to_owned()),
        prompt_text: Some(prompt.text.clone()),
        kind: None,
        request_id: Some(request_id.to_owned()),
        mutation_id: None,
        session_id: Some(session_label.to_owned()),
        phase: Some(phase.as_str().to_owned()),
        request: RequestEnvelope {
            method: "POST".to_owned(),
            url: format!("multi-session:{request_id}"),
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
    runs::append(run_path, &attempt).map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// Append a `LeakDetected` record (used by the scanner; also a public
/// helper so tests can verify the round-trip).
pub fn append_leak(run_path: &std::path::Path, leak: LeakDetected) -> Result<(), RunnerError> {
    runs::append(run_path, &RunRecord::LeakDetected(leak)).map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

// ── Helpers (header / footer / iso_now) shared with the matrix runner ────────
// Kept duplicated rather than refactored out of `run.rs` to keep this
// commit's blast radius small. A follow-up can extract a common module.

fn write_header(
    run_path: &std::path::Path,
    run_id: &str,
    engagement_dir: &std::path::Path,
    request_id: &str,
    runner_version: &str,
    prompt_ids: Vec<String>,
    scenario_id: Option<String>,
) -> Result<(), RunnerError> {
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
        prompt_files: prompt_ids,
        scenario_id,
        replay_of: None,
    });
    runs::append(run_path, &header).map_err(|e| anyhow::anyhow!(e).into())
}

fn iso_now() -> String {
    crate::run::iso_now()
}
