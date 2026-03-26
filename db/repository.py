"""Database repository — all DB access goes through this module.

Never use raw SQL outside this file.  See CLAUDE.md and Datamodel.md.
"""

from __future__ import annotations

import json
import sqlite3
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path

_SCHEMA_PATH = Path(__file__).parent / "schema.sql"


# ---------------------------------------------------------------------------
# Dataclasses
# ---------------------------------------------------------------------------

@dataclass
class Prompt:
    id: str
    text: str
    category: str
    owasp_ref: str
    severity: str
    tags: list[str] = field(default_factory=list)
    mode: str = "single"
    turns: list[dict] | None = None
    source: str | None = None
    created_at: str = ""
    updated_at: str = ""


@dataclass
class Target:
    id: str
    name: str
    url: str
    endpoint_type: str
    auth_type: str = "none"
    auth_header: str | None = None
    field_mapping: dict | None = None
    system_prompt: str | None = None
    notes: str | None = None
    created_at: str = ""


@dataclass
class Run:
    id: str
    target_id: str
    tester_name: str
    prompt_set_ids: list[str]
    status: str
    started_at: str
    concurrency: int = 1
    delay_ms: int = 0
    finished_at: str | None = None
    total_prompts: int | None = None
    completed: int = 0
    errors: int = 0
    notes: str | None = None


@dataclass
class Result:
    id: str
    run_id: str
    prompt_id: str
    prompt_text: str
    timestamp: str
    response_text: str | None = None
    status_code: int | None = None
    latency_ms: int | None = None
    error_message: str | None = None


@dataclass
class Verdict:
    id: str
    result_id: str
    verdict: str
    confidence: float
    reason: str
    model_used: str
    evaluated_at: str


# ---------------------------------------------------------------------------
# Connection helpers
# ---------------------------------------------------------------------------

def open_db(path: str | Path) -> sqlite3.Connection:
    """Open a SQLite connection with the project's standard PRAGMAs."""
    conn = sqlite3.connect(str(path))
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA foreign_keys=ON")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.row_factory = sqlite3.Row
    return conn


def init_db(conn: sqlite3.Connection) -> None:
    """Create all tables if they don't exist yet."""
    schema_sql = _SCHEMA_PATH.read_text()
    conn.executescript(schema_sql)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def _json_dumps(obj: object) -> str | None:
    if obj is None:
        return None
    return json.dumps(obj)


def _json_loads_list(raw: str | None) -> list:
    if not raw:
        return []
    return json.loads(raw)


def _json_loads_dict(raw: str | None) -> dict | None:
    if not raw:
        return None
    return json.loads(raw)


def _row_to_prompt(row: sqlite3.Row) -> Prompt:
    return Prompt(
        id=row["id"],
        text=row["text"],
        category=row["category"],
        owasp_ref=row["owasp_ref"],
        severity=row["severity"],
        tags=_json_loads_list(row["tags"]),
        mode=row["mode"],
        turns=_json_loads_list(row["turns"]) or None,
        source=row["source"],
        created_at=row["created_at"],
        updated_at=row["updated_at"],
    )


def _row_to_target(row: sqlite3.Row) -> Target:
    return Target(
        id=row["id"],
        name=row["name"],
        url=row["url"],
        endpoint_type=row["endpoint_type"],
        auth_type=row["auth_type"],
        auth_header=row["auth_header"],
        field_mapping=_json_loads_dict(row["field_mapping"]),
        system_prompt=row["system_prompt"],
        notes=row["notes"],
        created_at=row["created_at"],
    )


def _row_to_run(row: sqlite3.Row) -> Run:
    return Run(
        id=row["id"],
        target_id=row["target_id"],
        tester_name=row["tester_name"],
        prompt_set_ids=_json_loads_list(row["prompt_set_ids"]),
        status=row["status"],
        started_at=row["started_at"],
        concurrency=row["concurrency"],
        delay_ms=row["delay_ms"],
        finished_at=row["finished_at"],
        total_prompts=row["total_prompts"],
        completed=row["completed"],
        errors=row["errors"],
        notes=row["notes"],
    )


def _row_to_result(row: sqlite3.Row) -> Result:
    return Result(
        id=row["id"],
        run_id=row["run_id"],
        prompt_id=row["prompt_id"],
        prompt_text=row["prompt_text"],
        timestamp=row["timestamp"],
        response_text=row["response_text"],
        status_code=row["status_code"],
        latency_ms=row["latency_ms"],
        error_message=row["error_message"],
    )


