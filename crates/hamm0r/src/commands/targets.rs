use std::collections::HashMap;

use reqwest::Method;
use serde_json_path::JsonPath;
use serde::{Deserialize, Serialize};
use storage::types::{
    AdapterType, AuthAcquisitionConfig, AuthAcquisitionMode, AuthConfig, BodyConfig,
    BodyFormat, ExtractConfig, HttpLoginConfig, Request, ResponseConfig, SessionConfig, Target,
};
use storage::{requests, targets};
use tauri::State;

use super::AppPaths;
use super::{AppConfigState, LoggerState};
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
    /// HTTP request timeout in seconds. Defaults to 30 if not provided.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetMetaDto {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub request_ids: Vec<String>,
    /// "none" | "cookie" | "header" | "body_field"
    pub session_strategy: String,
    /// Header name or body field path for stateful sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_field: Option<String>,
    /// "manual" | "env_only" | "http_login"
    #[serde(default = "default_auth_source")]
    pub auth_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_password_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_method: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub auth_login_headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_body_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token_json_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_timeout_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquireTargetAuthRequestDto {
    pub auth_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_env: Option<String>,
    #[serde(default = "default_auth_source")]
    pub auth_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_password_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_method: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub auth_login_headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_body_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token_json_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_login_timeout_seconds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquireTargetAuthResultDto {
    pub ok: bool,
    pub stored_var: String,
    pub message: String,
}

fn default_auth_source() -> String {
    "manual".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestTargetConnectionResultDto {
    pub ok: bool,
    pub status: u16,
    pub response_headers: HashMap<String, String>,
    pub raw_response_body: String,
    pub extracted_response_body: Option<String>,
    pub duration_ms: u64,
    pub message: String,
}

fn auth_source_to_config(dto: &TargetMetaDto) -> AuthAcquisitionConfig {
    match dto.auth_source.as_str() {
        "env_only" => AuthAcquisitionConfig {
            mode: AuthAcquisitionMode::EnvOnly,
            http_login: None,
        },
        "http_login" => AuthAcquisitionConfig {
            mode: AuthAcquisitionMode::HttpLogin,
            http_login: Some(HttpLoginConfig {
                login_env: dto.auth_login_env.clone(),
                password_env: dto.auth_password_env.clone(),
                url: dto.auth_login_url.clone(),
                method: dto.auth_login_method.clone(),
                headers: dto.auth_login_headers.clone(),
                body_template: dto.auth_login_body_template.clone(),
                token_json_path: dto.auth_token_json_path.clone(),
                timeout_seconds: dto.auth_login_timeout_seconds,
            }),
        },
        _ => AuthAcquisitionConfig::default(),
    }
}

fn auth_config_to_source(
    config: &AuthAcquisitionConfig,
) -> (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    HashMap<String, String>,
    Option<String>,
    Option<String>,
    Option<u32>,
) {
    match config.mode {
        AuthAcquisitionMode::EnvOnly => (
            "env_only".to_owned(),
            None,
            None,
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
        ),
        AuthAcquisitionMode::HttpLogin => {
            let login = config.http_login.as_ref();
            (
                "http_login".to_owned(),
                login.and_then(|cfg| cfg.login_env.clone()),
                login.and_then(|cfg| cfg.password_env.clone()),
                login.and_then(|cfg| cfg.url.clone()),
                login.and_then(|cfg| cfg.method.clone()),
                login.map(|cfg| cfg.headers.clone()).unwrap_or_default(),
                login.and_then(|cfg| cfg.body_template.clone()),
                login.and_then(|cfg| cfg.token_json_path.clone()),
                login.and_then(|cfg| cfg.timeout_seconds),
            )
        }
        AuthAcquisitionMode::Manual => (
            "manual".to_owned(),
            None,
            None,
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
        ),
    }
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
        timeout_seconds: dto.timeout_seconds.unwrap_or(30),
        adapter,
    };

    let target = Target {
        version: 1,
        id: dto.id.clone(),
        name: dto.name.clone(),
        request_ids: vec![dto.id.clone()],
        request_id: dto.id.clone(),
        session_config,
        auth_acquisition: auth_source_to_config(&TargetMetaDto {
            id: dto.id.clone(),
            name: dto.name.clone(),
            request_ids: dto.request_ids.clone(),
            session_strategy: dto.session_strategy.clone(),
            session_field: dto.session_field.clone(),
            auth_source: dto.auth_source.clone(),
            auth_login_env: dto.auth_login_env.clone(),
            auth_password_env: dto.auth_password_env.clone(),
            auth_login_url: dto.auth_login_url.clone(),
            auth_login_method: dto.auth_login_method.clone(),
            auth_login_headers: dto.auth_login_headers.clone(),
            auth_login_body_template: dto.auth_login_body_template.clone(),
            auth_token_json_path: dto.auth_token_json_path.clone(),
            auth_login_timeout_seconds: dto.auth_login_timeout_seconds,
            notes: dto.notes.clone(),
        }),
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
    let (
        auth_source,
        auth_login_env,
        auth_password_env,
        auth_login_url,
        auth_login_method,
        auth_login_headers,
        auth_login_body_template,
        auth_token_json_path,
        auth_login_timeout_seconds,
    ) = auth_config_to_source(&target.auth_acquisition);

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
        auth_source,
        auth_login_env,
        auth_password_env,
        auth_login_url,
        auth_login_method,
        auth_login_headers,
        auth_login_body_template,
        auth_token_json_path,
        auth_login_timeout_seconds,
        notes: target.notes.clone(),
    }
}

fn meta_to_target(dto: &TargetMetaDto, existing: Option<&Target>) -> Target {
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

    let mut request_ids = dto.request_ids.clone();
    if request_ids.is_empty() {
        if let Some(existing) = existing {
            request_ids = if existing.request_ids.is_empty() {
                existing
                    .primary_request_id()
                    .map(|id| vec![id.to_owned()])
                    .unwrap_or_default()
            } else {
                existing.request_ids.clone()
            };
        }
    }

    let primary_request_id = existing
        .and_then(Target::primary_request_id)
        .map(str::to_owned)
        .or_else(|| request_ids.first().cloned())
        .unwrap_or_default();

    Target {
        version: 1,
        id: dto.id.clone(),
        name: dto.name.clone(),
        request_ids,
        request_id: primary_request_id,
        session_config,
        auth_acquisition: auth_source_to_config(dto),
        notes: dto.notes.clone(),
    }
}

fn target_to_meta(target: &Target) -> TargetMetaDto {
    let (session_strategy, session_field) = match &target.session_config {
        SessionConfig::None => ("none".into(), None),
        SessionConfig::Cookie => ("cookie".into(), None),
        SessionConfig::Header { header_name } => ("header".into(), Some(header_name.clone())),
        SessionConfig::BodyField { field_name } => ("body_field".into(), Some(field_name.clone())),
    };

    let (
        auth_source,
        auth_login_env,
        auth_password_env,
        auth_login_url,
        auth_login_method,
        auth_login_headers,
        auth_login_body_template,
        auth_token_json_path,
        auth_login_timeout_seconds,
    ) = auth_config_to_source(&target.auth_acquisition);

    TargetMetaDto {
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
        session_strategy,
        session_field,
        auth_source,
        auth_login_env,
        auth_password_env,
        auth_login_url,
        auth_login_method,
        auth_login_headers,
        auth_login_body_template,
        auth_token_json_path,
        auth_login_timeout_seconds,
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

/// Create or update a target (and its backing request file).
#[tauri::command]
pub fn save_target(paths: State<'_, AppPaths>, dto: TargetDto) -> Result<TargetDto, CommandError> {
    let (mut target, request) = dto_to_pair(&dto);
    if dto.request_ids.is_empty() {
        let mut existing_targets = targets::load_all(&paths.0.targets_dir())?;
        if let Some(existing) = existing_targets.remove(&dto.id) {
            if existing.request_ids.is_empty() {
                target.request_ids = existing
                    .primary_request_id()
                    .map(|id| vec![id.to_owned()])
                    .unwrap_or_else(|| vec![request.id.clone()]);
            } else {
                target.request_ids = existing.request_ids;
            }
        } else {
            target.request_ids = vec![request.id.clone()];
        }
    } else {
        target.request_ids = dto.request_ids.clone();
    }
    if target.request_id.trim().is_empty() {
        target.request_id = target
            .request_ids
            .first()
            .cloned()
            .unwrap_or_else(|| request.id.clone());
    }
    targets::save(&paths.0.targets_dir(), &target)?;
    requests::save(&paths.0.requests_dir(), &request)?;
    Ok(dto)
}

#[tauri::command]
pub async fn test_target_connection(
    config_state: State<'_, AppConfigState>,
    logger: State<'_, LoggerState>,
    dto: TargetDto,
    prompt_text: Option<String>,
) -> Result<TestTargetConnectionResultDto, CommandError> {
    let (target, request) = dto_to_pair(&dto);
    let (session_strategy, session_field) = match &target.session_config {
        SessionConfig::None => ("none".to_owned(), None),
        SessionConfig::Cookie => ("cookie".to_owned(), None),
        SessionConfig::Header { header_name } => ("header".to_owned(), Some(header_name.clone())),
        SessionConfig::BodyField { field_name } => {
            ("body_field".to_owned(), Some(field_name.clone()))
        }
    };

    let result = super::requests::run_test_request(
        &config_state.0,
        &logger.0,
        request,
        session_strategy,
        session_field,
        prompt_text.or_else(|| Some("connection test".to_owned())),
    )
    .await?;

    let ok = (200..400).contains(&result.status);
    let message = if ok {
        format!(
            "Connection test passed: HTTP {} in {} ms.",
            result.status, result.duration_ms
        )
    } else {
        format!(
            "Connection test failed: HTTP {} in {} ms.",
            result.status, result.duration_ms
        )
    };

    Ok(TestTargetConnectionResultDto {
        ok,
        status: result.status,
        response_headers: result.response_headers,
        raw_response_body: result.raw_response_body,
        extracted_response_body: result.extracted_response_body,
        duration_ms: result.duration_ms,
        message,
    })
}

#[tauri::command]
pub fn get_target_meta(
    paths: State<'_, AppPaths>,
    id: String,
) -> Result<Option<TargetMetaDto>, CommandError> {
    let all_targets = targets::load_all(&paths.0.targets_dir())?;
    Ok(all_targets.get(&id).map(target_to_meta))
}

#[tauri::command]
pub fn save_target_meta(
    paths: State<'_, AppPaths>,
    dto: TargetMetaDto,
) -> Result<TargetMetaDto, CommandError> {
    let all_targets = targets::load_all(&paths.0.targets_dir())?;
    let target = meta_to_target(&dto, all_targets.get(&dto.id));
    targets::save(&paths.0.targets_dir(), &target)?;
    Ok(target_to_meta(&target))
}

fn validate_acquire_target_auth_request(
    dto: &AcquireTargetAuthRequestDto,
) -> anyhow::Result<(String, HttpLoginConfig)> {
    if dto.auth_type != "bearer" {
        anyhow::bail!("Fetch token currently supports bearer auth targets only");
    }

    let token_env = dto
        .auth_env
        .clone()
        .unwrap_or_default()
        .trim()
        .to_owned();
    if token_env.is_empty() {
        anyhow::bail!("Auth env var name is required");
    }

    if dto.auth_source != "http_login" {
        anyhow::bail!("Fetch token requires auth source 'HTTP login'");
    }

    let config = HttpLoginConfig {
        login_env: dto.auth_login_env.clone(),
        password_env: dto.auth_password_env.clone(),
        url: dto.auth_login_url.clone(),
        method: dto.auth_login_method.clone(),
        headers: dto.auth_login_headers.clone(),
        body_template: dto.auth_login_body_template.clone(),
        token_json_path: dto.auth_token_json_path.clone(),
        timeout_seconds: dto.auth_login_timeout_seconds,
    };

    if config
        .login_env
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        anyhow::bail!("Login env var name is required");
    }
    if config
        .password_env
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        anyhow::bail!("Password env var name is required");
    }
    if config.url.as_deref().unwrap_or_default().trim().is_empty() {
        anyhow::bail!("Login URL is required");
    }
    if config
        .token_json_path
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        anyhow::bail!("Token JSONPath is required");
    }

    Ok((token_env, config))
}

