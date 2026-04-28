use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use storage::types::{
    AdapterType, AuthConfig, BodyConfig, BodyFormat, ExtractConfig, Request, ResponseConfig,
    SessionConfig, Target,
};
use storage::{requests, targets};
use tauri::State;

use super::AppPaths;
use crate::error::CommandError;

/// Flat target descriptor used by the UI — combines the Target YAML and its
/// associated Request YAML into one object so the frontend doesn't need two
/// round-trips.
///
/// Auth fields store env-var **names**, never the secret values themselves
/// (per CLAUDE.md invariant 11).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetDto {
    pub id: String,
    pub name: String,
    pub url: String,
    /// "openai_compat" | "custom_rest" | "raw_http"
    pub endpoint_type: String,
    /// "none" | "bearer" | "basic" | "api_key"
    pub auth_type: String,
    /// Name of the env var holding the bearer token or API key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_env: Option<String>,
    /// Custom header name (for auth_type == "api_key").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    /// "none" | "cookie" | "header" | "body_field"
    pub session_strategy: String,
    /// Header name or body field path for stateful sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_field: Option<String>,
    /// Request body field that receives the `{{prompt}}` (custom_rest only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_field: Option<String>,
    /// JSONPath into the response body (custom_rest only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

fn dto_to_pair(dto: &TargetDto) -> (Target, Request) {
    let adapter = match dto.endpoint_type.as_str() {
        "openai_compat" => AdapterType::OpenAiCompat,
        "raw_http" => AdapterType::RawHttp,
        _ => AdapterType::CustomRest,
    };

    let auth = match dto.auth_type.as_str() {
        "bearer" => AuthConfig::Bearer {
            token_env: dto
                .auth_env
                .clone()
                .unwrap_or_else(|| "HAMM0R_TOKEN".into()),
        },
        "basic" => AuthConfig::Basic {
            user_env: dto.auth_env.clone().unwrap_or_else(|| "HAMM0R_USER".into()),
            password_env: "HAMM0R_PASS".into(),
        },
        "api_key" => AuthConfig::CustomHeader {
            header_name: dto
                .auth_header
                .clone()
                .unwrap_or_else(|| "Authorization".into()),
            value_env: dto.auth_env.clone().unwrap_or_else(|| "HAMM0R_KEY".into()),
        },
        _ => AuthConfig::None,
    };

    let (body, extract) = match dto.endpoint_type.as_str() {
        "openai_compat" => (
            BodyConfig {
                format: BodyFormat::Json,
                content: serde_json::json!({
                    "messages": [{"role": "user", "content": "{{prompt}}"}]
                }),
            },
            ExtractConfig::Jsonpath {
                path: "$.choices[0].message.content".into(),
            },
        ),
        "raw_http" => (
            BodyConfig {
                format: BodyFormat::Text,
                content: serde_json::Value::String("{{prompt}}".into()),
            },
            ExtractConfig::Raw,
        ),
        _ => {
            let req_field = dto.request_field.as_deref().unwrap_or("prompt");
            let resp_field = dto.response_field.as_deref().unwrap_or("response");
            (
                BodyConfig {
                    format: BodyFormat::Json,
                    content: serde_json::json!({ req_field: "{{prompt}}" }),
                },
                ExtractConfig::Jsonpath {
                    path: format!("$.{resp_field}"),
                },
            )
        }
    };

    let session_config = match dto.session_strategy.as_str() {
        "cookie" => SessionConfig::Cookie,
        "header" => SessionConfig::Header {
            header_name: dto
                .session_field
                .clone()
                .unwrap_or_else(|| "X-Session-Id".into()),
        },
        "body_field" => SessionConfig::BodyField {
            field_name: dto
                .session_field
                .clone()
                .unwrap_or_else(|| "session_id".into()),
        },
        _ => SessionConfig::None,
    };

    let mut headers = HashMap::new();
    headers.insert("Content-Type".into(), "application/json".into());

    let request = Request {
        version: 1,
        id: dto.id.clone(),
        name: dto.name.clone(),
        method: "POST".into(),
        url: dto.url.clone(),
        auth,
        headers,
        body,
        response: ResponseConfig { extract },
        timeout_seconds: 30,
        adapter,
    };

    let target = Target {
        version: 1,
        id: dto.id.clone(),
        name: dto.name.clone(),
        request_id: dto.id.clone(),
        session_config,
        notes: dto.notes.clone(),
    };

    (target, request)
}

fn pair_to_dto(target: &Target, request: &Request) -> TargetDto {
    let endpoint_type = match request.adapter {
        AdapterType::OpenAiCompat => "openai_compat",
        AdapterType::RawHttp => "raw_http",
        AdapterType::CustomRest => "custom_rest",
    }
    .into();

    let (auth_type, auth_env, auth_header) = match &request.auth {
        AuthConfig::Bearer { token_env } => ("bearer".into(), Some(token_env.clone()), None),
        AuthConfig::Basic { user_env, .. } => ("basic".into(), Some(user_env.clone()), None),
        AuthConfig::CustomHeader {
            header_name,
            value_env,
        } => (
            "api_key".into(),
            Some(value_env.clone()),
            Some(header_name.clone()),
        ),
        AuthConfig::None => ("none".into(), None, None),
    };

    let (session_strategy, session_field) = match &target.session_config {
        SessionConfig::None => ("none".into(), None),
        SessionConfig::Cookie => ("cookie".into(), None),
        SessionConfig::Header { header_name } => ("header".into(), Some(header_name.clone())),
        SessionConfig::BodyField { field_name } => ("body_field".into(), Some(field_name.clone())),
    };

    // Try to extract request/response field names from the body content.
    let request_field = request
        .body
        .content
        .as_object()
        .and_then(|m| m.keys().find(|k| *k != "messages").cloned());
    let response_field = if let ExtractConfig::Jsonpath { path } = &request.response.extract {
        path.strip_prefix("$.").map(|s| s.to_owned())
    } else {
        None
    };

    TargetDto {
        id: target.id.clone(),
        name: target.name.clone(),
        url: request.url.clone(),
        endpoint_type,
        auth_type,
        auth_env,
        auth_header,
        session_strategy,
        session_field,
        request_field,
        response_field,
        notes: target.notes.clone(),
    }
}

/// Return all targets as flat DTOs, combining Target + Request data.
#[tauri::command]
pub fn list_targets(paths: State<'_, AppPaths>) -> Result<Vec<TargetDto>, CommandError> {
    let all_targets = targets::load_all(&paths.0.targets_dir())?;
    let all_requests = requests::load_all(&paths.0.requests_dir())?;
    let dtos = all_targets
        .values()
        .filter_map(|t| {
            let req = all_requests
                .get(&t.request_id)
                .or_else(|| all_requests.get(&t.id))?;
            Some(pair_to_dto(t, req))
        })
        .collect();
    Ok(dtos)
}

/// Create or update a target (and its backing request file).
#[tauri::command]
pub fn save_target(paths: State<'_, AppPaths>, dto: TargetDto) -> Result<TargetDto, CommandError> {
    let (target, request) = dto_to_pair(&dto);
    targets::save(&paths.0.targets_dir(), &target)?;
    requests::save(&paths.0.requests_dir(), &request)?;
    Ok(dto)
}

/// Delete a target and its backing request file.
#[tauri::command]
pub fn delete_target(paths: State<'_, AppPaths>, id: String) -> Result<(), CommandError> {
    targets::delete(&paths.0.targets_dir(), &id)?;
    requests::delete(&paths.0.requests_dir(), &id)?;
    Ok(())
}
