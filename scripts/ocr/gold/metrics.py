#!/usr/bin/env python3
"""Deterministic OCR accuracy metrics for the M PDF Processor gold set.

The metrics here are the only thing allowed to decide which OCR engine may
become a production default.  A provider's own confidence is *not* accuracy
and never appears in this module.

Every metric operates on NFC-normalized text, because the project contract is
that recognized text is emitted in NFC.  ``nfc_violations`` separately reports
how often a candidate emitted something that was *not* already NFC, so a
candidate cannot pass by relying on this module to clean up after it.
"""

from __future__ import annotations

import unicodedata
from dataclasses import dataclass, field
from typing import Iterable, Sequence

# Greek combining and precomposed marks whose loss is the specific failure this
# round exists to eliminate (breathings, accents, diaeresis, iota subscript).
_COMBINING_OF_INTEREST = frozenset(
    "̀́̄̆̈̓̔͂̓ͅͅ"
)

# Latin letters that a Greek-unaware recognizer typically substitutes for a
# visually similar Greek letter.  Both directions are counted as a script
# confusion because either one silently destroys searchability.
LOOKALIKE_PAIRS: tuple[tuple[str, str], ...] = (
    ("Α", "A"), ("Β", "B"), ("Ε", "E"), ("Ζ", "Z"), ("Η", "H"), ("Ι", "I"),
    ("Κ", "K"), ("Μ", "M"), ("Ν", "N"), ("Ο", "O"), ("Ρ", "P"), ("Τ", "T"),
    ("Υ", "Y"), ("Χ", "X"), ("α", "a"), ("ο", "o"), ("ν", "v"), ("υ", "u"),
    ("ρ", "p"), ("κ", "k"), ("χ", "x"), ("ι", "i"), ("ω", "w"), ("γ", "y"),
    ("ε", "e"), ("μ", "u"), ("τ", "t"),
)
_GREEK_TO_LATIN = {greek: latin for greek, latin in LOOKALIKE_PAIRS}


def nfc(text: str) -> str:
    return unicodedata.normalize("NFC", text)


def is_nfc(text: str) -> bool:
    return unicodedata.is_normalized("NFC", text)


def _norm_ws(text: str) -> str:
    return " ".join(nfc(text).split())


def levenshtein(a: Sequence[str], b: Sequence[str]) -> int:
    """Plain edit distance.  O(len(a) * len(b)) time, O(len(b)) space."""
    if a == b:
        return 0
    if not a:
        return len(b)
    if not b:
        return len(a)
    previous = list(range(len(b) + 1))
    for i, ai in enumerate(a, start=1):
        current = [i]
        for j, bj in enumerate(b, start=1):
            current.append(
                min(
                    previous[j] + 1,
                    current[j - 1] + 1,
                    previous[j - 1] + (0 if ai == bj else 1),
                )
            )
        previous = current
    return previous[-1]


def _align(reference: Sequence[str], hypothesis: Sequence[str]) -> list[tuple[str | None, str | None]]:
    """Backtracked Levenshtein alignment; ``None`` marks an insert/delete."""
    n, m = len(reference), len(hypothesis)
    table = [[0] * (m + 1) for _ in range(n + 1)]
    for i in range(n + 1):
        table[i][0] = i
    for j in range(m + 1):
        table[0][j] = j
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            cost = 0 if reference[i - 1] == hypothesis[j - 1] else 1
            table[i][j] = min(
                table[i - 1][j] + 1, table[i][j - 1] + 1, table[i - 1][j - 1] + cost
            )
    pairs: list[tuple[str | None, str | None]] = []
    i, j = n, m
    while i > 0 or j > 0:
        if i > 0 and j > 0:
            cost = 0 if reference[i - 1] == hypothesis[j - 1] else 1
            if table[i][j] == table[i - 1][j - 1] + cost:
                pairs.append((reference[i - 1], hypothesis[j - 1]))
                i, j = i - 1, j - 1
                continue
        if i > 0 and table[i][j] == table[i - 1][j] + 1:
            pairs.append((reference[i - 1], None))
            i -= 1
            continue
        pairs.append((None, hypothesis[j - 1]))
        j -= 1
    pairs.reverse()
    return pairs


def _marks(character: str) -> frozenset[str]:
    decomposed = unicodedata.normalize("NFD", character)
    return frozenset(c for c in decomposed if c in _COMBINING_OF_INTEREST)


def _base(character: str) -> str:
    decomposed = unicodedata.normalize("NFD", character)
    stripped = "".join(c for c in decomposed if not unicodedata.combining(c))
    return unicodedata.normalize("NFC", stripped)


