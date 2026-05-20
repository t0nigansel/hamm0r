use std::path::Path;

use serde::{Deserialize, Serialize};
use storage::prompts;
use storage::types::{PromptEntry, PromptMode, Severity};
use tauri::State;

use super::AppPaths;
use crate::error::CommandError;

const BUNDLED: &[(&str, &str)] = &[
    (
        "library.yaml",
        include_str!("../../../../prompts/library.yaml"),
    ),
    (
        "injection-classics.yaml",
        include_str!("../../../../prompts/injection-classics.yaml"),
    ),
    ("exfil.yaml", include_str!("../../../../prompts/exfil.yaml")),
    (
        "baselines.yaml",
        include_str!("../../../../prompts/baselines.yaml"),
    ),
    (
        "unbounded-consumption.yaml",
        include_str!("../../../../prompts/unbounded-consumption.yaml"),
    ),
    (
        "owasp-llm-2025.yaml",
        include_str!("../../../../prompts/owasp-llm-2025.yaml"),
    ),
    (
        "owasp-agentic-2026.yaml",
        include_str!("../../../../prompts/owasp-agentic-2026.yaml"),
    ),
];

#[derive(Debug, Serialize)]
pub struct SeedResult {
    pub loaded: usize,
    pub skipped: usize,
}

