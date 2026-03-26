"""Command handlers for the sidecar protocol.

Each handler receives (db, params) and returns a JSON-serialisable result.
The __main__ loop dispatches commands to these handlers.
"""

from __future__ import annotations

import asyncio
import csv
import io
import json
import sqlite3
import uuid
from dataclasses import asdict
from datetime import datetime, timezone
from pathlib import Path

from db.repository import (
    Prompt,
    Result,
    Run,
    Target,
    create_prompt,
    create_target,
    delete_prompt,
    get_all_prompts,
    get_all_runs,
    get_all_targets,
    get_prompt,
    get_prompts_by_category,
    get_prompts_by_owasp,
    get_results_by_run,
    get_run,
    get_target,
    init_db,
    open_db,
    upsert_prompt,
    update_run_status,
)
from runner.attack_runner import ProgressEvent, run_attack
from runner.target_config import TargetConfig
from sidecar.protocol import send_event


# ---------------------------------------------------------------------------
# State: managed by the main loop, passed into handlers
# ---------------------------------------------------------------------------

class SidecarState:
    """Mutable state shared across command handlers."""

    def __init__(self) -> None:
        self.db: sqlite3.Connection | None = None
        self.db_path: str | None = None
        self._active_run_task: asyncio.Task | None = None
        self._stop_requested: bool = False


# ---------------------------------------------------------------------------
# Engagement / DB management
# ---------------------------------------------------------------------------

def cmd_create_engagement(state: SidecarState, params: dict) -> dict:
    """Create a new engagement .db file and open it.

    params: {name: str, path: str}
    The path should end in .db. The name is informational.
    """
    db_path = params["path"]
    name = params.get("name", "unnamed")

    if state.db is not None:
        state.db.close()

    state.db = open_db(db_path)
    init_db(state.db)
    state.db_path = db_path
    return {"path": db_path, "name": name}


def cmd_open_db(state: SidecarState, params: dict) -> dict:
    """Open an existing .db file.

    params: {path: str}
    """
    db_path = params["path"]
    if state.db is not None:
        state.db.close()

    state.db = open_db(db_path)
    init_db(state.db)  # safe: CREATE IF NOT EXISTS
    state.db_path = db_path
    return {"path": db_path}


def _require_db(state: SidecarState) -> sqlite3.Connection:
    if state.db is None:
        raise RuntimeError("No database open. Create or open an engagement first.")
    return state.db


# ---------------------------------------------------------------------------
# Prompts
# ---------------------------------------------------------------------------

def _prompt_to_dict(p: Prompt) -> dict:
    return {
        "id": p.id,
        "text": p.text,
        "category": p.category,
        "owasp_ref": p.owasp_ref,
        "severity": p.severity,
        "tags": p.tags,
        "mode": p.mode,
        "turns": p.turns,
        "source": p.source,
        "created_at": p.created_at,
        "updated_at": p.updated_at,
    }


def cmd_list_prompts(state: SidecarState, params: dict) -> list[dict]:
    """List prompts, optionally filtered.

    params: {owasp?: str, category?: str}
    """
    db = _require_db(state)
    owasp = params.get("owasp")
    category = params.get("category")

    if owasp:
        prompts = get_prompts_by_owasp(db, owasp)
    elif category:
        prompts = get_prompts_by_category(db, category)
    else:
        prompts = get_all_prompts(db)

    return [_prompt_to_dict(p) for p in prompts]


def cmd_get_prompt(state: SidecarState, params: dict) -> dict | None:
    """Get a single prompt by ID.

    params: {id: str}
    """
    db = _require_db(state)
    p = get_prompt(db, params["id"])
    return _prompt_to_dict(p) if p else None


def cmd_create_prompt(state: SidecarState, params: dict) -> dict:
    """Create a new prompt.

    params: {id, text, category, owasp_ref, severity, tags?, mode?, turns?, source?}
    """
    db = _require_db(state)
    p = Prompt(
        id=params["id"],
        text=params["text"],
        category=params["category"],
        owasp_ref=params["owasp_ref"],
        severity=params["severity"],
        tags=params.get("tags", []),
        mode=params.get("mode", "single"),
        turns=params.get("turns"),
        source=params.get("source"),
    )
    create_prompt(db, p)
    return _prompt_to_dict(get_prompt(db, p.id))


