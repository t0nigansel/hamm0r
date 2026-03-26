"""Async attack runner — reads prompts from DB, sends to target, writes results.

Architecture.md data flow:
  3. Runner loads selected prompts from DB
  4. Runner sends each prompt to target via httpx (async)
  5. Each response written immediately to results table

Concurrency model (Architecture.md):
  - asyncio + httpx, default concurrency 1, max 10
  - Each completed request triggers an immediate DB write

Error handling (Architecture.md):
  - HTTP errors: record status code, mark ERROR, continue
  - Connection timeout: configurable (default 30s)
  - Graceful stop: finish in-flight, flush DB, write partial run
  - Never lose a result
"""

from __future__ import annotations

import asyncio
import signal
import sqlite3
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone

import httpx

from db.repository import (
    Prompt,
    Result,
    Run,
    create_result,
    create_run,
    create_target,
    get_all_prompts,
    get_prompt,
    get_prompts_by_category,
    get_prompts_by_owasp,
    get_run,
    increment_run_completed,
    increment_run_errors,
    update_run_status,
    Target,
)
from runner.http_client import AdapterResponse, create_adapter
from runner.target_config import TargetConfig


@dataclass
class ProgressEvent:
    """Emitted after each prompt is processed. UI can poll these."""
    run_id: str
    completed: int
    errors: int
    total: int
    last_prompt_id: str
    last_prompt_text: str
    last_response_preview: str | None
    last_status: str  # "ok" | "error"


# Type alias for the progress callback the caller can supply.
# It receives a ProgressEvent after every completed request.
# Accepts both sync and async callables.
from typing import Callable, Union
ProgressCallback = Callable[[ProgressEvent], None] | None


