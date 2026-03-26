"""CLI entry point: python -m runner --config target.json --db run.db

Loads target config, opens (or creates) the SQLite DB, seeds prompts
if the DB is empty, and runs the async attack runner.
"""

from __future__ import annotations

import argparse
import asyncio
import sys
from pathlib import Path

# Ensure project root is on sys.path so `db` and `runner` are importable
_PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(_PROJECT_ROOT))

from db.repository import get_all_prompts, init_db, open_db  # noqa: E402
from runner.attack_runner import ProgressEvent, run_attack  # noqa: E402
from runner.target_config import load_config  # noqa: E402


def _print_progress(event: ProgressEvent) -> None:
    """Simple CLI progress display."""
    bar_len = 30
    frac = event.completed / event.total if event.total else 0
    filled = int(bar_len * frac)
    bar = "█" * filled + "░" * (bar_len - filled)

    status_icon = "✓" if event.last_status == "ok" else "✗"
    preview = (event.last_response_preview or "")[:60]

    print(
        f"\r[{bar}] {event.completed}/{event.total} "
        f"(err:{event.errors}) {status_icon} {event.last_prompt_id}: {preview}",
        end="",
        flush=True,
    )
    if event.completed == event.total:
        print()  # newline at end


async def _main(args: argparse.Namespace) -> None:
    config = load_config(args.config)

    # Override tester name from CLI if provided
    if args.tester:
        config.tester_name = args.tester

    db = open_db(args.db)
    init_db(db)

    # Check if DB has prompts; if not, suggest seeding
    if not get_all_prompts(db):
        print(
            "No prompts in database. Run 'python scripts/seed_prompts.py "
            f"--db {args.db}' first.",
            file=sys.stderr,
        )
        sys.exit(1)

    print(f"Target: {config.name} ({config.url})")
    print(f"Endpoint: {config.endpoint_type} | Auth: {config.auth_type}")
    print(f"Concurrency: {config.concurrency} | Delay: {config.delay_ms}ms")

    # Build filter kwargs
    kwargs: dict = {}
    if args.owasp:
        kwargs["owasp_filter"] = args.owasp
        print(f"Filter: OWASP {args.owasp}")
    elif args.category:
        kwargs["category_filter"] = args.category
        print(f"Filter: category={args.category}")

    prompt_count = len(get_all_prompts(db))
    print(f"Prompts in DB: {prompt_count}")
    print("─" * 60)
    print("Starting attack run... (Ctrl+C to stop gracefully)")
    print()

    run_id = await run_attack(db, config, on_progress=_print_progress, **kwargs)

    run = None
    from db.repository import get_run
    run = get_run(db, run_id)
    if run:
        print()
        print("─" * 60)
        print(f"Run complete: {run.status}")
        print(f"  ID:        {run.id}")
        print(f"  Completed: {run.completed}/{run.total_prompts}")
        print(f"  Errors:    {run.errors}")
        print(f"  Duration:  {run.started_at} → {run.finished_at}")
        print(f"  DB:        {args.db}")

    db.close()


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="runner",
        description="promt0r attack runner — send prompts to a target LLM",
    )
    parser.add_argument(
        "--config",
        type=Path,
        required=True,
        help="Path to target config JSON file",
    )
    parser.add_argument(
        "--db",
        type=Path,
        required=True,
        help="Path to SQLite database file (created if not exists)",
    )
    parser.add_argument(
        "--tester",
        type=str,
        default=None,
        help="Tester name (overrides config file)",
    )
    parser.add_argument(
        "--owasp",
        type=str,
        default=None,
        help="Filter prompts by OWASP ref (e.g. A01)",
    )
    parser.add_argument(
        "--category",
        type=str,
        default=None,
        help="Filter prompts by category (e.g. prompt_injection)",
    )
    args = parser.parse_args()

    if not args.config.exists():
        print(f"Config file not found: {args.config}", file=sys.stderr)
        sys.exit(1)

    asyncio.run(_main(args))


if __name__ == "__main__":
    main()
