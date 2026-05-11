# RequestPlan.md — first-class Requests + Scenarios UI

> **⚠️ Superseded by [`docs/RefactorPlan.md`](RefactorPlan.md).** The
> data model has since collapsed to four primitives (Request, Prompt,
> Scenario, Engagement); Targets, the Workbench view, the Engagement
> Wizard, and the legacy step-based Scenario flow have all been
> retired. Parts of this document — especially anything about the
> Target editor, Q-C step prompt snapshots, or Scenario steps — no
> longer match the codebase. Read it for historical context only.
> When `RefactorPlan.md` and this doc disagree, `RefactorPlan.md` wins.
>
> **Status (2026-05-06):** core feature landed. The new top-level
> **Requests** menu, structured + raw body editor, references-aware delete
> dialog, runner raw-body support, and `{{prompt}}` substitution work
> end-to-end. Q-C (steps reference prompts by id, resolved at run time)
> was deferred to its own follow-up after we discovered `prompt_text`
> flows through 15 files (runner, analyzer, history, reports). The
> Target editor was not refactored — its inline request editor still
> works and writes to the same `requests/` folder, so requests created
> via either path are interchangeable. The Scenario builder's request
> picker already existed and continues to work unchanged.
>
> What landed:
>
> - `BodyFormat::Raw` variant + YAML round-trip test
> - `storage::requests::references()` scanning Targets and Scenarios
> - Runner: `Raw` body bytes verbatim with `{{prompt}}` substitution;
>   `set_keep_trailing_newline(true)` so HTTP bodies stay byte-accurate
> - Tauri commands: `save_request_global`, `delete_request_global`
>   (force-flag + cascade clean of Target references), and
>   `list_request_references`
> - UI: new Requests sidebar entry, list + editor with structured/raw
>   tabs, delete-with-references confirmation dialog

Goal: promote **Request** to a top-level concept the user manages from its own
menu item, give it a proper editor (structured + raw-body mode), and let the
user assemble 1..N Requests into a Scenario that fires them sequentially.
`{{prompt}}` substitution stays as today: body only, prompt comes from the
prompt library.

This plan **does not touch** the analyzer, the run JSONL contract, or the
on-disk response format. Datamodel changes are limited to Target/Scenario YAML
and one new field set.

---

## Decisions (locked in by the user)

1. **Ownership model:** Request is an independent top-level entity. Targets and
   Scenarios reference Requests by `id`. Existing target files that own
   `request_ids` get migrated.
2. **Raw mode:** raw textarea for the **body only**. URL, method, headers stay
   in structured fields. No HTTP-wire-format parser in this iteration.
3. **Scenarios:** ordered list of 1..N Request references, executed
   sequentially when the scenario is started. Step-level `request_id` already
   exists — wire up the UI.
4. **`{{prompt}}` placement:** body only. URL/headers are out of scope for now.

---

## Decisions locked in (previously open questions)

- **Q-A — delete with references:** soft-warn, list the referencing Targets
  and Scenarios, require user confirmation before the delete proceeds.
- **Q-B — raw body persistence:** add `body.format: raw` and store the
  literal string in `body.content` as a YAML scalar. Runner sends the bytes
  verbatim after `{{prompt}}` substitution.
- **Q-C — DEFERRED to a separate PR.** Tracing `prompt_text` through the
  codebase showed it flows through 15 files (runner `RunStep`, analyzer
  pipeline, per-attempt history, report rendering) — far beyond the
  scenario file. Landing it cleanly requires its own focused commit with
  full migration + runner + analyzer + UI changes. For this PR the
  scenario-step schema stays as-is (`prompt_text` snapshot kept). The
  scenario builder UI gets the new Request picker but the prompt picker
  continues to write a snapshot, the same as today. **Step 7 below is
  scoped to the snapshot model. Q-C lands in a follow-up.**

---

## Invariants this plan must respect

- CLAUDE.md invariant 6: all file I/O via the `storage/` layer.
- CLAUDE.md invariant 7: run JSONL contract unchanged.
- CLAUDE.md invariant 11: no secrets in YAML; only env var names.
- CLAUDE.md invariant 12: append-only run logs.
- ProductVision: no DB, no framework migration, files are the source of truth.

