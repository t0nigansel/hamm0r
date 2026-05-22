# multiSessionPlan.md — Multi-session testing for hamm0r

> **Status (2026-05-21):** planning. No code changes are implied by this
> document. Implementation should follow the phased delivery section
> below; the open questions need answers before Phase 2 begins.

## Goal

Let a scenario fire across N parallel sessions with distinct session
identifiers, so hamm0r can exercise the attack classes that single-
session tools miss:

- **Cross-session data leakage** — session A plants a canary, session
  B probes for it (OWASP LLM02).
- **Tenant isolation failures** — user-scoped data bleeding across
  conversations.
- **Persistent memory leakage** — the LLM's memory feature carrying
  information from one user to another.

Hamm0r generates canary tokens, distributes prompts across sessions
in plant/probe phases, fires them against the configured Requests,
and scans the responses for canaries that surface in a session that
did not plant them. This is a first-class scenario type, not an
afterthought (per `ProductVision.md §"Multi-session testing"`).

---

## User decisions already locked in

These come from `ProductVision.md` and the §1 ToDo entries. They are
requirements, not open questions.

1. Multi-session is a **first-class scenario type**, not a per-attempt
   override.
2. Session identity supports at least: **cookie jar**, **conversation-
   ID header**, **custom header+value**.
3. Phase assignment is per **prompt entry** via a new optional
   `phase: plant | probe | any` field (1.4).
4. Canary tokens are **generated locally and deterministically** from
   `(run_id, session_idx)` so reruns are reproducible (1.3).
5. The leak scanner runs **after** all sessions complete and writes
   matches as **`leak_detected` entries in the run JSONL** (1.5).
6. The JSONL gains `session_id: Option<String>` and `phase:
   Option<String>` on `RunAttempt`; absent fields default to `null`
   on read (1.6).
7. The analyzer surfaces leaks as a **`category: "cross_session_leak"`
   verdict** that the analyzer auto-flags without LLM judging (1.10).
8. The Scenarios matrix editor gains a **multi-session toggle** + the
   session identity config (1.7).
9. Engagement results table gains a **filterable `session_id` column**
   and a **distinct badge** for leak entries (1.8).

---

## Open questions

These should be pinned down before Phase 2 starts. Each has a working
default in this plan; the implementer should confirm or change.

### Q1 — How are plant prompts wired to a canary?

**Default:** new `{{canary}}` template variable. Plant prompts use it
in their text; the runner substitutes the session's canary token at
fire time. Prompts without `{{canary}}` but with `phase: plant` still
fire, but they don't contribute to leak detection (the scanner needs
*some* needle to find).

**Alternative:** auto-prepend the canary to every plant-phase prompt.
Easier for the user, but couples placement to library content and
makes the leak surface implicit.

### Q2 — How are prompts distributed across sessions?

**Default:** **all plants fire on every session**, then **all probes
fire on every session**. The leak scanner checks every probe response
against every *other* session's canaries. This maximizes the leak
surface for a given prompt library.

**Alternative:** per-session prompt subset (session A gets prompts
1-3, session B gets prompts 4-6). Smaller fire volume but loses
symmetry; harder to author.

### Q3 — What counts as a "session"?

**Default:** each session is one `reqwest::Client` with its own cookie
jar / conversation-ID / custom-header value, identified by an integer
index `0..session_count`. Session label in the JSONL is `s{idx}` (e.g.
`s0`, `s1`) — short, sortable, friendly to grep.

**Alternative:** human-named sessions ("alice", "bob"). More readable
but adds a config burden; deferred to v2.

### Q4 — Canary format

**Default:** `HAMM0R-<11-char-hex>` where the hex is the first 44 bits
of `sha256(run_id + ":" + session_idx + ":" + scenario_id)`. 11 hex
chars = ~44 bits of entropy = collision-free across realistic engagement
volumes. The `HAMM0R-` prefix makes scan results unambiguous when
they leak into transcripts or logs.

