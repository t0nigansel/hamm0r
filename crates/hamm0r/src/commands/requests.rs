use std::collections::HashMap;

use runner::adapter::execute_with_session;
use runner::session::{SessionManager, SessionStrategy};
use serde::{Deserialize, Serialize};
use storage::requests::RequestReference;
use storage::types::Request;
use storage::{requests as request_store, targets};
use tauri::State;

use super::{AppConfigState, AppPaths, LoggerState};
use crate::error::CommandError;
use storage::types::AppConfig;

/// Returned to the UI when a delete is rejected because of references and
/// `force` was not set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeleteRequestBlockedDto {
    pub blocked: bool,
    pub references: Vec<RequestReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRequestResultDto {
    pub request_method: String,
    pub request_url: String,
    pub request_headers: HashMap<String, String>,
    pub request_body: String,
    pub status: u16,
    pub response_headers: HashMap<String, String>,
    pub raw_response_body: String,
    pub extracted_response_body: Option<String>,
    pub duration_ms: u64,
}

pub(crate) async fn run_test_request(
    config: &AppConfig,
    logger: &crate::logger::AppLogger,
    request: Request,
    session_strategy: String,
    session_field: Option<String>,
    prompt_text: Option<String>,
) -> Result<TestRequestResultDto, CommandError> {
    let strategy = parse_session_strategy(&session_strategy, session_field);
    let mut session_manager = SessionManager::new(strategy.clone(), request.timeout_seconds);
    let client = session_manager
        .client_for_with_timeout("test", request.timeout_seconds)
        .map_err(anyhow::Error::from)?;
    let payload = prompt_text.unwrap_or_default();
    let rendered_request = render_request_preview(&request, &strategy, "test", &payload)?;

    logger.info(
        "request-test",
        None,
        &format!(
            "Test request started method={} url={} adapter={:?}",
            request.method, request.url, request.adapter
        ),
    );

    let result = execute_with_session(client, &request, &payload, &strategy, "test")
        .await
        .map_err(|e| {
            logger.error("request-test", None, &format!("Test request failed: {e}"));
            anyhow::anyhow!(e)
        })?;

    let raw_response_body = String::from_utf8_lossy(&result.body_bytes).into_owned();

    log_test_request(
        logger,
        &request,
        &result.response_headers,
        result.status,
        result.duration_ms,
        result.extracted.as_deref(),
        &raw_response_body,
        config.logging.body_logging_enabled,
    );

    Ok(TestRequestResultDto {
        request_method: request.method.clone(),
        request_url: request.url.clone(),
        request_headers: rendered_request.headers,
        request_body: rendered_request.body,
        status: result.status,
        response_headers: result.response_headers,
        raw_response_body,
        extracted_response_body: result.extracted,
        duration_ms: result.duration_ms,
    })
}

struct RenderedRequestPreview {
    headers: HashMap<String, String>,
    body: String,
}

#[tauri::command]
pub fn get_request(
    paths: State<'_, AppPaths>,
    id: String,
) -> Result<Option<Request>, CommandError> {
    let all_requests = request_store::load_all(&paths.0.requests_dir())?;
    Ok(all_requests.get(&id).cloned())
}

#[tauri::command]
pub async fn test_request(
    config_state: State<'_, AppConfigState>,
    logger: State<'_, LoggerState>,
    request: Request,
    session_strategy: String,
    session_field: Option<String>,
    prompt_text: Option<String>,
) -> Result<TestRequestResultDto, CommandError> {
    run_test_request(
        &config_state.0,
        &logger.0,
        request,
        session_strategy,
        session_field,
        prompt_text,
    )
    .await
}

pub(crate) fn parse_session_strategy(strategy: &str, field: Option<String>) -> SessionStrategy {
    match strategy {
        "cookie" => SessionStrategy::Cookie,
        "header" => SessionStrategy::Header {
            header_name: field.unwrap_or_else(|| "X-Session-Id".to_owned()),
        },
        "body_field" => SessionStrategy::BodyField {
            field_name: field.unwrap_or_else(|| "session_id".to_owned()),
        },
        _ => SessionStrategy::None,
    }
}

#[allow(clippy::too_many_arguments)]
fn log_test_request(
    logger: &crate::logger::AppLogger,
    request: &Request,
    response_headers: &HashMap<String, String>,
    status: u16,
    duration_ms: u64,
    extracted: Option<&str>,
    raw_body: &str,
    body_logging_enabled: bool,
) {
    let mut lines = vec![
        "Test request completed".to_owned(),
        format!("request.method={}", request.method),
        format!("request.url={}", request.url),
        format!(
            "request.headers={}",
            format_headers(&request_headers_for_log(request))
        ),
        format!("response.status={status}"),
        format!("response.headers={}", format_headers(response_headers)),
        format!("duration_ms={duration_ms}"),
    ];

    if let Some(extracted) = extracted {
        lines.push("response.extracted:".to_owned());
        lines.push(extracted.to_owned());
    }
    if body_logging_enabled {
        lines.push("response.body:".to_owned());
        lines.push(limit_body_for_log(raw_body));
    }

    logger.info("request-test", None, &lines.join("\n"));
}

