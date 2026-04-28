use serde::Serialize;
use storage::runs::{read_all, RunRecord, RunStatus};
use storage::types::{EngagementMeta, EngagementScope, EngagementTarget};
use storage::engagements;
use tauri::State;

use super::AppPaths;
use crate::error::CommandError;

#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub id: String,
    pub status: String,
    pub completed: u32,
    pub total_prompts: u32,
    pub errors: u32,
    pub started_at: String,
}

#[tauri::command]
pub fn list_engagements(paths: State<'_, AppPaths>) -> Result<Vec<EngagementMeta>, CommandError> {
    engagements::list(&paths.0.engagements_dir()).map_err(Into::into)
}

/// Create a new engagement directory tree and return its metadata.
/// The slug is generated as `<YYYY-MM-DD>-<slugified-name>`.
#[tauri::command]
pub fn create_engagement(
    paths: State<'_, AppPaths>,
    name: String,
) -> Result<EngagementMeta, CommandError> {
    let slug = make_slug(&name);
    let meta = EngagementMeta {
        version: 1,
        slug: slug.clone(),
        name: name.clone(),
        created_at: runner::run::iso_now(),
        target: EngagementTarget {
            request_id: String::new(),
            notes: None,
        },
        scope: EngagementScope {
            prompt_files: vec![],
        },
    };
    engagements::create(&paths.0.engagements_dir(), &meta)?;
    Ok(meta)
}

#[tauri::command]
pub fn list_runs(
    paths: State<'_, AppPaths>,
    engagement_slug: String,
) -> Result<Vec<RunSummary>, CommandError> {
    let runs_dir = paths.0.engagement_dir(&engagement_slug).join("runs");
    if !runs_dir.exists() {
        return Ok(vec![]);
    }

    let mut summaries = vec![];
    for entry in std::fs::read_dir(&runs_dir).map_err(|e| anyhow::anyhow!(e))? {
        let entry = entry.map_err(|e| anyhow::anyhow!(e))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".verdicts.jsonl"))
        {
            continue;
        }
        let run_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        if run_id.is_empty() {
            continue;
        }
        let records = read_all(&path)?;
        summaries.push(summarize_run(&run_id, &records));
    }

    summaries.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(summaries)
}

#[tauri::command]
pub fn get_run_progress(
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
) -> Result<Option<RunSummary>, CommandError> {
    let run_path = paths
        .0
        .engagement_dir(&engagement_slug)
        .join("runs")
        .join(format!("{run_id}.jsonl"));

    if !run_path.exists() {
        return Ok(None);
    }

    let records = read_all(&run_path)?;
    Ok(Some(summarize_run(&run_id, &records)))
}

fn make_slug(name: &str) -> String {
    let today = &runner::run::iso_now()[..10]; // "YYYY-MM-DD"
    let slug_part: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("{today}-{slug_part}")
}

fn summarize_run(run_id: &str, records: &[RunRecord]) -> RunSummary {
    let mut started_at = String::new();
    let mut completed = 0u32;
    let mut total_prompts = 0u32;
    let mut errors = 0u32;
    let mut status = "running".to_owned();

    for record in records {
        match record {
            RunRecord::Header(h) => started_at = h.started_at.clone(),
            RunRecord::Attempt(a) => {
                completed += 1;
                if a.response.status == 0 || a.response.error.is_some() {
                    errors += 1;
                }
            }
            RunRecord::Footer(f) => {
                total_prompts = f.attempts_total;
                errors = f.attempts_failed;
                status = match f.status {
                    RunStatus::Completed => "completed",
                    RunStatus::AbortedByUser => "aborted",
                    RunStatus::Crashed => "crashed",
                }
                .to_owned();
            }
        }
    }

    if total_prompts == 0 {
        total_prompts = completed;
    }

    RunSummary {
        id: run_id.to_owned(),
        status,
        completed,
        total_prompts,
        errors,
        started_at,
    }
}
