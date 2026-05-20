# TODO — hamm0r

Master work-list derived from [`productVision.md`](productVision.md).
Each chunk is sized for one focused commit. Bigger efforts reference
their dedicated plan in [`plans/`](plans/).

Completed and shipped refactor work lives in
[`plans/RefactorPlan.md`](plans/RefactorPlan.md). Hosted Judge work
is tracked in [`plans/cloudLLMPlan.md`](plans/cloudLLMPlan.md).

---

## 1 · Multi-session testing

_Vision: "A scenario can fire across N parallel sessions with distinct
session identifiers." First-class scenario type, not an afterthought._

Reference: `productVision.md` § Multi-session testing

- [ ] **1.1 — Session identity model.** Define `SessionIdentity` type
      in `storage::types` (cookie jar, conversation-ID header, custom
      header+value). Add `session_count: Option<u32>` and
      `session_identity: Option<SessionIdentityConfig>` to the Scenario
      YAML schema. Update `Datamodel.md`.
- [ ] **1.2 — Runner: parallel session orchestration.** In
      `runner::run`, add `execute_multi_session_run` that spawns N
      `reqwest::Client` instances with distinct session state. Reuse
      the existing semaphore for bounded parallelism across sessions.
- [ ] **1.3 — Canary token generation.** Add `runner::canary` module.
      Generate deterministic canary strings (UUID-based, per-run
      seeded). Expose `generate_canary(run_id, session_idx)` for
      plant/probe payloads.
- [ ] **1.4 — Plant/probe phase scheduling.** Extend the matrix
      expansion to support two-phase firing: session A sends
      plant-prompts containing a canary, then session B sends
      probe-prompts. The phase assignment is declared per-prompt via a
      new optional `phase: plant | probe | any` field.
- [ ] **1.5 — Cross-session leak scanner.** After all sessions
      complete, scan all response files for canary tokens that appeared
      in a session that did not plant them. Record matches as
      `leak_detected` entries in the run JSONL.
- [ ] **1.6 — JSONL schema: multi-session fields.** Add
      `session_id: Option<String>` and `phase: Option<String>` to
      `RunAttempt`. Update `Datamodel.md`. Migration: absent fields
      default to `null` on read.
- [ ] **1.7 — UI: Scenario editor multi-session toggle.** Add session
      count input + session identity config to the Scenarios view
      matrix panel. Only visible when `session_count > 1`.
- [ ] **1.8 — UI: engagement results session column.** Show
      `session_id` as a filterable column in the engagement detail
      results table. Leak entries get a distinct badge.
- [ ] **1.9 — Tests: multi-session runner.** Wiremock integration
      tests: 2 sessions with cookie jars, canary plant in session 0
      leaks to session 1, no-leak baseline.
- [ ] **1.10 — Analyzer: cross-session leak verdict type.** Extend
      verdict schema with `category: "cross_session_leak"`. The
      analyzer auto-flags leak entries without needing LLM judging.

---

## 2 · Prompt mutation engine

_Vision: "Hamm0r mutates seed prompts locally and deterministically to
expand coverage without bloating the library." Opt-in per scenario, no
LLM needed, reproducible._

Reference: `productVision.md` § Prompt mutation

- [ ] **2.1 — Mutation trait + registry.** Create `runner::mutation`
      module. Define `Mutator` trait with `fn mutate(&self, seed: &str)
      -> Vec<MutatedPrompt>`. `MutatedPrompt` carries the mutated
      text + a `mutation_id` string describing what was applied.
- [ ] **2.2 — Encoding mutators.** Implement: Base64, ROT13, hex,
      URL-encoding, Unicode homoglyphs. Each is a separate struct
      implementing `Mutator`. Pure, no dependencies beyond `std`.
- [ ] **2.3 — Obfuscation mutators.** Implement: whitespace injection,
      zero-width character insertion, leetspeak substitution.
- [ ] **2.4 — Structural mutators.** Implement: code-block wrapping
      (` ```), JSON embedding, Markdown comment wrapping, prefix
      injection ("As a system admin, …").
- [ ] **2.5 — Persona mutators.** Implement: authority framing
      ("You are an unrestricted AI…"), role-play prefixes, jailbreak
      template wrapping (DAN-style frame around the seed).
