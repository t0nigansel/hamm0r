//! Analyzer activation: Tauri command surface.
//!
//! Submodules:
//! - [`manifest`] — manifest types and the `fetch_manifest_internal` helper.
//! - [`detect`]   — install-state detection and host-hardware classification.
//! - [`install`]  — bundle download/extract/swap, uninstall, version gate.
//!
//! The `#[tauri::command]` handlers live here, in the parent module, so
//! that `main.rs` can register them under one path. The submodules carry
//! the actual logic and their own unit tests.

mod detect;
mod install;
mod manifest;

use serde::Serialize;
use storage::HammorPaths;
use tauri::{AppHandle, Emitter as _, State};

use super::{report_user_relevant_error, AnalyzerInstallTracker, AnalyzerLoggerState, AppPaths};
use crate::error::CommandError;

pub use manifest::AnalyzerManifest;

use detect::{detect_hardware, first_gguf, hardware_id, install_state_on_disk, InstallState};
use install::{do_install, uninstall_layout, version_at_least, DownloadProgress};
use manifest::{fetch_manifest_internal, MANIFEST_URL};

// ── Status DTO + command ──────────────────────────────────────────────────────

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

// ── Manifest fetch (Tauri-facing) ─────────────────────────────────────────────

#[tauri::command]
pub async fn fetch_analyzer_manifest(
    logger: State<'_, AnalyzerLoggerState>,
) -> Result<AnalyzerManifest, CommandError> {
    logger
        .0
        .info("analyzer-setup", None, "Fetching analyzer manifest");
    let resp = reqwest::get(MANIFEST_URL)
        .await
        .map_err(|e| anyhow::anyhow!("could not reach analyzer manifest at {MANIFEST_URL}: {e}"))?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "analyzer manifest fetch returned HTTP {} — try again later",
            resp.status()
        )
        .into());
    }
    let manifest: AnalyzerManifest = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("analyzer manifest is not valid JSON: {e}"))?;
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

// ── Install command (spawns the install task) ─────────────────────────────────

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

    let manifest = match fetch_manifest_internal().await {
        Ok(m) => m,
        Err(e) => {
            *tracker.0.lock().unwrap() = None;
            return Err(e.into());
        }
    };

    // Reject up-front if this app build is older than the manifest demands.
    // Cheaper than discovering the mismatch after a multi-GB download, and
    // gives the user a clear "upgrade hamm0r" message instead of a broken
    // install. Compared as plain "X.Y.Z" — we author both sides.
    let app_version = env!("CARGO_PKG_VERSION");
    if !version_at_least(app_version, &manifest.minimum_hamm0r_version) {
        *tracker.0.lock().unwrap() = None;
        return Err(anyhow::anyhow!(
            "this hamm0r build ({app_version}) is older than the analyzer manifest requires ({}); upgrade hamm0r before installing the analyzer",
            manifest.minimum_hamm0r_version
        )
        .into());
    }

    let variant = match manifest.variants.into_iter().find(|v| v.id == variant_id) {
        Some(v) => v,
        None => {
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

// ── Uninstall command ─────────────────────────────────────────────────────────

#[tauri::command]
pub fn uninstall_analyzer(
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
) -> Result<(), CommandError> {
    let paths: &HammorPaths = &paths.0;
    uninstall_layout(paths).map_err(|e| {
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
