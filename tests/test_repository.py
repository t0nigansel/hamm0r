"""Tests for db/repository.py — all use in-memory SQLite."""

from __future__ import annotations

import json
import sqlite3
import uuid

import pytest

from db.repository import (
    Prompt,
    Result,
    Run,
    Target,
    create_prompt,
    create_result,
    create_run,
    create_target,
    delete_prompt,
    get_all_prompts,
    get_all_runs,
    get_all_targets,
    get_prompt,
    get_prompts_by_category,
    get_prompts_by_owasp,
    get_result,
    get_results_by_run,
    get_run,
    get_target,
    get_verdicts_by_result,
    increment_run_completed,
    increment_run_errors,
    init_db,
    open_db,
    update_run_status,
    upsert_prompt,
)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture
def db() -> sqlite3.Connection:
    """Return an initialised in-memory database."""
    conn = open_db(":memory:")
    init_db(conn)
    return conn


def _make_prompt(**overrides) -> Prompt:
    defaults = dict(
        id="A01-001",
        text="Test prompt",
        category="prompt_injection",
        owasp_ref="A01",
        severity="HIGH",
        tags=["direct"],
        mode="single",
        turns=None,
        source="test",
    )
    defaults.update(overrides)
    return Prompt(**defaults)


def _make_target(**overrides) -> Target:
    defaults = dict(
        id=str(uuid.uuid4()),
        name="Test Target",
        url="http://localhost:8080/v1/chat",
        endpoint_type="openai_compat",
        auth_type="none",
    )
    defaults.update(overrides)
    return Target(**defaults)


def _make_run(target_id: str, **overrides) -> Run:
    defaults = dict(
        id=str(uuid.uuid4()),
        target_id=target_id,
        tester_name="tester",
        prompt_set_ids=["A01-001"],
        status="running",
        started_at="2026-03-26T10:00:00+00:00",
    )
    defaults.update(overrides)
    return Run(**defaults)


def _make_result(run_id: str, prompt_id: str = "A01-001", **overrides) -> Result:
    defaults = dict(
        id=str(uuid.uuid4()),
        run_id=run_id,
        prompt_id=prompt_id,
        prompt_text="Test prompt",
        timestamp="2026-03-26T10:00:01+00:00",
        response_text="I cannot comply",
        status_code=200,
        latency_ms=150,
    )
    defaults.update(overrides)
    return Result(**defaults)


# ---------------------------------------------------------------------------
# init_db
# ---------------------------------------------------------------------------

class TestInitDb:
    def test_creates_tables(self, db: sqlite3.Connection):
        tables = {
            row[0]
            for row in db.execute(
                "SELECT name FROM sqlite_master WHERE type='table'"
            ).fetchall()
        }
        assert {"prompts", "targets", "runs", "results", "verdicts"} <= tables

    def test_idempotent(self, db: sqlite3.Connection):
        """Calling init_db twice should not raise."""
        init_db(db)

    def test_foreign_keys_enabled(self, db: sqlite3.Connection):
        fk = db.execute("PRAGMA foreign_keys").fetchone()[0]
        assert fk == 1


# ---------------------------------------------------------------------------
# Prompts
# ---------------------------------------------------------------------------

