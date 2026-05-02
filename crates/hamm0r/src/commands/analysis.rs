use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use storage::verdicts::{self, JudgeVerdict, VerdictEntry};
use storage::HammorPaths;
use tauri::{AppHandle, State};

#[cfg(feature = "analyzer")]
use analyzer::report::{build_report_data, render_html_report, ReportAttempt, ReportBuildInput};
#[cfg(feature = "analyzer")]
use analyzer::JudgeInput;
#[cfg(feature = "analyzer")]
use storage::runs::{read_all, RunRecord};
#[cfg(feature = "analyzer")]
use storage::verdicts::{VerdictHeader, VerdictRunStatus};
#[cfg(feature = "analyzer")]
use storage::{atomic_write, verdicts::VerdictRecord};
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

    let latest = read_latest_verdicts(&verdict_path)?;
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
        return Err(anyhow::anyhow!(
            "Analyzer not available in this build. Rebuild hamm0r with --features analyzer."
        )
        .into());
    }

    #[cfg(feature = "analyzer")]
    {
        logger.0.info(
            "analysis",
            Some(&run_id),
            &format!("Generating report for engagement={engagement_slug}"),
        );
        let report_path = generate_report_inner(&paths.0, &engagement_slug, &run_id)?;
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
        return Err(anyhow::anyhow!(
            "Analyzer not available in this build. Rebuild hamm0r with --features analyzer."
        )
        .into());
    }

    #[cfg(feature = "analyzer")]
    {
        let (run_id, seq) = parse_result_id(&result_id)?;
        let force = force.unwrap_or(false);
        logger.0.info(
            "analysis",
            Some(&run_id),
            &format!("judge_result requested for seq={seq} force={force}"),
        );
        let verdict_path = verdict_path_for_run(&paths.0, &engagement_slug, &run_id);
        let latest = read_latest_verdicts(&verdict_path)?;

        if let Some(existing) = latest.get(&seq) {
            if !force {
                return Ok(to_judge_result_dto("skipped", &run_id, existing));
            }
        }

        let run_path = run_path_for(&paths.0, &engagement_slug, &run_id);
        let attempt = load_attempt(&run_path, seq)?;
        let prompt_index = build_prompt_index(&paths.0)?;
        let prompt_meta = prompt_index.get(&attempt.prompt_id);
        let engagement_dir = paths.0.engagement_dir(&engagement_slug);
        let verdict = evaluate_attempt(&engagement_dir, &attempt, prompt_meta).await?;

        ensure_verdict_header(&verdict_path, &run_id, &verdict.model_used)?;
        verdicts::append(
            &verdict_path,
            &VerdictRecord::Verdict(Box::new(verdict.clone())),
        )?;
        logger.0.info(
            "analysis",
            Some(&run_id),
            &format!("judge_result completed for seq={seq}"),
        );

        Ok(to_judge_result_dto("judged", &run_id, &verdict))
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
        return Err(anyhow::anyhow!(
            "Analyzer not available in this build. Rebuild hamm0r with --features analyzer."
        )
        .into());
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
        let prompt_index = build_prompt_index(&paths.0)?;

        let mut targets: Vec<(String, u32)> = if result_ids.is_empty() {
            let run_id = run_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("run_id is required when result_ids is empty"))?;
            let run_path = run_path_for(&paths.0, &engagement_slug, run_id);
            let attempts = load_attempts(&run_path)?;
            attempts
                .into_iter()
                .map(|a| (run_id.to_owned(), a.seq))
                .collect()
        } else {
            result_ids
                .iter()
                .map(|id| parse_result_id(id))
                .collect::<Result<Vec<_>, _>>()?
        };

        if let Some(run_filter) = run_id.as_deref() {
            targets.retain(|(rid, _)| rid == run_filter);
        }

        targets.sort();
        targets.dedup();

        let mut judged = 0u32;
        let mut skipped_existing = 0u32;
        let mut results = Vec::new();
        let mut latest_cache: HashMap<String, HashMap<u32, VerdictEntry>> = HashMap::new();
        let mut attempt_cache: HashMap<String, HashMap<u32, storage::runs::RunAttempt>> =
            HashMap::new();
        let engagement_dir = paths.0.engagement_dir(&engagement_slug);

        for (rid, seq) in targets {
            let verdict_path = verdict_path_for_run(&paths.0, &engagement_slug, &rid);
            if !latest_cache.contains_key(&rid) {
                latest_cache.insert(rid.clone(), read_latest_verdicts(&verdict_path)?);
            }

            if let Some(existing) = latest_cache
                .get(&rid)
                .and_then(|by_seq| by_seq.get(&seq))
                .cloned()
            {
                if !force {
                    skipped_existing += 1;
                    results.push(to_judge_result_dto("skipped", &rid, &existing));
                    continue;
                }
            }

            if !attempt_cache.contains_key(&rid) {
                let run_path = run_path_for(&paths.0, &engagement_slug, &rid);
                attempt_cache.insert(rid.clone(), load_attempt_map(&run_path)?);
            }
            let attempt = attempt_cache
                .get(&rid)
                .and_then(|m| m.get(&seq))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("attempt {seq} not found in {rid}"))?;

            let prompt_meta = prompt_index.get(&attempt.prompt_id);
            let verdict = evaluate_attempt(&engagement_dir, &attempt, prompt_meta).await?;
            ensure_verdict_header(&verdict_path, &rid, &verdict.model_used)?;
            verdicts::append(
                &verdict_path,
                &VerdictRecord::Verdict(Box::new(verdict.clone())),
            )?;

            judged += 1;
            if let Some(cache) = latest_cache.get_mut(&rid) {
                cache.insert(seq, verdict.clone());
            }
            results.push(to_judge_result_dto("judged", &rid, &verdict));
        }

        logger.0.info(
            "analysis",
            run_id.as_deref(),
            &format!("judge_all completed judged={judged} skipped_existing={skipped_existing}"),
        );

        Ok(JudgeAllDto {
            judged,
            skipped_existing,
            results,
        })
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
        return Err(anyhow::anyhow!(
            "Analyzer not available in this build. Rebuild hamm0r with --features analyzer."
        )
        .into());
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
    let run_path = run_path_for(&paths, &engagement_slug, &run_id);
    let attempts = load_attempts(&run_path)?;
    let total = attempts.len() as u32;
    let prompt_index = build_prompt_index(&paths)?;
    let verdict_path = verdict_path_for_run(&paths, &engagement_slug, &run_id);
    let latest = read_latest_verdicts(&verdict_path)?;
    let engagement_dir = paths.engagement_dir(&engagement_slug);

    let model_path = find_model_file(&paths.analyzer_models_dir());

    let (processed, judged, skipped_existing) = if let Some(mp) = model_path {
        run_llm_analysis(
            app.clone(),
            &engagement_dir,
            &verdict_path,
            &run_id,
            attempts,
            &prompt_index,
            &latest,
            total,
            force,
            mp,
        )
        .await?
    } else {
        run_heuristic_analysis(
            app.clone(),
            &engagement_dir,
            &verdict_path,
            &run_id,
            attempts,
            &prompt_index,
            latest,
            total,
            force,
        )
        .await?
    };

    // Write footer and generate report — shared by both paths.
    if verdict_path.exists() {
        let records = verdicts::read_all(&verdict_path)?;
        let footer = verdicts::summarize_footer(
            &run_id,
            &records,
            runner::run::iso_now(),
            VerdictRunStatus::Completed,
        );
        verdicts::append(&verdict_path, &VerdictRecord::Footer(footer))?;
    }

    let _report_path = generate_report_inner(&paths, &engagement_slug, &run_id)?;

    let _ = app.emit(
        "analysis-progress",
        AnalysisProgressEvent {
            run_id,
            processed,
            total,
            judged,
            skipped_existing,
            finished: true,
            error: None,
        },
    );

    Ok(())
}

