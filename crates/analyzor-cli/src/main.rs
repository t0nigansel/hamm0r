//! `analyz0r` — standalone CLI wrapping `analyzer::pipeline`.
//!
//! ## Subcommands
//!
//!   analyz0r judge-result    --engagement-dir <P> --prompts-dir <P>
//!                            --run <ID> --seq <N>
//!                            [--model <P>] [--force]
//!
//!   analyz0r judge-run       --engagement-dir <P> --prompts-dir <P>
//!                            --run <ID>
//!                            [--model <P>] [--force]
//!
//!   analyz0r generate-report --engagement-dir <P> --run <ID>
//!
//! ## Stdout contract — NDJSON
//!
//! One JSON object per line. Stable shape; consumers parse line-by-line.
//!
//!   {"event":"progress","processed":N,"total":M,"judged":J,
//!    "skipped_existing":S}
//!
//!   {"event":"result", ...}      // subcommand-specific payload
//!
//!   {"event":"error","message":"..."}
//!
//! Stderr carries human-readable diagnostics; do not parse it.
//!
//! ## Exit codes
//!
//!   0 — success
//!   2 — bad arguments (clap default)
//!   3 — pipeline / analyzer-internal error
//!   4 — reserved for I/O-distinguished errors (not yet emitted)

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use analyzer::pipeline::{
    self, JudgeOneOptions, JudgeRunOptions, Progress,
};
use clap::{Parser, Subcommand};
use serde_json::json;

const ANALYZER_VERSION: &str = env!("CARGO_PKG_VERSION");

const EXIT_OK: u8 = 0;
const EXIT_PIPELINE_ERROR: u8 = 3;

#[derive(Parser, Debug)]
#[command(
    name = "analyz0r",
    version,
    about = "Local LLM-judge for hamm0r run artifacts",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Judge a single attempt by sequence number.
    JudgeResult(JudgeResultArgs),
    /// Judge every attempt in a run.
    JudgeRun(JudgeRunArgs),
    /// Render the HTML report for an already-judged run.
    GenerateReport(GenerateReportArgs),
}

#[derive(Parser, Debug)]
struct JudgeResultArgs {
    #[arg(long)]
    engagement_dir: PathBuf,
    #[arg(long)]
    prompts_dir: PathBuf,
    #[arg(long)]
    run: String,
    #[arg(long)]
    seq: u32,
    /// LLM model file. Reserved — currently judge-result is heuristic only.
    #[arg(long)]
    model: Option<PathBuf>,
    #[arg(long)]
    force: bool,
}

#[derive(Parser, Debug)]
struct JudgeRunArgs {
    #[arg(long)]
    engagement_dir: PathBuf,
    #[arg(long)]
    prompts_dir: PathBuf,
    #[arg(long)]
    run: String,
    /// Path to a `.gguf` model file. If omitted, the heuristic judge is used.
    #[arg(long)]
    model: Option<PathBuf>,
    #[arg(long)]
    force: bool,
}

#[derive(Parser, Debug)]
struct GenerateReportArgs {
    #[arg(long)]
    engagement_dir: PathBuf,
    #[arg(long)]
    run: String,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli.command) {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(err) => {
            emit_error(&err);
            eprintln!("analyz0r: {err:#}");
            ExitCode::from(EXIT_PIPELINE_ERROR)
        }
    }
}

fn dispatch(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        Command::JudgeResult(args) => run_judge_result(args),
        Command::JudgeRun(args) => run_judge_run(args),
        Command::GenerateReport(args) => run_generate_report(args),
    }
}

// ── judge-result ──────────────────────────────────────────────────────────────

fn run_judge_result(args: JudgeResultArgs) -> anyhow::Result<()> {
    if args.model.is_some() {
        anyhow::bail!(
            "judge-result --model is reserved; LLM single-result judging is \
             not yet wired. Omit --model to use the heuristic judge."
        );
    }

    let outcome = pipeline::judge_one_heuristic(&JudgeOneOptions {
        engagement_dir: &args.engagement_dir,
        prompts_dir: &args.prompts_dir,
        run_id: &args.run,
        seq: args.seq,
        analyzer_version: ANALYZER_VERSION,
        force: args.force,
    })?;

    let entry = outcome.entry();
    let status = if outcome.was_judged() { "judged" } else { "skipped" };

    emit(json!({
        "event": "result",
        "status": status,
        "run_id": args.run,
        "seq": entry.seq,
        "verdict": pipeline::verdict_label(&entry.verdict),
        "confidence": entry.confidence,
        "reason": entry.rationale,
        "model_used": entry.model_used,
        "evaluated_at": entry.evaluated_at,
    }))
}

// ── judge-run ─────────────────────────────────────────────────────────────────

fn run_judge_run(args: JudgeRunArgs) -> anyhow::Result<()> {
    let run_id = args.run.clone();
    let mut on_progress = |p: Progress| {
        // Best-effort emission; ignore broken-pipe etc. so the run keeps going.
        let _ = emit(json!({
            "event": "progress",
            "processed": p.processed,
            "total": p.total,
            "judged": p.judged,
            "skipped_existing": p.skipped_existing,
        }));
    };

    let summary = pipeline::judge_run(
        &JudgeRunOptions {
            engagement_dir: &args.engagement_dir,
            prompts_dir: &args.prompts_dir,
            run_id: &run_id,
            model_path: args.model.as_deref(),
            analyzer_version: ANALYZER_VERSION,
            force: args.force,
        },
        &mut on_progress,
    )?;

    emit(json!({
        "event": "result",
        "run_id": run_id,
        "processed": summary.processed,
        "total": summary.total,
        "judged": summary.judged,
        "skipped_existing": summary.skipped_existing,
    }))
}

// ── generate-report ───────────────────────────────────────────────────────────

fn run_generate_report(args: GenerateReportArgs) -> anyhow::Result<()> {
    let report_path = pipeline::generate_report(&args.engagement_dir, &args.run)?;
    emit(json!({
        "event": "result",
        "run_id": args.run,
        "report_path": report_path.to_string_lossy(),
    }))
}

// ── NDJSON emission ───────────────────────────────────────────────────────────

fn emit(value: serde_json::Value) -> anyhow::Result<()> {
    let line = serde_json::to_string(&value)?;
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(line.as_bytes())?;
    handle.write_all(b"\n")?;
    handle.flush()?;
    Ok(())
}

fn emit_error(err: &anyhow::Error) {
    let _ = emit(json!({
        "event": "error",
        "message": format!("{err:#}"),
    }));
}
