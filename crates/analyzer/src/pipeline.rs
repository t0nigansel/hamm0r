//! End-to-end judging and report orchestration.
//!
//! The pipeline owns the on-disk side of the analyzer: reading run JSONL,
//! looking up prompt metadata, calling the heuristic or LLM judge, writing
//! verdict JSONL, and rendering reports. It is fully synchronous and has no
//! Tauri or async-runtime dependencies — callers wrap it as needed.
//!
//! Two consumers exist:
//!   * the in-process Tauri command layer in `crates/hamm0r`
//!   * (forthcoming) the standalone `analyz0r` CLI binary
//!
//! Both must produce byte-identical verdict files and reports for the same
//! inputs, which is why the orchestration lives here rather than in either
//! caller.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::hosted::{build_hosted_config, judge_with_hosted, HostedJudgeConfigInput};
#[cfg(feature = "runtime")]
use crate::judge_with_llm;
#[cfg(feature = "runtime")]
use crate::llm::LlmJudge;
use crate::ollama::{judge_with_ollama, OllamaJudge};
use crate::report::{
    build_report_data, render_html_report, render_markdown_report, ReportAttempt,
    ReportBuildInput,
};
use crate::{judge as judge_heuristic, to_verdict_entry, JudgeInput};
use storage::atomic_write;
use storage::runs::{read_all as read_run_records, RunAttempt, RunRecord};
use storage::types::Severity;
use storage::verdicts::{
    self, JudgeVerdict, VerdictEntry, VerdictHeader, VerdictRecord, VerdictRunStatus,
};

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PromptMeta {
    pub category: String,
    pub tags: Vec<String>,
    pub owasp_ref: Option<String>,
    pub severity: Option<String>,
    pub prompt_text: String,
}

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub processed: u32,
    pub total: u32,
    pub judged: u32,
    pub skipped_existing: u32,
}

/// Whether `judge_attempt` actually ran the judge or skipped because a verdict
/// already existed (and `force` was off).
#[derive(Debug, Clone)]
pub enum JudgeOutcome {
    Judged(VerdictEntry),
    Skipped(VerdictEntry),
}

impl JudgeOutcome {
    pub fn entry(&self) -> &VerdictEntry {
        match self {
            JudgeOutcome::Judged(e) | JudgeOutcome::Skipped(e) => e,
        }
    }

    pub fn was_judged(&self) -> bool {
        matches!(self, JudgeOutcome::Judged(_))
    }
}

#[derive(Debug, Clone)]
pub struct JudgeRunOptions<'a> {
    pub engagement_dir: &'a Path,
    pub prompts_dir: &'a Path,
    pub run_id: &'a str,
    pub judge_prompt_template: Option<&'a str>,
    /// If set, attempts are evaluated with the local LLM (requires the
    /// `runtime` feature). Ignored when `ollama` is also set.
    pub model_path: Option<&'a Path>,
    /// Dev-only Ollama backend. When `Some`, judges are run by POSTing
    /// to that Ollama instance instead of loading a GGUF in-process.
    /// Takes precedence over `model_path` so a developer can flip on
    /// the sidecar without changing other call-site plumbing.
    pub ollama: Option<OllamaConfig<'a>>,
    pub hosted: Option<HostedJudgeConfig<'a>>,
    pub analyzer_version: &'a str,
    pub force: bool,
}

#[derive(Debug, Clone)]
pub struct OllamaConfig<'a> {
    /// Base URL of the Ollama server, e.g. `http://localhost:11434`.
    pub url: &'a str,
    /// Model tag in Ollama (e.g. `qwen2.5:3b`).
    pub model: &'a str,
}

#[derive(Debug, Clone)]
pub struct HostedJudgeConfig<'a> {
    pub provider: &'a str,
    pub endpoint: &'a str,
    pub deployment: &'a str,
    pub api_style: &'a str,
    pub api_version: Option<&'a str>,
    pub api_key: &'a str,
    pub max_input_chars: u32,
    pub max_output_tokens: u32,
    pub request_timeout_seconds: u32,
    pub max_retries: u32,
}

