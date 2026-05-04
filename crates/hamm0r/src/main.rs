#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod error;
mod logger;

use commands::{ActiveRunsState, AnalyzerLoggerState, AppConfigState, AppPaths, LoggerState};
use logger::{new_app_session_id, AppLogger};
use storage::types::AppConfig;
use storage::HammorPaths;
use tauri::{Manager as _, RunEvent};

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let (paths, config) = first_launch_hook()?;
            let session_id = new_app_session_id();
            let logger =
                AppLogger::new_core(paths.hamm0r_logs_dir(), config.clone(), session_id.clone())?;
            let analyzer_logger =
                AppLogger::new_analyz0r(paths.analyz0r_logs_dir(), config.clone(), session_id)?;

            logger.info("app", None, "Application startup");
            logger.info("tauri", None, "Tauri setup completed");
            logger.debug(
                "settings",
                None,
                &format!("Loaded config from {}", paths.config_path().display()),
            );
            analyzer_logger.info("analyz0r", None, "Analyz0r logger initialized");

            app.manage(AppPaths(paths));
            app.manage(AppConfigState(config));
            app.manage(LoggerState(logger.clone()));
            app.manage(AnalyzerLoggerState(analyzer_logger));
            app.manage(ActiveRunsState(std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            ))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_settings::get_app_settings,
            commands::app_settings::save_app_settings,
            commands::log_ui_debug,
            commands::list_prompts,
            commands::list_requests,
            commands::requests::get_request,
            commands::requests::list_target_requests,
            commands::requests::save_request,
            commands::requests::delete_request,
            commands::requests::test_request,
            commands::library::seed_library,
            commands::targets::list_targets,
            commands::targets::get_target_meta,
            commands::targets::save_target_meta,
            commands::targets::save_target,
            commands::targets::test_target_connection,
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
            commands::runs::stop_run,
            commands::runs::read_run_attempts,
            commands::runs::read_response_body,
            commands::runs::get_run_diagnostics,
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
            commands::secrets::set_bearer_token,
            commands::secrets::forget_bearer_token,
            commands::secrets::bearer_token_status,
        ])
        .build(tauri::generate_context!())
        .expect("error building hamm0r");

    app.run(|app_handle, event| match event {
        RunEvent::Ready => {
            let logger = app_handle.state::<LoggerState>();
            logger.0.info("app", None, "UI ready");
        }
        RunEvent::Exit => {
            let logger = app_handle.state::<LoggerState>();
            logger.0.info("app", None, "Application shutdown");
        }
        _ => {}
    });
}

/// Ensure the hamm0r data directory tree exists and return the resolved paths.
///
/// Called once at startup. Creates top-level directories so the runner and
/// commands never have to create them. The starter library copy from bundled
/// resources is wired up in Milestone 2 once the AppHandle is available.
fn first_launch_hook() -> anyhow::Result<(HammorPaths, AppConfig)> {
    let paths = HammorPaths::new()?;

    for dir in [
        paths.prompts_dir(),
        paths.requests_dir(),
        paths.targets_dir(),
        paths.scenarios_dir(),
        paths.engagements_dir(),
        paths.analyzer_models_dir(),
        paths.hamm0r_logs_dir(),
        paths.analyz0r_logs_dir(),
    ] {
        std::fs::create_dir_all(&dir)?;
    }

    let config = storage::settings::load_or_default(
        &paths.config_path(),
        paths.root().to_string_lossy().into_owned(),
    )?;

    // Seed bundled prompt library into ~/hamm0r/prompts/ on first run.
    // update=false: skips files that already exist, so user edits are preserved.
    commands::library::seed_on_first_launch(&paths.prompts_dir())?;

    Ok((paths, config))
}
