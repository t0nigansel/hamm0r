# Architecture

## Overview

promt0r is a local desktop security testing tool. It has no backend server,
no cloud dependency, and no internet requirement during a test run.
Everything runs on the tester's machine.

## Module boundaries

```
┌─────────────────────────────────────────────┐
│                  promt0r                     │
│                                             │
│  ┌──────────┐    ┌──────────┐    ┌───────┐  │
│  │  Tauri   │───▶│  runner  │───▶│  DB   │  │
│  │   UI     │    │(Python)  │    │(.db)  │  │
│  └──────────┘    └──────────┘    └───┬───┘  │
└────────────────────────────────────-─│──────┘
                                       │ (shared SQLite file)
┌──────────────────────────────────────│──────┐
│                evaluat0r             │      │
│                                      │      │
│  ┌──────────┐    ┌──────────┐    ┌───┴───┐  │
│  │  report  │◀───│  judge   │◀───│  DB   │  │
│  │  (PDF)   │    │ (Qwen)   │    │(.db)  │  │
│  └──────────┘    └──────────┘    └───────┘  │
└─────────────────────────────────────────────┘
```

## Data flow

1. Tester enters target URL + config in UI
2. UI calls Python backend via Tauri commands
3. Runner loads selected prompts from DB
4. Runner sends each prompt to target via httpx (async)
5. Each response written immediately to results table
6. UI polls DB for progress updates (no websockets needed)
7. Tester opens .db file in evaluat0r when run is complete
8. evaluat0r reads results, runs Qwen judge, writes verdicts
9. evaluat0r generates PDF report from verdicts

## Tauri ↔ Python communication

Tauri's Rust shell spawns the Python backend as a sidecar process.
Communication via stdin/stdout (JSON lines) or a local Unix socket.
The UI is pure HTML/JS/CSS — no React, no bundler required for v0.1.

## Target adapter pattern

Different AI systems expose different APIs. The `http_client.py` module
uses an adapter pattern to normalise them:

```
TargetAdapter (abstract)
├── OpenAICompatAdapter    # /v1/chat/completions — covers most LLM APIs
├── CustomRESTAdapter      # user-defined field mapping
└── RawHTTPAdapter         # full control, user writes request template
```

Each adapter takes a prompt string and returns a response string.
The runner does not care which adapter is in use.

## Prompt delivery modes

**Single-shot** (default): one HTTP request per prompt, stateless.
Covers OWASP A01 (direct injection), A06 (data exfil), A07 (misleading content).

**Multi-turn** (v0.2): sequence of messages in one session.
Needed for A02 (memory poisoning), A04 (privilege escalation across turns).
Each multi-turn sequence defined as a list of messages in the prompt entry.

## Concurrency model

Runner uses asyncio + httpx for concurrent requests.
Default concurrency: 1 (sequential, stealthy, avoids WAF triggers).
Configurable up to 10 parallel requests.
Each completed request triggers an immediate DB write — no batching.

## Error handling

- HTTP errors (4xx, 5xx): record status code in results, mark as ERROR, continue
- Connection timeout: configurable timeout per request (default 30s)
- Graceful stop: finish in-flight request, flush DB, write partial run record
- Never lose a result — write to DB before processing next prompt

## Engagement file model

One SQLite file = one customer engagement.
File named by tester at run creation: `acme-bank-chatbot-2026-04.db`
File is the complete artefact: prompts used, responses, verdicts, report data.
Tester can archive, share with customer, or diff against a later run.