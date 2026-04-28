use std::path::Path;

use serde::Serialize;
use tauri::State;

use super::AppPaths;
use crate::error::CommandError;

const BUNDLED: &[(&str, &str)] = &[
    ("library.yaml", include_str!("../../../../prompts/library.yaml")),
    (
        "injection-classics.yaml",
        include_str!("../../../../prompts/injection-classics.yaml"),
    ),
    ("exfil.yaml", include_str!("../../../../prompts/exfil.yaml")),
    ("baselines.yaml", include_str!("../../../../prompts/baselines.yaml")),
];

#[derive(Debug, Serialize)]
pub struct SeedResult {
    pub loaded: usize,
    pub skipped: usize,
}

/// Write bundled YAMLs into `dir`.
///
/// `update = false` skips files that already exist (first-launch mode,
/// preserves user edits). `update = true` overwrites every file (Seed button).
fn write_bundled(dir: &Path, update: bool) -> anyhow::Result<SeedResult> {
    let mut loaded = 0usize;
    let mut skipped = 0usize;

    for (filename, contents) in BUNDLED {
        let dest = dir.join(filename);
        if !update && dest.exists() {
            skipped += 1;
            continue;
        }
        storage::atomic_write(&dest, contents.as_bytes())?;
        loaded += 1;
    }

    Ok(SeedResult { loaded, skipped })
}

/// Called from `first_launch_hook` — seeds missing files only.
pub fn seed_on_first_launch(dir: &Path) -> anyhow::Result<SeedResult> {
    std::fs::create_dir_all(dir)?;
    write_bundled(dir, false)
}

/// Tauri command called by the Library → Seed button.
#[tauri::command]
pub fn seed_library(
    paths: State<'_, AppPaths>,
    update: bool,
) -> Result<SeedResult, CommandError> {
    let dir = paths.0.prompts_dir();
    std::fs::create_dir_all(&dir).map_err(anyhow::Error::from)?;
    write_bundled(&dir, update).map_err(Into::into)
}
