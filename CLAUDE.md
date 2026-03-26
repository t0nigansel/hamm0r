# promt0r

AI security testing tool for LLM-based systems. Tests against OWASP 
Top 10 for Agentic Applications 2026.

## Modules
- promt0r: UI + attack runner (this repo)
- evaluat0r: Qwen judge + report generator (separate module, same DB)

## Stack
- Python 3.12 backend
- Tauri + HTML/JS frontend
- SQLite (single file per engagement, no ORM, raw sqlite3)
- httpx for async HTTP
- Ollama + Qwen 2.5 for evaluat0r
- WeasyPrint for PDF reports

## Commands
- `python -m promt0r` — start the app
- `python -m pytest` — run tests
- `python scripts/seed_prompts.py` — load starter prompt library

## Key conventions
- All DB access through db/repository.py — never raw SQL outside this file
- Schema lives in db/schema.sql — single source of truth
- promt0r writes to: prompts, targets, runs, results
- evaluat0r writes to: verdicts only
- Never change the results table schema without updating DATAMODEL.md

## What not to do
- No SQLAlchemy, no ORM
- No external database server
- No cloud calls from the runner (data residency)
- Don't invent new tables without checking DATAMODEL.md