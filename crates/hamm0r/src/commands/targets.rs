use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use storage::types::{
    AdapterType, AuthAcquisitionConfig, AuthAcquisitionMode, AuthConfig, ExtractConfig, Request,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub request_ids: Vec<String>,
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
    /// HTTP request timeout in seconds. Defaults to 50 if not provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    /// "manual" | "env_only" | "http_login"
    #[serde(default = "default_auth_source")]
    pub auth_source: String,
    /// Name of the env var holding the login or email for HTTP auth acquisition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_env: Option<String>,
    /// Name of the env var holding the password for HTTP auth acquisition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_password_env: Option<String>,
    /// Login endpoint URL for HTTP auth acquisition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_url: Option<String>,
    /// HTTP method used for the login request. Defaults to POST when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_method: Option<String>,
    /// Static headers for the login request.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub auth_login_headers: HashMap<String, String>,
    /// Raw body template for the login request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_body_template: Option<String>,
    /// JSONPath used to extract the bearer token from the login response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token_json_path: Option<String>,
    /// Timeout for the login request in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_timeout_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}


fn default_auth_source() -> String {
    "manual".to_owned()
}


fn auth_config_to_source(config: &AuthAcquisitionConfig) -> AuthSourceFields {
    match config.mode {
        AuthAcquisitionMode::EnvOnly => AuthSourceFields {
            auth_source: "env_only".to_owned(),
            ..AuthSourceFields::default()
        },
        AuthAcquisitionMode::HttpLogin => {
            let login = config.http_login.as_ref();
            AuthSourceFields {
                auth_source: "http_login".to_owned(),
                auth_login_env: login.and_then(|cfg| cfg.login_env.clone()),
                auth_password_env: login.and_then(|cfg| cfg.password_env.clone()),
                auth_login_url: login.and_then(|cfg| cfg.url.clone()),
                auth_login_method: login.and_then(|cfg| cfg.method.clone()),
                auth_login_headers: login.map(|cfg| cfg.headers.clone()).unwrap_or_default(),
                auth_login_body_template: login.and_then(|cfg| cfg.body_template.clone()),
                auth_token_json_path: login.and_then(|cfg| cfg.token_json_path.clone()),
                auth_login_timeout_seconds: login.and_then(|cfg| cfg.timeout_seconds),
            }
        }
        AuthAcquisitionMode::Manual => AuthSourceFields::manual(),
    }
}

#[derive(Debug, Clone)]
struct AuthSourceFields {
    auth_source: String,
    auth_login_env: Option<String>,
    auth_password_env: Option<String>,
    auth_login_url: Option<String>,
    auth_login_method: Option<String>,
    auth_login_headers: HashMap<String, String>,
    auth_login_body_template: Option<String>,
    auth_token_json_path: Option<String>,
    auth_login_timeout_seconds: Option<u32>,
}

impl Default for AuthSourceFields {
    fn default() -> Self {
        Self::manual()
    }
}

impl AuthSourceFields {
    fn manual() -> Self {
        Self {
            auth_source: default_auth_source(),
            auth_login_env: None,
            auth_password_env: None,
            auth_login_url: None,
            auth_login_method: None,
            auth_login_headers: HashMap::new(),
            auth_login_body_template: None,
            auth_token_json_path: None,
            auth_login_timeout_seconds: None,
        }
    }
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
    let auth_fields = auth_config_to_source(&target.auth_acquisition);

    TargetDto {
        id: target.id.clone(),
        name: target.name.clone(),
        request_ids: if target.request_ids.is_empty() {
            target
                .primary_request_id()
                .map(|id| vec![id.to_owned()])
                .unwrap_or_default()
        } else {
            target.request_ids.clone()
        },
        url: request.url.clone(),
        endpoint_type,
        auth_type,
        auth_env,
        auth_header,
        session_strategy,
        session_field,
        request_field,
        response_field,
        timeout_seconds: Some(request.timeout_seconds),
        auth_source: auth_fields.auth_source,
        auth_login_env: auth_fields.auth_login_env,
        auth_password_env: auth_fields.auth_password_env,
        auth_login_url: auth_fields.auth_login_url,
        auth_login_method: auth_fields.auth_login_method,
        auth_login_headers: auth_fields.auth_login_headers,
        auth_login_body_template: auth_fields.auth_login_body_template,
        auth_token_json_path: auth_fields.auth_token_json_path,
        auth_login_timeout_seconds: auth_fields.auth_login_timeout_seconds,
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
            let request_id = t.primary_request_id().unwrap_or(t.id.as_str());
            let req = all_requests
                .get(request_id)
                .or_else(|| all_requests.get(&t.id))?;
            Some(pair_to_dto(t, req))
        })
        .collect();
    Ok(dtos)
}

