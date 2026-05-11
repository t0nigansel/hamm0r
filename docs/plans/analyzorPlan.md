# analyz0r â€” Optional Install Plan

## Context

The intended product behavior is:

- A fresh `hamm0r` install contains **core only**
- The user can run attacks and inspect raw responses without any analyzer
- The user can open **Settings** and click **Download & Install**
- Only after that install is complete, analyz0r becomes available
- The user can then analyze a response or a full run locally on-device

That intent matches `docs/productVision.md` and `docs/Architecture.md`, but the
current implementation is split between two incompatible models:

1. **Runtime install UX exists**
   - Settings modal shows analyzer activation UI
   - model download code exists
2. **Actual analyzer execution is compile-time gated**
   - judging/report commands require `--features analyzer`
   - default builds cannot analyze even after a model download

This plan realigns the implementation with the product vision:
**analyz0r is an optional runtime-installed feature, not a compile-time-only build mode.**

---

## Product Goal

Deliver analyz0r as an **OS-independent optional add-on** with this v1 user flow:

1. User downloads and launches `hamm0r`
2. Analyzer is not present and core works normally
3. User opens Settings and sees analyzer status: **not installed**
4. User clicks **Download & Install**
5. App downloads one known-good default model/runtime bundle for the current OS
6. App enables **Judge** / **Analyze** actions
7. User selects a response or run and gets a local verdict:
   - vulnerable / problematic
   - not vulnerable / okay
   - inconclusive

Longer term, the same architecture must allow a model picker without changing
the core/analyzer boundary.

---

## Non-negotiable Constraints

This plan must preserve the current architectural invariants:

- **Core works without analyzer**
- **No dependency on `crates/analyzer` in the default core flow**
- **No cloud calls in the default workflow**
- **Analyzer never breaks core when absent**
- **Run JSONL remains the handoff contract**
- **Verdicts are written separately**
- **Cross-platform behavior must be possible on Windows, macOS, and Linux**

---

## Current Gap Summary

### What exists today

- Analyzer UI in Settings
- Analyzer status command
- Manifest fetch + model download command
- Judge/report logic in `crates/analyzer`
- Verdict JSONL and HTML report generation

### What is broken today

- Default app build can download a model but cannot actually judge
- Analyzer execution depends on Cargo feature `analyzer`
- Current install flow downloads only a model, not a real runtime bundle
- Activation state is not modeled cleanly in config/storage
- Windows analyzer build currently depends on extra native toolchain pieces

### Root problem

The repo currently mixes:

- **runtime activation UX**
- with **compile-time feature activation**

That is the core mismatch to remove.

---

## Target Architecture

Implement analyz0r as a **runtime-installed sidecar bundle**.

### Core app responsibilities

- Detect whether analyz0r is installed
- Show install / uninstall / analyze UI
- Write and read user data via `storage`
- Invoke analyzer via a narrow command boundary
- Continue to function perfectly when analyzer is absent

### Analyzer bundle responsibilities

- Ship the analyzer executable/runtime for the current OS
- Load exactly one supported default model in v1
- Read run evidence from disk
- Produce verdict JSONL and report HTML
- Return progress and status to the core app

### Boundary

The boundary between core and analyzer is:

- run JSONL
- response files
- verdict JSONL
- generated report
- a small invocation contract from core to analyzer

Core must not need to compile in llama.cpp or model-runtime crates to ship.

---

## Recommended v1 Design

### 1. Analyzer delivery format

Ship analyz0r as a **platform-specific bundle** downloaded at install time:

- Windows: analyzer executable + runtime files + default GGUF
- macOS: analyzer executable + runtime files + default GGUF
- Linux: analyzer executable + runtime files + default GGUF

Bundle contents:

- analyzer runner binary
- any required dynamic libs/runtime assets
- one default model file
- manifest metadata file

Recommended install layout:

```text
~/hamm0r/analyzer/
├── manifest.json
├── install.json
├── bin/
│   └── analyz0r[.exe]
├── runtime/
│   └── ...
└── models/
    └── default.gguf
```

### 2. Invocation model

Use a **subprocess boundary** from core to analyz0r, not an in-process Rust crate call.

Recommended command shape:

```text
analyz0r judge-result --engagement <slug> --run <run-id> --seq <n>
analyz0r judge-run --engagement <slug> --run <run-id>
analyz0r generate-report --engagement <slug> --run <run-id>
```

Why subprocess is the right choice:

- keeps core free of analyzer runtime dependencies
- matches the “optional installed component” product model
- works cross-platform
- avoids trying to `dlopen` Rust internals across OS/compiler boundaries
- simplifies release packaging

### 3. Model support in v1

Support **exactly one default model** in the first implementation.

Requirements for the model:

