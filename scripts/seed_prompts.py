#!/usr/bin/env python3
"""Load prompts from prompts/library.yaml into data/prompts.json.

Usage:
    python scripts/seed_prompts.py               # insert only (skip existing)
    python scripts/seed_prompts.py --update       # upsert (overwrite existing)
    python scripts/seed_prompts.py --data ./data  # specify data directory
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import yaml
from pydantic import BaseModel, field_validator

_PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(_PROJECT_ROOT))


# ---------------------------------------------------------------------------
# Pydantic validation model
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

    def to_prompt(self) -> "PromptEntry":
        """Return self — callers access .id, .text etc. directly."""
        return self


# ---------------------------------------------------------------------------
# Load + validate
# ---------------------------------------------------------------------------

_LIBRARY_PATH = _PROJECT_ROOT / "prompts" / "library.yaml"


def load_and_validate(path: Path) -> list[PromptEntry]:
    """Load YAML and validate each entry. Exits on validation errors."""
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

    ids = [e.id for e in entries]
    dupes = {x for x in ids if ids.count(x) > 1}
    if dupes:
        print(f"Duplicate prompt IDs: {dupes}", file=sys.stderr)
        sys.exit(1)

    return entries


# ---------------------------------------------------------------------------
# Seed
# ---------------------------------------------------------------------------

def seed(data_dir: Path, update: bool = False) -> None:
    """Write prompts from library.yaml into data/prompts.json."""
    from datetime import datetime, timezone
    from sidecar.store import JsonStore

    entries = load_and_validate(_LIBRARY_PATH)
    store = JsonStore(data_dir)
    existing = store.get_prompts()
    existing_ids = {p["id"]: i for i, p in enumerate(existing)}

    now = datetime.now(timezone.utc).isoformat()
    inserted = updated = skipped = 0

    for entry in entries:
        prompt_dict = {
            "id": entry.id,
            "text": entry.text,
            "category": entry.category,
            "owasp_ref": entry.owasp_ref,
            "severity": entry.severity,
            "tags": entry.tags,
            "mode": entry.mode,
            "turns": entry.turns,
            "source": entry.source or "library",
            "created_at": now,
            "updated_at": now,
        }
        if entry.id in existing_ids:
            if update:
                existing[existing_ids[entry.id]] = {
                    **existing[existing_ids[entry.id]],
                    **prompt_dict,
                    "created_at": existing[existing_ids[entry.id]].get("created_at", now),
                }
                updated += 1
            else:
                skipped += 1
        else:
            existing.append(prompt_dict)
            existing_ids[entry.id] = len(existing) - 1
            inserted += 1

    store.save_prompts(existing)

    if update:
        print(f"Seeded {inserted} new + {updated} updated prompts → {data_dir}/prompts.json")
    else:
        print(f"Seeded {inserted} new, skipped {skipped} existing → {data_dir}/prompts.json")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(description="Seed prompt library into data/prompts.json")
    parser.add_argument(
        "--data",
        type=Path,
        default=_PROJECT_ROOT / "data",
        help="Path to data directory (default: ./data)",
    )
    parser.add_argument(
        "--update",
        action="store_true",
        help="Upsert: overwrite existing prompts instead of skipping them",
    )
    args = parser.parse_args()
    seed(args.data, update=args.update)


if __name__ == "__main__":
    main()
