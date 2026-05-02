use std::collections::HashMap;
use std::path::Path;

use anyhow::Context as _;

use crate::types::Target;
use crate::write::atomic_write;

/// Load all targets from `dir`, keyed by filename stem.
pub fn load_all(dir: &Path) -> anyhow::Result<HashMap<String, Target>> {
    if !dir.exists() {
        return Ok(HashMap::new());
    }

    let mut map = HashMap::new();

    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("cannot read targets directory: {}", dir.display()))?
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

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;

        let target: Target = serde_yaml::from_str(&raw)
            .with_context(|| format!("cannot parse {}", path.display()))?;

        map.insert(stem, target);
    }

    Ok(map)
}

/// Remove a target YAML file by id. Returns Ok even if the file did not exist.
pub fn delete(dir: &Path, id: &str) -> anyhow::Result<()> {
    let path = dir.join(format!("{id}.yaml"));
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("cannot delete {}", path.display()))?;
    }
    Ok(())
}

/// Persist a target. The file is named `<target.id>.yaml`.
pub fn save(dir: &Path, target: &Target) -> anyhow::Result<()> {
    let path = dir.join(format!("{}.yaml", target.id));
    let yaml = serde_yaml::to_string(target).context("cannot serialise target")?;
    atomic_write(&path, yaml.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_target() -> Target {
        Target {
            version: 1,
            id: "acme-staging".into(),
            name: "Acme staging chatbot".into(),
            request_ids: vec!["openai-chat".into()],
            request_id: "openai-chat".into(),
            session_config: Default::default(),
            notes: Some("Rate limit 10 req/s".into()),
        }
    }

    #[test]
    fn save_then_load_all() {
        let dir = TempDir::new().unwrap();
        let t = sample_target();
        save(dir.path(), &t).unwrap();

        let map = load_all(dir.path()).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map["acme-staging"], t);
    }

    #[test]
    fn load_all_empty_dir() {
        let dir = TempDir::new().unwrap();
        assert!(load_all(dir.path()).unwrap().is_empty());
    }
}