---

## Checklist

### 1. Datamodel & migration

- [ ] Update `docs/Datamodel.md`:
  - Move the Request file section from "addendum" into the main schema.
  - Document that Requests are top-level and independent of Targets.
  - Add `body.format: raw` (string body, sent verbatim).
  - Update Target schema: `request_ids` becomes the canonical list; mark
    legacy `request_id` as a deprecated compat field.
  - Update Scenario schema: `steps[].request_id` is now the recommended way
    to point a step at a Request.
- [ ] Write a migration in `crates/storage/src/migrations/` (or extend the
  existing migration module) that, on first load:
  - Promotes legacy `request_id` on a Target to the head of `request_ids`.
  - Leaves orphan files alone; no destructive moves.
  - Idempotent — running twice is a no-op.
- [ ] Migration tests: legacy-only target, mixed target, already-migrated
  target.

### 2. Storage layer

- [ ] Confirm `crates/storage/src/requests.rs` `load_all` / `save` / `delete`
  cover the independent-Request case (they already do — no Target reference).
- [ ] Add `BodyFormat::Raw` variant in `crates/storage/src/types.rs` with
  serde rename `raw`.
- [ ] Update existing `BodyConfig` round-trip tests to cover the raw variant.
- [ ] Add a helper `requests::references(id)` that scans Targets and
  Scenarios and returns the list of files referencing a given request id —
  needed for delete-warning UX (Q-A).

### 3. Runner

- [ ] In `crates/runner/src/run.rs` (or wherever the body builder lives):
  when `body.format == raw`, skip JSON serialization and send the string as
  bytes. Apply `{{prompt}}` substitution against the raw string before send.
- [ ] Sanity test: `{{prompt}}` substitution in a raw body containing
  characters that would be invalid in JSON (quotes, backslashes, newlines).
- [ ] Confirm secret redaction in run JSONL still works unchanged for raw
  bodies (only `body_size` is logged — content stays in the response file).

### 4. Tauri command layer — Request CRUD (top-level)

- [ ] Add commands in `crates/hamm0r/src/commands/requests.rs` — independent
  of Target:
  - `list_requests()` → `Vec<Request>`
  - `get_request(id)` → `Option<Request>` (already exists, keep)
  - `save_request_global(request)` → `Request` (no `target_id`)
  - `delete_request_global(id, force: bool)` →
    `Result<(), {references: Vec<Ref>}>` (refuses unless `force` when
    references exist; UI passes `force=true` after the user confirms).
- [ ] Keep the existing target-scoped `save_request` / `delete_request` for
  the target-builder flow — they call into the global commands and additionally
  update the Target's `request_ids` list. No duplication of write logic.
- [ ] Register new commands in `lib.rs` and `capabilities.json`.

### 5. UI — top-level Requests menu

- [ ] Add "Requests" entry to the main nav in `ui/index.html`.
- [ ] New view: list of Requests (id, name, method, url, last-modified).
  Buttons: New, Edit, Duplicate, Delete.
- [ ] Editor view (single page, two tabs):
  - **Structured** tab: id, name, method, url, headers (key/value editor),
    body (JSON editor with `{{prompt}}` highlighting), auth, response
    extract, timeout. Adjacent to body: a "Detected placeholders" hint
    showing whether `{{prompt}}` was found.
  - **Raw body** tab: textarea for the body string. Switching tabs swaps
    `body.format` between `json` and `raw`. Warn on switch if the JSON body
    is non-empty and would be lost (or auto-serialize JSON → raw text).
- [ ] "Test request" button — reuses the existing `test_request` Tauri
  command. Allow the user to type a sample prompt for the substitution.
- [ ] Delete confirmation dialog: lists referencing Targets/Scenarios from
  `requests::references(id)`. (Resolves Q-A.)
- [ ] Styling consistent with existing views (`ui/style.css`). No new CSS
  framework.

### 6. UI — Target editor adjustments

- [ ] Target editor's "Requests" sub-section becomes a **picker** over the
  global Request list (multi-select + reorder), not an inline editor.