#[derive(Debug, Clone)]
pub struct JudgeRunSummary {
    pub processed: u32,
    pub total: u32,
    pub judged: u32,
    pub skipped_existing: u32,
}

// ── Path helpers ──────────────────────────────────────────────────────────────

pub fn run_path_for(engagement_dir: &Path, run_id: &str) -> PathBuf {
    engagement_dir.join("runs").join(format!("{run_id}.jsonl"))
}

pub fn verdict_path_for(engagement_dir: &Path, run_id: &str) -> PathBuf {
    engagement_dir
        .join("runs")
        .join(format!("{run_id}.verdicts.jsonl"))
}

pub fn report_path_for(engagement_dir: &Path, run_id: &str) -> PathBuf {
    engagement_dir
        .join("reports")
        .join(format!("report-{run_id}.html"))
}

/// Section 7.1: markdown report sibling to the HTML report. Same data,
/// different format. Written alongside the HTML so users can share or
/// convert the report (e.g. to PDF) externally.
pub fn markdown_report_path_for(engagement_dir: &Path, run_id: &str) -> PathBuf {
    engagement_dir
        .join("reports")
        .join(format!("report-{run_id}.md"))
}

/// Scan `models_dir` for the first `.gguf` file. Returns `None` when the
/// directory is missing or contains no model.
pub fn find_model_file(models_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(models_dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        (p.extension().and_then(|x| x.to_str()) == Some("gguf")).then_some(p)
    })
}

/// Split `"<run_id>-<seq>"` into `(run_id, seq)`.
pub fn parse_result_id(result_id: &str) -> anyhow::Result<(String, u32)> {
    let (run_id, seq_str) = result_id
        .rsplit_once('-')
        .ok_or_else(|| anyhow::anyhow!("invalid result_id: {result_id}"))?;
    let seq = seq_str
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("invalid seq in result_id: {result_id}"))?;
    Ok((run_id.to_owned(), seq))
}

// ── Run reading ───────────────────────────────────────────────────────────────

pub fn load_attempts(run_path: &Path) -> anyhow::Result<Vec<RunAttempt>> {
    let records = read_run_records(run_path)?;
    Ok(records
        .into_iter()
        .filter_map(|r| match r {
            RunRecord::Attempt(a) => Some(*a),
            _ => None,
        })
        .collect())
}

pub fn load_attempt_map(run_path: &Path) -> anyhow::Result<HashMap<u32, RunAttempt>> {
    let mut map = HashMap::new();
    for attempt in load_attempts(run_path)? {
        map.insert(attempt.seq, attempt);
    }
    Ok(map)
}

pub fn load_attempt(run_path: &Path, seq: u32) -> anyhow::Result<RunAttempt> {
    load_attempts(run_path)?
        .into_iter()
        .find(|a| a.seq == seq)
        .ok_or_else(|| anyhow::anyhow!("attempt {seq} not found in {}", run_path.display()))
}

pub fn read_latest_verdicts(verdict_path: &Path) -> anyhow::Result<HashMap<u32, VerdictEntry>> {
    if !verdict_path.exists() {
        return Ok(HashMap::new());
    }
    let records = verdicts::read_all(verdict_path)?;
    Ok(verdicts::latest_by_seq(&records))
}

pub fn load_attempt_response_text(engagement_dir: &Path, attempt: &RunAttempt) -> String {
    if let Some(ref body_file) = attempt.response.body_file {
        if let Ok(Some(text)) = storage::runs::read_body_by_relative_path(engagement_dir, body_file)
        {
            return text;
        }
    }
    attempt.response.error.clone().unwrap_or_default()
}

// ── Prompt index ──────────────────────────────────────────────────────────────

pub fn build_prompt_index(prompts_dir: &Path) -> anyhow::Result<HashMap<String, PromptMeta>> {
    let mut index = HashMap::new();
    let all = storage::prompts::load_all(prompts_dir)?;
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

pub fn severity_label(severity: &Severity) -> String {
    match severity {
        Severity::Low => "low",
        Severity::Medium => "medium",
        Severity::High => "high",
        Severity::Critical => "critical",
    }
    .to_owned()
}

// ── Verdict file management ───────────────────────────────────────────────────

pub fn ensure_verdict_header(
    verdict_path: &Path,
    run_id: &str,
    model: &str,
    analyzer_version: &str,
) -> anyhow::Result<()> {
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
        analyzer_version: analyzer_version.to_owned(),
        started_at: iso_now(),
    });
    verdicts::append(verdict_path, &header)
}

