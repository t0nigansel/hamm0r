# CLAUDE.md — hamm0r

Working instructions for Claude (Code / Agents) in this repo. This file is the
source of truth for working style, conventions, and invariants. If code and
this document disagree: ask, don't silently fix.

## Before anything else

Read `ProductVision.md`. It is the north star. When a feature, refactor, or
decision feels ambiguous, the vision decides — not this file, not the code,
not the task description. If the vision and a task conflict, the task is
wrong.

## Project

hamm0r is a local-first desktop tool for security testing of LLM-based
systems. The user configures a target, fires a curated attack library, and
gets a report. No cloud accounts, no YAML configs, no framework setup.

Benchmark: OWASP Top 10 for LLM Applications 2025 + Top 10 for Agentic
Applications 2026.

## Two modes

hamm0r ships as two components. Claude must respect this boundary in every
change.

### Core (always present, in the installer)

- Runner, UI, request builder, attack library, recording
- Writes run artifacts (append-only JSONL + response files) to the
  engagement folder on disk
- **Contains no LLM runtime, no local model, no analyzer code paths**
- Fully usable on its own — raw responses are the product in core-only mode

### Analyzer (opt-in, downloaded on activation)

- Local LLM runtime + model file, fetched from a manifest on first
  activation
- Reads the run artifacts produced by core, writes verdict files next
  to them, generates reports
- Lives in a separate module that core does not import
- Absence of the analyzer must never break core

**Invariant:** a fresh install with no analyzer activated must be able to
run a full attack, record responses, and let the user export raw data.
Nothing in core may depend on `crates/analyzer/` or a model runtime.

## Modules

- `crates/hamm0r` — Tauri desktop shell + Rust command layer (core)
- `crates/runner` — async execution engine for targets and scenarios (core)
- `crates/storage` — file I/O layer and on-disk contracts (core)
- `crates/analyzer` — judge + report generator (analyzer, separate module)
- Shared contract: the engagement folder on disk. The run JSONL file
  produced by core is the handoff to the analyzer.

## Stack

- Rust + Tauri 2 + HTML/JS/CSS (vanilla, no framework)
- Files on disk as source of truth: YAML for prompts and requests,
  append-only JSONL for run logs, plain files for responses. No
  database in core. See `Datamodel.md`.
- `reqwest` / Rust async HTTP in the runner
- Local LLM runtime for analyzer (model is replaceable, see
  `Architecture.md`)
- Rust test tooling via `cargo test`

## Project structure

- `crates/hamm0r/` — active Tauri desktop app
- `crates/runner/` — async attack/scenario execution (core)
- `crates/storage/` — file I/O layer for prompts, requests, engagements
- `crates/analyzer/` — judge + report tooling (analyzer)
- `ui/` — frontend (core)
- `prompts/` — bundled starter attack library, copied into the user
  folder on first run
- `tests/` — tests for all layers
- User's on-disk layout (not part of this repo, documented in
  `Datamodel.md`): `~/hamm0r/prompts/`, `~/hamm0r/requests/`,
  `~/hamm0r/engagements/<slug>/`

## How Claude works here

### Before any task

1. Read `ProductVision.md`. Check whether the task aligns with it.
2. Read the relevant spec file: `Architecture.md`, `Datamodel.md`,
   `PromptsSpec.md`, `Stack.md`. These are binding.
3. Read the affected files in full before changing them. Do not speculate
   about code you have not read.
4. Check whether the task touches an invariant (see below). If yes: ask
   before changing.

### While implementing

1. Small diffs. One commit = one thought. Three focused commits beat one
   sprawling one.
2. Tests belong in the same change as the code. No "tests will follow
   later".
3. Comments in English, identifiers in English.
4. No new dependencies without justification. Why this one, why not the
   standard library?
5. No `eval()`, no `exec()`, no `os.system()`. Subprocesses only via
   `subprocess.run()` with explicit argument lists.

### Definition of Done

A change is done when:

- [ ] Code implemented and locally runnable
- [ ] Tests present and passing (`cargo test --workspace`)
- [ ] Eval suite passing (see `tests/eval/` — the golden tasks, when they
      exist)
- [ ] Affected spec files updated (Architecture.md, Datamodel.md,
      PromptsSpec.md — whichever was touched)
- [ ] ProductVision.md not contradicted
- [ ] No new warnings from `cargo clippy`
- [ ] Commit description: What, Why, How tested
- [ ] For new attack types: entry in `prompts/library.yaml` with OWASP
      mapping