- [ ] "New Request" button in the picker opens the global Request editor
  pre-bound to add the new request to this target on save.
- [ ] Primary-request marking unchanged.

### 7. Scenarios — schema change + UI (Q-C alternative)

Scenario steps now reference prompts **by id** instead of carrying a
`prompt_text` snapshot.

Schema:

- [ ] In `crates/storage/src/types.rs`, change `ScenarioStep`:
  - Remove `prompt_text: String` (or keep it for one release as a deprecated
    fallback — see migration below).
  - Make `prompt_id: String` (currently `Option<String>`) required.
  - Keep `prompt_category: Option<String>` for disambiguation when two
    libraries expose the same `prompt_id` stem.
- [ ] Add `payload_id: Option<String>` if a step needs to pick one specific
  payload out of a multi-payload prompt; default = first payload.
- [ ] Document the new `ScenarioStep` schema in `docs/Datamodel.md` and bump
  the scenario file `version`.

Migration:

- [ ] Legacy scenarios with `prompt_text` and no `prompt_id`: on load, write
  the snapshot into a synthetic prompt file `prompts/_migrated-<scenario>-
  <step>.yaml` and rewrite the step to reference it. Idempotent. Logged.
- [ ] Migration test: legacy-only step, mixed step (both fields present —
  prefer `prompt_id`), already-migrated step.

Runner:

- [ ] Resolve `prompt_id` (+ optional `prompt_category`, `payload_id`)
  against the loaded prompt library at run start. Fail the step with a
  clear error if the prompt is missing.
- [ ] Confirm step ordering is preserved across `repeat` iterations.

UI:

- [ ] In the Scenario editor, each step gets:
  - A **Request** dropdown populated from the global Request list (filtered
    to the scenario Target's `request_ids` if a Target is set; otherwise
    all Requests).
  - A **Prompt** picker (library browser: pick a prompt file, optionally
    a specific payload).
- [ ] Steps remain ordered, can be reordered (drag or up/down buttons),
  added, removed. Minimum 1 step.
- [ ] If the referenced prompt is later deleted from the library, the
  scenario editor surfaces a "missing prompt" warning on that step (parallel
  to the delete-with-references flow in Q-A, but inverted: prompts are not
  reference-counted, so deletion is free; the scenario surfaces the dangling
  pointer at edit/run time).

### 8. Tests

- [ ] Storage: round-trip for `BodyFormat::Raw`.
- [ ] Storage: migration tests (see step 1).
- [ ] Runner: raw-body request with `{{prompt}}` substitution containing
  special characters.
- [ ] Tauri commands: `delete_request_global` without `force` returns a
  references error; with `force` it succeeds and updates referencing Targets.
- [ ] e2e (Playwright, via `/sync-tests`): create Request → attach to
  Target → build Scenario with 2 steps → run → see two attempts in the run
  log. Add to `tests/e2e/specs/`.

### 9. Docs

- [ ] `docs/Datamodel.md` — schema updates per step 1.
- [ ] `docs/Architecture.md` — note the elevation of Request to a
  top-level entity (one paragraph in the modules section).
- [ ] `ProductVision.md` — confirm no contradiction. (Expected: none.)
- [ ] CHANGELOG / commit message: explicit note that target-file format is
  unchanged on disk but the canonical list is now `request_ids`.

### 10. Definition of Done (mirrors CLAUDE.md)

- [ ] `cargo test --workspace` green.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all` applied.
- [ ] e2e specs added and passing locally.
- [ ] Migration verified against a real `~/hamm0r/` folder with legacy
  targets.
- [ ] Manual smoke: install fresh → create a Request in raw mode → attach
  to Target → run a Scenario with 2 steps → see 2 attempts in the run JSONL.

---

## Out of scope (do not pull in)

- HTTP wire-format paste-and-parse (Burp-style). Future iteration.
- `{{prompt}}` in URL/headers/query.
- Per-step header/body overrides on Scenario steps.
- Sharing Requests across hamm0r installs / import-export tooling.
- Any analyzer-side changes.
