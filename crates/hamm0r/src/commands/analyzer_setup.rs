/// Analyzer activation: manifest fetch, hardware detection, model download.
///
/// This module is intentionally NOT behind `#[cfg(feature = "analyzer")]`
/// so that users can download the model file in the default build.
/// Manifest types are defined locally (mirroring `analyzer::manifest`) to
/// avoid a hard dependency on the optional `analyzer` crate.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use storage::analyzer_install::{self, AnalyzerInstall, CURRENT_VERSION};
use storage::HammorPaths;
use tauri::{AppHandle, Emitter as _, State};
use tokio::io::AsyncWriteExt as _;

use super::{
    report_user_relevant_error, AnalyzerInstallTracker, AnalyzerLoggerState, AppPaths,
};
use crate::error::CommandError;

// ── Manifest types (mirror of analyzer::manifest) ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzerManifest {
    pub version: u32,
    pub generated_at: String,
    pub minimum_hamm0r_version: String,
    pub variants: Vec<AnalyzerVariant>,
}

/// One installable analyzer variant. The bundle artifact is a single
/// archive containing the analyz0r binary, any runtime assets, and the
/// model file — see `docs/analyzorPlan.md` (Phase 2 install layout).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzerVariant {
    pub id: String,
    pub label: String,
    /// Operating system tag, e.g. `"windows"`, `"macos"`, `"linux"`.
    pub os: String,
    /// CPU architecture tag, e.g. `"x86_64"`, `"aarch64"`.
    pub arch: String,
    pub hardware: HardwareClass,
    pub recommended: bool,
    /// Logical model identifier (separate from on-disk file name) — the
    /// bundle is responsible for placing the model where the runtime
    /// expects it.
    pub model_id: String,
    /// The bundle archive that gets downloaded, verified, and extracted.
    pub bundle: Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareClass {
    AppleSilicon,
    X86_64Avx2,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
}

const MANIFEST_URL: &str = "https://hamm0r.io/analyzer/manifest.json";

/// Last-resort fallback when the remote manifest is unreachable. The URLs
/// and hashes here are placeholders — they will fail SHA verification by
/// design so a user can't accidentally install an unverified blob from
/// this fallback. Replace with real values once bundles are published in
/// Phase 6.
fn bundled_manifest() -> AnalyzerManifest {
    fn placeholder_bundle(filename: &str) -> Artifact {
        Artifact {
            url: format!("https://hamm0r.io/analyzer/v0/{filename}"),
            sha256: "PLACEHOLDER".to_owned(),
            size_bytes: 0,
        }
    }
    AnalyzerManifest {
        version: 1,
        generated_at: "2026-05-04T00:00:00Z".to_owned(),
        minimum_hamm0r_version: "0.1.0".to_owned(),
        variants: vec![
            AnalyzerVariant {
                id: "qwen2.5-3b-q4-apple".to_owned(),
                label: "Qwen2.5 3B Q4_K_M (Apple Silicon — Metal)".to_owned(),
                os: "macos".to_owned(),
                arch: "aarch64".to_owned(),
                hardware: HardwareClass::AppleSilicon,
                recommended: true,
                model_id: "qwen2.5-3b-q4".to_owned(),
                bundle: placeholder_bundle("analyz0r-macos-aarch64.zip"),
            },
            AnalyzerVariant {
                id: "qwen2.5-3b-q4-x86".to_owned(),
                label: "Qwen2.5 3B Q4_K_M (x86-64 AVX2)".to_owned(),
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                hardware: HardwareClass::X86_64Avx2,
                recommended: true,
                model_id: "qwen2.5-3b-q4".to_owned(),
                bundle: placeholder_bundle("analyz0r-linux-x86_64.zip"),
            },
            AnalyzerVariant {
                id: "qwen2.5-3b-q4-windows".to_owned(),
                label: "Qwen2.5 3B Q4_K_M (Windows x86-64)".to_owned(),
                os: "windows".to_owned(),
                arch: "x86_64".to_owned(),
                hardware: HardwareClass::X86_64Avx2,
                recommended: true,
                model_id: "qwen2.5-3b-q4".to_owned(),
                bundle: placeholder_bundle("analyz0r-windows-x86_64.zip"),
            },
            AnalyzerVariant {
                id: "qwen2.5-3b-q4-generic".to_owned(),
                label: "Qwen2.5 3B Q4_K_M (CPU generic — slow)".to_owned(),
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                hardware: HardwareClass::Generic,
                recommended: false,
                model_id: "qwen2.5-3b-q4".to_owned(),
                bundle: placeholder_bundle("analyz0r-linux-x86_64-generic.zip"),
            },
        ],
    }
}

