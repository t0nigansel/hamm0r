"""Scenario runner — executes multi-step, multi-session attack scenarios.

Unlike the attack runner (which fires all prompts concurrently), the scenario
runner executes steps sequentially because ordering matters for multi-step attacks.

Session isolation:
  - none:       stateless, single httpx client
  - cookie:     one httpx client per session (separate cookie jars)
  - header:     shared client, unique session ID sent in a custom header
  - body_field: shared client, session ID injected into request body
"""

from __future__ import annotations

import asyncio
import time
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Optional

import httpx

from db.repository import (
    Result,
    Run,
    Scenario,
    ScenarioStep,
    Target,
    create_result,
    create_run,
    get_prompt,
    get_scenario,
    get_steps_by_scenario,
    get_target,
    increment_run_completed,
    increment_run_errors,
    update_run_status,
)
from runner.http_client import AdapterResponse, create_adapter
from runner.target_config import TargetConfig

import sqlite3


@dataclass
class ScenarioProgressEvent:
    run_id: str
    step_order: int
    session: str
    completed: int
    total: int
    errors: int
    last_status: str  # "ok" | "error"
    last_response_preview: Optional[str] = None


def _target_to_config(target: Target) -> TargetConfig:
    """Convert a DB Target to a runner TargetConfig."""
    return TargetConfig(
        name=target.name,
        url=target.url,
        endpoint_type=target.endpoint_type,
        auth_type=target.auth_type,
        auth_value=None,  # auth_value is stored in auth_header for DB targets
        auth_header=target.auth_header,
        field_mapping=target.field_mapping,
        system_prompt=target.system_prompt,
    )


def _build_timeout() -> httpx.Timeout:
    return httpx.Timeout(connect=10.0, read=30.0, write=30.0, pool=10.0)


async def run_scenario(
    db: sqlite3.Connection,
    scenario_id: str,
    *,
    tester_name: str = "default",
    on_progress: Optional[callable] = None,
    stop_event: Optional[asyncio.Event] = None,
    _transport: Optional[httpx.BaseTransport] = None,
) -> str:
    """Execute a scenario and return the run_id.

    Args:
        db: open SQLite connection
        scenario_id: scenario to execute
        tester_name: who is running this
        on_progress: optional callback(ScenarioProgressEvent)
        stop_event: set this event to request graceful stop
        _transport: inject mock transport for testing
    """
    scenario = get_scenario(db, scenario_id)
    if not scenario:
        raise ValueError(f"Scenario not found: {scenario_id}")
    if not scenario.target_id:
        raise ValueError("Scenario has no target assigned")

    target = get_target(db, scenario.target_id)
    if not target:
        raise ValueError(f"Target not found: {scenario.target_id}")

    steps = get_steps_by_scenario(db, scenario_id)
    if not steps:
        raise ValueError(f"Scenario has no steps: {scenario_id}")

    config = _target_to_config(target)
    total_steps = len(steps) * scenario.repeat_count

    # Create run record
    now = datetime.now(timezone.utc).isoformat()
    run_id = str(uuid.uuid4())
    prompt_ids = [s.prompt_id or f"custom-{s.step_order}" for s in steps]
    create_run(db, Run(
        id=run_id,
        target_id=target.id,
        tester_name=tester_name,
        prompt_set_ids=prompt_ids,
        status="running",
        started_at=now,
        total_prompts=total_steps,
    ))

    if stop_event is None:
        stop_event = asyncio.Event()

    completed = 0
    errors = 0

    try:
        for repeat_i in range(scenario.repeat_count):
            # Create one httpx client per session for isolation
            session_clients = {}
            session_adapters = {}
            session_ids = {}

            for session_name in scenario.sessions:
                sid = str(uuid.uuid4())
                session_ids[session_name] = sid

                extra_headers = dict(config.auth_headers())
                if target.session_strategy == "header" and target.session_field:
                    extra_headers[target.session_field] = sid

                client = httpx.AsyncClient(
                    timeout=_build_timeout(),
                    verify=config.verify_ssl,
                    headers=extra_headers,
                    **({"transport": _transport} if _transport else {}),
                )
                session_clients[session_name] = client
                session_adapters[session_name] = create_adapter(config, client)

            try:
                for step in steps:
                    if stop_event.is_set():
                        break

                    adapter = session_adapters.get(step.session)
                    if not adapter:
                        # Session not in scenario.sessions — skip
                        errors += 1
                        increment_run_errors(db, run_id)
                        continue

                    # Apply delay
                    if step.delay_ms > 0:
                        await asyncio.sleep(step.delay_ms / 1000.0)

                    # Resolve prompt text
                    prompt_text = step.prompt_text
                    prompt_id = step.prompt_id  # None for custom steps

                    # If referencing a library prompt, snapshot the text
                    if step.prompt_id:
                        lib_prompt = get_prompt(db, step.prompt_id)
                        if lib_prompt:
                            prompt_text = lib_prompt.text

                    # Send
                    t0 = time.monotonic()
                    try:
                        resp = await adapter.send(prompt_text)
                    except Exception as exc:
                        resp = AdapterResponse(text=None, status_code=None, error=str(exc))
                    latency_ms = int((time.monotonic() - t0) * 1000)

                    # Store result
                    result_id = str(uuid.uuid4())
                    result_now = datetime.now(timezone.utc).isoformat()
                    create_result(db, Result(
                        id=result_id,
                        run_id=run_id,
                        prompt_id=prompt_id,
                        prompt_text=prompt_text,
                        timestamp=result_now,
                        response_text=resp.text,
                        status_code=resp.status_code,
                        latency_ms=latency_ms,
                        error_message=resp.error,
                        step_order=step.step_order,
                        session_label=step.session,
                    ))

                    if resp.error:
                        errors += 1
                        increment_run_errors(db, run_id)
                    else:
                        increment_run_completed(db, run_id)
                    completed += 1

                    if on_progress:
                        event = ScenarioProgressEvent(
                            run_id=run_id,
                            step_order=step.step_order,
                            session=step.session,
                            completed=completed,
                            total=total_steps,
                            errors=errors,
                            last_status="error" if resp.error else "ok",
                            last_response_preview=(resp.text or "")[:200] if resp.text else None,
                        )
                        on_progress(event)

            finally:
                # Close all session clients
                for client in session_clients.values():
                    await client.aclose()

            if stop_event.is_set():
                break

    finally:
        final_status = "stopped" if stop_event.is_set() else "completed"
        finished = datetime.now(timezone.utc).isoformat()
        update_run_status(db, run_id, final_status, finished_at=finished)

    return run_id
