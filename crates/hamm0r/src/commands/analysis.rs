use std::path::PathBuf;

use serde::Serialize;
use storage::verdicts::VerdictEntry;
use storage::HammorPaths;
use tauri::{AppHandle, State};

#[cfg(feature = "analyzer")]
use analyzer::pipeline::{self, JudgeOneOptions, JudgeOutcome, JudgeRunOptions, Progress};
#[cfg(feature = "analyzer")]
use tauri::Emitter as _;

#[cfg(feature = "analyzer")]
use super::report_user_relevant_error;
use super::{AnalyzerLoggerState, AppPaths};
use crate::error::CommandError;

// ── DTOs ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct RunVerdictDto {
    pub run_id: String,
    pub result_id: String,
    pub seq: u32,
    pub judge_verdict: String,
    pub judge_confidence: f32,
    pub judge_reason: String,
    pub judge_model_used: String,
    pub judge_evaluated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JudgeResultDto {
    pub status: String,
    pub run_id: String,
    pub result_id: String,
    pub judge_verdict: String,
    pub judge_confidence: f32,
    pub judge_reason: String,
    pub judge_model_used: String,
    pub judge_evaluated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JudgeAllDto {
    pub judged: u32,
    pub skipped_existing: u32,
    pub results: Vec<JudgeResultDto>,
}

#[cfg(feature = "analyzer")]
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisProgressEvent {
    pub run_id: String,
    pub processed: u32,
    pub total: u32,
    pub judged: u32,
    pub skipped_existing: u32,
    pub finished: bool,
    pub error: Option<String>,
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn read_run_verdicts(
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
) -> Result<Vec<RunVerdictDto>, CommandError> {
    logger.0.debug(
        "analysis",
        Some(&run_id),
        &format!("read_run_verdicts invoked for engagement={engagement_slug}"),
    );
    let verdict_path = verdict_path_for_run(&paths.0, &engagement_slug, &run_id);
    if !verdict_path.exists() {
        return Ok(vec![]);
    }

    let latest = storage::verdicts::latest_by_seq(&storage::verdicts::read_all(&verdict_path)?);
    let mut dtos = latest
        .values()
        .map(|v| to_run_verdict_dto(&run_id, v))
        .collect::<Vec<_>>();
    dtos.sort_by_key(|v| v.seq);
    logger.0.debug(
        "analysis",
        Some(&run_id),
        &format!("read_run_verdicts completed count={}", dtos.len()),
    );
    Ok(dtos)
}

#[tauri::command]
pub fn generate_report(
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
) -> Result<String, CommandError> {
    #[cfg(not(feature = "analyzer"))]
    {
        let _ = (&logger, &paths, &engagement_slug, &run_id);
        Err(anyhow::anyhow!(
            "Analyzer not available in this build. Rebuild hamm0r with --features analyzer."
        )
        .into())
    }

    #[cfg(feature = "analyzer")]
    {
        logger.0.info(
            "analysis",
            Some(&run_id),
            &format!("Generating report for engagement={engagement_slug}"),
        );
        let engagement_dir = paths.0.engagement_dir(&engagement_slug);
        let report_path = pipeline::generate_report(&engagement_dir, &run_id)?;
        logger.0.info(
            "analysis",
            Some(&run_id),
            &format!("Report generated at {}", report_path.display()),
        );
        Ok(report_path.to_string_lossy().into_owned())
    }
}

#[tauri::command]
pub fn read_report_html(
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
) -> Result<Option<String>, CommandError> {
    logger.0.debug(
        "analysis",
        Some(&run_id),
        &format!("read_report_html invoked for engagement={engagement_slug}"),
    );
    let report_path = report_path_for(&paths.0, &engagement_slug, &run_id);
    if !report_path.exists() {
        return Ok(None);
    }
    let html = storage::runs::read_body_by_relative_path(
        &paths.0.engagement_dir(&engagement_slug),
        &format!("reports/report-{run_id}.html"),
    )
    .map_err(|err| {
        logger.0.error(
            "analysis",
            Some(&run_id),
            &format!("read_report_html failed: {err}"),
        );
        CommandError::from(err)
    })?;
    logger
        .0
        .debug("analysis", Some(&run_id), "read_report_html completed");
    Ok(html)
}

#[tauri::command]
pub async fn judge_result(
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    result_id: String,
    force: Option<bool>,
) -> Result<JudgeResultDto, CommandError> {
    #[cfg(not(feature = "analyzer"))]
    {
        let _ = (&logger, &paths, &engagement_slug, &result_id, force);
        Err(anyhow::anyhow!(
            "Analyzer not available in this build. Rebuild hamm0r with --features analyzer."
        )
        .into())
    }

    #[cfg(feature = "analyzer")]
    {
        let (run_id, seq) = pipeline::parse_result_id(&result_id)?;
        let force = force.unwrap_or(false);
        logger.0.info(
            "analysis",
            Some(&run_id),
            &format!("judge_result requested for seq={seq} force={force}"),
        );

        let engagement_dir = paths.0.engagement_dir(&engagement_slug);
        let prompts_dir = paths.0.prompts_dir();
        let analyzer_version = env!("CARGO_PKG_VERSION").to_owned();
        let run_id_for_blocking = run_id.clone();

        let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<JudgeOutcome> {
            pipeline::judge_one_heuristic(&JudgeOneOptions {
                engagement_dir: &engagement_dir,
                prompts_dir: &prompts_dir,
                run_id: &run_id_for_blocking,
                seq,
                analyzer_version: &analyzer_version,
                force,
            })
        })
        .await
        .map_err(|e| anyhow::anyhow!("judge task join failure: {e}"))??;

        let status = if outcome.was_judged() { "judged" } else { "skipped" };
        logger.0.info(
            "analysis",
            Some(&run_id),
            &format!("judge_result completed status={status} seq={seq}"),
        );

        Ok(to_judge_result_dto(status, &run_id, outcome.entry()))
    }
}

#[tauri::command]
pub async fn judge_all(
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    result_ids: Vec<String>,
    run_id: Option<String>,
    force: Option<bool>,
) -> Result<JudgeAllDto, CommandError> {
    #[cfg(not(feature = "analyzer"))]
    {
        let _ = (
            &logger,
            &paths,
            &engagement_slug,
            &result_ids,
            &run_id,
            force,
        );
        Err(anyhow::anyhow!(
            "Analyzer not available in this build. Rebuild hamm0r with --features analyzer."
        )
        .into())
    }

    #[cfg(feature = "analyzer")]
    {
        let force = force.unwrap_or(false);
        logger.0.info(
            "analysis",
            run_id.as_deref(),
            &format!(
                "judge_all requested for engagement={} explicit_results={} force={force}",
                engagement_slug,
                result_ids.len()
            ),
        );

        let engagement_dir = paths.0.engagement_dir(&engagement_slug);
        let prompts_dir = paths.0.prompts_dir();
        let analyzer_version = env!("CARGO_PKG_VERSION").to_owned();

        // Resolve target (run_id, seq) pairs synchronously up front so the
        // blocking task only does the judging work.
        let mut targets: Vec<(String, u32)> = if result_ids.is_empty() {
            let rid = run_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("run_id is required when result_ids is empty"))?;
            let run_path = pipeline::run_path_for(&engagement_dir, rid);
            let attempts = pipeline::load_attempts(&run_path)?;
            attempts.into_iter().map(|a| (rid.to_owned(), a.seq)).collect()
        } else {
            result_ids
                .iter()
                .map(|id| pipeline::parse_result_id(id))
                .collect::<Result<Vec<_>, _>>()?
        };

        if let Some(filter) = run_id.as_deref() {
            targets.retain(|(rid, _)| rid == filter);
        }
        targets.sort();
        targets.dedup();

        let dto = tokio::task::spawn_blocking(move || -> anyhow::Result<JudgeAllDto> {
            let mut judged = 0u32;
            let mut skipped_existing = 0u32;
            let mut results = Vec::new();

            for (rid, seq) in targets {
                let outcome = pipeline::judge_one_heuristic(&JudgeOneOptions {
                    engagement_dir: &engagement_dir,
                    prompts_dir: &prompts_dir,
                    run_id: &rid,
                    seq,
                    analyzer_version: &analyzer_version,
                    force,
                })?;
                let status = if outcome.was_judged() {
                    judged += 1;
                    "judged"
                } else {
                    skipped_existing += 1;
                    "skipped"
                };
                results.push(to_judge_result_dto(status, &rid, outcome.entry()));
            }

            Ok(JudgeAllDto {
                judged,
                skipped_existing,
                results,
            })
        })
        .await
        .map_err(|e| anyhow::anyhow!("judge_all task join failure: {e}"))??;

        logger.0.info(
            "analysis",
            run_id.as_deref(),
            &format!(
                "judge_all completed judged={} skipped_existing={}",
                dto.judged, dto.skipped_existing
            ),
        );

        Ok(dto)
    }
}