- works on CPU on all supported OSes
- small enough to remain practical
- stable JSON-style judging output
- supported by the selected local runtime

The manifest should still include a variant identifier even in v1, but only one
variant is offered to the user.

### 4. Future model picker

Design the manifest and install metadata now so later we can support:

- one default model today
- multiple selectable models later

Do this by keeping:

- `variant_id`
- `model_id`
- `hardware_class`
- `bundle_version`
- `minimum_hamm0r_version`

in the analyzer install metadata even before the UI exposes choices.

---

## Implementation Plan

### Phase 1 â€” Re-establish the boundary

Goal: stop relying on `--features analyzer` for end-user functionality.

Tasks:

- Remove analyzer execution from the default `hamm0r` runtime path
- Keep `crates/analyzer` as implementation source if useful, but stop requiring
  the main app binary to compile it for normal shipping
- Define a stable analyzer CLI contract:
  - judge one result
  - judge one run
  - generate one report
- Define machine-readable stdout/stderr behavior and exit codes
- Decide whether `crates/analyzer` becomes:
  - a standalone binary crate, or
  - a library used by a new `crates/analyzor-cli`

Recommendation:

- Create a dedicated analyzer CLI binary crate and keep shared judge/report code
  in `crates/analyzer`

### Phase 2 â€” Installer/manifest overhaul

Goal: make **Download & Install** install something actually runnable.

Tasks:

- Replace model-only download with bundle install
- Extend manifest schema to describe:
  - bundle artifact URL
  - bundle SHA256
  - model ID
  - OS
  - architecture
  - minimum hamm0r version
- Add install metadata file, e.g. `~/hamm0r/analyzer/install.json`
- Verify hashes for every downloaded artifact
- Persist selected/default variant in config

Important:

- The bundled fallback manifest must no longer contain placeholder hashes
- Install must be atomic:
  - download to temp
  - verify
  - extract/move into final location
  - then mark installed

### Phase 3 â€” Runtime detection and status

Goal: analyzer availability is determined by a complete installation, not by
“some `.gguf` file exists”.

Tasks:

- Replace `first_gguf()`-style detection with install metadata validation
- Add status states:
  - not_installed
  - downloading
  - installed
  - broken_install
  - incompatible_version
- Update Settings UI to show these states explicitly
- Add “Repair install” behavior when metadata exists but runtime is broken

Recommended status contract:

```json
{
  "state": "installed",
  "installed": true,
  "bundle_version": "0.1.0",
  "model_id": "qwen-default",
  "variant_id": "default-x86_64",
  "hardware": "x86_64_avx2"
}
```

### Phase 4 â€” Core-to-analyzer execution

Goal: the user can judge a response after installation.

Tasks:

- Replace direct feature-gated analyzer calls with subprocess execution
- Add command-layer functions that:
  - validate install state
  - launch analyz0r CLI
  - stream progress
  - translate results into existing UI DTOs
- Keep current UI calls if possible:
  - `judge_result`
  - `judge_all`
  - `start_analysis`
  - `generate_report`
- Internally route those commands to the analyzer executable instead of to
  compile-time Rust code

Important:

- The Tauri command layer should orchestrate only
- File reads/writes still go through `storage`
- Analyzer subprocess should write verdict/report outputs through the same data contract

### Phase 5 â€” UI completion

Goal: Settings install works, and analysis is clearly available only after install.

Tasks:

- Keep the current Settings analyzer section
- Rename copy from “Download & Install” behavior to reflect exact state
- Disable Judge/Analyze actions when analyzer is absent
- Show clear user errors:
  - analyzer not installed
  - analyzer install corrupted
  - incompatible model/runtime
- Add success path:
  - install completes
  - modal refreshes
  - analyze buttons become active

Additions to Workbench / Runs UX:

- single-response Judge button remains
- Judge All remains
- optional run-level Analyze button should be wired to `start_analysis`
- report preview should work after completed analysis

### Phase 6 â€” Packaging and release

Goal: make the system real outside dev machines.

Tasks:

- Produce analyzer bundles for:
  - Windows
  - macOS
  - Linux
- Define release naming convention
- Publish manifest + bundle artifacts
- Add compatibility checks:
  - OS
  - architecture
  - minimum hamm0r version
- Ensure uninstall removes:
  - installed binary
  - runtime
  - model
  - install metadata
  but leaves verdicts/reports untouched

---

## Data Model Changes

The following additions are recommended.

### `config.yaml`

Keep analyzer settings small and additive:

```yaml
analyzer:
  enabled: false
  model_variant: default
  install_state: not_installed
```

However, installation truth should not rely only on config. The real source of
truth should be install metadata on disk.

### `~/hamm0r/analyzer/install.json`

Recommended schema:

