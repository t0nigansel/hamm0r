//! Read/write the analyzer install metadata file.
//!
//! `~/hamm0r/analyzer/install.json` is the source of truth for whether the
//! analyz0r bundle is installed. Existence + valid JSON + matching schema
//! version mean "installed"; anything else means "absent" or "broken".
//!
//! Schema is defined in `docs/analyzorPlan.md` ("Data Model Changes →
//! `~/hamm0r/analyzer/install.json`"). Field changes require a `version`
//! bump and a migration plan.

use std::path::PathBuf;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::paths::HammorPaths;
use crate::write::atomic_write;

/// Current install-metadata schema version. Bump when the on-disk shape
/// changes; readers must reject anything they do not understand.
pub const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzerInstall {
    /// Schema version of this file.
    pub version: u32,
    /// Bundle release that produced the install (e.g. `"0.1.0"`).
    pub bundle_version: String,
    /// ISO-8601 UTC timestamp of when the install completed.
    pub installed_at: String,
    /// Manifest variant id this install was built from.
    pub variant_id: String,
    /// Logical model identifier (separate from the on-disk file name).
    pub model_id: String,
    /// Platform tag (e.g. `"windows-x86_64"`, `"macos-aarch64"`).
    pub platform: String,
    /// Path to the analyzer binary, relative to the install root.
    pub entrypoint: String,
    /// Path to the model file the installer placed, relative to the install
    /// root (e.g. `"models/qwen2.5-3b-q4.gguf"`). Optional for forward-
    /// compatibility with v1 install.json files written before this field
    /// was recorded; readers fall back to scanning `models/` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_file: Option<String>,
}

/// `~/hamm0r/analyzer/install.json`.
pub fn install_path(paths: &HammorPaths) -> PathBuf {
    paths.analyzer_dir().join("install.json")
}

/// Read the install metadata. Returns `Ok(None)` when the file is missing —
/// that is the canonical "not installed" signal. JSON / schema-version
/// errors propagate so callers can surface a "broken install" state.
pub fn read(paths: &HammorPaths) -> anyhow::Result<Option<AnalyzerInstall>> {
    let path = install_path(paths);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let install: AnalyzerInstall = serde_json::from_str(&raw)
        .with_context(|| format!("cannot parse {}", path.display()))?;
    if install.version != CURRENT_VERSION {
        anyhow::bail!(
            "install.json schema version {} not supported (expected {})",
            install.version,
            CURRENT_VERSION
        );
    }
    Ok(Some(install))
}

/// Write the install metadata atomically. Caller is responsible for
/// ordering: bundle files must be in place *before* this is written, so
/// readers never see install.json pointing at half-extracted contents.
pub fn write(paths: &HammorPaths, install: &AnalyzerInstall) -> anyhow::Result<()> {
    let path = install_path(paths);
    let json = serde_json::to_vec_pretty(install).context("serialise install metadata")?;
    atomic_write(&path, &json)
}

/// Delete the install metadata file. Missing file is not an error.
pub fn remove(paths: &HammorPaths) -> anyhow::Result<()> {
    let path = install_path(paths);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::anyhow!("cannot remove {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample() -> AnalyzerInstall {
        AnalyzerInstall {
            version: CURRENT_VERSION,
            bundle_version: "0.1.0".to_owned(),
            installed_at: "2026-05-04T12:00:00Z".to_owned(),
            variant_id: "default-x86_64".to_owned(),
            model_id: "qwen-default".to_owned(),
            platform: "windows-x86_64".to_owned(),
            entrypoint: "bin/analyz0r.exe".to_owned(),
            model_file: Some("models/qwen-default.gguf".to_owned()),
        }
    }

    #[test]
    fn legacy_install_without_model_file_still_parses() {
        // v1 install.json files written before model_file existed must
        // still load — that's why the field is optional.
        let tmp = TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        let path = install_path(&paths);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            br#"{"version":1,"bundle_version":"0.1.0","installed_at":"x","variant_id":"x","model_id":"x","platform":"x","entrypoint":"bin/x"}"#,
        )
        .unwrap();
        let install = read(&paths).unwrap().expect("should parse");
        assert!(install.model_file.is_none());
    }

    #[test]
    fn read_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        assert!(read(&paths).unwrap().is_none());
    }

    #[test]
    fn write_then_read_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        let original = sample();
        write(&paths, &original).unwrap();
        let read_back = read(&paths).unwrap().expect("should be present");
        assert_eq!(read_back, original);
    }

    #[test]
    fn read_rejects_unknown_schema_version() {
        let tmp = TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        let path = install_path(&paths);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            br#"{"version":99,"bundle_version":"x","installed_at":"x","variant_id":"x","model_id":"x","platform":"x","entrypoint":"x"}"#,
        )
        .unwrap();
        let err = read(&paths).unwrap_err();
        assert!(err.to_string().contains("schema version 99"));
    }

    #[test]
    fn remove_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        // No file yet — should still succeed.
        remove(&paths).unwrap();
        // Now create one, remove it, and re-remove.
        write(&paths, &sample()).unwrap();
        assert!(install_path(&paths).exists());
        remove(&paths).unwrap();
        assert!(!install_path(&paths).exists());
        remove(&paths).unwrap();
    }
}