// ── Status ────────────────────────────────────────────────────────────────────

/// State of the analyzer install. Stable string values are emitted in the
/// Tauri DTO so the UI can switch on them without depending on Rust enum
/// reordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallState {
    NotInstalled,
    Downloading,
    Installed,
    BrokenInstall,
    IncompatibleVersion,
}

impl InstallState {
    fn as_str(self) -> &'static str {
        match self {
            InstallState::NotInstalled => "not_installed",
            InstallState::Downloading => "downloading",
            InstallState::Installed => "installed",
            InstallState::BrokenInstall => "broken_install",
            InstallState::IncompatibleVersion => "incompatible_version",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalyzerStatus {
    /// One of `not_installed | downloading | installed | broken_install |
    /// incompatible_version`. Source of truth for the UI; the legacy
    /// `installed` bool below is just `state == "installed"`.
    pub state: String,
    /// Convenience field kept for the existing UI; equals `state == "installed"`.
    pub installed: bool,
    /// File name of the installed model (if any) — read from disk separately
    /// from install.json so the user sees the actual `.gguf` file present.
    pub model_file: Option<String>,
    /// Detected hardware class ("apple_silicon", "x86_64_avx2", "generic").
    pub hardware: String,
    /// Variant id from install.json, when installed or in a broken install.
    pub variant_id: Option<String>,
    /// Bundle version from install.json, when installed.
    pub bundle_version: Option<String>,
    /// Variant id of the in-flight download, when `state == "downloading"`.
    pub downloading_variant_id: Option<String>,
}

#[tauri::command]
pub fn get_analyzer_status(
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
    tracker: State<'_, AnalyzerInstallTracker>,
) -> AnalyzerStatus {
    let downloading = tracker.0.lock().ok().and_then(|g| g.clone());
    let (state, install) = if downloading.is_some() {
        // The on-disk layout during a download is mid-state — surface that
        // instead of "installed/not_installed", which would flap.
        (InstallState::Downloading, None)
    } else {
        install_state_on_disk(&paths.0)
    };

    let model_file = first_gguf(&paths.0.analyzer_models_dir())
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned());

    let status = AnalyzerStatus {
        state: state.as_str().to_owned(),
        installed: state == InstallState::Installed,
        model_file,
        hardware: hardware_id(detect_hardware()),
        variant_id: install.as_ref().map(|i| i.variant_id.clone()),
        bundle_version: install.as_ref().map(|i| i.bundle_version.clone()),
        downloading_variant_id: downloading,
    };
    logger.0.debug(
        "analyzer-setup",
        None,
        &format!(
            "Analyzer status checked state={} variant={:?} hardware={}",
            status.state, status.variant_id, status.hardware
        ),
    );
    status
}

/// Resolve the on-disk install state by reading `install.json` and
/// cross-checking the layout. Distinguishes broken-install from
/// incompatible-version by parsing the JSON in two passes (raw `Value`
/// for the schema-version probe, then typed deserialization).
fn install_state_on_disk(paths: &HammorPaths) -> (InstallState, Option<AnalyzerInstall>) {
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

    // Layout sanity check: the entrypoint binary and at least one model
    // file must actually exist for the install to count as healthy.
    let entrypoint = paths.analyzer_dir().join(&install.entrypoint);
    let model_present = first_gguf(&paths.analyzer_models_dir()).is_some();
    if !entrypoint.exists() || !model_present {
        return (InstallState::BrokenInstall, Some(install));
    }
    (InstallState::Installed, Some(install))
}

// ── Manifest ──────────────────────────────────────────────────────────────────

/// Fetch the remote manifest; fall back to bundled default if unreachable.
#[tauri::command]
pub async fn fetch_analyzer_manifest(
    logger: State<'_, AnalyzerLoggerState>,
) -> Result<AnalyzerManifest, CommandError> {
    logger
        .0
        .info("analyzer-setup", None, "Fetching analyzer manifest");
    match reqwest::get(MANIFEST_URL).await {
        Ok(resp) if resp.status().is_success() => {
            let manifest: AnalyzerManifest = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("manifest JSON parse: {e}"))?;
            logger.0.info(
                "analyzer-setup",
                None,
                &format!(
                    "Fetched analyzer manifest with {} variants",
                    manifest.variants.len()
                ),
            );
            Ok(manifest)
        }
        _ => {
            logger.0.info(
                "analyzer-setup",
                None,
                "Using bundled analyzer manifest fallback",
            );
            Ok(bundled_manifest())
        }
    }
}

