/// Analyzer activation: manifest fetch, hardware detection, model download.
///
/// This module is intentionally NOT behind `#[cfg(feature = "analyzer")]`
/// so that users can download the model file in the default build.
/// Manifest types are defined locally (mirroring `analyzer::manifest`) to
/// avoid a hard dependency on the optional `analyzer` crate.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter as _, State};
use tokio::io::AsyncWriteExt as _;

use super::{report_user_relevant_error, AnalyzerLoggerState, AppPaths};
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

#[derive(Debug, Clone, Serialize)]
pub struct AnalyzerStatus {
    /// `true` if at least one `.gguf` model file is present.
    pub installed: bool,
    /// File name of the installed model (if any).
    pub model_file: Option<String>,
    /// Detected hardware class ("apple_silicon", "x86_64_avx2", "generic").
    pub hardware: String,
}

#[tauri::command]
pub fn get_analyzer_status(
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
) -> AnalyzerStatus {
    let models_dir = paths.0.analyzer_models_dir();
    let model_file = first_gguf(&models_dir);
    let status = AnalyzerStatus {
        installed: model_file.is_some(),
        model_file: model_file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned()),
        hardware: hardware_id(detect_hardware()),
    };
    logger.0.debug(
        "analyzer-setup",
        None,
        &format!(
            "Analyzer status checked installed={} hardware={}",
            status.installed, status.hardware
        ),
    );
    status
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
/// Emits `analyzer-download-progress` events throughout.
#[tauri::command]
pub async fn download_and_install_analyzer(
    app: AppHandle,
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
    variant_id: String,
) -> Result<String, CommandError> {
    let manifest = match reqwest::get(MANIFEST_URL).await {
        Ok(r) if r.status().is_success() => r.json::<AnalyzerManifest>().await.ok(),
        _ => None,
    }
    .unwrap_or_else(bundled_manifest);

    let variant = manifest
        .variants
        .into_iter()
        .find(|v| v.id == variant_id)
        .ok_or_else(|| anyhow::anyhow!("unknown variant id: {variant_id}"))?;

    let models_dir = paths.0.analyzer_models_dir();
    let variant_id_ret = variant_id.clone();
    let logger = logger.0.clone();

    tokio::spawn(async move {
        logger.info(
            "analyzer-setup",
            None,
            &format!("Analyzer download task spawned for variant_id={variant_id}"),
        );
        if let Err(e) = do_download(app.clone(), models_dir, variant).await {
            let message = format!("analyzer download failed for variant {variant_id}: {e}");
            report_user_relevant_error(
                &app,
                &logger,
                "analyzer-setup",
                "analyzer-download",
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
                &format!("Analyzer download completed for variant_id={variant_id}"),
            );
        }
    });

    Ok(variant_id_ret)
}

/// Remove the currently installed model file (if any).
#[tauri::command]
pub fn uninstall_analyzer(
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
) -> Result<(), CommandError> {
    let models_dir = paths.0.analyzer_models_dir();
    if let Some(path) = first_gguf(&models_dir) {
        logger.0.info(
            "analyzer-setup",
            None,
            &format!("Removing analyzer model {}", path.display()),
        );
        std::fs::remove_file(&path)
            .map_err(|e| anyhow::anyhow!("remove {}: {e}", path.display()))?;
        logger
            .0
            .info("analyzer-setup", None, "Analyzer model removed");
    } else {
        logger.0.info(
            "analyzer-setup",
            None,
            "Uninstall requested with no analyzer model present",
        );
    }
    Ok(())
}

// ── Download internals ────────────────────────────────────────────────────────

async fn do_download(
    app: AppHandle,
    models_dir: PathBuf,
    variant: AnalyzerVariant,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&models_dir)?;

    let variant_id = variant.id.clone();
    let url = variant.bundle.url.clone();
    let expected_sha256 = variant.bundle.sha256.clone();
    let total_bytes = variant.bundle.size_bytes;

    let filename = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("bundle.zip");
    let dest = models_dir.join(filename);
    let tmp = models_dir.join(format!("{filename}.download"));

    let mut response = reqwest::get(&url)
        .await
        .map_err(|e| anyhow::anyhow!("GET {url}: {e}"))?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {} for {url}", response.status()));
    }

    let mut file = tokio::fs::File::create(&tmp).await?;
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
        std::fs::remove_file(&tmp).ok();
        return Err(anyhow::anyhow!(
            "SHA256 mismatch — expected {expected_sha256}, got {actual}"
        ));
    }

    // Atomic replace.
    if let Some(old) = first_gguf(&models_dir) {
        if old != dest {
            std::fs::remove_file(&old).ok();
        }
    }
    std::fs::rename(&tmp, &dest)?;

    let _ = app.emit(
        "analyzer-download-progress",
        DownloadProgress {
            variant_id,
            bytes_downloaded: downloaded,
            bytes_total: total_bytes,
            percent: 100.0,
            finished: true,
            error: None,
        },
    );
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
}
