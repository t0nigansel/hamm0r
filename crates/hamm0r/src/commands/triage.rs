use storage::triage;
use storage::types::{TriageEntry, TriageStatus};
use tauri::State;

use super::{AppPaths, LoggerState};
use crate::error::CommandError;

#[tauri::command]
pub fn get_triage(
    logger: State<'_, LoggerState>,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
) -> Result<Vec<TriageEntry>, CommandError> {
    logger.0.debug(
        "triage",
        Some(&run_id),
        &format!("get_triage engagement={engagement_slug}"),
    );
    let engagement_dir = paths.0.engagement_dir(&engagement_slug);
    triage::list_entries(&engagement_dir, &run_id).map_err(Into::into)
}

#[tauri::command]
pub fn set_triage_status(
    logger: State<'_, LoggerState>,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
    seq: u32,
    status: TriageStatus,
    note: Option<String>,
) -> Result<TriageEntry, CommandError> {
    let entry = TriageEntry {
        seq,
        status,
        note,
        updated_at: runner::run::iso_now(),
    };
    logger.0.debug(
        "triage",
        Some(&run_id),
        &format!("set_triage_status seq={seq} engagement={engagement_slug}"),
    );
    let engagement_dir = paths.0.engagement_dir(&engagement_slug);
    triage::save_entry(&engagement_dir, &run_id, entry.clone())?;
    Ok(entry)
}
