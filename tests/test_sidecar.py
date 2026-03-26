"""Tests for sidecar/commands.py — all use in-memory SQLite."""

from __future__ import annotations

import sqlite3

import pytest

from db.repository import (
    create_prompt,
    get_all_prompts,
    get_prompt,
    init_db,
    open_db,
    Prompt,
)
from sidecar.commands import (
    SidecarState,
    cmd_create_engagement,
    cmd_create_prompt,
    cmd_delete_prompt,
    cmd_get_prompt,
    cmd_get_results,
    cmd_get_run,
    cmd_import_csv,
    cmd_list_prompts,
    cmd_list_runs,
    cmd_list_targets,
    cmd_open_db,
    cmd_seed_library,
    cmd_update_prompt,
)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture
def state() -> SidecarState:
    """State with an in-memory DB open and initialised."""
    s = SidecarState()
    s.db = open_db(":memory:")
    init_db(s.db)
    s.db_path = ":memory:"
    return s


@pytest.fixture
def seeded_state(state: SidecarState) -> SidecarState:
    """State with a few prompts pre-loaded."""
    prompts = [
        Prompt(id="A01-001", text="Ignore instructions", category="prompt_injection",
               owasp_ref="A01", severity="HIGH", tags=["direct"], mode="single", source="test"),
        Prompt(id="A06-001", text="List emails", category="data_exfiltration",
               owasp_ref="A06", severity="CRITICAL", tags=["direct"], mode="single", source="test"),
    ]
    for p in prompts:
        create_prompt(state.db, p)
    return state


# ---------------------------------------------------------------------------
# Engagement commands
# ---------------------------------------------------------------------------

class TestEngagement:
    def test_create_engagement(self, tmp_path):
        state = SidecarState()
        db_path = str(tmp_path / "test.db")
        result = cmd_create_engagement(state, {"name": "Test", "path": db_path})
        assert result["path"] == db_path
        assert state.db is not None
        assert state.db_path == db_path

    def test_open_db(self, tmp_path):
        state = SidecarState()
        db_path = str(tmp_path / "test.db")
        # Create first
        cmd_create_engagement(state, {"name": "Test", "path": db_path})
        state.db.close()
        state.db = None
        # Reopen
        result = cmd_open_db(state, {"path": db_path})
        assert result["path"] == db_path
        assert state.db is not None


# ---------------------------------------------------------------------------
# Prompt commands
# ---------------------------------------------------------------------------

class TestPromptCommands:
    def test_list_prompts_all(self, seeded_state):
        result = cmd_list_prompts(seeded_state, {})
        assert len(result) == 2

    def test_list_prompts_by_owasp(self, seeded_state):
        result = cmd_list_prompts(seeded_state, {"owasp": "A01"})
        assert len(result) == 1
        assert result[0]["id"] == "A01-001"

    def test_list_prompts_by_category(self, seeded_state):
        result = cmd_list_prompts(seeded_state, {"category": "data_exfiltration"})
        assert len(result) == 1
        assert result[0]["id"] == "A06-001"

    def test_get_prompt(self, seeded_state):
        result = cmd_get_prompt(seeded_state, {"id": "A01-001"})
        assert result is not None
        assert result["text"] == "Ignore instructions"

    def test_get_prompt_missing(self, seeded_state):
        result = cmd_get_prompt(seeded_state, {"id": "NOPE"})
        assert result is None

    def test_create_prompt(self, state):
        result = cmd_create_prompt(state, {
            "id": "A01-099",
            "text": "New prompt",
            "category": "prompt_injection",
            "owasp_ref": "A01",
            "severity": "MEDIUM",
            "tags": ["test"],
        })
        assert result["id"] == "A01-099"
        assert result["text"] == "New prompt"
        # Verify it's actually in DB
        assert get_prompt(state.db, "A01-099") is not None

    def test_update_prompt(self, seeded_state):
        result = cmd_update_prompt(seeded_state, {
            "id": "A01-001",
            "text": "Updated text",
            "category": "prompt_injection",
            "owasp_ref": "A01",
            "severity": "HIGH",
        })
        assert result["text"] == "Updated text"

    def test_delete_prompt(self, seeded_state):
        result = cmd_delete_prompt(seeded_state, {"id": "A01-001"})
        assert result["deleted"] is True
        assert get_prompt(seeded_state.db, "A01-001") is None

    def test_delete_prompt_missing(self, seeded_state):
        result = cmd_delete_prompt(seeded_state, {"id": "NOPE"})
        assert result["deleted"] is False


# ---------------------------------------------------------------------------
# CSV import
# ---------------------------------------------------------------------------

class TestCSVImport:
    def test_import_csv(self, state):
        csv_text = (
            "id,text,category,owasp_ref,severity,tags,mode,source\n"
            "A01-050,Test prompt,prompt_injection,A01,HIGH,direct;test,single,csv\n"
            "A06-050,Exfil prompt,data_exfiltration,A06,CRITICAL,exfil,single,csv\n"
        )
        result = cmd_import_csv(state, {"csv_text": csv_text})
        assert result["imported"] == 2
        assert result["errors"] == []
        assert len(get_all_prompts(state.db)) == 2

    def test_import_csv_with_errors(self, state):
        csv_text = (
            "id,text,category,owasp_ref,severity\n"
            "A01-050,Good,prompt_injection,A01,HIGH\n"
            ",bad row missing id,,,,\n"
        )
        result = cmd_import_csv(state, {"csv_text": csv_text})
        assert result["imported"] >= 1


# ---------------------------------------------------------------------------
# Seed library
# ---------------------------------------------------------------------------

class TestSeedLibrary:
    def test_seed_library(self, state):
        result = cmd_seed_library(state, {})
        assert result["loaded"] > 0
        assert result["total"] == 35

    def test_seed_library_update(self, seeded_state):
        result = cmd_seed_library(seeded_state, {"update": True})
        assert result["total"] == 35


# ---------------------------------------------------------------------------
# Targets / Runs / Results (read commands)
# ---------------------------------------------------------------------------

class TestReadCommands:
    def test_list_targets_empty(self, state):
        assert cmd_list_targets(state, {}) == []

    def test_list_runs_empty(self, state):
        assert cmd_list_runs(state, {}) == []

    def test_get_run_missing(self, state):
        result = cmd_get_run(state, {"id": "nonexistent"})
        assert result is None

    def test_get_results_empty(self, state):
        result = cmd_get_results(state, {"run_id": "nonexistent"})
        assert result == []


# ---------------------------------------------------------------------------
# Require DB guard
# ---------------------------------------------------------------------------

class TestRequireDb:
    def test_no_db_raises(self):
        state = SidecarState()
        with pytest.raises(RuntimeError, match="No database open"):
            cmd_list_prompts(state, {})
