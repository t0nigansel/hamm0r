# Stack

Explicit decisions with reasons. When in doubt, follow this file.
Companion to `Architecture.md` — that one says how things fit together,
this one says which crates and tools we use.

## Rust 2024 edition (everything)

All business logic, HTTP runner, storage layer, Tauri commands, and
analyzer in Rust. Single language, single workspace, single binary
(plus the optional analyzer bundle).

Toolchain: stable, MSRV pinned in `rust-toolchain.toml`.
Package manager: Cargo. Lints: `clippy` with `-D warnings` in CI.
Format: `rustfmt` with default settings.

**Do not use**: any Python, Node, or shell-script glue in the runtime
path. Build scripts (`build.rs`) only when there is no Cargo-native
alternative.

## Tauri 2 (desktop shell)

Cross-platform desktop wrapper. Rust shell + WebView for the UI. The
backend is **not** a sidecar — it is the same Rust process as the
Tauri shell. Commands are exposed via `#[tauri::command]`. Events flow
back to the UI via `app.emit(...)`.

Why not Electron: Tauri produces smaller binaries, no Node.js runtime
to ship, better suited for a security tool that should feel
lightweight.

Why no sidecar: a sidecar means a second language, a second runtime,
IPC overhead, and a second thing to package per platform. We have
none of that.

**UI layer**: plain HTML + CSS + vanilla JS. No React, no Vue, no
bundler. Keep the frontend simple — this is a tool, not a web app.
Use Tauri's `invoke()` to call backend commands. Use CSS variables
for theming.

## tokio (async runtime)

Single multi-threaded `tokio` runtime, started by Tauri. Bounded
parallelism for in-flight HTTP via `tokio::sync::Semaphore`. Long
CPU-bound work (LLM inference in the analyzer) runs on
`spawn_blocking` so it does not stall the async runtime.

**Do not use**: `async-std`, `smol`, or hand-rolled thread pools.

## reqwest (HTTP runner)

Async HTTP client for sending attack prompts to target systems.
Build with `default-features = false, features = ["rustls-tls", "json", "stream"]`
to avoid an OpenSSL dependency.

Use a single `reqwest::Client` per run (it is `Clone` and pools
connections internally). Set explicit timeouts:

```rust
Client::builder()
    .connect_timeout(Duration::from_secs(10))
    .timeout(Duration::from_secs(30))
    .build()?
```

Redirects: follow by default. TLS verification: on by default,
configurable off per request (the user is testing their own systems
and may need this).

**Do not use**: `hyper` directly (too low-level), `surf` (dead),
`ureq` (sync only).

## File-based persistence

No database. State lives in files in `~/hamm0r/`, owned by the user.

- **YAML** for human-edited artefacts: prompts, requests, engagement
  metadata. Loader: `serde_yaml` with serde-derived types.
- **JSONL** for append-only run records and verdicts. Loader/writer:
  hand-rolled around `serde_json::to_writer` + newline. One JSON
  object per line, never broken across lines.
- **Plain text** for raw response bodies, one file per response.

All writes go through the `storage` crate. Atomic writes use the
write-to-temp + `rename` pattern via the `tempfile` crate.

**Do not use**: SQLite, sled, redb, or any embedded database in the
default build. The ProductVision allows SQLite-as-a-cache later if
performance demands it; that goes in `storage/` and stays an
implementation detail.

## minijinja (request templating)

Template engine for substituting `{{prompt}}` and friends into
request bodies and headers. `minijinja` because it is small, has
Jinja2-compatible syntax, and is pure Rust.

Render with auto-escape **off** for our use case — payloads are
intentionally literal. Sandbox the environment: no filesystem
access, no template includes from disk.

**Do not use**: `tera` (heavier than we need), `handlebars` (different
syntax for no benefit), `askama` (compile-time only).

## serde + serde_yaml + serde_json