// ── Download ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub variant_id: String,
    pub bytes_downloaded: u64,
    pub bytes_total: u64,
    /// 0.0 – 100.0
    pub percent: f32,
    pub finished: bool,
    pub error: Option<String>,
}

/// Start a background download + install of the specified analyzer variant.
/// Emits `analyzer-download-progress` events throughout. Refuses to start
/// a second install while one is already in flight.
#[tauri::command]
pub async fn download_and_install_analyzer(
    app: AppHandle,
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
    tracker: State<'_, AnalyzerInstallTracker>,
    variant_id: String,
) -> Result<String, CommandError> {
    // Atomically claim the install slot. Holding the guard across the manifest
    // fetch is fine — the lock is uncontended in the steady state.
    {
        let mut guard = tracker
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("analyzer install tracker poisoned"))?;
        if let Some(active) = guard.as_ref() {
            return Err(
                anyhow::anyhow!("analyzer install already in progress (variant {active})").into(),
            );
        }
        *guard = Some(variant_id.clone());
    }

    let manifest = match reqwest::get(MANIFEST_URL).await {
        Ok(r) if r.status().is_success() => r.json::<AnalyzerManifest>().await.ok(),
        _ => None,
    }
    .unwrap_or_else(bundled_manifest);

    let variant = match manifest.variants.into_iter().find(|v| v.id == variant_id) {
        Some(v) => v,
        None => {
            // Release the slot we just claimed before returning the error.
            *tracker.0.lock().unwrap() = None;
            return Err(anyhow::anyhow!("unknown variant id: {variant_id}").into());
        }
    };

    let paths_for_task = paths.0.clone();
    let tracker_for_task = tracker.0.clone();
    let variant_id_ret = variant_id.clone();
    let logger = logger.0.clone();

    tokio::spawn(async move {
        logger.info(
            "analyzer-setup",
            None,
            &format!("Analyzer install task spawned for variant_id={variant_id}"),
        );
        let result = do_install(app.clone(), paths_for_task, variant).await;

        // Release the install slot before reporting outcome so a follow-up
        // install attempt isn't blocked by a stale tracker entry.
        if let Ok(mut guard) = tracker_for_task.lock() {
            *guard = None;
        }

        if let Err(e) = result {
            let message = format!("analyzer install failed for variant {variant_id}: {e}");
            report_user_relevant_error(
                &app,
                &logger,
                "analyzer-setup",
                "analyzer-install",
                None,
                &message,
            );
            let _ = app.emit(
                "analyzer-download-progress",
                DownloadProgress {
                    variant_id,
                    bytes_downloaded: 0,
                    bytes_total: 0,
                    percent: 0.0,
                    finished: true,
                    error: Some(message),
                },
            );
        } else {
            logger.info(
                "analyzer-setup",
                None,
                &format!("Analyzer install completed for variant_id={variant_id}"),
            );
        }
    });

    Ok(variant_id_ret)
}

/// Remove the entire analyzer install layout (bin/, runtime/, models/,
/// install.json, plus any leftover staging dir). Verdicts and reports in
/// engagement folders are untouched.
#[tauri::command]
pub fn uninstall_analyzer(
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
) -> Result<(), CommandError> {
    uninstall_layout(&paths.0).map_err(|e| {
        logger.0.error(
            "analyzer-setup",
            None,
            &format!("Analyzer uninstall failed: {e}"),
        );
        CommandError::from(e)
    })?;
    logger
        .0
        .info("analyzer-setup", None, "Analyzer uninstall completed");
    Ok(())
}

