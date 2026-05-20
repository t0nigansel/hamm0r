use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::types::TriageEntry;
use crate::write::atomic_write;

/// In-memory representation of a run's triage sidecar file.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TriageFile {
    #[serde(default)]
    pub entries: Vec<TriageEntry>,
}

fn triage_path(engagement_dir: &Path, run_id: &str) -> PathBuf {
    engagement_dir
        .join("runs")
        .join(format!("{run_id}.triage.yaml"))
}

/// Load the triage sidecar for a run. Missing file returns an empty `TriageFile`.
pub fn load(engagement_dir: &Path, run_id: &str) -> anyhow::Result<TriageFile> {
    let path = triage_path(engagement_dir, run_id);
    if !path.exists() {
        return Ok(TriageFile::default());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read triage file: {}", path.display()))?;
    serde_yaml::from_str(&raw)
        .with_context(|| format!("cannot parse triage file: {}", path.display()))
}

/// Upsert one `TriageEntry` into the sidecar. Writes atomically.
pub fn save_entry(engagement_dir: &Path, run_id: &str, entry: TriageEntry) -> anyhow::Result<()> {
    let mut file = load(engagement_dir, run_id)?;
    match file.entries.iter_mut().find(|e| e.seq == entry.seq) {
        Some(existing) => *existing = entry,
        None => file.entries.push(entry),
    }
    file.entries.sort_by_key(|e| e.seq);
    let yaml = serde_yaml::to_string(&file).context("cannot serialize triage")?;
    atomic_write(&triage_path(engagement_dir, run_id), yaml.as_bytes())
}

/// Return all triage entries for a run, sorted by seq. Missing file → empty vec.
pub fn list_entries(engagement_dir: &Path, run_id: &str) -> anyhow::Result<Vec<TriageEntry>> {
    Ok(load(engagement_dir, run_id)?.entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TriageStatus;
    use tempfile::TempDir;

    fn eng(dir: &TempDir) -> std::path::PathBuf {
        let p = dir.path().to_path_buf();
        std::fs::create_dir_all(p.join("runs")).unwrap();
        p
    }

    fn entry(seq: u32, status: TriageStatus) -> TriageEntry {
        TriageEntry {
            seq,
            status,
            note: None,
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn missing_file_returns_empty() {
        let dir = TempDir::new().unwrap();
        let result = list_entries(dir.path(), "run-001").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = TempDir::new().unwrap();
        let eng = eng(&dir);
        let e = TriageEntry {
            seq: 3,
            status: TriageStatus::Confirmed,
            note: Some("looks real".into()),
            updated_at: "2026-05-01T12:00:00Z".into(),
        };
        save_entry(&eng, "run-001", e.clone()).unwrap();
        let loaded = list_entries(&eng, "run-001").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], e);
    }

    #[test]
    fn upsert_replaces_existing_seq() {
        let dir = TempDir::new().unwrap();
        let eng = eng(&dir);
        save_entry(&eng, "run-001", entry(1, TriageStatus::Unreviewed)).unwrap();
        save_entry(
            &eng,
            "run-001",
            TriageEntry {
                seq: 1,
                status: TriageStatus::FalsePositive,
                note: Some("not real".into()),
                updated_at: "2026-05-02T00:00:00Z".into(),
            },
        )
        .unwrap();
        let loaded = list_entries(&eng, "run-001").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].status, TriageStatus::FalsePositive);
        assert_eq!(loaded[0].note.as_deref(), Some("not real"));
    }

    #[test]
    fn multiple_entries_sorted_by_seq() {
        let dir = TempDir::new().unwrap();
        let eng = eng(&dir);
        save_entry(&eng, "run-001", entry(5, TriageStatus::Confirmed)).unwrap();
        save_entry(&eng, "run-001", entry(2, TriageStatus::NeedsReview)).unwrap();
        save_entry(&eng, "run-001", entry(9, TriageStatus::Unreviewed)).unwrap();
        let loaded = list_entries(&eng, "run-001").unwrap();
        assert_eq!(loaded.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2, 5, 9]);
    }

    #[test]
    fn separate_runs_are_independent() {
        let dir = TempDir::new().unwrap();
        let eng = eng(&dir);
        save_entry(&eng, "run-001", entry(1, TriageStatus::Confirmed)).unwrap();
        save_entry(&eng, "run-002", entry(1, TriageStatus::FalsePositive)).unwrap();
        assert_eq!(
            list_entries(&eng, "run-001").unwrap()[0].status,
            TriageStatus::Confirmed
        );
        assert_eq!(
            list_entries(&eng, "run-002").unwrap()[0].status,
            TriageStatus::FalsePositive
        );
    }

    #[test]
    fn all_status_variants_round_trip() {
        let dir = TempDir::new().unwrap();
        let eng = eng(&dir);
        let statuses = [
            TriageStatus::Unreviewed,
            TriageStatus::Confirmed,
            TriageStatus::FalsePositive,
            TriageStatus::NeedsReview,
        ];
        for (i, status) in statuses.iter().enumerate() {
            save_entry(&eng, "run-001", entry(i as u32, status.clone())).unwrap();
        }
        let loaded = list_entries(&eng, "run-001").unwrap();
        assert_eq!(loaded.len(), 4);
        for (i, status) in statuses.iter().enumerate() {
            assert_eq!(&loaded[i].status, status);
        }
    }
}
