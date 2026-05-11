//! Bundle install pipeline: download → SHA-verify → extract → atomic
//! swap → write `install.json`. Also owns `uninstall_layout` and the
//! `version_at_least` gate.
//!
//! Failure-handling priorities (read in order):
//! 1. install.json is the "is installed?" signal — write it last.
//! 2. The existing install is moved aside before the new layout lands;
//!    on failure, the backup is restored. A user who hits a flaky
//!    download from a Repair must not end up worse off than before.
//! 3. `do_install` only owns paths under `~/hamm0r/analyzer/`. It must
//!    never touch engagement folders, which carry user data.

use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};
use storage::analyzer_install::{self, AnalyzerInstall, CURRENT_VERSION};
use storage::HammorPaths;
use tauri::{AppHandle, Emitter as _};
use tokio::io::AsyncWriteExt as _;

use super::manifest::AnalyzerVariant;

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

/// Download the bundle, verify SHA-256, swap the layout into place, and
/// write install.json last.
///
/// Failure handling is the part to read carefully: an existing install
/// is moved aside to a backup directory *before* the new layout is
/// renamed in. If the swap fails partway through, we restore from the
/// backup so the user is not left worse off than before. install.json
/// is written last, so a crash anywhere before that final step leaves
/// `not_installed` rather than a broken install.
pub(super) async fn do_install(
    app: AppHandle,
    paths: HammorPaths,
    variant: AnalyzerVariant,
) -> anyhow::Result<()> {
    let analyzer_dir = paths.analyzer_dir();
    std::fs::create_dir_all(&analyzer_dir)?;
    let staging = analyzer_dir.join(".staging");

    // Wipe any half-baked previous attempt so we start from a clean slate.
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| anyhow::anyhow!("clean staging: {e}"))?;
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

    // 3. Move the existing install aside, slot the new one in. If the
    //    move-in fails, restore the backup so a Repair triggered by a
    //    flaky filesystem cannot strand the user without an analyzer.
    let backup_dir = staging.join(".old-install");
    backup_existing_install(&analyzer_dir, &backup_dir)?;
    if let Err(e) = move_extracted_into_place(&extract_dir, &analyzer_dir) {
        // Best-effort rollback: clear whatever did land, restore backup.
        for sub in ["bin", "runtime", "models"] {
            let p = analyzer_dir.join(sub);
            if p.exists() {
                let _ = std::fs::remove_dir_all(&p);
            }
        }
        let _ = restore_backup(&backup_dir, &analyzer_dir);
        return Err(e);
    }

    // 4. Mark the install as complete (last — see top-of-fn comment).
    let model_file = locate_installed_model(&analyzer_dir);
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
        model_file,
    };
    analyzer_install::write(&paths, &install)?;

    // 5. Best-effort cleanup of staging (which contains the now-stale
    //    backup of the old install).
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

/// Move the current install's top-level subdirs and `install.json` into
/// `backup_dir`. The backup lives inside `.staging/`, so it is wiped
/// alongside other staging artifacts on success.
fn backup_existing_install(analyzer_dir: &Path, backup_dir: &Path) -> anyhow::Result<()> {
    if backup_dir.exists() {
        std::fs::remove_dir_all(backup_dir)
            .map_err(|e| anyhow::anyhow!("clean backup dir: {e}"))?;
    }
    std::fs::create_dir_all(backup_dir)?;
    for sub in ["bin", "runtime", "models"] {
        let from = analyzer_dir.join(sub);
        if from.exists() {
            let to = backup_dir.join(sub);
            std::fs::rename(&from, &to)
                .map_err(|e| anyhow::anyhow!("backup {}: {e}", from.display()))?;
        }
    }
    let install_json = analyzer_dir.join("install.json");
    if install_json.exists() {
        std::fs::rename(&install_json, backup_dir.join("install.json"))
            .map_err(|e| anyhow::anyhow!("backup install.json: {e}"))?;
    }
    Ok(())
}

/// Best-effort rollback of `backup_existing_install`. Used when the new
/// layout fails to land; logs nothing on failure because we are already
/// on an error path and the caller's error is the one that matters.
fn restore_backup(backup_dir: &Path, analyzer_dir: &Path) -> anyhow::Result<()> {
    if !backup_dir.exists() {
        return Ok(());
    }
    for sub in ["bin", "runtime", "models"] {
        let from = backup_dir.join(sub);
        if from.exists() {
            let _ = std::fs::rename(&from, analyzer_dir.join(sub));
        }
    }
    let install_json = backup_dir.join("install.json");
    if install_json.exists() {
        let _ = std::fs::rename(&install_json, analyzer_dir.join("install.json"));
    }
    Ok(())
}