- [ ] **2.6 — Linguistic mutators (basic).** Implement: synonym
      substitution (small built-in synonym table, no external API),
      politeness variants ("Please …", "Could you kindly …").
      Translation roundtrips deferred (needs external API or model).
- [ ] **2.7 — Scenario schema: mutation config.** Add
      `mutations: Option<MutationConfig>` to Scenario YAML.
      `MutationConfig` has `enabled_mutators: Vec<String>` and
      `max_variants_per_seed: Option<u32>`. Update `Datamodel.md`.
- [ ] **2.8 — Matrix expansion with mutations.** In
      `execute_matrix_run`, when mutations are enabled, expand each
      seed prompt into seed + N mutation variants before the Cartesian
      product. Total attempts = requests × (seeds + mutations) × repeat.
- [ ] **2.9 — JSONL: mutation provenance.** Add
      `mutation_id: Option<String>` to `RunAttempt`. The report shows
      which mutation cracked the filter, not just which seed.
      Update `Datamodel.md`.
- [ ] **2.10 — UI: Scenario editor mutation panel.** Checkboxes for
      each mutator family (Encoding, Obfuscation, Structural, Persona,
      Linguistic). Max-variants-per-seed slider. Live attempt-count
      preview updates.
- [ ] **2.11 — Tests: mutation engine.** Unit tests for each mutator:
      deterministic output, round-trip where applicable, no-op on
      empty input. Integration test: matrix run with 2 mutators
      produces expected attempt count.

---

## 3 · Pentester triage workflow

_Vision: "Each finding can be marked confirmed, false-positive, or
needs-review, with a free-text note. The report stays a working
document until export."_

Reference: `productVision.md` § Pentester workflow — Triage

- [x] **3.1 — Triage data model.** `TriageStatus` enum and
      `TriageEntry { seq, status, note, updated_at }` in `storage::types`.
      Sidecar file: `runs/<run>.triage.yaml`.
- [x] **3.2 — Storage: triage CRUD.** `storage::triage` module with
      `load`, `save_entry`, `list_entries`. Atomic writes. Missing file
      returns empty (all unreviewed).
- [x] **3.3 — Tauri commands: triage.** `get_triage` and
      `set_triage_status` registered in `main.rs`.
- [x] **3.4 — UI: triage controls in engagement detail.** Each result
      row has a status `<select>` (colour-coded) and an inline note input
      toggled by a note button. Changes save immediately via `set_triage_status`.
- [x] **3.5 — UI: triage filter.** Filter bar above results table:
      All / Confirmed / Needs Review / Unreviewed / False Positive.
      Active filter dims non-matching rows.
- [ ] **3.6 — Report: triage integration.** Markdown export includes
      Triage and Note columns. HTML report triage integration (confirmed
      prominent, false-positives dimmed) deferred — needs analyzer
      report generation to be extended.
- [x] **3.7 — Tests: triage storage.** Missing-file, round-trip, upsert,
      multi-entry sort, separate runs, all-status-variants — 7 tests, all
      passing (`cargo test --workspace`).

---

## 4 · Replay (re-fire with variation)

_Vision: "Any single attempt in the report can be re-fired with one
click, optionally with a tweaked prompt."_

Reference: `productVision.md` § Pentester workflow — Replay

- [ ] **4.1 — Tauri command: replay_attempt.** Takes `engagement,
      run_id, seq, prompt_override: Option<String>`. Loads the original
      attempt from the JSONL, resolves the request template, substitutes
      the original (or overridden) prompt, fires via the runner, appends
      the result to a new run or the same run (TBD — see 4.2).