fn read_env_secret(var_name: &str, label: &str) -> anyhow::Result<String> {
    std::env::var(var_name)
        .map_err(|_| anyhow::anyhow!("{label} env var not set: {var_name}"))
        .and_then(|value| {
            if value.trim().is_empty() {
                Err(anyhow::anyhow!("{label} env var is empty: {var_name}"))
            } else {
                Ok(value)
            }
        })
}

fn escape_json_template_value(value: &str) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"\"".to_owned())
        .trim_matches('"')
        .to_owned()
}

fn render_http_login_body(template: &str, login: &str, password: &str) -> String {
    template
        .replace("{{login}}", &escape_json_template_value(login))
        .replace("{{password}}", &escape_json_template_value(password))
}

fn extract_bearer_token_from_json(body: &str, path: &str) -> anyhow::Result<String> {
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("Login response is not valid JSON: {e}"))?;
    let json_path =
        JsonPath::parse(path).map_err(|e| anyhow::anyhow!("Invalid token JSONPath '{path}': {e}"))?;
    let value = json_path
        .query(&json)
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Token field not found in response"))?;

    match value {
        serde_json::Value::String(token) if !token.trim().is_empty() => Ok(token),
        serde_json::Value::String(_) => Err(anyhow::anyhow!("Extracted token is empty")),
        other => {
            let rendered = other.to_string();
            if rendered.trim().is_empty() {
                Err(anyhow::anyhow!("Extracted token is empty"))
            } else {
                Ok(rendered)
            }
        }
    }
}

