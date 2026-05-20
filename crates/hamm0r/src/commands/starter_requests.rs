use std::path::Path;

const BUNDLED: &[(&str, &str)] = &[
    (
        "ollama-chat-local.yaml",
        include_str!("../../../../requests/ollama-chat-local.yaml"),
    ),
    (
        "openai-chat-completions.yaml",
        include_str!("../../../../requests/openai-chat-completions.yaml"),
    ),
    (
        "anthropic-messages.yaml",
        include_str!("../../../../requests/anthropic-messages.yaml"),
    ),
    (
        "azure-openai-chat-completions.yaml",
        include_str!("../../../../requests/azure-openai-chat-completions.yaml"),
    ),
    (
        "generic-rest-json.yaml",
        include_str!("../../../../requests/generic-rest-json.yaml"),
    ),
];

/// Write bundled request YAMLs into `dir`, skipping files that already exist.
///
/// This mirrors prompt-library seeding semantics: user-edited files are never
/// overwritten, but newly introduced starter requests appear automatically on
/// the next startup.
pub fn seed_on_startup(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (filename, contents) in BUNDLED {
        let dest = dir.join(filename);
        if dest.exists() {
            continue;
        }
        storage::atomic_write(&dest, contents.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::seed_on_startup;
    use tempfile::TempDir;

    #[test]
    fn seed_on_startup_writes_missing_bundled_requests() {
        let dir = TempDir::new().unwrap();
        seed_on_startup(dir.path()).unwrap();

        let seeded = dir.path().join("ollama-chat-local.yaml");
        assert!(seeded.exists());
        assert!(dir.path().join("openai-chat-completions.yaml").exists());
        assert!(dir.path().join("anthropic-messages.yaml").exists());
        assert!(dir
            .path()
            .join("azure-openai-chat-completions.yaml")
            .exists());
        assert!(dir.path().join("generic-rest-json.yaml").exists());
    }

    #[test]
    fn seed_on_startup_does_not_overwrite_existing_request() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ollama-chat-local.yaml");
        storage::atomic_write(&path, b"custom: true\n").unwrap();

        seed_on_startup(dir.path()).unwrap();

        let raw = std::fs::read_to_string(path).unwrap();
        assert_eq!(raw, "custom: true\n");
    }
}
