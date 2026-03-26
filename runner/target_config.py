"""Target configuration — load/save JSON config files for attack targets.

A target config JSON file describes the system under test and how to
authenticate to it.  The tester creates one config per engagement target.

Example target.json:
{
    "name": "Acme HR Chatbot",
    "url": "https://api.acme.com/v1/chat/completions",
    "endpoint_type": "openai_compat",
    "auth_type": "bearer",
    "auth_value": "sk-...",
    "system_prompt": "You are Acme's HR assistant.",
    "concurrency": 2,
    "delay_ms": 500,
    "timeout_connect": 10.0,
    "timeout_read": 30.0,
    "verify_ssl": true
}
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class TargetConfig:
    """Runtime configuration for a single attack target."""

    # Required
    name: str
    url: str
    endpoint_type: str  # openai_compat | custom_rest | raw_http

    # Auth
    auth_type: str = "none"  # none | bearer | api_key | basic
    auth_value: str | None = None  # the token / key / base64(user:pass)
    auth_header: str | None = None  # header name override (default depends on auth_type)

    # Custom REST adapter — field mapping
    # Decision: field_mapping lives here rather than only in the DB Target row
    # so the JSON config file is self-contained and portable.
    field_mapping: dict[str, str] | None = None

    # Optional target-level system prompt to inject
    system_prompt: str | None = None

    # Run parameters
    concurrency: int = 1
    delay_ms: int = 0
    tester_name: str = "default"

    # HTTP tuning (Stack.md: connect=10, read=30)
    timeout_connect: float = 10.0
    timeout_read: float = 30.0
    verify_ssl: bool = True

    # Notes
    notes: str | None = None

    def auth_headers(self) -> dict[str, str]:
        """Build the authentication headers dict for httpx."""
        if self.auth_type == "none" or not self.auth_value:
            return {}

        match self.auth_type:
            case "bearer":
                header = self.auth_header or "Authorization"
                return {header: f"Bearer {self.auth_value}"}
            case "api_key":
                # Decision: default header for api_key is "X-Api-Key" which is
                # the most common convention.  Overridable via auth_header.
                header = self.auth_header or "X-Api-Key"
                return {header: self.auth_value}
            case "basic":
                header = self.auth_header or "Authorization"
                return {header: f"Basic {self.auth_value}"}
            case _:
                return {}


def load_config(path: str | Path) -> TargetConfig:
    """Load a TargetConfig from a JSON file."""
    raw = json.loads(Path(path).read_text())
    return _dict_to_config(raw)


def save_config(config: TargetConfig, path: str | Path) -> None:
    """Save a TargetConfig to a JSON file."""
    Path(path).write_text(json.dumps(_config_to_dict(config), indent=2) + "\n")


def _dict_to_config(d: dict) -> TargetConfig:
    """Build a TargetConfig from a dict, ignoring unknown keys."""
    known_fields = {f.name for f in TargetConfig.__dataclass_fields__.values()}
    filtered = {k: v for k, v in d.items() if k in known_fields}
    return TargetConfig(**filtered)


def _config_to_dict(config: TargetConfig) -> dict:
    """Serialize a TargetConfig to a dict, dropping None values."""
    from dataclasses import asdict
    return {k: v for k, v in asdict(config).items() if v is not None}
