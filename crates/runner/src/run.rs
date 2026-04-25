use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use bytes::Bytes;
use sha2::{Digest, Sha256};
use storage::runs::{
    RequestEnvelope, ResponseEnvelope, RunAttempt, RunFooter, RunHeader, RunRecord, RunStatus,
    Timing,
};
use storage::types::Request;
use storage::{atomic_write, runs};
use tokio::sync::Semaphore;

use crate::adapter;
use crate::error::RunnerError;

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

// ── Execution ─────────────────────────────────────────────────────────────────

/// Execute a full run: fire all payloads, write a JSONL line per attempt,
/// emit progress after each one. Returns when all payloads are done.
///
/// Payloads execute in parallel up to `config.parallelism`. The JSONL file
/// is written sequentially (one fsync per line) to preserve append semantics.
///
/// `on_progress` is called from the tokio worker thread — implementations
/// must be `Send`.
pub async fn execute_run<F>(config: RunConfig, on_progress: F) -> Result<(), RunnerError>
where
    F: Fn(RunProgress) + Send + Sync + 'static,
{
    let total = config.payloads.len() as u32;
    let run_path = config.engagement_dir.join("runs").join(format!("{}.jsonl", config.run_id));
    let responses_dir = config.engagement_dir.join("responses").join(&config.run_id);

    std::fs::create_dir_all(&responses_dir).map_err(|e| anyhow::anyhow!(e))?;

    // Write JSONL header.
    let header = RunRecord::Header(RunHeader {
        run_id: config.run_id.clone(),
        engagement: config
            .engagement_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned(),
        request_id: config.request.id.clone(),
        started_at: iso_now(),
        runner_version: config.runner_version.clone(),
        prompt_files: config
            .payloads
            .iter()
            .map(|p| p.prompt_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect(),
    });
    runs::append(&run_path, &header).map_err(|e| anyhow::anyhow!(e))?;

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
            let send_time = std::time::Instant::now();

            let result = adapter::execute(&http, &request, &payload.text).await;

            let received_at = iso_now();

            #[allow(clippy::type_complexity)]
            let (status, error, response_headers, body_bytes, first_byte_ms, duration_ms): (
                u16,
                Option<String>,
                std::collections::HashMap<String, String>,
                Option<Bytes>,
                Option<u64>,
                u64,
            ) = match result {
                Ok(r) => (
                    r.status,
                    None,
                    r.response_headers,
                    Some(r.body_bytes),
                    r.first_byte_ms,
                    r.duration_ms,
                ),
                Err(e) => (
                    0,
                    Some(e.to_string()),
                    std::collections::HashMap::new(),
                    None,
                    None,
                    send_time.elapsed().as_millis() as u64,
                ),
            };

            // Write response body atomically before appending the JSONL line.
            let body_file = if let Some(ref bytes) = body_bytes {
                let path = responses_dir.join(format!("{seq:04}.txt"));
                let rel = format!("responses/{}/{seq:04}.txt", run_id.as_str());
                atomic_write(&path, bytes).map_err(|e| anyhow::anyhow!(e))?;
                Some(rel)
            } else {
                None
            };

            let body_size = body_bytes.as_ref().map(|b| b.len() as u64).unwrap_or(0);

            // Build the JSONL attempt record.
            let attempt = RunRecord::Attempt(Box::new(RunAttempt {
                seq,
                ts: sent_at.clone(),
                prompt_id: payload.prompt_id.clone(),
                payload_id: payload.payload_id.clone(),
                request: RequestEnvelope {
                    method: request.method.clone(),
                    url: request.url.clone(),
                    headers_hash: headers_hash(&request),
                    body_size: 0, // exact body_size tracked per-request in M5
                },
                response: ResponseEnvelope {
                    status,
                    headers: response_headers,
                    body_size,
                    body_file,
                    error: error.clone(),
                },
                timing: Timing {
                    sent_at,
                    first_byte_at: first_byte_ms
                        .map(|_| received_at.clone()),
                    received_at,
                    duration_ms,
                },
                indicators_matched: vec![],
            }));

            runs::append(&run_path, &attempt).map_err(|e| anyhow::anyhow!(e))?;

            on_progress(RunProgress {
                run_id: run_id.as_ref().clone(),
                seq,
                total,
                status,
                error,
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

    // Write footer.
    let footer = RunRecord::Footer(RunFooter {
        run_id: config.run_id.clone(),
        finished_at: iso_now(),
        attempts_total: total,
        attempts_failed,
        status: RunStatus::Completed,
    });
    runs::append(&run_path, &footer).map_err(|e| anyhow::anyhow!(e))?;

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

/// Compute SHA-256 of the request's static headers + auth type (not the token
/// value itself) for recording in the JSONL envelope.
///
/// Invariant: token values never appear in logs (CLAUDE.md invariant 10).
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
    // Include auth *type* (not value) so different auth modes are distinguishable.
    let auth_type = match &request.auth {
        storage::types::AuthConfig::Bearer { .. } => "bearer",
        storage::types::AuthConfig::Basic { .. } => "basic",
        storage::types::AuthConfig::CustomHeader { .. } => "custom-header",
        storage::types::AuthConfig::None => "none",
    };
    h.update(auth_type.as_bytes());
    format!(
        "sha256:{}",
        h.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
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
    let months = [31u64, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
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

