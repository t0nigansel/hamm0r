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
    auth_value: str | None = None
    field_mapping: dict | None = None
    system_prompt: str | None = None
    session_strategy: str = "none"
    session_field: str | None = None
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
    step_order: int | None = None
    session_label: str | None = None


@dataclass
class Scenario:
    id: str
    name: str
    target_id: str | None = None
    sessions: list[str] = field(default_factory=lambda: ["A"])
    tags: list[str] = field(default_factory=list)
    repeat_count: int = 1
    created_at: str = ""
    updated_at: str = ""


@dataclass
class ScenarioStep:
    id: str
    scenario_id: str
    step_order: int
    session: str = "A"
    prompt_id: str | None = None
    prompt_text: str = ""
    delay_ms: int = 0


@dataclass
class Verdict:
    id: str
    result_id: str
    verdict: str
    confidence: float
    reason: str
    model_used: str
    evaluated_at: str


@dataclass
class Finding:
    id: str
    result_id: str
    title: str
    severity: str
    owasp_refs: list[str] = field(default_factory=list)
    notes: str | None = None
    promoted_at: str = ""


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
    """Create all tables if they don't exist yet, and migrate existing ones."""
    schema_sql = _SCHEMA_PATH.read_text()
    conn.executescript(schema_sql)
    _migrate(conn)


def _migrate(conn: sqlite3.Connection) -> None:
    """Add new columns to existing tables (idempotent)."""
    migrations = [
        "ALTER TABLE targets ADD COLUMN session_strategy TEXT NOT NULL DEFAULT 'none'",
        "ALTER TABLE targets ADD COLUMN session_field TEXT",
        "ALTER TABLE targets ADD COLUMN auth_value TEXT",
        "ALTER TABLE results ADD COLUMN step_order INTEGER",
        "ALTER TABLE results ADD COLUMN session_label TEXT",
        "ALTER TABLE scenarios ADD COLUMN sessions TEXT NOT NULL DEFAULT '[\"A\"]'",
        "ALTER TABLE scenarios ADD COLUMN tags TEXT NOT NULL DEFAULT '[]'",
        "ALTER TABLE scenarios ADD COLUMN repeat_count INTEGER NOT NULL DEFAULT 1",
    ]
    for sql in migrations:
        try:
            conn.execute(sql)
        except sqlite3.OperationalError:
            pass  # column already exists


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
        auth_value=row["auth_value"],
        field_mapping=_json_loads_dict(row["field_mapping"]),
        system_prompt=row["system_prompt"],
        session_strategy=row["session_strategy"],
        session_field=row["session_field"],
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
        step_order=row["step_order"],
        session_label=row["session_label"],
    )


def _row_to_scenario(row: sqlite3.Row) -> Scenario:
    return Scenario(
        id=row["id"],
        name=row["name"],
        target_id=row["target_id"],
        sessions=_json_loads_list(row["sessions"]),
        tags=_json_loads_list(row["tags"]),
        repeat_count=row["repeat_count"],
        created_at=row["created_at"],
        updated_at=row["updated_at"],
    )


