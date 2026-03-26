#!/usr/bin/env python3
"""Load prompts from prompts/library.yaml into a SQLite database.

Usage:
    python scripts/seed_prompts.py                 # insert only (skip existing)
    python scripts/seed_prompts.py --update         # upsert (overwrite existing)
    python scripts/seed_prompts.py --db path/to.db  # specify DB path

Default DB path: ./promt0r.db
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import yaml
from pydantic import BaseModel, field_validator

# Add project root to path so we can import db.repository
_PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(_PROJECT_ROOT))

from db.repository import Prompt, init_db, open_db, create_prompt, upsert_prompt  # noqa: E402


# ---------------------------------------------------------------------------
# Pydantic validation model for prompt entries loaded from YAML
# ---------------------------------------------------------------------------

class PromptEntry(BaseModel):
    """Validates a single prompt entry from library.yaml."""

    id: str
    text: str
    category: str
    owasp_ref: str
    severity: str
    tags: list[str] = []
    mode: str = "single"
    turns: list[dict[str, str]] | None = None
    source: str | None = None

    @field_validator("id")
    @classmethod
    def id_format(cls, v: str) -> str:
        parts = v.split("-")
        if len(parts) != 2 or not parts[0].startswith("A") or not parts[1].isdigit():
            raise ValueError(f"Prompt ID must be in A{{NN}}-{{NNN}} format, got '{v}'")
        return v

    @field_validator("owasp_ref")
    @classmethod
    def valid_owasp(cls, v: str) -> str:
        valid = {f"A{i:02d}" for i in range(1, 11)}
        if v not in valid:
            raise ValueError(f"owasp_ref must be A01–A10, got '{v}'")
        return v

    @field_validator("severity")
    @classmethod
    def valid_severity(cls, v: str) -> str:
        allowed = {"LOW", "MEDIUM", "HIGH", "CRITICAL"}
        if v not in allowed:
            raise ValueError(f"severity must be one of {allowed}, got '{v}'")
        return v

    @field_validator("category")
    @classmethod
    def valid_category(cls, v: str) -> str:
        allowed = {
            "prompt_injection",
            "memory_poisoning",
            "identity_confusion",
            "privilege_escalation",
            "excessive_agency",
            "data_exfiltration",
            "misleading_content",
            "supply_chain",
            "misinformation",
            "unbounded_consumption",
        }
        if v not in allowed:
            raise ValueError(f"category must be one of {allowed}, got '{v}'")
        return v

    @field_validator("mode")
    @classmethod
    def valid_mode(cls, v: str) -> str:
        if v not in ("single", "multiturn"):
            raise ValueError(f"mode must be 'single' or 'multiturn', got '{v}'")
        return v

    def to_prompt(self) -> Prompt:
        return Prompt(
            id=self.id,
            text=self.text,
            category=self.category,
            owasp_ref=self.owasp_ref,
            severity=self.severity,
            tags=self.tags,
            mode=self.mode,
            turns=self.turns,
            source=self.source,
        )


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

_LIBRARY_PATH = _PROJECT_ROOT / "prompts" / "library.yaml"


def load_and_validate(path: Path) -> list[PromptEntry]:
    """Load YAML and validate each entry with Pydantic."""
    raw = yaml.safe_load(path.read_text())
    if not isinstance(raw, list):
        raise ValueError("library.yaml must be a YAML list of prompt objects")

    entries: list[PromptEntry] = []
    errors: list[str] = []
    for i, item in enumerate(raw):
        try:
            entries.append(PromptEntry(**item))
        except Exception as exc:
            errors.append(f"  Entry {i} (id={item.get('id', '?')}): {exc}")

    if errors:
        print(f"Validation failed for {len(errors)} entries:", file=sys.stderr)
        for e in errors:
            print(e, file=sys.stderr)
        sys.exit(1)

    # Check for duplicate IDs
    ids = [e.id for e in entries]
    dupes = {x for x in ids if ids.count(x) > 1}
    if dupes:
        print(f"Duplicate prompt IDs: {dupes}", file=sys.stderr)
        sys.exit(1)

    return entries


def seed(db_path: Path, update: bool = False) -> None:
    """Seed the database with prompts from library.yaml."""
    entries = load_and_validate(_LIBRARY_PATH)

    conn = open_db(db_path)
    init_db(conn)

    inserted = 0
    updated = 0
    skipped = 0

    for entry in entries:
        prompt = entry.to_prompt()
        if update:
            upsert_prompt(conn, prompt)
            updated += 1
        else:
            try:
                create_prompt(conn, prompt)
                inserted += 1
            except Exception:
                skipped += 1

    conn.close()

    if update:
        print(f"Upserted {updated} prompts into {db_path}")
    else:
        print(f"Inserted {inserted}, skipped {skipped} (already exist) into {db_path}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Seed prompt library into SQLite DB")
    parser.add_argument(
        "--db",
        type=Path,
        default=_PROJECT_ROOT / "promt0r.db",
        help="Path to SQLite database file (default: ./promt0r.db)",
    )
    parser.add_argument(
        "--update",
        action="store_true",
        help="Upsert: update existing prompts instead of skipping them",
    )
    args = parser.parse_args()
    seed(args.db, update=args.update)


if __name__ == "__main__":
    main()
