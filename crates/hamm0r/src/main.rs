#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod error;

use commands::AppPaths;
use storage::HammorPaths;
use tauri::Manager as _;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let paths = first_launch_hook()?;
            app.manage(AppPaths(paths));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_prompts,
            commands::list_requests,
            commands::list_targets,
            commands::save_target,
            commands::delete_target,
            commands::list_scenarios,
            commands::list_engagements,
            commands::create_engagement,
            commands::start_run,
            commands::read_run_attempts,
            commands::read_response_body,
        ])
        .run(tauri::generate_context!())
        .expect("error running hamm0r");
}

/// Ensure the hamm0r data directory tree exists and return the resolved paths.
///
/// Called once at startup. Creates top-level directories so the runner and
/// commands never have to create them. The starter library copy from bundled
/// resources is wired up in Milestone 2 once the AppHandle is available.
fn first_launch_hook() -> anyhow::Result<HammorPaths> {
    let paths = HammorPaths::new()?;

    for dir in [
        paths.prompts_dir(),
        paths.requests_dir(),
        paths.targets_dir(),
        paths.scenarios_dir(),
        paths.engagements_dir(),
    ] {
        std::fs::create_dir_all(&dir)?;
    }

    Ok(paths)
}
