"""Tests for runner/http_client.py — uses httpx MockTransport per Stack.md."""

from __future__ import annotations

import json

import httpx
import pytest
import pytest_asyncio

from runner.http_client import (
    CustomRESTAdapter,
    OpenAICompatAdapter,
    create_adapter,
)
from runner.target_config import TargetConfig


# ---------------------------------------------------------------------------
# Helpers: build mock transports
# ---------------------------------------------------------------------------

def _openai_ok_transport(response_text: str = "Hello from LLM") -> httpx.MockTransport:
    """Transport that returns a valid OpenAI chat completion response."""
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            json={
                "choices": [
                    {"message": {"role": "assistant", "content": response_text}}
                ]
            },
        )
    return httpx.MockTransport(handler)


def _custom_rest_ok_transport(
    resp_field: str = "response",
    response_text: str = "Custom response",
) -> httpx.MockTransport:
    """Transport that returns a simple JSON response with configurable field."""
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, json={resp_field: response_text})
    return httpx.MockTransport(handler)


def _error_transport(status: int, body: str = "error") -> httpx.MockTransport:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(status, text=body)
    return httpx.MockTransport(handler)


def _timeout_transport() -> httpx.MockTransport:
    def handler(request: httpx.Request) -> httpx.Response:
        raise httpx.ReadTimeout("mock timeout")
    return httpx.MockTransport(handler)


