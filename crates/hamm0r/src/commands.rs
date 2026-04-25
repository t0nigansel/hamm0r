use std::collections::HashMap;

use runner::{execute_run, Payload, RunConfig};
use serde::{Deserialize, Serialize};
use storage::runs::{read_all, RunRecord};
use storage::types::{
    AdapterType, AuthConfig, BodyConfig, BodyFormat, EngagementMeta, EngagementScope,
    EngagementTarget, ExtractConfig, PromptEntry, Request, ResponseConfig, Scenario,
    SessionConfig, Target,
};
use storage::{engagements, prompts, requests, scenarios, targets, HammorPaths};
use tauri::{AppHandle, Emitter as _, State};

use crate::error::CommandError;

pub struct AppPaths(pub HammorPaths);

// ── Read commands ─────────────────────────────────────────────────────────────

#[tauri::command]
pub fn list_prompts(
    paths: State<'_, AppPaths>,
) -> Result<HashMap<String, Vec<PromptEntry>>, CommandError> {
    prompts::load_all(&paths.0.prompts_dir()).map_err(Into::into)
}

#[tauri::command]
pub fn list_requests(
    paths: State<'_, AppPaths>,
) -> Result<HashMap<String, Request>, CommandError> {
    requests::load_all(&paths.0.requests_dir()).map_err(Into::into)
}

#[tauri::command]
pub fn list_scenarios(
    paths: State<'_, AppPaths>,
) -> Result<HashMap<String, Scenario>, CommandError> {
    scenarios::load_all(&paths.0.scenarios_dir()).map_err(Into::into)
}

#[tauri::command]
pub fn list_engagements(
    paths: State<'_, AppPaths>,
) -> Result<Vec<EngagementMeta>, CommandError> {
    engagements::list(&paths.0.engagements_dir()).map_err(Into::into)
}

// ── Target DTO (combines Target + Request for the UI) ─────────────────────────

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
            token_env: dto.auth_env.clone().unwrap_or_else(|| "HAMM0R_TOKEN".into()),
        },
        "basic" => AuthConfig::Basic {
            user_env: dto.auth_env.clone().unwrap_or_else(|| "HAMM0R_USER".into()),
            password_env: "HAMM0R_PASS".into(),
        },
        "api_key" => AuthConfig::CustomHeader {
            header_name: dto.auth_header.clone().unwrap_or_else(|| "Authorization".into()),
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
            ExtractConfig::Jsonpath { path: "$.choices[0].message.content".into() },
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
                ExtractConfig::Jsonpath { path: format!("$.{resp_field}") },
            )
        }
    };

    let session_config = match dto.session_strategy.as_str() {
        "cookie" => SessionConfig::Cookie,
        "header" => SessionConfig::Header {
            header_name: dto.session_field.clone().unwrap_or_else(|| "X-Session-Id".into()),
        },
        "body_field" => SessionConfig::BodyField {
            field_name: dto.session_field.clone().unwrap_or_else(|| "session_id".into()),
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
        AuthConfig::Bearer { token_env } => {
            ("bearer".into(), Some(token_env.clone()), None)
        }
        AuthConfig::Basic { user_env, .. } => ("basic".into(), Some(user_env.clone()), None),
        AuthConfig::CustomHeader { header_name, value_env } => {
            ("api_key".into(), Some(value_env.clone()), Some(header_name.clone()))
        }
        AuthConfig::None => ("none".into(), None, None),
    };

    let (session_strategy, session_field) = match &target.session_config {
        SessionConfig::None => ("none".into(), None),
        SessionConfig::Cookie => ("cookie".into(), None),
        SessionConfig::Header { header_name } => ("header".into(), Some(header_name.clone())),
        SessionConfig::BodyField { field_name } => ("body_field".into(), Some(field_name.clone())),
    };

    // Try to extract request/response field names from the body content.
    let request_field = request.body.content.as_object().and_then(|m| {
        m.keys().find(|k| *k != "messages").cloned()
    });
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
            let req = all_requests.get(&t.request_id).or_else(|| all_requests.get(&t.id))?;
            Some(pair_to_dto(t, req))
        })
        .collect();
    Ok(dtos)
}