#[tauri::command]
pub async fn acquire_target_auth(
    logger: State<'_, LoggerState>,
    dto: AcquireTargetAuthRequestDto,
) -> Result<AcquireTargetAuthResultDto, CommandError> {
    let (token_env, config) = validate_acquire_target_auth_request(&dto)?;
    let login_env = config.login_env.as_deref().unwrap_or_default().to_owned();
    let password_env = config.password_env.as_deref().unwrap_or_default().to_owned();
    let login = read_env_secret(&login_env, "Login")?;
    let password = read_env_secret(&password_env, "Password")?;
    let url = config.url.clone().unwrap_or_default();
    let method_text = config
        .method
        .clone()
        .unwrap_or_else(|| "POST".to_owned());
    let method = Method::from_bytes(method_text.as_bytes())
        .map_err(|_| anyhow::anyhow!("Invalid login HTTP method: {method_text}"))?;
    let timeout_seconds = config.timeout_seconds.unwrap_or(60);

    logger.0.info(
        "auth-fetch",
        None,
        &format!(
            "Token fetch started method={} url={} token_env={} login_env={} timeout_seconds={}",
            method, url, token_env, login_env, timeout_seconds
        ),
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(u64::from(timeout_seconds)))
        .build()
        .map_err(|e| anyhow::anyhow!("Couldn't prepare login HTTP client: {e}"))?;

    let mut builder = client.request(method, &url);
    for (name, value) in &config.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }

    if let Some(body_template) = config.body_template.as_deref() {
        let body = render_http_login_body(body_template, &login, &password);
        builder = builder.body(body);
    }

    let response = builder
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Token fetch failed: {e}"))?;
    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Couldn't read login response: {e}"))?;

    if !status.is_success() {
        logger.0.error(
            "auth-fetch",
            None,
            &format!(
                "Token fetch failed method={} url={} status={}",
                method_text, url, status
            ),
        );
        return Err(anyhow::anyhow!("Token fetch failed: HTTP {status}").into());
    }

    let token = extract_bearer_token_from_json(
        &response_text,
        config.token_json_path.as_deref().unwrap_or_default(),
    )?;
    storage::secrets::set_token(&token_env, &token)?;

    logger.0.info(
        "auth-fetch",
        None,
        &format!(
            "Token fetch completed method={} url={} status={} token_env={}",
            method_text, url, status, token_env
        ),
    );

    Ok(AcquireTargetAuthResultDto {
        ok: true,
        stored_var: token_env.clone(),
        message: format!("Token stored in keychain for {token_env}"),
    })
}