#[tauri::command]
pub async fn start_analysis(
    app: AppHandle,
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
    engagement_slug: String,
    run_id: String,
    force: Option<bool>,
) -> Result<String, CommandError> {
    #[cfg(not(feature = "analyzer"))]
    {
        let _ = (&app, &logger, &paths, &engagement_slug, &run_id, force);
        Err(anyhow::anyhow!(
            "Analyzer not available in this build. Rebuild hamm0r with --features analyzer."
        )
        .into())
    }

    #[cfg(feature = "analyzer")]
    {
        let paths = paths.0.clone();
        let run_id_ret = run_id.clone();
        let force = force.unwrap_or(false);
        let logger = logger.0.clone();

        tokio::spawn(async move {
            logger.info(
                "analysis",
                Some(&run_id),
                &format!("Analysis task spawned for engagement={engagement_slug} force={force}"),
            );
            if let Err(err) = analyze_run_and_emit(
                app.clone(),
                paths.clone(),
                engagement_slug.clone(),
                run_id.clone(),
                force,
            )
            .await
            {
                let message = format!("analysis execution failed: {err}");
                report_user_relevant_error(
                    &app,
                    &logger,
                    "analysis",
                    "analysis-execution",
                    Some(&run_id),
                    &message,
                );
                let _ = app.emit(
                    "analysis-progress",
                    AnalysisProgressEvent {
                        run_id: run_id.clone(),
                        processed: 0,
                        total: 0,
                        judged: 0,
                        skipped_existing: 0,
                        finished: true,
                        error: Some(message),
                    },
                );
            } else {
                logger.info("analysis", Some(&run_id), "Analysis completed");
            }
        });

        Ok(run_id_ret)
    }
}