class TestPrompts:
    def test_create_and_get(self, db: sqlite3.Connection):
        p = _make_prompt()
        create_prompt(db, p)
        fetched = get_prompt(db, "A01-001")
        assert fetched is not None
        assert fetched.id == "A01-001"
        assert fetched.text == "Test prompt"
        assert fetched.severity == "HIGH"
        assert fetched.tags == ["direct"]
        assert fetched.created_at != ""

    def test_get_nonexistent(self, db: sqlite3.Connection):
        assert get_prompt(db, "NOPE") is None

    def test_get_all(self, db: sqlite3.Connection):
        create_prompt(db, _make_prompt(id="A01-001"))
        create_prompt(db, _make_prompt(id="A01-002"))
        create_prompt(db, _make_prompt(id="A06-001", category="data_exfiltration", owasp_ref="A06"))
        assert len(get_all_prompts(db)) == 3

    def test_get_by_category(self, db: sqlite3.Connection):
        create_prompt(db, _make_prompt(id="A01-001"))
        create_prompt(db, _make_prompt(id="A06-001", category="data_exfiltration", owasp_ref="A06"))
        result = get_prompts_by_category(db, "prompt_injection")
        assert len(result) == 1
        assert result[0].id == "A01-001"

    def test_get_by_owasp(self, db: sqlite3.Connection):
        create_prompt(db, _make_prompt(id="A01-001"))
        create_prompt(db, _make_prompt(id="A01-002"))
        create_prompt(db, _make_prompt(id="A06-001", category="data_exfiltration", owasp_ref="A06"))
        result = get_prompts_by_owasp(db, "A01")
        assert len(result) == 2

    def test_upsert_insert(self, db: sqlite3.Connection):
        p = _make_prompt()
        upsert_prompt(db, p)
        fetched = get_prompt(db, "A01-001")
        assert fetched is not None
        assert fetched.text == "Test prompt"

    def test_upsert_update(self, db: sqlite3.Connection):
        create_prompt(db, _make_prompt())
        updated = _make_prompt(text="Updated prompt text")
        upsert_prompt(db, updated)
        fetched = get_prompt(db, "A01-001")
        assert fetched is not None
        assert fetched.text == "Updated prompt text"

    def test_delete(self, db: sqlite3.Connection):
        create_prompt(db, _make_prompt())
        assert delete_prompt(db, "A01-001") is True
        assert get_prompt(db, "A01-001") is None

    def test_delete_nonexistent(self, db: sqlite3.Connection):
        assert delete_prompt(db, "NOPE") is False

    def test_duplicate_id_raises(self, db: sqlite3.Connection):
        create_prompt(db, _make_prompt())
        with pytest.raises(sqlite3.IntegrityError):
            create_prompt(db, _make_prompt())

    def test_tags_roundtrip(self, db: sqlite3.Connection):
        p = _make_prompt(tags=["direct", "role_override", "classic"])
        create_prompt(db, p)
        fetched = get_prompt(db, "A01-001")
        assert fetched is not None
        assert fetched.tags == ["direct", "role_override", "classic"]

    def test_multiturn_roundtrip(self, db: sqlite3.Connection):
        turns = [
            {"role": "user", "content": "Remember this."},
            {"role": "user", "content": "What did I say?"},
        ]
        p = _make_prompt(id="A02-001", mode="multiturn", turns=turns)
        create_prompt(db, p)
        fetched = get_prompt(db, "A02-001")
        assert fetched is not None
        assert fetched.turns == turns


# ---------------------------------------------------------------------------
# Targets
# ---------------------------------------------------------------------------

class TestTargets:
    def test_create_and_get(self, db: sqlite3.Connection):
        t = _make_target()
        create_target(db, t)
        fetched = get_target(db, t.id)
        assert fetched is not None
        assert fetched.name == "Test Target"
        assert fetched.endpoint_type == "openai_compat"

    def test_get_all(self, db: sqlite3.Connection):
        create_target(db, _make_target(name="Alpha"))
        create_target(db, _make_target(name="Beta"))
        assert len(get_all_targets(db)) == 2

    def test_field_mapping_roundtrip(self, db: sqlite3.Connection):
        mapping = {"message": "input", "response": "output"}
        t = _make_target(field_mapping=mapping)
        create_target(db, t)
        fetched = get_target(db, t.id)
        assert fetched is not None
        assert fetched.field_mapping == mapping


# ---------------------------------------------------------------------------
# Runs
# ---------------------------------------------------------------------------

class TestRuns:
    def test_create_and_get(self, db: sqlite3.Connection):
        t = _make_target()
        create_target(db, t)
        r = _make_run(t.id)
        create_run(db, r)
        fetched = get_run(db, r.id)
        assert fetched is not None
        assert fetched.status == "running"
        assert fetched.tester_name == "tester"

    def test_get_all(self, db: sqlite3.Connection):
        t = _make_target()
        create_target(db, t)
        create_run(db, _make_run(t.id))
        create_run(db, _make_run(t.id))
        assert len(get_all_runs(db)) == 2

    def test_update_status(self, db: sqlite3.Connection):
        t = _make_target()
        create_target(db, t)
        r = _make_run(t.id)
        create_run(db, r)
        update_run_status(db, r.id, "completed", finished_at="2026-03-26T11:00:00+00:00")
        fetched = get_run(db, r.id)
        assert fetched is not None
        assert fetched.status == "completed"
        assert fetched.finished_at == "2026-03-26T11:00:00+00:00"

    def test_increment_completed(self, db: sqlite3.Connection):
        t = _make_target()
        create_target(db, t)
        r = _make_run(t.id)
        create_run(db, r)
        increment_run_completed(db, r.id)
        increment_run_completed(db, r.id)
        fetched = get_run(db, r.id)
        assert fetched is not None
        assert fetched.completed == 2

    def test_increment_errors(self, db: sqlite3.Connection):
        t = _make_target()
        create_target(db, t)
        r = _make_run(t.id)
        create_run(db, r)
        increment_run_errors(db, r.id)
        fetched = get_run(db, r.id)
        assert fetched is not None
        assert fetched.errors == 1

    def test_prompt_set_ids_roundtrip(self, db: sqlite3.Connection):
        t = _make_target()
        create_target(db, t)
        ids = ["A01-001", "A01-002", "A06-001"]
        r = _make_run(t.id, prompt_set_ids=ids)
        create_run(db, r)
        fetched = get_run(db, r.id)
        assert fetched is not None
        assert fetched.prompt_set_ids == ids


