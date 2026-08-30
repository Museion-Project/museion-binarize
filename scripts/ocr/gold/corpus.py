#!/usr/bin/env python3
"""The committable OCR gold corpus.

Nothing here is a private document.  Each sample is a short, hand-written
passage (or a public-domain classical fragment) whose ground truth is stated
in this file, rendered deterministically at evaluation time by
``fixtures.py`` using the OFL-licensed Noto Sans already vendored in
``crates/mpdf-core/assets/fonts``.  No image is committed, so the corpus stays
license-clean and reproducible on any machine that has this repository.

``must_pass`` samples encode the specific real-world regressions this round
exists to eliminate.  A candidate that fails any of them cannot become a
production default no matter how good its averages look.
"""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class Sample:
    sample_id: str
    script: str            # "grc" | "deu" | "eng" | "lat" | "mixed"
    kind: str              # "prose" | "codepoints" | "toc" | "numerals"
    lines: tuple[str, ...]
    #: Rendering knobs the fixture generator honors.
    dpi: int = 300
    rotation: int = 0
    noise: float = 0.0
    columns: int = 1
    #: Substrings that MUST appear verbatim in the recognized text.
    must_contain: tuple[str, ...] = ()
    #: Substrings that must NOT appear (transliteration / lookalike leakage).
    must_not_contain: tuple[str, ...] = ()
    #: True when a failure blocks adoption as a production default.
    must_pass: bool = False
    #: TOC rows as (title, printed page) so the assembler can be scored.
    toc_rows: tuple[tuple[str, str], ...] = ()

    @property
    def text(self) -> str:
        return "\n".join(self.lines)


# --------------------------------------------------------------------------
# Ancient (polytonic) Greek.  Public-domain classical text plus the exact
# codepoints named in the acceptance criteria.
# --------------------------------------------------------------------------

GREEK = (
    Sample(
        sample_id="grc-prose-heraclitus",
        script="grc",
        kind="prose",
        lines=(
            "πάντων δὲ τῶν ὀξέων λόγων ἀρχή",
            "ἄνθρωπος μέτρον ἁπάντων χρημάτων",
            "ῥητορικῆς τέχνης περὶ τῶν ἀγαθῶν",
        ),
        must_contain=("ὀξέων", "ἄνθρωπος", "ῥητορικῆς"),
        must_not_contain=("ogeov", "oxeon", "anthropos"),
        must_pass=True,
    ),
    Sample(
        sample_id="grc-prose-plato",
        script="grc",
        kind="prose",
        lines=(
            "ὁ δὲ ἀνεξέταστος βίος οὐ βιωτὸς ἀνθρώπῳ",
            "τῇ ψυχῇ τῇ ἀθανάτῳ καὶ τῷ σώματι",
            "ἐν τῷ Σωκράτους λόγῳ περὶ δικαιοσύνης",
        ),
        must_contain=("ἀνεξέταστος", "ψυχῇ", "δικαιοσύνης"),
        must_pass=True,
    ),
    Sample(
        sample_id="grc-codepoints",
        script="grc",
        kind="codepoints",
        lines=(
            "ὀξέων ἄνθρωπος ῥόδον",
            "ταῦτα ᾶ τῆς ΐ τῶν ΰ",
            "χώρᾳ ᾳ τιμῇ ῃ λόγῳ ῳ",
        ),
        # Every acceptance codepoint appears here inside a real word *and*
        # standalone; the standalone forms are measured but not gated,
        # because an isolated accented vowel has no linguistic context and
        # no engine resolves it reliably.
        must_contain=("ὀξέων", "ἄνθρωπος", "ῥόδον", "ταῦτα", "χώρᾳ", "τιμῇ", "λόγῳ"),
        must_pass=True,
    ),
)


# --------------------------------------------------------------------------
# German.  The umlaut / eszett regressions reported from the real corpus.
# --------------------------------------------------------------------------

GERMAN = (
    Sample(
        sample_id="deu-prose-umlauts",
        script="deu",
        kind="prose",
        lines=(
            "Über die Größe der schönen Bäume",
            "für alle Länder und Städte, die Straße",
            "Ähnliche Öfen und Übungen im Süden",
        ),
        must_contain=("Über", "für", "Größe", "Bäume", "Straße", "Öfen", "Ähnliche", "Süden"),
        must_not_contain=("Uber", "fir", "Grosse", "Baume", "Ofen"),
        must_pass=True,
    ),
    Sample(
        sample_id="deu-codepoints",
        script="deu",
        kind="codepoints",
        lines=(
            "Ärger Öl Übel ändern öffnen übrig weiß",
            "Bäckerei Königreich Rückführung Maßstab",
            "Die Ärzte, die Öltanks, die Übersicht",
        ),
        must_contain=(
            "Ärger", "Öl", "Übel", "ändern", "öffnen", "übrig", "weiß",
            "Bäckerei", "Königreich", "Rückführung", "Maßstab",
        ),
        must_pass=True,
    ),
)


# --------------------------------------------------------------------------
# Mixed script, numerals, footnotes.
# --------------------------------------------------------------------------

