"""JSON file store for hamm0r.

Directory layout:
    data/
        prompts.json
        engagements/
            <slug>/
                meta.json
                targets.json
                scenarios.json   (steps stored inline)
                results.json
                findings.json
                runs.json
"""
from __future__ import annotations

import json
import re
from datetime import datetime, timezone
from pathlib import Path
from threading import Lock

_lock = Lock()


class JsonStore:
    def __init__(self, data_dir: str | Path) -> None:
        self.data_dir = Path(data_dir)
        self.data_dir.mkdir(parents=True, exist_ok=True)
        (self.data_dir / "engagements").mkdir(exist_ok=True)
        self.active_slug: str | None = None

    # ── Low-level I/O ────────────────────────────────────────────────

    def _path(self, *parts: str) -> Path:
        return self.data_dir.joinpath(*parts)

    def _read_list(self, *parts: str) -> list:
        p = self._path(*parts)
        if not p.exists():
            return []
        with _lock:
            data = json.loads(p.read_text(encoding="utf-8"))
            return data if isinstance(data, list) else []

    def _read_dict(self, *parts: str) -> dict:
        p = self._path(*parts)
        if not p.exists():
            return {}
        with _lock:
            data = json.loads(p.read_text(encoding="utf-8"))
            return data if isinstance(data, dict) else {}

    def _write(self, data: list | dict, *parts: str) -> None:
        p = self._path(*parts)
        p.parent.mkdir(parents=True, exist_ok=True)
        with _lock:
            p.write_text(
                json.dumps(data, indent=2, ensure_ascii=False),
                encoding="utf-8",
            )

    # ── Engagement management ────────────────────────────────────────

    @staticmethod
    def slugify(name: str) -> str:
        slug = name.lower().strip()
        slug = re.sub(r"[^\w\s-]", "", slug)
        slug = re.sub(r"[\s_-]+", "-", slug)
        return slug.strip("-") or "engagement"

    def open_engagement(self, slug: str, name: str | None = None) -> dict:
        eng_dir = self._path("engagements", slug)
        eng_dir.mkdir(parents=True, exist_ok=True)
        meta_path = self._path("engagements", slug, "meta.json")
        if meta_path.exists():
            meta = self._read_dict("engagements", slug, "meta.json")
        else:
            meta = {
                "slug": slug,
                "name": name or slug,
                "created_at": datetime.now(timezone.utc).isoformat(),
            }
            self._write(meta, "engagements", slug, "meta.json")
        self.active_slug = slug
        return meta

    def list_engagements(self) -> list[dict]:
        eng_dir = self._path("engagements")
        result = []
        if not eng_dir.exists():
            return result
        for d in sorted(eng_dir.iterdir()):
            if d.is_dir():
                meta = self._read_dict("engagements", d.name, "meta.json")
                if meta:
                    result.append(meta)
        return result

    def require_engagement(self) -> str:
        if not self.active_slug:
            raise RuntimeError(
                "No engagement open. Create or open an engagement first."
            )
        return self.active_slug

    def engagement_dir(self) -> Path:
        return self._path("engagements", self.require_engagement())

    # ── Prompts (global, shared across engagements) ──────────────────

    def get_prompts(self) -> list[dict]:
        return self._read_list("prompts.json")

    def save_prompts(self, prompts: list[dict]) -> None:
        self._write(prompts, "prompts.json")

    # ── Targets ──────────────────────────────────────────────────────

    def get_targets(self) -> list[dict]:
        slug = self.require_engagement()
        return self._read_list("engagements", slug, "targets.json")

    def save_targets(self, targets: list[dict]) -> None:
        slug = self.require_engagement()
        self._write(targets, "engagements", slug, "targets.json")

    # ── Scenarios (steps stored inline) ─────────────────────────────

    def get_scenarios(self) -> list[dict]:
        slug = self.require_engagement()
        return self._read_list("engagements", slug, "scenarios.json")

    def save_scenarios(self, scenarios: list[dict]) -> None:
        slug = self.require_engagement()
        self._write(scenarios, "engagements", slug, "scenarios.json")

    # ── Runs ─────────────────────────────────────────────────────────

    def get_runs(self) -> list[dict]:
        slug = self.require_engagement()
        return self._read_list("engagements", slug, "runs.json")

    def save_runs(self, runs: list[dict]) -> None:
        slug = self.require_engagement()
        self._write(runs, "engagements", slug, "runs.json")

    def append_run(self, run: dict) -> None:
        runs = self.get_runs()
        runs.append(run)
        self.save_runs(runs)

    def update_run(self, run_id: str, **fields) -> dict | None:
        runs = self.get_runs()
        for run in runs:
            if run["id"] == run_id:
                run.update(fields)
                self.save_runs(runs)
                return run
        return None

    # ── Results ──────────────────────────────────────────────────────

    def get_results(self) -> list[dict]:
        slug = self.require_engagement()
        return self._read_list("engagements", slug, "results.json")

    def save_results(self, results: list[dict]) -> None:
        slug = self.require_engagement()
        self._write(results, "engagements", slug, "results.json")

    def append_result(self, result: dict) -> None:
        results = self.get_results()
        results.append(result)
        self.save_results(results)

    def get_result(self, result_id: str) -> dict | None:
        return next((r for r in self.get_results() if r["id"] == result_id), None)

    def update_result(self, result_id: str, **fields) -> dict | None:
        results = self.get_results()
        for result in results:
            if result.get("id") == result_id:
                result.update(fields)
                self.save_results(results)
                return result
        return None

    # ── Findings ─────────────────────────────────────────────────────

    def get_findings(self) -> list[dict]:
        slug = self.require_engagement()
        return self._read_list("engagements", slug, "findings.json")

    def save_findings(self, findings: list[dict]) -> None:
        slug = self.require_engagement()
        self._write(findings, "engagements", slug, "findings.json")

    def append_finding(self, finding: dict) -> None:
        findings = self.get_findings()
        findings.append(finding)
        self.save_findings(findings)
