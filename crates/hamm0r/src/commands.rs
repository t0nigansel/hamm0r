pub mod analysis;
pub mod analyzer_setup;
pub mod engagements;
pub mod library;
pub mod runs;
pub mod scenarios;
pub mod targets;

use std::collections::HashMap;

use storage::types::{PromptEntry, Request};
use storage::{prompts, requests, HammorPaths};
use tauri::State;

use crate::error::CommandError;

pub struct AppPaths(pub HammorPaths);

#[tauri::command]
pub fn list_prompts(
    paths: State<'_, AppPaths>,
) -> Result<HashMap<String, Vec<PromptEntry>>, CommandError> {
    prompts::load_all(&paths.0.prompts_dir()).map_err(Into::into)
}

#[tauri::command]
pub fn list_requests(paths: State<'_, AppPaths>) -> Result<HashMap<String, Request>, CommandError> {
    requests::load_all(&paths.0.requests_dir()).map_err(Into::into)
}
