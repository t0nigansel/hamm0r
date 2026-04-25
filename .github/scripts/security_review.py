#!/usr/bin/env python3
"""
Security review agent for hamm0r — push-to-main variant.

Runs on every push to main. Reads the diff of the push, loads the rule
catalog, asks Claude to evaluate, and opens a GitHub issue if findings
exist. No issue is opened when the review is clean.

Idempotent: if an issue already exists for the same commit range, it is
updated rather than duplicated.

Does not change code. Does not block anything. Advisory only.

Environment:
    GITHUB_TOKEN          — GitHub token with issues write permission
    GITHUB_REPOSITORY     — "owner/repo"
    GITHUB_SHA            — the commit SHA that was pushed (HEAD)
    GITHUB_BEFORE_SHA     — the previous SHA (from github.event.before)
    ANTHROPIC_API_KEY     — Anthropic API key
    REVIEW_MODEL          — optional, defaults to claude-opus-4-7
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import anthropic
import httpx

ISSUE_LABEL = "security-review"
ISSUE_TITLE_PREFIX = "Security review:"
RULES_PATH = Path(".github/security-rules.md")
CLAUDE_MD_PATH = Path("CLAUDE.md")
DEFAULT_MODEL = "claude-opus-4-7"

FULL_FILE_PATHS = (
    "db/repository.py",
    "db/schema.sql",
    "runner/",
    "sidecar/",
)

NULL_SHA = "0000000000000000000000000000000000000000"


def env(name: str, required: bool = True) -> str:
    value = os.environ.get(name, "")
    if required and not value:
        print(f"ERROR: environment variable {name} is not set", file=sys.stderr)
        sys.exit(1)
    return value


def run(cmd: list[str]) -> str:
    result = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        print(f"Command failed: {' '.join(cmd)}", file=sys.stderr)
        print(result.stderr, file=sys.stderr)
        sys.exit(1)
    return result.stdout


def load_diff(before: str, after: str) -> str:
    return run(["git", "diff", f"{before}..{after}"])


def changed_files(before: str, after: str) -> list[str]:
    out = run(["git", "diff", "--name-only", f"{before}..{after}"])
    return [line.strip() for line in out.splitlines() if line.strip()]


def needs_full_context(path: str) -> bool:
    return any(path.startswith(prefix) or path == prefix for prefix in FULL_FILE_PATHS)


def load_full_files(paths: list[str]) -> dict[str, str]:
    result = {}
    for path in paths:
        if not needs_full_context(path):
            continue
        p = Path(path)
        if p.is_file():
            try:
                result[path] = p.read_text(encoding="utf-8")
            except (UnicodeDecodeError, OSError):
                continue
    return result


def build_prompt(diff: str, full_files: dict[str, str], rules: str, claude_md: str) -> str:
    full_files_block = ""
    if full_files:
        parts = [f"### {path}\n\n```\n{content}\n```" for path, content in full_files.items()]
        full_files_block = "\n\n## Full file contents (security-sensitive paths)\n\n" + "\n\n".join(parts)

    return f"""You are a security reviewer for the hamm0r project. Review the following
diff against the rule catalog below.

## Rules

{rules}

## Project conventions (from CLAUDE.md)

{claude_md}

## Diff

```diff
{diff}
```
{full_files_block}

## Your task

Evaluate the diff against every rule. Report only violations you can defend
with a specific file and line reference. Do not report style preferences,
refactoring opportunities, or anything outside the rule catalog.

Respond with ONLY a JSON object in this exact schema, no prose before or
after:

{{
  "findings": [
    {{
      "rule_id": "hamm0r-001",
      "severity": "HIGH",
      "file": "runner/scenario.py",
      "line_hint": 42,
      "message": "Raw SQL query outside repository layer.",
      "suggestion": "Move this query to db/repository.py as a named method."
    }}
  ],
  "summary": "One sentence describing overall risk posture of this change."
}}

If there are no findings, return:

