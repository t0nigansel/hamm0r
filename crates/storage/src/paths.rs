use directories::UserDirs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathError {
    #[error("could not determine home directory")]
    NoHomeDir,
}

/// Resolved paths for the hamm0r data directory tree.
///
/// Default root is `~/hamm0r/`. Everything else is derived from it.
/// Construct once at startup and pass by reference — do not call
/// `UserDirs::new()` repeatedly.
#[derive(Debug, Clone)]
pub struct HammorPaths {
    root: PathBuf,
}

impl HammorPaths {
    /// Create using the default `~/hamm0r/` root.
    pub fn new() -> Result<Self, PathError> {
        let dirs = UserDirs::new().ok_or(PathError::NoHomeDir)?;
        let root = dirs.home_dir().join("hamm0r");
        Ok(Self { root })
    }

    /// Create with an explicit root — used in tests.
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn prompts_dir(&self) -> PathBuf {
        self.root.join("prompts")
    }

    pub fn requests_dir(&self) -> PathBuf {
        self.root.join("requests")
    }

    pub fn targets_dir(&self) -> PathBuf {
        self.root.join("targets")
    }

    pub fn scenarios_dir(&self) -> PathBuf {
        self.root.join("scenarios")
    }

    pub fn engagements_dir(&self) -> PathBuf {
        self.root.join("engagements")
    }

    pub fn engagement_dir(&self, slug: &str) -> PathBuf {
        self.engagements_dir().join(slug)
    }

    pub fn analyzer_dir(&self) -> PathBuf {
        self.root.join("analyzer")
    }
}
