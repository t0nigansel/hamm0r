# TODO

## Status legend
- [ ] not started
- [~] in progress
- [x] done

This is the hamm0r build plan.

The predecessor app's milestones live at the bottom of this file under
**Pre-history** for reference. They are not "done" in the hamm0r sense
— most of that code does not survive the rewrite. The lessons do.

---

## Milestone 1 — Workspace + crates skeleton (Week 1)

The goal: an empty hamm0r app launches, opens a window, prints
"hello" from each crate. No business logic yet. Get the boundaries
right before there is anything to break.

- [x] `Cargo.toml` workspace manifest with members `hamm0r`,
  `runner`, `storage`, `analyzer`
- [x] `rust-toolchain.toml` pinning a stable MSRV
- [x] `analyzer` crate behind a Cargo feature `analyzer`, off by
  default
- [x] CI lint: `cargo tree -p hamm0r --no-default-features` must not
  list `analyzer` or any LLM-runtime crate
- [x] CI: `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo test`
- [x] `crates/storage`: types for `Prompt`, `Request`, `Target`,
  `Scenario`, `EngagementMeta`; serde-derived; round-trip tests
  against tempdir
- [x] `crates/storage`: atomic write helper (write-to-temp + rename
  via `tempfile`)
- [x] `crates/storage`: path resolver using `directories` crate
  (`~/hamm0r/` on Linux/macOS, appdata equivalent on Windows)
- [x] `crates/runner`: empty crate compiling, no logic yet
- [x] `crates/hamm0r`: Tauri 2 shell, opens a window, no UI content
  beyond a placeholder
- [ ] First-launch hook: copies starter library from binary resources
  into `~/hamm0r/` if absent
- [x] README with build instructions and the "no sidecar" note

## Milestone 2 — Storage + library loader (Week 2)

- [x] `storage::prompts::load_all(dir)` reading every `*.yaml` in
  `~/hamm0r/prompts/`, returning `HashMap<Category, Vec<Prompt>>`
  where category is the filename stem
- [x] `storage::targets::load_all` / `save` for
  `~/hamm0r/targets/*.yaml`
- [x] `storage::requests::load_all` / `save` for
  `~/hamm0r/requests/*.yaml`
- [x] `storage::scenarios::load_all` / `save` for
  `~/hamm0r/scenarios/*.yaml`
- [x] `storage::engagements::create(slug)` / `list`
- [x] `storage::runs::append(engagement, run_id, record)` —
  JSONL append with one fsync per line (no batching, per
  Architecture.md)
- [x] Starter library: 5–10 prompts in `injection-classics.yaml`,
  `exfil.yaml`, `baselines.yaml` — enough to demo, not enough to
  pretend it's production-ready
- [x] Tauri commands: `list_prompts`, `list_targets`, `list_requests`,
  `list_scenarios`, `list_engagements`
- [x] Round-trip tests for every type (load → save → load equals
  original)

## Milestone 3 — Runner + single-step execution (Week 3)

- [x] `runner::Client` wrapping `reqwest` with rustls and explicit
  timeouts
- [x] Target adapter trait + implementations: `OpenAICompat`,
  `CustomREST`, `RawHTTP`
- [x] `minijinja` template substitution for request body and headers,
  auto-escape off, sandboxed environment
- [x] Session manager: `cookie`, `header`, `body_field`, `none`
  strategies — one `reqwest::Client` per session within a run
- [x] Bounded parallelism via `tokio::sync::Semaphore` (default 4)
- [x] Single-step run end-to-end: pick target + one prompt → fire →
  JSONL line written
- [x] `wiremock` integration tests: each adapter hits a mock target
  correctly, session strategies isolate state
- [x] Tauri command `start_run` + `emit("run-progress", ...)` for
  live UI updates

## Milestone 4 — UI core: prompts, targets, quick run (Week 4)

- [x] Tauri shell loads the UI from bundled HTML/CSS/JS — no bundler,
  no framework
- [x] Sidebar layout per `UI-Scenarios.md`: Targets, Scenarios, Runs,
  Report
- [x] Style tokens from `StyleGuide.md` wired up; fonts bundled
  locally, no CDN
- [x] Prompt browser: list categories (filenames), expand to show
  prompts, filter by tag; OWASP chip filter + search
- [x] Target editor: create/edit/delete, including session strategy
  selector; `save_target` / `delete_target` wired to Rust backend
- [x] Quick Run dialog: pick target + prompt → fire → see result
  (fire_prompt → start_run → run-progress event → read_response_body)
- [x] Live run view: response cards with status code, latency,
  verdict badge; diff / signals / raw / judge detail pane
- [x] User content rendered via `textContent`; structured chrome via
  `innerHTML` with `esc()` sanitiser — never raw untrusted HTML

## Milestone 5 — Scenarios (Week 5)

- [x] Scenario builder UI per `UI-Scenarios.md`: header, step
  timeline, session color-coding
- [x] Add Step dialog: library prompt or custom text
- [x] Library prompt snapshot at *edit* time into `prompt_text`
  (regression test: editing the library does not change saved
  scenarios)
