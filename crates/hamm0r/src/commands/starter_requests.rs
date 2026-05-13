use std::path::Path;

const BUNDLED: &[(&str, &str)] = &[(
    "ollama-chat-local.yaml",
    include_str!("../../../../requests/ollama-chat-local.yaml"),
)];

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