/// Run analysis using the local LLM.  All inference is done in a single
/// `spawn_blocking` task so the model is loaded exactly once per run.
/// Progress signals are forwarded to the async side via an mpsc channel.
#[cfg(feature = "analyzer")]
async fn run_llm_analysis(
    app: AppHandle,
    engagement_dir: &std::path::Path,
    verdict_path: &std::path::Path,
    run_id: &str,
    attempts: Vec<storage::runs::RunAttempt>,
    prompt_index: &HashMap<String, PromptMeta>,
    latest: &HashMap<u32, VerdictEntry>,
    total: u32,
    force: bool,
    model_path: std::path::PathBuf,
) -> anyhow::Result<(u32, u32, u32)> {
    // Pre-filter: collect (attempt, meta, skip) tuples so the blocking
    // thread gets everything it needs without borrowing async-side state.
    let work: Vec<(storage::runs::RunAttempt, Option<PromptMeta>, bool)> = attempts
        .into_iter()
        .map(|a| {
            let skip = !force && latest.contains_key(&a.seq);
            let meta = prompt_index.get(&a.prompt_id).cloned();
            (a, meta, skip)
        })
        .collect();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<bool>(); // true = judged
    let engagement_dir = engagement_dir.to_path_buf();
    let verdict_path = verdict_path.to_path_buf();
    let run_id_cl = run_id.to_owned();

    let handle = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let judge = analyzer::llm::LlmJudge::load(&model_path)?;

        for (attempt, meta, skip) in work {
            if skip {
                let _ = tx.send(false);
                continue;
            }

            let verdict = evaluate_with_llm_sync(&engagement_dir, &attempt, meta.as_ref(), &judge)?;
            ensure_verdict_header(&verdict_path, &run_id_cl, &verdict.model_used)?;
            verdicts::append(&verdict_path, &VerdictRecord::Verdict(Box::new(verdict)))?;
            let _ = tx.send(true);
        }
        Ok(())
    });

    let mut processed = 0u32;
    let mut judged = 0u32;
    let mut skipped_existing = 0u32;

    while let Some(was_judged) = rx.recv().await {
        processed += 1;
        if was_judged {
            judged += 1;
        } else {
            skipped_existing += 1;
        }
        let _ = app.emit(
            "analysis-progress",
            AnalysisProgressEvent {
                run_id: run_id.to_owned(),
                processed,
                total,
                judged,
                skipped_existing,
                finished: false,
                error: None,
            },
        );
    }

    handle.await??;
    Ok((processed, judged, skipped_existing))
}