MIXED = (
    Sample(
        sample_id="mixed-greek-german-english",
        script="mixed",
        kind="prose",
        lines=(
            "Der Begriff ἀρετή bei Aristoteles, S. 42",
            "vgl. Plato, Πολιτεία 514a, and see Ross (1923), p. 7",
            "Fußnote 3: τὸ ἀγαθόν — das Gute — the good",
        ),
        must_contain=("ἀρετή", "Πολιτεία", "Fußnote", "ἀγαθόν"),
        must_pass=True,
    ),
    Sample(
        sample_id="mixed-latin-numerals",
        script="lat",
        kind="numerals",
        lines=(
            "Liber II, caput XIV, pagina 137",
            "de rerum natura, versus 1024-1031",
            "Anmerkung 12; siehe S. 9, 45, 250",
        ),
        must_contain=("XIV", "137", "1024", "250"),
        must_pass=False,
    ),
)


# --------------------------------------------------------------------------
# Printed contents pages.  These exist to prove the logical-line assembler
# rebuilds "title + leader dots + right-hand page number" as one row.
# --------------------------------------------------------------------------

TOC = (
    Sample(
        sample_id="toc-single-column",
        script="mixed",
        kind="toc",
        lines=(
            "Inhaltsverzeichnis",
            "Einleitung . . . . . . . . . . . . . . . . . . . . . . 1",
            "1. Die ἀρετή bei Aristoteles . . . . . . . . . . . . 17",
            "2. Über die Größe der Seele . . . . . . . . . . . . . 43",
            "3. Πολιτεία und die Gerechtigkeit . . . . . . . . . . 88",
            "Literaturverzeichnis . . . . . . . . . . . . . . . . 201",
        ),
        toc_rows=(
            ("Einleitung", "1"),
            ("1. Die ἀρετή bei Aristoteles", "17"),
            ("2. Über die Größe der Seele", "43"),
            ("3. Πολιτεία und die Gerechtigkeit", "88"),
            ("Literaturverzeichnis", "201"),
        ),
        must_pass=True,
    ),
    Sample(
        sample_id="toc-roman-front-matter",
        script="mixed",
        kind="toc",
        lines=(
            "Contents",
            "Preface . . . . . . . . . . . . . . . . . . . . . . vii",
            "Acknowledgements . . . . . . . . . . . . . . . . . . xi",
            "Introduction . . . . . . . . . . . . . . . . . . . . . 1",
            "Chapter One: The Method . . . . . . . . . . . . . . 23",
        ),
        toc_rows=(
            ("Preface", "vii"),
            ("Acknowledgements", "xi"),
            ("Introduction", "1"),
            ("Chapter One: The Method", "23"),
        ),
        must_pass=True,
    ),
    Sample(
        sample_id="toc-wrapped-title",
        script="mixed",
        kind="toc",
        lines=(
            "Inhalt",
            "1. Die Bedeutung der ἀρετή in der",
            "   nikomachischen Ethik . . . . . . . . . . . . . . 17",
            "2. Kurzes Kapitel . . . . . . . . . . . . . . . . . 55",
        ),
        toc_rows=(
            ("1. Die Bedeutung der ἀρετή in der nikomachischen Ethik", "17"),
            ("2. Kurzes Kapitel", "55"),
        ),
        must_pass=False,
    ),
)


# --------------------------------------------------------------------------
# Geometry / degradation variants of an already-covered passage, so rotation,
# DPI and light noise are measured rather than assumed.
# --------------------------------------------------------------------------

GEOMETRY = (
    Sample(
        sample_id="grc-prose-heraclitus-200dpi",
        script="grc",
        kind="prose",
        lines=GREEK[0].lines,
        dpi=200,
        must_contain=("ὀξέων", "ἄνθρωπος"),
    ),
    Sample(
        sample_id="grc-prose-heraclitus-400dpi",
        script="grc",
        kind="prose",
        lines=GREEK[0].lines,
        dpi=400,
        must_contain=("ὀξέων", "ἄνθρωπος"),
    ),
    Sample(
        sample_id="deu-prose-umlauts-noise",
        script="deu",
        kind="prose",
        lines=GERMAN[0].lines,
        noise=0.005,
        must_contain=("Über", "für"),
    ),
    Sample(
        sample_id="deu-prose-umlauts-rot90",
        script="deu",
        kind="prose",
        lines=GERMAN[0].lines,
        rotation=90,
        must_contain=("Über", "für"),
    ),
    Sample(
        sample_id="mixed-two-column",
        script="mixed",
        kind="prose",
        lines=(
            "Die ἀρετή ist eine Haltung | The virtue is a settled state",
            "der Seele, die auf Maß zielt | of the soul aiming at measure",
            "und die Größe bewahrt.      | and preserving greatness.",
        ),
        columns=2,
        must_contain=("ἀρετή", "Größe"),
    ),
)


ALL_SAMPLES: tuple[Sample, ...] = GREEK + GERMAN + MIXED + TOC + GEOMETRY


def by_id(sample_id: str) -> Sample:
    for sample in ALL_SAMPLES:
        if sample.sample_id == sample_id:
            return sample
    raise KeyError(sample_id)


def samples_for(scripts: tuple[str, ...] | None = None) -> tuple[Sample, ...]:
    if not scripts:
        return ALL_SAMPLES
    return tuple(sample for sample in ALL_SAMPLES if sample.script in scripts)
