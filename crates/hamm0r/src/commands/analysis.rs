//! Tauri command surface for analyzer/judge functionality.
//!
//! Heuristic-only commands (`judge_result`, `judge_all`, …) call into
//! `analyzer::pipeline` directly — runtime-free, always available in core.
//! `start_analysis` is the LLM-backed path: it shells out to the standalone
//! `analyz0r` binary and requires the analyzer bundle to be installed.
//! When the bundle is missing, the command returns an error rather than
//! silently degrading to the heuristic — the UI already exposes the
//! per-result Judge buttons for that case, and a hidden fallback would
//! make "Analyze" mean two different things depending on install state.
//!
//! Binary discovery (in priority order):
//!   1. `$HAMM0R_ANALYZOR_BIN` — explicit override (dev convenience).
//!   2. `~/hamm0r/analyzer/bin/analyz0r[.exe]` — production install layout.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use analyzer::hosted::{build_hosted_config, judge_with_hosted, HostedJudgeConfigInput};
use analyzer::pipeline::{self, HostedJudgeConfig, JudgeOneOptions, JudgeOutcome, JudgeRunSummary};
use analyzer::JudgeInput;
use serde::Serialize;
use storage::verdicts::VerdictEntry;
use storage::HammorPaths;
use tauri::{AppHandle, Emitter as _, State};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};
use tokio::process::Command;

use super::{report_user_relevant_error, AnalysisCancelTracker, AnalyzerLoggerState, AppPaths};
use crate::error::CommandError;

const ANALYZOR_BIN_ENV: &str = "HAMM0R_ANALYZOR_BIN";
const ANALYZOR_JUDGE_PROMPT_TEMPLATE_ENV: &str = "HAMM0R_ANALYZOR_JUDGE_PROMPT_TEMPLATE";

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

#[derive(Debug, Clone, Serialize)]
pub struct HostedJudgeTestDto {
    pub ok: bool,
    pub judge_mode: String,
    pub model_used: String,
    pub verdict: String,
    pub confidence: f32,
    pub reason: String,
}

#[derive(Debug, Clone)]
enum ResolvedJudgeSettings {
    Local {
        judge_prompt_template: Option<String>,
    },
    Hosted {
        judge_prompt_template: Option<String>,
        provider: String,
        endpoint: String,
        deployment: String,
        api_style: String,
        api_version: Option<String>,
        api_key: String,
        max_input_chars: u32,
        max_output_tokens: u32,
        request_timeout_seconds: u32,
        max_retries: u32,
    },
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
    let judge_settings = load_resolved_judge_settings(&paths.0)?;
    let run_id_for_blocking = run_id.clone();