/// Synchronous evaluation using the LLM judge — called from inside
/// `spawn_blocking`, so blocking I/O is fine here.
#[cfg(feature = "analyzer")]
fn evaluate_with_llm_sync(
    engagement_dir: &std::path::Path,
    attempt: &storage::runs::RunAttempt,
    prompt_meta: Option<&PromptMeta>,
    judge: &analyzer::llm::LlmJudge,
) -> anyhow::Result<VerdictEntry> {
    let evaluated_at = runner::run::iso_now();
    let response_text = load_attempt_response_text(engagement_dir, attempt);
    let prompt_text = attempt
        .prompt_text
        .clone()
        .or_else(|| prompt_meta.map(|p| p.prompt_text.clone()))
        .unwrap_or_default();

    let input = JudgeInput {
        prompt_text,
        response_text,
        category: prompt_meta
            .map(|p| p.category.clone())
            .unwrap_or_else(|| attempt.prompt_id.clone()),
        tags: prompt_meta.map(|p| p.tags.clone()).unwrap_or_default(),
        owasp_ref: prompt_meta.and_then(|p| p.owasp_ref.clone()),
        severity: prompt_meta.and_then(|p| p.severity.clone()),
        request_failed: attempt.response.status == 0 || attempt.response.error.is_some(),
    };

    let output = analyzer::judge_with_llm(&input, judge)?;
    Ok(analyzer::to_verdict_entry(
        attempt.seq,
        &evaluated_at,
        &input,
        output,
    ))
}