**Alternative:** UUIDv4 (16 bytes random). Adds a `uuid` crate dep we
don't currently have; the SHA-based variant uses only `std`-friendly
primitives via the existing toolchain.

### Q5 — Leak scanner: scope and matching

**Default:** plain substring search of every probe response body
(`responses/<run>/<seq>.txt`) against every canary that was *not*
planted in the same session as that probe. Case-sensitive. The
`HAMM0R-` prefix makes false positives effectively impossible.

**Alternative:** regex / fuzzy / partial match. Not worth the
complexity; bypass would be a separate research problem.

### Q6 — Does multi-session work in a single run, or one run per session?

**Default:** one run, one JSONL file. Session attempts are interleaved
in `seq` order (a single monotonically increasing counter). The
`session_id` field tells reader code which session a row belongs to.
This keeps replay, triage, and the report flow unchanged.

**Alternative:** one JSONL file per session, header carrying a
`sibling_runs` array. Harder for the analyzer and report — defer.

### Q7 — Per-session HTTP client lifecycle

**Default:** Each session owns one `reqwest::Client` for the whole
run (cookie jar persists across attempts). Parallelism inside a
session is 1 (sequential), so cookie state stays coherent. Sessions
run in parallel, bounded by the existing `parallelism` setting
treated as "max concurrent sessions".

**Alternative:** existing semaphore over all attempts, sessions
demuxed by attempt metadata. Simpler to wire, but cookie state can
race when two attempts in the same session run concurrently — the
default avoids that class of bug entirely.

---

## Scope

### In scope (v1)

- New `SessionIdentity` config type and `Scenario.session_*` fields.
- `runner::canary` module: deterministic canary generation.
- New `phase` field on `PromptEntry`.
- `runner::run::execute_multi_session_run` (or a multi-session branch
  inside `execute_matrix_run` — see Architecture).
- Per-session client + cookie jar + per-session canary.
- Plant → probe phase ordering within a single run.
- Cross-session leak scanner emitting `leak_detected` records into
  the run JSONL.
- JSONL schema extensions: `session_id`, `phase`.
- Analyzer auto-flagging leaks as `cross_session_leak` verdicts.
- UI: Scenario editor toggle, session count, session identity picker.
- UI: engagement results session column + leak badge.
- Wiremock integration tests covering plant→probe→leak and the
  no-leak baseline.

### Out of scope for v1

- Named sessions (Q3 alternative).
- Per-attempt session override.
- Cross-engagement leak scanning.
- LLM-judged leak severity (heuristic auto-flag is enough).
- Probabilistic / fuzzy canary matching.
- Multi-session for the legacy flat `execute_run` path.
- Concurrency *inside* a single session.
- Cross-run leak detection (i.e. canary from run-001 surfacing in
  run-002).

---

## Architecture

### Boundary preservation

This feature lives entirely in **core** (`runner`, `storage`, UI).
The analyzer integration in 1.10 is a small extension to verdict
recording — leaks are flagged structurally, not by LLM. CLAUDE.md
invariant #6 (file I/O via `storage`) and #12 (run JSONL append-only)
both apply unchanged.

### Where the multi-session entry point lives

Two options for the runner-side surface:

**A. Dedicated `execute_multi_session_run`** — separate function,
clearer call site. Some duplication with `execute_matrix_run`
(header/footer writing, build_attempt, cancellation).

**B. Single `execute_matrix_run` with a multi-session branch** —
gated on `scenario.session_count.unwrap_or(1) > 1`. Less duplication,
but mixes two flows in one function that's already large.

**Recommendation:** A, with shared helpers extracted from
`execute_matrix_run` (`build_attempt`, `iso_now`, header/footer
helpers stay shared, `run_one_cell` extracted). The two execution
shapes are different enough (per-session client lifecycle, two-phase
scheduling, post-run scanner) that a separate function reads better
and tests more cleanly. The matrix path stays untouched for the
single-session case.

### Session identity