{{"findings": [], "summary": "No security rule violations detected."}}
"""


def call_claude(prompt: str, model: str) -> dict:
    client = anthropic.Anthropic()
    message = client.messages.create(
        model=model,
        max_tokens=4096,
        messages=[{"role": "user", "content": prompt}],
    )

    text = "".join(block.text for block in message.content if block.type == "text").strip()

    if text.startswith("```"):
        text = text.split("\n", 1)[1] if "\n" in text else text
        if text.endswith("```"):
            text = text.rsplit("```", 1)[0]

    try:
        return json.loads(text)
    except json.JSONDecodeError as e:
        print(f"Claude response was not valid JSON:\n{text}", file=sys.stderr)
        print(f"Parse error: {e}", file=sys.stderr)
        sys.exit(1)


def short_sha(sha: str) -> str:
    return sha[:7] if sha else "unknown"


def format_issue_title(before: str, after: str) -> str:
    return f"{ISSUE_TITLE_PREFIX} {short_sha(before)}..{short_sha(after)}"


def format_issue_body(result: dict, before: str, after: str, repo: str) -> str:
    findings = result.get("findings", [])
    summary = result.get("summary", "")

    compare_url = f"https://github.com/{repo}/compare/{before}...{after}"
    lines = [
        f"**Commit range:** [`{short_sha(before)}..{short_sha(after)}`]({compare_url})",
        "",
        f"**Summary:** {summary}",
        "",
    ]

    severity_order = ["CRITICAL", "HIGH", "MEDIUM", "LOW", "INFO"]
    by_severity: dict[str, list[dict]] = {s: [] for s in severity_order}
    for f in findings:
        sev = f.get("severity", "INFO").upper()
        by_severity.setdefault(sev, []).append(f)

    severity_emoji = {
        "CRITICAL": "🔴",
        "HIGH": "🟠",
        "MEDIUM": "🟡",
        "LOW": "🔵",
        "INFO": "⚪",
    }

    total = len(findings)
    counts = ", ".join(
        f"{len(by_severity[s])} {s.lower()}"
        for s in severity_order
        if by_severity[s]
    )
    lines.append(f"**{total} finding(s):** {counts}")
    lines.append("")

    for sev in severity_order:
        items = by_severity[sev]
        if not items:
            continue
        lines.append(f"### {severity_emoji.get(sev, '')} {sev}")
        lines.append("")
        for f in items:
            rule = f.get("rule_id", "?")
            file_ = f.get("file", "?")
            line = f.get("line_hint", "?")
            msg = f.get("message", "")
            sugg = f.get("suggestion", "")
            lines.append(f"- **[{rule}]** `{file_}:{line}` — {msg}")
            if sugg:
                lines.append(f"  _Suggestion:_ {sugg}")
        lines.append("")

    lines.append("---")
    lines.append("_Advisory review. Close this issue once addressed or acknowledged._")
    return "\n".join(lines)


def find_open_issue(repo: str, token: str, title: str) -> int | None:
    url = f"https://api.github.com/repos/{repo}/issues"
    headers = {"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json"}
    params = {"state": "open", "labels": ISSUE_LABEL, "per_page": 100}
    with httpx.Client(timeout=30.0) as client:
        r = client.get(url, headers=headers, params=params)
        r.raise_for_status()
        for issue in r.json():
            if issue.get("title") == title and "pull_request" not in issue:
                return issue["number"]
    return None


def ensure_label(repo: str, token: str) -> None:
    url = f"https://api.github.com/repos/{repo}/labels"
    headers = {"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json"}
    with httpx.Client(timeout=30.0) as client:
        r = client.post(
            url,
            headers=headers,
            json={"name": ISSUE_LABEL, "color": "d73a4a", "description": "Automated security review finding"},
        )
        if r.status_code not in (201, 422):
            r.raise_for_status()


def upsert_issue(repo: str, token: str, title: str, body: str) -> None:
    headers = {"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json"}
    existing = find_open_issue(repo, token, title)

    with httpx.Client(timeout=30.0) as client:
        if existing:
            url = f"https://api.github.com/repos/{repo}/issues/{existing}"
            r = client.patch(url, headers=headers, json={"body": body})
        else:
            url = f"https://api.github.com/repos/{repo}/issues"
            r = client.post(
                url,
                headers=headers,
                json={"title": title, "body": body, "labels": [ISSUE_LABEL]},
            )
        r.raise_for_status()


def main() -> None:
    token = env("GITHUB_TOKEN")
    repo = env("GITHUB_REPOSITORY")
    after = env("GITHUB_SHA")
    before = env("GITHUB_BEFORE_SHA")
    env("ANTHROPIC_API_KEY")
    model = os.environ.get("REVIEW_MODEL", DEFAULT_MODEL)

    if before == NULL_SHA or not before:
        print("First push to branch — no previous commit to diff against. Skipping.")
        sys.exit(0)

    if not RULES_PATH.is_file():
        print(f"ERROR: rules file missing at {RULES_PATH}", file=sys.stderr)
        sys.exit(1)
    rules = RULES_PATH.read_text(encoding="utf-8")
    claude_md = CLAUDE_MD_PATH.read_text(encoding="utf-8") if CLAUDE_MD_PATH.is_file() else ""

    diff = load_diff(before, after)
    if not diff.strip():
        print("Empty diff — nothing to review.")
        sys.exit(0)

    files = changed_files(before, after)
    full_files = load_full_files(files)

    prompt = build_prompt(diff, full_files, rules, claude_md)
    result = call_claude(prompt, model)

    findings = result.get("findings", [])
    if not findings:
        print("Clean review — no issue opened.")
        sys.exit(0)

    ensure_label(repo, token)
    title = format_issue_title(before, after)
    body = format_issue_body(result, before, after, repo)
    upsert_issue(repo, token, title, body)

    print(f"Opened/updated issue with {len(findings)} finding(s).")


if __name__ == "__main__":
    main()