async def run_attack(
    db: sqlite3.Connection,
    config: TargetConfig,
    *,
    prompt_ids: list[str] | None = None,
    owasp_filter: str | None = None,
    category_filter: str | None = None,
    on_progress: ProgressCallback = None,
    _transport: httpx.BaseTransport | None = None,
) -> str:
    """Execute an attack run. Returns the run_id.

    Prompt selection priority:
      1. prompt_ids — explicit list of prompt IDs
      2. owasp_filter — all prompts matching an OWASP ref (e.g. "A01")
      3. category_filter — all prompts matching a category
      4. None of the above — all prompts in the DB
    """

    # ── Select prompts ──────────────────────────────────────────────
    if prompt_ids:
        prompts = [p for pid in prompt_ids if (p := get_prompt(db, pid)) is not None]
    elif owasp_filter:
        prompts = get_prompts_by_owasp(db, owasp_filter)
    elif category_filter:
        prompts = get_prompts_by_category(db, category_filter)
    else:
        prompts = get_all_prompts(db)

    # Skip multiturn prompts — not supported yet (backlog)
    prompts = [p for p in prompts if p.mode == "single"]

    if not prompts:
        raise ValueError("No matching single-shot prompts found in the database.")

    # ── Create target row in DB ─────────────────────────────────────
    target_id = str(uuid.uuid4())
    target = Target(
        id=target_id,
        name=config.name,
        url=config.url,
        endpoint_type=config.endpoint_type,
        auth_type=config.auth_type,
        auth_header=config.auth_header,
        field_mapping=config.field_mapping,
        system_prompt=config.system_prompt,
        notes=config.notes,
    )
    create_target(db, target)

    # ── Create run row ──────────────────────────────────────────────
    run_id = str(uuid.uuid4())
    now = datetime.now(timezone.utc).isoformat()
    run = Run(
        id=run_id,
        target_id=target_id,
        tester_name=config.tester_name,
        prompt_set_ids=[p.id for p in prompts],
        status="running",
        started_at=now,
        concurrency=config.concurrency,
        delay_ms=config.delay_ms,
        total_prompts=len(prompts),
    )
    create_run(db, run)

    # ── Set up SIGINT handling for graceful stop ────────────────────
    stop_event = asyncio.Event()
    loop = asyncio.get_running_loop()

    def _handle_sigint() -> None:
        stop_event.set()

    # Decision: only install SIGINT handler when running in the main thread.
    # In tests (and when called from Tauri sidecar), the caller controls
    # cancellation via asyncio task cancellation instead.
    try:
        loop.add_signal_handler(signal.SIGINT, _handle_sigint)
        signal_installed = True
    except (NotImplementedError, RuntimeError):
        # Windows doesn't support add_signal_handler; also fails in non-main threads
        signal_installed = False

    # ── Build httpx client + adapter ────────────────────────────────
    timeout = httpx.Timeout(
        connect=config.timeout_connect,
        read=config.timeout_read,
        write=config.timeout_read,
        pool=config.timeout_connect,
    )
    client_kwargs: dict = dict(
        timeout=timeout,
        follow_redirects=True,
        verify=config.verify_ssl,
    )
    if _transport is not None:
        # _transport is for testing only — inject a mock transport
        client_kwargs["transport"] = _transport
    async with httpx.AsyncClient(**client_kwargs) as client:
        adapter = create_adapter(config, client)

        # ── Semaphore for concurrency control ───────────────────────
        sem = asyncio.Semaphore(config.concurrency)
        completed_count = 0
        error_count = 0

        async def _process_prompt(prompt: Prompt) -> None:
            nonlocal completed_count, error_count

            if stop_event.is_set():
                return

            async with sem:
                if stop_event.is_set():
                    return

                # Delay between requests (for stealth / rate limiting)
                if config.delay_ms > 0 and completed_count > 0:
                    await asyncio.sleep(config.delay_ms / 1000.0)

                timestamp = datetime.now(timezone.utc).isoformat()
                start_ms = _monotonic_ms()

                resp = await adapter.send(prompt.text)

                latency = _monotonic_ms() - start_ms

                # ── Write result immediately ────────────────────────
                result = Result(
                    id=str(uuid.uuid4()),
                    run_id=run_id,
                    prompt_id=prompt.id,
                    prompt_text=prompt.text,
                    timestamp=timestamp,
                    response_text=resp.text,
                    status_code=resp.status_code,
                    latency_ms=latency,
                    error_message=resp.error,
                )
                create_result(db, result)

                if resp.error:
                    increment_run_errors(db, run_id)
                    error_count += 1
                    status = "error"
                else:
                    status = "ok"

                increment_run_completed(db, run_id)
                completed_count += 1

                # ── Notify progress callback ────────────────────────
                if on_progress is not None:
                    event = ProgressEvent(
                        run_id=run_id,
                        completed=completed_count,
                        errors=error_count,
                        total=len(prompts),
                        last_prompt_id=prompt.id,
                        last_prompt_text=prompt.text[:100],
                        last_response_preview=(resp.text or "")[:200] if resp.text else resp.error,
                        last_status=status,
                    )
                    # Support both sync and async callbacks
                    if asyncio.iscoroutinefunction(on_progress):
                        await on_progress(event)
                    else:
                        on_progress(event)

        # ── Launch all prompt tasks with concurrency control ────────
        tasks = [asyncio.create_task(_process_prompt(p)) for p in prompts]

        # Wait for completion or SIGINT
        done, pending = await asyncio.wait(tasks, return_when=asyncio.ALL_COMPLETED)

        # If stopped early, cancel pending tasks and let in-flight ones finish
        if stop_event.is_set() and pending:
            for t in pending:
                t.cancel()
            await asyncio.gather(*pending, return_exceptions=True)

    # ── Finalise run record ─────────────────────────────────────────
    final_status = "stopped" if stop_event.is_set() else "completed"
    finished_at = datetime.now(timezone.utc).isoformat()
    update_run_status(db, run_id, final_status, finished_at=finished_at)

    if signal_installed:
        loop.remove_signal_handler(signal.SIGINT)

    return run_id


def _monotonic_ms() -> int:
    """Return monotonic time in milliseconds."""
    import time
    return int(time.monotonic() * 1000)
