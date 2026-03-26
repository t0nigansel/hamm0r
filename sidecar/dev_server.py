"""Minimal development server for testing the UI without Tauri.

Serves the UI static files and proxies API calls to the sidecar commands.
This is NOT used in production — only for development.

Usage:
    python -m sidecar.dev_server [--db engagement.db] [--port 9274]

Then open http://localhost:9274/ in a browser.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import sys
from http.server import HTTPServer, SimpleHTTPRequestHandler
from pathlib import Path
from threading import Thread
from urllib.parse import urlparse

_PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(_PROJECT_ROOT))

from db.repository import init_db, open_db  # noqa: E402
from sidecar.commands import (  # noqa: E402
    ASYNC_COMMANDS,
    SYNC_COMMANDS,
    SidecarState,
)
from sidecar.protocol import send_event  # noqa: E402

# Global state shared between the HTTP handler and command handlers
_state = SidecarState()


class DevHandler(SimpleHTTPRequestHandler):
    """Serves static files from ui/ and handles API calls at /api."""

    def __init__(self, *args, **kwargs):
        # Serve files from the ui/ directory
        super().__init__(*args, directory=str(_PROJECT_ROOT / "ui"), **kwargs)

    def do_POST(self):
        if self.path == '/api':
            self._handle_api()
        else:
            self.send_error(404)

    def _handle_api(self):
        length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(length)
        req = json.loads(body)

        req_id = req.get("id", "?")
        cmd = req.get("cmd", "")
        params = req.get("params", {})

        # Add CORS headers for dev
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Access-Control-Allow-Origin', '*')
        self.end_headers()

        try:
            if cmd in SYNC_COMMANDS:
                result = SYNC_COMMANDS[cmd](_state, params)
                self.wfile.write(json.dumps({"id": req_id, "ok": True, "data": result}).encode())
            elif cmd in ASYNC_COMMANDS:
                # Run async command in event loop
                loop = asyncio.new_event_loop()
                result = loop.run_until_complete(ASYNC_COMMANDS[cmd](_state, params, req_id))
                loop.close()
                self.wfile.write(json.dumps({"id": req_id, "ok": True, "data": result}).encode())
            elif cmd == "ping":
                self.wfile.write(json.dumps({"id": req_id, "ok": True, "data": {"pong": True}}).encode())
            else:
                self.wfile.write(json.dumps({"id": req_id, "ok": False, "error": f"Unknown command: {cmd}"}).encode())
        except Exception as exc:
            self.wfile.write(json.dumps({"id": req_id, "ok": False, "error": str(exc)}).encode())

    def do_OPTIONS(self):
        """Handle CORS preflight."""
        self.send_response(200)
        self.send_header('Access-Control-Allow-Origin', '*')
        self.send_header('Access-Control-Allow-Methods', 'POST, OPTIONS')
        self.send_header('Access-Control-Allow-Headers', 'Content-Type')
        self.end_headers()

    def log_message(self, format, *args):
        # Quieter logging
        if '/api' in str(args):
            return
        super().log_message(format, *args)


def main():
    parser = argparse.ArgumentParser(description="promt0r dev server")
    parser.add_argument("--db", type=str, default=None, help="Auto-open a .db file on start")
    parser.add_argument("--port", type=int, default=9274, help="Port (default 9274)")
    args = parser.parse_args()

    if args.db:
        _state.db = open_db(args.db)
        init_db(_state.db)
        _state.db_path = args.db
        print(f"Opened DB: {args.db}")

    server = HTTPServer(('localhost', args.port), DevHandler)
    print(f"Dev server running at http://localhost:{args.port}/")
    print("Press Ctrl+C to stop.")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down.")
        if _state.db:
            _state.db.close()
        server.shutdown()


if __name__ == "__main__":
    main()
