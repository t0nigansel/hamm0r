use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use bytes::Bytes;
use sha2::{Digest, Sha256};
use storage::runs::{
    RequestEnvelope, ResponseEnvelope, RunAttempt, RunFooter, RunHeader, RunRecord, RunStatus,
    Timing,
};
use storage::types::Request;
use storage::{atomic_write, runs};
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
}

/// A scenario step execution unit.
#[derive(Debug, Clone)]
pub struct ScenarioStep {
    pub id: String,
    pub prompt_id: Option<String>,
    pub prompt_text: String,
    pub session: String,
}

/// Configuration for sequential scenario execution.
pub struct ScenarioRunConfig {
    pub engagement_dir: PathBuf,
    pub run_id: String,
    pub request: Request,
    pub session_strategy: SessionStrategy,
    pub steps: Vec<ScenarioStep>,
    pub repeat: u32,
    pub runner_version: String,
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

// ── Internal types ────────────────────────────────────────────────────────────

/// Result of one HTTP exchange, decoded from the adapter response.
struct RequestOutcome {
    status: u16,
    error: Option<String>,
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
                response_headers: r.response_headers,
                body_bytes: Some(r.body_bytes),
                first_byte_ms: r.first_byte_ms,
                duration_ms: r.duration_ms,
            },
            Err(e) => Self {
                status: 0,
                error: Some(e.to_string()),
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
    )?;

    let sem = Arc::new(Semaphore::new(config.parallelism));
    let on_progress = Arc::new(on_progress);
    let request = Arc::new(config.request);
    let run_id = Arc::new(config.run_id.clone());
    let run_path = Arc::new(run_path);
    let responses_dir = Arc::new(responses_dir);

    let mut handles = Vec::new();

    for (seq_0, payload) in config.payloads.into_iter().enumerate() {
        let seq = (seq_0 + 1) as u32;
        let permit = sem.clone().acquire_owned().await.unwrap();
        let request = Arc::clone(&request);
        let run_id = Arc::clone(&run_id);
        let run_path = Arc::clone(&run_path);
        let responses_dir = Arc::clone(&responses_dir);
        let on_progress = Arc::clone(&on_progress);

        let handle = tokio::spawn(async move {
            let _permit = permit;

            let http = crate::client::build_http_client(request.timeout_seconds)?;
            let sent_at = iso_now();
            let send_time = Instant::now();

            let result = adapter::execute_with_session(
                &http,
                &request,
                &payload.text,
                &SessionStrategy::None,
                &payload.session,
            )
            .await;

            let received_at = iso_now();
            let outcome =
                RequestOutcome::from_adapter(result, send_time.elapsed().as_millis() as u64);
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
            };

            let attempt =
                build_attempt(meta, outcome, &run_id, &request, sent_at, received_at, &responses_dir)?;
            runs::append(&run_path, &attempt).map_err(|e| anyhow::anyhow!(e))?;

            on_progress(RunProgress {
                run_id: run_id.as_ref().clone(),
                seq,
                total,
                status: progress_status,
                error: progress_error,
                finished: false,
            });

            Ok::<_, RunnerError>(())
        });

        handles.push(handle);
    }

    let mut attempts_failed = 0u32;
    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => attempts_failed += 1,
        }
    }

    write_footer(&run_path, &config.run_id, total, attempts_failed)?;

    on_progress(RunProgress {
        run_id: config.run_id,
        seq: total,
        total,
        status: 0,
        error: None,
        finished: true,
    });

    Ok(())
}

