use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::write::atomic_write;

/// Write a Markdown export for a run into the engagement's reports directory.
///
/// The filename is `report-<run>.md` so exported artifacts sit next to the
/// analyzer's HTML reports while staying clearly separated by extension.
pub fn write_markdown_report(
    engagement_dir: &Path,
    run_id: &str,
    markdown: &str,
) -> anyhow::Result<PathBuf> {
    let path = engagement_dir
        .join("reports")
        .join(format!("report-{run_id}.md"));
    atomic_write(&path, markdown.as_bytes())
        .with_context(|| format!("cannot write markdown report: {}", path.display()))?;
    Ok(path)
}
