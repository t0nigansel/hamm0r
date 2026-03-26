# Stack

Explicit decisions with reasons. When in doubt, follow this file.

## Python 3.12 (backend)

All business logic, DB access, HTTP runner, and evaluat0r in Python.
Version: 3.12 minimum (use match statements, tomllib, task groups freely).
Package manager: uv (not pip, not poetry).

**Do not use**: Django, Flask, FastAPI — there is no web server in this app.

## Tauri (desktop shell)

Cross-platform desktop wrapper. Rust shell + WebView for the UI.
The Python backend runs as a Tauri sidecar (child process).
Communication: JSON over stdin/stdout between Rust and Python.

Why not Electron: Tauri produces smaller binaries, no Node.js runtime to ship,
better suited for a security tool that should feel lightweight.

Why not a pure Python UI (tkinter, PyQt): Tauri gives a proper native window
and makes it easy to ship a real cross-platform binary.

**UI layer**: plain HTML + CSS + vanilla JS. No React, no Vue, no bundler.
Keep the frontend simple — this is a tool, not a web app.
Use `fetch()` to call Tauri commands. Use CSS variables for theming.

## SQLite via stdlib sqlite3

Single `.db` file per engagement. No installation, no server, no config.
Use `sqlite3` from the Python stdlib directly — no SQLAlchemy, no Tortoise.

Connection settings to always apply:
```python
conn.execute("PRAGMA journal_mode=WAL")    # safe concurrent reads
conn.execute("PRAGMA foreign_keys=ON")     # enforce referential integrity
conn.execute("PRAGMA synchronous=NORMAL")  # safe + fast enough
```

Open one connection per session. Pass it as a parameter. Do not use globals.

## httpx (HTTP runner)

Async HTTP client for sending attack prompts to target systems.
Use `httpx.AsyncClient` with a shared client instance per run.
Set explicit timeouts: `httpx.Timeout(connect=10.0, read=30.0)`.
Follow redirects: yes. Verify SSL: yes by default, configurable off.

**Do not use**: requests (sync), aiohttp (heavier, less ergonomic).

## YAML (prompt library)

Human-readable, git-diffable prompt library format.
Use PyYAML for parsing. Schema validated with pydantic on load.

**Do not use**: TOML (less readable for multi-line strings), JSON (no comments).

## Pydantic v2 (data validation)

Use for validating prompt library on load and target config on save.
Dataclasses for internal runtime objects (no validation needed).

## Ollama + Qwen 2.5 (evaluat0r only)

Local inference. No data leaves the machine.
Target model: `qwen2.5:14b` (good balance of quality and speed on CPU).
Fallback: `qwen2.5:7b` for machines with less RAM.
Communicate via Ollama's OpenAI-compatible local endpoint: `http://localhost:11434/v1`.

**evaluat0r only** — promt0r never calls Ollama.

## WeasyPrint (evaluat0r only)

PDF generation from HTML templates. Same library used in the CV tool.
Report template in `evaluat0r/templates/report.html`.
CSS for print layout in `evaluat0r/templates/report.css`.

## Testing

pytest + pytest-asyncio for async tests.
httpx's built-in `MockTransport` for mocking target responses in runner tests.
No mocking frameworks — use simple fakes and fixtures.
Test DB: in-memory SQLite (`sqlite3.connect(":memory:")`).

## Dependency list (promt0r)

```toml
[project]
requires-python = ">=3.12"
dependencies = [
    "httpx>=0.27",
    "pyyaml>=6.0",
    "pydantic>=2.0",
]

[project.optional-dependencies]
dev = ["pytest", "pytest-asyncio", "ruff"]
```

evaluat0r has its own pyproject.toml with additional deps (WeasyPrint, etc.).

## What is explicitly forbidden

| Thing | Why |
|-------|-----|
| SQLAlchemy / any ORM | Unnecessary complexity, raw SQL is fine |
| Any cloud API in runner | Data residency requirement |
| React / Vue / bundler | Overkill for this UI |
| FastAPI / Flask | No web server needed |
| Threading (use asyncio) | Avoid GIL issues with httpx |
| Global state | Makes testing hard |
| Hardcoded credentials | Use env vars or OS keychain |