    let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<JudgeOutcome> {
        match &judge_settings {
            ResolvedJudgeSettings::Local {
                judge_prompt_template,
            } => pipeline::judge_one_heuristic(&JudgeOneOptions {
                engagement_dir: &engagement_dir,
                prompts_dir: &prompts_dir,
                run_id: &run_id_for_blocking,
                seq,
                judge_prompt_template: judge_prompt_template.as_deref(),
                analyzer_version: &analyzer_version,
                force,
            }),
            ResolvedJudgeSettings::Hosted {
                judge_prompt_template,
                provider,
                endpoint,
                deployment,
                api_style,
                api_version,
                api_key,
                max_input_chars,
                max_output_tokens,
                request_timeout_seconds,
                max_retries,
            } => pipeline::judge_one_hosted(
                &JudgeOneOptions {
                    engagement_dir: &engagement_dir,
                    prompts_dir: &prompts_dir,
                    run_id: &run_id_for_blocking,
                    seq,
                    judge_prompt_template: judge_prompt_template.as_deref(),
                    analyzer_version: &analyzer_version,
                    force,
                },
                &HostedJudgeConfig {
                    provider,
                    endpoint,
                    deployment,
                    api_style,
                    api_version: api_version.as_deref(),
                    api_key,
                    max_input_chars: *max_input_chars,
                    max_output_tokens: *max_output_tokens,
                    request_timeout_seconds: *request_timeout_seconds,
                    max_retries: *max_retries,
                },
            ),
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!("judge task join failure: {e}"))??;

    let status = if outcome.was_judged() {
        "judged"
    } else {
        "skipped"
    };
    logger.0.info(
        "analysis",
        Some(&run_id),
        &format!("judge_result completed status={status} seq={seq}"),
    );

    Ok(to_judge_result_dto(status, &run_id, outcome.entry()))
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
    let judge_settings = load_resolved_judge_settings(&paths.0)?;

    // Resolve target (run_id, seq) pairs synchronously up front so the
    // blocking task only does the judging work.
    let mut targets: Vec<(String, u32)> = if result_ids.is_empty() {
        let rid = run_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("run_id is required when result_ids is empty"))?;
        let run_path = pipeline::run_path_for(&engagement_dir, rid);
        let attempts = pipeline::load_attempts(&run_path)?;
        attempts
            .into_iter()
            .map(|a| (rid.to_owned(), a.seq))
            .collect()
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
            let outcome = match &judge_settings {
                ResolvedJudgeSettings::Local {
                    judge_prompt_template,
                } => pipeline::judge_one_heuristic(&JudgeOneOptions {
                    engagement_dir: &engagement_dir,
                    prompts_dir: &prompts_dir,
                    run_id: &rid,
                    seq,
                    judge_prompt_template: judge_prompt_template.as_deref(),
                    analyzer_version: &analyzer_version,
                    force,
                })?,
                ResolvedJudgeSettings::Hosted {
                    judge_prompt_template,
                    provider,
                    endpoint,
                    deployment,
                    api_style,
                    api_version,
                    api_key,
                    max_input_chars,
                    max_output_tokens,
                    request_timeout_seconds,
                    max_retries,
                } => pipeline::judge_one_hosted(
                    &JudgeOneOptions {
                        engagement_dir: &engagement_dir,
                        prompts_dir: &prompts_dir,
                        run_id: &rid,
                        seq,
                        judge_prompt_template: judge_prompt_template.as_deref(),
                        analyzer_version: &analyzer_version,
                        force,
                    },
                    &HostedJudgeConfig {
                        provider,
                        endpoint,
                        deployment,
                        api_style,
                        api_version: api_version.as_deref(),
                        api_key,
                        max_input_chars: *max_input_chars,
                        max_output_tokens: *max_output_tokens,
                        request_timeout_seconds: *request_timeout_seconds,
                        max_retries: *max_retries,
                    },
                )?,
            };
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

#[tauri::command]
pub async fn start_analysis(
    app: AppHandle,
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
    cancel_tracker: State<'_, AnalysisCancelTracker>,
    engagement_slug: String,
    run_id: String,
    force: Option<bool>,
) -> Result<String, CommandError> {
    let paths = paths.0.clone();
    let run_id_ret = run_id.clone();
    let force = force.unwrap_or(false);
    let logger = logger.0.clone();
    let tracker = cancel_tracker.0.clone();
    let judge_settings = load_resolved_judge_settings(&paths)?;

    tokio::spawn(async move {
        logger.info(
            "analysis",
            Some(&run_id),
            &format!("Analysis task spawned for engagement={engagement_slug} force={force}"),
        );
        if let Err(err) = analyze_run_and_emit(
            app.clone(),
            paths.clone(),
            tracker.clone(),
            engagement_slug.clone(),
            run_id.clone(),
            force,
            judge_settings.clone(),
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

#[tauri::command]
pub fn test_hosted_judge(
    logger: State<'_, AnalyzerLoggerState>,
    paths: State<'_, AppPaths>,
) -> Result<HostedJudgeTestDto, CommandError> {
    let settings = load_resolved_judge_settings(&paths.0)?;
    let hosted = match settings {
        ResolvedJudgeSettings::Hosted {
            provider,
            endpoint,
            deployment,
            api_style,
            api_version,
            api_key,
            max_input_chars,
            max_output_tokens,
            request_timeout_seconds,
            max_retries,
            ..
        } => build_hosted_config(HostedJudgeConfigInput {
            provider: &provider,
            endpoint: &endpoint,
            deployment: &deployment,
            api_style: &api_style,
            api_version: api_version.as_deref(),
            api_key: &api_key,
            max_input_chars,
            max_output_tokens,
            request_timeout_seconds,
            max_retries,
        })?,
        ResolvedJudgeSettings::Local { .. } => {
            return Err(anyhow::anyhow!(
                "Hosted Judge test is only available when judge mode is set to hosted."
            )
            .into())
        }
    };

    let input = JudgeInput {
        prompt_text: "Summarize the candidate profile in one short sentence.".to_owned(),
        response_text:
            "The candidate appears to be an experienced SAP-focused test manager with strong communication skills."
                .to_owned(),
        category: "hosted-judge-test".to_owned(),
        tags: vec!["test".to_owned(), "benign".to_owned()],
        owasp_ref: None,
        severity: Some("info".to_owned()),
        request_failed: false,
        judge_prompt_template: Some(analyzer::default_judge_prompt_template().to_owned()),
    };
    let output = judge_with_hosted(&input, &hosted)
        .map_err(|err| anyhow::anyhow!("Hosted Judge test failed: {err}"))?;
    let verdict_label = match output.verdict {
        storage::verdicts::JudgeVerdict::Success => "SUCCESS",
        storage::verdicts::JudgeVerdict::Fail => "FAIL",
        storage::verdicts::JudgeVerdict::Partial => "PARTIAL",
        storage::verdicts::JudgeVerdict::Unclear => "UNCLEAR",
    };
    logger.0.info(
        "analysis",
        None,
        &format!(
            "Hosted Judge connectivity test completed model={} verdict={}",
            output.model_used, verdict_label
        ),
    );
    Ok(HostedJudgeTestDto {
        ok: true,
        judge_mode: "hosted".to_owned(),
        model_used: output.model_used,
        verdict: verdict_label.to_owned(),
        confidence: output.confidence,
        reason: output.reason,
    })
}

// ── start_analysis implementation ─────────────────────────────────────────────

type CancelMap = Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>>;

async fn analyze_run_and_emit(
    app: AppHandle,
    paths: HammorPaths,
    cancel_tracker: CancelMap,
    engagement_slug: String,
    run_id: String,
    force: bool,
    judge_settings: ResolvedJudgeSettings,
) -> anyhow::Result<()> {
    let summary = match judge_settings {
        ResolvedJudgeSettings::Hosted {
            judge_prompt_template,
            provider,
            endpoint,
            deployment,
            api_style,
            api_version,
            api_key,
            max_input_chars,
            max_output_tokens,
            request_timeout_seconds,
            max_retries,
        } => {
            run_hosted_analysis(
                app.clone(),
                paths.clone(),
                engagement_slug.clone(),
                run_id.clone(),
                force,
                judge_prompt_template,
                provider,
                endpoint,
                deployment,
                api_style,
                api_version,
                api_key,
                max_input_chars,
                max_output_tokens,
                request_timeout_seconds,
                max_retries,
            )
            .await?
        }
        ResolvedJudgeSettings::Local {
            judge_prompt_template,
        } => {
            let bin = resolve_analyzor_bin(&paths).ok_or_else(|| {
                anyhow::anyhow!(
                    "analyzer is not installed (no analyz0r binary at expected layout); \
                     install it from Settings → Analyz0r before running an analysis"
                )
            })?;
            let ollama_dev_path = std::env::var("HAMM0R_ANALYZOR_OLLAMA_URL").is_ok()
                && std::env::var("HAMM0R_ANALYZOR_OLLAMA_MODEL").is_ok();
            let model_path = if ollama_dev_path {
                PathBuf::new()
            } else {
                pipeline::find_model_file(&paths.analyzer_models_dir()).ok_or_else(|| {
                    anyhow::anyhow!(
                        "analyzer install is broken (no model file present); \
                         repair the install from Settings → Analyz0r"
                    )
                })?
            };

            run_via_subprocess(
                app.clone(),
                bin,
                paths.engagement_dir(&engagement_slug),
                paths.prompts_dir(),
                run_id.clone(),
                model_path,
                force,
                judge_prompt_template,
                cancel_tracker,
            )
            .await?
        }
    };

    // Both paths leave verdict JSONL behind; rendering the report is a
    // pure reader, so we always do it here in the orchestrator.
    pipeline::generate_report(&paths.engagement_dir(&engagement_slug), &run_id)?;

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

// Single call site; the params are all distinct concerns the orchestrator
// holds locally, so wrapping in a struct would just trade arg-list noise
// for boilerplate without making any of them group meaningfully.
#[allow(clippy::too_many_arguments)]
async fn run_via_subprocess(
    app: AppHandle,
    bin: PathBuf,
    engagement_dir: PathBuf,
    prompts_dir: PathBuf,
    run_id: String,
    model_path: PathBuf,
    force: bool,
    judge_prompt_template: Option<String>,
    cancel_tracker: CancelMap,
) -> anyhow::Result<JudgeRunSummary> {
    let mut cmd = Command::new(&bin);
    cmd.arg("judge-run")
        .arg("--engagement-dir")
        .arg(&engagement_dir)
        .arg("--prompts-dir")
        .arg(&prompts_dir)
        .arg("--run")
        .arg(&run_id);

    // Dev-only escape hatch: if both env vars are set, route the run
    // through Ollama instead of the in-process llama-cpp model. Lets a
    // developer validate the end-to-end flow without the C++ toolchain
    // needed to compile `llama-cpp-2`. Production builds leave these
    // unset and use --model. See docs/analyzorPlan.md.
    let ollama_url = std::env::var("HAMM0R_ANALYZOR_OLLAMA_URL").ok();
    let ollama_model = std::env::var("HAMM0R_ANALYZOR_OLLAMA_MODEL").ok();
    match (ollama_url, ollama_model) {
        (Some(url), Some(model)) => {
            cmd.arg("--ollama-url").arg(url);
            cmd.arg("--ollama-model").arg(model);
        }
        _ => {
            cmd.arg("--model").arg(&model_path);
        }
    }

    if force {
        cmd.arg("--force");
    }
    if let Some(template) = judge_prompt_template {
        cmd.env(ANALYZOR_JUDGE_PROMPT_TEMPLATE_ENV, template);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn {}: {e}", bin.display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("analyz0r stdout not captured"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("analyz0r stderr not captured"))?;

    // Register a cancel channel so `cancel_analysis` can interrupt this
    // subprocess. Drop guard removes the entry on every exit path so a
    // crashed analysis doesn't leave a stale sender that blocks a
    // re-run with the same run_id.
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut guard = cancel_tracker
            .lock()
            .map_err(|_| anyhow::anyhow!("analysis cancel tracker poisoned"))?;
        guard.insert(run_id.clone(), cancel_tx);
    }
    struct CancelEntryGuard {
        tracker: CancelMap,
        run_id: String,
    }
    impl Drop for CancelEntryGuard {
        fn drop(&mut self) {
            if let Ok(mut g) = self.tracker.lock() {
                g.remove(&self.run_id);
            }
        }
    }
    let _entry_guard = CancelEntryGuard {
        tracker: cancel_tracker.clone(),
        run_id: run_id.clone(),
    };

    let app_clone = app.clone();
    let run_id_clone = run_id.clone();
    let stdout_task = tokio::spawn(async move {
        let mut summary: Option<JudgeRunSummary> = None;
        let mut error_message: Option<String> = None;
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            match value.get("event").and_then(|v| v.as_str()) {
                Some("progress") => {
                    let _ = app_clone.emit(
                        "analysis-progress",
                        AnalysisProgressEvent {
                            run_id: run_id_clone.clone(),
                            processed: u32_field(&value, "processed"),
                            total: u32_field(&value, "total"),
                            judged: u32_field(&value, "judged"),
                            skipped_existing: u32_field(&value, "skipped_existing"),
                            finished: false,
                            error: None,
                        },
                    );
                }
                Some("result") => {
                    summary = Some(JudgeRunSummary {
                        processed: u32_field(&value, "processed"),
                        total: u32_field(&value, "total"),
                        judged: u32_field(&value, "judged"),
                        skipped_existing: u32_field(&value, "skipped_existing"),
                    });
                }
                Some("error") => {
                    error_message = value
                        .get("message")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                }
                _ => {}
            }
        }
        (summary, error_message)
    });

    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut buf).await;
        buf
    });

    // Race the subprocess against the cancel channel. If cancel fires,
    // signal the child and let `wait()` return; the bog-standard
    // "exited with non-zero status" error path then surfaces it as a
    // cancellation to the caller.
    let mut cancelled = false;
    let status = tokio::select! {
        s = child.wait() => s.map_err(|e| anyhow::anyhow!("analyz0r wait: {e}"))?,
        _ = &mut cancel_rx => {
            cancelled = true;
            let _ = child.start_kill();
            child.wait().await.map_err(|e| anyhow::anyhow!("analyz0r wait after cancel: {e}"))?
        }
    };
    let (summary, error_message) = stdout_task
        .await
        .map_err(|e| anyhow::anyhow!("analyz0r stdout task: {e}"))?;
    let stderr_text = stderr_task.await.unwrap_or_default();

    if cancelled {
        return Err(anyhow::anyhow!("analysis cancelled by user"));
    }

    if !status.success() {
        let detail = error_message
            .or_else(|| Some(stderr_text.trim().to_owned()).filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "no diagnostic output".to_owned());
        return Err(anyhow::anyhow!(
            "analyz0r exited with code {}: {}",
            status.code().unwrap_or(-1),
            detail
        ));
    }

    summary.ok_or_else(|| anyhow::anyhow!("analyz0r finished without emitting a result event"))
}

#[tauri::command]
pub fn cancel_analysis(
    logger: State<'_, AnalyzerLoggerState>,
    cancel_tracker: State<'_, AnalysisCancelTracker>,
    run_id: String,
) -> Result<bool, CommandError> {
    let sender = cancel_tracker
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("analysis cancel tracker poisoned"))?
        .remove(&run_id);
    match sender {
        Some(tx) => {
            // send returns Err only if the receiver was dropped, which
            // means the orchestrator already exited — treat that as
            // "nothing to cancel" rather than failing the command.
            let delivered = tx.send(()).is_ok();
            logger.0.info(
                "analysis",
                Some(&run_id),
                if delivered {
                    "cancel_analysis sent kill signal"
                } else {
                    "cancel_analysis: orchestrator already exited"
                },
            );
            Ok(delivered)
        }
        None => Ok(false),
    }
}

fn u32_field(value: &serde_json::Value, key: &str) -> u32 {
    value
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(0)
}

fn load_resolved_judge_settings(paths: &HammorPaths) -> anyhow::Result<ResolvedJudgeSettings> {
    let root = paths.root().to_string_lossy().into_owned();
    let config = storage::settings::load_or_default(&paths.config_path(), root)?;
    let judge_prompt_template = config
        .analyzer
        .judge_prompt_template
        .filter(|s| !s.trim().is_empty());
    match config.analyzer.judge_mode {
        storage::types::AnalyzerJudgeMode::Local => Ok(ResolvedJudgeSettings::Local {
            judge_prompt_template,
        }),
        storage::types::AnalyzerJudgeMode::Hosted => {
            let endpoint = config.analyzer.hosted_judge.endpoint.trim().to_owned();
            if endpoint.is_empty() {
                anyhow::bail!("Hosted Judge is selected, but no endpoint is configured.");
            }
            let deployment = config.analyzer.hosted_judge.deployment.trim().to_owned();
            if deployment.is_empty() {
                anyhow::bail!("Hosted Judge is selected, but no deployment/model is configured.");
            }
            let secret_ref = config.analyzer.hosted_judge.secret_ref.trim().to_owned();
            let api_key = storage::secrets::resolve_token(&secret_ref)?
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Hosted Judge is selected, but no API key is stored for {}.",
                        secret_ref
                    )
                })?;

            Ok(ResolvedJudgeSettings::Hosted {
                judge_prompt_template,
                provider: match config.analyzer.hosted_judge.provider {
                    storage::types::HostedJudgeProvider::AzureOpenai => "azure_openai".to_owned(),
                },
                endpoint,
                deployment,
                api_style: match config.analyzer.hosted_judge.api_style {
                    storage::types::HostedJudgeApiStyle::Auto => "auto".to_owned(),
                    storage::types::HostedJudgeApiStyle::ChatCompletions => {
                        "chat_completions".to_owned()
                    }
                    storage::types::HostedJudgeApiStyle::Responses => "responses".to_owned(),
                },
                api_version: config.analyzer.hosted_judge.api_version.clone(),
                api_key,
                max_input_chars: config.analyzer.hosted_judge.max_input_chars,
                max_output_tokens: config.analyzer.hosted_judge.max_output_tokens,
                request_timeout_seconds: config.analyzer.hosted_judge.request_timeout_seconds,
                max_retries: config.analyzer.hosted_judge.max_retries,
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_hosted_analysis(
    app: AppHandle,
    paths: HammorPaths,
    engagement_slug: String,
    run_id: String,
    force: bool,
    judge_prompt_template: Option<String>,
    provider: String,
    endpoint: String,
    deployment: String,
    api_style: String,
    api_version: Option<String>,
    api_key: String,
    max_input_chars: u32,
    max_output_tokens: u32,
    request_timeout_seconds: u32,
    max_retries: u32,
) -> anyhow::Result<JudgeRunSummary> {
    let engagement_dir = paths.engagement_dir(&engagement_slug);
    let prompts_dir = paths.prompts_dir();
    let analyzer_version = env!("CARGO_PKG_VERSION").to_owned();
    tokio::task::spawn_blocking(move || {
        let mut emit_progress = |progress: analyzer::pipeline::Progress| {
            let _ = app.emit(
                "analysis-progress",
                AnalysisProgressEvent {
                    run_id: run_id.clone(),
                    processed: progress.processed,
                    total: progress.total,
                    judged: progress.judged,
                    skipped_existing: progress.skipped_existing,
                    finished: false,
                    error: None,
                },
            );
        };
        pipeline::judge_run(
            &pipeline::JudgeRunOptions {
                engagement_dir: &engagement_dir,
                prompts_dir: &prompts_dir,
                run_id: &run_id,
                judge_prompt_template: judge_prompt_template.as_deref(),
                model_path: None,
                ollama: None,
                hosted: Some(HostedJudgeConfig {
                    provider: &provider,
                    endpoint: &endpoint,
                    deployment: &deployment,
                    api_style: &api_style,
                    api_version: api_version.as_deref(),
                    api_key: &api_key,
                    max_input_chars,
                    max_output_tokens,
                    request_timeout_seconds,
                    max_retries,
                }),
                analyzer_version: &analyzer_version,
                force,
            },
            &mut emit_progress,
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("hosted analysis task join failure: {e}"))?
}

fn resolve_analyzor_bin(paths: &HammorPaths) -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var(ANALYZOR_BIN_ENV) {
        let p = PathBuf::from(env_path);
        if p.exists() {
            return Some(p);
        }
    }
    let bundled = paths.analyzer_dir().join("bin").join(if cfg!(windows) {
        "analyz0r.exe"
    } else {
        "analyz0r"
    });
    bundled.exists().then_some(bundled)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Bundled-path discovery and env-var override are exercised in a single
    /// test because both touch the process-wide `HAMM0R_ANALYZOR_BIN` and
    /// would otherwise race under `cargo test`'s parallel execution.
    #[test]
    fn resolve_analyzor_bin_picks_env_then_bundled() {
        let tmp = TempDir::new().unwrap();
        let paths = HammorPaths::with_root(tmp.path());

        // Baseline: nothing installed → None.
        std::env::remove_var(ANALYZOR_BIN_ENV);
        assert!(resolve_analyzor_bin(&paths).is_none());

        // Create the bundled-install layout and check it gets picked up.
        let bin_dir = paths.analyzer_dir().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let bin_name = if cfg!(windows) {
            "analyz0r.exe"
        } else {
            "analyz0r"
        };
        let bundled = bin_dir.join(bin_name);
        fs::write(&bundled, b"#!/bin/sh\nexit 0\n").unwrap();
        assert_eq!(resolve_analyzor_bin(&paths), Some(bundled.clone()));

        // Env var must override the bundled path, even when both exist.
        let override_path = tmp.path().join("custom-analyz0r");
        fs::write(&override_path, b"#!/bin/sh\nexit 0\n").unwrap();
        std::env::set_var(ANALYZOR_BIN_ENV, &override_path);
        let resolved = resolve_analyzor_bin(&paths);
        std::env::remove_var(ANALYZOR_BIN_ENV);
        assert_eq!(resolved, Some(override_path));
    }
}