/// Section 1.10 of docs/ToDo.md and Phase 5 of multiSessionPlan.md.
///
/// Walks the run JSONL for `LeakDetected` records (produced by the
/// runner's leak scanner) and emits one `cross_session_leak` verdict
/// per leak. Auto-flagged: no LLM judging, no judge prompt — the leak
/// is structurally proven by the canary substring match.
///
/// Idempotent: if the verdict log already carries a `cross_session_leak`
/// verdict for the same `(seq, planted_session)` pair, it isn't
/// re-emitted. This means re-running the analyzer on a run with leaks
/// doesn't pile up duplicates.
fn emit_cross_session_leak_verdicts(
    engagement_dir: &Path,
    run_id: &str,
    analyzer_version: &str,
    verdict_path: &Path,
) -> anyhow::Result<()> {
    let run_path = run_path_for(engagement_dir, run_id);
    if !run_path.exists() {
        return Ok(());
    }
    let records = read_run_records(&run_path)?;
    let leaks: Vec<&storage::runs::LeakDetected> = records
        .iter()
        .filter_map(|r| match r {
            RunRecord::LeakDetected(l) => Some(l),
            _ => None,
        })
        .collect();
    if leaks.is_empty() {
        return Ok(());
    }

    // Idempotency: collect (seq, rationale-prefix) tuples already
    // present in the verdict log so a re-judge doesn't duplicate them.
    let mut existing: std::collections::HashSet<(u32, String)> = std::collections::HashSet::new();
    if verdict_path.exists() {
        for record in verdicts::read_all(verdict_path)?.iter() {
            if let VerdictRecord::Verdict(v) = record {
                if v.category == "cross_session_leak" {
                    existing.insert((v.seq, v.model_output_hash.clone()));
                }
            }
        }
    }

    ensure_verdict_header(verdict_path, run_id, "leak-scanner", analyzer_version)?;

    for leak in leaks {
        // Encode the (probe_seq, planted_session, canary) tuple in the
        // model_output_hash so idempotency can detect duplicates.
        let key = format!(
            "leak:{}:{}:{}",
            leak.probe_seq, leak.planted_session, leak.canary
        );
        if existing.contains(&(leak.probe_seq, key.clone())) {
            continue;
        }
        let verdict = VerdictEntry {
            seq: leak.probe_seq,
            verdict: JudgeVerdict::Success,
            confidence: 1.0,
            category: "cross_session_leak".to_owned(),
            tags: vec!["multi_session".to_owned()],
            owasp_ref: Some("LLM02".to_owned()),
            severity: Some("high".to_owned()),
            rationale: format!(
                "Canary {} planted in session {} surfaced in session {}'s probe response.",
                leak.canary, leak.planted_session, leak.probe_session
            ),
            model_output_hash: key,
            model_used: "leak-scanner".to_owned(),
            evaluated_at: iso_now(),
        };
        verdicts::append(verdict_path, &VerdictRecord::Verdict(Box::new(verdict)))?;
    }
    Ok(())
}

fn write_verdict_footer(verdict_path: &Path, run_id: &str) -> anyhow::Result<()> {
    if !verdict_path.exists() {
        return Ok(());
    }
    let records = verdicts::read_all(verdict_path)?;
    let footer =
        verdicts::summarize_footer(run_id, &records, iso_now(), VerdictRunStatus::Completed);
    verdicts::append(verdict_path, &VerdictRecord::Footer(footer))
}

// ── Single-attempt judge ──────────────────────────────────────────────────────