def _row_to_verdict(row: sqlite3.Row) -> Verdict:
    return Verdict(
        id=row["id"],
        result_id=row["result_id"],
        verdict=row["verdict"],
        confidence=row["confidence"],
        reason=row["reason"],
        model_used=row["model_used"],
        evaluated_at=row["evaluated_at"],
    )


# ---------------------------------------------------------------------------
# Prompts
# ---------------------------------------------------------------------------

def create_prompt(db: sqlite3.Connection, prompt: Prompt) -> None:
    """Insert a new prompt."""
    now = _now_iso()
    db.execute(
        """INSERT INTO prompts (id, text, category, owasp_ref, severity,
                                tags, mode, turns, source, created_at, updated_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (
            prompt.id,
            prompt.text,
            prompt.category,
            prompt.owasp_ref,
            prompt.severity,
            _json_dumps(prompt.tags),
            prompt.mode,
            _json_dumps(prompt.turns),
            prompt.source,
            now,
            now,
        ),
    )
    db.commit()


def upsert_prompt(db: sqlite3.Connection, prompt: Prompt) -> None:
    """Insert or update a prompt (used by seed script with --update)."""
    now = _now_iso()
    db.execute(
        """INSERT INTO prompts (id, text, category, owasp_ref, severity,
                                tags, mode, turns, source, created_at, updated_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(id) DO UPDATE SET
               text=excluded.text,
               category=excluded.category,
               owasp_ref=excluded.owasp_ref,
               severity=excluded.severity,
               tags=excluded.tags,
               mode=excluded.mode,
               turns=excluded.turns,
               source=excluded.source,
               updated_at=excluded.updated_at""",
        (
            prompt.id,
            prompt.text,
            prompt.category,
            prompt.owasp_ref,
            prompt.severity,
            _json_dumps(prompt.tags),
            prompt.mode,
            _json_dumps(prompt.turns),
            prompt.source,
            now,
            now,
        ),
    )
    db.commit()


def get_prompt(db: sqlite3.Connection, prompt_id: str) -> Prompt | None:
    """Fetch a single prompt by ID."""
    row = db.execute("SELECT * FROM prompts WHERE id = ?", (prompt_id,)).fetchone()
    return _row_to_prompt(row) if row else None


def get_all_prompts(db: sqlite3.Connection) -> list[Prompt]:
    """Fetch all prompts ordered by ID."""
    rows = db.execute("SELECT * FROM prompts ORDER BY id").fetchall()
    return [_row_to_prompt(r) for r in rows]


def get_prompts_by_category(db: sqlite3.Connection, category: str) -> list[Prompt]:
    """Fetch prompts filtered by category."""
    rows = db.execute(
        "SELECT * FROM prompts WHERE category = ? ORDER BY id", (category,)
    ).fetchall()
    return [_row_to_prompt(r) for r in rows]


def get_prompts_by_owasp(db: sqlite3.Connection, owasp_ref: str) -> list[Prompt]:
    """Fetch prompts filtered by OWASP reference code."""
    rows = db.execute(
        "SELECT * FROM prompts WHERE owasp_ref = ? ORDER BY id", (owasp_ref,)
    ).fetchall()
    return [_row_to_prompt(r) for r in rows]


def delete_prompt(db: sqlite3.Connection, prompt_id: str) -> bool:
    """Delete a prompt by ID. Returns True if a row was deleted."""
    cursor = db.execute("DELETE FROM prompts WHERE id = ?", (prompt_id,))
    db.commit()
    return cursor.rowcount > 0


# ---------------------------------------------------------------------------
# Targets
# ---------------------------------------------------------------------------

def create_target(db: sqlite3.Connection, target: Target) -> None:
    """Insert a new target."""
    now = _now_iso()
    db.execute(
        """INSERT INTO targets (id, name, url, endpoint_type, auth_type,
                                auth_header, field_mapping, system_prompt,
                                notes, created_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (
            target.id,
            target.name,
            target.url,
            target.endpoint_type,
            target.auth_type,
            target.auth_header,
            _json_dumps(target.field_mapping),
            target.system_prompt,
            target.notes,
            now,
        ),
    )
    db.commit()


def get_target(db: sqlite3.Connection, target_id: str) -> Target | None:
    """Fetch a single target by ID."""
    row = db.execute("SELECT * FROM targets WHERE id = ?", (target_id,)).fetchone()
    return _row_to_target(row) if row else None


def get_all_targets(db: sqlite3.Connection) -> list[Target]:
    """Fetch all targets."""
    rows = db.execute("SELECT * FROM targets ORDER BY name").fetchall()
    return [_row_to_target(r) for r in rows]


# ---------------------------------------------------------------------------
# Runs
# ---------------------------------------------------------------------------

def create_run(db: sqlite3.Connection, run: Run) -> None:
    """Insert a new run."""
    db.execute(
        """INSERT INTO runs (id, target_id, tester_name, prompt_set_ids,
                             concurrency, delay_ms, status, started_at,
                             finished_at, total_prompts, completed, errors, notes)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (
            run.id,
            run.target_id,
            run.tester_name,
            _json_dumps(run.prompt_set_ids),
            run.concurrency,
            run.delay_ms,
            run.status,
            run.started_at,
            run.finished_at,
            run.total_prompts,
            run.completed,
            run.errors,
            run.notes,
        ),
    )
    db.commit()


def get_run(db: sqlite3.Connection, run_id: str) -> Run | None:
    """Fetch a single run by ID."""
    row = db.execute("SELECT * FROM runs WHERE id = ?", (run_id,)).fetchone()
    return _row_to_run(row) if row else None


def get_all_runs(db: sqlite3.Connection) -> list[Run]:
    """Fetch all runs ordered by start time descending."""
    rows = db.execute("SELECT * FROM runs ORDER BY started_at DESC").fetchall()
    return [_row_to_run(r) for r in rows]


def update_run_status(
    db: sqlite3.Connection, run_id: str, status: str, *, finished_at: str | None = None
) -> None:
    """Update a run's status (and optionally finished_at)."""
    if finished_at:
        db.execute(
            "UPDATE runs SET status = ?, finished_at = ? WHERE id = ?",
            (status, finished_at, run_id),
        )
    else:
        db.execute("UPDATE runs SET status = ? WHERE id = ?", (status, run_id))
    db.commit()


def increment_run_completed(db: sqlite3.Connection, run_id: str) -> None:
    """Increment the completed counter for a run."""
    db.execute("UPDATE runs SET completed = completed + 1 WHERE id = ?", (run_id,))
    db.commit()


def increment_run_errors(db: sqlite3.Connection, run_id: str) -> None:
    """Increment the errors counter for a run."""
    db.execute("UPDATE runs SET errors = errors + 1 WHERE id = ?", (run_id,))
    db.commit()


# ---------------------------------------------------------------------------
# Results
# ---------------------------------------------------------------------------

def create_result(db: sqlite3.Connection, result: Result) -> None:
    """Insert a new result. Written immediately after each request — never batch."""
    db.execute(
        """INSERT INTO results (id, run_id, prompt_id, prompt_text,
                                response_text, status_code, latency_ms,
                                error_message, timestamp)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (
            result.id,
            result.run_id,
            result.prompt_id,
            result.prompt_text,
            result.response_text,
            result.status_code,
            result.latency_ms,
            result.error_message,
            result.timestamp,
        ),
    )
    db.commit()


def get_result(db: sqlite3.Connection, result_id: str) -> Result | None:
    """Fetch a single result by ID."""
    row = db.execute("SELECT * FROM results WHERE id = ?", (result_id,)).fetchone()
    return _row_to_result(row) if row else None


def get_results_by_run(db: sqlite3.Connection, run_id: str) -> list[Result]:
    """Fetch all results for a given run."""
    rows = db.execute(
        "SELECT * FROM results WHERE run_id = ? ORDER BY timestamp", (run_id,)
    ).fetchall()
    return [_row_to_result(r) for r in rows]


# ---------------------------------------------------------------------------
# Verdicts (read-only from promt0r's perspective)
# ---------------------------------------------------------------------------

def get_verdicts_by_result(db: sqlite3.Connection, result_id: str) -> list[Verdict]:
    """Fetch verdicts for a given result (written by evaluat0r)."""
    rows = db.execute(
        "SELECT * FROM verdicts WHERE result_id = ? ORDER BY evaluated_at",
        (result_id,),
    ).fetchall()
    return [_row_to_verdict(r) for r in rows]