/// Find the model file that was just extracted into `analyzer_dir/models`.
/// Returns the path relative to `analyzer_dir` (e.g. `"models/foo.gguf"`)
/// so it can be stored in install.json. None when no `.gguf` was found —
/// the caller still writes install.json so detection can flag broken.
fn locate_installed_model(analyzer_dir: &Path) -> Option<String> {
    let models_dir = analyzer_dir.join("models");
    let gguf = super::detect::first_gguf(&models_dir)?;
    let name = gguf.file_name()?.to_str()?;
    Some(format!("models/{name}"))
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
            if target.exists() {
                if target.is_dir() {
                    std::fs::remove_dir_all(&target).ok();
                } else {
                    std::fs::remove_file(&target).ok();
                }
                std::fs::rename(entry.path(), &target).map_err(|e2| {
                    anyhow::anyhow!(
                        "rename {} -> {}: {e2}",
                        entry.path().display(),
                        target.display()
                    )
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

/// Remove the entire analyzer install layout (bin/, runtime/, models/,
/// install.json, plus any leftover staging dir). Verdicts and reports in
/// engagement folders are untouched — they are user data.
pub(super) fn uninstall_layout(paths: &HammorPaths) -> anyhow::Result<()> {
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

/// Compare two dotted version strings componentwise as integers. Trailing
/// pre-release / build suffixes (after `-` or `+`) are stripped — we gate
/// only on `MAJOR.MINOR.PATCH`. Returns `false` if either string fails to
/// parse: this is an install gate, so ambiguous input must block, not
/// silently allow.
pub(super) fn version_at_least(actual: &str, minimum: &str) -> bool {
    fn parts(v: &str) -> Option<Vec<u32>> {
        let core = v.split(['-', '+']).next().unwrap_or(v);
        if core.is_empty() {
            return None;
        }
        core.split('.').map(|p| p.parse::<u32>().ok()).collect()
    }
    let (a, m) = match (parts(actual), parts(minimum)) {
        (Some(a), Some(m)) => (a, m),
        _ => return false,
    };
    let len = a.len().max(m.len());
    for i in 0..len {
        let av = *a.get(i).unwrap_or(&0);
        let mv = *m.get(i).unwrap_or(&0);
        if av > mv {
            return true;
        }
        if av < mv {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!analyzer.join("bin/old").exists());
    }

    #[test]
    fn uninstall_layout_clears_everything() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());
        let analyzer = paths.analyzer_dir();

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
                model_file: None,
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

    #[test]
    fn uninstall_layout_does_not_touch_engagement_artifacts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());

        let analyzer = paths.analyzer_dir();
        std::fs::create_dir_all(analyzer.join("bin")).unwrap();
        std::fs::write(analyzer.join("bin/dummy"), b"x").unwrap();
        analyzer_install::write(
            &paths,
            &AnalyzerInstall {
                version: CURRENT_VERSION,
                bundle_version: "v".into(),
                installed_at: "now".into(),
                variant_id: "v".into(),
                model_id: "m".into(),
                platform: "p".into(),
                entrypoint: "bin/dummy".into(),
                model_file: None,
            },
        )
        .unwrap();

        let engagement_dir = paths.engagement_dir("demo");
        let runs_dir = engagement_dir.join("runs");
        let reports_dir = engagement_dir.join("reports");
        std::fs::create_dir_all(&runs_dir).unwrap();
        std::fs::create_dir_all(&reports_dir).unwrap();
        let verdict_path = runs_dir.join("run-001.verdicts.jsonl");
        let report_path = reports_dir.join("report-001.html");
        std::fs::write(&verdict_path, b"{\"seq\":1}\n").unwrap();
        std::fs::write(&report_path, b"<html></html>").unwrap();

        uninstall_layout(&paths).unwrap();

        assert!(!analyzer.join("bin").exists());
        assert!(analyzer_install::read(&paths).unwrap().is_none());
        assert!(verdict_path.exists(), "verdict file must survive uninstall");
        assert!(report_path.exists(), "report file must survive uninstall");
    }

    #[test]
    fn version_at_least_handles_typical_cases() {
        assert!(version_at_least("0.1.0", "0.1.0"));
        assert!(version_at_least("0.2.0", "0.1.0"));
        assert!(version_at_least("0.10.0", "0.2.0"));
        assert!(version_at_least("1.0.0", "0.99.99"));
        assert!(!version_at_least("0.1.0", "0.2.0"));
        assert!(!version_at_least("0.1.0", "0.1.1"));
        assert!(version_at_least("1.2.3-alpha", "1.2.3"));
        assert!(!version_at_least("0.1.0", "abc.def"));
        assert!(!version_at_least("garbage", "0.1.0"));
        assert!(!version_at_least("", "0.1.0"));
    }

    #[test]
    fn restore_backup_recovers_old_install_when_swap_fails() {
        let tmp = tempfile::TempDir::new().unwrap();
        let analyzer = tmp.path().join("analyzer");
        std::fs::create_dir_all(analyzer.join("bin")).unwrap();
        std::fs::create_dir_all(analyzer.join("models")).unwrap();
        std::fs::write(analyzer.join("bin/old"), b"old-bin").unwrap();
        std::fs::write(analyzer.join("models/old.gguf"), b"old-model").unwrap();
        std::fs::write(analyzer.join("install.json"), b"{}").unwrap();

        let backup = tmp.path().join("backup");
        backup_existing_install(&analyzer, &backup).unwrap();

        assert!(!analyzer.join("bin").exists());
        assert!(backup.join("bin/old").exists());
        assert!(backup.join("install.json").exists());

        restore_backup(&backup, &analyzer).unwrap();

        assert_eq!(std::fs::read(analyzer.join("bin/old")).unwrap(), b"old-bin");
        assert_eq!(
            std::fs::read(analyzer.join("models/old.gguf")).unwrap(),
            b"old-model"
        );
        assert!(analyzer.join("install.json").exists());
    }

    #[test]
    fn locate_installed_model_returns_relative_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let analyzer = tmp.path().join("analyzer");
        std::fs::create_dir_all(analyzer.join("models")).unwrap();
        std::fs::write(analyzer.join("models/qwen.gguf"), b"x").unwrap();
        assert_eq!(
            locate_installed_model(&analyzer).as_deref(),
            Some("models/qwen.gguf")
        );
    }

    #[test]
    fn write_test_zip_is_used() {
        let tmp = tempfile::TempDir::new().unwrap();
        let zip_path = tmp.path().join("z.zip");
        write_test_zip(&zip_path);
        let dest = tmp.path().join("out");
        extract_zip(&zip_path, &dest).unwrap();
        assert!(dest.join("bin/analyz0r").exists());
        assert!(dest.join("models/test.gguf").exists());
    }
}
