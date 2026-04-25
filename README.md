# hamm0r

Local-first AI security testing tool for LLM-based systems.

`hamm0r` helps you run OWASP-style attack prompts against AI endpoints, capture and signal-detect responses, promote findings, and export PDF reports — all without any cloud calls or external database server.

## Highlights

- **Local-first**: no cloud calls, no external DB server.
- **Engagement folders**: each engagement is a directory under `data/engagements/<slug>/` containing plain JSON files.
- **Workbench**: fire single prompts, inspect signals, diff responses, promote findings.
- **Scenario builder**: multi-step, multi-session attack flows.
- **OWASP coverage grid**: live coverage view across A01–A10.
- **Mutation engine**: base64, rot13, unicode homoglyphs, role-prefix, emoji-smuggle variants.
- **PDF reports** via WeasyPrint + Jinja2.

## Architecture

```text
ui/          Plain HTML/CSS/JS frontend
sidecar/     Command handlers + dev server bridge
runner/      Async HTTP client, signal detection, mutation generator
evaluat0r/   Verdict judge (Ollama/Qwen) + report templates
prompts/     Curated prompt library (library.yaml)
data/        Runtime data (JSON, gitignored)
scripts/     Seed and install helpers
tests/       pytest suite
```

Data layout:

```text
data/
  prompts.json                          ← shared prompt library
  engagements/
    <slug>/
      meta.json
      targets.json
      runs.json
      results.json
      findings.json
      artifacts/                        ← exported PDFs
```

## Quick Start

### 1. Install

```bash
python -m venv .venv
source .venv/bin/activate
pip install -e .
pip install -e ./evaluat0r   # optional — only needed for PDF export / LLM judging
```

For dev/test tools:

```bash
pip install -e ".[dev]"
```

### 2. Seed the prompt library (recommended)

```bash
python scripts/seed_prompts.py
```

This writes `data/prompts.json` with the built-in OWASP attack library.

### 3. Start the dev server

```bash
python -m sidecar.dev_server
```

Then open: `http://localhost:9274`

Optionally auto-open an engagement on start:

```bash
python -m sidecar.dev_server --engagement acme-chatbot
```

### 4. Create an engagement in the UI

Click **+** in the topbar → give it a name → check "Seed with default prompt library" → **Create**.

## Workflow

1. **Targets** (T sidebar) — add the LLM endpoint you want to test.
2. **Workbench** (W sidebar) — select a target, pick or write a prompt, fire it.
3. Inspect signals (PII, system-prompt leakage, injection echo, internal hostnames).
4. **Promote** interesting results to findings with a severity rating.
5. **Export PDF** from the findings drawer.

## evaluat0r (optional LLM judging)

Requires Ollama with `qwen2.5:14b` (or another model).

```bash
ollama pull qwen2.5:14b
python -m evaluat0r --engagement <slug> --output report.pdf
```

## Tests

```bash
pytest
```

## License

MIT — see [LICENSE](LICENSE).
