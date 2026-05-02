use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use bytes::Bytes;
use storage::runs::{
    RequestEnvelope, ResponseEnvelope, RunAttempt, RunFooter, RunHeader, RunRecord, RunStatus,
    Timing,
};
use storage::types::{AuthConfig, BodyFormat, Request, SessionConfig};
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
    pub body_logging_enabled: bool,
    pub on_attempt_log: Option<Arc<dyn Fn(AttemptLog) + Send + Sync>>,
}

/// A scenario step execution unit.
#[derive(Debug, Clone)]
pub struct ScenarioStep {
    pub id: String,
    pub request_id: Option<String>,
    pub prompt_id: Option<String>,
    pub prompt_text: String,
    pub session: String,
}

/// Configuration for sequential scenario execution.
pub struct ScenarioRunConfig {
    pub engagement_dir: PathBuf,
    pub run_id: String,
    pub request: Request,
    pub requests_by_id: HashMap<String, Request>,
    pub session_strategy: SessionStrategy,
    pub steps: Vec<ScenarioStep>,
    pub repeat: u32,
    pub runner_version: String,
    pub body_logging_enabled: bool,
    pub on_attempt_log: Option<Arc<dyn Fn(AttemptLog) + Send + Sync>>,
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
}

struct RequestLogPayload {
    headers: HashMap<String, String>,
    body_size: u64,
    body_text: Option<String>,
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
    let on_attempt_log = config.on_attempt_log.clone();
    let request = Arc::new(config.request);
    let run_id = Arc::new(config.run_id.clone());
    let run_path = Arc::new(run_path);
    let responses_dir = Arc::new(responses_dir);
    let body_logging_enabled = config.body_logging_enabled;

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
    let on_attempt_log = config.on_attempt_log.clone();

    for iteration in 1..=config.repeat.max(1) {
        for step in &config.steps {
            seq += 1;

            let sent_at = iso_now();
            let send_time = Instant::now();
            let request = step
                .request_id
                .as_ref()
                .and_then(|request_id| config.requests_by_id.get(request_id))
                .unwrap_or(&config.request);
            let session_client =
                session_manager.client_for_with_timeout(&step.session, request.timeout_seconds)?;

            let result = adapter::execute_with_session(
                session_client,
                request,
                &step.prompt_text,
                &config.session_strategy,
                &step.session,
            )
            .await;

            let received_at = iso_now();
            let outcome =
                RequestOutcome::from_adapter(result, send_time.elapsed().as_millis() as u64);
            let request_log = render_request_for_log(
                request,
                &step.prompt_text,
                &config.session_strategy,
                &step.session,
                config.body_logging_enabled,
            );
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

            let response_status = outcome.status;
            let response_headers = outcome.response_headers.clone();
            let response_body_size = outcome
                .body_bytes
                .as_ref()
                .map(|b| b.len() as u64)
                .unwrap_or(0);
            let response_body = render_response_body_for_log(
                outcome.body_bytes.as_ref(),
                config.body_logging_enabled,
            );

            let attempt = build_attempt(
                meta,
                &outcome,
                &config.run_id,
                request,
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
                    run_id: config.run_id.clone(),
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
    let request_headers = request_headers_for_log(request);

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

fn request_headers_for_log(request: &Request) -> std::collections::HashMap<String, String> {
    let mut headers = request.headers.clone();
    redact_known_secret_headers(&mut headers);

    match &request.auth {
        AuthConfig::Bearer { .. } => {
            upsert_masked_header(&mut headers, "Authorization", "Bearer <redacted>");
        }
        AuthConfig::Basic { .. } => {
            upsert_masked_header(&mut headers, "Authorization", "Basic <redacted>");
        }
        AuthConfig::CustomHeader { header_name, .. } => {
            upsert_masked_header(&mut headers, header_name, "<redacted>");
        }
        AuthConfig::None => {}
    }

    headers
}

fn redact_known_secret_headers(headers: &mut HashMap<String, String>) {
    const SECRET_HEADERS: &[&str] = &[
        "authorization",
        "proxy-authorization",
        "x-api-key",
        "api-key",
        "x-auth-token",
    ];

    for (name, value) in headers.iter_mut() {
        if SECRET_HEADERS
            .iter()
            .any(|secret| name.eq_ignore_ascii_case(secret))
        {
            *value = "<redacted>".to_owned();
        }
    }
}

fn upsert_masked_header(headers: &mut HashMap<String, String>, header_name: &str, masked: &str) {
    if let Some((_, value)) = headers
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case(header_name))
    {
        *value = masked.to_owned();
        return;
    }

    headers.insert(header_name.to_owned(), masked.to_owned());
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

    let mut headers = request_headers_for_log(request);
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
                let body_str =
                    serde_json::to_string(&body).map_err(|e| RunnerError::Extraction {
                        reason: e.to_string(),
                    })?;
                let rendered = crate::template::render(&body_str, prompt)?;
                let json: serde_json::Value =
                    serde_json::from_str(&rendered).map_err(|e| RunnerError::Extraction {
                        reason: e.to_string(),
                    })?;
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
            BodyFormat::Text => {
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
            },
            timeout_seconds: 30,
            adapter: storage::types::AdapterType::CustomRest,
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

        let masked = request_headers_for_log(&request);

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

        let masked = request_headers_for_log(&request);

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
            .map(|s| s.as_bytes().len() as u64)
            .unwrap_or(text.as_bytes().len() as u64)
    };

    match request.body.format {
        BodyFormat::Json => {
            let body = inject_session_body_field(
                request.body.content.clone(),
                &SessionConfig::None,
                session_value,
            );
            let text = serde_json::to_string(&body).unwrap_or_default();
            render_len(text)
        }
        BodyFormat::Form => {
            let text = serde_json::to_string(&request.body.content).unwrap_or_default();
            render_len(text)
        }
        BodyFormat::Text => {
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