def _row_to_step(row: sqlite3.Row) -> ScenarioStep:
    return ScenarioStep(
        id=row["id"],
        scenario_id=row["scenario_id"],
        step_order=row["step_order"],
        session=row["session"],
        prompt_id=row["prompt_id"],
        prompt_text=row["prompt_text"],
        delay_ms=row["delay_ms"],
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


def _row_to_finding(row: sqlite3.Row) -> Finding:
    return Finding(
        id=row["id"],
        result_id=row["result_id"],
        title=row["title"],
        severity=row["severity"],
        owasp_refs=_json_loads_list(row["owasp_refs"]),
        notes=row["notes"],
        promoted_at=row["promoted_at"],
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
                                auth_header, auth_value, field_mapping, system_prompt,
                                session_strategy, session_field, notes, created_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (
            target.id,
            target.name,
            target.url,
            target.endpoint_type,
            target.auth_type,
            target.auth_header,
            target.auth_value,
            _json_dumps(target.field_mapping),
            target.system_prompt,
            target.session_strategy,
            target.session_field,
            target.notes,
            now,
        ),
    )
    db.commit()


def update_target(db: sqlite3.Connection, target: Target) -> None:
    """Update an existing target."""
    db.execute(
        """UPDATE targets SET name=?, url=?, endpoint_type=?, auth_type=?,
                              auth_header=?, auth_value=?, field_mapping=?, system_prompt=?,
                              session_strategy=?, session_field=?, notes=?
           WHERE id=?""",
        (
            target.name,
            target.url,
            target.endpoint_type,
            target.auth_type,
            target.auth_header,
            target.auth_value,
            _json_dumps(target.field_mapping),
            target.system_prompt,
            target.session_strategy,
            target.session_field,
            target.notes,
            target.id,
        ),
    )
    db.commit()


def upsert_target(db: sqlite3.Connection, target: Target) -> None:
    """Insert or update a target."""
    existing = get_target(db, target.id)
    if existing:
        update_target(db, target)
    else:
        create_target(db, target)


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
                                error_message, timestamp, step_order, session_label)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
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
            result.step_order,
            result.session_label,
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


def count_results_by_run(db: sqlite3.Connection, run_id: str) -> int:
    """Count how many results exist for a run."""
    row = db.execute(
        "SELECT COUNT(*) AS c FROM results WHERE run_id = ?",
        (run_id,),
    ).fetchone()
    return int(row["c"] if row else 0)


# ---------------------------------------------------------------------------
# Verdicts (read-only from hamm0r's perspective)
# ---------------------------------------------------------------------------

def get_verdicts_by_result(db: sqlite3.Connection, result_id: str) -> list[Verdict]:
    """Fetch verdicts for a given result (written by evaluat0r)."""
    rows = db.execute(
        "SELECT * FROM verdicts WHERE result_id = ? ORDER BY evaluated_at",
        (result_id,),
    ).fetchall()
    return [_row_to_verdict(r) for r in rows]


def count_verdicts_by_run(db: sqlite3.Connection, run_id: str) -> int:
    """Count verdict rows for all results in a run."""
    row = db.execute(
        """SELECT COUNT(*) AS c
           FROM verdicts v
           JOIN results r ON r.id = v.result_id
           WHERE r.run_id = ?""",
        (run_id,),
    ).fetchone()
    return int(row["c"] if row else 0)


def delete_verdicts_by_run(db: sqlite3.Connection, run_id: str) -> int:
    """Delete all verdicts for a run. Returns deleted row count."""
    cursor = db.execute(
        """DELETE FROM verdicts
           WHERE result_id IN (
               SELECT id FROM results WHERE run_id = ?
           )""",
        (run_id,),
    )
    db.commit()
    return cursor.rowcount


# ---------------------------------------------------------------------------
# Scenarios
# ---------------------------------------------------------------------------

