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
            commands::library::seed_library,
            commands::targets::list_targets,
            commands::targets::save_target,
            commands::targets::delete_target,
            commands::scenarios::list_scenarios,
            commands::scenarios::create_scenario,
            commands::scenarios::get_scenario,
            commands::scenarios::save_scenario,
            commands::scenarios::delete_scenario,
            commands::engagements::list_engagements,
            commands::engagements::create_engagement,
            commands::engagements::list_runs,
            commands::engagements::get_run_progress,
            commands::runs::start_run,
            commands::runs::start_scenario_run,
            commands::runs::start_transient_scenario_run,
            commands::runs::read_run_attempts,
            commands::runs::read_response_body,
            commands::analysis::read_run_verdicts,
            commands::analysis::generate_report,
            commands::analysis::read_report_html,
            commands::analysis::judge_result,
            commands::analysis::judge_all,
            commands::analysis::start_analysis,
            commands::analyzer_setup::get_analyzer_status,
            commands::analyzer_setup::fetch_analyzer_manifest,
            commands::analyzer_setup::download_and_install_analyzer,
            commands::analyzer_setup::uninstall_analyzer,
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
        paths.analyzer_models_dir(),
    ] {
        std::fs::create_dir_all(&dir)?;
    }

    // Seed bundled prompt library into ~/hamm0r/prompts/ on first run.
    // update=false: skips files that already exist, so user edits are preserved.
    commands::library::seed_on_first_launch(&paths.prompts_dir())?;

    Ok(paths)
}
