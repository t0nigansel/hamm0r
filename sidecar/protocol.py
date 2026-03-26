"""JSON-lines protocol for Tauri ↔ Python sidecar communication.

Protocol format (one JSON object per line):

  → stdin  (request):   {"id": "1", "cmd": "list_prompts", "params": {}}
  ← stdout (response):  {"id": "1", "ok": true, "data": [...]}
  ← stdout (error):     {"id": "1", "ok": false, "error": "message"}
  ← stdout (event):     {"id": "1", "event": "progress", "data": {...}}

Design decisions:
  - Every request has a unique "id" so the Rust side can match responses.
  - Progress events share the request id of the command that triggered them.
  - stderr is reserved for debug logging (not part of the protocol).
"""

from __future__ import annotations

import json
import sys


def read_request() -> dict | None:
    """Read one JSON-line request from stdin. Returns None on EOF."""
    line = sys.stdin.readline()
    if not line:
        return None
    return json.loads(line.strip())


def send_response(req_id: str, data: object) -> None:
    """Send a success response."""
    msg = json.dumps({"id": req_id, "ok": True, "data": data})
    sys.stdout.write(msg + "\n")
    sys.stdout.flush()


def send_error(req_id: str, error: str) -> None:
    """Send an error response."""
    msg = json.dumps({"id": req_id, "ok": False, "error": error})
    sys.stdout.write(msg + "\n")
    sys.stdout.flush()


def send_event(req_id: str, event_type: str, data: object) -> None:
    """Send a progress/status event (not a final response)."""
    msg = json.dumps({"id": req_id, "event": event_type, "data": data})
    sys.stdout.write(msg + "\n")
    sys.stdout.flush()
