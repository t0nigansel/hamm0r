#!/usr/bin/env python3
"""
PreToolUse hook for hamm0r.

Blocks reads of .env* files (any dotenv variant: .env, .env.local,
.env.prod, .env.test, ...).

Triggered on: Read, Bash, Grep, Glob.

Exit codes:
    0 — allow
    2 — block (stderr is shown to Claude as the reason)
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import PurePosixPath

# Matches .env, .env.local, .env.anything — but not env.py, .environment, etc.
ENV_FILE_RE = re.compile(r"(^|/)\.env(\..+)?$")

# Bash patterns that indicate someone trying to read a dotenv file.
# Keep this list tight — false positives are worse than missing one edge case
# in an interactive security tool.
BASH_READ_PATTERNS = [
    re.compile(r"\bcat\s+[^\s;|&]*\.env(\.[^\s;|&]+)?\b"),
    re.compile(r"\bless\s+[^\s;|&]*\.env(\.[^\s;|&]+)?\b"),
    re.compile(r"\bmore\s+[^\s;|&]*\.env(\.[^\s;|&]+)?\b"),
    re.compile(r"\bhead\s+(-\S+\s+)?[^\s;|&]*\.env(\.[^\s;|&]+)?\b"),
    re.compile(r"\btail\s+(-\S+\s+)?[^\s;|&]*\.env(\.[^\s;|&]+)?\b"),
    re.compile(r"\bgrep\s+[^\s]+\s+[^\s;|&]*\.env(\.[^\s;|&]+)?\b"),
    re.compile(r"\bxxd\s+[^\s;|&]*\.env(\.[^\s;|&]+)?\b"),
    re.compile(r"\bsource\s+[^\s;|&]*\.env(\.[^\s;|&]+)?\b"),
    re.compile(r"\.\s+[^\s;|&]*\.env(\.[^\s;|&]+)?\b"),  # POSIX `. file`
    re.compile(r"<\s*[^\s;|&]*\.env(\.[^\s;|&]+)?\b"),   # redirection from file
]


def is_env_path(path: str) -> bool:
    """Check whether a filesystem path refers to a .env* file."""
    if not path:
        return False
    # Normalize — strip quotes, resolve simple ./ prefixes
    cleaned = path.strip().strip("'\"")
    name = PurePosixPath(cleaned).name
    return bool(ENV_FILE_RE.search(cleaned)) or bool(ENV_FILE_RE.match(f"/{name}"))


def block(reason: str) -> None:
    """Emit a block reason and exit with code 2."""
    print(reason, file=sys.stderr)
    sys.exit(2)


def check_read(tool_input: dict) -> None:
    path = tool_input.get("file_path", "")
    if is_env_path(path):
        block(
            f"BLOCKED: Reading dotenv files is forbidden by repo policy "
            f"(path: {path}).\n"
            f"Reason: .env* files hold secrets. If you need a config value, "
            f"ask the user which env var to document in README, or read it "
            f"via os.environ at runtime — never from the file itself."
        )


def check_grep_glob(tool_input: dict) -> None:
    # Grep's `path` or Glob's `path`/`pattern` could target .env files.
    for key in ("path", "pattern", "include"):
        val = tool_input.get(key, "")
        if isinstance(val, str) and is_env_path(val):
            block(
                f"BLOCKED: Searching inside dotenv files is forbidden "
                f"(target: {val}). Secrets must not be indexed or grepped."
            )


def check_bash(tool_input: dict) -> None:
    command = tool_input.get("command", "")
    if not command:
        return
    for pattern in BASH_READ_PATTERNS:
        if pattern.search(command):
            block(
                f"BLOCKED: Bash command reads a dotenv file.\n"
                f"Command: {command}\n"
                f"Reason: .env* files hold secrets. Use os.environ at "
                f"runtime instead of reading the file directly."
            )


def main() -> None:
    try:
        payload = json.load(sys.stdin)
    except json.JSONDecodeError:
        # If the hook input is malformed, don't block — let Claude Code
        # handle the error itself. Hooks should fail open on their own bugs.
        sys.exit(0)

    tool_name = payload.get("tool_name", "")
    tool_input = payload.get("tool_input", {}) or {}

    if tool_name == "Read":
        check_read(tool_input)
    elif tool_name == "Bash":
        check_bash(tool_input)
    elif tool_name in ("Grep", "Glob"):
        check_grep_glob(tool_input)

    # Any other tool: allow.
    sys.exit(0)


if __name__ == "__main__":
    main()