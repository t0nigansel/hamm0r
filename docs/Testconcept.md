# Test Concept

This document describes the quality assurance measures in place for
`hamm0r`. It is aimed at human contributors and reviewers, not at
automation.

The scope is intentionally pragmatic: `hamm0r` is in active
development, the UI is changing frequently, and the data model still
moves. The QA setup is therefore weighted toward **fast, deterministic,
low-cost feedback** (linting, type checking, focused unit tests) and
away from brittle high-cost layers (full E2E, UI snapshot tests) for
now.

## 1. Stack

- **Backend:** Rust (Tauri application, crates under workspace).
- **Frontend:** Vanilla JavaScript (loaded by the Tauri webview; no
  framework, no build step, no TypeScript).
- **Persistence:** Files on disk — YAML for configs, append-only JSONL
  for run logs, plain text for responses. No database in core.

## 2. Goals

- Catch regressions in the stable core (storage layer, runner, Tauri
  command layer, analyzer) before they reach `main`.
- Keep the codebase consistent in style and type-safe enough that
  refactors are cheap.
- Keep AI-assisted development (Claude Code) honest: every edit passes
  through a deterministic toolchain that catches hallucinated APIs,
  unused imports, and type mismatches before a human reviews them.
- Do **not** spend effort on UI E2E or snapshot tests while the UI is
  unstable.

## 3. Scope

| Area                          | Coverage focus                            |
| ----------------------------- | ----------------------------------------- |
| Rust core (storage, runner)   | Schema, repository functions, async exec  |
| Rust Tauri commands           | Tauri command handlers                    |
| Rust analyzer / analyzor-cli  | Verdict logic, report generation          |
| Frontend (vanilla JS)         | Manual smoke testing only                 |
| Prompt library (`prompts/`)   | YAML schema validity                      |

## 4. Quality Layers

### 4.1 Rust — formatting (`cargo fmt`)

`rustfmt` runs in check mode in CI and is auto-applied locally.

- Run locally: `cargo fmt --all`
- CI check: `cargo fmt --all -- --check`

Rationale: zero-token, zero-discussion style enforcement. No style
debates in PRs.

### 4.2 Rust — linting (`cargo clippy`)

`clippy` runs on all targets, warnings treated as errors.

- Run: `cargo clippy --workspace --all-targets -- -D warnings`
- Lint configuration lives in `Cargo.toml` under
  `[workspace.lints.clippy]` (pedantic + `unwrap_used` + `expect_used`
  at warn level; `unsafe_code` forbidden).

Rationale: clippy catches a large class of real bugs (unused results,
incorrect lifetime patterns, performance traps) and enforces idiomatic
Rust at near-zero cost.

### 4.3 Rust — type checking (`cargo check`)

`cargo check` runs after every change as the fastest correctness
signal. It catches hallucinated APIs, wrong signatures, missing trait
implementations, and type mismatches without producing a full binary.

- Run: `cargo check --all-targets`

Rationale: ~3–5× faster than `cargo build` and gives the same
correctness guarantees for typing and borrow checking.

### 4.4 Rust — unit and integration tests (`cargo test`)

Tests live alongside their modules (`#[cfg(test)] mod tests`) and in
`tests/` for integration tests.

Covered:

- **Storage tests** — round-trip writes and reads against the
  file-based storage layer.
- **Runner tests** — scenario state machine, retry behavior, timeouts.
- **Tauri command tests** — command handlers with a temporary fixture.
- **Analyzer tests** — verdict assignment, report rendering, and
  snapshot tests for report output.

- Run all: `cargo test`
- Run a single crate: `cargo test -p <crate-name>`

Tests target the **public surface** of each crate (repository
functions, command handlers, CLI entry points), not private helpers.
This keeps tests stable across refactors.

### 4.5 Frontend — manual smoke testing

The frontend is vanilla JavaScript with no build step and no type
toolchain. There is no `tsc`, no ESLint, no Prettier, and none is
planned while the UI is actively changing.

UI changes are validated by **manual smoke testing** against a local
dev build (`cd crates/hamm0r && cargo tauri dev`) before merging.

### 4.6 End-to-end and UI tests — deliberately out of scope

E2E tests (Tauri WebDriver, Playwright) and UI snapshot tests are
**not** in the QA pipeline at this stage.

Reason: the UI surface is still changing, and the
Target / Library / Scenario / Engagement / Workbench model is being
restructured. Maintaining E2E suites against a moving target would cost
more than it catches.

E2E will be reintroduced once the Engagement Wizard flow and the
Workbench model stabilize.

### 4.7 AI-assisted development guardrails

`hamm0r` is developed with Claude Code. To prevent hallucinated APIs
and silent regressions, two hooks are configured in
`.claude/settings.json`.

**PostToolUse hook** (fires after every `Edit` or `Write`):

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo check
```

`cargo test` is **not** in the hook (kept out to avoid forced test
writing while modules are changing).

**PreToolUse hook** (fires before `Read`, `Bash`, `Grep`, `Glob`):

Runs `block-env-read.py` to prevent the agent from reading environment
variables or secrets from the host.

Rationale: hallucinated function names, wrong signatures, and unused
imports get caught within the same edit cycle, before a human reviews
the diff. The pre-read hook enforces the invariant that secrets never
leave the host.

## 5. Local Workflow

Recommended sequence before pushing a change:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo check --all-targets
cargo test
```

If all steps pass, the change is ready for review.

## 6. Continuous Integration

CI runs on every push and pull request (`.github/workflows/ci.yml`):

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test`

A failing step blocks merge.

## 7. What This Concept Deliberately Excludes

- **Coverage thresholds.** Coverage is informational, not gated. A
  hard threshold would push contributors to write low-value tests for
  metric reasons.
- **Mutation testing.** Too early; the test suite is still growing.
- **UI snapshot tests.** UI is unstable.
- **Load and performance tests.** Not a concern for a local-first
  desktop tool at current scale.
- **Security scanning of `hamm0r` itself** (`cargo audit`, supply chain
  checks). Planned, not yet in place.

## 8. Review Checklist

For a pull request to be considered ready:

- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo test` passes.
- [ ] Manual smoke test of any affected UI flow.
- [ ] Public API changes (storage schema, Tauri commands, CLI flags) are
      reflected in `Architecture.md`, `Datamodel.md`, or `README.md`.

## 9. Open Items

- Reintroduce E2E once Workbench / Engagement Wizard stabilize.
- Add `cargo audit` and frontend dependency audit to CI.
- Decide on `tarpaulin` / `llvm-cov` for informational coverage once
  the test suite is more complete.