- [x] Save/load scenarios to/from `~/hamm0r/scenarios/<slug>.yaml`
- [x] Repeat field with N independent iterations per run
- [x] Sequential multi-session execution in the runner
- [x] Run results show `iteration` and `step_id` clearly when
  `repeat > 1`
- [x] Quick Run uses the same code path: creates a transient
  one-step scenario

## Milestone 6 — Analyzer activation + verdicts (Week 6)

- [x] Manifest format for the analyzer bundle
  (`https://hamm0r.io/analyzer/manifest.json` — URL TBD, falls back to bundled)
- [x] "Activate analyz0r" UI flow: hardware detection → variant
  selection → download with progress → install to
  `~/hamm0r/analyzer/`
- [x] `analyzer` crate: `llama-cpp-2` integration, GGUF loading from
  `~/hamm0r/analyzer/models/` (Qwen2.5-3B Q4_K_M; SHA256 TODO)
- [x] Judge prompt template per `PROMPTS_SPEC.md`, with `owasp_ref`
  line omitted when missing
- [x] Verdict writer: `run-NNN.verdicts.jsonl` alongside the run file
- [x] Inference on `tokio::task::spawn_blocking` so the UI stays
  responsive
- [x] Tauri command `start_analysis(run_id)` + progress events

## Milestone 7 — HTML report + dogfood (Week 7)

- [x] Single-file HTML report rendered via `minijinja` (already in
  core)
- [x] Inline CSS, no external assets, no JS — opens in any browser
- [x] Sections: executive summary, findings grouped by category
  (filename) and by `owasp_ref` when present, evidence table
- [x] Snapshot tests via `insta` against a fixture run
- [ ] Dogfood pass: run hamm0r against the internal Azure-OpenAI CV
  app, write a report, fix whatever feels wrong in the UX

---

## Backlog (post-MVP)

- [ ] Drag-and-drop step reordering
- [ ] Compare two runs against the same target (regression testing)
- [ ] Export prompts to CSV
- [ ] Remediation recommendations per category in the report
- [ ] Signed release binaries (Windows + macOS + Linux)
- [ ] Expand starter library to 200+ prompts across more categories
- [ ] A04 privilege-escalation scenarios
- [ ] A08 supply-chain / tool-abuse scenarios
- [ ] Optional SQLite cache in `storage` if YAML scan times exceed
  ~200 ms on large libraries (per ProductVision clause)

---

## Decisions log

| Date | Decision | Reason |
|------|----------|--------|
| 2026-04 | **Rewrite in Rust as hamm0r** | Single binary, no Python sidecar, no IPC overhead |
| 2026-04 | **Files instead of SQLite** | Bruno/JMeter model: user owns the artefacts, git-friendly, no migration story |
| 2026-04 | **No client/server split** | Tauri shell hosts everything in-process |
| 2026-04 | **Analyzer as opt-in download** | Model files are 1–4 GB; default install must stay small |
| 2026-04 | **HTML reports, no PDF** | One self-contained file, browser-openable, no rendering dependency |
| 2026-04 | **One YAML per prompt category, user-named** | No fixed taxonomy; OWASP is an optional per-prompt field |
| 2026-04 | **Library snapshot at scenario edit time** | Reproducibility: editing the library does not silently change saved scenarios |
| 2026-04 | **`llama-cpp-2` over `candle`** | Model reach matters more than toolchain purity for the judge |
| 2026-04 | **`minijinja` over `tera`** | Smaller, Jinja-compatible, sufficient for our templating |
| 2026-04 | Tauri over Electron | Smaller binary, no Node runtime |

---

## Pre-history — predecessor milestones (2026-03)

These were completed in the predecessor project before the rewrite.
Listed here so we know what existed and why hamm0r re-implements
rather than ports.

- [x] Schema + repository in SQLite, Pydantic-validated YAML loader
- [x] Async runner with httpx, OpenAICompat + CustomREST adapters,
  graceful stop, progress callback
- [x] Tauri shell with Python sidecar over JSON-on-stdio, prompt
  browser, target config, run UI
- [x] Early local analyzer: Ollama + Qwen judge, heuristic pre-filter,
  PDF reports
- [x] Scenario-based testing v1: scenarios + steps in SQLite,
  multi-session sequential runner, sidebar UI, 29 tests

### What carries over

- The data model (prompts, targets, scenarios, sessions) — proven in
  use, schema migrates 1:1 to YAML
- The UI patterns (sidebar nav, color-coded session timeline,
  quick-run flow) — keep
- The judge prompt template — keep, with the schema adjustments
  documented in `PROMPTS_SPEC.md`
- Lessons from the dogfood run against the Azure-OpenAI CV app —
  keep, drive the Milestone 7 rerun

### What does not carry over

- Python codebase (replaced by Rust)
- SQLite as the persistence model (replaced by YAML/JSONL)
- Sidecar architecture (replaced by single-process Tauri)
- WeasyPrint and the PDF report path (replaced by HTML)
- Ollama dependency (replaced by in-process `llama-cpp-2`)