fn request_headers_for_log(request: &Request) -> HashMap<String, String> {
    let mut headers = request.headers.clone();
    redact_known_secret_headers(&mut headers);

    match &request.auth {
        storage::types::AuthConfig::Bearer { .. } => {
            upsert_masked_header(&mut headers, "Authorization", "Bearer <redacted>");
        }
        storage::types::AuthConfig::Basic { .. } => {
            upsert_masked_header(&mut headers, "Authorization", "Basic <redacted>");
        }
        storage::types::AuthConfig::CustomHeader { header_name, .. } => {
            upsert_masked_header(&mut headers, header_name, "<redacted>");
        }
        storage::types::AuthConfig::None => {}
    }

    headers
}

fn render_request_preview(
    request: &Request,
    session_strategy: &SessionStrategy,
    session_value: &str,
    prompt: &str,
) -> Result<RenderedRequestPreview, CommandError> {
    let mut headers = request_headers_for_log(request);
    if let SessionStrategy::Header { header_name } = session_strategy {
        headers.insert(header_name.clone(), session_value.to_owned());
    }
    let headers = runner::template::render_headers(&headers, prompt).map_err(anyhow::Error::from)?;

    let body = match request.adapter {
        storage::types::AdapterType::CustomRest => match request.body.format {
            storage::types::BodyFormat::Json => {
                let body = inject_session_body_field(
                    request.body.content.clone(),
                    session_strategy,
                    session_value,
                );
                let body_str = serde_json::to_string(&body).map_err(anyhow::Error::from)?;
                let rendered = runner::template::render(&body_str, prompt).map_err(anyhow::Error::from)?;
                let json: serde_json::Value =
                    serde_json::from_str(&rendered).map_err(anyhow::Error::from)?;
                serde_json::to_string_pretty(&json).map_err(anyhow::Error::from)?
            }
            storage::types::BodyFormat::Form => {
                let body_str =
                    serde_json::to_string(&request.body.content).map_err(anyhow::Error::from)?;
                runner::template::render(&body_str, prompt).map_err(anyhow::Error::from)?
            }
            storage::types::BodyFormat::Text | storage::types::BodyFormat::Raw => {
                let text = request.body.content.as_str().unwrap_or_default().to_owned();
                runner::template::render(&text, prompt).map_err(anyhow::Error::from)?
            }
        },
        storage::types::AdapterType::OpenAiCompat => {
            let mut body = request.body.content.clone();
            if let Some(messages) = body.get_mut("messages") {
                let msgs_str = serde_json::to_string(messages).map_err(anyhow::Error::from)?;
                let rendered = runner::template::render(&msgs_str, prompt).map_err(anyhow::Error::from)?;
                *messages = serde_json::from_str(&rendered).map_err(anyhow::Error::from)?;
            } else {
                body["messages"] = serde_json::json!([{"role": "user", "content": prompt}]);
            }
            let body = inject_session_body_field(body, session_strategy, session_value);
            serde_json::to_string_pretty(&body).map_err(anyhow::Error::from)?
        }
        storage::types::AdapterType::RawHttp => match &request.body.content {
            serde_json::Value::String(s) => {
                runner::template::render(s, prompt).map_err(anyhow::Error::from)?
            }
            other => runner::template::render(&other.to_string(), prompt).map_err(anyhow::Error::from)?,
        },
    };

    Ok(RenderedRequestPreview { headers, body })
}

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

fn format_headers(headers: &HashMap<String, String>) -> String {
    let mut pairs = headers
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>();
    pairs.sort();
    pairs.join(", ")
}

fn limit_body_for_log(body: &str) -> String {
    const MAX_LOG_BODY_BYTES: usize = 500 * 1024;
    let bytes = body.as_bytes();
    if bytes.len() <= MAX_LOG_BODY_BYTES {
        return body.to_owned();
    }
    format!(
        "Won't log the payload since the size is {:.1}kb",
        bytes.len() as f64 / 1024.0
    )
}

// ── Top-level Request CRUD (independent of Target) ────────────────────────────
// These commands back the new "Requests" menu item. The Target-scoped
// `save_request` / `delete_request` above are retained for the Target editor
// flow.

#[tauri::command]
pub fn list_request_references(
    paths: State<'_, AppPaths>,
    id: String,
) -> Result<Vec<RequestReference>, CommandError> {
    request_store::references(&paths.0.targets_dir(), &paths.0.scenarios_dir(), &id)
        .map_err(Into::into)
}

