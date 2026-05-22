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

Full plan: [`plans/multiSessionPlan.md`](plans/multiSessionPlan.md)

- [x] **1.1 — Session identity model.** `SessionIdentityConfig` +
      `SessionIdentityKind` (`CookieJar` / `ConversationHeader` /
      `CustomHeader`) in `storage::types`. `Scenario.session_count` +
      `Scenario.session_identity` added. `Phase` enum added with
      `PromptEntry.phase: Phase` (defaults to `Any` — legacy YAML loads
      unchanged). `Datamodel.md §"Scenario shape (matrix)"` and
      `PromptsSpec.md` documented.
- [x] **1.2 — Runner: multi-session orchestration.** New
      `runner::multi_session::execute_multi_session_run`. Per-session
      `reqwest::Client` with its own cookie jar + optional default
      identity header. V1 runs sessions sequentially within each phase;
      the plan's parallel-across-sessions optimization is deferred (see
      `plans/multiSessionPlan.md` Q7).
- [x] **1.3 — Canary token generation.** `runner::canary::generate(run_id,
      session_idx, scenario_id)` returns `HAMM0R-<11-hex>` deterministically
      via SHA-256. Sibling `runner::canary::inject(text, canary)`
      substitutes `{{canary}}` markers without invoking the template
      engine (so attack prompts' other braces stay opaque).
- [x] **1.4 — Plant/probe phase scheduling.** `PromptEntry.phase`
      drives the scheduler in `execute_multi_session_run`: all plants
      fire (sessions iterated in order) before any probe/any prompt.
      Phase recorded on each attempt as `Some("plant" | "probe" |
      "any")`.
- [x] **1.5 — Cross-session leak scanner.** New `runner::leak_scanner`
      module runs at the end of every multi-session run. It walks
      probe/any attempts, loads their response bodies, fast-filters by
      `HAMM0R-` prefix, then checks each canary planted in a _different_
      session. Matches emit `RunRecord::LeakDetected` JSONL records.
      4 unit tests in `runner::leak_scanner::tests`.
- [x] **1.6 — JSONL schema: multi-session fields.** `RunAttempt.session_id`
      and `RunAttempt.phase` added as `Option<String>`, both
      `skip_serializing_if = "Option::is_none"`. `RunRecord::LeakDetected`
      variant added. `Datamodel.md §"Run log"` documents the new fields
      and the `leak_detected` record shape. Legacy logs load unchanged
      (round-trip test in `storage::runs::tests`).
- [x] **1.7 — UI: Scenario editor multi-session toggle.** Session-count
      input + identity-kind picker (cookie jar / conversation header /
      custom header) + optional header-name input added to the matrix
      editor in `ui/index.html`. Visibility collapses to a single
      input when `session_count <= 1`; multi-session controls reveal
      when the count is >1.
- [x] **1.8 — UI: engagement results session column + leak badge.**
      Existing `Session` column in the results table now prefers
      `attempt.session_id` over the legacy `session` label. A new
      `read_run_leaks` Tauri command exposes `LeakDetected` records;
      `get_results` bundles them by probe seq and the row renderer
      adds a red `leak` badge plus a phase badge (`plant` / `probe`)
      next to the session label.
- [x] **1.9 — Tests: multi-session runner.** Wiremock integration
      tests in `crates/runner/tests/integration.rs`:
      `multi_session_plant_probe_leak_is_detected_and_recorded` (2
      sessions with cookie jars, leaky-echo mock, asserts the JSONL
      carries plant/probe attempts with `session_id` + `phase` and the
      scanner records s1's probe surfacing s0's canary) and
      `multi_session_no_leak_when_server_does_not_echo_canary` (clean
      baseline, asserts zero leak records).
- [x] **1.10 — Analyzer: cross-session leak verdict type.**
      `analyzer::pipeline::emit_cross_session_leak_verdicts` runs as the
      first step of `judge_run`: reads every `LeakDetected` from the run
      JSONL and emits one `category: "cross_session_leak"` verdict per
      leak with `verdict: Success`, `severity: "high"`, `owasp_ref:
      "LLM02"`, `model_used: "leak-scanner"`. Idempotent across re-judges
      via a `(seq, leak-key)` dedup set.

---

## 2 · Prompt mutation engine

_Vision: "Hamm0r mutates seed prompts locally and deterministically to
expand coverage without bloating the library." Opt-in per scenario, no
LLM needed, reproducible._

Reference: `productVision.md` § Prompt mutation

- [x] **2.1 — Mutation trait + registry.** `runner::mutation` ships the
      `Mutator` trait, `MutatedPrompt { text, mutation_id }`, and a
      stable `registry()` listing every shipped mutator in declaration
      order. `expand_seed(seed, enabled_ids, max_variants)` is the
      single entry point used by the orchestrator.
- [x] **2.2 — Encoding mutators.** Base64 (hand-rolled, no new crate),
      ROT13, hex, URL-encoding (RFC 3986 unreserved passthrough),
      Cyrillic homoglyph substitution. All in `runner::mutation::encoding`.
- [x] **2.3 — Obfuscation mutators.** Whitespace injection between
      ASCII letter pairs, U+200B zero-width insertion between every
      char, leetspeak vowel/consonant substitution. In
      `runner::mutation::obfuscation`.
- [x] **2.4 — Structural mutators.** Triple-backtick code block wrap,
      JSON `{"task": …}` embedding, HTML/Markdown comment wrap,
      authority-framing prefix injection. In `runner::mutation::structural`.
- [x] **2.5 — Persona mutators.** Authority framing, role-play
      framing, DAN-style jailbreak wrap. In `runner::mutation::persona`.
- [x] **2.6 — Linguistic mutators (basic).** Built-in synonym table
      (first-match replace), politeness wrap. Translation roundtrips
      remain deferred. In `runner::mutation::linguistic`.
- [x] **2.7 — Scenario schema: mutation config.** `MutationConfig`
      added to `storage::types` with `enabled_mutators` and optional
      `max_variants_per_seed`. `Scenario.mutations: Option<MutationConfig>`
      is absent-by-default and skipped on serialize when unset.
      `Datamodel.md §"Scenario shape (matrix)"` updated.
- [x] **2.8 — Matrix expansion with mutations.** `commands::runs`
      expands each seed payload via `runner::mutation::expand_seed`
      before constructing `MatrixRunConfig.payloads`, so the matrix
      runner sees seed + variants up-front. Total target attempts =
      seeds × (1 + variants_kept) × request_firings × repeat.
- [x] **2.9 — JSONL: mutation provenance.** `RunAttempt.mutation_id:
      Option<String>` added; target attempts carry the mutator id (or
      `"seed"`), prerequisite/synthetic attempts carry `None`. Replay
      runs inherit the original attempt's `mutation_id`. Field is
      additive and `#[serde(default, skip_serializing_if =
      "Option::is_none")]` — legacy run files load unchanged and a
      truncated last line still terminates cleanly per CLAUDE.md #12.
      `Datamodel.md §"Run log"` updated.
- [x] **2.10 — UI: Scenario editor mutation panel.** Mutation fieldset
      added to the Scenarios matrix editor: per-family checkboxes
      (Encoding, Obfuscation, Structural, Persona, Linguistic) populated
      from the new `list_mutators` Tauri command, a Max-variants input,
      and the existing attempt-count hint updated to include the
      mutation multiplier.
- [x] **2.11 — Tests: mutation engine.** 21 unit tests in
      `runner::mutation::tests` (per-family determinism, encoding
      round-trips, registry uniqueness, `expand_seed` cap behaviour,
      empty-input no-op). Integration test
      `matrix_run_with_mutations_records_mutation_id_per_attempt` in
      `crates/runner/tests/integration.rs` fires a matrix run with two
      mutators and asserts the JSONL attempts carry the expected
      `mutation_id` set.

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
- [x] **3.6 — Report: triage integration.** `ReportBuildInput` now
      carries `triage: Vec<TriageEntry>`; `ReportEvidenceRow` exposes
      `triage_status` (lower-case enum label, defaulting to `unreviewed`
      when no sidecar entry exists) and `triage_note`. The Markdown
      evidence section in `render_markdown_report` renders a Triage row
      per finding and an optional Triage note row when present.
      `pipeline::generate_report` loads `<run>.triage.yaml` via
      `storage::triage::list_entries` (errors are non-fatal — missing or
      unreadable sidecar means every finding renders as `unreviewed`).
      HTML report triage integration (confirmed prominent, false-positives
      dimmed) stays deferred per the original ToDo — the existing HTML
      template doesn't reference the new fields.
- [x] **3.7 — Tests: triage storage.** Missing-file, round-trip, upsert,
      multi-entry sort, separate runs, all-status-variants — 7 tests, all
      passing (`cargo test --workspace`).

---

## 4 · Replay (re-fire with variation)

_Vision: "Any single attempt in the report can be re-fired with one
click, optionally with a tweaked prompt."_

Reference: `productVision.md` § Pentester workflow — Replay

- [x] **4.1 — Tauri command: replay_attempt.** Takes `engagement,
      run_id, seq, prompt_override: Option<String>`, loads the original
      attempt, resolves the Request (via the new `RunAttempt.request_id`
      field, falling back to URL+method match for legacy logs),
      substitutes the prompt, fires via `execute_run`, and writes a
      sibling `<run_id>-replay-<n>.jsonl` file. Companion
      `list_replays` command exposes them.
- [x] **4.2 — Decision: sibling file.** Replays go to
      `runs/<run_id>-replay-<n>.jsonl` with `replay_of: {run_id, seq,
      prompt_overridden}` in the header. Append-to-original was
      rejected because it violates CLAUDE.md #12 (footer-terminates).
      Documented in `Datamodel.md §"Replay run files"`.
- [x] **4.3 — UI: Replay in row detail panel.** The Result Detail
      modal grew a Replay section: prompt textarea (pre-filled with
      the original), Reset button, ▶ Replay button. The replay run is
      polled and its response rendered inline. Replay files are kept
      out of the top-level runs list and are auto-cleaned when the
      original run is deleted.
- [x] **4.4 — Tests.** Wiremock integration test verifies the replay
      JSONL carries `replay_of` in the header, starts seq at 1, and
      records the new `request_id`. Storage tests cover the schema
      round-trip and the delete-cascades-to-replays path.

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

- [x] **6.1 — UI: compact progress bar component.** Top-bar
      `[==========     ] 11/32` indicator with an inline Stop button.
      Hidden when no run is active, fed by the existing `run-progress`
      events.
- [x] **6.2 — UI: expand/collapse progress detail.** Clicking the
      compact bar opens a small floating panel showing run id, elapsed
      time, attempts, errors, current request id and current prompt id.
      Cancel sits in the top bar next to the bar (delivered with 6.1).
      `RunProgress` was extended with `request_id` + `prompt_id`.
- [x] **6.3 — UI: top bar breadcrumb.** Three-segment path
      `view › engagement › run` rendered via `updateBreadcrumb()`. The
      global `+`, `▶` and `?` buttons were removed; creation moves
      entirely to the Engagements view.

---

## 7 · Report export formats

_Vision: "The output is a shareable artifact (PDF, Markdown), not a
session you have to stay logged into."_

Reference: `productVision.md` § Core product principles #5

- [x] **7.1 — Markdown report generation.** `analyzer::report::render_markdown_report`
      hand-rolls a Markdown document from the same `ReportData` the HTML
      template consumes. `pipeline::generate_report` writes the `.md`
      sibling next to the `.html`; failure to write the `.md` is logged
      but non-fatal (HTML stays the primary artifact). New helper
      `markdown_report_path_for(engagement_dir, run_id)` exposes the
      canonical path.
- [x] **7.2 — UI: export format picker.** The existing "Export Markdown"
      button in the runs table now prefers the canonical analyzer
      `report-<run>.md` (via a new `read_report_md` Tauri command) when
      the run has been analyzed, falling back to the client-side
      raw-results markdown for unanalyzed runs. The HTML report is
      already openable via the existing analyzer flow; PDF stays a
      stretch goal (the user can convert the Markdown externally).
- [x] **7.3 — Update `Datamodel.md`.** `§"Reports"` documents the
      sibling `.md` file, the data source, and the non-fatal write
      semantics.

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
- [x] **8.2 — Config + secret plumbing.** `judge_mode`, `HostedJudgeConfig`,
      enums and defaults live in `storage::types`. Keychain save/read goes
      through `storage::secrets` (service `hamm0r`, account = `secret_ref`)
      exposed via Tauri `set_secret_ref` / `forget_secret_ref` /
      `secret_ref_status`. Settings DTOs in
      `crates/hamm0r/src/commands/app_settings.rs` round-trip the
      non-secret fields plus `secret_stored` / `keychain_available` status.
      Settings UI form (`#settings-analyzer-judge-mode` + hosted-judge
      fieldset) rendered in `ui/index.html` and wired in `ui/js/app.js`.
- [ ] **8.3 — JudgeBackend abstraction.** Deferred. The pipeline currently
      dispatches via direct functions in `crates/analyzer/src/pipeline.rs`
      (`judge_one_heuristic` / `judge_one_hosted`, `run_heuristic` /
      `run_hosted` / `run_llm` / `run_ollama`). Functionally equivalent to
      the plan's trait split; revisit only if a third backend or shared
      streaming surface justifies the abstraction.
- [x] **8.4 — Azure provider adapter.** `crates/analyzer/src/hosted.rs`
      ships the Azure OpenAI adapter for both `chat_completions` and
      `responses` styles with an `auto` fallback that tries chat first then
      responses.
- [x] **8.5 — End-to-end hosted analysis.** `judge_one_hosted` (single
      result) and `run_hosted` (full run) in `pipeline.rs` are reached from
      `judge_result` and `judge_all` Tauri commands when `judge_mode ==
      hosted`. `generate_report` consumes the resulting verdict files.
- [x] **8.6 — Hardening.** Verdict headers carry `judge_mode`, `provider`,
      `deployment`; the Settings UI surfaces `secret_stored` /
      `keychain_available` and disables the Run/Judge buttons with a
      specific reason when hosted config is incomplete (see
      `analyzerAvailability` checks in `ui/js/app.js`). Usage controls
      (`max_input_chars`, `max_output_tokens`, `request_timeout_seconds`,
      `max_retries`) plumb through to the adapter; hosted errors fail hard
      with provider-specific messages and no silent fallback. A
      `test_hosted_judge` command lets the user dry-run the configuration
      from Settings.

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

- [x] **10.1 — First-run guided start.** "Quick Start" tile on the
      Home view opens a single modal: endpoint URL + model + optional
      bearer token + OWASP category chips + ▶ Fire. Behind the scenes
      it creates an `openai-compat` Request (tag `quickstart`), an
      ad-hoc Scenario with the chosen OWASP refs, an Engagement, and
      fires `start_scenario_run`. Closes itself and routes to the
      engagement detail. One screen, dismissible, never blocks the
      full UI — respects principle 3.
- [ ] **10.2 — Installer size audit.** Verify the hamm0r binary
      (without analyzer) is a small download. Flag any crate
      that adds >1 MB to the binary.
- [ ] **10.3 — Cold-start benchmark.** Measure download → first
      report on macOS, Windows, Linux. Automate the measurement
      if feasible. Target: under 5 minutes on a standard workload.

---

## 11 · UI conventions & polish

Reference: `productVision.md` § UI conventions; `STYLEGUIDE.md`

- [x] **11.1 — Sidebar audit.** Verified the six entries match the
      vision (Home, Engagements, Requests, Scenarios, Library, Settings).
      The only mismatch was the "Prompts" label — renamed to "Library"
      in the sidebar tooltip and breadcrumb. View internal id stays
      `view-prompts` to avoid cross-reference churn (the CSS/JS already
      uses `lib-*` classes for this view).
- [x] **11.2 — "Fire from where it belongs".** Per-view fire surfaces
      wired: `▶ Fire` in the Request editor (calls `fireSelectedRequest`),
      `▶ Run` in the Engagement detail header (calls
      `fireSelectedEngagementScenario`). The Scenarios view intentionally
      has no fire button — scenarios fire only inside an engagement per
      `productVision.md`. The global topbar `▶` was removed in 6.3.
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
