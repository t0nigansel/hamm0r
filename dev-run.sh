#!/usr/bin/env bash
# Dev launcher: builds the analyzor sidecar, wires the Ollama env vars,
# and starts the Tauri app in dev mode. Run from the repo root:
#
#   ./dev-run.sh                          # defaults: model=qwen2.5:3b
#   MODEL=llama3.2:3b ./dev-run.sh        # override the model tag
#   SKIP_BUILD=1 ./dev-run.sh             # skip rebuilding analyzor-cli
#   OLLAMA_URL=http://host:11434 ./dev-run.sh

set -euo pipefail

MODEL="${MODEL:-qwen2.5:3b}"
OLLAMA_URL="${OLLAMA_URL:-http://localhost:11434}"
SKIP_BUILD="${SKIP_BUILD:-0}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$repo_root"

# Sanity: Ollama reachable?
if ! curl -fsS --max-time 3 "$OLLAMA_URL/api/tags" >/dev/null 2>&1; then
    echo "Ollama not reachable at $OLLAMA_URL. Start it with 'ollama serve' in another terminal." >&2
    exit 1
fi

# Sanity: model pulled?
if ! curl -fsS "$OLLAMA_URL/api/tags" | grep -q "\"$MODEL\""; then
    echo "Model '$MODEL' is not pulled. Run: ollama pull $MODEL" >&2
    exit 1
fi

if [[ "$SKIP_BUILD" != "1" ]]; then
    echo "Building analyzor-cli..."
    cargo build -p analyzor-cli
fi

# Pick the right binary name for the current platform.
if [[ "${OS:-}" == "Windows_NT" ]] || [[ "$(uname -s 2>/dev/null)" == MINGW* ]] || [[ "$(uname -s 2>/dev/null)" == CYGWIN* ]]; then
    analyzor_bin="$repo_root/target/debug/analyz0r.exe"
else
    analyzor_bin="$repo_root/target/debug/analyz0r"
fi

if [[ ! -f "$analyzor_bin" ]]; then
    echo "analyz0r binary not found at $analyzor_bin. Run without SKIP_BUILD=1." >&2
    exit 1
fi

export HAMM0R_ANALYZOR_BIN="$analyzor_bin"
export HAMM0R_ANALYZOR_OLLAMA_URL="$OLLAMA_URL"
export HAMM0R_ANALYZOR_OLLAMA_MODEL="$MODEL"

echo "HAMM0R_ANALYZOR_BIN          = $HAMM0R_ANALYZOR_BIN"
echo "HAMM0R_ANALYZOR_OLLAMA_URL   = $HAMM0R_ANALYZOR_OLLAMA_URL"
echo "HAMM0R_ANALYZOR_OLLAMA_MODEL = $HAMM0R_ANALYZOR_OLLAMA_MODEL"
echo "Launching cargo tauri dev..."

cd "$repo_root/crates/hamm0r"
exec cargo tauri dev