# ---------------------------------------------------------------------------
# Results
# ---------------------------------------------------------------------------

class TestResults:
    def _setup_run(self, db: sqlite3.Connection) -> tuple[str, str]:
        """Create a target, prompt, and run. Return (run_id, prompt_id)."""
        t = _make_target()
        create_target(db, t)
        p = _make_prompt()
        create_prompt(db, p)
        r = _make_run(t.id)
        create_run(db, r)
        return r.id, p.id

    def test_create_and_get(self, db: sqlite3.Connection):
        run_id, prompt_id = self._setup_run(db)
        res = _make_result(run_id, prompt_id)
        create_result(db, res)
        fetched = get_result(db, res.id)
        assert fetched is not None
        assert fetched.response_text == "I cannot comply"
        assert fetched.status_code == 200

    def test_get_by_run(self, db: sqlite3.Connection):
        run_id, prompt_id = self._setup_run(db)
        create_result(db, _make_result(run_id, prompt_id))
        create_result(db, _make_result(run_id, prompt_id))
        results = get_results_by_run(db, run_id)
        assert len(results) == 2

    def test_error_result(self, db: sqlite3.Connection):
        run_id, prompt_id = self._setup_run(db)
        res = _make_result(
            run_id,
            prompt_id,
            response_text=None,
            status_code=None,
            error_message="Connection timeout",
        )
        create_result(db, res)
        fetched = get_result(db, res.id)
        assert fetched is not None
        assert fetched.response_text is None
        assert fetched.error_message == "Connection timeout"

    def test_foreign_key_enforced(self, db: sqlite3.Connection):
        """Result with a non-existent run_id should fail."""
        res = _make_result("nonexistent-run-id", "A01-001")
        with pytest.raises(sqlite3.IntegrityError):
            create_result(db, res)


# ---------------------------------------------------------------------------
# Verdicts (read-only)
# ---------------------------------------------------------------------------

class TestVerdicts:
    def test_get_empty(self, db: sqlite3.Connection):
        assert get_verdicts_by_result(db, "nonexistent") == []


# ---------------------------------------------------------------------------
# Seed script validation
# ---------------------------------------------------------------------------

class TestSeedValidation:
    """Test that library.yaml passes Pydantic validation."""

    def test_library_yaml_validates(self):
        """Load and validate every entry in the prompt library."""
        import sys
        from pathlib import Path

        sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
        from scripts.seed_prompts import load_and_validate

        library_path = Path(__file__).resolve().parent.parent / "prompts" / "library.yaml"
        entries = load_and_validate(library_path)
        assert len(entries) == 35  # 10 + 5 + 5 + 5 + 5 + 5 as specced

    def test_seed_into_memory_db(self, db: sqlite3.Connection):
        """Seed the full library into an in-memory DB and verify counts."""
        import sys
        from pathlib import Path

        sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
        from scripts.seed_prompts import load_and_validate

        library_path = Path(__file__).resolve().parent.parent / "prompts" / "library.yaml"
        entries = load_and_validate(library_path)

        for entry in entries:
            create_prompt(db, entry.to_prompt())

        all_prompts = get_all_prompts(db)
        assert len(all_prompts) == 35

        # Verify category counts
        assert len(get_prompts_by_owasp(db, "A01")) == 17  # 10 direct + 5 indirect + 2 baseline
        assert len(get_prompts_by_owasp(db, "A06")) == 6   # 5 + 1 baseline
        assert len(get_prompts_by_owasp(db, "A03")) == 6   # 5 + 1 baseline
        assert len(get_prompts_by_owasp(db, "A07")) == 6   # 5 + 1 baseline