## Invariants — do not change without asking

These are architecture and product, not style. If a task violates them, the
task is misstated — not the rule.

### Product invariants

1. **Core must work without the analyzer.** No dependency on
   `crates/analyzer/` in the default core flow. No assumed model at runtime.
2. **No cloud call in the default workflow.** The user must be able to run
   hamm0r in an air-gapped environment without the product losing its
   primary function.
3. **No YAML or config file required for the standard user journey.** If a
   feature needs configuration, it belongs in the UI.
4. **The five-minute promise holds.** Download → first report under five
   minutes on a standard workload. Features that break this promise need
   an explicit product decision.
5. **Responses are user data.** They are never sent off-device, never
   logged in plaintext, never cached outside the engagement DB.

### Technical invariants

6. **File I/O goes only through the `storage/` layer.** No direct
   `open()`, `pathlib.read_text()`, or YAML parsing of user artifacts
   anywhere else in the codebase. The storage layer is the single
   enforcement point for paths, encoding, and atomic writes. Exception:
   tests that use the storage layer's own helpers.
7. **The run JSONL file is the handoff contract** between core and
   analyzer. Its schema is defined in `Datamodel.md`. Changes require:
   update in `Datamodel.md`, a migration path for existing files, and
   an explicit note in the commit.
8. **The Tauri command layer never talks to the LLM directly.** The
   app shell calls runner/storage logic, and runner calls the target.
   This layering is deliberate.
9. **No cloud calls from the runner.** Only `localhost`, `127.0.0.1`,
   and user-configured target endpoints.
10. **Prompts and responses are never written to logs in plaintext.**
    Hash, run ID, length — yes. Content — no. Responses belong in the
    engagement folder on disk, not in application logs.
11. **No secrets in code, in `prompts/` files, or in bundled defaults.**
    API keys come from environment variables or user-provided inputs
    at runtime.
12. **Writes to run JSONL files are append-only.** Never rewrite an
    existing line. Never truncate. A crash during a run may leave a
    truncated last line — that is acceptable and handled on read.
13. **No database in core.** If a feature seems to need one, stop and
    ask. Files are the source of truth. See `ProductVision.md`
    principle 7.

## What not to do

- No ORM, no SQLAlchemy, no query builders. No database in core.
- No binary formats for user artifacts. Files must be human-readable
  (YAML for configs, JSONL for logs, plain text for responses).
- No Typescript/Node/framework migration in the UI without an explicit
  decision documented in `ProductVision.md`
- No async code without reason — if there is no suspension point, keep it
  synchronous
- No "while-I'm-here" refactors. If you notice something: separate issue,
  not mixed into the current change.
- No plugin systems, no YAML-driven workflows, no "power user" escape
  hatches in core. These contradict `ProductVision.md`.

## When Claude should stop and ask

Not every ambiguity is an invitation to guess. Stop and ask when:

- A requirement conflicts with `ProductVision.md`
- A requirement conflicts with one of the invariants above
- The change is security-relevant (auth, crypto, secret handling, new
  network calls)
- The change touches the run JSONL schema or the verdict JSONL schema
- The change blurs the boundary between core and analyzer
- The task implies introducing a database, a binary file format, or
  any storage hamm0r owns instead of the user
- New dependency with more than 10k LOC or questionable maintenance
- The task asks to disable a test or an eval
- The task would require a cloud account or an API key at runtime for
  core

Phrasing when asking: one concrete question, not a list of seven. If there
really are seven questions, the task is too big.

## Commands

_Flag names are illustrative; see current CLI for exact syntax._

- `cd crates/hamm0r && cargo tauri dev` — start the active desktop app in
  development mode
- `cd crates/hamm0r && cargo tauri build` — build the desktop bundle
- `cargo test --workspace` — all tests
- `cargo clippy --workspace --all-targets -- -D warnings` — linting
- `cargo fmt --all` — formatting

## When something is missing

If you read this file and something is unclear: that is a defect of this
file, not a reason to guess. Open an issue with the label
`docs:claude-md` before starting the implementation.

## Guidelines
Respect /skills/guidelines.md

## Dev workflow

After any edit, the PostToolUse hook runs:
`ruff check --fix . && ruff format --check . && mypy sidecar runner db`

Do not write tests unless explicitly asked.
Do not run pytest unless asked — UI and schema are still changing.
