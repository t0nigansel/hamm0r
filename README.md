# hamm0r
*Ninja Hamm0r of Righteousness.*

Local-first AI security testing tool for LLM-based systems.

`hamm0r` fires OWASP-style attack prompts against AI endpoints, records raw responses, and — with the optional analyzer — produces verdict-annotated HTML reports. No cloud accounts, no external servers, no config files required.

## Highlights

- **Local-first**: no cloud calls, no external DB, air-gap friendly
- **Single binary**: Rust + Tauri 2 — no Python runtime, no Node, no sidecar
- **Engagement folders**: each engagement is a directory under `~/hamm0r/engagements/<slug>/` containing plain YAML/JSONL files you own
- **Matrix Scenarios**: pick N Requests × a prompt-library subset (OWASP refs + categories), fire as a Cartesian product. Auth-chain prerequisites declared via `Request.response.bind` resolve automatically.
- **OWASP coverage**: prompts tagged to OWASP Top 10 for LLM Applications 2025
- **Analyzer (opt-in)**: local LLM judge (llama-cpp-2 + GGUF) — downloaded on first activation, never required for core use. Hosted-judge mode (Azure OpenAI) supported as an alternative.
- **HTML reports**: single self-contained file, opens in any browser, no rendering dependency

## Architecture

```text
crates/
  hamm0r/    Tauri 2 shell — commands, window management
  runner/    Async HTTP client, adapter layer, session manager
  storage/   File I/O layer — YAML configs, JSONL run logs
  analyzer/  Local LLM judge (opt-in, off by default)
prompts/     Bundled starter attack library (YAML)
requests/    Bundled starter request templates (YAML)
```

The active desktop app lives in `crates/hamm0r/`.

User data layout (`~/hamm0r/`):

```text
~/hamm0r/
  prompts/            ← YAML prompt library (one file per category)
  requests/           ← YAML request templates
  scenarios/          ← YAML matrix-Scenario definitions
  engagements/
    <slug>/
      engagement.yaml
      runs/
        run-NNN.jsonl            ← append-only run log (header + attempts + footer)
        run-NNN.verdicts.jsonl   ← analyzer output (when present)
      responses/<run>/<seq>.txt  ← raw response bodies, one per attempt
      reports/report-<run>.html  ← single self-contained HTML report
  analyzer/           ← downloaded analyz0r bundle (opt-in)
```

`~/hamm0r/targets/` may also exist on long-lived installs — it's a legacy
location read for back-compat. Targets are no longer a user-facing
concept; the same data now lives under `requests/` and `scenarios/`.

## How to Install

### Prerequisites

| Tool | Version | Install |
| ---- | ------- | ------- |
| Rust (via rustup) | stable (≥ 1.88) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Tauri CLI | 2.x | `cargo install tauri-cli --version "^2"` |
| System webview | — | macOS: built-in · Windows: WebView2 (ships with Win10+) · Linux: `libwebkit2gtk-4.1` |

On **macOS** also install Xcode Command Line Tools if not present:

```bash
xcode-select --install
```

On **Linux** (Debian/Ubuntu) install the Tauri system dependencies:

```bash
sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

### Build & run (development)

```bash
git clone https://github.com/SpiritTesting/hamm0r.git
cd hamm0r
cd crates/hamm0r
PATH=~/.cargo/bin:$PATH cargo tauri dev
```

The app window opens as a native Tauri desktop app. Missing starter prompts and
starter request templates are seeded into `~/hamm0r/` on startup without
overwriting your existing files.

If `cargo` is already on your `PATH`, you can simply run:

```bash
cd /path/to/hamm0r/crates/hamm0r
cargo tauri dev
```

On Windows PowerShell:

```powershell
Set-Location crates/hamm0r
cargo tauri dev
```

This app is not meant to be opened directly in a browser. The frontend expects
to run inside Tauri and uses the Rust command layer exposed there.

### Tester setup with analyz0r (temporary)

The packaged one-click local analyz0r download is not published yet. For the
current tester setup, run analyz0r through a local Ollama model.

Install Ollama first:

```text
https://ollama.com/download
```

Then in Git Bash:

```bash
git clone https://github.com/SpiritTesting/hamm0r.git
cd hamm0r

ollama pull qwen2.5:3b
cargo build -p analyzor-cli

export HAMM0R_ANALYZOR_BIN="$PWD/target/debug/analyz0r.exe"
export HAMM0R_ANALYZOR_OLLAMA_URL="http://localhost:11434"
export HAMM0R_ANALYZOR_OLLAMA_MODEL="qwen2.5:3b"

cd crates/hamm0r
cargo tauri dev
```

If Ollama is not already running, open a second terminal and run:

```bash
ollama serve
```

Keep the terminal with the `export ...` commands open while hamm0r is running.
The app inherits those environment variables only when it is started from that
same terminal.

### WSL troubleshooting (Linux graphics / EGL)

If you run `cargo tauri dev` inside WSL and see warnings like:

```text
libEGL warning: ...
MESA: error: ZINK: failed to choose pdev
egl: failed to create dri2 screen
```

this is a WSL graphics stack issue (EGL/Mesa), not a hamm0r run-logic error.

Recommended workaround: run from Windows PowerShell instead of WSL:

```powershell
cd C:\workspace\hamm0r\crates\hamm0r
cargo tauri dev
```

If you must stay in WSL, try software rendering:

```bash
LIBGL_ALWAYS_SOFTWARE=1 MESA_LOADER_DRIVER_OVERRIDE=llvmpipe cargo tauri dev
```

Alternative fallback:

```bash
WEBKIT_DISABLE_COMPOSITING_MODE=1 cargo tauri dev
```

### Build a release binary

```bash
cd crates/hamm0r
PATH=~/.cargo/bin:$PATH cargo tauri build
```

The installer is placed in `crates/hamm0r/target/release/bundle/`.

## Workflow

1. **Requests** — define one or more HTTP requests against your LLM
   endpoints (URL, headers, auth, body). Use `{{prompt}}` in the body
   to mark where attack payloads get substituted at fire time. If
   authentication needs a per-run login, model the login as a separate
   Request with `response.bind: bearer_token` and reference it from the
   chat Request's headers as `{{login_id.bearer_token}}`.
2. **Prompts** — the bundled OWASP-tagged library lives under
   `~/hamm0r/prompts/`. Add or edit prompts in the Library view.
3. **Scenarios** — pick the Requests you want to fire and a library
   subset (OWASP refs and/or categories). hamm0r runs the Cartesian
   product as one engagement.
4. **Fire** — Home → "Run a Scenario", or open a Scenario and click Run.
   Responses stream into the engagement folder in real time.
5. **Analyze** *(optional)* — activate the analyzer (Settings → Analyz0r)
   for per-response verdicts and an HTML report. Local-judge and
   hosted-judge modes are both supported.

## Analyzer (optional LLM judging)

The analyzer is a separate opt-in component. It is **not required** to run attacks or record responses.

Activate it from the UI: **Settings → Activate analyz0r**. hamm0r will detect your hardware, download the appropriate GGUF model variant, and install it to `~/hamm0r/analyzer/`. After activation, any completed run can be analyzed in-process — no Ollama, no external service.

## Tests

```bash
PATH=~/.cargo/bin:$PATH cargo test
```

## License

MIT — see [LICENSE](LICENSE).