/// Run analysis using the built-in heuristic judge (no model required).
#[cfg(feature = "analyzer")]
async fn run_heuristic_analysis(
    app: AppHandle,
    engagement_dir: &std::path::Path,
    verdict_path: &std::path::Path,
    run_id: &str,
    attempts: Vec<storage::runs::RunAttempt>,
    prompt_index: &HashMap<String, PromptMeta>,
    mut latest: HashMap<u32, VerdictEntry>,
    total: u32,
    force: bool,
) -> anyhow::Result<(u32, u32, u32)> {
    let mut processed = 0u32;
    let mut judged = 0u32;
    let mut skipped_existing = 0u32;

    for attempt in attempts {
        processed += 1;

        if latest.contains_key(&attempt.seq) && !force {
            skipped_existing += 1;
            let _ = app.emit(
                "analysis-progress",
                AnalysisProgressEvent {
                    run_id: run_id.to_owned(),
                    processed,
                    total,
                    judged,
                    skipped_existing,
                    finished: false,
                    error: None,
                },
            );
            continue;
        }

        let prompt_meta = prompt_index.get(&attempt.prompt_id);
        let verdict = evaluate_attempt(engagement_dir, &attempt, prompt_meta).await?;
        ensure_verdict_header(verdict_path, run_id, &verdict.model_used)?;
        verdicts::append(
            verdict_path,
            &VerdictRecord::Verdict(Box::new(verdict.clone())),
        )?;
        latest.insert(attempt.seq, verdict);
        judged += 1;

        let _ = app.emit(
            "analysis-progress",
            AnalysisProgressEvent {
                run_id: run_id.to_owned(),
                processed,
                total,
                judged,
                skipped_existing,
                finished: false,
                error: None,
            },
        );
    }

    Ok((processed, judged, skipped_existing))
}

/// Scan the models directory for the first `.gguf` file.
#[cfg(feature = "analyzer")]
fn find_model_file(models_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(models_dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        (p.extension().and_then(|x| x.to_str()) == Some("gguf")).then_some(p)
    })
}

#[cfg(feature = "analyzer")]
fn generate_report_inner(
    paths: &HammorPaths,
    engagement_slug: &str,
    run_id: &str,
) -> anyhow::Result<PathBuf> {
    let engagement_dir = paths.engagement_dir(engagement_slug);
    let run_path = run_path_for(paths, engagement_slug, run_id);
    if !run_path.exists() {
        return Err(anyhow::anyhow!("run '{}' not found", run_id));
    }

    let records = read_all(&run_path)?;
    let mut started_at = None;
    let mut finished_at = None;
    let mut attempts = Vec::new();

    for record in &records {
        match record {
            RunRecord::Header(h) => started_at = Some(h.started_at.clone()),
            RunRecord::Attempt(a) => attempts.push((**a).clone()),
            RunRecord::Footer(f) => finished_at = Some(f.finished_at.clone()),
        }
    }

    let verdict_path = verdict_path_for_run(paths, engagement_slug, run_id);
    let latest_verdicts = read_latest_verdicts(&verdict_path)?;
    let mut verdict_list = latest_verdicts.values().cloned().collect::<Vec<_>>();
    verdict_list.sort_by_key(|v| v.seq);

    let report_attempts = attempts
        .into_iter()
        .map(|attempt| {
            let response_excerpt = load_attempt_response_text(&engagement_dir, &attempt);
            ReportAttempt {
                seq: attempt.seq,
                prompt_id: attempt.prompt_id,
                step_id: attempt.step_id,
                iteration: attempt.iteration,
                session: attempt.session,
                http_status: attempt.response.status,
                latency_ms: Some(attempt.timing.duration_ms),
                response_excerpt,
            }
        })
        .collect::<Vec<_>>();

    let report_data = build_report_data(ReportBuildInput {
        engagement_slug: engagement_slug.to_owned(),
        run_id: run_id.to_owned(),
        generated_at: runner::run::iso_now(),
        started_at,
        finished_at,
        attempts: report_attempts,
        verdicts: verdict_list,
    });

    let html = render_html_report(&report_data)?;
    let report_path = report_path_for(paths, engagement_slug, run_id);
    atomic_write(&report_path, html.as_bytes())?;
    Ok(report_path)
}

