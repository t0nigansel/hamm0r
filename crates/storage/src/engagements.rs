use std::path::Path;

use anyhow::{anyhow, Context as _};

use crate::types::EngagementMeta;
use crate::write::atomic_write;

const ENGAGEMENT_YAML: &str = "engagement.yaml";

/// Reject slugs that could escape the engagements directory or that are
/// obviously bogus. Keeps `engagements::delete` safe even if a caller
/// forgets to validate upstream.
fn ensure_safe_slug(slug: &str) -> anyhow::Result<()> {
    let trimmed = slug.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("engagement slug must not be empty"));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(anyhow!(
            "engagement slug '{slug}' is not a valid folder name"
        ));
    }
    Ok(())
}

/// Create a new engagement folder and write its `engagement.yaml`.
///
/// The engagement folder is `<engagements_dir>/<meta.slug>/`. Sub-directories
/// (`runs/`, `responses/`, `reports/`) are created at the same time so the
/// runner never has to create directories itself.
pub fn create(engagements_dir: &Path, meta: &EngagementMeta) -> anyhow::Result<()> {
    let root = engagements_dir.join(&meta.slug);
    for sub in ["runs", "responses", "reports"] {
        std::fs::create_dir_all(root.join(sub))
            .with_context(|| format!("cannot create {}/{sub}", root.display()))?;
    }

    let path = root.join(ENGAGEMENT_YAML);
    let yaml = serde_yaml::to_string(meta).context("cannot serialise engagement")?;
    atomic_write(&path, yaml.as_bytes())
}

/// Overwrite `engagement.yaml` for an existing engagement. Used to persist
/// late-bound fields like `target.scenario_id` after the engagement was
/// originally created with empty defaults.
pub fn save_meta(engagements_dir: &Path, meta: &EngagementMeta) -> anyhow::Result<()> {
    ensure_safe_slug(&meta.slug)?;
    let path = engagements_dir
        .join(&meta.slug)
        .join(ENGAGEMENT_YAML);
    let yaml = serde_yaml::to_string(meta).context("cannot serialise engagement")?;
    atomic_write(&path, yaml.as_bytes())
}

/// Load a single engagement's metadata by slug. Returns `None` when the
/// folder doesn't exist or its `engagement.yaml` is missing/unreadable.
pub fn load(engagements_dir: &Path, slug: &str) -> anyhow::Result<Option<EngagementMeta>> {
    ensure_safe_slug(slug)?;
    let path = engagements_dir.join(slug).join(ENGAGEMENT_YAML);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    Ok(serde_yaml::from_str::<EngagementMeta>(&raw).ok())
}

/// List all engagements found under `engagements_dir`.
///
/// Each sub-directory that contains a readable `engagement.yaml` is returned.
/// Directories without one are silently skipped (they may be stale or
/// partially created).
pub fn list(engagements_dir: &Path) -> anyhow::Result<Vec<EngagementMeta>> {
    if !engagements_dir.exists() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();

    for entry in std::fs::read_dir(engagements_dir).with_context(|| {
        format!(
            "cannot read engagements directory: {}",
            engagements_dir.display()
        )
    })? {
        let entry = entry?;
        let meta_path = entry.path().join(ENGAGEMENT_YAML);

        if !meta_path.exists() {
            continue;
        }

        let raw = match std::fs::read_to_string(&meta_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "[storage] skipping {}: cannot read: {}",
                    meta_path.display(),
                    e
                );
                continue;
            }
        };

        match serde_yaml::from_str::<EngagementMeta>(&raw) {
            Ok(meta) => results.push(meta),
            Err(e) => {
                eprintln!(
                    "[storage] skipping {}: cannot parse: {}",
                    meta_path.display(),
                    e
                );
            }
        }
    }

    // Stable order: sort by slug so the UI sees engagements in creation-date order.
    results.sort_by(|a, b| a.slug.cmp(&b.slug));

    Ok(results)
}

/// Permanently remove an engagement folder. Idempotent — succeeds when
/// the folder isn't there. Refuses path-traversal slugs.
///
/// Removes everything under `<engagements_dir>/<slug>/`: the run JSONLs,
/// verdict logs, response files, generated reports, and the
/// `engagement.yaml` itself. The caller (Tauri command layer) is
/// responsible for refusing the call while any run inside the
/// engagement is still active.
pub fn delete(engagements_dir: &Path, slug: &str) -> anyhow::Result<bool> {
    ensure_safe_slug(slug)?;
    let root = engagements_dir.join(slug);
    if !root.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&root)
        .with_context(|| format!("cannot remove engagement folder {}", root.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EngagementScope, EngagementTarget};
    use tempfile::TempDir;

    fn sample_meta(slug: &str) -> EngagementMeta {
        EngagementMeta {
            version: 1,
            slug: slug.into(),
            name: format!("Test engagement {slug}"),
            created_at: "2026-04-25T09:00:00Z".into(),
            target: EngagementTarget {
                scenario_id: "openai-chat".into(),
                notes: None,
            },
            scope: EngagementScope {
                prompt_files: vec!["injection-classics".into()],
            },
        }
    }

    #[test]
    fn create_and_list() {
        let dir = TempDir::new().unwrap();
        let meta = sample_meta("2026-04-25-acme");

        create(dir.path(), &meta).unwrap();

        // Engagement directory and sub-directories must exist.
        let root = dir.path().join(&meta.slug);
        assert!(root.join("runs").is_dir());
        assert!(root.join("responses").is_dir());
        assert!(root.join("reports").is_dir());

        let all = list(dir.path()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], meta);
    }

    #[test]
    fn list_multiple_sorted() {
        let dir = TempDir::new().unwrap();
        create(dir.path(), &sample_meta("2026-04-25-b")).unwrap();
        create(dir.path(), &sample_meta("2026-04-25-a")).unwrap();

        let all = list(dir.path()).unwrap();
        assert_eq!(all[0].slug, "2026-04-25-a");
        assert_eq!(all[1].slug, "2026-04-25-b");
    }

    #[test]
    fn list_empty_dir() {
        let dir = TempDir::new().unwrap();
        assert!(list(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn delete_removes_folder_and_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let meta = sample_meta("2026-05-11-acme");
        create(dir.path(), &meta).unwrap();
        assert!(dir.path().join(&meta.slug).is_dir());

        assert!(delete(dir.path(), &meta.slug).unwrap());
        assert!(!dir.path().join(&meta.slug).exists());

        // Second call is a no-op.
        assert!(!delete(dir.path(), &meta.slug).unwrap());
        assert!(list(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn delete_rejects_path_traversal() {
        let dir = TempDir::new().unwrap();
        assert!(delete(dir.path(), "").is_err());
        assert!(delete(dir.path(), "..").is_err());
        assert!(delete(dir.path(), "../escape").is_err());
        assert!(delete(dir.path(), "a/b").is_err());
        assert!(delete(dir.path(), r"a\b").is_err());
    }
}