/// Build the `JudgeInput` from an attempt + its prompt metadata.
fn build_judge_input(
    engagement_dir: &Path,
    attempt: &RunAttempt,
    prompt_meta: Option<&PromptMeta>,
    judge_prompt_template: Option<&str>,
) -> JudgeInput {
    let response_text = load_attempt_response_text(engagement_dir, attempt);
    let prompt_text = attempt
        .prompt_text
        .clone()
        .or_else(|| prompt_meta.map(|p| p.prompt_text.clone()))
        .unwrap_or_default();
    JudgeInput {
        prompt_text,
        response_text,
        category: prompt_meta
            .map(|p| p.category.clone())
            .unwrap_or_else(|| attempt.prompt_id.clone()),
        tags: prompt_meta.map(|p| p.tags.clone()).unwrap_or_default(),
        owasp_ref: prompt_meta.and_then(|p| p.owasp_ref.clone()),
        severity: prompt_meta.and_then(|p| p.severity.clone()),
        request_failed: attempt.response.status == 0 || attempt.response.error.is_some(),
        judge_prompt_template: judge_prompt_template.map(str::to_owned),
    }
}

/// Heuristic judge for a single attempt. No LLM required.
pub fn judge_attempt_heuristic(
    engagement_dir: &Path,
    attempt: &RunAttempt,
    prompt_meta: Option<&PromptMeta>,
    judge_prompt_template: Option<&str>,
    evaluated_at: &str,
) -> anyhow::Result<VerdictEntry> {
    let input = build_judge_input(engagement_dir, attempt, prompt_meta, judge_prompt_template);
    let output = judge_heuristic(&input)?;
    Ok(to_verdict_entry(attempt.seq, evaluated_at, &input, output))
}

/// LLM-backed judge for a single attempt. Caller owns the loaded `LlmJudge`.
#[cfg(feature = "runtime")]
pub fn judge_attempt_with_llm(
    engagement_dir: &Path,
    attempt: &RunAttempt,
    prompt_meta: Option<&PromptMeta>,
    judge_prompt_template: Option<&str>,
    evaluated_at: &str,
    judge: &LlmJudge,
) -> anyhow::Result<VerdictEntry> {
    let input = build_judge_input(engagement_dir, attempt, prompt_meta, judge_prompt_template);
    let output = judge_with_llm(&input, judge)?;
    Ok(to_verdict_entry(attempt.seq, evaluated_at, &input, output))
}

/// Judge a single attempt, persisting the verdict. Skips if a verdict already
/// exists and `force` is false.
pub struct JudgeOneOptions<'a> {
    pub engagement_dir: &'a Path,
    pub prompts_dir: &'a Path,
    pub run_id: &'a str,
    pub seq: u32,
    pub judge_prompt_template: Option<&'a str>,
    pub analyzer_version: &'a str,
    pub force: bool,
}

pub fn judge_one_heuristic(opts: &JudgeOneOptions) -> anyhow::Result<JudgeOutcome> {
    let verdict_path = verdict_path_for(opts.engagement_dir, opts.run_id);
    let latest = read_latest_verdicts(&verdict_path)?;

    if let Some(existing) = latest.get(&opts.seq) {
        if !opts.force {
            return Ok(JudgeOutcome::Skipped(existing.clone()));
        }
    }

    let run_path = run_path_for(opts.engagement_dir, opts.run_id);
    let attempt = load_attempt(&run_path, opts.seq)?;
    let prompt_index = build_prompt_index(opts.prompts_dir)?;
    let prompt_meta = prompt_index.get(&attempt.prompt_id);

    let evaluated_at = iso_now();
    let verdict = judge_attempt_heuristic(
        opts.engagement_dir,
        &attempt,
        prompt_meta,
        opts.judge_prompt_template,
        &evaluated_at,
    )?;

    ensure_verdict_header(
        &verdict_path,
        opts.run_id,
        &verdict.model_used,
        opts.analyzer_version,
    )?;
    verdicts::append(
        &verdict_path,
        &VerdictRecord::Verdict(Box::new(verdict.clone())),
    )?;

    Ok(JudgeOutcome::Judged(verdict))
}

