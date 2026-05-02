use std::path::{Path, PathBuf};

use anyhow::Context as _;

const MAX_LOG_FILES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationMode {
    Startup,
    SizeLimit,
}

pub fn ensure_component_dir(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("cannot create log directory: {}", dir.display()))
}

pub fn rotate_logs_on_startup(dir: &Path, base_name: &str) -> anyhow::Result<()> {
    rotate(dir, base_name, RotationMode::Startup)
}

pub fn rotate_logs_for_size(dir: &Path, base_name: &str, max_bytes: u64) -> anyhow::Result<bool> {
    let active = active_log_path(dir, base_name);
    let size = std::fs::metadata(&active).map(|m| m.len()).unwrap_or(0);
    if size <= max_bytes {
        return Ok(false);
    }

    rotate(dir, base_name, RotationMode::SizeLimit)?;
    Ok(true)
}

pub fn append_text(dir: &Path, base_name: &str, text: &str) -> anyhow::Result<()> {
    let path = active_log_path(dir, base_name);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("cannot open log file: {}", path.display()))?;
    use std::io::Write as _;
    file.write_all(text.as_bytes())
        .with_context(|| format!("cannot append log file: {}", path.display()))
}

pub fn active_log_path(dir: &Path, base_name: &str) -> PathBuf {
    dir.join(base_name)
}

fn rotate(dir: &Path, base_name: &str, _mode: RotationMode) -> anyhow::Result<()> {
    ensure_component_dir(dir)?;

    let active = active_log_path(dir, base_name);
    if !active.exists() {
        prune_old_logs(dir, base_name)?;
        return Ok(());
    }

    for idx in (1..MAX_LOG_FILES).rev() {
        let src = rotated_log_path(dir, base_name, idx);
        if src.exists() {
            if idx == MAX_LOG_FILES - 1 {
                std::fs::remove_file(&src).ok();
            } else {
                let dst = rotated_log_path(dir, base_name, idx + 1);
                std::fs::rename(&src, &dst).with_context(|| {
                    format!("cannot rotate log {} -> {}", src.display(), dst.display())
                })?;
            }
        }
    }

    let first_rotated = rotated_log_path(dir, base_name, 1);
    std::fs::rename(&active, &first_rotated).with_context(|| {
        format!(
            "cannot rotate active log {} -> {}",
            active.display(),
            first_rotated.display()
        )
    })?;

    prune_old_logs(dir, base_name)
}

fn rotated_log_path(dir: &Path, base_name: &str, idx: usize) -> PathBuf {
    let (stem, ext) = match base_name.rsplit_once('.') {
        Some((stem, ext)) => (stem.to_owned(), format!(".{ext}")),
        None => (base_name.to_owned(), String::new()),
    };
    dir.join(format!("{stem}.{idx}{ext}"))
}

fn prune_old_logs(dir: &Path, base_name: &str) -> anyhow::Result<()> {
    for idx in MAX_LOG_FILES..(MAX_LOG_FILES + 16) {
        let path = rotated_log_path(dir, base_name, idx);
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("cannot prune old log: {}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn startup_rotation_keeps_active_and_four_archives() {
        let dir = TempDir::new().unwrap();
        ensure_component_dir(dir.path()).unwrap();

        for idx in 1..=6 {
            let active = active_log_path(dir.path(), "hamm0r.log");
            std::fs::write(&active, format!("run-{idx}")).unwrap();
            rotate_logs_on_startup(dir.path(), "hamm0r.log").unwrap();
        }

        assert!(!active_log_path(dir.path(), "hamm0r.log").exists());
        assert!(rotated_log_path(dir.path(), "hamm0r.log", 1).exists());
        assert!(rotated_log_path(dir.path(), "hamm0r.log", 4).exists());
        assert!(!rotated_log_path(dir.path(), "hamm0r.log", 5).exists());
    }

    #[test]
    fn size_rotation_triggers_when_limit_exceeded() {
        let dir = TempDir::new().unwrap();
        ensure_component_dir(dir.path()).unwrap();

        append_text(dir.path(), "hamm0r.log", "abcdef").unwrap();
        let rotated = rotate_logs_for_size(dir.path(), "hamm0r.log", 3).unwrap();

        assert!(rotated);
        assert!(rotated_log_path(dir.path(), "hamm0r.log", 1).exists());
        assert!(!active_log_path(dir.path(), "hamm0r.log").exists());
    }
}
