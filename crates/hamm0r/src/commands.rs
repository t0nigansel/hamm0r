pub mod analysis;
pub mod analyzer_setup;
pub mod app_settings;
pub mod engagements;
pub mod library;
pub mod requests;
pub mod runs;
pub mod scenarios;
pub mod secrets;
pub mod starter_requests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use runner::run::RunCancellation;
use serde::{Deserialize, Serialize};
use storage::types::{AppConfig, PromptEntry, Request};
use storage::{prompts, requests as request_store, HammorPaths};
use tauri::{AppHandle, Emitter as _, State};

use crate::error::CommandError;
use crate::logger::AppLogger;

pub struct AppPaths(pub HammorPaths);
pub struct AppConfigState(pub AppConfig);
pub struct LoggerState(pub AppLogger);
pub struct AnalyzerLoggerState(pub AppLogger);
pub struct ActiveRunsState(pub Arc<Mutex<HashMap<String, RunCancellation>>>);
/// Tracks an in-flight analyzer install. `Some(variant_id)` while the
/// download/extract task is running, `None` otherwise. Used by
/// `get_analyzer_status` to surface the `downloading` state.
pub struct AnalyzerInstallTracker(pub Arc<Mutex<Option<String>>>);

/// Tracks the in-flight `analyz0r` subprocess for each running analysis.
/// We don't store the `Child` itself (the orchestrator needs `&mut`
/// access to drive `wait()`) — instead we keep a oneshot sender that the
/// orchestrator races against `wait()` via `tokio::select!`. Cancel pops
/// the sender out and `send(())`s on it; the orchestrator wakes up and
/// calls `start_kill()` on its locally-held child.
pub struct AnalysisCancelTracker(pub Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>>);

#[derive(Debug, Clone, Serialize)]
pub struct UserRelevantErrorEvent {
    pub scope: String,
    pub run_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UiDebugLogRequest {
    pub component: String,
    pub event: String,
    #[serde(default)]
    pub fields: HashMap<String, String>,
}

pub fn emit_user_relevant_error(app: &AppHandle, scope: &str, run_id: Option<&str>, message: &str) {
    let _ = app.emit(
        "user-relevant-error",
        UserRelevantErrorEvent {
            scope: scope.to_owned(),
            run_id: run_id.map(str::to_owned),
            message: message.to_owned(),
        },
    );
}

pub fn report_user_relevant_error(
    app: &AppHandle,
    logger: &AppLogger,
    component: &str,
    scope: &str,
    run_id: Option<&str>,
    message: &str,
) {
    logger.error(component, run_id, message);
    emit_user_relevant_error(app, scope, run_id, message);
}

#[tauri::command]
pub fn log_ui_debug(
    logger: State<'_, LoggerState>,
    payload: UiDebugLogRequest,
) -> Result<(), CommandError> {
    let mut field_pairs: Vec<String> = payload
        .fields
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    field_pairs.sort();

    let suffix = if field_pairs.is_empty() {
        String::new()
    } else {
        format!(" {}", field_pairs.join(" "))
    };

    logger.0.info(
        &payload.component,
        None,
        &format!("ui-event={}{}", payload.event, suffix),
    );

    Ok(())
}

#[tauri::command]
pub fn list_prompts(
    paths: State<'_, AppPaths>,
) -> Result<HashMap<String, Vec<PromptEntry>>, CommandError> {
    prompts::load_all(&paths.0.prompts_dir()).map_err(Into::into)
}

#[tauri::command]
pub fn list_requests(paths: State<'_, AppPaths>) -> Result<HashMap<String, Request>, CommandError> {
    request_store::load_all(&paths.0.requests_dir()).map_err(Into::into)
}
