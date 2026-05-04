# Architecture.md — hamm0r

This document describes the structural decisions behind hamm0r. It is
subordinate to `ProductVision.md`: the vision says what hamm0r is, this
document says how it is built to be that.

If you are looking for rules about writing code, see `CLAUDE.md`. If
you are looking for the shape of data on disk, see `Datamodel.md`.

---

## The shape of the system

hamm0r is a single desktop binary. No client/server split, no language
boundary, no IPC. Tauri provides the window; everything behind the
window is Rust.

The system has two cleanly separated components: **core** (always
present, compiled into the main binary) and **analyzer** (opt-in,
downloaded on activation as a separate dynamic library plus model
files). The user experience is a single app; the architecture keeps
the two components isolated.

```
┌──────────────────────────────────────────────────────────────────┐
│                         Tauri window                             │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │                     UI (HTML / CSS / JS)                   │  │
│  │                                                            │  │
│  │    Browse prompts · Build request · Fire · See report      │  │
│  └──────────────────────┬─────────────────────────────────────┘  │
│                         │ Tauri invoke / emit                    │
│  ┌──────────────────────▼─────────────────────────────────────┐  │
│  │                   Rust backend (core)                      │  │
│  │                                                            │  │
│  │    Command handlers · Runner · Storage                     │  │
│  └───────┬───────────────────────────────────────┬────────────┘  │
│          │                                       │               │
│          │ in-process                            │ dynamic load  │
│          │                                       │ (opt-in)      │
│  ┌───────▼─────────┐                    ┌────────▼────────────┐  │
│  │  Storage crate  │                    │   Analyzer crate    │  │
│  │                 │                    │      (opt-in)       │  │
│  │  YAML · JSONL · │                    │                     │  │
│  │   atomic writes │                    │   embedded LLM +    │  │
│  └───────┬─────────┘                    │      verdicts       │  │
│          │                              └─────────┬───────────┘  │
│          │                                        │              │
│          │ reads/writes                           │ reads        │
│          ▼                                        │              │
│  ┌─────────────────────────────────────────────-──▼──────────┐   │
│  │                    Filesystem layer                       │   │
│  └──────────────────────┬────────────────────────────────────┘   │
└─────────────────────────┼────────────────────────────────────────┘
                          │
                          ▼
            ~/hamm0r/  (user's on-disk library)
            ├── prompts/
            ├── requests/
            ├── analyzer/         (only if activated)
            └── engagements/<slug>/
                ├── engagement.yaml
                ├── runs/run-NNN.jsonl
                ├── runs/run-NNN.verdicts.jsonl
                ├── reports/report-NNN.html
                └── responses/<run>/*.txt
```
Two things to notice:

- The storage crate is the only component that touches the filesystem.
  Runner and analyzer both go through it.
- Runner and analyzer never talk to each other directly. Their handoff
  is the run JSONL file on disk. The analyzer can run days after the
  runner finished. The runner can run without the analyzer ever being
  installed.

---

## Module boundaries

Each module is a Rust crate (or sub-module) with one job and a clear
rule about what it may not do.

### `ui/` — the surface

Pure front-end: HTML, CSS, vanilla JavaScript. Lives in the Tauri
window. Its job is to let the user browse prompts and requests, compose
a run, fire it, and view results. Talks to the Rust backend via Tauri's
`invoke` (UI → backend command) and `emit` (backend → UI event).

**Must not:** read or write files directly, hold secrets in memory
beyond the current form, make HTTP calls.

### `backend/` — the command surface

The Rust crate behind the Tauri window. Exposes `#[tauri::command]`
handlers (list prompts, list requests, start run, get run status,
activate analyzer). Spawns the runner as a `tokio` task, streams
progress back to the UI via `emit`.

**Must not:** call targets directly, hold business logic that belongs
in runner or analyzer, format reports.

### `runner/` — the firing line

Takes a request template and a set of prompt payloads. Substitutes
each payload into the request, sends the HTTP call to the target via
`reqwest`, receives the response, hands it to storage. Parallelism,
retries, timeouts live here.

**Must not:** depend on the `analyzer` crate, interpret responses
(beyond extracting them from the HTTP envelope), write reports, or
assume an analyzer exists.

### `storage/` — the only filesystem

The single crate that knows how hamm0r's files are laid out and how
they are read and written safely. Provides typed load/save functions
for prompts, requests, engagements, runs, responses. Handles atomic
writes (write-to-temp + rename), append semantics, and path
resolution. Uses `serde_yaml` for YAML and serde-driven JSONL for run
records.

**Must not:** contain business logic, know about OWASP categories,
know about the analyzer, format reports.

Why it matters: if the file layout changes, one crate changes. If a
race condition appears in a write, one crate is responsible. If we
ever add an optional index (SQLite-as-cache, per the ProductVision
clause), it plugs in here and only here.

