"""Ollama client wrapper — communicates via the OpenAI-compatible local endpoint.

Stack.md:
  - Target model: qwen2.5:14b, fallback qwen2.5:7b
  - Endpoint: http://localhost:11434/v1
  - evaluat0r only — promt0r never calls Ollama
  - No data leaves the machine
"""

from __future__ import annotations

import json
from typing import Optional

import httpx


# Default Ollama endpoint (OpenAI-compatible)
DEFAULT_BASE_URL = "http://localhost:11434/v1"
DEFAULT_MODEL = "qwen2.5:14b"
FALLBACK_MODEL = "qwen2.5:7b"


async def chat_completion(
    prompt: str,
    *,
    model: str = DEFAULT_MODEL,
    base_url: str = DEFAULT_BASE_URL,
    temperature: float = 0.0,
    max_tokens: int = 1024,
    client: Optional[httpx.AsyncClient] = None,
) -> str:
    """Send a chat completion request to Ollama and return the response text.

    Uses the OpenAI-compatible /v1/chat/completions endpoint so this
    could also work with any OpenAI-compatible local server.
    """
    url = f"{base_url.rstrip('/')}/chat/completions"
    payload = {
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": temperature,
        "max_tokens": max_tokens,
    }

    # Decision: temperature=0.0 for deterministic judge output.
    # Security evaluation should be reproducible.

    should_close = client is None
    if client is None:
        client = httpx.AsyncClient(timeout=httpx.Timeout(connect=10.0, read=120.0, write=30.0, pool=10.0))

    try:
        resp = await client.post(url, json=payload)
        resp.raise_for_status()
        data = resp.json()
        return data["choices"][0]["message"]["content"]
    finally:
        if should_close:
            await client.aclose()


async def check_ollama_available(
    base_url: str = DEFAULT_BASE_URL,
    model: str = DEFAULT_MODEL,
) -> dict:
    """Check if Ollama is running and the model is available.

    Returns {"available": True/False, "model": str, "error": str|None}
    """
    try:
        async with httpx.AsyncClient(timeout=httpx.Timeout(5.0, connect=5.0, read=5.0, write=5.0, pool=5.0)) as client:
            # Check if Ollama is reachable via the tags endpoint
            resp = await client.get(f"{base_url.rstrip('/')}/models")
            resp.raise_for_status()
            data = resp.json()

            # Check if our model is in the list
            model_ids = [m.get("id", "") for m in data.get("data", [])]
            if model in model_ids:
                return {"available": True, "model": model, "error": None}

            # Try fallback model
            if FALLBACK_MODEL in model_ids:
                return {"available": True, "model": FALLBACK_MODEL, "error": None}

            return {
                "available": False,
                "model": model,
                "error": f"Model '{model}' not found. Available: {model_ids}. "
                         f"Run: ollama pull {model}",
            }
    except Exception as exc:
        return {
            "available": False,
            "model": model,
            "error": f"Ollama not reachable at {base_url}: {exc}",
        }
