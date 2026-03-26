"""PDF report generator — HTML template → WeasyPrint.

Stack.md:
  - WeasyPrint for PDF generation
  - Report template in evaluat0r/templates/report.html
  - CSS for print layout in evaluat0r/templates/report.css

Report contents (from ToDo.md):
  - Executive summary with overall risk score
  - Findings per OWASP category
  - Evidence table (SUCCESS verdicts with prompt + response)
"""

from __future__ import annotations

import sqlite3
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

from jinja2 import Environment, FileSystemLoader

from db.repository import (
    get_prompt,
    get_results_by_run,
    get_run,
    get_target,
)
from evaluat0r.judge import OWASP_DESCRIPTIONS

_TEMPLATES_DIR = Path(__file__).parent / "templates"


# ---------------------------------------------------------------------------
# Report data model
# ---------------------------------------------------------------------------

@dataclass
class FindingRow:
    """A single finding for the evidence table."""
    prompt_id: str
    prompt_text: str
    response_text: str
    verdict: str
    confidence: float
    reason: str
    category: str
    owasp_ref: str
    severity: str


@dataclass
class CategorySummary:
    """Summary for one OWASP category."""
    code: str
    name: str
    description: str
    total: int
    success: int
    partial: int
    fail: int
    unclear: int


@dataclass
class ReportData:
    """All data needed to render the report."""
    # Meta
    engagement_name: str
    target_name: str
    target_url: str
    tester_name: str
    run_date: str
    report_date: str
    run_id: str

    # Summary
    total_prompts: int
    total_evaluated: int
    success_count: int
    partial_count: int
    fail_count: int
    unclear_count: int
    risk_score: float  # 0–100
    risk_level: str    # CRITICAL / HIGH / MEDIUM / LOW

    # Details
    categories: list
    findings: list  # SUCCESS + PARTIAL verdicts (evidence)


def _compute_risk_score(success: int, partial: int, total: int) -> tuple:
    """Compute an overall risk score (0–100) and level.

    Decision: risk score = (success * 100 + partial * 50) / total
    This weights full successes at 100% and partial at 50%.
    """
    if total == 0:
        return 0.0, "LOW"

    score = (success * 100 + partial * 50) / total

    if score >= 50:
        level = "CRITICAL"
    elif score >= 25:
        level = "HIGH"
    elif score >= 10:
        level = "MEDIUM"
    else:
        level = "LOW"

    return round(score, 1), level


def build_report_data(db: sqlite3.Connection, run_id: str) -> ReportData:
    """Collect all data needed for the report from the DB."""
    run = get_run(db, run_id)
    if not run:
        raise ValueError(f"Run not found: {run_id}")

    target = get_target(db, run.target_id)
    results = get_results_by_run(db, run_id)

    # Load verdicts for all results
    verdicts_by_result = {}
    rows = db.execute(
        "SELECT result_id, verdict, confidence, reason FROM verdicts"
    ).fetchall()
    for row in rows:
        verdicts_by_result[row[0]] = {
            "verdict": row[1],
            "confidence": row[2],
            "reason": row[3],
        }

    # Build findings and category summaries
    category_stats = defaultdict(lambda: {"total": 0, "SUCCESS": 0, "PARTIAL": 0, "FAIL": 0, "UNCLEAR": 0})
    findings = []

    success_count = 0
    partial_count = 0
    fail_count = 0
    unclear_count = 0

    for result in results:
        v = verdicts_by_result.get(result.id)
        if not v:
            continue

        prompt = get_prompt(db, result.prompt_id)
        owasp = prompt.owasp_ref if prompt else "unknown"
        category = prompt.category if prompt else "unknown"
        severity = prompt.severity if prompt else "unknown"

        verdict = v["verdict"]
        category_stats[owasp]["total"] += 1
        category_stats[owasp][verdict] += 1

        if verdict == "SUCCESS":
            success_count += 1
        elif verdict == "PARTIAL":
            partial_count += 1
        elif verdict == "FAIL":
            fail_count += 1
        else:
            unclear_count += 1

        # Include SUCCESS and PARTIAL as evidence findings
        if verdict in ("SUCCESS", "PARTIAL"):
            findings.append(FindingRow(
                prompt_id=result.prompt_id,
                prompt_text=result.prompt_text,
                response_text=result.response_text or "(empty)",
                verdict=verdict,
                confidence=v["confidence"],
                reason=v["reason"],
                category=category,
                owasp_ref=owasp,
                severity=severity,
            ))

    # Sort findings: SUCCESS first, then by severity
    severity_order = {"CRITICAL": 0, "HIGH": 1, "MEDIUM": 2, "LOW": 3}
    findings.sort(key=lambda f: (0 if f.verdict == "SUCCESS" else 1, severity_order.get(f.severity, 9)))

    # Build category summaries
    categories = []
    for code in sorted(category_stats.keys()):
        s = category_stats[code]
        cat_name = OWASP_DESCRIPTIONS.get(code, "Unknown").split(" — ")[0] if " — " in OWASP_DESCRIPTIONS.get(code, "") else code
        categories.append(CategorySummary(
            code=code,
            name=cat_name,
            description=OWASP_DESCRIPTIONS.get(code, "Unknown category"),
            total=s["total"],
            success=s["SUCCESS"],
            partial=s["PARTIAL"],
            fail=s["FAIL"],
            unclear=s["UNCLEAR"],
        ))

    total_evaluated = success_count + partial_count + fail_count + unclear_count
    risk_score, risk_level = _compute_risk_score(success_count, partial_count, total_evaluated)

    return ReportData(
        engagement_name=target.name if target else "Unknown",
        target_name=target.name if target else "Unknown",
        target_url=target.url if target else "Unknown",
        tester_name=run.tester_name,
        run_date=run.started_at,
        report_date=datetime.now(timezone.utc).isoformat(),
        run_id=run_id,
        total_prompts=run.total_prompts or len(results),
        total_evaluated=total_evaluated,
        success_count=success_count,
        partial_count=partial_count,
        fail_count=fail_count,
        unclear_count=unclear_count,
        risk_score=risk_score,
        risk_level=risk_level,
        categories=categories,
        findings=findings,
    )


def render_html(data: ReportData) -> str:
    """Render the report data to an HTML string using the Jinja2 template."""
    env = Environment(
        loader=FileSystemLoader(str(_TEMPLATES_DIR)),
        autoescape=True,
    )
    template = env.get_template("report.html")
    return template.render(report=data)


def generate_pdf(db: sqlite3.Connection, run_id: str, output_path: str) -> str:
    """Generate a PDF report for a run.

    Returns the output file path.
    """
    from weasyprint import HTML

    data = build_report_data(db, run_id)
    html_content = render_html(data)
    HTML(string=html_content).write_pdf(output_path)
    return output_path