#[tauri::command]
pub fn save_request_global(
    paths: State<'_, AppPaths>,
    mut request: Request,
) -> Result<Request, CommandError> {
    if request.version == 0 {
        request.version = 1;
    }
    if request.id.trim().is_empty() {
        return Err(anyhow::anyhow!("request id must not be empty").into());
    }
    if request.name.trim().is_empty() {
        request.name = request.id.clone();
    }
    request_store::save(&paths.0.requests_dir(), &request)?;
    Ok(request)
}

/// Delete a Request file. When `force` is false and references exist, the
/// command returns a `DeleteRequestBlockedDto` describing the blocking
/// references — the UI surfaces a confirmation dialog and re-invokes with
/// `force = true`. With `force = true`, referencing Targets are cleaned up
/// (the id is removed from `request_ids`, primary is reset if needed).
/// Scenario steps are intentionally not modified — the UI surfaces a
/// "missing request" warning on affected scenarios at edit/run time.
#[tauri::command]
pub fn delete_request_global(
    paths: State<'_, AppPaths>,
    id: String,
    force: bool,
) -> Result<DeleteRequestBlockedDto, CommandError> {
    let refs = request_store::references(&paths.0.targets_dir(), &paths.0.scenarios_dir(), &id)?;

    if !refs.is_empty() && !force {
        return Ok(DeleteRequestBlockedDto {
            blocked: true,
            references: refs,
        });
    }

    // Clean references in Targets when the user has confirmed.
    if force {
        let all_targets = targets::load_all(&paths.0.targets_dir())?;
        for (_, mut target) in all_targets {
            let mut changed = false;
            if target.request_ids.iter().any(|r| r == &id) {
                target.request_ids.retain(|r| r != &id);
                changed = true;
            }
            if target.request_id == id {
                target.request_id = target.request_ids.first().cloned().unwrap_or_default();
                changed = true;
            }
            if changed {
                targets::save(&paths.0.targets_dir(), &target)?;
            }
        }
    }

    request_store::delete(&paths.0.requests_dir(), &id)?;
    Ok(DeleteRequestBlockedDto {
        blocked: false,
        references: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        parse_session_strategy, render_request_preview, request_headers_for_log,
    };
    use runner::session::SessionStrategy;
    use std::collections::HashMap;
    use storage::types::{
        AuthConfig, BodyConfig, BodyFormat, ExtractConfig, Request, ResponseConfig,
    };

    #[test]
    fn parse_session_strategy_maps_header_mode() {
        match parse_session_strategy("header", Some("X-Test".to_owned())) {
            SessionStrategy::Header { header_name } => assert_eq!(header_name, "X-Test"),
            _ => panic!("expected header strategy"),
        }
    }

    #[test]
    fn request_headers_for_log_masks_auth_values() {
        let request = Request {
            version: 1,
            id: "req".into(),
            name: "Req".into(),
            method: "POST".into(),
            url: "https://example.test".into(),
            auth: AuthConfig::Bearer {
                token_env: "TOKEN".into(),
            },
            headers: HashMap::from([
                ("Authorization".into(), "Bearer secret".into()),
                ("Content-Type".into(), "application/json".into()),
            ]),
            body: BodyConfig {
                format: BodyFormat::Json,
                content: serde_json::json!({"message":"{{prompt}}"}),
            },
            response: ResponseConfig {
                extract: ExtractConfig::Raw,
                bind: None,
            },
            timeout_seconds: 50,
            adapter: Default::default(),
            tag: None,
        };

        let headers = request_headers_for_log(&request);
        assert_eq!(
            headers.get("Authorization"),
            Some(&"Bearer <redacted>".to_owned())
        );
    }

    #[test]
    fn render_request_preview_masks_bearer_and_substitutes_prompt() {
        let request = Request {
            version: 1,
            id: "req".into(),
            name: "Req".into(),
            method: "POST".into(),
            url: "https://example.test".into(),
            auth: AuthConfig::Bearer {
                token_env: "TOKEN".into(),
            },
            headers: HashMap::from([("Content-Type".into(), "application/json".into())]),
            body: BodyConfig {
                format: BodyFormat::Json,
                content: serde_json::json!({"message":"{{prompt}}"}),
            },
            response: ResponseConfig {
                extract: ExtractConfig::Raw,
                bind: None,
            },
            timeout_seconds: 50,
            adapter: Default::default(),
            tag: None,
        };

        let preview = render_request_preview(&request, &SessionStrategy::None, "test", "hello")
            .expect("preview should render");

        assert_eq!(
            preview.headers.get("Authorization"),
            Some(&"Bearer <redacted>".to_owned())
        );
        assert!(preview.body.contains("\"hello\""));
    }
}
