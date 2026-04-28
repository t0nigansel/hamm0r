use std::collections::HashMap;
use std::time::Instant;

use bytes::Bytes;
use serde_json_path::JsonPath;
use storage::types::{AdapterType, AuthConfig, BodyFormat, ExtractConfig, Request};

use crate::error::RunnerError;
use crate::session::SessionStrategy;
use crate::template;

/// The result of one HTTP exchange.
pub struct AdapterResponse {
    pub status: u16,
    pub response_headers: HashMap<String, String>,
    pub body_bytes: Bytes,
    /// The portion of the response that the analyzer evaluates — extracted
    /// according to `request.response.extract`.
    pub extracted: Option<String>,
    pub duration_ms: u64,
    pub first_byte_ms: Option<u64>,
}

/// Apply auth config to a request builder, reading secrets from env vars.
///
/// Invariant: secrets never appear in logs or in the request body stored on
/// disk. Auth headers are hashed before storage (see `run::headers_hash`).
fn apply_auth(
    mut builder: reqwest::RequestBuilder,
    auth: &AuthConfig,
) -> Result<reqwest::RequestBuilder, RunnerError> {
    match auth {
        AuthConfig::Bearer { token_env } => {
            let token = std::env::var(token_env).map_err(|_| RunnerError::MissingEnvVar {
                var: token_env.clone(),
            })?;
            builder = builder.bearer_auth(token);
        }
        AuthConfig::Basic {
            user_env,
            password_env,
        } => {
            let user = std::env::var(user_env).map_err(|_| RunnerError::MissingEnvVar {
                var: user_env.clone(),
            })?;
            let pass = std::env::var(password_env).map_err(|_| RunnerError::MissingEnvVar {
                var: password_env.clone(),
            })?;
            builder = builder.basic_auth(user, Some(pass));
        }
        AuthConfig::CustomHeader {
            header_name,
            value_env,
        } => {
            let value = std::env::var(value_env).map_err(|_| RunnerError::MissingEnvVar {
                var: value_env.clone(),
            })?;
            builder = builder.header(header_name.as_str(), value);
        }
        AuthConfig::None => {}
    }
    Ok(builder)
}

/// Extract the LLM's answer from a response body.
fn extract(body: &Bytes, config: &ExtractConfig) -> Result<Option<String>, RunnerError> {
    match config {
        ExtractConfig::Raw => Ok(Some(String::from_utf8_lossy(body).into_owned())),

        ExtractConfig::Jsonpath { path } => {
            let json: serde_json::Value =
                serde_json::from_slice(body).map_err(|e| RunnerError::Extraction {
                    reason: format!("body is not JSON: {e}"),
                })?;
            let jp = JsonPath::parse(path).map_err(|e| RunnerError::Extraction {
                reason: format!("invalid JSONPath '{path}': {e}"),
            })?;
            let hit = jp.query(&json).first().cloned();
            Ok(hit.map(|v| match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            }))
        }

        ExtractConfig::Regex { pattern } => {
            let re = regex::Regex::new(pattern).map_err(|e| RunnerError::Extraction {
                reason: format!("invalid regex '{pattern}': {e}"),
            })?;
            let text = String::from_utf8_lossy(body);
            Ok(re
                .captures(&text)
                .and_then(|c| c.get(1).or_else(|| c.get(0)))
                .map(|m| m.as_str().to_owned()))
        }
    }
}

/// Send one HTTP request for the given `request` template and `payload`,
/// using the adapter type specified in `request.adapter`.
pub async fn execute(
    http: &reqwest::Client,
    request: &Request,
    payload: &str,
) -> Result<AdapterResponse, RunnerError> {
    execute_with_session(http, request, payload, &SessionStrategy::None, "default").await
}

/// Send one HTTP request with optional per-session injection strategy.
pub async fn execute_with_session(
    http: &reqwest::Client,
    request: &Request,
    payload: &str,
    session_strategy: &SessionStrategy,
    session_value: &str,
) -> Result<AdapterResponse, RunnerError> {
    match request.adapter {
        AdapterType::CustomRest => {
            execute_custom_rest(http, request, payload, session_strategy, session_value).await
        }
        AdapterType::OpenAiCompat => {
            execute_openai_compat(http, request, payload, session_strategy, session_value).await
        }
        AdapterType::RawHttp => {
            execute_raw_http(http, request, payload, session_strategy, session_value).await
        }
    }
}

// ── CustomREST ────────────────────────────────────────────────────────────────

