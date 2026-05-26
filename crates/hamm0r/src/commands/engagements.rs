use serde::Serialize;
use storage::engagements;
use storage::metrics::{compute_asv, AsvReport, RunInputs};
use storage::prompts;
use storage::runs::{read_all, RunRecord, RunStatus};
use storage::types::{EngagementMeta, EngagementScope, EngagementTarget};
use storage::verdicts;
use tauri::State;

use super::{ActiveRunsState, AppPaths};
use crate::error::CommandError;

#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub id: String,
    pub status: String,
    pub completed: u32,
    pub total_prompts: u32,
    pub errors: u32,
    pub started_at: String,
    /// Scenario this run came from. `None` for ad-hoc runs (rerun path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario_id: Option<String>,
    /// True when a sibling `<run_id>.verdicts.jsonl` exists on disk.
    /// Drives the "this run has been analyzed" indicator in the runs list.
    pub analyzed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportFileDto {
    pub path: String,
}

#[tauri::command]
pub fn list_engagements(paths: State<'_, AppPaths>) -> Result<Vec<EngagementMeta>, CommandError> {
    engagements::list(&paths.0.engagements_dir()).map_err(Into::into)
}

#[tauri::command]
pub fn get_engagement(
    paths: State<'_, AppPaths>,
    slug: String,
) -> Result<Option<EngagementMeta>, CommandError> {
    engagements::load(&paths.0.engagements_dir(), &slug).map_err(Into::into)
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
            scenario_id: String::new(),
            notes: None,
        },
        scope: EngagementScope {
            prompt_files: vec![],
        },
    };
    engagements::create(&paths.0.engagements_dir(), &meta)?;
    Ok(meta)
}

/// Bind a Scenario to an engagement without starting a run. The binding
/// is the same field that `start_scenario_run` writes; setting it up-front
/// lets the user create an engagement and pre-select what it should run.
/// Passing an empty `scenario_id` clears the binding.
#[tauri::command]
pub fn set_engagement_scenario(
    paths: State<'_, AppPaths>,
    slug: String,
    scenario_id: String,
) -> Result<EngagementMeta, CommandError> {
    let engagements_dir = paths.0.engagements_dir();
    let mut meta = engagements::load(&engagements_dir, &slug)?
        .ok_or_else(|| anyhow::anyhow!("Engagement '{slug}' not found"))?;
    meta.target.scenario_id = scenario_id;
    engagements::save_meta(&engagements_dir, &meta)?;
    Ok(meta)
}

/// Rename an existing engagement. Only the display name in
/// `engagement.yaml` is updated; the slug (folder name) stays the same so
/// run paths, response files, and reports remain valid.
#[tauri::command]
pub fn rename_engagement(
    paths: State<'_, AppPaths>,
    slug: String,
    name: String,
) -> Result<EngagementMeta, CommandError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("Engagement name must not be empty").into());
    }
    let engagements_dir = paths.0.engagements_dir();
    let mut meta = engagements::load(&engagements_dir, &slug)?
        .ok_or_else(|| anyhow::anyhow!("Engagement '{slug}' not found"))?;
    meta.name = trimmed.to_owned();
    engagements::save_meta(&engagements_dir, &meta)?;
    Ok(meta)
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteEngagementResult {
    pub deleted: bool,
}

/// Permanently remove an engagement folder (runs, verdicts, responses,
/// reports, metadata). Refuses if any run inside the engagement is still
/// active — the user must stop it first. Idempotent: deleting an
/// already-gone slug returns `deleted: false` without error.
#[tauri::command]
pub fn delete_engagement(
    paths: State<'_, AppPaths>,
    active_runs: State<'_, ActiveRunsState>,
    slug: String,
) -> Result<DeleteEngagementResult, CommandError> {
    // Refuse while a run inside this engagement is active. We don't
    // track engagement-slug → run-id, so the safe play is to refuse if
    // ANY active run sits in the matching engagement folder.
    let active = active_runs
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("active run registry poisoned"))?;
    if !active.is_empty() {
        let runs_dir = paths.0.engagement_dir(&slug).join("runs");
        for run_id in active.keys() {
            let run_path = runs_dir.join(format!("{run_id}.jsonl"));
            if run_path.exists() {
                return Err(anyhow::anyhow!(
                    "Run '{run_id}' is still active in this engagement. Stop it first, then delete."
                )
                .into());
            }
        }
    }
    drop(active);

    let removed = engagements::delete(&paths.0.engagements_dir(), &slug)?;
    Ok(DeleteEngagementResult { deleted: removed })
}