def _connect_error_transport() -> httpx.MockTransport:
    def handler(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("mock connection refused")
    return httpx.MockTransport(handler)


def _capture_transport(captured: list) -> httpx.MockTransport:
    """Transport that captures request details and returns a valid OpenAI response."""
    def handler(request: httpx.Request) -> httpx.Response:
        captured.append({
            "url": str(request.url),
            "headers": dict(request.headers),
            "body": json.loads(request.content) if request.content else None,
        })
        return httpx.Response(
            200,
            json={"choices": [{"message": {"role": "assistant", "content": "ok"}}]},
        )
    return httpx.MockTransport(handler)


def _make_config(**overrides) -> TargetConfig:
    defaults = dict(
        name="Test Target",
        url="http://test-target/v1/chat/completions",
        endpoint_type="openai_compat",
    )
    defaults.update(overrides)
    return TargetConfig(**defaults)


# ---------------------------------------------------------------------------
# OpenAICompatAdapter tests
# ---------------------------------------------------------------------------

class TestOpenAICompatAdapter:
    @pytest.mark.asyncio
    async def test_success(self):
        config = _make_config()
        async with httpx.AsyncClient(transport=_openai_ok_transport("Hi there")) as client:
            adapter = OpenAICompatAdapter(config, client)
            resp = await adapter.send("Hello")
            assert resp.text == "Hi there"
            assert resp.status_code == 200
            assert resp.error is None

    @pytest.mark.asyncio
    async def test_sends_system_prompt(self):
        captured: list = []
        config = _make_config(system_prompt="You are helpful.")
        async with httpx.AsyncClient(transport=_capture_transport(captured)) as client:
            adapter = OpenAICompatAdapter(config, client)
            await adapter.send("Hello")
        body = captured[0]["body"]
        assert body["messages"][0] == {"role": "system", "content": "You are helpful."}
        assert body["messages"][1] == {"role": "user", "content": "Hello"}

    @pytest.mark.asyncio
    async def test_sends_auth_header(self):
        captured: list = []
        config = _make_config(auth_type="bearer", auth_value="sk-test")
        async with httpx.AsyncClient(transport=_capture_transport(captured)) as client:
            adapter = OpenAICompatAdapter(config, client)
            await adapter.send("Hello")
        assert "bearer sk-test" in captured[0]["headers"]["authorization"].lower()

    @pytest.mark.asyncio
    async def test_http_error(self):
        config = _make_config()
        async with httpx.AsyncClient(transport=_error_transport(429, "rate limited")) as client:
            adapter = OpenAICompatAdapter(config, client)
            resp = await adapter.send("Hello")
            assert resp.text is None
            assert resp.status_code == 429
            assert "429" in resp.error

    @pytest.mark.asyncio
    async def test_timeout(self):
        config = _make_config()
        async with httpx.AsyncClient(transport=_timeout_transport()) as client:
            adapter = OpenAICompatAdapter(config, client)
            resp = await adapter.send("Hello")
            assert resp.text is None
            assert resp.status_code is None
            assert "Timeout" in resp.error

    @pytest.mark.asyncio
    async def test_connect_error(self):
        config = _make_config()
        async with httpx.AsyncClient(transport=_connect_error_transport()) as client:
            adapter = OpenAICompatAdapter(config, client)
            resp = await adapter.send("Hello")
            assert resp.text is None
            assert resp.status_code is None
            assert "Connection error" in resp.error

    @pytest.mark.asyncio
    async def test_malformed_json_response(self):
        """Response is 200 but body doesn't match OpenAI format."""
        def handler(request: httpx.Request) -> httpx.Response:
            return httpx.Response(200, json={"bad": "format"})

        config = _make_config()
        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            adapter = OpenAICompatAdapter(config, client)
            resp = await adapter.send("Hello")
            assert resp.text is None
            assert resp.status_code == 200
            assert "Failed to parse" in resp.error


# ---------------------------------------------------------------------------
# CustomRESTAdapter tests
# ---------------------------------------------------------------------------

class TestCustomRESTAdapter:
    @pytest.mark.asyncio
    async def test_success_default_fields(self):
        config = _make_config(endpoint_type="custom_rest")
        async with httpx.AsyncClient(transport=_custom_rest_ok_transport()) as client:
            adapter = CustomRESTAdapter(config, client)
            resp = await adapter.send("Hello")
            assert resp.text == "Custom response"
            assert resp.error is None

    @pytest.mark.asyncio
    async def test_custom_field_mapping(self):
        captured: list = []
        config = _make_config(
            endpoint_type="custom_rest",
            field_mapping={"request_field": "input", "response_field": "output"},
        )

        def handler(request: httpx.Request) -> httpx.Response:
            body = json.loads(request.content)
            captured.append(body)
            return httpx.Response(200, json={"output": "mapped response"})

        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            adapter = CustomRESTAdapter(config, client)
            resp = await adapter.send("Hello")
            assert captured[0]["input"] == "Hello"
            assert resp.text == "mapped response"

    @pytest.mark.asyncio
    async def test_nested_response_field(self):
        """Support dot notation in response_field: 'data.text'"""
        config = _make_config(
            endpoint_type="custom_rest",
            field_mapping={"response_field": "data.text"},
        )

        def handler(request: httpx.Request) -> httpx.Response:
            return httpx.Response(200, json={"data": {"text": "nested value"}})

        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            adapter = CustomRESTAdapter(config, client)
            resp = await adapter.send("Hello")
            assert resp.text == "nested value"

    @pytest.mark.asyncio
    async def test_system_prompt_included(self):
        captured: list = []
        config = _make_config(
            endpoint_type="custom_rest",
            system_prompt="Be helpful",
        )

        def handler(request: httpx.Request) -> httpx.Response:
            captured.append(json.loads(request.content))
            return httpx.Response(200, json={"response": "ok"})

        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            adapter = CustomRESTAdapter(config, client)
            await adapter.send("Hello")
            assert captured[0]["system_prompt"] == "Be helpful"

    @pytest.mark.asyncio
    async def test_http_error(self):
        config = _make_config(endpoint_type="custom_rest")
        async with httpx.AsyncClient(transport=_error_transport(500, "internal")) as client:
            adapter = CustomRESTAdapter(config, client)
            resp = await adapter.send("Hello")
            assert resp.text is None
            assert resp.status_code == 500

    @pytest.mark.asyncio
    async def test_missing_response_field(self):
        config = _make_config(
            endpoint_type="custom_rest",
            field_mapping={"response_field": "nonexistent"},
        )

        def handler(request: httpx.Request) -> httpx.Response:
            return httpx.Response(200, json={"other_field": "value"})

        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            adapter = CustomRESTAdapter(config, client)
            resp = await adapter.send("Hello")
            assert resp.text is None
            assert "Failed to extract" in resp.error


# ---------------------------------------------------------------------------
# Factory tests
# ---------------------------------------------------------------------------

class TestCreateAdapter:
    @pytest.mark.asyncio
    async def test_openai_compat(self):
        config = _make_config(endpoint_type="openai_compat")
        async with httpx.AsyncClient(transport=_openai_ok_transport()) as client:
            adapter = create_adapter(config, client)
            assert isinstance(adapter, OpenAICompatAdapter)

    @pytest.mark.asyncio
    async def test_custom_rest(self):
        config = _make_config(endpoint_type="custom_rest")
        async with httpx.AsyncClient(transport=_custom_rest_ok_transport()) as client:
            adapter = create_adapter(config, client)
            assert isinstance(adapter, CustomRESTAdapter)

    @pytest.mark.asyncio
    async def test_raw_http_not_implemented(self):
        config = _make_config(endpoint_type="raw_http")
        async with httpx.AsyncClient() as client:
            with pytest.raises(NotImplementedError):
                create_adapter(config, client)

    @pytest.mark.asyncio
    async def test_unknown_type(self):
        config = _make_config(endpoint_type="banana")
        async with httpx.AsyncClient() as client:
            with pytest.raises(ValueError, match="banana"):
                create_adapter(config, client)