pub fn judge_one_hosted(
    opts: &JudgeOneOptions,
    hosted: &HostedJudgeConfig<'_>,
) -> anyhow::Result<JudgeOutcome> {
    let verdict_path = verdict_path_for(opts.engagement_dir, opts.run_id);
    let latest = read_latest_verdicts(&verdict_path)?;

    if let Some(existing) = latest.get(&opts.seq) {
        if !opts.force {
            return Ok(JudgeOutcome::Skipped(existing.clone()));
        }
    }

    let run_path = run_path_for(opts.engagement_dir, opts.run_id);
    let attempt = load_attempt(&run_path, opts.seq)?;
    let prompt_index = build_prompt_index(opts.prompts_dir)?;
    let prompt_meta = prompt_index.get(&attempt.prompt_id);
    let input = build_judge_input(
        opts.engagement_dir,
        &attempt,
        prompt_meta,
        opts.judge_prompt_template,
    );
    let evaluated_at = iso_now();
    let hosted_cfg = build_hosted_config(HostedJudgeConfigInput {
        provider: hosted.provider,
        endpoint: hosted.endpoint,
        deployment: hosted.deployment,
        api_style: hosted.api_style,
        api_version: hosted.api_version,
        api_key: hosted.api_key,
        max_input_chars: hosted.max_input_chars,
        max_output_tokens: hosted.max_output_tokens,
        request_timeout_seconds: hosted.request_timeout_seconds,
        max_retries: hosted.max_retries,
    })?;
    let verdict = to_verdict_entry(
        attempt.seq,
        &evaluated_at,
        &input,
        judge_with_hosted(&input, &hosted_cfg)?,
    );

    ensure_verdict_header(
        &verdict_path,
        opts.run_id,
        &verdict.model_used,
        opts.analyzer_version,
    )?;
    verdicts::append(
        &verdict_path,
        &VerdictRecord::Verdict(Box::new(verdict.clone())),
    )?;

    Ok(JudgeOutcome::Judged(verdict))
}

// ── Run-level orchestration ───────────────────────────────────────────────────

