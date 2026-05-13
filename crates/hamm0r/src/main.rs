#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod error;
mod logger;

use commands::{
    ActiveRunsState, AnalysisCancelTracker, AnalyzerInstallTracker, AnalyzerLoggerState,
    AppConfigState, AppPaths, LoggerState,
};
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
            app.manage(AnalyzerInstallTracker(std::sync::Arc::new(
                std::sync::Mutex::new(None),
            )));
            app.manage(AnalysisCancelTracker(std::sync::Arc::new(
                std::sync::Mutex::new(std::collections::HashMap::new()),
            )));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_settings::get_app_settings,
            commands::app_settings::save_app_settings,
            commands::log_ui_debug,
            commands::list_prompts,
            commands::list_requests,
            commands::requests::get_request,
            commands::requests::test_request,
            commands::requests::save_request_global,
            commands::requests::delete_request_global,
            commands::requests::list_request_references,
            commands::library::seed_library,
            commands::library::get_prompt,
            commands::library::create_prompt,
            commands::library::update_prompt,
            commands::library::delete_prompt,
            commands::scenarios::list_scenarios,
            commands::scenarios::create_scenario,
            commands::scenarios::get_scenario,
            commands::scenarios::save_scenario,
            commands::scenarios::delete_scenario,
            commands::engagements::list_engagements,
            commands::engagements::create_engagement,
            commands::engagements::delete_engagement,
            commands::engagements::list_runs,
            commands::engagements::get_run_progress,
            commands::engagements::save_markdown_export,
            commands::engagements::open_export_path,
            commands::runs::start_run,
            commands::runs::start_scenario_run,
            commands::runs::stop_run,
            commands::runs::delete_run,
            commands::runs::read_run_attempts,
            commands::runs::read_response_body,
            commands::runs::get_run_diagnostics,
            commands::analysis::read_run_verdicts,
            commands::analysis::generate_report,
            commands::analysis::read_report_html,
            commands::analysis::judge_result,
            commands::analysis::judge_all,
            commands::analysis::start_analysis,
            commands::analysis::test_hosted_judge,
            commands::analysis::cancel_analysis,
            commands::analyzer_setup::get_analyzer_status,
            commands::analyzer_setup::fetch_analyzer_manifest,
            commands::analyzer_setup::download_and_install_analyzer,
            commands::analyzer_setup::uninstall_analyzer,
            commands::secrets::set_bearer_token,
            commands::secrets::set_secret_ref,
            commands::secrets::forget_bearer_token,
            commands::secrets::forget_secret_ref,
            commands::secrets::bearer_token_status,
            commands::secrets::secret_ref_status,
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

    // Seed bundled prompt library into ~/hamm0r/prompts/ on startup.
    // Missing files are copied in; existing user-edited files are preserved.
    commands::library::seed_on_first_launch(&paths.prompts_dir())?;
    // Seed bundled starter requests into ~/hamm0r/requests/ using the same
    // "missing only" rule so newly introduced templates appear automatically
    // without overwriting user customizations.
    commands::starter_requests::seed_on_startup(&paths.requests_dir())?;

    // Phase 2D of docs/RefactorPlan.md: copy each Target's `name` into the
    // `tag` field of every Request the Target references. Idempotent — runs
    // every startup, only writes Requests whose tag is currently None.
    // Logged but never fatal: a migration failure shouldn't block launch.
    match storage::migrations::v2::tag_requests_from_targets(
        &paths.targets_dir(),
        &paths.requests_dir(),
    ) {
        Ok(report) if report.tagged > 0 => {
            eprintln!(
                "[migrate:v2:tag] tagged {} request(s) from {} target name(s); orphan refs: {}",
                report.tagged,
                report.already_tagged + report.tagged,
                report.orphan_target_refs,
            );
        }
        Ok(_) => {}
        Err(err) => {
            eprintln!("[migrate:v2:tag] migration error (non-fatal): {err:#}");
        }
    }

    // Phase 2D of docs/RefactorPlan.md (Q-H resolved per-run): translate each
    // Target's `auth_acquisition.http_login` into a real Request bound to
    // `bearer_token`, and wire the chat Requests' Authorization headers to
    // reference it. Idempotent. Existing manual Authorization headers are
    // preserved. Non-fatal on error.
    match storage::migrations::v2::synthesize_auth_chain_requests(
        &paths.targets_dir(),
        &paths.requests_dir(),
    ) {
        Ok(report) if report.login_requests_synthesized > 0 || report.chat_requests_wired > 0 => {
            eprintln!(
                "[migrate:v2:auth-chain] synthesized {} login request(s); wired {} chat request(s); kept {} existing auth header(s); skipped {} target(s)",
                report.login_requests_synthesized,
                report.chat_requests_wired,
                report.chat_requests_already_wired,
                report.targets_skipped,
            );
        }
        Ok(_) => {}
        Err(err) => {
            eprintln!("[migrate:v2:auth-chain] migration error (non-fatal): {err:#}");
        }
    }

    Ok((paths, config))
}
