"""HTTP client adapters — normalise different AI API shapes.

Architecture.md defines the adapter pattern:

    TargetAdapter (abstract)
    ├── OpenAICompatAdapter    # /v1/chat/completions
    ├── CustomRESTAdapter      # user-defined field mapping
    └── RawHTTPAdapter         # full control (backlog, not implemented yet)

Each adapter's send() takes a prompt string and returns the extracted
response string.  The runner doesn't care which adapter is in use.
"""

from __future__ import annotations

import abc
import json
from dataclasses import dataclass

import httpx

from runner.target_config import TargetConfig


@dataclass
class AdapterResponse:
    """Uniform response from any adapter."""
    text: str | None       # extracted response text, None on error
    status_code: int | None  # HTTP status, None on connection-level failure
    error: str | None      # error description, None on success


class TargetAdapter(abc.ABC):
    """Abstract base for target API adapters."""

    def __init__(self, config: TargetConfig, client: httpx.AsyncClient) -> None:
        self.config = config
        self.client = client

    @abc.abstractmethod
    async def send(self, prompt: str) -> AdapterResponse:
        """Send a prompt to the target and return the normalised response."""


class OpenAICompatAdapter(TargetAdapter):
    """Adapter for OpenAI-compatible /v1/chat/completions endpoints.

    Covers: OpenAI, Azure OpenAI, Anthropic proxy, vLLM, Ollama, LiteLLM,
    and most hosted LLM APIs that follow the OpenAI chat format.
    """

    async def send(self, prompt: str) -> AdapterResponse:
        messages = [{"role": "user", "content": prompt}]
        if self.config.system_prompt:
            messages.insert(0, {"role": "system", "content": self.config.system_prompt})

        payload = {"messages": messages}
        # Decision: we intentionally omit "model" from the payload.
        # Most proxy endpoints (vLLM, Ollama, LiteLLM) route to a single model
        # and ignore this field.  If needed, model can be added to field_mapping
        # in a future version, or the target URL can include it in the path.

        # Log request for debugging
        auth_headers = self.config.auth_headers()
        print(f"\n[DEBUG] Sending request to: {self.config.url}")
        print(f"[DEBUG] Headers: {json.dumps(auth_headers, indent=2)}")
        print(f"[DEBUG] Payload: {json.dumps(payload, indent=2)[:200]}...")

        try:
            resp = await self.client.post(
                self.config.url,
                json=payload,
                headers=auth_headers,
            )
        except httpx.TimeoutException as exc:
            return AdapterResponse(text=None, status_code=None, error=f"Timeout: {exc}")
        except httpx.ConnectError as exc:
            return AdapterResponse(text=None, status_code=None, error=f"Connection error: {exc}")
        except httpx.HTTPError as exc:
            return AdapterResponse(text=None, status_code=None, error=f"HTTP error: {exc}")

        if resp.status_code != 200:
            # Record the body for diagnostics but cap it to avoid huge error messages
            body_preview = resp.text[:500] if resp.text else ""
            print(f"[DEBUG] Response status: {resp.status_code}")
            print(f"[DEBUG] Response body: {body_preview}")
            return AdapterResponse(
                text=None,
                status_code=resp.status_code,
                error=f"HTTP {resp.status_code}: {body_preview}",
            )

        try:
            data = resp.json()
            text = data["choices"][0]["message"]["content"]
        except (KeyError, IndexError, TypeError) as exc:
            return AdapterResponse(
                text=None,
                status_code=resp.status_code,
                error=f"Failed to parse OpenAI response: {exc}. Body: {resp.text[:300]}",
            )

        return AdapterResponse(text=text, status_code=resp.status_code, error=None)


class CustomRESTAdapter(TargetAdapter):
    """Adapter for arbitrary REST APIs with user-defined field mapping.

    The field_mapping dict in TargetConfig maps our field names to the
    target's field names:

        {
            "request_field": "input",     # field name for the prompt in request body
            "response_field": "output"    # field name to extract response from JSON body
        }

    Defaults if not specified:
        request_field  = "message"
        response_field = "response"
    """

    async def send(self, prompt: str) -> AdapterResponse:
        mapping = self.config.field_mapping or {}
        req_field = mapping.get("request_field", "message")
        resp_field = mapping.get("response_field", "response")

        payload = {req_field: prompt}

        # Decision: if system_prompt is set, include it as a separate field.
        # There is no standard for custom REST APIs, so we use "system_prompt"
        # as the key.  This can be overridden via field_mapping in a future version.
        if self.config.system_prompt:
            sys_field = mapping.get("system_prompt_field", "system_prompt")
            payload[sys_field] = self.config.system_prompt

        # Log request for debugging
        auth_headers = self.config.auth_headers()
        print(f"\n[DEBUG] Sending request to: {self.config.url}")
        print(f"[DEBUG] Headers: {json.dumps(auth_headers, indent=2)}")
        print(f"[DEBUG] Payload: {json.dumps(payload, indent=2)[:200]}...")

        try:
            resp = await self.client.post(
                self.config.url,
                json=payload,
                headers=auth_headers,
            )
        except httpx.TimeoutException as exc:
            return AdapterResponse(text=None, status_code=None, error=f"Timeout: {exc}")
        except httpx.ConnectError as exc:
            return AdapterResponse(text=None, status_code=None, error=f"Connection error: {exc}")
        except httpx.HTTPError as exc:
            return AdapterResponse(text=None, status_code=None, error=f"HTTP error: {exc}")

        if resp.status_code != 200:
            body_preview = resp.text[:500] if resp.text else ""
            print(f"[DEBUG] Response status: {resp.status_code}")
            print(f"[DEBUG] Response body: {body_preview}")
            return AdapterResponse(
                text=None,
                status_code=resp.status_code,
                error=f"HTTP {resp.status_code}: {body_preview}",
            )

        try:
            data = resp.json()
            # Support nested response fields with dot notation, e.g. "data.text"
            text = data
            for key in resp_field.split("."):
                text = text[key]
            if not isinstance(text, str):
                text = str(text)
        except (KeyError, IndexError, TypeError) as exc:
            return AdapterResponse(
                text=None,
                status_code=resp.status_code,
                error=f"Failed to extract '{resp_field}' from response: {exc}. Body: {resp.text[:300]}",
            )

        return AdapterResponse(text=text, status_code=resp.status_code, error=None)


def create_adapter(config: TargetConfig, client: httpx.AsyncClient) -> TargetAdapter:
    """Factory: build the right adapter based on endpoint_type."""
    if config.endpoint_type == "openai_compat":
        return OpenAICompatAdapter(config, client)
    elif config.endpoint_type == "custom_rest":
        return CustomRESTAdapter(config, client)
    elif config.endpoint_type == "raw_http":
        # RawHTTPAdapter is in the backlog — not implemented yet.
        raise NotImplementedError(
            "raw_http endpoint type is planned for a future version. "
            "Use openai_compat or custom_rest for now."
        )
    else:
        raise ValueError(f"Unknown endpoint_type: {config.endpoint_type!r}")