async fn execute_custom_rest(
    http: &reqwest::Client,
    request: &Request,
    payload: &str,
    session_strategy: &SessionStrategy,
    session_value: &str,
) -> Result<AdapterResponse, RunnerError> {
    let mut rendered_headers = template::render_headers(&request.headers, payload)?;
    apply_session_header(&mut rendered_headers, session_strategy, session_value);

    let mut builder = http
        .request(
            request
                .method
                .parse()
                .map_err(|_| RunnerError::Extraction {
                    reason: format!("invalid HTTP method: {}", request.method),
                })?,
            &request.url,
        )
        .headers(to_header_map(&rendered_headers)?);

    builder = apply_auth(builder, &request.auth)?;

    builder = match request.body.format {
        BodyFormat::Json => {
            let body = inject_session_body_field(
                request.body.content.clone(),
                session_strategy,
                session_value,
            );

            let body_str = serde_json::to_string(&body).map_err(|e| RunnerError::Extraction {
                reason: e.to_string(),
            })?;
            let rendered = template::render(&body_str, payload)?;
            let json: serde_json::Value =
                serde_json::from_str(&rendered).map_err(|e| RunnerError::Extraction {
                    reason: e.to_string(),
                })?;
            builder.json(&json)
        }
        BodyFormat::Form => {
            let body_str = serde_json::to_string(&request.body.content).map_err(|e| {
                RunnerError::Extraction {
                    reason: e.to_string(),
                }
            })?;
            let rendered = template::render(&body_str, payload)?;
            builder.body(rendered)
        }
        BodyFormat::Text => {
            let text = request.body.content.as_str().unwrap_or_default().to_owned();
            let rendered = template::render(&text, payload)?;
            builder.body(rendered)
        }
    };

    send_and_extract(builder, &request.response.extract).await
}

// ── OpenAI-compatible ─────────────────────────────────────────────────────────

async fn execute_openai_compat(
    http: &reqwest::Client,
    request: &Request,
    payload: &str,
    session_strategy: &SessionStrategy,
    session_value: &str,
) -> Result<AdapterResponse, RunnerError> {
    // Merge the request body content with the payload substituted into the
    // messages array. If content already has `messages`, honour that structure;
    // otherwise wrap the payload directly.
    let mut body = request.body.content.clone();
    if let Some(messages) = body.get_mut("messages") {
        // Substitute {{ prompt }} inside the messages array.
        let msgs_str = serde_json::to_string(messages).map_err(|e| RunnerError::Extraction {
            reason: e.to_string(),
        })?;
        let rendered = template::render(&msgs_str, payload)?;
        *messages = serde_json::from_str(&rendered).map_err(|e| RunnerError::Extraction {
            reason: e.to_string(),
        })?;
    } else {
        body["messages"] = serde_json::json!([{"role": "user", "content": payload}]);
    }

    body = inject_session_body_field(body, session_strategy, session_value);

    let mut rendered_headers = template::render_headers(&request.headers, payload)?;
    apply_session_header(&mut rendered_headers, session_strategy, session_value);
    let mut builder = http
        .request(
            request
                .method
                .parse()
                .map_err(|_| RunnerError::Extraction {
                    reason: format!("invalid HTTP method: {}", request.method),
                })?,
            &request.url,
        )
        .headers(to_header_map(&rendered_headers)?)
        .json(&body);

    builder = apply_auth(builder, &request.auth)?;

    send_and_extract(builder, &request.response.extract).await
}

// ── RawHTTP ───────────────────────────────────────────────────────────────────

async fn execute_raw_http(
    http: &reqwest::Client,
    request: &Request,
    payload: &str,
    session_strategy: &SessionStrategy,
    session_value: &str,
) -> Result<AdapterResponse, RunnerError> {
    let body_str = match &request.body.content {
        serde_json::Value::String(s) => template::render(s, payload)?,
        other => template::render(&other.to_string(), payload)?,
    };

    let mut rendered_headers = template::render_headers(&request.headers, payload)?;
    apply_session_header(&mut rendered_headers, session_strategy, session_value);
    let mut builder = http
        .request(
            request
                .method
                .parse()
                .map_err(|_| RunnerError::Extraction {
                    reason: format!("invalid HTTP method: {}", request.method),
                })?,
            &request.url,
        )
        .headers(to_header_map(&rendered_headers)?)
        .body(body_str);

    builder = apply_auth(builder, &request.auth)?;

    send_and_extract(builder, &ExtractConfig::Raw).await
}

// ── Shared helpers ────────────────────────────────────────────────────────────

async fn send_and_extract(
    builder: reqwest::RequestBuilder,
    extract_cfg: &ExtractConfig,
) -> Result<AdapterResponse, RunnerError> {
    let start = Instant::now();
    let resp = builder.send().await?;
    let first_byte_ms = Some(start.elapsed().as_millis() as u64);

    let status = resp.status().as_u16();
    let response_headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
        .collect();

    let body_bytes = resp.bytes().await?;
    let duration_ms = start.elapsed().as_millis() as u64;

    let extracted = extract(&body_bytes, extract_cfg)?;

    Ok(AdapterResponse {
        status,
        response_headers,
        body_bytes,
        extracted,
        duration_ms,
        first_byte_ms,
    })
}

fn to_header_map(
    headers: &HashMap<String, String>,
) -> Result<reqwest::header::HeaderMap, RunnerError> {
    let mut map = reqwest::header::HeaderMap::new();
    for (k, v) in headers {
        let name = reqwest::header::HeaderName::from_bytes(k.as_bytes()).map_err(|e| {
            RunnerError::Extraction {
                reason: e.to_string(),
            }
        })?;
        let value =
            reqwest::header::HeaderValue::from_str(v).map_err(|e| RunnerError::Extraction {
                reason: e.to_string(),
            })?;
        map.insert(name, value);
    }
    Ok(map)
}

fn apply_session_header(
    rendered_headers: &mut HashMap<String, String>,
    session_strategy: &SessionStrategy,
    session_value: &str,
) {
    if let SessionStrategy::Header { header_name } = session_strategy {
        rendered_headers.insert(header_name.clone(), session_value.to_owned());
    }
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