/// Create or update a target (and its backing request file).
#[tauri::command]
pub fn save_target(
    paths: State<'_, AppPaths>,
    dto: TargetDto,
) -> Result<TargetDto, CommandError> {
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

// ── Engagement commands ───────────────────────────────────────────────────────

/// Create a new engagement directory tree and return its metadata.
/// The slug is generated as `<YYYY-MM-DD>-<slugified-name>`.
#[tauri::command]
pub fn create_engagement(
    paths: State<'_, AppPaths>,
    name: String,
) -> Result<EngagementMeta, CommandError> {
    let slug = make_slug(&name);
    let meta = EngagementMeta {
        version: 1,
        slug: slug.clone(),
        name: name.clone(),
        created_at: runner::run::iso_now(),
        target: EngagementTarget { request_id: String::new(), notes: None },
        scope: EngagementScope { prompt_files: vec![] },
    };
    engagements::create(&paths.0.engagements_dir(), &meta)?;
    Ok(meta)
}

fn make_slug(name: &str) -> String {
    let today = &runner::run::iso_now()[..10]; // "YYYY-MM-DD"
    let slug_part: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("{today}-{slug_part}")
}

// ── Run commands ──────────────────────────────────────────────────────────────

/// Payload descriptor sent from the UI for a single fire.
#[derive(Debug, Deserialize)]
pub struct PayloadSpec {
    pub prompt_id: String,
    pub payload_id: String,
    pub text: String,
}

/// Progress event emitted to the UI after each attempt.
#[derive(Debug, Clone, Serialize)]
pub struct RunProgressEvent {
    pub run_id: String,
    pub seq: u32,
    pub total: u32,
    pub status: u16,
    pub error: Option<String>,
    pub finished: bool,
}

/// Start a run for the given engagement + request + payload list.
///
/// Returns the run_id immediately and fires progress events (`run-progress`)
/// via Tauri as each attempt completes. The JSONL file is written to
/// `<engagements_dir>/<engagement_slug>/runs/<run_id>.jsonl`.
#[tauri::command]
pub async fn start_run(
    app: AppHandle,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    request_id: String,
    payloads: Vec<PayloadSpec>,
    parallelism: Option<usize>,
) -> Result<String, CommandError> {
    let all_requests = requests::load_all(&paths.0.requests_dir())?;
    let request = all_requests
        .get(&request_id)
        .ok_or_else(|| anyhow::anyhow!("request '{}' not found", request_id))?
        .clone();

    let run_id = next_run_id(&paths.0.engagement_dir(&engagement_slug))?;
    let engagement_dir = paths.0.engagement_dir(&engagement_slug);

    let runner_payloads: Vec<Payload> = payloads
        .into_iter()
        .map(|p| Payload {
            prompt_id: p.prompt_id,
            payload_id: p.payload_id,
            text: p.text,
            session: "default".into(),
        })
        .collect();

    let config = RunConfig {
        engagement_dir,
        run_id: run_id.clone(),
        request,
        payloads: runner_payloads,
        parallelism: parallelism.unwrap_or(4),
        runner_version: env!("CARGO_PKG_VERSION").to_owned(),
    };

    let run_id_ret = run_id.clone();

    tokio::spawn(async move {
        let result = execute_run(config, move |progress| {
            let event = RunProgressEvent {
                run_id: progress.run_id,
                seq: progress.seq,
                total: progress.total,
                status: progress.status,
                error: progress.error,
                finished: progress.finished,
            };
            let _ = app.emit("run-progress", event);
        })
        .await;

        if let Err(e) = result {
            eprintln!("run {run_id} failed: {e}");
        }
    });

    Ok(run_id_ret)
}

/// Read attempt records from a run's JSONL file. Returns a JSON array of
/// attempt objects (headers and footers are omitted).
#[tauri::command]
pub fn read_run_attempts(
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
) -> Result<Vec<serde_json::Value>, CommandError> {
    let run_path = paths
        .0
        .engagement_dir(&engagement_slug)
        .join("runs")
        .join(format!("{run_id}.jsonl"));

    if !run_path.exists() {
        return Ok(vec![]);
    }

    let records = read_all(&run_path)?;
    let attempts = records
        .into_iter()
        .filter_map(|r| match r {
            RunRecord::Attempt(a) => serde_json::to_value(*a).ok(),
            _ => None,
        })
        .collect();
    Ok(attempts)
}

/// Read the raw text of one response body file.
#[tauri::command]
pub fn read_response_body(
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
    seq: u32,
) -> Result<Option<String>, CommandError> {
    let body_path = paths
        .0
        .engagement_dir(&engagement_slug)
        .join("responses")
        .join(&run_id)
        .join(format!("{seq:04}.txt"));

    if !body_path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&body_path)
        .map_err(|e| anyhow::anyhow!("cannot read response body: {e}"))?;
    Ok(Some(text))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn next_run_id(engagement_dir: &std::path::Path) -> anyhow::Result<String> {
    let runs_dir = engagement_dir.join("runs");
    if !runs_dir.exists() {
        return Ok("run-001".to_owned());
    }

    let max_seq = std::fs::read_dir(&runs_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            s.strip_prefix("run-")
                .and_then(|r| r.strip_suffix(".jsonl"))
                .and_then(|n| n.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);

    Ok(format!("run-{:03}", max_seq + 1))
}