```json
{
  "version": 1,
  "bundle_version": "0.1.0",
  "installed_at": "2026-05-04T12:00:00Z",
  "variant_id": "default-x86_64",
  "model_id": "qwen-default",
  "platform": "windows-x86_64",
  "entrypoint": "bin/analyz0r.exe"
}
```

### Manifest schema

Recommended manifest fields:

- `version`
- `generated_at`
- `minimum_hamm0r_version`
- `variants[]`
- for each variant:
  - `id`
  - `label`
  - `os`
  - `arch`
  - `hardware`
  - `recommended`
  - `bundle.url`
  - `bundle.sha256`
  - `bundle.size_bytes`
  - `model_id`

---

## Crates and Files to Change

### New / reshaped Rust modules

- `crates/analyzer/`
  - keep judge/report logic reusable
- new analyzer CLI crate
  - recommended: `crates/analyzor-cli/`
- `crates/hamm0r/src/commands/analyzer_setup.rs`
  - upgrade installer and status logic
- `crates/hamm0r/src/commands/analysis.rs`
  - replace feature-gated direct calls with subprocess orchestration
- `crates/storage/`
  - add install metadata load/save helpers

### Frontend

- [ui/index.html](../ui/index.html)
  - analyzer section copy/states
- [ui/js/api.js](../ui/js/api.js)
  - keep command surface stable where possible
- [ui/js/app.js](../ui/js/app.js)
  - install status, enable/disable Judge actions, run-level analyze wiring
- [ui/style.css](../ui/style.css)
  - status styles for installed/broken/downloading

### Docs

- [docs/Architecture.md](Architecture.md)
  - update analyzer section to reflect subprocess/runtime bundle architecture
- [docs/Datamodel.md](Datamodel.md)
  - document analyzer install metadata and any manifest/install-state fields
- [docs/Stack.md](Stack.md)
  - document runtime/toolchain choice for analyz0r

---

## Testing Strategy

### Unit tests

- manifest parsing
- install metadata round-trip
- broken install detection
- analyzer status mapping
- subprocess result parsing
- verdict writing/report generation

### Integration tests

- core build works with no analyzer installed
- install metadata absent â†’ analyzer unavailable
- fake installed bundle â†’ analyzer available
- judge one result writes one verdict line
- judge run writes verdict header, verdicts, footer
- report generation produces HTML file

### End-to-end manual checks

1. Fresh install, no analyzer:
   - app launches
   - attacks run
   - export raw data works
   - Judge actions clearly unavailable
2. Open Settings, install analyzer:
   - progress shown
   - install completes
   - app now shows analyzer as available
3. Judge one response:
   - verdict appears in UI
   - verdict file written
4. Analyze full run:
   - progress updates shown
   - report generated
5. Uninstall analyzer:
   - analyzer unavailable again
   - prior verdict/report artifacts still readable

---

## Risks

### 1. Native runtime portability

The biggest risk is shipping a truly cross-platform local LLM runtime. The plan
assumes this is solvable by shipping per-OS analyzer bundles, not by making the
core app compile the runtime everywhere.

### 2. Build complexity

If the analyzer remains tied to `llama-cpp` builds on user machines, this
feature will stay fragile. The runtime must be delivered prebuilt where
possible.

### 3. Version skew

Core app version and analyzer bundle version can drift apart. The manifest and
install metadata need explicit compatibility checks.

### 4. Large downloads

Even with one default model, downloads are large. Install flow needs:

- robust progress
- resume/retry-friendly behavior later
- clear disk-space messaging

---

## Recommended Delivery Order

Implement in this order:

1. Define analyzer CLI contract
2. Create runtime-install metadata model
3. Convert install flow from model-only to bundle install
4. Replace direct feature-gated judging with subprocess orchestration
5. Wire UI availability states
6. Add report/run-level analysis flow
7. Package per-OS bundles and publish manifest

This order gets to “button works and user can judge a response” fastest without
breaking the core/analyzer boundary.

---

## Explicit Non-goals for v1

- Multiple model choices in the UI
- Automatic hardware benchmarking
- Background auto-update of analyzer bundles
- Cloud-hosted inference fallback
- Sharing analyzer state between machines
- Installing analyzer during initial hamm0r setup

Those can come later. v1 is:

- optional
- manually installed
- one default model
- cross-platform by shipped bundle
- able to judge responses and generate reports locally

---

## Success Criteria

This feature is done when:

1. `hamm0r` ships and runs without analyzer present
2. Settings â†’ **Download & Install** installs analyz0r successfully
3. Install works on Windows, macOS, and Linux
4. After install, the user can judge a response and a full run
5. Verdict JSONL and HTML report are written correctly
6. Uninstall removes analyz0r without breaking core
7. No analyzer dependency is required for the standard core workflow