def create_scenario(db: sqlite3.Connection, scenario: Scenario) -> None:
    """Insert a new scenario."""
    now = _now_iso()
    db.execute(
        """INSERT INTO scenarios (id, name, target_id, sessions, tags,
                                  repeat_count, created_at, updated_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
        (
            scenario.id,
            scenario.name,
            scenario.target_id,
            _json_dumps(scenario.sessions),
            _json_dumps(scenario.tags),
            scenario.repeat_count,
            now,
            now,
        ),
    )
    db.commit()


def update_scenario(db: sqlite3.Connection, scenario: Scenario) -> None:
    """Update an existing scenario."""
    now = _now_iso()
    db.execute(
        """UPDATE scenarios SET name=?, target_id=?, sessions=?, tags=?,
                                repeat_count=?, updated_at=?
           WHERE id=?""",
        (
            scenario.name,
            scenario.target_id,
            _json_dumps(scenario.sessions),
            _json_dumps(scenario.tags),
            scenario.repeat_count,
            now,
            scenario.id,
        ),
    )
    db.commit()


def get_scenario(db: sqlite3.Connection, scenario_id: str) -> Scenario | None:
    """Fetch a single scenario by ID."""
    row = db.execute("SELECT * FROM scenarios WHERE id = ?", (scenario_id,)).fetchone()
    return _row_to_scenario(row) if row else None


def get_all_scenarios(db: sqlite3.Connection) -> list[Scenario]:
    """Fetch all scenarios ordered by name."""
    rows = db.execute("SELECT * FROM scenarios ORDER BY name").fetchall()
    return [_row_to_scenario(r) for r in rows]


def delete_scenario(db: sqlite3.Connection, scenario_id: str) -> bool:
    """Delete a scenario and its steps (CASCADE)."""
    cursor = db.execute("DELETE FROM scenarios WHERE id = ?", (scenario_id,))
    db.commit()
    return cursor.rowcount > 0


# ---------------------------------------------------------------------------
# Scenario Steps
# ---------------------------------------------------------------------------

def create_step(db: sqlite3.Connection, step: ScenarioStep) -> None:
    """Insert a new scenario step."""
    db.execute(
        """INSERT INTO scenario_steps (id, scenario_id, step_order, session,
                                       prompt_id, prompt_text, delay_ms)
           VALUES (?, ?, ?, ?, ?, ?, ?)""",
        (
            step.id,
            step.scenario_id,
            step.step_order,
            step.session,
            step.prompt_id,
            step.prompt_text,
            step.delay_ms,
        ),
    )
    db.commit()


def update_step(db: sqlite3.Connection, step: ScenarioStep) -> None:
    """Update an existing step."""
    db.execute(
        """UPDATE scenario_steps SET step_order=?, session=?, prompt_id=?,
                                     prompt_text=?, delay_ms=?
           WHERE id=?""",
        (
            step.step_order,
            step.session,
            step.prompt_id,
            step.prompt_text,
            step.delay_ms,
            step.id,
        ),
    )
    db.commit()


def get_steps_by_scenario(db: sqlite3.Connection, scenario_id: str) -> list[ScenarioStep]:
    """Fetch all steps for a scenario, ordered by step_order."""
    rows = db.execute(
        "SELECT * FROM scenario_steps WHERE scenario_id = ? ORDER BY step_order",
        (scenario_id,),
    ).fetchall()
    return [_row_to_step(r) for r in rows]


def delete_step(db: sqlite3.Connection, step_id: str) -> bool:
    """Delete a single step."""
    cursor = db.execute("DELETE FROM scenario_steps WHERE id = ?", (step_id,))
    db.commit()
    return cursor.rowcount > 0


def delete_steps_by_scenario(db: sqlite3.Connection, scenario_id: str) -> int:
    """Delete all steps for a scenario. Returns count deleted."""
    cursor = db.execute("DELETE FROM scenario_steps WHERE scenario_id = ?", (scenario_id,))
    db.commit()
    return cursor.rowcount


# ---------------------------------------------------------------------------
# Findings
# ---------------------------------------------------------------------------

def add_finding(
    db: sqlite3.Connection,
    finding_id: str,
    result_id: str,
    title: str,
    severity: str,
    owasp_refs: list[str],
    notes: str | None = None,
) -> Finding:
    """Insert a new finding promoted from a result."""
    now = _now_iso()
    db.execute(
        """INSERT INTO findings (id, result_id, title, severity, owasp_refs, notes, promoted_at)
           VALUES (?, ?, ?, ?, ?, ?, ?)""",
        (finding_id, result_id, title, severity, _json_dumps(owasp_refs), notes, now),
    )
    db.commit()
    row = db.execute("SELECT * FROM findings WHERE id = ?", (finding_id,)).fetchone()
    return _row_to_finding(row)


def list_findings(db: sqlite3.Connection) -> list[Finding]:
    """Fetch all findings ordered by promoted_at descending."""
    rows = db.execute("SELECT * FROM findings ORDER BY promoted_at DESC").fetchall()
    return [_row_to_finding(r) for r in rows]


def get_finding(db: sqlite3.Connection, finding_id: str) -> Finding | None:
    """Fetch a single finding by ID."""
    row = db.execute("SELECT * FROM findings WHERE id = ?", (finding_id,)).fetchone()
    return _row_to_finding(row) if row else None
