"""Tests for runner/target_config.py"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

import pytest

from runner.target_config import TargetConfig, load_config, save_config


class TestTargetConfig:
    def test_defaults(self):
        cfg = TargetConfig(name="Test", url="http://localhost", endpoint_type="openai_compat")
        assert cfg.auth_type == "none"
        assert cfg.concurrency == 1
        assert cfg.delay_ms == 0
        assert cfg.timeout_connect == 10.0
        assert cfg.timeout_read == 30.0
        assert cfg.verify_ssl is True

    def test_auth_headers_none(self):
        cfg = TargetConfig(name="T", url="http://x", endpoint_type="openai_compat")
        assert cfg.auth_headers() == {}

    def test_auth_headers_bearer(self):
        cfg = TargetConfig(
            name="T", url="http://x", endpoint_type="openai_compat",
            auth_type="bearer", auth_value="sk-test123",
        )
        assert cfg.auth_headers() == {"Authorization": "Bearer sk-test123"}

    def test_auth_headers_bearer_custom_header(self):
        cfg = TargetConfig(
            name="T", url="http://x", endpoint_type="openai_compat",
            auth_type="bearer", auth_value="tok", auth_header="X-Custom",
        )
        assert cfg.auth_headers() == {"X-Custom": "Bearer tok"}

    def test_auth_headers_api_key(self):
        cfg = TargetConfig(
            name="T", url="http://x", endpoint_type="openai_compat",
            auth_type="api_key", auth_value="mykey",
        )
        assert cfg.auth_headers() == {"X-Api-Key": "mykey"}

    def test_auth_headers_basic(self):
        cfg = TargetConfig(
            name="T", url="http://x", endpoint_type="openai_compat",
            auth_type="basic", auth_value="dXNlcjpwYXNz",
        )
        assert cfg.auth_headers() == {"Authorization": "Basic dXNlcjpwYXNz"}


class TestLoadSaveConfig:
    def test_roundtrip(self, tmp_path: Path):
        cfg = TargetConfig(
            name="Acme Bot",
            url="https://api.acme.com/chat",
            endpoint_type="openai_compat",
            auth_type="bearer",
            auth_value="sk-xyz",
            concurrency=3,
            delay_ms=200,
        )
        path = tmp_path / "target.json"
        save_config(cfg, path)

        loaded = load_config(path)
        assert loaded.name == "Acme Bot"
        assert loaded.url == "https://api.acme.com/chat"
        assert loaded.auth_type == "bearer"
        assert loaded.auth_value == "sk-xyz"
        assert loaded.concurrency == 3

    def test_ignores_unknown_keys(self, tmp_path: Path):
        path = tmp_path / "target.json"
        path.write_text(json.dumps({
            "name": "T",
            "url": "http://x",
            "endpoint_type": "openai_compat",
            "unknown_future_field": True,
        }))
        cfg = load_config(path)
        assert cfg.name == "T"

    def test_save_drops_none(self, tmp_path: Path):
        cfg = TargetConfig(name="T", url="http://x", endpoint_type="openai_compat")
        path = tmp_path / "target.json"
        save_config(cfg, path)
        data = json.loads(path.read_text())
        assert "auth_value" not in data
        assert "notes" not in data