#[cfg(feature = "analyzer")]
async fn evaluate_attempt(
    engagement_dir: &Path,
    attempt: &storage::runs::RunAttempt,
    prompt_meta: Option<&PromptMeta>,
) -> anyhow::Result<VerdictEntry> {
    let seq = attempt.seq;
    let evaluated_at = runner::run::iso_now();
    let response_text = load_attempt_response_text(engagement_dir, attempt);
    let prompt_text = attempt
        .prompt_text
        .clone()
        .or_else(|| prompt_meta.map(|p| p.prompt_text.clone()))
        .unwrap_or_default();
    let input = JudgeInput {
        prompt_text,
        response_text,
        category: prompt_meta
            .map(|p| p.category.clone())
            .unwrap_or_else(|| attempt.prompt_id.clone()),
        tags: prompt_meta.map(|p| p.tags.clone()).unwrap_or_default(),
        owasp_ref: prompt_meta.and_then(|p| p.owasp_ref.clone()),
        severity: prompt_meta.and_then(|p| p.severity.clone()),
        request_failed: attempt.response.status == 0 || attempt.response.error.is_some(),
    };

    let verdict = tokio::task::spawn_blocking(move || -> anyhow::Result<VerdictEntry> {
        let output = analyzer::judge(&input)?;
        Ok(analyzer::to_verdict_entry(
            seq,
            &evaluated_at,
            &input,
            output,
        ))
    })
    .await
    .map_err(|e| anyhow::anyhow!("analysis task join failure: {e}"))??;

    Ok(verdict)
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

fn read_latest_verdicts(verdict_path: &Path) -> anyhow::Result<HashMap<u32, VerdictEntry>> {
    if !verdict_path.exists() {
        return Ok(HashMap::new());
    }
    let records = verdicts::read_all(verdict_path)?;
    Ok(verdicts::latest_by_seq(&records))
}

fn verdict_label(verdict: JudgeVerdict) -> String {
    match verdict {
        JudgeVerdict::Success => "SUCCESS",
        JudgeVerdict::Fail => "FAIL",
        JudgeVerdict::Partial => "PARTIAL",
        JudgeVerdict::Unclear => "UNCLEAR",
    }
    .to_owned()
}

fn to_run_verdict_dto(run_id: &str, verdict: &VerdictEntry) -> RunVerdictDto {
    RunVerdictDto {
        run_id: run_id.to_owned(),
        result_id: format!("{run_id}-{}", verdict.seq),
        seq: verdict.seq,
        judge_verdict: verdict_label(verdict.verdict.clone()),
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
        judge_verdict: verdict_label(verdict.verdict.clone()),
        judge_confidence: verdict.confidence,
        judge_reason: verdict.rationale.clone(),
        judge_model_used: verdict.model_used.clone(),
        judge_evaluated_at: verdict.evaluated_at.clone(),
    }
}

#[cfg(feature = "analyzer")]
fn ensure_verdict_header(verdict_path: &Path, run_id: &str, model: &str) -> anyhow::Result<()> {
    if verdict_path.exists() {
        let records = verdicts::read_all(verdict_path)?;
        if records
            .iter()
            .any(|r| matches!(r, VerdictRecord::Header(_)))
        {
            return Ok(());
        }
    }

    let header = VerdictRecord::Header(VerdictHeader {
        run_id: run_id.to_owned(),
        model: model.to_owned(),
        analyzer_version: env!("CARGO_PKG_VERSION").to_owned(),
        started_at: runner::run::iso_now(),
    });
    verdicts::append(verdict_path, &header)
}

/// Split `"<run_id>-<seq>"` into `(run_id, seq)`.
#[cfg(feature = "analyzer")]
fn parse_result_id(result_id: &str) -> anyhow::Result<(String, u32)> {
    let (run_id, seq_str) = result_id
        .rsplit_once('-')
        .ok_or_else(|| anyhow::anyhow!("invalid result_id: {result_id}"))?;
    let seq = seq_str
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("invalid seq in result_id: {result_id}"))?;
    Ok((run_id.to_owned(), seq))
}