def cmd_update_prompt(state: SidecarState, params: dict) -> dict:
    """Update an existing prompt (upsert).

    params: same as create_prompt
    """
    db = _require_db(state)
    p = Prompt(
        id=params["id"],
        text=params["text"],
        category=params["category"],
        owasp_ref=params["owasp_ref"],
        severity=params["severity"],
        tags=params.get("tags", []),
        mode=params.get("mode", "single"),
        turns=params.get("turns"),
        source=params.get("source"),
    )
    upsert_prompt(db, p)
    return _prompt_to_dict(get_prompt(db, p.id))


def cmd_delete_prompt(state: SidecarState, params: dict) -> dict:
    """Delete a prompt by ID.

    params: {id: str}
    """
    db = _require_db(state)
    deleted = delete_prompt(db, params["id"])
    return {"deleted": deleted}


def cmd_import_csv(state: SidecarState, params: dict) -> dict:
    """Import prompts from CSV content.

    params: {csv_text: str}
    Expected CSV columns: id, text, category, owasp_ref, severity, tags, mode, source
    Tags should be semicolon-separated in the CSV (commas conflict with CSV format).
    """
    db = _require_db(state)
    reader = csv.DictReader(io.StringIO(params["csv_text"]))

    imported = 0
    errors = []
    for i, row in enumerate(reader):
        try:
            tags_raw = row.get("tags", "")
            tags = [t.strip() for t in tags_raw.split(";") if t.strip()] if tags_raw else []

            p = Prompt(
                id=row["id"],
                text=row["text"],
                category=row["category"],
                owasp_ref=row["owasp_ref"],
                severity=row["severity"],
                tags=tags,
                mode=row.get("mode", "single"),
                source=row.get("source", "csv_import"),
            )
            upsert_prompt(db, p)
            imported += 1
        except Exception as exc:
            errors.append(f"Row {i + 1}: {exc}")

    return {"imported": imported, "errors": errors}


def cmd_seed_library(state: SidecarState, params: dict) -> dict:
    """Seed prompts from library.yaml into the current DB.

    params: {update?: bool}  — if true, upsert; else skip existing
    """
    import sys
    from pathlib import Path as P

    db = _require_db(state)
    project_root = P(__file__).resolve().parent.parent
    sys.path.insert(0, str(project_root))
    from scripts.seed_prompts import load_and_validate

    library_path = project_root / "prompts" / "library.yaml"
    entries = load_and_validate(library_path)

    update = params.get("update", False)
    count = 0
    for entry in entries:
        prompt = entry.to_prompt()
        if update:
            upsert_prompt(db, prompt)
        else:
            try:
                create_prompt(db, prompt)
            except Exception:
                continue
        count += 1

    return {"loaded": count, "total": len(entries)}


# ---------------------------------------------------------------------------
# Targets
# ---------------------------------------------------------------------------

def _target_to_dict(t: Target) -> dict:
    return {
        "id": t.id,
        "name": t.name,
        "url": t.url,
        "endpoint_type": t.endpoint_type,
        "auth_type": t.auth_type,
        "auth_header": t.auth_header,
        "field_mapping": t.field_mapping,
        "system_prompt": t.system_prompt,
        "notes": t.notes,
        "created_at": t.created_at,
    }


def cmd_list_targets(state: SidecarState, params: dict) -> list[dict]:
    db = _require_db(state)
    return [_target_to_dict(t) for t in get_all_targets(db)]


# ---------------------------------------------------------------------------
# Runs + Attack
# ---------------------------------------------------------------------------

def _run_to_dict(r: Run) -> dict:
    return {
        "id": r.id,
        "target_id": r.target_id,
        "tester_name": r.tester_name,
        "prompt_set_ids": r.prompt_set_ids,
        "concurrency": r.concurrency,
        "delay_ms": r.delay_ms,
        "status": r.status,
        "started_at": r.started_at,
        "finished_at": r.finished_at,
        "total_prompts": r.total_prompts,
        "completed": r.completed,
        "errors": r.errors,
        "notes": r.notes,
    }


def _result_to_dict(r: Result) -> dict:
    return {
        "id": r.id,
        "run_id": r.run_id,
        "prompt_id": r.prompt_id,
        "prompt_text": r.prompt_text,
        "response_text": r.response_text,
        "status_code": r.status_code,
        "latency_ms": r.latency_ms,
        "error_message": r.error_message,
        "timestamp": r.timestamp,
    }