#[tauri::command]
pub fn list_runs(
    active_runs: State<'_, ActiveRunsState>,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
) -> Result<Vec<RunSummary>, CommandError> {
    let runs_dir = paths.0.engagement_dir(&engagement_slug).join("runs");
    if !runs_dir.exists() {
        return Ok(vec![]);
    }

    let mut summaries = vec![];
    let active_run_ids = active_runs
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("active run registry poisoned"))?
        .keys()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
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
        // Replay run files live in the same dir but are not listed at the
        // top level — they're surfaced inline next to the original attempt.
        if run_id.contains("-replay-") {
            continue;
        }
        let records = read_all(&path)?;
        let verdicts_path = path.with_extension("verdicts.jsonl");
        summaries.push(summarize_run(
            &run_id,
            &records,
            active_run_ids.contains(&run_id),
            verdicts_path.exists(),
        ));
    }

    summaries.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(summaries)
}

#[tauri::command]
pub fn get_run_progress(
    active_runs: State<'_, ActiveRunsState>,
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
    let is_active = active_runs
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("active run registry poisoned"))?
        .contains_key(&run_id);
    let verdicts_path = run_path.with_extension("verdicts.jsonl");
    Ok(Some(summarize_run(
        &run_id,
        &records,
        is_active,
        verdicts_path.exists(),
    )))
}

#[tauri::command]
pub fn save_markdown_export(
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
    markdown: String,
) -> Result<ExportFileDto, CommandError> {
    let path = storage::reports::write_markdown_report(
        &paths.0.engagement_dir(&engagement_slug),
        &run_id,
        &markdown,
    )?;
    Ok(ExportFileDto {
        path: path.to_string_lossy().into_owned(),
    })
}

/// Attack Success Value report aggregated across every run JSONL +
/// verdict JSONL in the engagement folder. See
/// `docs/TODO-opi-integration.md` (Milestone B) for the metric
/// definition. Returns an empty report (all zeroes) when the
/// engagement has no runs or no verdicts yet.
#[tauri::command]
pub fn get_asv_report(
    paths: State<'_, AppPaths>,
    engagement_slug: String,
) -> Result<AsvReport, CommandError> {
    let runs_dir = paths.0.engagement_dir(&engagement_slug).join("runs");
    if !runs_dir.exists() {
        return Ok(AsvReport::default());
    }

    let library = prompts::load_all(&paths.0.prompts_dir())?;
    let mut library_by_id = std::collections::HashMap::new();
    for entries in library.values() {
        for entry in entries {
            library_by_id.insert(entry.id.clone(), entry.clone());
        }
    }

    // Read every `<run_id>.jsonl` + matching `<run_id>.verdicts.jsonl`.
    // Replays live in the same directory but represent ad-hoc reruns —
    // they're surfaced inline in the UI rather than as standalone runs,
    // so we exclude them here to keep the rollup focused on the
    // engagement's primary attempts.
    let mut run_files: Vec<(Vec<RunRecord>, Vec<verdicts::VerdictRecord>)> = Vec::new();
    for entry in std::fs::read_dir(&runs_dir).map_err(|e| anyhow::anyhow!(e))? {
        let entry = entry.map_err(|e| anyhow::anyhow!(e))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".jsonl") || name.ends_with(".verdicts.jsonl") {
            continue;
        }
        if name.contains("-replay-") {
            continue;
        }
        let run_records = read_all(&path)?;
        let verdict_path = path.with_extension("verdicts.jsonl");
        let verdict_records = if verdict_path.exists() {
            verdicts::read_all(&verdict_path)?
        } else {
            Vec::new()
        };
        run_files.push((run_records, verdict_records));
    }

    let inputs: Vec<RunInputs<'_>> = run_files
        .iter()
        .map(|(r, v)| RunInputs {
            run_records: r,
            verdict_records: v,
        })
        .collect();
    Ok(compute_asv(&inputs, &library_by_id))
}

#[tauri::command]
pub fn open_export_path(path: String) -> Result<(), CommandError> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path])
            .spawn()
            .map_err(anyhow::Error::from)?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(anyhow::Error::from)?;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(anyhow::Error::from)?;
    }

    Ok(())
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

fn summarize_run(
    run_id: &str,
    records: &[RunRecord],
    is_active: bool,
    analyzed: bool,
) -> RunSummary {
    let mut started_at = String::new();
    let mut completed = 0u32;
    let mut total_prompts = 0u32;
    let mut errors = 0u32;
    let mut scenario_id: Option<String> = None;
    let mut status = if is_active {
        "running".to_owned()
    } else {
        "aborted".to_owned()
    };

    for record in records {
        match record {
            RunRecord::Header(h) => {
                started_at = h.started_at.clone();
                scenario_id = h.scenario_id.clone();
            }
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
            RunRecord::LeakDetected(_) => {}
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
        scenario_id,
        analyzed,
    }
}
