//! Install-state detection and host-hardware detection.
//!
//! Reads `install.json`, cross-checks the on-disk layout, and produces
//! the five-state machine the UI renders.

use std::path::{Path, PathBuf};

use storage::analyzer_install::{self, AnalyzerInstall, CURRENT_VERSION};
use storage::HammorPaths;

use super::manifest::HardwareClass;

/// State of the analyzer install. Stable string values are emitted in the
/// Tauri DTO so the UI can switch on them without depending on Rust enum
/// reordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InstallState {
    NotInstalled,
    Downloading,
    Installed,
    BrokenInstall,
    IncompatibleVersion,
}

impl InstallState {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            InstallState::NotInstalled => "not_installed",
            InstallState::Downloading => "downloading",
            InstallState::Installed => "installed",
            InstallState::BrokenInstall => "broken_install",
            InstallState::IncompatibleVersion => "incompatible_version",
        }
    }
}

/// Resolve the on-disk install state by reading `install.json` and
/// cross-checking the layout. Distinguishes broken-install from
/// incompatible-version by parsing the JSON in two passes (raw `Value`
/// for the schema-version probe, then typed deserialization).
pub(super) fn install_state_on_disk(
    paths: &HammorPaths,
) -> (InstallState, Option<AnalyzerInstall>) {
    let path = analyzer_install::install_path(paths);
    if !path.exists() {
        return (InstallState::NotInstalled, None);
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return (InstallState::BrokenInstall, None),
    };
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return (InstallState::BrokenInstall, None),
    };
    let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if version != CURRENT_VERSION {
        return (InstallState::IncompatibleVersion, None);
    }
    let install: AnalyzerInstall = match serde_json::from_value(value) {
        Ok(v) => v,
        Err(_) => return (InstallState::BrokenInstall, None),
    };

    // Layout sanity check: the entrypoint binary and the recorded model
    // file must actually exist. We prefer the exact path from
    // install.json — falling back to "any .gguf" only when the file was
    // written before model_file was recorded (legacy v1 installs).
    let entrypoint = paths.analyzer_dir().join(&install.entrypoint);
    let model_present = match install.model_file.as_deref() {
        Some(rel) => paths.analyzer_dir().join(rel).exists(),
        None => first_gguf(&paths.analyzer_models_dir()).is_some(),
    };
    if !entrypoint.exists() || !model_present {
        return (InstallState::BrokenInstall, Some(install));
    }
    (InstallState::Installed, Some(install))
}

pub(super) fn detect_hardware() -> HardwareClass {
    #[cfg(target_arch = "aarch64")]
    return HardwareClass::AppleSilicon;

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            return HardwareClass::X86_64Avx2;
        }
        HardwareClass::Generic
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    HardwareClass::Generic
}

pub(super) fn hardware_id(hw: HardwareClass) -> String {
    match hw {
        HardwareClass::AppleSilicon => "apple_silicon",
        HardwareClass::X86_64Avx2 => "x86_64_avx2",
        HardwareClass::Generic => "generic",
    }
    .to_owned()
}

