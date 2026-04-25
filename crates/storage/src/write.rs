use std::fs;
use std::io::Write as _;
use std::path::Path;

use anyhow::Context as _;
use tempfile::NamedTempFile;

/// Write `contents` to `dest` atomically: write to a temp file in the same
/// directory, then rename into place. A rename on the same filesystem is
/// atomic on POSIX (and best-effort on Windows). Callers never see a partial
/// write at `dest`.
pub fn atomic_write(dest: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let dir = dest.parent().with_context(|| {
        format!("destination path has no parent: {}", dest.display())
    })?;

    fs::create_dir_all(dir)
        .with_context(|| format!("could not create directory: {}", dir.display()))?;

    // Create the temp file in the same directory so the rename stays on the
    // same filesystem (cross-device rename is not atomic).
    let mut tmp = NamedTempFile::new_in(dir)
        .with_context(|| format!("could not create temp file in {}", dir.display()))?;

    tmp.write_all(contents)
        .context("could not write to temp file")?;

    tmp.flush().context("could not flush temp file")?;

    tmp.persist(dest)
        .map(|_| ())
        .with_context(|| format!("could not rename temp file to {}", dest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn roundtrip_bytes() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("out.txt");
        atomic_write(&dest, b"hello world").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"hello world");
    }

    #[test]
    fn creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("a").join("b").join("c.txt");
        atomic_write(&dest, b"nested").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"nested");
    }

    #[test]
    fn overwrites_existing_file() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("file.txt");
        atomic_write(&dest, b"first").unwrap();
        atomic_write(&dest, b"second").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"second");
    }
}
