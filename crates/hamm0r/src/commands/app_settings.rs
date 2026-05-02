use serde::{Deserialize, Serialize};
use tauri::State;

use super::{AppPaths, LoggerState};
use crate::error::CommandError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingSettingsDto {
    pub enabled: bool,
    pub level: String,
    pub body_logging_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettingsDto {
    pub logging: LoggingSettingsDto,
}

#[tauri::command]
pub fn get_app_settings(
    logger: State<'_, LoggerState>,
    paths: State<'_, AppPaths>,
) -> Result<AppSettingsDto, CommandError> {
    logger.0.info("settings", None, "get_app_settings invoked");
    let root = paths.0.root().to_string_lossy().into_owned();
    let config = storage::settings::load_or_default(&paths.0.config_path(), root)?;
    let dto = AppSettingsDto {
        logging: LoggingSettingsDto {
            enabled: config.logging.enabled,
            level: log_level_label(&config.logging.level).to_owned(),
            body_logging_enabled: config.logging.body_logging_enabled,
        },
    };
    logger
        .0
        .info("settings", None, "get_app_settings completed");
    Ok(dto)
}

#[tauri::command]
pub fn save_app_settings(
    logger: State<'_, LoggerState>,
    paths: State<'_, AppPaths>,
    settings: AppSettingsDto,
) -> Result<AppSettingsDto, CommandError> {
    logger.0.info("settings", None, "save_app_settings invoked");
    let root = paths.0.root().to_string_lossy().into_owned();
    let mut config = storage::settings::load_or_default(&paths.0.config_path(), root)?;

    config.logging.enabled = settings.logging.enabled;
    config.logging.level = parse_log_level(&settings.logging.level)?;
    config.logging.body_logging_enabled = settings.logging.body_logging_enabled;

    storage::settings::save(&paths.0.config_path(), &config)?;

    let dto = AppSettingsDto {
        logging: LoggingSettingsDto {
            enabled: config.logging.enabled,
            level: log_level_label(&config.logging.level).to_owned(),
            body_logging_enabled: config.logging.body_logging_enabled,
        },
    };
    logger
        .0
        .info("settings", None, "save_app_settings completed");
    Ok(dto)
}

fn parse_log_level(level: &str) -> Result<storage::types::LogLevel, CommandError> {
    match level.trim().to_ascii_lowercase().as_str() {
        "error" => Ok(storage::types::LogLevel::Error),
        "info" => Ok(storage::types::LogLevel::Info),
        "debug" => Ok(storage::types::LogLevel::Debug),
        other => Err(anyhow::anyhow!("unsupported log level '{}'", other).into()),
    }
}

fn log_level_label(level: &storage::types::LogLevel) -> &'static str {
    match level {
        storage::types::LogLevel::Error => "error",
        storage::types::LogLevel::Info => "info",
        storage::types::LogLevel::Debug => "debug",
    }
}
