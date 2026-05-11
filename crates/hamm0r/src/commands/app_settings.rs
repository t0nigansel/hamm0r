use serde::{Deserialize, Serialize};
use tauri::State;

use super::{AppPaths, LoggerState};
use crate::error::CommandError;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoggingSettingsDto {
    pub enabled: bool,
    pub level: String,
    pub body_logging_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzerSettingsDto {
    #[serde(default = "default_judge_mode")]
    pub judge_mode: String,
    #[serde(default)]
    pub judge_prompt_template: String,
    #[serde(default)]
    pub default_judge_prompt_template: String,
    #[serde(default = "default_uses_default_judge_prompt")]
    pub uses_default_judge_prompt: bool,
    #[serde(default)]
    pub hosted_judge: HostedJudgeSettingsDto,
}

impl Default for AnalyzerSettingsDto {
    fn default() -> Self {
        Self {
            judge_mode: "local".to_owned(),
            judge_prompt_template: String::new(),
            default_judge_prompt_template: String::new(),
            uses_default_judge_prompt: true,
            hosted_judge: HostedJudgeSettingsDto::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostedJudgeSettingsDto {
    #[serde(default = "default_hosted_provider")]
    pub provider: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub deployment: String,
    #[serde(default = "default_hosted_api_style")]
    pub api_style: String,
    #[serde(default)]
    pub api_version: String,
    #[serde(default = "default_hosted_secret_ref")]
    pub secret_ref: String,
    #[serde(default)]
    pub secret_stored: bool,
    #[serde(default = "default_keychain_available")]
    pub keychain_available: bool,
    #[serde(default = "default_hosted_max_input_chars")]
    pub max_input_chars: u32,
    #[serde(default = "default_hosted_max_output_tokens")]
    pub max_output_tokens: u32,
    #[serde(default = "default_hosted_request_timeout_seconds")]
    pub request_timeout_seconds: u32,
    #[serde(default = "default_hosted_max_retries")]
    pub max_retries: u32,
}

impl Default for HostedJudgeSettingsDto {
    fn default() -> Self {
        let defaults = storage::types::HostedJudgeConfig::default();
        Self {
            provider: hosted_provider_label(&defaults.provider).to_owned(),
            endpoint: String::new(),
            deployment: String::new(),
            api_style: hosted_api_style_label(&defaults.api_style).to_owned(),
            api_version: String::new(),
            secret_ref: defaults.secret_ref,
            secret_stored: false,
            keychain_available: true,
            max_input_chars: defaults.max_input_chars,
            max_output_tokens: defaults.max_output_tokens,
            request_timeout_seconds: defaults.request_timeout_seconds,
            max_retries: defaults.max_retries,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettingsDto {
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub analyzer: AnalyzerSettingsDto,
    #[serde(default)]
    pub logging: LoggingSettingsDto,
}

fn default_judge_mode() -> String {
    "local".to_owned()
}

fn default_uses_default_judge_prompt() -> bool {
    true
}

fn default_hosted_provider() -> String {
    "azure_openai".to_owned()
}

fn default_hosted_api_style() -> String {
    "auto".to_owned()
}

fn default_hosted_secret_ref() -> String {
    storage::types::HostedJudgeConfig::default().secret_ref
}

fn default_keychain_available() -> bool {
    true
}

fn default_hosted_max_input_chars() -> u32 {
    storage::types::HostedJudgeConfig::default().max_input_chars
}

fn default_hosted_max_output_tokens() -> u32 {
    storage::types::HostedJudgeConfig::default().max_output_tokens
}

fn default_hosted_request_timeout_seconds() -> u32 {
    storage::types::HostedJudgeConfig::default().request_timeout_seconds
}

fn default_hosted_max_retries() -> u32 {
    storage::types::HostedJudgeConfig::default().max_retries
}

#[tauri::command]
pub fn get_app_settings(
    logger: State<'_, LoggerState>,
    paths: State<'_, AppPaths>,
) -> Result<AppSettingsDto, CommandError> {
    logger.0.info("settings", None, "get_app_settings invoked");
    let root = paths.0.root().to_string_lossy().into_owned();
    let config = storage::settings::load_or_default(&paths.0.config_path(), root)?;
    let default_prompt = analyzer::default_judge_prompt_template().to_owned();
    let configured_prompt = config
        .analyzer
        .judge_prompt_template
        .clone()
        .filter(|s| !s.trim().is_empty());
    let hosted_secret_status =
        storage::secrets::token_status(&config.analyzer.hosted_judge.secret_ref).unwrap_or(
            storage::secrets::TokenStatus {
                stored_in_keychain: false,
                env_var_set: false,
                keychain_available: false,
            },
        );
    let dto = AppSettingsDto {
        app_version: "0.4".to_owned(),
        analyzer: AnalyzerSettingsDto {
            judge_mode: judge_mode_label(&config.analyzer.judge_mode).to_owned(),
            judge_prompt_template: configured_prompt
                .clone()
                .unwrap_or_else(|| default_prompt.clone()),
            default_judge_prompt_template: default_prompt,
            uses_default_judge_prompt: configured_prompt.is_none(),
            hosted_judge: HostedJudgeSettingsDto {
                provider: hosted_provider_label(&config.analyzer.hosted_judge.provider).to_owned(),
                endpoint: config.analyzer.hosted_judge.endpoint.clone(),
                deployment: config.analyzer.hosted_judge.deployment.clone(),
                api_style: hosted_api_style_label(&config.analyzer.hosted_judge.api_style)
                    .to_owned(),
                api_version: config
                    .analyzer
                    .hosted_judge
                    .api_version
                    .clone()
                    .unwrap_or_default(),
                secret_ref: config.analyzer.hosted_judge.secret_ref.clone(),
                secret_stored: hosted_secret_status.stored_in_keychain,
                keychain_available: hosted_secret_status.keychain_available,
                max_input_chars: config.analyzer.hosted_judge.max_input_chars,
                max_output_tokens: config.analyzer.hosted_judge.max_output_tokens,
                request_timeout_seconds: config.analyzer.hosted_judge.request_timeout_seconds,
                max_retries: config.analyzer.hosted_judge.max_retries,
            },
        },
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

    let requested_prompt = settings.analyzer.judge_prompt_template.trim();
    let default_prompt = analyzer::default_judge_prompt_template();
    config.analyzer.judge_mode = parse_judge_mode(&settings.analyzer.judge_mode)?;
    config.analyzer.judge_prompt_template =
        if requested_prompt.is_empty() || requested_prompt == default_prompt.trim() {
            None
        } else {
            Some(requested_prompt.to_owned())
        };
    config.analyzer.hosted_judge.provider =
        parse_hosted_provider(&settings.analyzer.hosted_judge.provider)?;
    config.analyzer.hosted_judge.endpoint = settings.analyzer.hosted_judge.endpoint.trim().to_owned();
    config.analyzer.hosted_judge.deployment =
        settings.analyzer.hosted_judge.deployment.trim().to_owned();
    config.analyzer.hosted_judge.api_style =
        parse_hosted_api_style(&settings.analyzer.hosted_judge.api_style)?;
    config.analyzer.hosted_judge.api_version =
        normalize_optional_text(&settings.analyzer.hosted_judge.api_version);
    config.analyzer.hosted_judge.secret_ref =
        normalize_secret_ref(&settings.analyzer.hosted_judge.secret_ref);
    config.analyzer.hosted_judge.max_input_chars =
        settings.analyzer.hosted_judge.max_input_chars.max(1_000);
    config.analyzer.hosted_judge.max_output_tokens =
        settings.analyzer.hosted_judge.max_output_tokens.max(1);
    config.analyzer.hosted_judge.request_timeout_seconds =
        settings.analyzer.hosted_judge.request_timeout_seconds.max(1);
    config.analyzer.hosted_judge.max_retries = settings.analyzer.hosted_judge.max_retries;
    if matches!(
        config.analyzer.judge_mode,
        storage::types::AnalyzerJudgeMode::Hosted
    ) && config.analyzer.hosted_judge.secret_ref.trim().is_empty()
    {
        return Err(anyhow::anyhow!(
            "Hosted Judge requires a non-empty API key reference."
        )
        .into());
    }
    config.logging.enabled = settings.logging.enabled;
    config.logging.level = parse_log_level(&settings.logging.level)?;
    config.logging.body_logging_enabled = settings.logging.body_logging_enabled;

    storage::settings::save(&paths.0.config_path(), &config)?;

    let effective_prompt = config
        .analyzer
        .judge_prompt_template
        .clone()
        .unwrap_or_else(|| default_prompt.to_owned());
    let hosted_secret_status =
        storage::secrets::token_status(&config.analyzer.hosted_judge.secret_ref).unwrap_or(
            storage::secrets::TokenStatus {
                stored_in_keychain: false,
                env_var_set: false,
                keychain_available: false,
            },
        );
    let dto = AppSettingsDto {
        app_version: "0.4".to_owned(),
        analyzer: AnalyzerSettingsDto {
            judge_mode: judge_mode_label(&config.analyzer.judge_mode).to_owned(),
            judge_prompt_template: effective_prompt,
            default_judge_prompt_template: default_prompt.to_owned(),
            uses_default_judge_prompt: config.analyzer.judge_prompt_template.is_none(),
            hosted_judge: HostedJudgeSettingsDto {
                provider: hosted_provider_label(&config.analyzer.hosted_judge.provider).to_owned(),
                endpoint: config.analyzer.hosted_judge.endpoint.clone(),
                deployment: config.analyzer.hosted_judge.deployment.clone(),
                api_style: hosted_api_style_label(&config.analyzer.hosted_judge.api_style)
                    .to_owned(),
                api_version: config
                    .analyzer
                    .hosted_judge
                    .api_version
                    .clone()
                    .unwrap_or_default(),
                secret_ref: config.analyzer.hosted_judge.secret_ref.clone(),
                secret_stored: hosted_secret_status.stored_in_keychain,
                keychain_available: hosted_secret_status.keychain_available,
                max_input_chars: config.analyzer.hosted_judge.max_input_chars,
                max_output_tokens: config.analyzer.hosted_judge.max_output_tokens,
                request_timeout_seconds: config.analyzer.hosted_judge.request_timeout_seconds,
                max_retries: config.analyzer.hosted_judge.max_retries,
            },
        },
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

fn judge_mode_label(mode: &storage::types::AnalyzerJudgeMode) -> &'static str {
    match mode {
        storage::types::AnalyzerJudgeMode::Local => "local",
        storage::types::AnalyzerJudgeMode::Hosted => "hosted",
    }
}

fn parse_judge_mode(mode: &str) -> Result<storage::types::AnalyzerJudgeMode, CommandError> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "local" => Ok(storage::types::AnalyzerJudgeMode::Local),
        "hosted" => Ok(storage::types::AnalyzerJudgeMode::Hosted),
        other => Err(anyhow::anyhow!("unsupported judge mode '{}'", other).into()),
    }
}

fn hosted_provider_label(provider: &storage::types::HostedJudgeProvider) -> &'static str {
    match provider {
        storage::types::HostedJudgeProvider::AzureOpenai => "azure_openai",
    }
}

fn parse_hosted_provider(
    provider: &str,
) -> Result<storage::types::HostedJudgeProvider, CommandError> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "azure_openai" => Ok(storage::types::HostedJudgeProvider::AzureOpenai),
        other => Err(anyhow::anyhow!("unsupported hosted judge provider '{}'", other).into()),
    }
}

fn hosted_api_style_label(style: &storage::types::HostedJudgeApiStyle) -> &'static str {
    match style {
        storage::types::HostedJudgeApiStyle::Auto => "auto",
        storage::types::HostedJudgeApiStyle::ChatCompletions => "chat_completions",
        storage::types::HostedJudgeApiStyle::Responses => "responses",
    }
}

fn parse_hosted_api_style(
    style: &str,
) -> Result<storage::types::HostedJudgeApiStyle, CommandError> {
    match style.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(storage::types::HostedJudgeApiStyle::Auto),
        "chat_completions" => Ok(storage::types::HostedJudgeApiStyle::ChatCompletions),
        "responses" => Ok(storage::types::HostedJudgeApiStyle::Responses),
        other => Err(anyhow::anyhow!("unsupported hosted judge api style '{}'", other).into()),
    }
}

fn normalize_optional_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn normalize_secret_ref(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        storage::types::HostedJudgeConfig::default().secret_ref
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::AppSettingsDto;

    #[test]
    fn save_payload_deserializes_without_read_only_settings_fields() {
        let payload = serde_json::json!({
            "logging": {
                "enabled": true,
                "level": "info",
                "body_logging_enabled": false
            },
            "analyzer": {
                "judge_mode": "hosted",
                "judge_prompt_template": "custom",
                "hosted_judge": {
                    "provider": "azure_openai",
                    "endpoint": "https://example.openai.azure.com",
                    "deployment": "gpt-5.2-chat",
                    "api_style": "auto",
                    "api_version": "2024-10-21",
                    "secret_ref": "HOSTED_JUDGE_API_KEY",
                    "max_input_chars": 24000,
                    "max_output_tokens": 1200,
                    "request_timeout_seconds": 60,
                    "max_retries": 1
                }
            }
        });

        let dto: AppSettingsDto =
            serde_json::from_value(payload).expect("frontend save payload should deserialize");
        assert_eq!(dto.analyzer.judge_mode, "hosted");
        assert_eq!(dto.analyzer.default_judge_prompt_template, "");
        assert!(dto.analyzer.uses_default_judge_prompt);
        assert!(!dto.analyzer.hosted_judge.secret_stored);
        assert!(dto.analyzer.hosted_judge.keychain_available);
    }
}
