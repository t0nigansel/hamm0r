# RefactorPlan.md — collapse Target, formalise auth chains, rebuild Scenario

The current data model evolved organically and now duplicates concepts.
This plan collapses it to four primitives — **Request, Prompt, Scenario,
Engagement** — and removes the rest. Driven by the conversation that
followed the first-class Requests work in `RequestPlan.md`.

> Before reading: this contradicts parts of `RequestPlan.md` (Q-C deferred,
> Scenario step prompt snapshots) on purpose. Q-C is no longer deferred —
> it is the point. When in doubt, this document wins.

---

## Current status (snapshot)

- ✅ **Phase 1** — drop the inline Request editor in Target config. Shipped.
- ✅ **Phase 2A** — additive schema (`Request.tag`, `ResponseConfig.bind`,
  `Scenario.request_ids` / `library` / `shared_session`). Shipped, with one
  intentional design deviation (see below).
- ✅ **Phase 2B** — runner request-dependency resolver, bind cache,
  `fire_chain`, `kind: prerequisite` markers in JSONL. Shipped.
- ✅ **Phase 2C** — `execute_matrix_run`: prompts × Requests Cartesian
  product with `shared_session` toggle. Shipped.
- ✅ **Phase 2D** — storage migration. Tag migration shipped. Auth-chain
  Request synthesis from `auth_acquisition.http_login` shipped (Q-H
  resolved per-run): synthesizes `<target.id>__login` Requests with a
  `bearer_token` bind and wires referenced chat Requests'
  `Authorization` header to `Bearer {{<id>.bearer_token}}`. Manual
  Authorization headers preserved. Template renderer learned `{{ env.X }}`
  substitution as a prerequisite; missing env vars render as empty
  string. Scenario v1→v2 translation intentionally not done — legacy +
  matrix coexist.
- ✅ **Phase 2E** — UI matrix-mode editor, backend matrix dispatcher,
  Home CTAs, sidebar trim. Shipped.
- ✅ **Phase 2F (UI)** — Wizard, Targets view, Workbench view, the
  Workbench target picker dialog, and **all of their JS controllers**
  deleted. ~3600 lines removed from `app.js` (Workbench state + judge
  helpers, target selector, prompt picker, OWASP coverage, mutations
  picker, fire-prompt, response cards, detail pane, findings drawer,
  Target editor, Target list+CRUD, debug helpers). `app.js` is now
  4054 lines (was 7663). `onDbOpen` and `checkDatabaseStatus` no longer
  call into deleted helpers. `cachedPrompts` replaces `wb.allPrompts`
  for the engagement-report OWASP coverage chip.
- ✅ **Phase 2F (backend)** — dead Tauri commands pruned. Removed:
  `save_target`, `delete_target`, `get_target_meta`, `save_target_meta`,
  `acquire_target_auth`, `test_target_connection`, target-scoped
  `save_request` / `delete_request`. Their helpers and DTOs went too.
  `targets.rs` shrank from 937 → 245 lines.
  **Kept:** `list_targets` (engagement-detail row → target name lookup),
  `get_request` (Requests view editor), `start_run` (per-result rerun
  path — uses the flat `execute_run` runner, independent of legacy
  step machinery).