### `analyzer/` — the verdict engine

A Rust crate that the user opts into. Embeds a local LLM runtime
(`llama-cpp-2` or `candle` — see D-1) and the model file. Reads run
JSONL plus response files, produces verdicts, generates a single-file
HTML report.

**Must not:** modify run JSONL files written by the runner, make cloud
calls, be linked into the default build.

### `prompts/` (repo), `~/hamm0r/prompts/` (user)

In the repo: the curated starter library that ships with hamm0r. On
first run, it is copied into the user's `~/hamm0r/prompts/` folder,
after which the user owns it. Subsequent updates to the starter library
are offered as additions, never forced — the user's copy is sacred.

---

## The analyzer as a separable module

The analyzer is large (model files alone are 1–4 GB depending on
variant). It must not be part of the default install. The two-modes
invariant is enforced at the crate-graph level:

- The default workspace build (`cargo build -p hamm0r`) does **not**
  compile `analyzer`. It is behind a Cargo feature `analyzer` that is
  off by default.
- Lint check in CI: `cargo tree -p hamm0r --no-default-features` must
  not list `analyzer` or any LLM-runtime crate.
- The release pipeline produces two artifacts: `hamm0r` (the core app,
  small) and `hamm0r-analyzer-bundle-<platform>` (a separately
  downloadable bundle containing the analyzer dynamic library plus
  the default model file).

When the user clicks **"Activate analyz0r"** in the UI:

1. Backend checks `~/hamm0r/analyzer/` for an existing installation.
2. If none, backend downloads the manifest from a fixed URL
   (`https://hamm0r.example/analyzer/manifest.json` — to be finalized).
   The manifest lists model variants by hardware class.
3. Backend picks a default variant based on host detection (Apple
   Silicon, x86-64 with AVX2, fallback). The user may override.
4. Backend downloads the dynamic library and the model file to
   `~/hamm0r/analyzer/`. Progress is streamed to the UI via `emit`.
5. On next launch, the backend `dlopen`s the analyzer library if
   present and the "Analyze" button becomes active. On failure, the
   user gets a plain error and may retry.

Once loaded, the analyzer is invoked in-process. It reads a specific
run's JSONL, produces `<run>.verdicts.jsonl` alongside it, and
optionally emits a report PDF.

The analyzer never writes anywhere except:

- `~/hamm0r/engagements/<slug>/runs/<run>.verdicts.jsonl`
- `~/hamm0r/engagements/<slug>/reports/<report>.html`

It never modifies prompts, requests, engagement metadata, or the run
JSONL itself. That is the contract.

---

## The data flow of a single run

Walking through the five-minute promise, step by step:

1. **User picks a request and a prompt set in the UI.**
   UI invokes `list_prompts` and `list_requests` on the backend.
   Backend asks storage. Storage reads `~/hamm0r/requests/` and
   `~/hamm0r/prompts/`.

2. **User clicks FIRE.**
   UI invokes `start_run` on the backend with the selected request
   and prompt set. Backend validates, creates a new engagement folder
   if needed, allocates a run ID, and spawns the runner as a `tokio`
   task.

3. **Runner executes.**
   For each payload in the prompt set: substitute into the request
   template (via `minijinja`), send HTTP via `reqwest`,
   receive response. Hand the full exchange to storage. Storage
   appends a line to `run-NNN.jsonl` and writes the raw response body
   to `responses/<run>/<seq>.txt`. Backend `emit`s status updates to
   the UI as each response lands.

4. **Runner finishes.**
   The final JSONL line carries a `run_finished` marker. Backend
   emits a final event. The user can now export, or — if the
   analyzer is installed — click "Analyze."

5. **Analyzer executes (optional).**
   Reads `run-NNN.jsonl` and the corresponding response files. For
   each response, asks the embedded LLM for a verdict against the
   OWASP category the attack targets. Writes
   `run-NNN.verdicts.jsonl`. When done, optionally renders a report.

6. **User reads the report.**
   The report is a single self-contained HTML file in
   `engagements/<slug>/reports/`. The UI embeds a preview; the file
   is already on disk, openable in any browser, and shareable as-is.

Crucial property: every step's output is a file. If the user closes
hamm0r after step 4, they can reopen it a week later and pick up at
step 5. If they email the engagement folder to a colleague, the
colleague sees the same state. If the app crashes in step 3, the
JSONL up to that point is valid; the next run starts fresh.

---

## Concurrency model

The runner uses `tokio` + `reqwest`. Inside a single run, attack
payloads are fired with bounded parallelism (default 4, configurable
per request to respect target rate limits) via a `tokio::sync::Semaphore`.