Serialisation for everything that crosses a boundary (file, HTTP, IPC
to UI). Derive `Serialize` and `Deserialize` on every domain type
that hits storage.

**YAML** is the safe loader by default — `serde_yaml` does not have
the unsafe loader problem PyYAML did.

## directories (paths)

Use the `directories` crate for XDG-conformant paths. `~/hamm0r/` is
the documented default; on Windows that resolves to the local
appdata equivalent. Never hardcode `home_dir() + "/hamm0r"`.

## anyhow + thiserror (errors)

- `anyhow::Error` for application-level error propagation in commands
  and the runner.
- `thiserror::Error` for typed errors at crate boundaries (e.g.
  `storage::Error`, `runner::Error`) so callers can match on them.

Errors that cross the Tauri boundary to the UI are serialised as
`{ kind: string, message: string }` — the UI never sees a Rust
`Debug` representation.

## llama-cpp-2 (analyzer bundle only)

Local LLM inference, used inside the standalone `analyz0r` binary
that ships in the per-OS analyzer bundle. Loads GGUF models from
`~/hamm0r/analyzer/models/`. Default judge model: a small Qwen 2.5
variant in 4-bit quantization, selected by the manifest based on
host hardware.

Only `crates/analyzor-cli/` (and the parts of `crates/analyzer/`
reused inside the bundle) depend on `llama-cpp-2`. Core does **not**
link it — the boundary is the subprocess + the on-disk artifacts
described in [`docs/analyzorPlan.md`](analyzorPlan.md). There is no
longer a `--features analyzer` Cargo gate; the analyzer is delivered
as runtime install metadata, not as a build mode.

**Do not use**: `ollama` as a third-party sidecar, `candle` (revisit
later — see Architecture D-1), any cloud inference API.

## HTML reports (analyzer only)

The report is a single self-contained HTML file rendered via
`minijinja` (already in core). Inline CSS, no external assets, no
JS — open in any browser, mail as one file, print to paper if anyone
must.

**Do not use**: WeasyPrint, typst, wkhtmltopdf, headless Chrome, or
anything else that turns HTML into PDF. We are not making PDFs.

## Testing

- `cargo test` for unit and integration tests.
- `tokio::test` for async tests.
- `wiremock` (Rust crate) for mocking HTTP targets in runner tests.
- `tempfile::TempDir` for storage tests — never write to a fixed path.
- Snapshot tests for report HTML via `insta`.

No mocking framework beyond `wiremock`. Use plain structs and
fixtures.

## Workspace layout

```
hamm0r/
├── Cargo.toml             (workspace manifest)
├── rust-toolchain.toml
├── crates/
│   ├── hamm0r/            (binary, Tauri shell + commands)
│   ├── runner/            (HTTP firing line)
│   ├── storage/           (filesystem I/O)
│   └── analyzer/          (opt-in, behind `analyzer` feature)
├── ui/                    (HTML/CSS/JS, served by Tauri)
└── prompts/               (starter library, copied to ~/hamm0r/ on first run)
```

The `analyzer` crate is a dependency of `hamm0r` only when the
`analyzer` Cargo feature is enabled. Default builds omit it
entirely.

## What is explicitly forbidden

| Thing | Why |
|-------|-----|
| Python in the runtime path | Single-binary story, single language |
| A sidecar process | We removed it deliberately; do not add it back |
| Any cloud API in the runner | Data residency requirement |
| React / Vue / any bundler | Overkill for this UI |
| Web server frameworks (axum, actix) | No server, only Tauri commands |
| SQLite or any embedded DB by default | Files are the contract — Bruno/JMeter model |
| ORMs of any kind | Not applicable; no DB |
| OpenSSL | Use rustls — no system OpenSSL dependency |
| PDF generation | HTML is the report format |
| `unsafe` outside of FFI bindings | Default-deny; justify per occurrence |
| Hardcoded credentials | Use env vars or OS keychain |
| Global mutable state | Pass dependencies; makes testing tractable |
