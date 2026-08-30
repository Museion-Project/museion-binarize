#!/usr/bin/env python3
"""Reference mirror of the shipped conservative Greek text enhancer.

This module is deliberately smaller than a spellchecker.  It has no corpus,
dictionary, language model, page-image, box, or gold-text input.  It mirrors
``ConservativeGreekEnhancer`` in
``crates/mpdf-core/src/ocr_provider/text_enhancer.rs``: NFC is applied first;
a token is eligible at normalized Tesseract word confidence >= 0.95 when it
contains Greek and every alphabetic character is Greek or one of the fixed
Latin lookalikes; those lookalikes are then replaced one code point for one
code point.  NFC normalization is the only other operation.

The caller keeps token identity and geometry.  :func:`enhance_token` returns
text and an audit count; it cannot create, remove, split, or merge tokens.
"""

from __future__ import annotations

import unicodedata
from dataclasses import dataclass

# This threshold is part of the pre-registered candidate.  Changing it changes
# the candidate digest emitted by the CGPG bake-off runner.
MINIMUM_CONFIDENCE = 0.95
ALGORITHM_ID = "conservative-greek-text"
ALGORITHM_VERSION = "1"

# Exact one-to-one mapping shipped by the Rust candidate.
LATIN_TO_GREEK: dict[str, str] = {
    "A": "Α",
    "B": "Β",
    "E": "Ε",
    "H": "Η",
    "I": "Ι",
    "K": "Κ",
    "M": "Μ",
    "N": "Ν",
    "O": "Ο",
    "P": "Ρ",
    "T": "Τ",
    "X": "Χ",
    "Y": "Υ",
    "Z": "Ζ",
    "o": "ο",
    "p": "ρ",
    "x": "χ",
}


@dataclass(frozen=True)
class Enhancement:
    text: str
    rewrites: int
    normalized: bool


def _is_greek(character: str) -> bool:
    codepoint = ord(character)
    return (0x0370 <= codepoint <= 0x03FF) or (0x1F00 <= codepoint <= 0x1FFF)


def is_conservative_greek_token(text: str) -> bool:
    """Exact text eligibility predicate used by the shipped Rust candidate."""
    return any(_is_greek(character) for character in text) and all(
        _is_greek(character) or character in LATIN_TO_GREEK
        for character in text
        if character.isalpha()
    )


def enhance_token(text: str, confidence: float) -> Enhancement:
    """NFC-normalizes and conservatively repairs one OCR token.

    ``confidence`` is Tesseract's word confidence divided by 100.  Every
    lookalike repair replaces exactly one code point with exactly one code
    point, including at token edges, exactly as the Rust implementation does.
    """
    normalized = unicodedata.normalize("NFC", text)
    was_normalized = normalized != text
    if confidence < MINIMUM_CONFIDENCE or not is_conservative_greek_token(normalized):
        return Enhancement(normalized, 0, was_normalized)

    characters = list(normalized)
    rewrites = 0
    for index in range(len(characters)):
        replacement = LATIN_TO_GREEK.get(characters[index])
        if replacement is None:
            continue
        characters[index] = replacement
        rewrites += 1
    return Enhancement("".join(characters), rewrites, was_normalized)