Between runs: only one run is active at a time per engagement folder.
The UI disables FIRE while a run is in flight. This is simpler than
multi-run concurrency and maps to the user's mental model — they watch
a run the way they watch a nuclei scan.

The analyzer is single-threaded per invocation but can be invoked on
multiple past runs sequentially. LLM inference happens on a dedicated
`tokio::task::spawn_blocking` thread to avoid stalling the async
runtime.

---

## Secret handling

Bearer tokens, basic-auth credentials, and custom-header values are
secrets. Per ProductVision and CLAUDE.md Invariant 11, they never live
in `config.yaml`, in run logs, or in any artifact under
`~/hamm0r/engagements/`.

Each `AuthConfig` variant identifies a secret by the **name** of an
env var that shadows it (e.g. `token_env: "PROFILER_BEARER_TOKEN"`).
At request time the runner resolves the value via `storage::secrets`,
which checks two sources in order:

1. **OS keychain** — Windows Credential Manager, macOS Keychain, or
   Linux Secret Service, queried under service `"hamm0r"` with the
   env-var name as the account. Populated when the user pastes a
   token into the Target editor.
2. **Environment variable** — `std::env::var(name)`, the original
   mechanism. Still works when no keychain entry is set.

If neither has a value, the runner returns `MissingEnvVar` and the UI
shows a hint pointing the user at both options.

The plaintext token crosses the JS→Rust bridge exactly once per
save, via the `set_bearer_token` Tauri command. No command exposes
the stored value back to the UI; the renderer shows status only
("stored in keychain", "using env var", "not set"). The runner reads
the value directly from `storage::secrets` — it never travels
through the command layer.

Diagnostic exports redact the env-var name only, never include
keychain values, and never read environment variables.

---

## Crates and dependencies

Core depends on:

- `tauri` for the window and command surface
- `tokio` for async runtime
- `reqwest` for async HTTP (rustls backend, no OpenSSL)
- `serde` + `serde_yaml` + `serde_json` for serialization
- `minijinja` for request template substitution
- `anyhow` / `thiserror` for error handling
- `keyring` for OS-credential-vault access (bearer/api-key storage)

Analyzer additionally depends on:

- `llama-cpp-2` or `candle` for local LLM inference (see D-1)

The HTML report is rendered via `minijinja` (already in core) — no
extra rendering dependency.

Core must build and run on a machine with no analyzer dependencies
installed. The analyzer's dependencies live behind the `analyzer`
Cargo feature and are only pulled in when that feature is enabled or
the analyzer bundle is loaded at runtime.

---

## Open architectural decisions

Things I have deliberately not decided for you yet. Each is a real
fork where the wrong choice is expensive to undo.

### D-1. LLM runtime for the analyzer

Two finalists, both Rust:

- **`llama-cpp-2`** (Rust binding to llama.cpp). Broad GGUF model
  support, mature, fast on CPU and Metal. C++ under the hood, which
  means build complexity on the maintainer side — but shipped as a
  prebuilt dynamic library to users.
- **`candle`** (pure Rust, by HuggingFace). No C++. Fewer models
  supported, slower-moving ecosystem, but a single-language stack
  end-to-end.

Recommendation: `llama-cpp-2`. Model reach matters more than
toolchain purity for an analyzer that needs to keep up with whatever
the best small judge model is at the time. Revisit if `candle`
catches up on Qwen/Gemma/DeepSeek support and quantization quality.

### D-2. Model manifest hosting

The manifest that lists downloadable models needs a home. Options:

- A static JSON on the hamm0r project website (or GitHub Pages).
  Cheapest, simplest.
- A GitHub release assets approach (manifest is a release artifact).
  Versioned for free.
- HuggingFace as the CDN for the models themselves, with only the
  manifest hosted by us. Models are already there, no bandwidth costs
  on our side.

Recommendation: manifest on GitHub Pages, models fetched from
HuggingFace. Zero infrastructure.

### D-3. Engagement folder location

Default is `~/hamm0r/`. On Windows this is an odd choice; native
would be `%APPDATA%` or Documents.

Recommendation: use the `directories` crate for XDG-conformant
defaults per OS, configurable on first launch but never after
(changing later is an invitation to bugs).

---

## What we do not build

Listed so we can say no cleanly when someone suggests them:

- A plugin system. Contradicts ProductVision principle 7 (files) and
  principle 2 (click over config).
- A scheduler / cron mode for recurring scans. Out of scope for v1.
- Team sync of engagements. Out of scope — `rsync` and Git exist.
- A REST API for third-party integrations. There is no server.
- Telemetry. Not now, not quietly.

---

## How this document changes

When an invariant, boundary, or decision listed here proves wrong in
practice: change the document in the same commit that changes the
code. Never let the map disagree with the territory.

When a new architectural decision arises (a D-4, D-5): add it here
before the code, not after.