/// Execute a scenario run sequentially, preserving order and session state.
pub async fn execute_scenario_run<F>(
    config: ScenarioRunConfig,
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

    let total = (config.steps.len() as u32).saturating_mul(config.repeat.max(1));
    write_header(
        &run_path,
        &config.run_id,
        &config.engagement_dir,
        &config.request.id,
        &config.runner_version,
        config
            .steps
            .iter()
            .map(|s| s.prompt_id.clone().unwrap_or_else(|| "custom".to_owned())),
    )?;

    let mut session_manager = SessionManager::new(
        config.session_strategy.clone(),
        config.request.timeout_seconds,
    );

    let mut seq = 0u32;
    let mut attempts_failed = 0u32;

    for iteration in 1..=config.repeat.max(1) {
        for step in &config.steps {
            seq += 1;

            let sent_at = iso_now();
            let send_time = Instant::now();
            let session_client = session_manager.client_for(&step.session)?;

            let result = adapter::execute_with_session(
                session_client,
                &config.request,
                &step.prompt_text,
                &config.session_strategy,
                &step.session,
            )
            .await;

            let received_at = iso_now();
            let outcome =
                RequestOutcome::from_adapter(result, send_time.elapsed().as_millis() as u64);
            let progress_status = outcome.status;
            let progress_error = outcome.error.clone();

            if outcome.error.is_some() {
                attempts_failed += 1;
            }

            let prompt_id = step
                .prompt_id
                .clone()
                .unwrap_or_else(|| "custom".to_owned());

            let meta = AttemptMeta {
                seq,
                prompt_id,
                payload_id: step.id.clone(),
                step_id: Some(step.id.clone()),
                iteration: Some(iteration),
                session: Some(step.session.clone()),
                prompt_text: Some(step.prompt_text.clone()),
            };

            let attempt = build_attempt(
                meta,
                outcome,
                &config.run_id,
                &config.request,
                sent_at,
                received_at,
                &responses_dir,
            )?;
            runs::append(&run_path, &attempt).map_err(|e| anyhow::anyhow!(e))?;

            on_progress(RunProgress {
                run_id: config.run_id.clone(),
                seq,
                total,
                status: progress_status,
                error: progress_error,
                finished: false,
            });
        }
    }

    write_footer(&run_path, &config.run_id, total, attempts_failed)?;

    on_progress(RunProgress {
        run_id: config.run_id,
        seq: total,
        total,
        status: 0,
        error: None,
        finished: true,
    });

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
    });
    runs::append(run_path, &header).map_err(|e| anyhow::anyhow!(e).into())
}

fn write_footer(
    run_path: &std::path::Path,
    run_id: &str,
    attempts_total: u32,
    attempts_failed: u32,
) -> Result<(), RunnerError> {
    let footer = RunRecord::Footer(RunFooter {
        run_id: run_id.to_owned(),
        finished_at: iso_now(),
        attempts_total,
        attempts_failed,
        status: RunStatus::Completed,
    });
    runs::append(run_path, &footer).map_err(|e| anyhow::anyhow!(e).into())
}

fn build_attempt(
    meta: AttemptMeta,
    outcome: RequestOutcome,
    run_id: &str,
    request: &Request,
    sent_at: String,
    received_at: String,
    responses_dir: &std::path::Path,
) -> Result<RunRecord, RunnerError> {
    let body_file = if let Some(ref bytes) = outcome.body_bytes {
        let path = responses_dir.join(format!("{:04}.txt", meta.seq));
        let rel = format!("responses/{run_id}/{:04}.txt", meta.seq);
        atomic_write(&path, bytes).map_err(|e| anyhow::anyhow!(e))?;
        Some(rel)
    } else {
        None
    };

    let body_size = outcome.body_bytes.as_ref().map(|b| b.len() as u64).unwrap_or(0);

    Ok(RunRecord::Attempt(Box::new(RunAttempt {
        seq: meta.seq,
        ts: sent_at.clone(),
        prompt_id: meta.prompt_id,
        payload_id: meta.payload_id,
        step_id: meta.step_id,
        iteration: meta.iteration,
        session: meta.session,
        prompt_text: meta.prompt_text,
        request: RequestEnvelope {
            method: request.method.clone(),
            url: request.url.clone(),
            headers_hash: headers_hash(request),
            body_size: 0, // exact body size tracked in a later milestone
        },
        response: ResponseEnvelope {
            status: outcome.status,
            headers: outcome.response_headers,
            body_size,
            body_file,
            error: outcome.error,
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

/// Compute SHA-256 of the request's static headers + auth type (not the token
/// value itself) for recording in the JSONL envelope.
///
/// Invariant: token values never appear in logs.
fn headers_hash(request: &Request) -> String {
    let mut h = Sha256::new();
    let mut pairs: Vec<_> = request.headers.iter().collect();
    pairs.sort_by_key(|(k, _)| k.as_str());
    for (k, v) in pairs {
        h.update(k.as_bytes());
        h.update(b":");
        h.update(v.as_bytes());
        h.update(b"\n");
    }
    let auth_type = match &request.auth {
        storage::types::AuthConfig::Bearer { .. } => "bearer",
        storage::types::AuthConfig::Basic { .. } => "basic",
        storage::types::AuthConfig::CustomHeader { .. } => "custom-header",
        storage::types::AuthConfig::None => "none",
    };
    h.update(auth_type.as_bytes());
    format!(
        "sha256:{}",
        h.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
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
