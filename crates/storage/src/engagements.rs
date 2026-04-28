use std::path::Path;

use anyhow::Context as _;

use crate::types::EngagementMeta;
use crate::write::atomic_write;

const ENGAGEMENT_YAML: &str = "engagement.yaml";

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

        let raw = std::fs::read_to_string(&meta_path)
            .with_context(|| format!("cannot read {}", meta_path.display()))?;

        let meta: EngagementMeta = serde_yaml::from_str(&raw)
            .with_context(|| format!("cannot parse {}", meta_path.display()))?;

        results.push(meta);
    }

    // Stable order: sort by slug so the UI sees engagements in creation-date order.
    results.sort_by(|a, b| a.slug.cmp(&b.slug));

    Ok(results)
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
                request_id: "openai-chat".into(),
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
}