/// Judge every attempt in a run. Picks LLM when `opts.model_path` is set,
/// heuristic otherwise. Writes verdict header (once), every verdict, and a
/// footer at the end. Reports progress via the callback after each attempt.
pub fn judge_run(
    opts: &JudgeRunOptions,
    on_progress: &mut dyn FnMut(Progress),
) -> anyhow::Result<JudgeRunSummary> {
    let run_path = run_path_for(opts.engagement_dir, opts.run_id);
    let attempts = load_attempts(&run_path)?;
    let total = attempts.len() as u32;
    let prompt_index = build_prompt_index(opts.prompts_dir)?;
    let verdict_path = verdict_path_for(opts.engagement_dir, opts.run_id);
    let latest = read_latest_verdicts(&verdict_path)?;

    // Section 1.10 of docs/ToDo.md: surface every cross-session leak as
    // a `cross_session_leak` verdict, auto-flagged without LLM judging.
    // Done before the main dispatch so the verdict header is in place
    // and so a hosted/heuristic re-judge of the same seq doesn't paper
    // over the leak (heuristic judges of the probe attempt still run;
    // the leak verdict is an *additional* signal, not a replacement).
    emit_cross_session_leak_verdicts(
        opts.engagement_dir,
        opts.run_id,
        opts.analyzer_version,
        &verdict_path,
    )?;

    let summary = if let Some(hosted_cfg) = opts.hosted.as_ref() {
        run_hosted(
            opts,
            &verdict_path,
            attempts,
            &prompt_index,
            &latest,
            total,
            hosted_cfg,
            on_progress,
        )?
    } else if let Some(ollama_cfg) = opts.ollama.as_ref() {
        run_ollama(
            opts,
            &verdict_path,
            attempts,
            &prompt_index,
            &latest,
            total,
            ollama_cfg,
            on_progress,
        )?
    } else if opts.model_path.is_some() {
        #[cfg(feature = "runtime")]
        {
            run_llm(
                opts,
                &verdict_path,
                attempts,
                &prompt_index,
                &latest,
                total,
                opts.model_path.unwrap(),
                on_progress,
            )?
        }
        #[cfg(not(feature = "runtime"))]
        {
            return Err(anyhow::anyhow!(
                "analyzer compiled without `runtime` feature; LLM judging unavailable. \
                 Use --ollama-url for the dev sidecar backend, or build with --features runtime."
            ));
        }
    } else {
        run_heuristic(
            opts,
            &verdict_path,
            attempts,
            &prompt_index,
            &latest,
            total,
            on_progress,
        )?
    };

    write_verdict_footer(&verdict_path, opts.run_id)?;
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
fn run_hosted(
    opts: &JudgeRunOptions,
    verdict_path: &Path,
    attempts: Vec<RunAttempt>,
    prompt_index: &HashMap<String, PromptMeta>,
    latest: &HashMap<u32, VerdictEntry>,
    total: u32,
    hosted: &HostedJudgeConfig<'_>,
    on_progress: &mut dyn FnMut(Progress),
) -> anyhow::Result<JudgeRunSummary> {
    let hosted_cfg = build_hosted_config(HostedJudgeConfigInput {
        provider: hosted.provider,
        endpoint: hosted.endpoint,
        deployment: hosted.deployment,
        api_style: hosted.api_style,
        api_version: hosted.api_version,
        api_key: hosted.api_key,
        max_input_chars: hosted.max_input_chars,
        max_output_tokens: hosted.max_output_tokens,
        request_timeout_seconds: hosted.request_timeout_seconds,
        max_retries: hosted.max_retries,
    })?;
    let mut processed = 0u32;
    let mut judged = 0u32;
    let mut skipped_existing = 0u32;

    for attempt in attempts {
        processed += 1;

        if latest.contains_key(&attempt.seq) && !opts.force {
            skipped_existing += 1;
            on_progress(Progress {
                processed,
                total,
                judged,
                skipped_existing,
            });
            continue;
        }

        let prompt_meta = prompt_index.get(&attempt.prompt_id);
        let input = build_judge_input(
            opts.engagement_dir,
            &attempt,
            prompt_meta,
            opts.judge_prompt_template,
        );
        let evaluated_at = iso_now();
        let verdict = to_verdict_entry(
            attempt.seq,
            &evaluated_at,
            &input,
            judge_with_hosted(&input, &hosted_cfg)?,
        );
        ensure_verdict_header(
            verdict_path,
            opts.run_id,
            &verdict.model_used,
            opts.analyzer_version,
        )?;
        verdicts::append(verdict_path, &VerdictRecord::Verdict(Box::new(verdict)))?;
        judged += 1;
        on_progress(Progress {
            processed,
            total,
            judged,
            skipped_existing,
        });
    }

    Ok(JudgeRunSummary {
        processed,
        total,
        judged,
        skipped_existing,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_heuristic(
    opts: &JudgeRunOptions,
    verdict_path: &Path,
    attempts: Vec<RunAttempt>,
    prompt_index: &HashMap<String, PromptMeta>,
    latest: &HashMap<u32, VerdictEntry>,
    total: u32,
    on_progress: &mut dyn FnMut(Progress),
) -> anyhow::Result<JudgeRunSummary> {
    let mut processed = 0u32;
    let mut judged = 0u32;
    let mut skipped_existing = 0u32;
    let mut local_latest = latest.clone();

    for attempt in attempts {
        processed += 1;

        if local_latest.contains_key(&attempt.seq) && !opts.force {
            skipped_existing += 1;
            on_progress(Progress {
                processed,
                total,
                judged,
                skipped_existing,
            });
            continue;
        }

        let prompt_meta = prompt_index.get(&attempt.prompt_id);
        let evaluated_at = iso_now();
        let verdict = judge_attempt_heuristic(
            opts.engagement_dir,
            &attempt,
            prompt_meta,
            opts.judge_prompt_template,
            &evaluated_at,
        )?;
        ensure_verdict_header(
            verdict_path,
            opts.run_id,
            &verdict.model_used,
            opts.analyzer_version,
        )?;
        verdicts::append(
            verdict_path,
            &VerdictRecord::Verdict(Box::new(verdict.clone())),
        )?;
        local_latest.insert(attempt.seq, verdict);
        judged += 1;
        on_progress(Progress {
            processed,
            total,
            judged,
            skipped_existing,
        });
    }

    Ok(JudgeRunSummary {
        processed,
        total,
        judged,
        skipped_existing,
    })
}

/// Ollama-backed run loop. Mirrors `run_llm` but uses HTTP instead of
/// the in-process `LlamaModel`. Lives outside the `runtime` feature
/// gate so it works on hosts without the C++ toolchain.
#[allow(clippy::too_many_arguments)]
fn run_ollama(
    opts: &JudgeRunOptions,
    verdict_path: &Path,
    attempts: Vec<RunAttempt>,
    prompt_index: &HashMap<String, PromptMeta>,
    latest: &HashMap<u32, VerdictEntry>,
    total: u32,
    cfg: &OllamaConfig,
    on_progress: &mut dyn FnMut(Progress),
) -> anyhow::Result<JudgeRunSummary> {
    let judge = OllamaJudge::new(cfg.url, cfg.model)?;
    let mut processed = 0u32;
    let mut judged = 0u32;
    let mut skipped_existing = 0u32;

    for attempt in attempts {
        processed += 1;

        if latest.contains_key(&attempt.seq) && !opts.force {
            skipped_existing += 1;
            on_progress(Progress {
                processed,
                total,
                judged,
                skipped_existing,
            });
            continue;
        }

        let prompt_meta = prompt_index.get(&attempt.prompt_id);
        let evaluated_at = iso_now();
        let input = build_judge_input(
            opts.engagement_dir,
            &attempt,
            prompt_meta,
            opts.judge_prompt_template,
        );
        let output = judge_with_ollama(&input, &judge)?;
        let verdict = to_verdict_entry(attempt.seq, &evaluated_at, &input, output);
        ensure_verdict_header(
            verdict_path,
            opts.run_id,
            &verdict.model_used,
            opts.analyzer_version,
        )?;
        verdicts::append(verdict_path, &VerdictRecord::Verdict(Box::new(verdict)))?;
        judged += 1;
        on_progress(Progress {
            processed,
            total,
            judged,
            skipped_existing,
        });
    }

    Ok(JudgeRunSummary {
        processed,
        total,
        judged,
        skipped_existing,
    })
}

#[cfg(feature = "runtime")]
#[allow(clippy::too_many_arguments)]
fn run_llm(
    opts: &JudgeRunOptions,
    verdict_path: &Path,
    attempts: Vec<RunAttempt>,
    prompt_index: &HashMap<String, PromptMeta>,
    latest: &HashMap<u32, VerdictEntry>,
    total: u32,
    model_path: &Path,
    on_progress: &mut dyn FnMut(Progress),
) -> anyhow::Result<JudgeRunSummary> {
    let judge = LlmJudge::load(model_path)?;
    let mut processed = 0u32;
    let mut judged = 0u32;
    let mut skipped_existing = 0u32;

    for attempt in attempts {
        processed += 1;

        if latest.contains_key(&attempt.seq) && !opts.force {
            skipped_existing += 1;
            on_progress(Progress {
                processed,
                total,
                judged,
                skipped_existing,
            });
            continue;
        }

        let prompt_meta = prompt_index.get(&attempt.prompt_id);
        let evaluated_at = iso_now();
        let verdict = judge_attempt_with_llm(
            opts.engagement_dir,
            &attempt,
            prompt_meta,
            opts.judge_prompt_template,
            &evaluated_at,
            &judge,
        )?;
        ensure_verdict_header(
            verdict_path,
            opts.run_id,
            &verdict.model_used,
            opts.analyzer_version,
        )?;
        verdicts::append(verdict_path, &VerdictRecord::Verdict(Box::new(verdict)))?;
        judged += 1;
        on_progress(Progress {
            processed,
            total,
            judged,
            skipped_existing,
        });
    }

    Ok(JudgeRunSummary {
        processed,
        total,
        judged,
        skipped_existing,
    })
}

// ── Report generation ─────────────────────────────────────────────────────────

pub fn generate_report(engagement_dir: &Path, run_id: &str) -> anyhow::Result<PathBuf> {
    let run_path = run_path_for(engagement_dir, run_id);
    if !run_path.exists() {
        return Err(anyhow::anyhow!("run '{}' not found", run_id));
    }

    let records = read_run_records(&run_path)?;
    let mut started_at = None;
    let mut finished_at = None;
    let mut attempts = Vec::new();

    for record in &records {
        match record {
            RunRecord::Header(h) => started_at = Some(h.started_at.clone()),
            RunRecord::Attempt(a) => attempts.push((**a).clone()),
            RunRecord::Footer(f) => finished_at = Some(f.finished_at.clone()),
            // Phase 5 of plans/multiSessionPlan.md will surface these as
            // cross_session_leak verdicts. Phase 1 just tolerates the
            // variant so the JSONL reader doesn't fail.
            RunRecord::LeakDetected(_) => {}
        }
    }

    let verdict_path = verdict_path_for(engagement_dir, run_id);
    let verdict_records = if verdict_path.exists() {
        verdicts::read_all(&verdict_path)?
    } else {
        Vec::new()
    };
    let latest_verdicts = verdicts::latest_by_seq(&verdict_records);
    let mut verdict_list = latest_verdicts.values().cloned().collect::<Vec<_>>();
    verdict_list.sort_by_key(|v| v.seq);
    let judge_model = verdict_records.iter().find_map(|record| match record {
        VerdictRecord::Header(header) => Some(header.model.clone()),
        _ => None,
    });

    // engagement_slug shown in the report — derive from directory name.
    let engagement_slug = engagement_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("engagement")
        .to_owned();

    let report_attempts = attempts
        .into_iter()
        .map(|attempt| {
            let response_excerpt = load_attempt_response_text(engagement_dir, &attempt);
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

    // Section 3.6: read the triage sidecar (if any) so the Markdown
    // export reflects pentester judgments. Missing sidecar = empty vec
    // = every finding rendered as `unreviewed`. Read errors are
    // non-fatal: the report stays valid without triage data.
    let triage_entries = storage::triage::list_entries(engagement_dir, run_id).unwrap_or_default();

    let report_data = build_report_data(ReportBuildInput {
        engagement_slug,
        run_id: run_id.to_owned(),
        generated_at: iso_now(),
        started_at,
        finished_at,
        judge_model,
        attempts: report_attempts,
        verdicts: verdict_list,
        triage: triage_entries,
    });

    let html = render_html_report(&report_data)?;
    let report_path = report_path_for(engagement_dir, run_id);
    atomic_write(&report_path, html.as_bytes())?;

    // Section 7.1: write the Markdown sibling next to the HTML. Same
    // ReportData, different rendering. Failure to write the .md must not
    // fail the whole report — the HTML is the primary artifact and the
    // .md is a convenience.
    let markdown = render_markdown_report(&report_data);
    let md_path = markdown_report_path_for(engagement_dir, run_id);
    if let Err(err) = atomic_write(&md_path, markdown.as_bytes()) {
        eprintln!("warning: could not write markdown report {}: {err}", md_path.display());
    }

    Ok(report_path)
}

// ── Misc helpers ──────────────────────────────────────────────────────────────

pub fn verdict_label(verdict: &JudgeVerdict) -> &'static str {
    match verdict {
        JudgeVerdict::Success => "SUCCESS",
        JudgeVerdict::Fail => "FAIL",
        JudgeVerdict::Partial => "PARTIAL",
        JudgeVerdict::Unclear => "UNCLEAR",
    }
}

/// ISO-8601 UTC timestamp without external date deps. Mirrors
/// `runner::run::iso_now` byte-for-byte; duplicated here to keep the analyzer
/// crate from depending on the runner crate.
fn iso_now() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();

    let days = secs / 86_400;
    let secs_of_day = secs % 86_400;
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    let (year, month, day) = days_to_ymd(days as i64);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn days_to_ymd(mut days: i64) -> (i32, u32, u32) {
    // Civil-from-days algorithm by Howard Hinnant.
    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = (days - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

// Re-exports for callers — saves them from importing two crates for trivial use.
pub use crate::JudgeOutput;
