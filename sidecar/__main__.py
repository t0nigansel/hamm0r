"""Python sidecar entry point — reads JSON-lines from stdin, dispatches commands.

Run with: python -m sidecar

The Tauri Rust shell spawns this as a child process.
Communication is via JSON-lines on stdin/stdout (see protocol.py).
stderr is used for debug logging.
"""

from __future__ import annotations

import asyncio
import json
import sys
from pathlib import Path

# Ensure project root is importable
_PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(_PROJECT_ROOT))

from sidecar.commands import ASYNC_COMMANDS, SYNC_COMMANDS, SidecarState  # noqa: E402
from sidecar.protocol import read_request, send_error, send_response  # noqa: E402


async def _handle_request(state: SidecarState, req: dict) -> None:
    """Dispatch a single request to the appropriate command handler."""
    req_id = req.get("id", "?")
    cmd = req.get("cmd", "")
    params = req.get("params", {})

    if cmd in SYNC_COMMANDS:
        try:
            result = SYNC_COMMANDS[cmd](state, params)
            send_response(req_id, result)
        except Exception as exc:
            send_error(req_id, str(exc))

    elif cmd in ASYNC_COMMANDS:
        try:
            result = await ASYNC_COMMANDS[cmd](state, params, req_id)
            send_response(req_id, result)
        except asyncio.CancelledError:
            send_error(req_id, "Run cancelled")
        except Exception as exc:
            send_error(req_id, str(exc))

    elif cmd == "ping":
        send_response(req_id, {"pong": True})

    elif cmd == "quit":
        send_response(req_id, {"bye": True})
        sys.exit(0)

    else:
        send_error(req_id, f"Unknown command: {cmd}")


async def main_loop() -> None:
    """Main event loop — read stdin, dispatch commands."""
    state = SidecarState()

    # Log to stderr so it doesn't interfere with the JSON protocol on stdout
    print("sidecar: ready", file=sys.stderr, flush=True)

    loop = asyncio.get_running_loop()

    while True:
        # Read from stdin in a thread so we don't block the event loop
        # (important: async commands like start_run need the loop free)
        line = await loop.run_in_executor(None, sys.stdin.readline)
        if not line:
            break  # EOF — parent process closed stdin

        line = line.strip()
        if not line:
            continue

        try:
            req = json.loads(line)
        except json.JSONDecodeError as exc:
            send_error("?", f"Invalid JSON: {exc}")
            continue

        await _handle_request(state, req)

    # Clean up
    if state.db is not None:
        state.db.close()
    print("sidecar: shutdown", file=sys.stderr, flush=True)


def main() -> None:
    asyncio.run(main_loop())


if __name__ == "__main__":
    main()