@dataclass
class Metrics:
    """One candidate's score on one gold sample (or an aggregate of many)."""

    reference_chars: int = 0
    char_edits: int = 0
    reference_words: int = 0
    word_edits: int = 0
    exact_words: int = 0
    diacritic_bearing_chars: int = 0
    diacritic_errors: int = 0
    script_confusions: int = 0
    nfc_violations: int = 0
    samples: int = 0
    exact_lines: int = 0
    reference_lines: int = 0
    diacritic_error_examples: list[str] = field(default_factory=list)
    script_confusion_examples: list[str] = field(default_factory=list)

    def merge(self, other: "Metrics") -> "Metrics":
        merged = Metrics(
            reference_chars=self.reference_chars + other.reference_chars,
            char_edits=self.char_edits + other.char_edits,
            reference_words=self.reference_words + other.reference_words,
            word_edits=self.word_edits + other.word_edits,
            exact_words=self.exact_words + other.exact_words,
            diacritic_bearing_chars=self.diacritic_bearing_chars
            + other.diacritic_bearing_chars,
            diacritic_errors=self.diacritic_errors + other.diacritic_errors,
            script_confusions=self.script_confusions + other.script_confusions,
            nfc_violations=self.nfc_violations + other.nfc_violations,
            samples=self.samples + other.samples,
            exact_lines=self.exact_lines + other.exact_lines,
            reference_lines=self.reference_lines + other.reference_lines,
        )
        merged.diacritic_error_examples = (
            self.diacritic_error_examples + other.diacritic_error_examples
        )[:20]
        merged.script_confusion_examples = (
            self.script_confusion_examples + other.script_confusion_examples
        )[:20]
        return merged

    @property
    def cer(self) -> float:
        return self.char_edits / self.reference_chars if self.reference_chars else 0.0

    @property
    def wer(self) -> float:
        return self.word_edits / self.reference_words if self.reference_words else 0.0

    @property
    def exact_word_rate(self) -> float:
        return self.exact_words / self.reference_words if self.reference_words else 0.0

    @property
    def diacritic_error_rate(self) -> float:
        if not self.diacritic_bearing_chars:
            return 0.0
        return self.diacritic_errors / self.diacritic_bearing_chars

    @property
    def script_confusion_rate(self) -> float:
        return (
            self.script_confusions / self.reference_chars if self.reference_chars else 0.0
        )

    @property
    def line_exact_rate(self) -> float:
        return self.exact_lines / self.reference_lines if self.reference_lines else 0.0

    def as_dict(self) -> dict:
        return {
            "samples": self.samples,
            "reference_chars": self.reference_chars,
            "reference_words": self.reference_words,
            "cer": round(self.cer, 6),
            "wer": round(self.wer, 6),
            "exact_word_rate": round(self.exact_word_rate, 6),
            "diacritic_bearing_chars": self.diacritic_bearing_chars,
            "diacritic_error_rate": round(self.diacritic_error_rate, 6),
            "script_confusion_rate": round(self.script_confusion_rate, 6),
            "script_confusions": self.script_confusions,
            "nfc_violations": self.nfc_violations,
            "line_exact_rate": round(self.line_exact_rate, 6),
        }


def score(reference: str, hypothesis: str) -> Metrics:
    """Scores one recognized text against its ground truth.

    ``hypothesis`` is measured for NFC compliance *before* normalization, so a
    candidate that emits NFD is reported even though the accuracy numbers are
    computed on the normalized forms.
    """
    result = Metrics(samples=1)
    if not is_nfc(hypothesis):
        result.nfc_violations = 1

    reference_norm = _norm_ws(reference)
    hypothesis_norm = _norm_ws(hypothesis)

    reference_chars = list(reference_norm)
    hypothesis_chars = list(hypothesis_norm)
    result.reference_chars = len(reference_chars)
    result.char_edits = levenshtein(reference_chars, hypothesis_chars)

    reference_words = reference_norm.split()
    hypothesis_words = hypothesis_norm.split()
    result.reference_words = len(reference_words)
    result.word_edits = levenshtein(reference_words, hypothesis_words)
    for expected, actual in _align(reference_words, hypothesis_words):
        if expected is not None and expected == actual:
            result.exact_words += 1

    reference_lines = [line for line in _split_lines(reference) if line]
    hypothesis_lines = [line for line in _split_lines(hypothesis) if line]
    result.reference_lines = len(reference_lines)
    for expected, actual in _align(reference_lines, hypothesis_lines):
        if expected is not None and expected == actual:
            result.exact_lines += 1

    for expected, actual in _align(reference_chars, hypothesis_chars):
        if expected is None:
            continue
        expected_marks = _marks(expected)
        if expected_marks:
            result.diacritic_bearing_chars += 1
            actual_marks = _marks(actual) if actual is not None else frozenset()
            if actual_marks != expected_marks:
                result.diacritic_errors += 1
                if len(result.diacritic_error_examples) < 20:
                    result.diacritic_error_examples.append(
                        f"{expected!r}->{actual!r}"
                    )
        if actual is not None and actual != expected:
            if _is_script_confusion(expected, actual):
                result.script_confusions += 1
                if len(result.script_confusion_examples) < 20:
                    result.script_confusion_examples.append(
                        f"{expected!r}->{actual!r}"
                    )
    return result


def _split_lines(text: str) -> list[str]:
    return [_norm_ws(line) for line in nfc(text).splitlines()]


def _is_script_confusion(expected: str, actual: str) -> bool:
    """True when a Greek letter was replaced by its Latin lookalike (or back).

    Comparison is on the accent-stripped base letter, so ``ὀ`` -> ``o`` counts
    as a script confusion in addition to being a diacritic error.
    """
    expected_base = _base(expected)
    actual_base = _base(actual)
    if _GREEK_TO_LATIN.get(expected_base) == actual_base:
        return True
    if _GREEK_TO_LATIN.get(actual_base) == expected_base:
        return True
    return False


def aggregate(items: Iterable[Metrics]) -> Metrics:
    total = Metrics()
    for item in items:
        total = total.merge(item)
    return total
