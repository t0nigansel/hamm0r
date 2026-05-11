# Settings Modal — Build-Out Plan

## Context

The Settings button at [ui/index.html:93-99](../ui/index.html#L93-L99) opens
a modal (historically named `analyzer-modal`) that today only exposes
**Logging** controls and **Analyzer** activation. Several config fields
already exist in [crates/storage/src/types.rs:303-345](../crates/storage/src/types.rs#L303-L345)
but have no UI surface (`ui.theme`, `analyzer.model_variant`,
`default_parallelism`), and there are no workspace actions
(open folder, reset library, diagnostic export, reset settings)
or About info.

This change builds out the Settings modal so it covers the full set of
non-secret, non-cloud user preferences and workspace actions hamm0r
should expose. It explicitly does **not** add any storage of API
keys/bearer tokens (Invariant 11 — env vars only) and adds no default
network calls (Invariant 2).

Scope (single PR, per user choice):

1. Theme selector
2. Analyzer model variant picker
3. Default runner parallelism
4. Default request timeout
5. Default rate limit (optional)
6. Open hamm0r folder
7. Engagement folder location (relocate `hamm0r_root`)
8. Reset attack library to bundled defaults
9. Diagnostic bundle export
10. About section
11. Reset all settings to defaults

## Approach

Treat the modal as six logical sections:
**Appearance · Runner · Logging · Analyzer · Workspace · About**
(Logging and Analyzer keep their current panels; the rest are new.)

Two principles guiding the design:

- **Reuse the existing `<settings-section>` pattern** in
  [ui/index.html:1144-1179](../ui/index.html#L1144-L1179) so we don't
  refactor the modal shell.
- **Extend `AppConfig` minimally.** Add a `RunnerConfig` substruct
  and a few fields. Don't restructure existing keys — additive,
  `#[serde(default)]`-friendly so old configs round-trip cleanly
  (Invariant 7 spirit).

Naming cleanup: the modal id `analyzer-modal` and JS function
`openAnalyzerModal()` are misnamed now that the modal is generic
Settings. Rename to `settings-modal` / `openSettingsModal()` in the
same PR — the modal's user-visible title is already "Settings".

### Section-by-section design

**1. Appearance — theme**
- New section at top of modal.
- `<select>` with options: System / Light / Dark.
- Persists to `ui.theme` (already in [crates/storage/src/types.rs:312](../crates/storage/src/types.rs#L312)).
- Apply via `data-theme` attribute on `<html>`; CSS variables already
  drive the palette (verify in `ui/css/`).
- Saves immediately on change (no extra Save button).

**2. Runner — parallelism, timeout, rate limit**
- New section. Adds `RunnerConfig { default_parallelism, default_timeout_secs, default_rate_limit_rps: Option<u32> }`.
- Migrate the existing top-level `default_parallelism: u32` field
  (currently at [crates/storage/src/types.rs:321](../crates/storage/src/types.rs#L321))
  into `RunnerConfig`. Keep schema `version: 1` since the change is
  additive via `#[serde(default)]` — old files still load with
  defaults filled in. Document in `Datamodel.md` per Invariant 7.
- UI: number inputs for parallelism (1-16), timeout (5-600s),
  rate limit (blank = unlimited).
- Wire the runner to read these as defaults in
  [crates/runner/src/run.rs](../crates/runner/src/run.rs) and apply
  timeout via `reqwest::ClientBuilder::timeout()` in
  [crates/runner/src/adapter.rs:34-55](../crates/runner/src/adapter.rs#L34-L55).
- Rate limit: implement only if cheap (a simple async
  `tokio::time::interval`-based throttle in `runner::run`); if it
  starts to grow, drop it from this PR and file a follow-up. Mark
  this as the **one item that may get cut for scope** during
  implementation.

**3. Logging — unchanged**
- Existing controls remain; just placed below Runner.

**4. Analyzer — add model variant picker**
- Existing activation/download UI stays.
- Add a `<select>` for `analyzer.model_variant` with options sourced
  from the analyzer manifest (already loaded for download).
  Disabled until the analyzer is activated.
- Persists to `analyzer.model_variant`
  ([crates/storage/src/types.rs:307](../crates/storage/src/types.rs#L307)).

**5. Workspace — actions**
All actions in this section are Tauri commands invoked from JS.

- **Open hamm0r folder** — new command `open_hamm0r_folder` that
  reveals `hamm0r_root` in the OS file browser. Use Tauri's
  built-in `tauri-plugin-opener` (already in the Tauri 2 ecosystem,
  small surface; justify in commit). Cross-platform.
- **Engagement folder location** — text field + "Choose…" button
  that opens a Tauri folder picker.
  - Updates `hamm0r_root` in config; on save, the app warns:
    *"Existing data stays at the old location. Move files manually
    if you want them in the new location."* No automatic data
    migration (keeps blast radius small; user owns their files
    per ProductVision principle 7).
  - Restart-required notice (matches current logging note style).
- **Reset attack library** — new command `reset_prompt_library`
  that re-copies bundled `prompts/` into `~/hamm0r/prompts/`.
  Confirmation dialog. Goes through the storage layer
  (Invariant 6). Does not touch engagements.
- **Diagnostic bundle export** — new command
  `export_diagnostic_bundle(target_path: PathBuf)`. Bundles:
  - `~/hamm0r/logs/*` (last N MB only)
  - `~/hamm0r/config.yaml` with sensitive paths redacted to
    `<HAMM0R_ROOT>` placeholders
  - `version.txt` (app version + git hash)
  - **No** engagement responses, **no** prompts, **no** env values.
  - Output: zip file at user-chosen location via Tauri save dialog.
  - Adds one dep: `zip` crate (well-maintained, ~10k LOC) — justify
    in commit. Alternative: shell out to OS `tar` — rejected for
    cross-platform reasons.
- **Reset all settings** — new command `reset_app_config` that
  overwrites `config.yaml` with `AppConfig::defaults(hamm0r_root)`.
  Confirmation dialog. Engagements untouched.

**6. About**
- Static section: app version (from `CARGO_PKG_VERSION`), build hash
  (from a simple `build.rs` reading `git rev-parse --short HEAD`,
  falling back to `"unknown"` if git is unavailable),
  link to ProductVision and Architecture docs (rendered as plain
  text paths, not clickable URLs — air-gapped friendly).

## Files to change

**Frontend**
- [ui/index.html](../ui/index.html#L1140-L1179) — extend modal:
  rename `analyzer-modal` → `settings-modal`; add Appearance,
  Runner, Workspace, About sections.
- [ui/js/app.js:4976](../ui/js/app.js#L4976) — rename
  `openAnalyzerModal` → `openSettingsModal`; add handlers for new
  controls and Tauri command invocations.
- `ui/css/*` — minor styling for new fields (reuse existing
  `settings-grid`, `settings-field`, `settings-toggle-row`).

**Rust — storage**
- [crates/storage/src/types.rs:303-345](../crates/storage/src/types.rs#L303-L345) —
  add `RunnerConfig`; move `default_parallelism` into it;
  ensure `#[serde(default)]` everywhere.
- [crates/storage/src/app_settings.rs](../crates/storage/src/app_settings.rs) —
  extend save/load for new fields.
- New helpers (still in storage layer per Invariant 6):
  `reset_prompt_library`, `redacted_config_for_diagnostic`,
  `reset_app_config`, `set_hamm0r_root`.

**Rust — runner**
- [crates/runner/src/adapter.rs:34-55](../crates/runner/src/adapter.rs#L34-L55) —
  thread `default_timeout_secs` into the reqwest client.
- [crates/runner/src/run.rs](../crates/runner/src/run.rs) — read
  `default_parallelism` from new path; optionally apply rate limit.

**Rust — Tauri shell**
- `crates/hamm0r/src/` — register four new commands:
  `open_hamm0r_folder`, `reset_prompt_library`,
  `export_diagnostic_bundle`, `reset_app_config`.
  These call into runner/storage; **no** direct LLM or filesystem
  calls in the command layer (Invariant 8).
- `crates/hamm0r/build.rs` — emit `GIT_HASH` env var at build time
  for the About section.
- `crates/hamm0r/Cargo.toml` — add `zip` (justified) and
  `tauri-plugin-opener` (justified).

**Docs**
- [docs/Architecture.md](Architecture.md) — note the new
  Settings sections and the `RunnerConfig` field move.
- [docs/Datamodel.md](Datamodel.md) — document
  `runner.*` keys and the `default_parallelism` move (additive,
  no migration needed since `#[serde(default)]` covers it).

## Tests (in same PR — Definition of Done)

- `crates/storage/src/types.rs` round-trip tests — extend the
  existing block at [crates/storage/src/types.rs:374+](../crates/storage/src/types.rs#L374)
  to cover the new `RunnerConfig` and the legacy
  `default_parallelism` migration path (loading an old config
  without `runner.*` should yield a `RunnerConfig` with the
  legacy value preserved).
- Storage helper unit tests:
  - `reset_prompt_library` re-creates files from bundled defaults
    without touching engagements.
  - `redacted_config_for_diagnostic` strips paths/env names.
  - `set_hamm0r_root` validates the path exists and is writable.
- Runner test: `default_timeout_secs` is honored by the reqwest
  client (extend an existing adapter test).
- No UI test framework in core — manual verification covers UI
  (see below).

## Verification (manual, end-to-end)

After `cd crates/hamm0r && cargo tauri dev`:

1. Click Settings → modal opens, six sections visible in order.
2. Switch theme — UI repaints immediately, persists across restart.
3. Set Runner parallelism = 8, timeout = 30s — start an engagement,
   verify in logs that the runner uses the new values.
4. Toggle Analyzer model variant (with analyzer activated) —
   `~/hamm0r/config.yaml` reflects the change.
5. Click "Open hamm0r folder" — OS file browser opens at the
   configured root.
6. Change engagement folder location → confirmation dialog →
   restart → app reads from new location, old data stays put.
7. Reset attack library → confirmation → bundled prompts present
   under `~/hamm0r/prompts/`, an unrelated engagement folder is
   untouched.
8. Export diagnostic bundle → choose path → verify zip contains
   logs, redacted config, version.txt, and **no** engagement
   responses or env values. Inspect with `unzip -l`.
9. About section shows app version + git hash.
10. Reset all settings → confirmation → `config.yaml` matches
    `AppConfig::defaults(hamm0r_root)`; engagements untouched.

Run `cargo test --workspace` and
`cargo clippy --workspace --all-targets -- -D warnings` —
both must pass clean (CLAUDE.md Definition of Done).

## Non-goals (deliberately excluded)

- ~~Storing API keys / bearer tokens in Settings — Invariant 11
  forbids it; tokens stay in environment variables, named in the
  Target form.~~
  **Superseded** by a separate change that adds keychain-backed
  token storage in the **Target editor** (not Settings). Tokens
  still never land in `config.yaml` — they live in the OS credential
  vault. See the *Secret handling* section in `docs/Architecture.md`.
- Auto-update check or any default network call — Invariant 2.
  Could be added later as opt-in, off by default; out of scope here.
- Cloud sync, account login, telemetry — ProductVision rules these out.
- A YAML editor or "advanced config" panel — ProductVision
  principle 2 ("Click beats config") and Invariant 3.
- Automatic migration of existing engagement data when relocating
  `hamm0r_root` — keeps blast radius small; user owns the files.

## Open questions / risks

- **Rate limit may slip.** If the throttle implementation grows
  beyond a few dozen lines, drop it from this PR and file a
  follow-up. The other ten items are independent.
- **`zip` and `tauri-plugin-opener` deps.** Both are justified
  but new. If reviewer pushes back, alternatives: shell out for
  open-folder (acceptable cross-platform via `std::process::Command`
  + per-OS branching), and write a tar via stdlib + `flate2` (already
  likely transitively present).
- **Git hash at build time.** If the build environment lacks git
  (CI minimal image), the `build.rs` must fall back gracefully to
  `"unknown"` — already in plan, but flag for review.