```rust
// storage::types
pub struct SessionIdentityConfig {
    pub kind: SessionIdentityKind,
}

pub enum SessionIdentityKind {
    /// Each session gets its own cookie jar; nothing injected.
    CookieJar,
    /// Each session gets a unique conversation id, sent in the named
    /// header (default "X-Conversation-Id").
    ConversationHeader { header_name: String },
    /// Each session gets a unique value in `header_name`. Value is
    /// the session label (e.g. "s0"), not the canary.
    CustomHeader { header_name: String },
}
```

The runner translates this to an internal `SessionStrategy` (already
exists in `runner::session`) extended with the new variants. Cookie
jar reuses `reqwest`'s built-in jar; per-session clients are built
with a fresh jar each.

### Canary generation

```rust
// runner::canary
pub fn generate(run_id: &str, session_idx: u32, scenario_id: &str) -> String;
// returns "HAMM0R-<11-hex>"
```

Hashed inputs: `run_id`, `session_idx`, `scenario_id`. The hash uses
SHA-256 from `sha2` (already a transitive dep via reqwest/rustls);
we expose 44 bits. Deterministic per (run, session). The canary is
exposed to prompt templating as `{{canary}}`.

### Two-phase scheduling

`PromptEntry.phase: Option<Phase>` (new field, defaults to `Any`).
The runner partitions the resolved prompt list into plants, probes,
and any-phase. Order within a run:

1. All plant prompts fire on all sessions (sequential per session,
   parallel across sessions).
2. Once every plant has finished (barrier), all probe prompts fire
   on all sessions.