/// Write bundled YAMLs into `dir`.
///
/// Existing files are never overwritten. This is true both for startup
/// seeding and for the Library seed button: the user's prompt library is
/// sacred, and starter-library updates arrive as new missing files only.
fn write_bundled(dir: &Path) -> anyhow::Result<SeedResult> {
    let mut loaded = 0usize;
    let mut skipped = 0usize;

    for (filename, contents) in BUNDLED {
        let dest = dir.join(filename);
        if dest.exists() {
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
    write_bundled(dir)
}

/// Tauri command called by the Library → Seed button.
#[tauri::command]
pub fn seed_library(paths: State<'_, AppPaths>, _update: bool) -> Result<SeedResult, CommandError> {
    let dir = paths.0.prompts_dir();
    std::fs::create_dir_all(&dir).map_err(anyhow::Error::from)?;
    write_bundled(&dir).map_err(Into::into)
}

// ── Prompt CRUD ───────────────────────────────────────────────────────────────
//
// The UI sees Prompts in human terms: a Name, a Text, a Severity, an
// optional OWASP reference, and an optional bag of free-form tags. The
// id field is auto-derived from the name on first save and never changes
// thereafter — it remains the stable key used by run JSONL, verdict
// logs, and Scenario provenance. The category is the YAML filename the
// prompt lives in (one file per theme).

/// DTO sent from the UI's prompt editor. `id` is the stable key (empty on
/// create); `name` is what the user types.
#[derive(Debug, Clone, Deserialize)]
pub struct PromptDto {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Target category file (`~/hamm0r/prompts/<category>.yaml`).
    pub category: String,
    #[serde(default)]
    pub text: String,
    pub severity: Severity,
    #[serde(default = "default_mode")]
    pub mode: PromptMode,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub owasp_ref: Option<String>,
}

fn default_mode() -> PromptMode {
    PromptMode::Single
}

/// Look up a prompt by id across all categories. Returns the entry plus
/// its category so the UI can pre-fill the editor without a second call.
#[derive(Debug, Clone, Serialize)]
pub struct PromptWithCategory {
    pub category: String,
    pub prompt: PromptEntry,
}

#[tauri::command]
pub fn get_prompt(
    paths: State<'_, AppPaths>,
    id: String,
) -> Result<Option<PromptWithCategory>, CommandError> {
    let map = prompts::load_all(&paths.0.prompts_dir())?;
    for (category, entries) in map {
        if let Some(p) = entries.into_iter().find(|p| p.id == id) {
            return Ok(Some(PromptWithCategory {
                category,
                prompt: p,
            }));
        }
    }
    Ok(None)
}

#[tauri::command]
pub fn create_prompt(
    paths: State<'_, AppPaths>,
    dto: PromptDto,
) -> Result<PromptEntry, CommandError> {
    let name = dto.name.trim().to_owned();
    if name.is_empty() {
        return Err(anyhow::anyhow!("Name is required").into());
    }
    let category = dto.category.trim().to_owned();
    if category.is_empty() {
        return Err(anyhow::anyhow!("Category is required").into());
    }

    let map = prompts::load_all(&paths.0.prompts_dir())?;
    let existing_ids: std::collections::HashSet<String> = map
        .values()
        .flat_map(|v| v.iter().map(|p| p.id.clone()))
        .collect();
    let id = unique_id_from_name(&name, &existing_ids);

    let entry = PromptEntry {
        id: id.clone(),
        name: Some(name),
        text: dto.text,
        severity: dto.severity,
        mode: dto.mode,
        turns: Vec::new(),
        tags: dedupe_tags(dto.tags),
        owasp_ref: normalize_owasp_ref(dto.owasp_ref),
    };
    prompts::save_one(&paths.0.prompts_dir(), &category, &entry)?;
    Ok(entry)
}

#[tauri::command]
pub fn update_prompt(
    paths: State<'_, AppPaths>,
    dto: PromptDto,
) -> Result<PromptEntry, CommandError> {
    if dto.id.trim().is_empty() {
        return Err(anyhow::anyhow!("Prompt id is required for update").into());
    }
    let name = dto.name.trim().to_owned();
    if name.is_empty() {
        return Err(anyhow::anyhow!("Name is required").into());
    }
    let new_category = dto.category.trim().to_owned();
    if new_category.is_empty() {
        return Err(anyhow::anyhow!("Category is required").into());
    }

    // If the user moved the prompt to a different category, remove the
    // old row first. We never re-slug the id — see PromptEntry::id docs.
    let dir = paths.0.prompts_dir();
    let map = prompts::load_all(&dir)?;
    let mut old_category: Option<String> = None;
    for (cat, entries) in &map {
        if entries.iter().any(|p| p.id == dto.id) {
            old_category = Some(cat.clone());
            break;
        }
    }
    let Some(old_category) = old_category else {
        return Err(anyhow::anyhow!("Prompt '{}' not found", dto.id).into());
    };

    let entry = PromptEntry {
        id: dto.id.clone(),
        name: Some(name),
        text: dto.text,
        severity: dto.severity,
        mode: dto.mode,
        turns: Vec::new(),
        tags: dedupe_tags(dto.tags),
        owasp_ref: normalize_owasp_ref(dto.owasp_ref),
    };

    if old_category != new_category {
        prompts::delete_one(&dir, &old_category, &entry.id)?;
    }
    prompts::save_one(&dir, &new_category, &entry)?;
    Ok(entry)
}

#[tauri::command]
pub fn delete_prompt(paths: State<'_, AppPaths>, id: String) -> Result<bool, CommandError> {
    let dir = paths.0.prompts_dir();
    let map = prompts::load_all(&dir)?;
    for (cat, entries) in map {
        if entries.iter().any(|p| p.id == id) {
            return prompts::delete_one(&dir, &cat, &id).map_err(Into::into);
        }
    }
    Ok(false)
}

/// Slugify a name to kebab-case ASCII. Strips diacritics in the cheap way
/// (anything non-ASCII collapses to '-'). Adds a `-N` suffix when the
/// slug would collide with an existing id.
fn unique_id_from_name(name: &str, existing: &std::collections::HashSet<String>) -> String {
    let base = slugify(name);
    if !existing.contains(&base) && !base.is_empty() {
        return base;
    }
    let stem = if base.is_empty() { "prompt" } else { &base };
    for n in 2..u32::MAX {
        let candidate = format!("{stem}-{n}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    // Practically unreachable; final fallback.
    format!("{stem}-{}", existing.len() + 1)
}

fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = true;
    for ch in input.chars() {
        let mapped: char = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if prev_dash {
                continue;
            }
            prev_dash = true;
            out.push('-');
        } else {
            prev_dash = false;
            out.push(mapped);
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn dedupe_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    tags.into_iter()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .collect()
}

fn normalize_owasp_ref(raw: Option<String>) -> Option<String> {
    let v = raw?.trim().to_owned();
    if v.is_empty() {
        None
    } else {
        Some(v.to_uppercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn slugify_handles_unicode_and_punctuation() {
        assert_eq!(slugify("Hello World!"), "hello-world");
        assert_eq!(slugify("  spaced  "), "spaced");
        assert_eq!(slugify("user/path-thing"), "user-path-thing");
        assert_eq!(slugify("ÜberCool 🙃 attack"), "bercool-attack");
        assert_eq!(slugify("---"), "");
    }

    #[test]
    fn unique_id_appends_suffix_on_collision() {
        let mut existing = HashSet::new();
        existing.insert("ignore-me".to_owned());
        assert_eq!(unique_id_from_name("Ignore me", &existing), "ignore-me-2");

        existing.insert("ignore-me-2".to_owned());
        assert_eq!(unique_id_from_name("Ignore me", &existing), "ignore-me-3");
    }

    #[test]
    fn unique_id_empty_falls_back_to_prompt_n() {
        let mut existing = HashSet::new();
        let id = unique_id_from_name("🙃🙃🙃", &existing);
        assert_eq!(id, "prompt-2");
        existing.insert(id);
        assert_eq!(unique_id_from_name("🙃🙃🙃", &existing), "prompt-3");
    }

    #[test]
    fn dedupe_tags_strips_blanks_and_duplicates() {
        let got = dedupe_tags(vec![
            "  one ".into(),
            "two".into(),
            "one".into(),
            "".into(),
            "  ".into(),
            "two".into(),
        ]);
        assert_eq!(got, vec!["one", "two"]);
    }

    #[test]
    fn normalize_owasp_ref_uppercases_and_trims() {
        assert_eq!(
            normalize_owasp_ref(Some(" a01 ".into())).as_deref(),
            Some("A01")
        );
        assert_eq!(normalize_owasp_ref(Some("".into())), None);
        assert_eq!(normalize_owasp_ref(None), None);
    }

    #[test]
    fn seed_library_never_overwrites_existing_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let existing = dir.path().join("library.yaml");
        storage::atomic_write(&existing, b"# user edited\n[]\n").unwrap();

        let result = seed_on_first_launch(dir.path()).unwrap();

        assert!(result.loaded > 0);
        assert!(result.skipped > 0);
        let raw = std::fs::read_to_string(existing).unwrap();
        assert_eq!(raw, "# user edited\n[]\n");
        assert!(dir.path().join("owasp-llm-2025.yaml").exists());
        assert!(dir.path().join("owasp-agentic-2026.yaml").exists());
    }
}