#[cfg(feature = "analyzer")]
fn run_path_for(paths: &HammorPaths, engagement_slug: &str, run_id: &str) -> PathBuf {
    paths
        .engagement_dir(engagement_slug)
        .join("runs")
        .join(format!("{run_id}.jsonl"))
}

#[cfg(feature = "analyzer")]
fn load_attempts(run_path: &Path) -> anyhow::Result<Vec<storage::runs::RunAttempt>> {
    let records = read_all(run_path)?;
    Ok(records
        .into_iter()
        .filter_map(|r| match r {
            RunRecord::Attempt(a) => Some(*a),
            _ => None,
        })
        .collect())
}

#[cfg(feature = "analyzer")]
fn load_attempt_map(run_path: &Path) -> anyhow::Result<HashMap<u32, storage::runs::RunAttempt>> {
    let mut map = HashMap::new();
    for attempt in load_attempts(run_path)? {
        map.insert(attempt.seq, attempt);
    }
    Ok(map)
}

#[cfg(feature = "analyzer")]
fn load_attempt(run_path: &Path, seq: u32) -> anyhow::Result<storage::runs::RunAttempt> {
    load_attempts(run_path)?
        .into_iter()
        .find(|a| a.seq == seq)
        .ok_or_else(|| anyhow::anyhow!("attempt {} not found in {}", seq, run_path.display()))
}

#[cfg(feature = "analyzer")]
#[derive(Debug, Clone)]
struct PromptMeta {
    category: String,
    tags: Vec<String>,
    owasp_ref: Option<String>,
    severity: Option<String>,
    prompt_text: String,
}

#[cfg(feature = "analyzer")]
fn build_prompt_index(paths: &HammorPaths) -> anyhow::Result<HashMap<String, PromptMeta>> {
    let mut index = HashMap::new();
    let all = storage::prompts::load_all(&paths.prompts_dir())?;
    for (category, entries) in all {
        for entry in entries {
            index.entry(entry.id.clone()).or_insert_with(|| PromptMeta {
                category: category.clone(),
                tags: entry.tags.clone(),
                owasp_ref: entry.owasp_ref.clone(),
                severity: Some(severity_label(&entry.severity)),
                prompt_text: entry.text.clone(),
            });
        }
    }
    Ok(index)
}

#[cfg(feature = "analyzer")]
fn severity_label(severity: &storage::types::Severity) -> String {
    match severity {
        storage::types::Severity::Low => "low",
        storage::types::Severity::Medium => "medium",
        storage::types::Severity::High => "high",
        storage::types::Severity::Critical => "critical",
    }
    .to_owned()
}

#[cfg(feature = "analyzer")]
fn load_attempt_response_text(
    engagement_dir: &Path,
    attempt: &storage::runs::RunAttempt,
) -> String {
    if let Some(ref body_file) = attempt.response.body_file {
        if let Ok(Some(text)) = storage::runs::read_body_by_relative_path(engagement_dir, body_file)
        {
            return text;
        }
    }
    attempt.response.error.clone().unwrap_or_default()
}