/// Locate the first `.gguf` in a directory. Used both for status display
/// (the user sees the actual file present) and as a v1-install fallback
/// when `install.json` doesn't record the model path.
pub(super) fn first_gguf(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        (p.extension().and_then(|x| x.to_str()) == Some("gguf")).then_some(p)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_install() -> AnalyzerInstall {
        AnalyzerInstall {
            version: CURRENT_VERSION,
            bundle_version: "0.1.0".into(),
            installed_at: "2026-05-04T00:00:00Z".into(),
            variant_id: "qwen2.5-3b-q4-windows".into(),
            model_id: "qwen2.5-3b-q4".into(),
            platform: "windows-x86_64".into(),
            entrypoint: if cfg!(windows) {
                "bin/analyz0r.exe".into()
            } else {
                "bin/analyz0r".into()
            },
            model_file: Some("models/test.gguf".into()),
        }
    }

    fn lay_down_intact_install(paths: &HammorPaths) {
        let analyzer = paths.analyzer_dir();
        let bin_dir = analyzer.join("bin");
        let models_dir = analyzer.join("models");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&models_dir).unwrap();
        let bin_name = if cfg!(windows) { "analyz0r.exe" } else { "analyz0r" };
        std::fs::write(bin_dir.join(bin_name), b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::write(models_dir.join("test.gguf"), b"FAKE GGUF").unwrap();
        analyzer_install::write(paths, &sample_install()).unwrap();
    }

    #[test]
    fn state_is_not_installed_when_install_json_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        let (state, install) = install_state_on_disk(&paths);
        assert_eq!(state, InstallState::NotInstalled);
        assert!(install.is_none());
    }

    #[test]
    fn state_is_installed_when_layout_intact() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        lay_down_intact_install(&paths);
        let (state, install) = install_state_on_disk(&paths);
        assert_eq!(state, InstallState::Installed);
        assert_eq!(install.unwrap().variant_id, "qwen2.5-3b-q4-windows");
    }

    #[test]
    fn state_is_broken_install_when_entrypoint_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        let models_dir = paths.analyzer_dir().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("test.gguf"), b"FAKE").unwrap();
        analyzer_install::write(&paths, &sample_install()).unwrap();

        let (state, install) = install_state_on_disk(&paths);
        assert_eq!(state, InstallState::BrokenInstall);
        assert!(install.is_some());
    }

    #[test]
    fn state_is_broken_install_when_model_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        let bin_dir = paths.analyzer_dir().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let bin_name = if cfg!(windows) { "analyz0r.exe" } else { "analyz0r" };
        std::fs::write(bin_dir.join(bin_name), b"x").unwrap();
        analyzer_install::write(&paths, &sample_install()).unwrap();

        let (state, _) = install_state_on_disk(&paths);
        assert_eq!(state, InstallState::BrokenInstall);
    }

    #[test]
    fn state_is_incompatible_version_for_unknown_schema() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        let install_path = analyzer_install::install_path(&paths);
        std::fs::create_dir_all(install_path.parent().unwrap()).unwrap();
        std::fs::write(
            &install_path,
            br#"{"version":99,"bundle_version":"x","installed_at":"x","variant_id":"x","model_id":"x","platform":"x","entrypoint":"x"}"#,
        )
        .unwrap();

        let (state, install) = install_state_on_disk(&paths);
        assert_eq!(state, InstallState::IncompatibleVersion);
        assert!(install.is_none());
    }

    #[test]
    fn state_is_broken_install_for_malformed_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        let install_path = analyzer_install::install_path(&paths);
        std::fs::create_dir_all(install_path.parent().unwrap()).unwrap();
        std::fs::write(&install_path, b"{not json").unwrap();

        let (state, _) = install_state_on_disk(&paths);
        assert_eq!(state, InstallState::BrokenInstall);
    }

    #[test]
    fn detection_uses_exact_model_path_from_install_json() {
        // A stale .gguf from a prior variant must not mask a missing
        // model when install.json names a specific file.
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        let analyzer = paths.analyzer_dir();
        let bin_dir = analyzer.join("bin");
        let models_dir = analyzer.join("models");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&models_dir).unwrap();
        let bin_name = if cfg!(windows) { "analyz0r.exe" } else { "analyz0r" };
        std::fs::write(bin_dir.join(bin_name), b"x").unwrap();
        std::fs::write(models_dir.join("stale.gguf"), b"x").unwrap();

        let mut install = sample_install();
        install.model_file = Some("models/expected.gguf".into());
        analyzer_install::write(&paths, &install).unwrap();

        let (state, _) = install_state_on_disk(&paths);
        assert_eq!(state, InstallState::BrokenInstall);
    }

    #[test]
    fn install_state_string_values_are_stable() {
        // The UI switches on these strings — pin them so a Rust enum
        // reorder can't silently rename a state.
        assert_eq!(InstallState::NotInstalled.as_str(), "not_installed");
        assert_eq!(InstallState::Downloading.as_str(), "downloading");
        assert_eq!(InstallState::Installed.as_str(), "installed");
        assert_eq!(InstallState::BrokenInstall.as_str(), "broken_install");
        assert_eq!(
            InstallState::IncompatibleVersion.as_str(),
            "incompatible_version"
        );
    }
}