fn uninstall_layout(paths: &HammorPaths) -> anyhow::Result<()> {
    // Remove install metadata FIRST so a crash mid-cleanup leaves a
    // "not installed" state — better to lose orphan files than to leave
    // install.json pointing at a half-removed install.
    analyzer_install::remove(paths)?;
    let analyzer_dir = paths.analyzer_dir();
    for sub in ["bin", "runtime", "models", ".staging"] {
        let path = analyzer_dir.join(sub);
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .map_err(|e| anyhow::anyhow!("remove {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

// ── Install internals ─────────────────────────────────────────────────────────

/// Download the bundle, verify SHA-256, extract atomically, then write
/// install.json. Order matters — install.json is the "is installed?"
/// signal, so a crash before the final write leaves a "not installed"
/// state instead of a broken install pointing at half-extracted files.
async fn do_install(
    app: AppHandle,
    paths: HammorPaths,
    variant: AnalyzerVariant,
) -> anyhow::Result<()> {
    let analyzer_dir = paths.analyzer_dir();
    std::fs::create_dir_all(&analyzer_dir)?;
    let staging = analyzer_dir.join(".staging");

    // Wipe any half-baked previous attempt so we start from a clean slate.
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|e| anyhow::anyhow!("clean staging: {e}"))?;
    }
    std::fs::create_dir_all(&staging)?;

    // 1. Download to staging area, streaming SHA hash as we go.
    let bundle_filename = variant
        .bundle
        .url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("bundle.zip");
    let bundle_path = staging.join(bundle_filename);
    download_bundle(app.clone(), &variant, &bundle_path).await?;

    // 2. Extract — synchronous and CPU-bound, so push into spawn_blocking.
    let extract_dir = staging.join("extracted");
    let bundle_path_clone = bundle_path.clone();
    let extract_dir_clone = extract_dir.clone();
    tokio::task::spawn_blocking(move || extract_zip(&bundle_path_clone, &extract_dir_clone))
        .await
        .map_err(|e| anyhow::anyhow!("extract task join: {e}"))??;

    // 3. Atomically swap the install layout. Wipe any prior install first
    //    so leftover files (different model variant, etc.) don't shadow
    //    the new bundle.
    uninstall_layout(&paths)?;
    move_extracted_into_place(&extract_dir, &analyzer_dir)?;

    // 4. Mark the install as complete (last — see top-of-fn comment).
    let install = AnalyzerInstall {
        version: CURRENT_VERSION,
        bundle_version: variant.id.clone(),
        installed_at: runner::run::iso_now(),
        variant_id: variant.id.clone(),
        model_id: variant.model_id.clone(),
        platform: format!("{}-{}", variant.os, variant.arch),
        entrypoint: format!(
            "bin/{}",
            if variant.os == "windows" {
                "analyz0r.exe"
            } else {
                "analyz0r"
            }
        ),
    };
    analyzer_install::write(&paths, &install)?;

    // 5. Best-effort cleanup of staging.
    let _ = std::fs::remove_dir_all(&staging);

    // 6. Emit a final "finished" event so the UI knows to refresh.
    let _ = app.emit(
        "analyzer-download-progress",
        DownloadProgress {
            variant_id: variant.id,
            bytes_downloaded: variant.bundle.size_bytes,
            bytes_total: variant.bundle.size_bytes,
            percent: 100.0,
            finished: true,
            error: None,
        },
    );
    Ok(())
}

async fn download_bundle(
    app: AppHandle,
    variant: &AnalyzerVariant,
    bundle_path: &Path,
) -> anyhow::Result<()> {
    let variant_id = variant.id.clone();
    let url = variant.bundle.url.clone();
    let expected_sha256 = variant.bundle.sha256.clone();
    let total_bytes = variant.bundle.size_bytes;

    let mut response = reqwest::get(&url)
        .await
        .map_err(|e| anyhow::anyhow!("GET {url}: {e}"))?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {} for {url}", response.status()));
    }

    let mut file = tokio::fs::File::create(bundle_path).await?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| anyhow::anyhow!("stream error: {e}"))?
    {
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;

        let percent = if total_bytes > 0 {
            (downloaded as f32 / total_bytes as f32 * 100.0).min(100.0)
        } else {
            0.0
        };
        let _ = app.emit(
            "analyzer-download-progress",
            DownloadProgress {
                variant_id: variant_id.clone(),
                bytes_downloaded: downloaded,
                bytes_total: total_bytes,
                percent,
                finished: false,
                error: None,
            },
        );
    }
    file.flush().await?;
    drop(file);

    // Verify SHA256 unconditionally — the placeholder bundled-fallback
    // entries will fail verification by design (sha256 = "PLACEHOLDER")
    // so an offline user can never silently install an unverified blob.
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_sha256 {
        std::fs::remove_file(bundle_path).ok();
        return Err(anyhow::anyhow!(
            "SHA256 mismatch — expected {expected_sha256}, got {actual}"
        ));
    }
    Ok(())
}

