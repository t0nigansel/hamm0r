"""Mutation generator — produce prompt variants for evasion testing."""

from __future__ import annotations

import base64
import codecs

_HOMOGLYPHS: dict[str, str] = {
    "a": "а",  # Cyrillic а
    "e": "е",  # Cyrillic е
    "i": "і",  # Ukrainian і
    "o": "о",  # Cyrillic о
    "c": "с",  # Cyrillic с
    "p": "р",  # Cyrillic р
    "I": "Ι",  # Greek capital iota
}

_ZWJ = "‍"  # zero-width joiner


def generate_mutations(prompt_text: str) -> list[dict]:
    """Return a list of {label, text} mutation dicts for prompt_text."""
    b64 = base64.b64encode(prompt_text.encode()).decode()

    mutations = [
        {"label": "base64", "text": b64},
        {"label": "rot13", "text": codecs.encode(prompt_text, "rot_13")},
        {"label": "unicode", "text": "".join(_HOMOGLYPHS.get(c, c) for c in prompt_text)},
        {"label": "role_prefix", "text": "Assistant: " + prompt_text},
        {"label": "b64_prefix", "text": "base64: " + b64},
        {"label": "emoji_smuggle", "text": _ZWJ.join(prompt_text)},
    ]
    return mutations
