"""Deterministic eligibility for one scholarly support, never text repair.

A lossless candidate still needs the final PDF round-trip gate. Canonical
representation or unresolved glyphs require an explicit decision; this module
does not remove controls, flatten formulas, fold superscripts, or guess text.
"""
from dataclasses import asdict, dataclass
import re
import unicodedata

POLICY = 'scholarly-transcription-fidelity/1'
_LATEX_COMMANDS = ('sqrt', 'frac', 'dfrac', 'tfrac', 'text', 'textrm', 'textbf', 'textit', 'mathrm', 'mathbf', 'mathit', 'mathbb', 'mathcal', 'operatorname', 'begin', 'end', 'left', 'right', 'overline', 'underline', 'overset', 'underset', 'superscript', 'subscript', 'alpha', 'beta', 'gamma', 'delta', 'theta', 'lambda', 'mu', 'pi', 'sigma', 'phi', 'omega', 'sum', 'prod', 'int', 'infty', 'cdot', 'times', 'leq', 'geq')
_LATEX = re.compile(r"\\(?:" + "|".join(_LATEX_COMMANDS) + r")(?![A-Za-z])")
_RANK = {'LOSSLESS_CANDIDATE': 0, 'NEEDS_REVIEW': 1,
         'CANONICAL_REQUIRED': 2, 'INVALID': 3}


@dataclass(frozen=True)
class Assessment:
    state: str
    reasons: tuple[str, ...]
    policy: str = POLICY

    def to_dict(self):
        return asdict(self)


class TranscriptionFidelityError(ValueError):
    def __init__(self, assessment):
        self.assessment = assessment
        # Never include arbitrary provider text in exception/log messages.
        super().__init__(f'{assessment.state}: {",".join(assessment.reasons)}')


def assess(text):
    findings = []

    def add(state, reason):
        if (state, reason) not in findings:
            findings.append((state, reason))

    if not isinstance(text, str):
        return Assessment('INVALID', ('invalid_text_type',))
    if not text or not text.strip():
        add('INVALID', 'empty_or_whitespace')
    for char in text:
        cp = ord(char)
        if cp == 0:
            add('INVALID', 'nul_character')
        elif cp in (10, 13, 0x2028, 0x2029):
            add('CANONICAL_REQUIRED', 'line_break_requires_representation')
        elif cp < 32 or 0x7f <= cp <= 0x9f:
            add('INVALID', 'control_character')
        if 0xd800 <= cp <= 0xdfff:
            add('INVALID', 'invalid_unicode_scalar')
        if 0xfdd0 <= cp <= 0xfdef or cp & 0xffff in (0xfffe, 0xffff):
            add('INVALID', 'unicode_noncharacter')
        if cp == 0xfffd:
            add('NEEDS_REVIEW', 'replacement_character')
    if unicodedata.normalize('NFC', text) != text:
        add('CANONICAL_REQUIRED', 'non_nfc')
    first = next((c for c in text if not c.isspace()), '')
    if first and unicodedata.category(first).startswith('M'):
        add('NEEDS_REVIEW', 'leading_isolated_combining_mark')
    if _LATEX.search(text):
        add('CANONICAL_REQUIRED', 'latex_markup')
    state = max((s for s, _ in findings), key=_RANK.get, default='LOSSLESS_CANDIDATE')
    return Assessment(state, tuple(reason for _, reason in findings))


def require_lossless_candidate(text):
    assessment = assess(text)
    if assessment.state != 'LOSSLESS_CANDIDATE':
        raise TranscriptionFidelityError(assessment)
    return assessment


def explicit_nfc(text):
    """Return an auditable canonical equivalent, never an inferred correction.

    Callers must persist both raw text and the returned change record. The
    normalized value must still pass require_lossless_candidate before use.
    """
    if not isinstance(text, str) or any(0xd800 <= ord(c) <= 0xdfff for c in text):
        raise TranscriptionFidelityError(assess(text))
    canonical = unicodedata.normalize('NFC', text)
    assert unicodedata.normalize('NFD', text) == unicodedata.normalize('NFD', canonical)
    return canonical, None if text == canonical else {
        'operation': 'unicode_canonical_composition_NFC',
        'raw': text, 'nfc': canonical,
    }
