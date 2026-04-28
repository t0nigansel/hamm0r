# hamm0r
*Ninja Hamm0r of Righteousness.*

Local-first AI security testing tool for LLM-based systems.

`hamm0r` fires OWASP-style attack prompts against AI endpoints, records raw responses, and — with the optional analyzer — produces verdict-annotated HTML reports. No cloud accounts, no external servers, no config files required.

## Highlights

- **Local-first**: no cloud calls, no external DB, air-gap friendly
- **Single binary**: Rust + Tauri 2 — no Python runtime, no Node, no sidecar
- **Engagement folders**: each engagement is a directory under `~/hamm0r/engagements/<slug>/` containing plain YAML/JSONL files you own
- **Scenario builder**: multi-step, multi-session attack flows with independent iterations
- **OWASP coverage**: prompts tagged to OWASP Top 10 for LLM Applications 2025
- **Analyzer (opt-in)**: local LLM judge (llama-cpp-2 + GGUF) — downloaded on first activation, never required for core use
- **HTML reports**: single self-contained file, opens in any browser, no rendering dependency

## Architecture

```text
crates/
  hamm0r/    Tauri 2 shell — commands, window management
  runner/    Async HTTP client, adapter layer, session manager
  storage/   File I/O layer — YAML configs, JSONL run logs
  analyzer/  Local LLM judge (opt-in, off by default)
prompts/     Bundled starter attack library (YAML)
```

User data layout (`~/hamm0r/`):

```text
~/hamm0r/
  prompts/            ← YAML prompt library (one file per category)
  targets/            ← YAML target configs
  requests/           ← YAML request templates
  scenarios/          ← YAML scenario definitions
  engagements/
    <slug>/
      meta.yaml
      run-NNN.jsonl         ← append-only run log
      run-NNN.verdicts.jsonl  ← analyzer output (when present)
      run-NNN-<attempt>.txt   ← raw response bodies
  analyzer/           ← downloaded model + runtime (opt-in)
```

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
git clone https://github.com/t0nigansel/hamm0r.git
cd hamm0r
PATH=~/.cargo/bin:$PATH cargo tauri dev
```

The app window opens automatically. The starter prompt library is seeded into `~/hamm0r/` on first launch.

### Build a release binary

```bash
PATH=~/.cargo/bin:$PATH cargo tauri build
```

The installer is placed in `target/release/bundle/`.

## Workflow

1. **Targets** — add the LLM endpoint you want to test (URL, auth header, session strategy)
2. **Scenarios** — pick prompts from the library or write your own, chain them into multi-step flows
3. **Run** — fire the scenario; responses are written to the engagement folder in real time
4. **Analyze** *(optional)* — activate the analyzer to get per-response verdicts and an HTML report

## Analyzer (optional LLM judging)

The analyzer is a separate opt-in component. It is **not required** to run attacks or record responses.

Activate it from the UI: **Settings → Activate analyz0r**. hamm0r will detect your hardware, download the appropriate GGUF model variant, and install it to `~/hamm0r/analyzer/`. After activation, any completed run can be analyzed in-process — no Ollama, no external service.

## Tests

```bash
PATH=~/.cargo/bin:$PATH cargo test
```

## License

MIT — see [LICENSE](LICENSE).
