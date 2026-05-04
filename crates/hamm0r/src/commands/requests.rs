use std::collections::HashMap;

use runner::adapter::execute_with_session;
use runner::session::{SessionManager, SessionStrategy};
use serde::{Deserialize, Serialize};
use storage::types::Request;
use storage::{requests as request_store, targets};
use tauri::State;

use super::{AppConfigState, AppPaths, LoggerState};
use crate::error::CommandError;
use storage::types::AppConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRecordDto {
    pub target_id: String,
    pub primary: bool,
    pub request: Request,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRequestResultDto {
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
        status: result.status,
        response_headers: result.response_headers,
        raw_response_body,
        extracted_response_body: result.extracted,
        duration_ms: result.duration_ms,
    })
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
pub fn list_target_requests(
    paths: State<'_, AppPaths>,
    target_id: String,
) -> Result<Vec<RequestRecordDto>, CommandError> {
    let all_targets = targets::load_all(&paths.0.targets_dir())?;
    let target = all_targets
        .get(&target_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("target '{}' not found", target_id))?;
    let all_requests = request_store::load_all(&paths.0.requests_dir())?;

    let primary_id = target.primary_request_id().map(str::to_owned);
    let ids = normalized_request_ids(&target);
    let records = ids
        .into_iter()
        .filter_map(|request_id| {
            let request = all_requests.get(&request_id)?.clone();
            Some(RequestRecordDto {
                target_id: target.id.clone(),
                primary: primary_id.as_deref() == Some(request_id.as_str()),
                request,
            })
        })
        .collect();

    Ok(records)
}

#[tauri::command]
pub fn save_request(
    paths: State<'_, AppPaths>,
    target_id: String,
    mut request: Request,
) -> Result<RequestRecordDto, CommandError> {
    if request.version == 0 {
        request.version = 1;
    }
    if request.id.trim().is_empty() {
        return Err(anyhow::anyhow!("request id must not be empty").into());
    }
    if request.name.trim().is_empty() {
        request.name = request.id.clone();
    }

    let mut all_targets = targets::load_all(&paths.0.targets_dir())?;
    let mut target = all_targets
        .remove(&target_id)
        .ok_or_else(|| anyhow::anyhow!("target '{}' not found", target_id))?;

    let mut request_ids = normalized_request_ids(&target);
    if !request_ids.iter().any(|id| id == &request.id) {
        request_ids.push(request.id.clone());
    }
    if target.request_id.trim().is_empty() {
        target.request_id = request.id.clone();
    }
    target.request_ids = request_ids;

    request_store::save(&paths.0.requests_dir(), &request)?;
    targets::save(&paths.0.targets_dir(), &target)?;

    let is_primary = target.primary_request_id() == Some(request.id.as_str());
    Ok(RequestRecordDto {
        target_id: target.id,
        primary: is_primary,
        request,
    })
}

#[tauri::command]
pub fn delete_request(
    paths: State<'_, AppPaths>,
    target_id: String,
    id: String,
) -> Result<(), CommandError> {
    let mut all_targets = targets::load_all(&paths.0.targets_dir())?;
    let mut target = all_targets
        .remove(&target_id)
        .ok_or_else(|| anyhow::anyhow!("target '{}' not found", target_id))?;

    let mut request_ids = normalized_request_ids(&target);
    request_ids.retain(|request_id| request_id != &id);
    target.request_ids = request_ids;

    if target.request_id == id {
        target.request_id = target.request_ids.first().cloned().unwrap_or_default();
    }

    request_store::delete(&paths.0.requests_dir(), &id)?;
    targets::save(&paths.0.targets_dir(), &target)?;
    Ok(())
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

fn normalized_request_ids(target: &storage::types::Target) -> Vec<String> {
    let mut ids = Vec::new();

    if !target.request_id.trim().is_empty() {
        ids.push(target.request_id.clone());
    }
    for request_id in &target.request_ids {
        if request_id.trim().is_empty() {
            continue;
        }
        if ids.iter().any(|existing| existing == request_id) {
            continue;
        }
        ids.push(request_id.clone());
    }

    ids
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

#[cfg(test)]
mod tests {
    use super::{normalized_request_ids, parse_session_strategy, request_headers_for_log};
    use runner::session::SessionStrategy;
    use std::collections::HashMap;
    use storage::types::{
        AuthConfig, BodyConfig, BodyFormat, ExtractConfig, Request, ResponseConfig, Target,
    };

    #[test]
    fn normalized_request_ids_prefers_primary_and_deduplicates() {
        let target = Target {
            version: 1,
            id: "acme".into(),
            name: "Acme".into(),
            request_ids: vec!["secondary".into(), "primary".into(), "secondary".into()],
            request_id: "primary".into(),
            session_config: Default::default(),
            notes: None,
        };

        assert_eq!(
            normalized_request_ids(&target),
            vec!["primary".to_owned(), "secondary".to_owned(),]
        );
    }

    #[test]
    fn normalized_request_ids_falls_back_to_request_ids_for_legacy_empty_primary() {
        let target = Target {
            version: 1,
            id: "acme".into(),
            name: "Acme".into(),
            request_ids: vec!["secondary".into()],
            request_id: String::new(),
            session_config: Default::default(),
            notes: None,
        };

        assert_eq!(
            normalized_request_ids(&target),
            vec!["secondary".to_owned()]
        );
    }

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
            },
            timeout_seconds: 30,
            adapter: Default::default(),
        };

        let headers = request_headers_for_log(&request);
        assert_eq!(
            headers.get("Authorization"),
            Some(&"Bearer <redacted>".to_owned())
        );
    }
}
