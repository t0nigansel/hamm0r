use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use storage::scenarios;
use storage::types::Scenario;
use tauri::State;

use super::AppPaths;
use crate::error::CommandError;

#[tauri::command]
pub fn list_scenarios(
    paths: State<'_, AppPaths>,
) -> Result<std::collections::HashMap<String, Scenario>, CommandError> {
    scenarios::load_all(&paths.0.scenarios_dir()).map_err(Into::into)
}

#[tauri::command]
pub fn create_scenario(paths: State<'_, AppPaths>, name: String) -> Result<Scenario, CommandError> {
    let scenario = Scenario {
        version: 1,
        id: make_entity_id("scenario"),
        name: if name.trim().is_empty() {
            "New Scenario".to_owned()
        } else {
            name
        },
        repeat: 1,
        description: None,
        request_ids: Vec::new(),
        library: None,
        shared_session: false,
        mutations: None,
        session_count: None,
        session_identity: None,
    };
    scenarios::save(&paths.0.scenarios_dir(), &scenario)?;
    Ok(scenario)
}

#[tauri::command]
pub fn get_scenario(
    paths: State<'_, AppPaths>,
    id: String,
) -> Result<Option<Scenario>, CommandError> {
    scenarios::load(&paths.0.scenarios_dir(), &id).map_err(Into::into)
}

#[tauri::command]
pub fn save_scenario(
    paths: State<'_, AppPaths>,
    mut scenario: Scenario,
) -> Result<Scenario, CommandError> {
    if scenario.version == 0 {
        scenario.version = 1;
    }
    if scenario.repeat == 0 {
        scenario.repeat = 1;
    }
    if scenario.id.trim().is_empty() {
        scenario.id = make_entity_id("scenario");
    }
    scenarios::save(&paths.0.scenarios_dir(), &scenario)?;
    Ok(scenario)
}

#[tauri::command]
pub fn delete_scenario(paths: State<'_, AppPaths>, id: String) -> Result<(), CommandError> {
    scenarios::delete(&paths.0.scenarios_dir(), &id)?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct MutatorInfoDto {
    pub id: String,
    pub family: String,
}

/// Return every shipped mutator's id and family, in registry order.
/// Used by the Scenario editor to render the mutation panel (Section
/// 2.10 of docs/ToDo.md).
#[tauri::command]
pub fn list_mutators() -> Vec<MutatorInfoDto> {
    runner::mutation::registry()
        .iter()
        .map(|m| MutatorInfoDto {
            id: m.id().to_owned(),
            family: m.family().as_str().to_owned(),
        })
        .collect()
}

fn make_entity_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{nanos:x}")
}