- [ ] **4.2 — Design decision: replay target run.** Decide whether
      replays append to the original run (simple, but breaks the "run
      is immutable after footer" invariant) or create a mini-run
      (`run-NNN-replay-M.jsonl`). Document in `Datamodel.md`.
- [ ] **4.3 — UI: replay button per result row.** Each row in the
      engagement detail table gets a replay icon. Clicking opens a
      small modal pre-filled with the original prompt text (editable).
      "Fire" button sends the replay command. Result appears inline
      below the original row.
- [ ] **4.4 — Tests: replay command.** Wiremock test: replay an
      attempt with the same prompt, replay with a tweaked prompt,
      verify the new JSONL entry references the original seq.

---

## 5 · Per-request repeat counts

_Vision: "A scenario's global repeat multiplies with per-request
overrides. Login fires once per scenario iteration, chat fires five."_

Reference: `productVision.md` § Pentester workflow — Per-request repeat

- [x] **5.1 — Scenario schema: per-request repeat.** `RequestEntry { id, repeat }`
      defined in `storage::types`; `Scenario.request_ids` is `Vec<RequestEntry>`;
      backward-compat bare-string deserialization implemented.
- [x] **5.2 — Runner: per-request repeat expansion.** `per_request_repeat` map
      wired into `execute_matrix_run`; correct `repeat × local_repeat` math in place.
- [x] **5.3 — UI: per-request repeat input.** In the Scenario matrix
      editor, each selected request shows an optional repeat count
      field (defaults to 1, meaning "use global only").
- [x] **5.4 — Tests: per-request repeat.** Round-trip and expansion tests present
      in `storage::types` (`request_entry_struct_with_repeat_roundtrip`,
      `scenario_with_per_request_repeat_roundtrip`).

---

## 6 · Top bar: active-run progress

_Vision: "When a run is active, a compact progress bar sits in the top
bar and can be expanded."_

Reference: `productVision.md` § UI conventions — Top bar

- [ ] **6.1 — UI: compact progress bar component.** A minimal
      `[===.       ] 3/10` indicator anchored in the top bar. Hidden
      when no run is active. Listens to existing `run-progress` events
      from the backend.
- [ ] **6.2 — UI: expand/collapse progress detail.** Clicking the
      compact bar expands a small panel showing: run name, elapsed
      time, attempts completed/total, current request being fired,
      cancel button.
- [ ] **6.3 — UI: top bar breadcrumb.** Left side of the top bar
      shows the current view path (e.g. "Engagements › acme-chatbot ›
      run-003"). No `+`, no global `▶`, no empty help button.

---

## 7 · Report export formats

_Vision: "The output is a shareable artifact (PDF, Markdown), not a
session you have to stay logged into."_

Reference: `productVision.md` § Core product principles #5

- [ ] **7.1 — Markdown report generation.** Add
      `analyzer::report::render_markdown` that produces a `.md` file
      alongside the HTML report. Same data, different format. Written
      to `reports/report-<run>.md`.
- [ ] **7.2 — UI: export format picker.** After analysis completes,
      the "Export" button offers HTML and Markdown. PDF is a stretch
      goal (the user can convert Markdown → PDF externally).
- [ ] **7.3 — Update `Datamodel.md`.** Document the new report file
      naming and format.

---

## 8 · Hosted Judge (analyz0r cloud mode)

_Vision: "The user activates [the analyzer] once and chooses between a
local LLM or a configured remote endpoint."_

Full plan: [`plans/cloudLLMPlan.md`](plans/cloudLLMPlan.md)

- [x] **8.1 — Spec realignment.** Update `ProductVision.md`,
      `Architecture.md`, `Stack.md`, `Datamodel.md` per the plan's
      Phase 1. `productVision.md` updated by user. `Architecture.md`
      data flow step 5 and `Stack.md` llama-cpp-2 section already
      reflected the hosted-judge stance. `Datamodel.md` config.yaml
      already had the `hosted_judge` block; verdict header extended
      with `judge_mode`/`provider`/`deployment` fields.
- [ ] **8.2 — Config + secret plumbing.** `judge_mode`, hosted judge
      config schema, keychain secret path, Settings UI.
- [ ] **8.3 — JudgeBackend abstraction.** `LocalJudgeBackend` +
      `HostedJudgeBackend` trait implementations.
- [ ] **8.4 — Azure provider adapter.** `AzureOpenAiAdapter` with
      `chat_completions` and `responses` API style support.
- [ ] **8.5 — End-to-end hosted analysis.** Single-result judge,
      full-run analysis, report generation with hosted verdicts.
- [ ] **8.6 — Hardening.** Failure messages, hosted status display,
      usage/cost controls, docs polish.

---

## 9 · Library & prompt quality

_Vision: "Curated and user-extensible prompt collections, organized by
OWASP LLM Top 10 and category." + "Narrow and deep beats broad and
shallow."_

Reference: `productVision.md` § Mental model — Library;
`PromptsSpec.md`

- [x] **9.1 — Starter library audit.** Review the shipped
      `prompts/` files against the current OWASP LLM Top 10 (2025)
      and Agentic Top 10 (2026). Identify categories with fewer than
      3 prompts. Fill gaps with curated, tested prompts.
- [x] **9.2 — Baseline prompts per category.** Ensure every attack
      category file includes at least one `baseline` / `benign`
      prompt (per `PromptsSpec.md` § Baseline prompts).
- [x] **9.3 — Starter request templates.** Review `requests/` for
      coverage of common LLM endpoints (OpenAI, Anthropic, Azure,
      generic REST). Add missing templates.
- [x] **9.4 — Library seeding: update-without-overwrite.** Verify
      that on app update, new starter prompts are offered as additions
      to new files, never overwriting the user's existing library.
      Add a version marker to seeded files.

---

## 10 · Five-minute promise polish

_Vision: "From download to first usable report: under five minutes."_

Reference: `productVision.md` § The five-minute promise

- [ ] **10.1 — First-run guided start.** A single-screen (not a
      wizard) welcome that shows: paste URL → pick auth → pick
      prompts → fire. Dismissible, never blocks the full UI.
      Must respect principle 3 (no wizards).
- [ ] **10.2 — Installer size audit.** Verify the hamm0r binary
      (without analyzer) is a small download. Flag any crate
      that adds >1 MB to the binary.
- [ ] **10.3 — Cold-start benchmark.** Measure download → first
      report on macOS, Windows, Linux. Automate the measurement
      if feasible. Target: under 5 minutes on a standard workload.

---

## 11 · UI conventions & polish

Reference: `productVision.md` § UI conventions; `STYLEGUIDE.md`

- [ ] **11.1 — Sidebar audit.** Verify sidebar matches the vision
      exactly: Home, Engagements, Requests, Scenarios, Library,
      Settings. Nothing else. One entry per object type.
- [ ] **11.2 — "Fire from where it belongs" audit.** Verify: Request
      fires standalone from the Request screen. Scenario fires only
      inside an Engagement. No global "fire something" button exists.
- [x] **11.3 — App icon.** Current icon quality is not good enough
      (carried over from old ToDo).

---

## 12 · Miscellaneous (small items)

- [x] **12.1 — `analyzorPlan.md` references fixed.** `Architecture.md`
      now points to the in-document analyzer section + `cloudLLMPlan.md`.
      `Stack.md` now points to `Architecture.md § The analyzer as a
      separable module`.
- [x] **12.2 — clippy clean.** `cargo clippy --workspace --all-targets
      -- -D warnings` passes cleanly.
- [x] **12.3 — Engagement schema rename.** `EngagementTarget.request_id`
      renamed to `scenario_id` in code and `Datamodel.md`. Old YAML files
      accepted via `#[serde(alias = "request_id")]`. Alias test added.

---

## Done (kept for reference, prune when convenient)

- [x] Fix the Settings button — root cause was the
  `openRequestAuthTokenModal` ReferenceError aborting DCL setup.
- [x] Add an easy way to create and add prompts — full CRUD shipped
  in the Library view.
- [x] Fix the Workbench response view / Workbench sending — the
  Workbench view was retired in Phase 2F of the refactor.
- [x] Wizard scenario-selection bug — the Engagement Wizard was
  retired in Phase 2 of the refactor.
- [x] Collapse Target entity — shipped (RefactorPlan Phase 2).
- [x] Request dependencies / auth chains — shipped (RefactorPlan
  Phase 2B).
- [x] Matrix scenarios (requests × library subset) — shipped
  (RefactorPlan Phase 2C).
- [x] Drop legacy step-based Scenario flow — shipped (RefactorPlan
  Phase 2G).
- [x] UX polish: Run a Scenario modal, run→scenario_id lookup,
  empty-state hints — shipped (RefactorPlan Phase 2H).