/// Extract `zip_path` into `dest`. Uses `enclosed_name` so Zip-Slip-style
/// path-traversal entries are silently skipped instead of escaping the
/// destination directory.
fn extract_zip(zip_path: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    let file = std::fs::File::open(zip_path)
        .map_err(|e| anyhow::anyhow!("open {}: {e}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| anyhow::anyhow!("read zip {}: {e}", zip_path.display()))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| anyhow::anyhow!("zip entry {i}: {e}"))?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let outpath = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&outpath)?;
            continue;
        }
        if let Some(parent) = outpath.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&outpath)
            .map_err(|e| anyhow::anyhow!("create {}: {e}", outpath.display()))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| anyhow::anyhow!("write {}: {e}", outpath.display()))?;

        // Preserve the executable bit on unix so `bin/analyz0r` stays runnable.
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

/// Move every top-level entry of `extract_dir` into `analyzer_dir`. Uses
/// `rename`, which is atomic on the same filesystem; if the rename fails
/// because the destination already exists (race / leftover), it falls
/// back to remove-then-rename.
fn move_extracted_into_place(extract_dir: &Path, analyzer_dir: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(extract_dir)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", extract_dir.display()))?
    {
        let entry = entry?;
        let target = analyzer_dir.join(entry.file_name());
        if let Err(e) = std::fs::rename(entry.path(), &target) {
            // Already exists or cross-fs — wipe target and retry.
            if target.exists() {
                if target.is_dir() {
                    std::fs::remove_dir_all(&target).ok();
                } else {
                    std::fs::remove_file(&target).ok();
                }
                std::fs::rename(entry.path(), &target).map_err(|e2| {
                    anyhow::anyhow!("rename {} -> {}: {e2}", entry.path().display(), target.display())
                })?;
            } else {
                return Err(anyhow::anyhow!(
                    "rename {} -> {}: {e}",
                    entry.path().display(),
                    target.display()
                ));
            }
        }
    }
    Ok(())
}

// ── Hardware detection ────────────────────────────────────────────────────────