// ── Analyzer-only implementation ──────────────────────────────────────────────

#[cfg(feature = "analyzer")]
async fn analyze_run_and_emit(
    app: AppHandle,
    paths: HammorPaths,
    engagement_slug: String,
    run_id: String,
    force: bool,
) -> anyhow::Result<()> {
    let engagement_dir = paths.engagement_dir(&engagement_slug);
    let prompts_dir = paths.prompts_dir();
    let model_path = pipeline::find_model_file(&paths.analyzer_models_dir());
    let analyzer_version = env!("CARGO_PKG_VERSION").to_owned();
    let run_id_for_blocking = run_id.clone();
    let run_id_for_progress = run_id.clone();
    let app_for_progress = app.clone();

    let summary = tokio::task::spawn_blocking(move || -> anyhow::Result<pipeline::JudgeRunSummary> {
        let mut on_progress = |p: Progress| {
            let _ = app_for_progress.emit(
                "analysis-progress",
                AnalysisProgressEvent {
                    run_id: run_id_for_progress.clone(),
                    processed: p.processed,
                    total: p.total,
                    judged: p.judged,
                    skipped_existing: p.skipped_existing,
                    finished: false,
                    error: None,
                },
            );
        };
        pipeline::judge_run(
            &JudgeRunOptions {
                engagement_dir: &engagement_dir,
                prompts_dir: &prompts_dir,
                run_id: &run_id_for_blocking,
                model_path: model_path.as_deref(),
                analyzer_version: &analyzer_version,
                force,
            },
            &mut on_progress,
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("analysis task join failure: {e}"))??;

    // Generate the report once judging finished.
    let _report = pipeline::generate_report(&paths.engagement_dir(&engagement_slug), &run_id)?;

    let _ = app.emit(
        "analysis-progress",
        AnalysisProgressEvent {
            run_id,
            processed: summary.processed,
            total: summary.total,
            judged: summary.judged,
            skipped_existing: summary.skipped_existing,
            finished: true,
            error: None,
        },
    );

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn verdict_path_for_run(paths: &HammorPaths, engagement_slug: &str, run_id: &str) -> PathBuf {
    paths
        .engagement_dir(engagement_slug)
        .join("runs")
        .join(format!("{run_id}.verdicts.jsonl"))
}

fn report_path_for(paths: &HammorPaths, engagement_slug: &str, run_id: &str) -> PathBuf {
    paths
        .engagement_dir(engagement_slug)
        .join("reports")
        .join(format!("report-{run_id}.html"))
}

fn verdict_label(verdict: &storage::verdicts::JudgeVerdict) -> String {
    use storage::verdicts::JudgeVerdict::*;
    match verdict {
        Success => "SUCCESS",
        Fail => "FAIL",
        Partial => "PARTIAL",
        Unclear => "UNCLEAR",
    }
    .to_owned()
}

fn to_run_verdict_dto(run_id: &str, verdict: &VerdictEntry) -> RunVerdictDto {
    RunVerdictDto {
        run_id: run_id.to_owned(),
        result_id: format!("{run_id}-{}", verdict.seq),
        seq: verdict.seq,
        judge_verdict: verdict_label(&verdict.verdict),
        judge_confidence: verdict.confidence,
        judge_reason: verdict.rationale.clone(),
        judge_model_used: verdict.model_used.clone(),
        judge_evaluated_at: verdict.evaluated_at.clone(),
    }
}

#[cfg(feature = "analyzer")]
fn to_judge_result_dto(status: &str, run_id: &str, verdict: &VerdictEntry) -> JudgeResultDto {
    JudgeResultDto {
        status: status.to_owned(),
        run_id: run_id.to_owned(),
        result_id: format!("{run_id}-{}", verdict.seq),
        judge_verdict: verdict_label(&verdict.verdict),
        judge_confidence: verdict.confidence,
        judge_reason: verdict.rationale.clone(),
        judge_model_used: verdict.model_used.clone(),
        judge_evaluated_at: verdict.evaluated_at.clone(),
    }
}
