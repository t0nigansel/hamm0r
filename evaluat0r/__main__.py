"""CLI entry point for evaluat0r.

Usage:
    python -m evaluat0r --db run.db --run <run-id> --output report.pdf

Evaluates all results in the given run, then generates a PDF report.
"""

from __future__ import annotations

import argparse
import asyncio
import sys

from db.repository import get_all_runs, get_run, open_db
from evaluat0r.judge import evaluate_run
from evaluat0r.ollama_client import DEFAULT_BASE_URL, DEFAULT_MODEL, FALLBACK_MODEL, check_ollama_available
from evaluat0r.report import generate_pdf


def _progress(completed: int, total: int, verdicts: dict) -> None:
    """Print evaluation progress to stderr."""
    bar_width = 30
    frac = completed / total if total else 0
    filled = int(bar_width * frac)
    bar = "█" * filled + "░" * (bar_width - filled)
    counts = " | ".join(f"{k}: {v}" for k, v in sorted(verdicts.items()))
    sys.stderr.write(f"\r  [{bar}] {completed}/{total}  {counts}")
    sys.stderr.flush()


def _pick_run(db, run_id: str) -> str:
    """Resolve run ID — if 'latest', pick the most recent run."""
    if run_id != "latest":
        run = get_run(db, run_id)
        if not run:
            print(f"Error: run '{run_id}' not found.", file=sys.stderr)
            sys.exit(1)
        return run_id

    runs = get_all_runs(db)
    if not runs:
        print("Error: no runs found in database.", file=sys.stderr)
        sys.exit(1)
    # Most recent run (last created)
    return runs[-1].id


async def _main(args: argparse.Namespace) -> None:
    db = open_db(args.db)

    run_id = _pick_run(db, args.run)
    run = get_run(db, run_id)
    print(f"evaluat0r v0.1 — evaluating run {run_id[:12]}...")
    print(f"  Target: {run.target_id}")
    print(f"  Tester: {run.tester_name}")
    print()

    # Check Ollama availability
    model = args.model
    if not args.skip_ollama_check:
        print("Checking Ollama availability...", end=" ")
        status = await check_ollama_available(base_url=args.ollama_url, model=model)
        if status["available"]:
            model = status["model"]
            print(f"OK (model: {model})")
        else:
            print(f"WARNING: {status['error']}", file=sys.stderr)
            if not args.heuristics_only:
                print("  Use --heuristics-only to skip LLM evaluation.", file=sys.stderr)
                sys.exit(1)
            print("  Proceeding with heuristics only.")

    # Evaluate
    print("\nEvaluating results...")
    stats = await evaluate_run(
        db, run_id,
        model=model,
        base_url=args.ollama_url,
        skip_heuristics=args.skip_heuristics,
        on_progress=_progress,
    )
    print()  # newline after progress bar

    # Print summary
    print(f"\n  Total results:      {stats['total']}")
    print(f"  Already evaluated:  {stats['skipped_existing']}")
    print(f"  Evaluated now:      {stats['evaluated']}")
    print(f"  Heuristic verdicts: {stats['heuristic_verdicts']}")
    print(f"  LLM verdicts:       {stats['llm_verdicts']}")
    print(f"  Verdicts: {stats['verdicts']}")

    # Generate PDF
    if args.output:
        print(f"\nGenerating report: {args.output}")
        generate_pdf(db, run_id, args.output)
        print(f"  Report saved to: {args.output}")

    db.close()


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="evaluat0r",
        description="Evaluate attack results and generate security reports.",
    )
    parser.add_argument(
        "--db", required=True,
        help="Path to the SQLite engagement database",
    )
    parser.add_argument(
        "--run", default="latest",
        help="Run ID to evaluate (default: latest)",
    )
    parser.add_argument(
        "--output", "-o",
        help="Output PDF report path (optional, skips PDF if not given)",
    )
    parser.add_argument(
        "--model", default=DEFAULT_MODEL,
        help=f"Ollama model name (default: {DEFAULT_MODEL})",
    )
    parser.add_argument(
        "--ollama-url", default=DEFAULT_BASE_URL,
        help=f"Ollama base URL (default: {DEFAULT_BASE_URL})",
    )
    parser.add_argument(
        "--heuristics-only", action="store_true",
        help="Skip LLM judge, use heuristics only (verdicts will be partial)",
    )
    parser.add_argument(
        "--skip-heuristics", action="store_true",
        help="Skip heuristic pre-filter, always use LLM judge",
    )
    parser.add_argument(
        "--skip-ollama-check", action="store_true",
        help="Skip Ollama availability check",
    )
    args = parser.parse_args()

    # Decision: --heuristics-only overrides skip_heuristics and skips LLM entirely.
    # Implemented via skip_ollama_check + a modified evaluate_run that only uses heuristics.
    # For now, heuristics-only just sets skip_ollama_check and lets evaluate_run handle
    # errors from the LLM gracefully (catches exceptions → UNCLEAR verdicts).
    if args.heuristics_only:
        args.skip_ollama_check = True

    asyncio.run(_main(args))


if __name__ == "__main__":
    main()