- ✅ **Phase 2G — drop legacy step-based Scenario flow.** Steps are gone.
  - Runner: `ScenarioStep`, `ScenarioRunConfig`, `execute_scenario_run`
    deleted (~270 lines from `runner/run.rs`).
  - Storage: `Scenario.target_id`, `Scenario.steps`, `ScenarioStep`
    removed; old YAML files with those fields still load (serde drops
    unknown keys) and become inert matrix scenarios. `RequestReference::Scenario.step_id` removed too.
  - hamm0r commands: `start_scenario_run` is matrix-only;
    `start_transient_scenario_run` (Workbench's quick-fire), `save_steps`,
    and `list_target_requests` deleted along with the
    `load_target_and_request*` / `session_strategy_from_target` helpers.
  - UI: step section + step-dialog modal in `index.html`, the entire
    Step timeline / Step dialog JS, `loadTargetDropdown`,
    `loadScenarioRequestOptions`, `currentScenarioSteps`,
    `editingStepIndex`, `sc-target` / `sc-tags`, `save_steps` and
    `fire_prompt` api wrappers — all gone. The Scenarios view is
    matrix-only. `inferScenarioNameFromResults` no longer guesses from
    step prompt sequences (returns `—`; replacing with a proper
    `run → scenario_id` lookup is a follow-up).
  - Tests: 4 legacy scenario integration tests removed; storage
    `legacy_scenario_yaml_with_steps_still_parses` retained as a
    smoke that old YAML still loads.
- ✅ **Phase 2 docs** — `Datamodel.md` rewritten for matrix-only
  Scenarios (legacy `steps` / `target_id` documented as silently
  dropped on load); `Architecture.md` Target-editor reference fixed;
  `RequestPlan.md` carries a "superseded by RefactorPlan.md" banner.
- ✅ **Phase 2H — UX polish.** Scenarios view has a proper welcome /
  empty-state pane and list-empty hint. The Home → "Run a Scenario"
  CTA opens a real modal listing saved Scenarios with request +
  library summaries; non-runnable scenarios are rendered disabled.
  `RunHeader.scenario_id` records each matrix run's source
  Scenario, propagated through `RunSummary` to the engagement-detail
  header so the legacy step-sequence name guess is replaced with a
  real lookup.
- ❌ **Phase 2 tests** — e2e Playwright spec for the auth-chain
  matrix flow not yet written; only meaningful work remaining.

Workspace test count: **149 passing** (-8 from removing legacy step
integration + helper tests; zero functional regressions).

---

## Why

Today the user juggles five concepts: **Target, Request, Scenario,
Engagement, Workbench**. The conversation showed that:

- Target is a vague aggregator. Strip it apart and the only thing that
  doesn't already live elsewhere is `auth_acquisition` (a login pre-flight)
  and `session_strategy` (how requests share state).
- `auth_acquisition` is just another Request whose response feeds the next
  Request. That generalises to **request dependencies** — useful for auth,
  CSRF, OAuth, request signing, anything where one call's output is another
  call's input.
- Scenario today encodes ordered multi-step plans with prompt-text snapshots.
  In practice the only stateful multi-step pattern that matters now is
  "log in, then send the test request" — which is exactly what auth chains
  cover. Cross-user / context-poisoning attacks can be modelled as separate
  scenarios with a shared-session toggle when needed.
- Workbench is a free-play view from when Scenarios were heavyweight. The
  "test one prompt against this Request" use case is already covered by
  the Request editor's **Test request** button.
- The wizard duplicates the regular UI. Once Scenario captures
  "Requests + library subset", the wizard adds nothing the regular masks
  don't already do.

**Endpoint:** sidebar = `Home · Requests · Prompts · Scenarios ·
Engagements · Settings`. Six views, each with one job.

---

## Decisions locked in (and how they actually shipped)

- ✅ **Drop Target as a top-level entity.** Sidebar nav entry, Targets
  view DOM, Target editor JS (~1500 lines), Target list+CRUD JS, and the
  associated debug helpers all deleted.
- ✅ **Drop the Workbench view.** Sidebar nav entry, Workbench view DOM,
  the wb-target-picker dialog, and all Workbench JS (state, judge helpers,
  prompt picker, fire-prompt, detail pane, findings drawer — ~1700 lines)
  deleted.
- ✅ **Drop the Engagement Wizard.** Wizard tile removed from Home and
  replaced with "Run a Scenario" + "Open Scenarios" CTAs. Modal HTML
  and ~970 lines of wizard JS (state machine, step renderers, listeners)
  deleted. The "+" buttons on Home and Runs now open the lightweight
  engagement-dialog directly.
- ✅ **Request dependencies (auth chains).** `response.bind` declares a
  bind name; `{{<request_id>.<bind_name>}}` interpolation works in any
  string field. Runner builds the DAG, fires prereqs, caches values,
  substitutes at fire time. Cycles detected statically.
  - **Design deviation:** the plan said put `bind` *inside* each
    `ExtractConfig` variant. We put it on `ResponseConfig` next to
    `extract` instead, because changing `ExtractConfig::Raw` from a unit
    variant to a struct variant would have broken ~20 construction
    sites. User-facing YAML difference is one indentation level; the
    semantics are identical.
- ✅ **Scenario = N Requests × library subset, fired as a matrix.**
  Implemented additively alongside legacy `steps`. Runner picks the
  matrix path when both `request_ids` and `library` are populated.
  Legacy step-based scenarios continue to work unchanged via
  `execute_scenario_run`.
  - **Design deviation:** the plan said drop `target_id`, `steps`,
    `prompt_text` snapshots from the Scenario schema. We added the
    matrix fields *additively* and left the legacy fields in place
    so old scenarios keep loading and firing. This trades schema
    cleanliness for zero-risk backward compatibility — worth it given
    nothing forces a hard cutover.

---

## Open questions — current state

- **Q-D — Tag granularity.** ✅ **Resolved as planned:** single optional
  string. Multi-tag is YAGNI.
- **Q-E — Library subset shape.** ✅ **Resolved as planned:** both
  `owasp_refs: Vec<String>` and `categories: Vec<String>`. The matrix
  resolver matches a prompt entry if either list contains it.
- **Q-F — DAG cycle handling.** ✅ **Resolved as planned:** static
  detection in `runner::deps::topological_order`, error names the cycle
  members.
- **Q-G — Existing multi-step scenarios.** ⚠️ **Resolved differently.**
  We did NOT auto-migrate. Legacy step-based scenarios coexist with the
  new matrix shape because the runner picks the path at fire time.
  Users who want matrix mode build a new Scenario manually. No
  `<file>.yaml.bak` mechanism was needed because nothing got rewritten.
- **Q-H — Auth-chain semantics vs. existing http_login flow.**
  ✅ **Resolved:** per-run is acceptable. The new auth-chain Request
  synthesis (Phase 2D, deferred) will fire login on every run — no
  keychain-side caching. Existing `auth_acquisition.http_login` data
  becomes a per-run chain when migrated. Implementation still pending,
  but unblocked.

---

## Invariants this plan must respect

(From `CLAUDE.md` and `ProductVision.md`. Same constraints as before.)

- All file I/O via `storage/`. No direct `open()` outside it.
- Run JSONL contract unchanged. Auth-chain prerequisite calls produce
  their own attempt records (with a marker so the UI can group them) but
  the schema doesn't change.
- No secrets in YAML. Env var names only. Bind values stay in memory.
- No DB. Files remain the source of truth.
- Append-only run logs.
- Old engagement folders must remain readable.

---

## Phase 1 — drop the inline Request editor in Target config ✅

Independent of the rewrite. Pure UI cleanup. Shipped.

- [x] Replace the inline request editor with a multi-select picker over
      the global Request list.
- [x] Hide the request-form fields from the Target form. Inline editor
      kept in the DOM (display:none) so existing save/test-request JS
      stays functional. Will be removed entirely with the Target view.
- [x] Test request still works — tests the picker's primary Request.
- [ ] e2e smoke (Playwright). **Not done.** Folded into the Phase 2 e2e
      work.

---

## Phase 2 — the big rewrite

### 2.1  Storage / schema

- [x] **Request schema:** `tag: Option<String>` added.
- [x] **Response schema:** `bind: Option<String>` added (next to
      `extract`, not inside ExtractConfig — see deviation note above).
- [x] **Scenario schema additions:** `request_ids`, `library`,
      `shared_session` added *alongside* legacy fields. Schema `version`
      not bumped because legacy + new coexist.
- [x] Datamodel docs updated: matrix scenario example, bind/
      interpolation contract, body formats incl. `raw`.
- [ ] **Engagement schema:** `target.request_id` → `scenario_id`
      rename. **Not done — turns out not needed.** The existing field is
      already a Request id; the field name is just historically
      misleading. Not load-bearing. Leaving as-is until Target is
      fully removed (Phase 2F-rest), at which point we'll rename.
- [x] **Migration scaffold** in `crates/storage/src/migrations/`.
- [x] **Tag migration** (`tag_requests_from_targets`): copies
      `Target.name` into each referenced Request's `tag` field.
      Idempotent. 7 tests.
- [ ] **Auth-chain Request synthesis** from
      `auth_acquisition.http_login`. **Deferred** pending Q-H.
- [ ] **Scenario v1 → v2 translation.** **Deliberately not done.**
      Legacy `steps`-based scenarios keep working via
      `execute_scenario_run`; the matrix path is opt-in. No backups
      written because nothing gets rewritten.
- [x] Migration tests: idempotency, mixed-state, multi-target shared
      Request, no-overwrite-existing-tag, orphan refs counted not fatal,
      empty Target name skipped, missing dirs tolerated.

### 2.2  Runner — request dependencies ✅

- [x] DAG resolver in `runner::deps`: extracts `{{<id>.<bind>}}`
      references from URL/headers/body/JSON-content recursively;
      tolerates Jinja whitespace and filters; ignores bare `{{prompt}}`.
- [x] Topological sort (Kahn's algorithm). Detects cycles, validates
      that every reference points to a known Request and a declared
      bind.
- [x] `BindCache` type alias and `template::render_with` for substitution.
- [x] `fire_chain(http, registry, target_id, payload, …, &mut cache)`
      orchestrates prereqs + target. Skips already-cached prereqs (this
      is what makes `shared_session: true` semantics work).
- [x] `adapter::execute_with_session_and_binds` (sibling of the existing
      `execute_with_session`) plumbs the cache through render calls.
      Existing public API preserved.
- [x] Run JSONL: `RunAttempt.kind: Option<String>` field added.
      Prerequisite firings get `Some("prerequisite")`; targets get
      `None`.
- [x] Tests: simple chain, two-deep chain, cycle detection,
      unknown-id error, undeclared-bind error, end-to-end wiremock
      auth chain.

### 2.3  Runner — Scenario as matrix ✅

- [x] `MatrixRunConfig` + `execute_matrix_run` (sequential, prompts
      outer × requests inner).
- [x] `shared_session: true` shares one BindCache for the whole run;
      `false` uses a fresh cache per cell.
- [x] Cancellation honoured between cells and via `tokio::select!`
      while a chain is in flight.
- [x] Tests: matrix N×M with shared_session true (login fires once),
      matrix with shared_session false (login fires per cell).
- [ ] **Library resolution lives in the Tauri command layer**, not the
      runner. The runner takes a flat `Vec<Payload>`. Acceptable for now
      but worth noting if we ever build a CLI runner that needs the same
      resolution.
- [ ] **Parallelism within a matrix run.** Today everything is
      sequential. Out of scope for the refactor; flagged as a follow-up
      perf issue.

### 2.4  Tauri commands

- [x] **Drop legacy commands:** done. `list_targets` and its
      `commands/targets.rs` module are removed; the rest of the
      Target-scoped commands had already been deregistered. The
      legacy `RequestReference::Target` variant in
      `crates/storage/src/requests.rs` stays — migration v2 still
      reads legacy Target YAML at upgrade time, and `references()`
      uses it to warn the user before deleting a Request that an
      un-migrated Target points at.
- [⚠️] **`start_run` is NOT dead** (RefactorPlan was wrong). It
      backs the Engagement detail "Re-run" button (see
      `ui/js/app.js`, `start_run` call inside the rerun handler).
      Keeping it. The flat-payload shape pre-dates `start_scenario_run`
      but still serves a real UI flow.
- [x] **`start_scenario_run` is now the only matrix launcher.** It
      detects `request_ids` + `library` and dispatches to the new
      `dispatch_matrix_scenario`; legacy step-based scenarios still
      route to `execute_scenario_run`.
- [x] No new commands needed for auth chains; runner picks up
      bind/source from Request YAML directly.

### 2.5  UI

- [x] **Sidebar:** Targets and Workbench buttons hidden via
      `style="display:none"` (full removal pending).
- [x] **Home:** new tiles "Run a Scenario" and "Open Scenarios"
      replace "Start Engagement" / "Open Workbench".
- [⚠️] **Run a Scenario picker** uses `window.prompt` for v1.
      Functional but ugly. Proper modal picker is a follow-up.
- [x] **Empty-state hints** for Requests and Scenarios views shipped
      with the polish pass (see "Empty-state hints" entry below).
- [x] **Requests view:** `tag` field added; `bind` field on the
      response-extract section with a multi-line help blurb showing
      the `{{login.bearer_token}}` example and explaining the chain
      semantics.
- [x] **Scenarios view:** "Matrix" panel below Steps. Request
      multi-select, OWASP A01–A10 chips, category chips (auto-built
      from prompt library), shared_session checkbox, live counter.
- [x] **Engagements view:** unchanged.
- [x] **Wizard modal HTML + JS state machine + WIZARD_SCENARIOS
      preset list:** removed. Only explanatory comments remain in
      `ui/js/app.js` pointing at this plan.
- [x] **Targets view + Workbench view DOM + JS:** removed. A handful
      of legacy CSS class names (`target-test-result`,
      `target-request-picker`) still exist but are inert — left for a
      cosmetic follow-up.

### 2.6  Tests

- [x] All storage migration tests (Phase 2D).
- [x] Runner tests for auth chain + DAG (Phase 2B): 8 unit + 3
      template + 1 wiremock integration.
- [x] Runner tests for matrix expansion (Phase 2C): 2 wiremock
      integration tests covering shared_session true and false.
- [ ] **End-to-end Playwright spec.** Not written. The plan says:
      create Request, create login-chain Request that binds a token,
      create Scenario referencing both with a library subset, fire,
      observe N×M attempts in the run JSONL with one prerequisite
      firing.

### 2.7  Docs

- [x] `docs/Datamodel.md`: matrix scenarios documented; bind/
      interpolation contract added; body formats (incl. `raw`)
      enumerated; tag noted.
- [x] `docs/Architecture.md`: verified. No Target-entity or Wizard
      references remain (only the generic "target system" usage).
- [x] `docs/ProductVision.md`: verified. No contradictions with the
      collapsed four-primitive model.
- [x] **`RequestPlan.md` retired.** Deleted from the repo; this
      document is now the single source for the Request/Scenario
      refactor.

---

## What's still open — work-list

1. **End-to-end Playwright spec** for the auth-chain matrix flow.
   Closes the Definition of Done for Phase 2. Owned by the
   `hamm0r-testmanager` MCP workflow (`/sync-tests`), not by ad-hoc
   spec writing.

The polish items below all shipped:

- ✅ **"Run a Scenario" modal picker.** New `#run-scenario-picker`
  modal lists saved Scenarios with request count + library subset
  summary; non-runnable scenarios (no Requests / no library) are
  rendered disabled with a tooltip pointing the user at the editor.
  Replaces the `window.prompt` shim.
- ✅ **`run → scenario_id` lookup.** `RunHeader` learned an optional
  `scenario_id` field; the matrix dispatcher writes it on every run.
  `RunSummary` and `summarize_run` propagate it to the UI;
  `lookupScenarioNameForRun` replaces the dead step-sequence
  heuristic. Ad-hoc rerun runs render `—`; runs whose source
  Scenario was deleted show `(deleted scenario: <id>)`.
- ✅ **Empty-state hints.** Scenarios view's right-pane welcome
  explains the matrix model and offers a one-click "Create your
  first scenario"; the list panel shows "No scenarios yet" when
  empty. Requests view already had a parallel hint; left as-is.
- ✅ **Docs updates.** `Datamodel.md` rewritten for the matrix-only
  Scenario shape, with a short "legacy YAML" note about
  `target_id` / `steps` being silently dropped on load. `scenarios/`
  added to the directory layout. `Architecture.md` Target-editor
  reference replaced with Request editor.
- ✅ **`RequestPlan.md` retired.** Removed from the repo; this
  document is the single source for the refactor.

---

## Out of scope

- Multi-tag on Requests (Q-D).
- Saved "test plans" as a separate entity (Engagement stays a workspace).
- Cross-user / multi-persona orchestration (model as separate Scenarios
  with `shared_session` if it ever comes up).
- A web/cloud edition. This is local-first per `ProductVision.md`.
- A "compare two runs side-by-side" view. Worth doing but its own thing.
- Parallelism inside a matrix run. Sequential is fine for v1.

---

## Migration risk and mitigation — actual outcome

The conservative design choices that paid off:

1. **Schema additions, not replacements.** `Scenario.steps` and
   `Scenario.target_id` weren't dropped. Old scenarios load and fire
   today. Zero migration risk.
2. **No mass YAML rewrites.** The tag migration is the only one that
   touches existing files, and it only sets a previously-absent field.
   No `<file>.yaml.bak` mechanism was needed.
3. **Hide-don't-delete in the UI.** Targets and Workbench views still
   exist in the DOM; the user can revert by removing two `display:none`
   tweaks if anything goes wrong.

The remaining deletion work in items 3–5 of the work-list IS riskier
because it removes code paths that legacy data may still reach via
stale state. Doing each as its own change (with focused tests) keeps
the blast radius small.

---

## Definition of Done (Phase 2)

- [x] `cargo test --workspace` green (151 tests pass).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
      Last verified clean modulo a pre-existing 9-tuple warning in
      `targets.rs` (which goes away with Target deletion).
- [ ] e2e Playwright spec passing.
- [ ] Manual smoke: install fresh hamm0r → create Request with `bind:
      bearer_token` → create chat Request that references it →
      build matrix Scenario → Fire from Home → see N×M attempts +
      1 prerequisite attempt in the run JSONL.
- [ ] Smoke against an existing `~/hamm0r/` folder: launch app, see
      tag migration applied to existing Requests, fire a legacy
      step-based Scenario unchanged, build a new matrix Scenario and
      fire it.
