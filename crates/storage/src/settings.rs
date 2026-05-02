use std::path::Path;

use anyhow::Context as _;

use crate::types::AppConfig;
use crate::write::atomic_write;

pub fn load_or_default(config_path: &Path, hamm0r_root: String) -> anyhow::Result<AppConfig> {
    if !config_path.exists() {
        let config = AppConfig::defaults(hamm0r_root);
        save(config_path, &config)?;
        return Ok(config);
    }

    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("cannot read config: {}", config_path.display()))?;
    let mut config = serde_yaml::from_str::<AppConfig>(&raw)
        .with_context(|| format!("cannot parse config: {}", config_path.display()))?;
    if config.version == 0 {
        config.version = 1;
    }
    if config.hamm0r_root.trim().is_empty() {
        config.hamm0r_root = hamm0r_root;
    }
    Ok(config)
}

pub fn save(config_path: &Path, config: &AppConfig) -> anyhow::Result<()> {
    let yaml = serde_yaml::to_string(config).context("cannot serialize app config")?;
    atomic_write(config_path, yaml.as_bytes())
        .with_context(|| format!("cannot write config: {}", config_path.display()))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::types::{AppConfig, LogLevel, Theme};

    use super::*;

    #[test]
    fn creates_default_config_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let config = load_or_default(&path, "C:/tmp/hamm0r".to_owned()).unwrap();

        assert!(path.exists());
        assert_eq!(config.version, 1);
        assert_eq!(config.default_parallelism, 4);
        assert!(config.logging.enabled);
        assert_eq!(config.logging.level, LogLevel::Info);
        assert!(!config.logging.body_logging_enabled);
        assert_eq!(config.ui.theme, Theme::System);
    }

    #[test]
    fn roundtrips_existing_config() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let config = AppConfig::defaults("C:/tmp/hamm0r".to_owned());
        save(&path, &config).unwrap();

        let loaded = load_or_default(&path, "ignored".to_owned()).unwrap();
        assert_eq!(loaded, config);
    }
}
