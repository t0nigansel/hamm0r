#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

PYTHON_BIN="${PYTHON_BIN:-python3}"
VENV_DIR="${VENV_DIR:-${ROOT_DIR}/.venv}"
DB_PATH="${DB_PATH:-${ROOT_DIR}/default_engagement.db}"
MODEL="${OLLAMA_MODEL:-qwen2.5:14b}"

WITH_DEV=0
WITH_PDF=0
SKIP_MODEL_PULL=0
AUTO_INSTALL_OLLAMA=0

usage() {
  cat <<'EOF'
promt0r installer

Usage:
  scripts/install.sh [options]

Options:
  --python <bin>          Python executable (default: python3)
  --venv <path>           Virtualenv path (default: ./.venv)
  --db <path>             SQLite engagement path (default: ./default_engagement.db)
  --model <name>          Ollama model for analyzer (default: qwen2.5:14b)
  --with-dev              Install dev dependencies (pytest, ruff)
  --with-pdf              Install PDF report dependencies (weasyprint)
  --skip-model-pull       Do not pull Ollama model
  --install-ollama        Try to install Ollama automatically (Homebrew on macOS)
  -h, --help              Show this help

Environment variables:
  PYTHON_BIN, VENV_DIR, DB_PATH, OLLAMA_MODEL
EOF
}

log() {
  printf '[install] %s\n' "$1"
}

fail() {
  printf '[install] ERROR: %s\n' "$1" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --python)
      [[ $# -ge 2 ]] || fail "Missing value for --python"
      PYTHON_BIN="$2"
      shift 2
      ;;
    --venv)
      [[ $# -ge 2 ]] || fail "Missing value for --venv"
      VENV_DIR="$2"
      shift 2
      ;;
    --db)
      [[ $# -ge 2 ]] || fail "Missing value for --db"
      DB_PATH="$2"
      shift 2
      ;;
    --model)
      [[ $# -ge 2 ]] || fail "Missing value for --model"
      MODEL="$2"
      shift 2
      ;;
    --with-dev)
      WITH_DEV=1
      shift
      ;;
    --with-pdf)
      WITH_PDF=1
      shift
      ;;
    --skip-model-pull)
      SKIP_MODEL_PULL=1
      shift
      ;;
    --install-ollama)
      AUTO_INSTALL_OLLAMA=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "Unknown option: $1"
      ;;
  esac
done

command -v "${PYTHON_BIN}" >/dev/null 2>&1 || fail "Python not found: ${PYTHON_BIN}"

if ! "${PYTHON_BIN}" -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 12) else 1)'; then
  fail "Python 3.12+ is required."
fi

if [[ ! -x "${VENV_DIR}/bin/python" ]]; then
  log "Creating virtual environment at ${VENV_DIR}"
  "${PYTHON_BIN}" -m venv "${VENV_DIR}"
else
  log "Using existing virtual environment at ${VENV_DIR}"
fi

VENV_PY="${VENV_DIR}/bin/python"

log "Upgrading pip tooling"
"${VENV_PY}" -m pip install --upgrade pip setuptools wheel

log "Installing project runtime dependencies"
if ! (cd "${ROOT_DIR}" && "${VENV_PY}" -m pip install -e .); then
  log "Editable install failed, installing runtime dependencies directly"
  "${VENV_PY}" -m pip install "httpx>=0.27" "pyyaml>=6.0" "pydantic>=2.0"
fi

# jinja2 is required when rendering HTML/PDF reports and is lightweight.
"${VENV_PY}" -m pip install "jinja2>=3.1"

if [[ "${WITH_DEV}" -eq 1 ]]; then
  log "Installing developer dependencies"
  if ! (cd "${ROOT_DIR}" && "${VENV_PY}" -m pip install -e ".[dev]"); then
    "${VENV_PY}" -m pip install pytest pytest-asyncio ruff
  fi
fi

if [[ "${WITH_PDF}" -eq 1 ]]; then
  log "Installing evaluat0r PDF dependencies (weasyprint)"
  (cd "${ROOT_DIR}" && "${VENV_PY}" -m pip install -e ./evaluat0r)
fi

if [[ ! -f "${DB_PATH}" && "$(basename -- "${DB_PATH}")" == "default_engagement.db" ]]; then
  log "Creating default engagement database"
  (cd "${ROOT_DIR}" && "${VENV_PY}" scripts/init_default_engagement.py)
fi

log "Seeding prompt library into ${DB_PATH}"
(cd "${ROOT_DIR}" && "${VENV_PY}" scripts/seed_prompts.py --db "${DB_PATH}" --update)

ensure_ollama() {
  if command -v ollama >/dev/null 2>&1; then
    return 0
  fi

  if [[ "${AUTO_INSTALL_OLLAMA}" -eq 1 ]]; then
    if [[ "$(uname -s)" == "Darwin" ]] && command -v brew >/dev/null 2>&1; then
      log "Installing Ollama via Homebrew"
      brew install ollama
      return 0
    fi
    fail "Automatic Ollama install is only supported on macOS + Homebrew in this script."
  fi

  cat >&2 <<'EOF'
[install] ERROR: Ollama is not installed.
[install] Install Ollama first:
[install]   - macOS (Homebrew): brew install ollama
[install]   - or download: https://ollama.com/download
EOF
  exit 1
}

if [[ "${SKIP_MODEL_PULL}" -eq 0 ]]; then
  ensure_ollama

  if ! ollama list >/dev/null 2>&1; then
    log "Starting local Ollama server (ollama serve)"
    nohup ollama serve >/tmp/promt0r-ollama.log 2>&1 &
    sleep 3
  fi

  if ! ollama list >/dev/null 2>&1; then
    fail "Could not reach Ollama. Start it manually with: ollama serve"
  fi

  log "Pulling analyzer model: ${MODEL}"
  if ! ollama pull "${MODEL}"; then
    ollama list || true
    fail "Failed to pull model '${MODEL}'. Use --model <available-model> to choose another."
  fi
fi

cat <<EOF

Installation complete.

Next steps:
  1) Activate virtualenv:
     source "${VENV_DIR}/bin/activate"
  2) Start UI:
     python -m sidecar.dev_server --db "${DB_PATH}" --port 9274
  3) Open:
     http://localhost:9274

Analyzer model:
  ${MODEL}
EOF