fn detect_hardware() -> HardwareClass {
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

fn hardware_id(hw: HardwareClass) -> String {
    match hw {
        HardwareClass::AppleSilicon => "apple_silicon",
        HardwareClass::X86_64Avx2 => "x86_64_avx2",
        HardwareClass::Generic => "generic",
    }
    .to_owned()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn first_gguf(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        (p.extension().and_then(|x| x.to_str()) == Some("gguf")).then_some(p)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled-fallback manifest must always parse with the current
    /// schema. Catches accidental drift in either side.
    #[test]
    fn bundled_manifest_roundtrips_through_json() {
        let original = bundled_manifest();
        let json = serde_json::to_string(&original).unwrap();
        let parsed: AnalyzerManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, original.version);
        assert_eq!(parsed.variants.len(), original.variants.len());
        for (a, b) in parsed.variants.iter().zip(original.variants.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn variant_serialises_with_new_bundle_shape() {
        let v = AnalyzerVariant {
            id: "v1".into(),
            label: "v1".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            hardware: HardwareClass::Generic,
            recommended: false,
            model_id: "qwen-test".into(),
            bundle: Artifact {
                url: "https://example/x.zip".into(),
                sha256: "abc".into(),
                size_bytes: 42,
            },
        };
        let json = serde_json::to_value(&v).unwrap();
        // Bundle is a single object, not split into model+runtime.
        assert!(json.get("bundle").is_some());
        assert!(json.get("model").is_none());
        assert!(json.get("runtime").is_none());
        assert_eq!(json["os"], "linux");
        assert_eq!(json["arch"], "x86_64");
        assert_eq!(json["model_id"], "qwen-test");
    }

    #[test]
    fn placeholder_sha_is_kept_invalid_so_install_fails() {
        // Sanity check: any placeholder bundle in the fallback must keep
        // a sentinel sha that real verification will reject.
        let m = bundled_manifest();
        for v in &m.variants {
            assert_eq!(
                v.bundle.sha256, "PLACEHOLDER",
                "fallback bundle {} must keep PLACEHOLDER sha",
                v.id
            );
        }
    }

    fn write_test_zip(zip_path: &Path) {
        use std::io::Write as _;
        let file = std::fs::File::create(zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        writer.add_directory("bin/", opts).unwrap();
        writer.start_file("bin/analyz0r", opts).unwrap();
        writer.write_all(b"#!/bin/sh\nexit 0\n").unwrap();

        writer.add_directory("models/", opts).unwrap();
        writer.start_file("models/test.gguf", opts).unwrap();
        writer.write_all(b"FAKE GGUF").unwrap();

        writer.finish().unwrap();
    }

    #[test]
    fn extract_zip_preserves_layout_and_skips_path_traversal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let zip_path = tmp.path().join("test.zip");

        // Build a zip with a normal entry plus a Zip-Slip-style entry that
        // tries to escape the destination — extract_zip must skip it.
        use std::io::Write as _;
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("bin/inside.txt", opts).unwrap();
        writer.write_all(b"safe").unwrap();
        writer.start_file("../escaped.txt", opts).unwrap();
        writer.write_all(b"BAD").unwrap();
        writer.finish().unwrap();

        let dest = tmp.path().join("out");
        extract_zip(&zip_path, &dest).unwrap();

        assert_eq!(
            std::fs::read_to_string(dest.join("bin/inside.txt")).unwrap(),
            "safe"
        );
        // The traversal entry must NOT have escaped to the parent.
        assert!(!tmp.path().join("escaped.txt").exists());
    }

    #[test]
    fn move_extracted_into_place_replaces_existing_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let analyzer = tmp.path().join("analyzer");
        let extract = tmp.path().join("extract");
        std::fs::create_dir_all(analyzer.join("bin")).unwrap();
        std::fs::write(analyzer.join("bin/old"), b"old").unwrap();
        std::fs::create_dir_all(extract.join("bin")).unwrap();
        std::fs::write(extract.join("bin/new"), b"new").unwrap();

        move_extracted_into_place(&extract, &analyzer).unwrap();
        assert!(analyzer.join("bin/new").exists());
        assert!(!analyzer.join("bin/old").exists()); // old wiped out
    }

    #[test]
    fn uninstall_layout_clears_everything() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        let analyzer = paths.analyzer_dir();

        // Lay down a fake install so we can prove uninstall_layout cleans it.
        for sub in ["bin", "runtime", "models", ".staging"] {
            let p = analyzer.join(sub);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("dummy"), b"x").unwrap();
        }
        analyzer_install::write(
            &paths,
            &AnalyzerInstall {
                version: CURRENT_VERSION,
                bundle_version: "v".into(),
                installed_at: "now".into(),
                variant_id: "v".into(),
                model_id: "m".into(),
                platform: "p".into(),
                entrypoint: "bin/x".into(),
            },
        )
        .unwrap();

        uninstall_layout(&paths).unwrap();

        assert!(!analyzer.join("bin").exists());
        assert!(!analyzer.join("runtime").exists());
        assert!(!analyzer.join("models").exists());
        assert!(!analyzer.join(".staging").exists());
        assert!(analyzer_install::read(&paths).unwrap().is_none());
    }

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
        // install.json + model present, but no bin/.
        let models_dir = paths.analyzer_dir().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("test.gguf"), b"FAKE").unwrap();
        analyzer_install::write(&paths, &sample_install()).unwrap();

        let (state, install) = install_state_on_disk(&paths);
        assert_eq!(state, InstallState::BrokenInstall);
        // install.json is parsed, so we still hand the metadata back.
        assert!(install.is_some());
    }

    #[test]
    fn state_is_broken_install_when_model_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        // install.json + bin present, but models/ empty.
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
        // Lay down an install.json with a version this binary doesn't understand.
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

    #[test]
    fn write_test_zip_is_used() {
        // Smoke-check the writer helper itself produces a valid archive.
        let tmp = tempfile::TempDir::new().unwrap();
        let zip_path = tmp.path().join("z.zip");
        write_test_zip(&zip_path);
        let dest = tmp.path().join("out");
        extract_zip(&zip_path, &dest).unwrap();
        assert!(dest.join("bin/analyz0r").exists());
        assert!(dest.join("models/test.gguf").exists());
    }
}