/// Delete a target and its backing request file.
#[tauri::command]
pub fn delete_target(paths: State<'_, AppPaths>, id: String) -> Result<(), CommandError> {
    targets::delete(&paths.0.targets_dir(), &id)?;
    requests::delete(&paths.0.requests_dir(), &id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        extract_bearer_token_from_json, render_http_login_body,
        validate_acquire_target_auth_request, AcquireTargetAuthRequestDto,
    };
    use std::collections::HashMap;

    fn sample_acquire_request() -> AcquireTargetAuthRequestDto {
        AcquireTargetAuthRequestDto {
            auth_type: "bearer".into(),
            auth_env: Some("PROFILER_BEARER_TOKEN".into()),
            auth_source: "http_login".into(),
            auth_login_env: Some("PROFILER_LOGIN".into()),
            auth_password_env: Some("PROFILER_PASSWORD".into()),
            auth_login_url: Some("https://example.test/api/auth/login".into()),
            auth_login_method: Some("POST".into()),
            auth_login_headers: HashMap::from([(
                "Content-Type".into(),
                "application/json".into(),
            )]),
            auth_login_body_template: Some(
                "{\"email\":\"{{login}}\",\"password\":\"{{password}}\"}".into(),
            ),
            auth_token_json_path: Some("$.jwToken".into()),
            auth_login_timeout_seconds: Some(60),
        }
    }

    #[test]
    fn validate_acquire_request_accepts_complete_http_login_config() {
        let (token_env, config) =
            validate_acquire_target_auth_request(&sample_acquire_request()).unwrap();
        assert_eq!(token_env, "PROFILER_BEARER_TOKEN");
        assert_eq!(config.login_env.as_deref(), Some("PROFILER_LOGIN"));
        assert_eq!(config.token_json_path.as_deref(), Some("$.jwToken"));
    }

    #[test]
    fn validate_acquire_request_rejects_non_bearer_targets() {
        let mut request = sample_acquire_request();
        request.auth_type = "api_key".into();
        assert!(validate_acquire_target_auth_request(&request).is_err());
    }

    #[test]
    fn render_http_login_body_escapes_json_string_values() {
        let rendered = render_http_login_body(
            "{\"email\":\"{{login}}\",\"password\":\"{{password}}\"}",
            "user\"name@example.test",
            "pa\\ss",
        );
        assert_eq!(
            rendered,
            "{\"email\":\"user\\\"name@example.test\",\"password\":\"pa\\\\ss\"}"
        );
    }

    #[test]
    fn extract_bearer_token_from_json_reads_string_value() {
        let token = extract_bearer_token_from_json(
            "{\"jwToken\":\"secret-token\",\"identityDTO\":{\"email\":\"qa@example.test\"}}",
            "$.jwToken",
        )
        .unwrap();
        assert_eq!(token, "secret-token");
    }

    #[test]
    fn extract_bearer_token_from_json_errors_when_path_missing() {
        let err = extract_bearer_token_from_json("{\"other\":\"value\"}", "$.jwToken")
            .expect_err("path should be missing");
        assert!(err.to_string().contains("Token field not found"));
    }
}
