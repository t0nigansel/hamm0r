//! Generic helper for `load_all`-style functions across storage modules.
//!
//! Every module that owns a directory of YAML artifacts (requests, scenarios,
//! targets, prompts, …) needs the same loop: list `*.yaml` files, parse each
//! one, skip and log the broken ones, return a map keyed by filename stem.
//! This module is the single implementation of that loop.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Context as _;
use serde::de::DeserializeOwned;

/// Load every `*.yaml` file in `dir` and deserialize it as `T`, keyed by
/// filename stem. Missing directories are not an error (returns empty map);
/// individual unreadable or malformed files are skipped and logged to stderr.
///
/// `kind` is the human-readable label used in error and log messages
/// (e.g. `"requests"`, `"scenarios"`).
pub fn load_all<T: DeserializeOwned>(dir: &Path, kind: &str) -> anyhow::Result<HashMap<String, T>> {
    if !dir.exists() {
        return Ok(HashMap::new());
    }

    let mut map = HashMap::new();

    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("cannot read {kind} directory: {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();

        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[storage] skipping {}: cannot read: {}", path.display(), e);
                continue;
            }
        };

        match serde_yaml::from_str::<T>(&raw) {
            Ok(value) => {
                map.insert(stem, value);
            }
            Err(e) => {
                eprintln!("[storage] skipping {}: cannot parse: {}", path.display(), e);
            }
        }
    }

    Ok(map)
}