def cmd_list_runs(state: SidecarState, params: dict) -> list[dict]:
    db = _require_db(state)
    return [_run_to_dict(r) for r in get_all_runs(db)]


def cmd_get_run(state: SidecarState, params: dict) -> dict | None:
    db = _require_db(state)
    r = get_run(db, params["id"])
    return _run_to_dict(r) if r else None


def cmd_get_results(state: SidecarState, params: dict) -> list[dict]:
    """Get results for a run.

    params: {run_id: str}
    """
    db = _require_db(state)
    return [_result_to_dict(r) for r in get_results_by_run(db, params["run_id"])]


async def cmd_start_run(state: SidecarState, params: dict, req_id: str) -> dict:
    """Start an attack run asynchronously.

    params: {
        name: str, url: str, endpoint_type: str,
        auth_type?: str, auth_value?: str, auth_header?: str,
        field_mapping?: dict, system_prompt?: str,
        tester_name?: str, concurrency?: int, delay_ms?: int,
        owasp?: str, category?: str, prompt_ids?: list[str],
        verify_ssl?: bool
    }
    """
    db = _require_db(state)

    config = TargetConfig(
        name=params["name"],
        url=params["url"],
        endpoint_type=params["endpoint_type"],
        auth_type=params.get("auth_type", "none"),
        auth_value=params.get("auth_value"),
        auth_header=params.get("auth_header"),
        field_mapping=params.get("field_mapping"),
        system_prompt=params.get("system_prompt"),
        tester_name=params.get("tester_name", "default"),
        concurrency=params.get("concurrency", 1),
        delay_ms=params.get("delay_ms", 0),
        verify_ssl=params.get("verify_ssl", True),
    )

    kwargs: dict = {}
    if params.get("prompt_ids"):
        kwargs["prompt_ids"] = params["prompt_ids"]
    elif params.get("owasp"):
        kwargs["owasp_filter"] = params["owasp"]
    elif params.get("category"):
        kwargs["category_filter"] = params["category"]

    def on_progress(event: ProgressEvent) -> None:
        send_event(req_id, "progress", {
            "run_id": event.run_id,
            "completed": event.completed,
            "errors": event.errors,
            "total": event.total,
            "last_prompt_id": event.last_prompt_id,
            "last_prompt_text": event.last_prompt_text,
            "last_response_preview": event.last_response_preview,
            "last_status": event.last_status,
        })

    state._stop_requested = False

    run_id = await run_attack(db, config, on_progress=on_progress, **kwargs)
    run = get_run(db, run_id)
    return _run_to_dict(run) if run else {"run_id": run_id}


def cmd_stop_run(state: SidecarState, params: dict) -> dict:
    """Request graceful stop of the current run.

    The runner checks stop_event and will finish in-flight requests.
    """
    # Decision: for v0.1, we cancel the active run task.
    # In a future version, the runner's stop_event would be exposed directly.
    if state._active_run_task and not state._active_run_task.done():
        state._active_run_task.cancel()
        return {"stopped": True}
    return {"stopped": False}


def cmd_get_run_progress(state: SidecarState, params: dict) -> dict | None:
    """Poll current run status from DB.

    params: {run_id: str}
    Used by the UI to poll progress independently of events.
    """
    db = _require_db(state)
    r = get_run(db, params["run_id"])
    return _run_to_dict(r) if r else None


# ---------------------------------------------------------------------------
# Command registry
# ---------------------------------------------------------------------------

# Sync commands return immediately.
# Async commands (start_run) are handled specially in __main__.
SYNC_COMMANDS: dict[str, callable] = {
    "create_engagement": cmd_create_engagement,
    "open_db": cmd_open_db,
    "list_prompts": cmd_list_prompts,
    "get_prompt": cmd_get_prompt,
    "create_prompt": cmd_create_prompt,
    "update_prompt": cmd_update_prompt,
    "delete_prompt": cmd_delete_prompt,
    "import_csv": cmd_import_csv,
    "seed_library": cmd_seed_library,
    "list_targets": cmd_list_targets,
    "list_runs": cmd_list_runs,
    "get_run": cmd_get_run,
    "get_results": cmd_get_results,
    "stop_run": cmd_stop_run,
    "get_run_progress": cmd_get_run_progress,
}

# Async commands need the event loop and req_id for streaming events.
ASYNC_COMMANDS: dict[str, callable] = {
    "start_run": cmd_start_run,
}