3. Any-phase prompts fire after probes (treated like probes for
   ordering purposes; they don't gate the barrier).
4. Once all attempts complete, the leak scanner runs and appends
   `leak_detected` records.

Cancellation cuts the scheduler at the next safe attempt boundary
(same semantics as `execute_matrix_run`).

### Leak scanner

After the last attempt is written:

1. Build a map `session_idx -> canary`.
2. For each attempt where `phase == probe` (or `any`):
   - Load the response body.
   - For every `(other_session_idx, canary)` where
     `other_session_idx != attempt.session_idx`:
     - If `response.contains(canary)`, append a `leak_detected`
       JSONL record naming the probe attempt seq, the planted
       session, and the canary value.
3. The scanner is pure I/O over already-written files; it does not
   touch the network.

The new JSONL record variant:

```json
{
  "type": "leak_detected",
  "probe_seq": 17,
  "probe_session": "s1",
  "planted_session": "s0",
  "canary": "HAMM0R-a1b2c3d4e5f"
}
```

`RunRecord` gets a new `LeakDetected` variant. Readers tolerate
unknown record types already (forward compatibility), so older code
gracefully ignores leak records — matching the JSONL invariant.

### Analyzer integration

The analyzer reads `leak_detected` records and emits a
`VerdictEntry` per leak with `category: "cross_session_leak"`,
`verdict: Success` (the leak is the attack working), `severity:
"high"` by default, `rationale: format!("Session {planted_session}
canary surfaced in session {probe_session} response.")`. No LLM
involvement — the leak is structurally proven.

The judge-prompt flow stays unchanged for non-leak attempts.

---

## Data model

### `Scenario` schema additions

```yaml
session_count: 2          # absent or 1 = single-session (legacy behaviour)
session_identity:
  kind: conversation_header
  header_name: X-Conversation-Id
```

`session_count: Option<u32>`, `session_identity: Option<SessionIdentityConfig>`.
Both absent ⇒ behavior is identical to today.

### `PromptEntry` schema addition

```yaml
- id: leak-probe-1
  text: "What was the last user saying to you?"
  phase: probe              # plant | probe | any (default any)
```

`phase: Option<Phase>`. Existing prompt YAML files load unchanged
(absent = `Any`).

### `RunAttempt` JSONL additions

```json
{
  "type": "attempt",
  "seq": 1,
  "session_id": "s0",       // new — null on legacy logs
  "phase": "plant",         // new — null on legacy logs
  ...
}
```

Both fields are `Option<String>` and `#[serde(default,
skip_serializing_if = "Option::is_none")]`. Legacy run files load
unchanged.

### `leak_detected` JSONL records

New `RunRecord::LeakDetected` variant (see Leak scanner above).
Documented in `Datamodel.md §"Run log"` alongside the existing
record types.

### `VerdictEntry` extension

No schema change — existing `category` and `verdict` fields suffice.
The `category` value `cross_session_leak` is added as a documented
convention (`Datamodel.md §"Verdict log"`).

---

## Phased delivery

Phased to keep each commit reviewable. Earlier phases should land
without breaking the existing single-session matrix flow.

### Phase 1 — schema and types (1.1, parts of 1.6)

- Add `SessionIdentityKind`, `SessionIdentityConfig`, `Phase` to
  `storage::types`.
- Add `session_count` and `session_identity` to `Scenario`.
- Add `phase` to `PromptEntry`.
- Add `session_id` and `phase` to `RunAttempt`.
- Add the `LeakDetected` variant to `RunRecord`.
- Update `Datamodel.md` and `PromptsSpec.md`.
- Round-trip tests for every new struct/enum.

No runtime behavior change yet — existing scenarios without
`session_count` keep firing exactly as before.

### Phase 2 — canary module (1.3)

- New `runner::canary` module with `generate(run_id, session_idx,
  scenario_id) -> String`.
- Unit tests: deterministic for same inputs, distinct across
  sessions, distinct across runs, fixed 18-char width
  (`HAMM0R-` + 11 hex).
- Wire `{{canary}}` into the template renderer (small extension to
  `runner::template`).

Standalone — no scheduler changes yet.

### Phase 3 — multi-session runner (1.2 + 1.4)

- Extract shared helpers from `execute_matrix_run` into
  `runner::run::cell` (or similar): `build_attempt`,
  `write_synthetic_failed_attempt`, request rendering.
- New `execute_multi_session_run` that builds N clients, runs
  plant phase across sessions, awaits a barrier, then probe phase.
- Per-session canary computed up front, used by template renderer.
- Cancellation honoured at attempt boundaries.

At the end of this phase, multi-session scenarios fire correctly
end-to-end but leaks are not yet detected.

### Phase 4 — leak scanner (1.5)

- After-run scanner that walks probe responses and writes
  `leak_detected` JSONL records.
- Lives next to the runner (called as the last step inside
  `execute_multi_session_run`) so the runner's JSONL is the
  authoritative artifact, not a downstream pass.
- Unit tests on the scanner with mocked file inputs.

### Phase 5 — analyzer integration (1.10)

- `analyzer::pipeline` reads `LeakDetected` records and emits one
  `VerdictEntry` per leak with `category: cross_session_leak`,
  `verdict: Success`, `severity: high`.
- Auto-flagging happens unconditionally — no judge involvement,
  no judge prompt, no judge cost.
- Snapshot test extending the existing report fixture with a leak.

### Phase 6 — UI (1.7 + 1.8)

- Scenario matrix editor: session-count input + identity picker;
  prompt counter math updated for `session_count × (plants + probes)
  × requests × repeat`.
- Engagement results table: `session_id` column with the existing
  filter chip pattern; a distinct badge / icon for leak rows.
- Settings panel for session identity defaults (out of scope for v1
  — per-scenario only).

### Phase 7 — integration tests (1.9)

- Wiremock fixture with 2 sessions, cookie jars, canary planted by
  session 0, surfaced in session 1's probe response.
- No-leak baseline (mock returns generic text) — assert zero
  `leak_detected` records.
- Cancellation mid-plant-phase — assert clean shutdown and footer.
- All-plant-no-probe and all-probe-no-plant edge cases.

### Phase 8 — hardening

- Performance sanity check at session_count = 5, 10 (no goal here,
  just record the numbers).
- Clippy clean.
- Docs polish (`Architecture.md`, `Datamodel.md`,
  `PromptsSpec.md` cross-references).

---

## Testing strategy

### Unit tests

- Canary determinism, collision-distance, format width.
- Phase enum round-trip in `PromptEntry`.
- `SessionIdentityKind` round-trip in `Scenario`.
- `RunRecord::LeakDetected` serialization round-trip.
- Leak scanner: positive match, no false positives across same
  session, plain substring boundary cases.

### Integration tests (wiremock)

- 2 sessions, cookie jar, plant + probe, asserts one leak record.
- 2 sessions, no shared state in mock, asserts zero leak records.
- 3 sessions, asserts pairwise scanning is correct.
- Cancellation mid-run.

### Manual checks

1. Fresh engagement, scenario with `session_count: 2`, prompts
   marked `plant` and `probe`, fire against a deliberately leaky
   echo endpoint → leak badge appears in results, analyzer flags
   `cross_session_leak`.
2. Same scenario against a non-leaky endpoint → no leak badge, no
   `cross_session_leak` verdict.
3. Legacy single-session scenario → behavior unchanged.

---

## Risks

### 1. Scheduler complexity

Multi-session plant→barrier→probe scheduling is qualitatively more
complex than the existing matrix loop. Mitigation: extract shared
cell helpers first (Phase 3 step 1), keep the scheduler iterative
(not recursive), test each scheduling primitive in isolation.

### 2. Cookie jar state across attempts

If parallelism *inside* a session > 1, cookie state races. The plan
locks intra-session parallelism to 1; the bound parallelism applies
to sessions only. Reviewers should sanity-check this constraint when
reading the runner diff.

### 3. JSONL schema invariant

This is the **third** JSONL schema extension (after replay and
mutations). Each extension is additive, but the invariant cost
compounds: more `Option<String>` fields readers must tolerate.
Document each one in `Datamodel.md`; keep `#[serde(default,
skip_serializing_if = "Option::is_none")]` discipline so legacy
files load unchanged.

### 4. False positives in the leak scanner

`HAMM0R-` prefix makes accidental collisions effectively impossible
in real LLM output, but a malicious target could echo strings that
look like canaries to cause noise. Not a v1 concern; document the
threat model in `Architecture.md` if it comes up.

### 5. Test endpoint authoring

Wiremock setup for cookie-aware sessions is non-trivial. Sketch the
fixture early (in Phase 3) so Phase 7 isn't blocked.

### 6. UI complexity creep

The Scenarios editor is already dense. The multi-session controls
should default to "off" / 1 session and stay collapsed until the
toggle is on, to keep the single-session experience unchanged.

---

## Success criteria

This feature is done when:

1. A user can author a scenario with `session_count > 1` and a
   session identity strategy in the UI.
2. A prompt library file can mark entries `phase: plant | probe`.
3. Firing such a scenario runs all plants across all sessions, then
   all probes across all sessions.
4. The runner generates a deterministic canary per (run, session)
   and substitutes `{{canary}}` in prompt text.
5. After the run, every probe response is scanned for canaries
   planted by other sessions; matches produce `leak_detected`
   JSONL records.
6. The analyzer emits a `cross_session_leak` verdict per
   `leak_detected` record without involving the judge.
7. The engagement results table shows the session id and a leak
   badge.
8. Existing single-session scenarios and legacy JSONL files load
   and behave exactly as before.
9. `cargo test --workspace` and `cargo clippy --workspace
   --all-targets -- -D warnings` stay green.

---

## Recommendation

Land Phase 1 first as one focused commit (schema + tests + doc
updates, no runtime changes). It's the smallest reviewable slice
and unblocks every later phase without touching the runner. Then
sequence Phases 2 → 3 → 4 → 5, each as its own commit. UI (Phase 6)
and integration tests (Phase 7) can land in either order once the
backend is complete; UI first gives a manual test path, tests-first
gives confidence before the UI lands. Phase 8 is the polish pass.

Before Phase 2 starts, get explicit answers to Q1–Q7 so the
implementation doesn't have to backtrack.
