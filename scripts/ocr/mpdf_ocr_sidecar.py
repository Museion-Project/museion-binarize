#!/usr/bin/env python3
"""Offline OCR provider sidecar for ``mpdf`` (protocol ``mpdf-ocr`` 0.1).

Design contract
---------------
* **Original pages only.**  The Rust runner hands this process a raster of the
  *authoritative colour/grayscale page*.  Nothing here ever sees a binarized
  derivative, and nothing here decides what gets binarized.
* **No network, ever.**  Models are provisioned out of band and located
  through ``--model-dir``.  This script never downloads, installs, or probes a
  remote host.  A missing model is a clean ``78`` exit, not a fetch.
* **No dictionary repair.**  Recognized text is emitted exactly as the engine
  produced it, normalized to NFC and nothing else.  Combining marks,
  breathings, accents, diaereses, and iota subscripts are never stripped, and
  no word list, language model, or LLM is consulted to "fix" a result.
* **No text in diagnostics.**  Failures print a fixed string to stderr.  Page
  content never reaches a log.
* **Script honesty.**  A Greek profile recognizes with a Greek-capable model
  and returns Greek Unicode.  There is no silent transliteration or Latin
  lookalike fallback: if the configured profile cannot handle the page, the
  page fails instead of returning plausible-looking Latin.

Structure
---------
Detection/segmentation and recognition are kept distinct.  The response is a
real block/line/word tree with per-word boxes and confidences — one detector
rectangle is *not* published as a block, a line, and a word at once, which is
what made printed-contents rows lose their page numbers.

Invocation is argv-only; the per-page request arrives as a single JSON line on
stdin and the single JSON response is written to stdout.
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unicodedata
from pathlib import Path

PROTOCOL = "mpdf-ocr"
PROTOCOL_VERSION = "0.1"

EXIT_OK = 0
EXIT_UNAVAILABLE = 78
EXIT_FAILED = 79

#: Hard ceiling on the recognition subprocess, mirroring the Rust runner's own
#: timeout so a wedged engine cannot outlive its parent's patience.
ENGINE_TIMEOUT_SECONDS = 110
#: Ceiling on engine stdout, so a malformed model cannot exhaust memory.
MAX_ENGINE_OUTPUT_BYTES = 24 * 1024 * 1024

# ---------------------------------------------------------------------------
# Block-level script routing
# ---------------------------------------------------------------------------
#
# Why this exists.  A page of a Greek edition is not "a mixed page" in any
# useful sense: it is a Greek text block, a German running head, and a Latin
# apparatus, each internally monoscript.  Recognizing all of it in one
# ``grc+deu+eng`` pass lets the Latin models win inside Greek words, which is
# what turns ``ἐκεῖνο`` into ``EXELVO`` and ``νοῦς`` into ``vole``.  Measured
# on four real Greek-dense pages, the combined pass produced 138 Greek->Latin
# confusions where a Greek-only pass produced 5.
#
# The reverse is equally true, which is why a Greek-only pass is not the
# answer either: it renders the running head ``I. Metaphysische Kausalität``
# as ``Ι. Μειαρῃγϑιβοῆε Καιβα τᾶῖ``.  So the combined pass stays the baseline
# and the source of layout, and only blocks that are *unambiguously* Greek are
# re-recognized with the Greek model.
#
#: Routing algorithm identity, recorded in the response parameters.
ROUTING_MODE = "block-script-v1"
#: Threshold set identity.  Bump this whenever a threshold below changes, so
#: evidence produced under different calibrations is never silently compared.
ROUTING_THRESHOLDS_VERSION = "2"
#: Share of a region's *letters* that must be Greek before it is
#: re-recognized.
#:
#: Swept on the CGPG dev split and on real Greek and German/mixed pages.  The
#: curve is not monotonic in either direction, which is the whole difficulty:
#: lowering it catches more corrupted Greek but starts rerouting the mixed
#: footnote lines where a classical edition puts German or Latin *inside* a
#: Greek quotation, and the Greek-only model cannot emit those scripts at all.
#: 0.80 was chosen because it is the setting that met the script-confusion
#: rate target on real pages; 0.90 gave a better aggregate CER but missed it.
#: See docs/ocr-engines.md for the full ablation.
ROUTING_GREEK_RATIO = 0.80
#: A region must also carry this many Greek letters outright.  Ratio alone
#: would reroute a two-word caption on one lucky character.
ROUTING_MIN_GREEK_CHARS = 12
#: A region carrying more Latin letters than this is never rerouted, however
#: Greek-dominant it looks by ratio.
#:
#: The Greek-only model cannot emit Latin at all, so every Latin letter inside
#: a rerouted region is destroyed.  A classical footnote is the case that
#: matters: "... παραπλησίως τοῖς Πυθαγορείοις ἔλεγε. Vgl. dazu oben Punkt
#: 1.3.1." is comfortably Greek by ratio, and rerouting it costs the German.
#:
#: The value is a measured compromise, not a principle.  Tightening it to 4 or
#: 20 protected the German but blocked the long Greek body regions too, which
#: legitimately carry 25-30 Latin characters of footnote markers and
#: references: at 20 the script-confusion count on real Greek pages only fell
#: from 138 to 79, against 20 at this setting.
ROUTING_MAX_LATIN_CHARS = 40
#: A line carrying at least this many Latin letters is treated as genuinely
#: mixed and keeps its combined reading even when the surrounding region is
#: rerouted.  This is what makes a German running head, a Latin apparatus
#: line, a reference or a page number survive inside a Greek column.
#:
#: It must stay well below ROUTING_MAX_LATIN_CHARS, or the run-level retention
#: check below can never fire: a run is only routable when it has at most
#: ROUTING_MAX_LATIN_CHARS Latin letters, so a retention floor at or above
#: that ceiling is unreachable by construction.
ROUTING_SUBSTANTIAL_LATIN_CHARS = 8
#: How confidently the combined pass must have read a Latin-bearing line
#: before that line is judged genuinely Latin and kept.
#:
#: Latin letter count alone cannot make this call, and that is the crux of the
#: whole problem: in a *corrupted* Greek line the "Latin" letters are the
#: corruption (``ἐκεῖνο`` read as ``EXELVO``), so preserving every line that
#: contains Latin preserves the very defect the router exists to remove --
#: measured, that put Greek confusions back up from 31 to 106.  Confidence
#: separates the two cases, and the same signal works at line level as at
#: region level: the combined model is confident on real German and unsure
#: when it is misreading Greek as Latin.
ROUTING_MIXED_LINE_MIN_CONFIDENCE = 0.90
#: Safety net behind the line-level guard: a candidate that dropped most of
#: the Latin the combined pass had found is rejected even if it slipped
#: through.  Reachable because it triggers from
#: ROUTING_SUBSTANTIAL_LATIN_CHARS, not from the routing ceiling.
ROUTING_MIN_LATIN_RETENTION = 0.50
#: Per-page ceiling on re-recognition subprocesses.  Exceeding it is not an
#: error: the remaining regions deterministically keep their combined result.
ROUTING_MAX_SEGMENTS = 12
#: Horizontal padding around a region crop, so a glyph flush against the
#: column edge is not clipped away.
ROUTING_CROP_PADDING = 12
#: Vertical padding.  Greek accents sit high and clear of the letter body, so
#: the crop must reach above the measured line box — but every pixel of this
#: also exposes the neighbouring line to the Greek pass, which is why the
#: candidate is afterwards restricted to the region's own vertical span.
ROUTING_CROP_PADDING_Y = 10
#: How far outside the region's vertical span a candidate line's centre may
#: fall and still be kept, as a share of the region's median line height.
#: Without this, the crop's padding reliably imports the line above and the
#: line below, and splicing those in duplicates text that the combined pass
#: still holds — measured as a CER regression from 0.064 to 0.152.
ROUTING_LINE_CENTRE_TOLERANCE = 0.30
#: A region smaller than this in either axis is not worth a subprocess.
ROUTING_MIN_CROP_PX = 24
#: Wall-clock budget for all re-recognition on one page, leaving the bulk of
#: ENGINE_TIMEOUT_SECONDS to the combined pass that must always complete.
ROUTING_TIME_BUDGET_SECONDS = 45
#: A replacement must keep the region's character count inside this band.  Both
#: ends matter: collapse means the crop lost text, expansion means the crop
#: picked up noise.
ROUTING_MIN_COVERAGE = 0.60
ROUTING_MAX_COVERAGE = 1.60
#: A replacement must retain this share of the Greek letters the combined pass
#: already found.  A "Greek" candidate that dropped the Greek is a regression
#: however good its confidence looks.
ROUTING_MIN_GREEK_RETENTION = 0.80
#: Floor on a replacement's mean word confidence.
ROUTING_MIN_CONFIDENCE = 0.30
# --- small-type Latin regions (footnotes, apparatus, bibliographies) -------
#
# German umlauts survive 11 pt body text perfectly and fail in 8 pt notes: on
# a real scanned page the body scored 0 wrong of 24 umlaut/eszett characters
# while the footnote scored 5 of 16, with the two diaeresis dots merging into
# the letter body (``diesbezüglichen`` -> ``diesbeziiglichen``).
#
# The fix is not resolution. Re-recognizing the footnote *as its own region*
# corrects it at every scale tested, including 1.0x -- the errors come from
# the note competing with body text for one page-wide set of layout and scale
# assumptions, not from missing pixels.
#
#: Share of the page's median line pitch below which a run counts as small
#: type. Pitch, not glyph height: measured on a real page, body and footnote
#: word heights overlap (0.97-1.03 vs 0.86-1.03 of the median) while their
#: line pitches separate cleanly (about 53 px against 44 px).
ROUTING_SMALL_TYPE_PITCH_RATIO = 0.92
#: A small-type run must be this many lines before it is worth a subprocess.
ROUTING_SMALL_TYPE_MIN_LINES = 3
#: ... and carry this many Latin letters.
ROUTING_SMALL_TYPE_MIN_LATIN = 40
#: Upscale applied to a small-type crop. 1.0 measured as well as 2.0 here, so
#: this stays modest; it exists to give the engine a little more to work with
#: on a genuinely small glyph, not to paper over the region isolation that is
#: doing the real work.
ROUTING_SMALL_TYPE_SCALE = 2.0

# --- detached Greek accent bands -------------------------------------------
#
# In Teubner-style Greek the breathings and accents sit high and clear of the
# letter body. Tesseract's line finder treats that band as a text line of its
# own and emits it as marks: `\ »" / 3 \ 3 \ »" 3 \ RA 3 / N /`. Measured on
# four real pages, 16 such lines carried 394 characters of pure noise into the
# text layer.
#
# Two different remedies, and the difference matters when reporting results:
# deleting the band removes the noise but recovers nothing (diacritic errors
# stay at 81/1331); re-recognizing the band *together with* its base line as a
# single line recovers real diacritics (81 -> 61) and Greek that had been read
# as Latin (137 -> 87 confusions). This pass does the second, and falls back
# to keeping the combined reading -- never to silent deletion -- whenever the
# candidate fails a gate.
#
# Raising page DPI is not the remedy: 400 -> 600 dpi measured worse, because
# the accents separate further.
#
#: A band this much shorter than the page's median line is a candidate.
DETACHED_MAX_HEIGHT_RATIO = 0.55
#: ... and the engine must be unsure about part of it.
#:
#: This is the *minimum* word confidence, not the mean. A band of stray marks
#: has a high mean -- Tesseract is quite sure a backslash is a backslash --
#: while always containing something it cannot read at all. Measured on real
#: bands: mean 0.66-0.81, minimum 0.10-0.35.
DETACHED_MAX_MIN_CONFIDENCE = 0.55
#: A band carrying this many Greek letters is real text, not marks.
DETACHED_MAX_GREEK_CHARS = 4
#: The base line must be at least this tall relative to the page median.
DETACHED_MIN_BASE_HEIGHT_RATIO = 0.80
#: Share of the band's width that must overlap the base line horizontally.
DETACHED_MIN_HORIZONTAL_OVERLAP = 0.50
#: Page-segmentation mode for the union crop. 13 ("raw line") measured best:
#: 7 was close, 6 was far worse because it re-ran layout analysis on a crop
#: that is by construction a single line.
DETACHED_CROP_PSM = 13
#: Native resolution. 1.5x and 2.0x both measured slightly worse.
DETACHED_CROP_SCALE = 1.0
DETACHED_CROP_PADDING_X = 10
DETACHED_CROP_PADDING_Y = 6
#: Coverage band for a replacement, measured on the base line's own text.
DETACHED_MIN_COVERAGE = 0.60
DETACHED_MAX_COVERAGE = 1.80
#: Floor on a replacement's mean word confidence.
DETACHED_MIN_CONFIDENCE = 0.20

#: A region the combined pass read *this* confidently is left alone.
#:
#: This is the single most useful signal in the whole router, and it was found
#: by measurement rather than assumed.  The regions where rerouting made a
#: page worse were exactly the ones the combined pass had already read with
#: mean confidence 0.95-0.96 -- Greek quotations inside a German page, which
#: it gets right.  The regions where rerouting helped were read at 0.72-0.92:
#: the combined pass was *unsure*, and being unsure is what precedes deciding
#: that ``ἐκεῖνο`` is ``EXELVO``.  Reroute where the engine shows doubt; leave
#: alone where it does not.
ROUTING_MAX_COMBINED_CONFIDENCE = 0.90

# ---------------------------------------------------------------------------
# Language profiles
# ---------------------------------------------------------------------------
#
# A profile names the scripts a page may contain, not a guess about what it
# does contain.  "auto" is this project's actual corpus (classical Greek plus
# German plus English), recognized by one combined model pass so a mixed line
# does not have to be split before it can be read.  "Greek" is *not* a synonym
# for polytonic Ancient Greek: ``greek-modern`` and ``greek-ancient`` are
# separate profiles because they need different models.

PROFILES: dict[str, dict] = {
    "auto": {
        "tesseract": "grc+deu+eng",
        "scripts": ["Greek", "Latin"],
        "description": "ancient Greek + German + English, single combined pass",
    },
    "greek-ancient": {
        "tesseract": "grc",
        "scripts": ["Greek"],
        "description": "polytonic Ancient Greek only",
    },
    "greek-ancient-german-english": {
        "tesseract": "grc+deu+eng",
        "scripts": ["Greek", "Latin"],
        "description": "polytonic Ancient Greek + German + English",
    },
    "greek-modern": {
        "tesseract": "ell",
        "scripts": ["Greek"],
        "description": "monotonic Modern Greek only",
    },
    "german": {
        "tesseract": "deu",
        "scripts": ["Latin"],
        "description": "German only",
    },
    "german-english": {
        "tesseract": "deu+eng",
        "scripts": ["Latin"],
        "description": "German + English",
    },
    "english": {
        "tesseract": "eng",
        "scripts": ["Latin"],
        "description": "English only",
    },
    "latin-german-english": {
        "tesseract": "lat+deu+eng",
        "scripts": ["Latin"],
        "description": "Latin + German + English",
    },
}

DEFAULT_PROFILE = "auto"

#: Unicode blocks a profile is allowed to emit.  Used only to *report* a
#: script violation, never to rewrite text.
_SCRIPT_RANGES = {
    "Greek": ((0x0370, 0x03FF), (0x1F00, 0x1FFF)),
    "Latin": ((0x0041, 0x024F), (0x1E00, 0x1EFF)),
}


def _profile(name: str) -> dict:
    if name not in PROFILES:
        raise ValueError(f"unknown language profile: {name}")
    return PROFILES[name]


# ---------------------------------------------------------------------------
# Model manifest
# ---------------------------------------------------------------------------


def _load_manifest(model_dir: Path) -> dict:
    """Reads the sidecar model manifest that must accompany every model set.

    The manifest pins engine, model file names, sizes, SHA-256 digests and
    licenses.  It is required: a model directory without one is treated as
    unprovisioned rather than trusted.
    """
    path = model_dir / "manifest.json"
    if not path.is_file():
        raise FileNotFoundError("model manifest is missing")
    if path.stat().st_size > 1024 * 1024:
        raise ValueError("model manifest is too large")
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if manifest.get("schema") != "mpdf-ocr-models" or manifest.get(
        "schema_version"
    ) not in ("1.0",):
        raise ValueError("unsupported model manifest schema")
    return manifest


def _verify_models(model_dir: Path, manifest: dict, engine: str, langs: list[str]) -> dict:
    """Verifies every model file this run will actually use.

    Returns the provenance dict recorded on the page: engine, model set name,
    version, per-file SHA-256, and license.  A digest mismatch is fatal — a
    silently swapped model would invalidate every downstream checkpoint.
    """
    entries = {entry["language"]: entry for entry in manifest.get("models", [])
               if entry.get("engine") == engine}
    used = []
    for language in langs:
        entry = entries.get(language)
        if entry is None:
            raise FileNotFoundError(f"model for language {language!r} is not in the manifest")
        file_path = model_dir / entry["filename"]
        if not file_path.is_file():
            raise FileNotFoundError(f"model file is missing: {entry['filename']}")
        digest = hashlib.sha256(file_path.read_bytes()).hexdigest()
        if digest != entry["sha256"]:
            raise ValueError(f"model digest mismatch for {entry['filename']}")
        used.append(
            {
                "language": language,
                "filename": entry["filename"],
                "sha256": digest,
                "license": entry["license"],
                "source": entry.get("source", ""),
                "model_version": entry.get("model_version", ""),
            }
        )
    licenses = sorted({item["license"] for item in used})
    return {
        "model_set": manifest.get("model_set", ""),
        "model_set_version": manifest.get("model_set_version", ""),
        "files": used,
        "license": " AND ".join(licenses),
    }


# ---------------------------------------------------------------------------
# Text normalization
# ---------------------------------------------------------------------------


def normalize(text: str) -> str:
    """NFC only.  No mark stripping, no case folding, no dictionary repair."""
    return unicodedata.normalize("NFC", text)


def script_violations(text: str, allowed: list[str]) -> int:
    """Counts letters outside the profile's declared scripts.

    Reported in the response so the caller can route a page to review; never
    used to alter the text.
    """
    ranges = [span for name in allowed for span in _SCRIPT_RANGES.get(name, ())]
    if not ranges:
        return 0
    violations = 0
    for character in text:
        if not character.isalpha():
            continue
        code = ord(character)
        if code < 0x0370 and code < 0x0041:
            continue
        if not any(low <= code <= high for low, high in ranges):
            violations += 1
    return violations


# ---------------------------------------------------------------------------
# Tesseract engine (LSTM only)
# ---------------------------------------------------------------------------

_TSV_FIELDS = (
    "level page_num block_num par_num line_num word_num "
    "left top width height conf text"
).split()


def _tesseract_env(model_dir: Path) -> dict:
    environment = dict(os.environ)
    # TESSDATA_PREFIX must point at the directory that *contains* the
    # .traineddata files for Tesseract 4/5.
    environment["TESSDATA_PREFIX"] = str(model_dir)
    # Single-threaded so a page costs a predictable amount of CPU and results
    # do not vary with machine load.
    environment["OMP_THREAD_LIMIT"] = "1"
    return environment


_OSD_ROTATE = re.compile(r"^Rotate:\s*(\d+)", re.MULTILINE)
_OSD_CONFIDENCE = re.compile(r"^Orientation confidence:\s*([0-9.]+)", re.MULTILINE)

#: Below this, the orientation estimate is treated as no information at all
#: and the page is recognized as it arrived.
MIN_ORIENTATION_CONFIDENCE = 1.0


def detect_orientation(binary: str, image: Path, model_dir: Path) -> tuple[int, float]:
    """Returns (clockwise degrees to upright, confidence).

    Segmentation and recognition are separate passes on purpose. Letting
    ``--psm 1`` fold orientation detection into recognition costs real
    content: on a printed contents page it drops the right-flush page number
    off the end of every row, which is the whole reason those rows exist.
    Detecting orientation explicitly lets recognition run in the mode that
    reads a text block properly.
    """
    try:
        completed = subprocess.run(
            [binary, str(image), "stdout", "--psm", "0"],
            capture_output=True,
            timeout=ENGINE_TIMEOUT_SECONDS,
            env=_tesseract_env(model_dir),
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return 0, 0.0
    if completed.returncode != 0:
        return 0, 0.0
    report = completed.stdout.decode("utf-8", errors="replace")
    rotate = _OSD_ROTATE.search(report)
    confidence = _OSD_CONFIDENCE.search(report)
    degrees = int(rotate.group(1)) % 360 if rotate else 0
    score = float(confidence.group(1)) if confidence else 0.0
    if degrees not in (0, 90, 180, 270) or score < MIN_ORIENTATION_CONFIDENCE:
        return 0, score
    return degrees, score


def unrotate_box(
    box: dict, clockwise: int, original_width: int, original_height: int
) -> dict:
    """Maps a box measured on the upright raster back to original coordinates.

    The protocol promises boxes in the coordinate space of the page image the
    runner supplied. Recognizing a rotated scan upright is an internal detail,
    so the inverse rotation is applied here rather than leaking a second
    coordinate space into the evidence.
    """
    x, y, width, height = box["x"], box["y"], box["width"], box["height"]
    if clockwise == 0:
        return dict(box)
    if clockwise == 90:
        # Original (x, y) -> upright (H - y, x); inverse of that, as a box.
        return {
            "x": y,
            "y": float(original_height) - (x + width),
            "width": height,
            "height": width,
        }
    if clockwise == 180:
        return {
            "x": float(original_width) - (x + width),
            "y": float(original_height) - (y + height),
            "width": width,
            "height": height,
        }
    # clockwise == 270
    return {
        "x": float(original_width) - (y + height),
        "y": x,
        "width": height,
        "height": width,
    }


def _run_tesseract(
    binary: str,
    image: Path,
    model_dir: Path,
    languages: str,
    psm: int,
    timeout: float | None = None,
) -> str:
    """Runs one recognition pass.

    ``timeout`` is explicit so a caller working inside a smaller budget than
    the engine ceiling can enforce it.  The routing budget is only a real
    budget if the subprocess it launches can actually be cut short by it.
    """
    completed = subprocess.run(
        [
            binary,
            str(image),
            "stdout",
            "-l",
            languages,
            "--psm",
            str(psm),
            # LSTM only: the legacy engine has no polytonic Greek model and
            # its presence would silently change results per tessdata build.
            "--oem",
            "1",
            "-c",
            "preserve_interword_spaces=1",
            # Requested as a parameter rather than the `tsv` config *file*, so
            # the model directory needs to hold nothing but the pinned
            # .traineddata files and their manifest.
            "-c",
            "tessedit_create_tsv=1",
        ],
        capture_output=True,
        timeout=ENGINE_TIMEOUT_SECONDS if timeout is None else timeout,
        env=_tesseract_env(model_dir),
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError("recognition engine failed")
    if len(completed.stdout) > MAX_ENGINE_OUTPUT_BYTES:
        raise RuntimeError("recognition engine output exceeds limit")
    return completed.stdout.decode("utf-8", errors="strict")


def _parse_tsv(tsv: str) -> list[dict]:
    """Turns Tesseract's TSV into a block -> line -> word tree.

    Tesseract already reports its own layout analysis (block, paragraph, line,
    word).  Preserving that hierarchy — instead of republishing each detected
    rectangle as a one-word line — is what keeps a printed-contents row
    ("title …… 137") a single line.

    Level-2 (block) rectangles are kept as ``bbox`` alongside the words.  The
    router crops from *that* rectangle rather than from the union of the words
    it managed to read: a block whose Greek was misread as Latin may have lost
    words entirely, so a word-derived crop would omit exactly the region the
    re-recognition pass exists to recover.
    """
    rows = tsv.splitlines()
    if not rows:
        return []
    header = rows[0].split("\t")
    if header != _TSV_FIELDS:
        raise RuntimeError("unexpected recognizer output format")
    blocks: dict[int, dict] = {}
    for row in rows[1:]:
        parts = row.split("\t")
        if len(parts) != len(_TSV_FIELDS):
            continue
        record = dict(zip(_TSV_FIELDS, parts))
        level = int(record["level"])
        if level == 2:
            try:
                block_num = int(record["block_num"])
            except ValueError:
                continue
            block = blocks.setdefault(block_num, {"lines": {}, "bbox": None})
            block["bbox"] = {
                "x": float(record["left"]),
                "y": float(record["top"]),
                "width": float(record["width"]),
                "height": float(record["height"]),
            }
            continue
        if level != 5:
            continue
        text = normalize(record["text"])
        if not text.strip():
            continue
        try:
            confidence = float(record["conf"])
        except ValueError:
            continue
        if confidence < 0:
            continue
        block_num = int(record["block_num"])
        line_key = (int(record["par_num"]), int(record["line_num"]))
        block = blocks.setdefault(block_num, {"lines": {}, "bbox": None})
        line = block["lines"].setdefault(line_key, {"words": []})
        line["words"].append(
            {
                "text": text,
                "left": int(record["left"]),
                "top": int(record["top"]),
                "width": int(record["width"]),
                "height": int(record["height"]),
                # Tesseract reports 0-100; the protocol is 0-1.
                "confidence": max(0.0, min(1.0, confidence / 100.0)),
                "word_num": int(record["word_num"]),
            }
        )
    ordered_blocks = []
    for block_num in sorted(blocks):
        lines = []
        for line_key in sorted(blocks[block_num]["lines"]):
            words = sorted(
                blocks[block_num]["lines"][line_key]["words"],
                key=lambda word: (word["left"], word["word_num"]),
            )
            if words:
                lines.append(words)
        if lines:
            ordered_blocks.append({"bbox": blocks[block_num]["bbox"], "lines": lines})
    return ordered_blocks


# ---------------------------------------------------------------------------
# Block-level script routing
# ---------------------------------------------------------------------------


def _text_of(lines: list[list[dict]]) -> str:
    return " ".join(word["text"] for line in lines for word in line)


def _script_counts(text: str) -> tuple[int, int, int]:
    """Returns (greek letters, latin letters, other letters)."""
    greek = latin = other = 0
    for character in text:
        if not character.isalpha():
            continue
        code = ord(unicodedata.normalize("NFD", character)[0])
        if (0x0370 <= code <= 0x03FF) or (0x1F00 <= code <= 0x1FFF):
            greek += 1
        elif (0x0041 <= code <= 0x024F) or (0x1E00 <= code <= 0x1EFF):
            latin += 1
        else:
            other += 1
    return greek, latin, other


def _line_leans_greek(line: list[dict]) -> bool:
    """Strict majority, used only to group adjacent lines into runs."""
    greek, latin, _ = _script_counts(_text_of([line]))
    return greek > latin


def _script_runs(lines: list[list[dict]]) -> list[tuple[bool, int, int]]:
    """Splits a block's lines into maximal runs of one leaning script.

    Routing operates on these runs rather than on whole Tesseract blocks.
    With ``--psm 6`` — which this sidecar uses because page-segmentation modes
    drop the right-flush page number off printed-contents rows — Tesseract
    reports the entire page as a single block, so "block routing" would in
    practice be *page* routing.  Measured, that destroyed the German running
    head (``I. Metaphysische Kausalität`` came back as Greek) even while it
    removed the Greek->Latin confusions.

    A run is still a region, not a line: one subprocess covers a whole
    contiguous stretch of Greek, so this never becomes the per-line dual-model
    recognition that would multiply cost by the number of lines on the page.
    """
    runs: list[tuple[bool, int, int]] = []
    for index, line in enumerate(lines):
        greek = _line_leans_greek(line)
        if runs and runs[-1][0] == greek:
            leaning, first, _ = runs[-1]
            runs[-1] = (leaning, first, index + 1)
        else:
            runs.append((greek, index, index + 1))
    return runs


def _is_greek_dominant(greek: int, latin: int, other: int) -> bool:
    """Conservative: only an unambiguously Greek run is rerouted.

    Everything else — Latin-dominant runs, genuinely mixed runs, short
    captions, page numbers, and headers whose script is unclear — keeps the
    combined result.  Rerouting those is how a router breaks German.
    """
    letters = greek + latin + other
    if letters == 0 or greek < ROUTING_MIN_GREEK_CHARS:
        return False
    if latin > ROUTING_MAX_LATIN_CHARS:
        return False
    return (greek / letters) >= ROUTING_GREEK_RATIO


def _run_bbox(lines: list[list[dict]], block_bbox: dict | None) -> dict:
    """Vertical extent from the run's own lines, horizontal from the block.

    The level-2 block rectangle is what gives the crop its full column width.
    Deriving the horizontal extent from recognized words instead would narrow
    the crop by exactly the words the combined pass failed to read — which are
    the words this pass exists to recover.
    """
    words = [word for line in lines for word in line]
    top = min(word["top"] for word in words)
    bottom = max(word["top"] + word["height"] for word in words)
    left = min(word["left"] for word in words)
    right = max(word["left"] + word["width"] for word in words)
    if block_bbox is not None:
        # Union, not substitution. The block rectangle is what gives the crop
        # its full column width where recognition dropped words at the line
        # ends; but it can also be *narrower* than the run -- a footnote set
        # wider than the body block clipped its own first and last words, and
        # with them ``Würdigung`` and ``Stählin``.
        left = min(left, block_bbox["x"])
        right = max(right, block_bbox["x"] + block_bbox["width"])
    return {
        "x": float(left),
        "y": float(top),
        "width": float(right - left),
        "height": float(bottom - top),
    }


def _crop_box(bbox: dict, page_width: int, page_height: int) -> tuple[int, int, int, int] | None:
    left = int(max(0, math.floor(bbox["x"]) - ROUTING_CROP_PADDING))
    top = int(max(0, math.floor(bbox["y"]) - ROUTING_CROP_PADDING_Y))
    right = int(min(page_width, math.ceil(bbox["x"] + bbox["width"]) + ROUTING_CROP_PADDING))
    bottom = int(min(page_height, math.ceil(bbox["y"] + bbox["height"]) + ROUTING_CROP_PADDING_Y))
    if right - left < ROUTING_MIN_CROP_PX or bottom - top < ROUTING_MIN_CROP_PX:
        return None
    return left, top, right, bottom


def _mean_confidence(lines: list[list[dict]]) -> float:
    values = [word["confidence"] for line in lines for word in line]
    return sum(values) / len(values) if values else 0.0


def _offset_lines(lines: list[list[dict]], dx: int, dy: int) -> list[list[dict]]:
    """Moves lines recognized on a crop back into upright page coordinates."""
    return [
        [dict(word, left=word["left"] + dx, top=word["top"] + dy) for word in line]
        for line in lines
    ]


def _line_centre(line: list[dict]) -> float:
    top = min(word["top"] for word in line)
    bottom = max(word["top"] + word["height"] for word in line)
    return (top + bottom) / 2.0


def _median_line_height(lines: list[list[dict]]) -> float:
    heights = sorted(
        max(word["top"] + word["height"] for word in line)
        - min(word["top"] for word in line)
        for line in lines
    )
    return float(heights[len(heights) // 2]) if heights else 0.0


def _restrict_to_span(
    candidate: list[list[dict]], segment: list[list[dict]]
) -> list[list[dict]]:
    """Drops candidate lines that belong to a neighbouring region.

    The crop is padded so tall Greek accents survive, which necessarily lets
    the Greek pass see part of the line above and below.  Anything whose centre
    lies outside the region's own vertical span is one of those neighbours: the
    combined pass still holds that line, so keeping it here would duplicate it.
    """
    words = [word for line in segment for word in line]
    top = min(word["top"] for word in words)
    bottom = max(word["top"] + word["height"] for word in words)
    tolerance = _median_line_height(segment) * ROUTING_LINE_CENTRE_TOLERANCE
    return [
        line
        for line in candidate
        if top - tolerance <= _line_centre(line) <= bottom + tolerance
    ]


def _line_band(line: list[dict]) -> tuple[float, float]:
    top = min(word["top"] for word in line)
    bottom = max(word["top"] + word["height"] for word in line)
    return float(top), float(bottom)


def _merge_by_geometry(
    segment: list[list[dict]], candidate: list[list[dict]]
) -> tuple[list[list[dict]], int]:
    """Replaces only the lines that are Greek; keeps genuinely mixed ones.

    Replacing a whole region wholesale is what put the German at risk: a Greek
    column routinely carries a running head, a reference, or an apparatus line
    inside it, and the Greek-only model cannot emit Latin at all, so those
    lines came back as Greek-shaped noise.

    Alignment is by vertical band, so this stays one subprocess per region --
    it is a decision about which of the already-recognized lines to keep, not
    a second recognition pass per line.

    Returns the merged lines and the number of original lines preserved
    because they carried substantial Latin.
    """
    merged: list[list[dict]] = []
    used: set[int] = set()
    preserved = 0
    for original in segment:
        top, bottom = _line_band(original)
        tolerance = (bottom - top) * 0.5
        matches = [
            index
            for index, line in enumerate(candidate)
            if index not in used
            and top - tolerance <= _line_centre(line) <= bottom + tolerance
        ]
        _, latin, _ = _script_counts(_text_of([original]))
        aligned = [candidate[index] for index in sorted(matches)]
        genuinely_latin = (
            latin >= ROUTING_SUBSTANTIAL_LATIN_CHARS
            and _mean_confidence([original]) >= ROUTING_MIXED_LINE_MIN_CONFIDENCE
        )
        if genuinely_latin or not matches:
            # Genuinely mixed, or nothing aligned to it: keep what the
            # combined pass read.
            merged.append(original)
            if genuinely_latin:
                preserved += 1
            continue
        used.update(matches)
        merged.extend(candidate[index] for index in sorted(matches))
    return merged, preserved


def _accept_replacement(
    original: list[list[dict]],
    candidate: list[list[dict]],
    crop: tuple[int, int, int, int],
) -> str | None:
    """Decides whether a Greek-only candidate may replace a combined run.

    Returns ``None`` to accept, or a short reason to reject.  Every rejection
    keeps the combined result, so a router failure degrades to exactly today's
    behaviour rather than to missing text.
    """
    if not candidate:
        return "empty"
    left, top, right, bottom = crop
    for line in candidate:
        for word in line:
            if (
                word["left"] < left - 1
                or word["top"] < top - 1
                or word["left"] + word["width"] > right + 1
                or word["top"] + word["height"] > bottom + 1
            ):
                return "bbox-outside-crop"
    before, after = _text_of(original), _text_of(candidate)
    letters_before = sum(1 for c in before if c.isalpha())
    letters_after = sum(1 for c in after if c.isalpha())
    if letters_before == 0:
        return "no-baseline-text"
    coverage = letters_after / letters_before
    if coverage < ROUTING_MIN_COVERAGE:
        return "coverage-collapsed"
    if coverage > ROUTING_MAX_COVERAGE:
        return "coverage-expanded"
    greek_before, latin_before, _ = _script_counts(before)
    greek_after, latin_after, _ = _script_counts(after)
    if greek_before and greek_after / greek_before < ROUTING_MIN_GREEK_RETENTION:
        return "greek-lost"
    if (
        latin_before >= ROUTING_SUBSTANTIAL_LATIN_CHARS
        and latin_after / latin_before < ROUTING_MIN_LATIN_RETENTION
    ):
        return "latin-lost"
    if _mean_confidence(candidate) < ROUTING_MIN_CONFIDENCE:
        return "low-confidence"
    if any(
        not unicodedata.is_normalized("NFC", word["text"])
        for line in candidate
        for word in line
    ):
        return "not-nfc"
    return None


def _line_pitches(lines: list[list[dict]]) -> list[float]:
    """Distance from each line's top to the next line's top."""
    tops = [min(word["top"] for word in line) for line in lines]
    return [float(tops[i + 1] - tops[i]) for i in range(len(tops) - 1)]


def _small_type_runs(lines: list[list[dict]], median_pitch: float) -> list[tuple[int, int]]:
    """Maximal runs of contiguous small-type, Latin-dominant lines.

    Detection is geometric and script-based only. It never consults which
    words are present, so it cannot be tuned towards a known failing token.
    """
    if median_pitch <= 0 or len(lines) < ROUTING_SMALL_TYPE_MIN_LINES + 1:
        return []
    pitches = _line_pitches(lines)
    threshold = median_pitch * ROUTING_SMALL_TYPE_PITCH_RATIO
    small = [pitch <= threshold for pitch in pitches]
    # The last line inherits its predecessor's pitch; it has none of its own.
    small.append(small[-1] if small else False)

    runs: list[tuple[int, int]] = []
    start: int | None = None
    for index, is_small in enumerate(small):
        greek, latin, _ = _script_counts(_text_of([lines[index]]))
        qualifies = is_small and latin > greek
        if qualifies and start is None:
            start = index
        elif not qualifies and start is not None:
            runs.append((start, index))
            start = None
    if start is not None:
        runs.append((start, len(lines)))
    return [
        (first, last)
        for first, last in runs
        if last - first >= ROUTING_SMALL_TYPE_MIN_LINES
        and _script_counts(_text_of(lines[first:last]))[1] >= ROUTING_SMALL_TYPE_MIN_LATIN
    ]


def _latin_languages(languages: str) -> str:
    """The Latin-script subset of a profile's language list, order preserved."""
    latin = [name for name in languages.split("+") if name in ("deu", "eng", "lat")]
    return "+".join(latin)


def route_small_latin_regions(
    binary: str,
    upright: Path,
    model_dir: Path,
    tree: list[dict],
    psm: int,
    page_width: int,
    page_height: int,
    languages: str,
    budget: "_Budget",
) -> tuple[list[dict], list[dict]]:
    """Re-recognizes small-type Latin regions on their own.

    Shares the Greek router's subprocess and time budget, and applies the same
    replacement safety checks, so a failure keeps the combined reading.
    """
    from PIL import Image  # noqa: PLC0415

    latin_langs = _latin_languages(languages)
    decisions: list[dict] = []
    routed_tree: list[dict] = []
    if not latin_langs:
        return tree, decisions

    with Image.open(upright) as page_image:
        page = page_image.convert("L")
        for block_index, block in enumerate(tree):
            lines = block["lines"]
            pitches = _line_pitches(lines)
            median_pitch = (
                sorted(pitches)[len(pitches) // 2] if pitches else 0.0
            )
            spans = _small_type_runs(lines, median_pitch)
            if not spans:
                routed_tree.append(block)
                continue

            new_lines: list[list[dict]] = []
            cursor = 0
            for first, last in spans:
                new_lines.extend(lines[cursor:first])
                cursor = last
                segment = lines[first:last]
                greek, latin, other = _script_counts(_text_of(segment))
                record = {
                    "block": block_index,
                    "line_range": [first, last],
                    "greek_chars": greek,
                    "latin_chars": latin,
                    "other_chars": other,
                    "selected_profile": "combined",
                    "reason": "",
                    "kind": "small-latin",
                }

                def keep(reason: str) -> None:
                    record["reason"] = reason
                    decisions.append(record)
                    new_lines.extend(segment)

                remaining = budget.remaining()
                if not budget.may_start():
                    keep("segment-budget-exhausted")
                    continue
                if remaining <= 0:
                    keep("time-budget-exhausted")
                    continue

                bbox = _run_bbox(segment, block.get("bbox"))
                record["bbox"] = bbox
                crop = _crop_box(bbox, page_width, page_height)
                if crop is None:
                    keep("crop-too-small")
                    continue

                budget.spend()
                handle, temporary = tempfile.mkstemp(suffix=".png")
                os.close(handle)
                try:
                    cropped = page.crop(crop)
                    if ROUTING_SMALL_TYPE_SCALE != 1.0:
                        cropped = cropped.resize(
                            (
                                int(cropped.width * ROUTING_SMALL_TYPE_SCALE),
                                int(cropped.height * ROUTING_SMALL_TYPE_SCALE),
                            ),
                            Image.LANCZOS,
                        )
                    cropped.save(temporary, format="PNG")
                    tsv = _run_tesseract(
                        binary,
                        Path(temporary),
                        model_dir,
                        latin_langs,
                        psm,
                        timeout=min(ENGINE_TIMEOUT_SECONDS, remaining),
                    )
                    candidate_tree = _parse_tsv(tsv)
                except subprocess.TimeoutExpired:
                    keep("latin-pass-timed-out")
                    continue
                except (RuntimeError, OSError):
                    keep("latin-pass-failed")
                    continue
                finally:
                    try:
                        os.unlink(temporary)
                    except OSError:
                        pass

                scale = ROUTING_SMALL_TYPE_SCALE
                rescaled = [
                    [
                        dict(
                            word,
                            left=word["left"] / scale,
                            top=word["top"] / scale,
                            width=word["width"] / scale,
                            height=word["height"] / scale,
                        )
                        for word in line
                    ]
                    for part in candidate_tree
                    for line in part["lines"]
                ]
                candidate = _restrict_to_span(
                    _offset_lines(rescaled, crop[0], crop[1]), segment
                )
                record["combined_confidence"] = round(_mean_confidence(segment), 4)
                record["candidate_confidence"] = round(_mean_confidence(candidate), 4)
                before = sum(1 for c in _text_of(segment) if c.isalpha())
                after = sum(1 for c in _text_of(candidate) if c.isalpha())
                record["coverage"] = round(after / before if before else 0.0, 4)

                rejection = _accept_replacement(segment, candidate, crop)
                if rejection is not None:
                    keep(f"rejected:{rejection}")
                    continue

                record["selected_profile"] = latin_langs
                record["reason"] = "replaced"
                decisions.append(record)
                new_lines.extend(candidate)

            new_lines.extend(lines[cursor:])
            routed_tree.append({"bbox": block.get("bbox"), "lines": new_lines})

    return routed_tree, decisions


def _median_of(values: list[float]) -> float:
    ordered = sorted(values)
    return float(ordered[len(ordered) // 2]) if ordered else 0.0


def _line_bbox(line: list[dict]) -> tuple[float, float, float, float]:
    left = min(word["left"] for word in line)
    top = min(word["top"] for word in line)
    right = max(word["left"] + word["width"] for word in line)
    bottom = max(word["top"] + word["height"] for word in line)
    return float(left), float(top), float(right), float(bottom)


def _is_detached_band(line: list[dict], median_height: float) -> bool:
    """Geometry and script only -- never a list of known-bad strings.

    The characters such a band happens to produce (``~``, ``\``, ``/``, stray
    digits) are corroborating evidence at most: a line is a candidate because
    of where it sits and how short and uncertain it is, not because of what it
    says. Keying on the strings would delete real punctuation, page numbers,
    apparatus sigla and mathematical symbols.
    """
    _, top, _, bottom = _line_bbox(line)
    height = bottom - top
    if median_height <= 0 or height > DETACHED_MAX_HEIGHT_RATIO * median_height:
        return False
    if min(word["confidence"] for word in line) > DETACHED_MAX_MIN_CONFIDENCE:
        return False
    text = _text_of([line])
    greek, _, _ = _script_counts(text)
    if greek >= DETACHED_MAX_GREEK_CHARS:
        return False
    letters = sum(1 for character in text if character.isalpha())
    tokens = [token for token in text.split() if token]
    if not tokens:
        return True
    substantive = sum(
        1
        for token in tokens
        if len(token) >= 3 and any(character.isalpha() for character in token)
    )
    return substantive <= 1 and letters <= max(3, len(text) // 6)


def _base_line_for(
    band: list[dict], lines: list[list[dict]], median_height: float
) -> int | None:
    """The Greek base line a band belongs to, or None."""
    bleft, btop, bright, bbottom = _line_bbox(band)
    band_width = bright - bleft
    best: tuple[float, int] | None = None
    for index, line in enumerate(lines):
        if line is band:
            continue
        left, top, right, bottom = _line_bbox(line)
        height = bottom - top
        if height < DETACHED_MIN_BASE_HEIGHT_RATIO * median_height:
            continue
        greek, latin, _ = _script_counts(_text_of([line]))
        if greek <= latin:
            continue
        # The band must sit inside the top part of the base line's own box.
        if not (top - 0.35 * median_height <= btop and bbottom <= top + 0.75 * height):
            continue
        overlap = max(0.0, min(bright, right) - max(bleft, left))
        if band_width <= 0 or overlap / band_width < DETACHED_MIN_HORIZONTAL_OVERLAP:
            continue
        distance = abs(top - btop)
        if best is None or distance < best[0]:
            best = (distance, index)
    return best[1] if best else None


def _accept_detached(
    base: list[dict], candidate: list[list[dict]], crop: tuple[int, int, int, int]
) -> str | None:
    """Gates a merged band+base candidate. Any failure keeps the combined text."""
    if not candidate:
        return "empty"
    left, top, right, bottom = crop
    for line in candidate:
        for word in line:
            if (
                word["left"] < left - 1
                or word["top"] < top - 1
                or word["left"] + word["width"] > right + 1
                or word["top"] + word["height"] > bottom + 1
            ):
                return "bbox-outside-crop"
    before, after = _text_of([base]), _text_of(candidate)
    letters_before = sum(1 for character in before if character.isalpha())
    letters_after = sum(1 for character in after if character.isalpha())
    if letters_before == 0:
        return "no-baseline-text"
    coverage = letters_after / letters_before
    if coverage < DETACHED_MIN_COVERAGE:
        return "coverage-collapsed"
    if coverage > DETACHED_MAX_COVERAGE:
        return "coverage-expanded"
    greek_before, latin_before, _ = _script_counts(before)
    greek_after, latin_after, _ = _script_counts(after)
    if greek_after < greek_before * ROUTING_MIN_GREEK_RETENTION:
        return "greek-lost"
    if latin_after < latin_before:
        return "latin-lost"
    if sum(c.isdigit() for c in after) < sum(c.isdigit() for c in before):
        return "digits-lost"
    if _mean_confidence(candidate) < DETACHED_MIN_CONFIDENCE:
        return "low-confidence"
    if any(
        not unicodedata.is_normalized("NFC", word["text"])
        for line in candidate
        for word in line
    ):
        return "not-nfc"
    return None


def route_detached_greek_accents(
    binary: str,
    upright: Path,
    model_dir: Path,
    tree: list[dict],
    page_width: int,
    page_height: int,
    budget: "_Budget",
) -> tuple[list[dict], list[dict]]:
    """Re-recognizes a detached accent band together with its base line."""
    from PIL import Image  # noqa: PLC0415

    decisions: list[dict] = []
    routed: list[dict] = []
    all_lines = [line for block in tree for line in block["lines"]]
    if not all_lines:
        return tree, decisions
    median_height = _median_of(
        [_line_bbox(line)[3] - _line_bbox(line)[1] for line in all_lines]
    )

    with Image.open(upright) as page_image:
        page = page_image.convert("L")
        for block_index, block in enumerate(tree):
            lines = block["lines"]
            bands = {
                index
                for index, line in enumerate(lines)
                if _is_detached_band(line, median_height)
            }
            if not bands:
                routed.append(block)
                continue
            replaced: dict[int, list[list[dict]]] = {}
            dropped: set[int] = set()
            for index in sorted(bands):
                band = lines[index]
                bleft, btop, bright, bbottom = _line_bbox(band)
                record = {
                    "kind": "detached-greek-accent",
                    "block": block_index,
                    "line": index,
                    "band_bbox": {"x": bleft, "y": btop,
                                  "width": bright - bleft, "height": bbottom - btop},
                    "band_height_ratio": round((bbottom - btop) / median_height, 4)
                    if median_height
                    else 0.0,
                    "band_confidence": round(_mean_confidence([band]), 4),
                    "selected_profile": "combined",
                    "reason": "",
                    "thresholds_version": ROUTING_THRESHOLDS_VERSION,
                }
                base_index = _base_line_for(band, lines, median_height)
                if base_index is None:
                    record["reason"] = "no-associated-base-line"
                    decisions.append(record)
                    continue
                base = lines[base_index]
                left0, top0, right0, bottom0 = _line_bbox(base)
                record["base_bbox"] = {"x": left0, "y": top0,
                                       "width": right0 - left0, "height": bottom0 - top0}
                remaining = budget.remaining()
                if not budget.may_start():
                    record["reason"] = (
                        "segment-budget-exhausted" if remaining > 0
                        else "time-budget-exhausted"
                    )
                    decisions.append(record)
                    continue
                crop = _crop_box(
                    {
                        "x": min(bleft, left0),
                        "y": min(btop, top0),
                        "width": max(bright, right0) - min(bleft, left0),
                        "height": max(bbottom, bottom0) - min(btop, top0),
                    },
                    page_width,
                    page_height,
                )
                if crop is None:
                    record["reason"] = "crop-too-small"
                    decisions.append(record)
                    continue

                budget.spend()
                handle, temporary = tempfile.mkstemp(suffix=".png")
                os.close(handle)
                try:
                    cropped = page.crop(crop)
                    if DETACHED_CROP_SCALE != 1.0:
                        cropped = cropped.resize(
                            (int(cropped.width * DETACHED_CROP_SCALE),
                             int(cropped.height * DETACHED_CROP_SCALE)),
                            Image.LANCZOS,
                        )
                    cropped.save(temporary, format="PNG")
                    tsv = _run_tesseract(
                        binary, Path(temporary), model_dir, "grc",
                        DETACHED_CROP_PSM,
                        timeout=min(ENGINE_TIMEOUT_SECONDS, remaining),
                    )
                    candidate_tree = _parse_tsv(tsv)
                except subprocess.TimeoutExpired:
                    record["reason"] = "greek-pass-timed-out"
                    decisions.append(record)
                    continue
                except (RuntimeError, OSError):
                    record["reason"] = "greek-pass-failed"
                    decisions.append(record)
                    continue
                finally:
                    try:
                        os.unlink(temporary)
                    except OSError:
                        pass

                scale = DETACHED_CROP_SCALE
                candidate = [
                    [
                        dict(word,
                             left=word["left"] / scale + crop[0],
                             top=word["top"] / scale + crop[1],
                             width=word["width"] / scale,
                             height=word["height"] / scale)
                        for word in line
                    ]
                    for part in candidate_tree
                    for line in part["lines"]
                ]
                record["candidate_confidence"] = round(_mean_confidence(candidate), 4)
                record["base_confidence"] = round(_mean_confidence([base]), 4)
                before = _text_of([base])
                after = _text_of(candidate)
                record["coverage"] = round(
                    sum(c.isalpha() for c in after)
                    / max(1, sum(c.isalpha() for c in before)),
                    4,
                )
                gb, lb, _ = _script_counts(before)
                ga, la, _ = _script_counts(after)
                record["greek_before"], record["greek_after"] = gb, ga
                record["latin_before"], record["latin_after"] = lb, la
                record["digits_before"] = sum(c.isdigit() for c in before)
                record["digits_after"] = sum(c.isdigit() for c in after)

                rejection = _accept_detached(base, candidate, crop)
                if rejection is not None:
                    # Re-recognition is not safe here, so the base line keeps
                    # its combined reading. The band itself is still dropped:
                    # it is, by the detection criteria above, a short,
                    # low-confidence line with no substantive alphanumeric
                    # content sitting inside another line's box, and shipping
                    # it as text is what puts `\ »" / 3 \ 3` in the text
                    # layer. Suppression removes noise; it recovers nothing,
                    # and is reported separately for exactly that reason.
                    dropped.add(index)
                    record["reason"] = f"suppressed:{rejection}"
                    decisions.append(record)
                    continue
                replaced[base_index] = candidate
                dropped.add(index)
                record["selected_profile"] = "grc"
                record["reason"] = "replaced"
                decisions.append(record)

            new_lines: list[list[dict]] = []
            for index, line in enumerate(lines):
                if index in dropped:
                    continue
                if index in replaced:
                    new_lines.extend(replaced[index])
                else:
                    new_lines.append(line)
            routed.append({"bbox": block.get("bbox"), "lines": new_lines})

    return routed, decisions


class _Budget:
    """Shared subprocess and wall-clock allowance for all routing passes."""

    def __init__(self, segments: int, seconds: float) -> None:
        self._segments = segments
        self._seconds = seconds
        self._started = time.monotonic()

    def remaining(self) -> float:
        return self._seconds - (time.monotonic() - self._started)

    def may_start(self) -> bool:
        return self._segments > 0 and self.remaining() > 0

    def spend(self) -> None:
        self._segments -= 1


def route_greek_blocks(
    binary: str,
    upright: Path,
    model_dir: Path,
    tree: list[dict],
    psm: int,
    page_width: int,
    page_height: int,
    budget: "_Budget",
) -> tuple[list[dict], list[dict]]:
    """Re-recognizes unambiguously Greek regions with the Greek-only model.

    The combined pass stays authoritative for layout, reading order, and every
    region this declines to touch.  Returns the (possibly updated) tree and one
    audit record per considered region.
    """
    from PIL import Image  # noqa: PLC0415

    decisions: list[dict] = []
    routed_tree: list[dict] = []

    with Image.open(upright) as page_image:
        page = page_image.convert("L")

        for block_index, block in enumerate(tree):
            lines = block["lines"]
            # Rebuilt in reading order rather than spliced by index: a
            # candidate rarely has the same line count as the region it
            # replaces, and index splicing into a list that is already
            # changing length silently overlaps later regions. That produced
            # visibly duplicated lines and a CER regression from 0.064 to
            # 0.151 before it was caught.
            new_lines: list[list[dict]] = []

            for leans_greek, first, last in _script_runs(lines):
                segment = lines[first:last]
                greek, latin, other = _script_counts(_text_of(segment))
                record = {
                    "block": block_index,
                    "line_range": [first, last],
                    "greek_chars": greek,
                    "latin_chars": latin,
                    "other_chars": other,
                    "selected_profile": "combined",
                    "reason": "",
                }

                def keep(reason: str) -> None:
                    record["reason"] = reason
                    decisions.append(record)
                    new_lines.extend(segment)

                if not leans_greek or not _is_greek_dominant(greek, latin, other):
                    keep("not-greek-dominant")
                    continue
                remaining = budget.remaining()
                if not budget.may_start():
                    keep("segment-budget-exhausted" if remaining > 0
                         else "time-budget-exhausted")
                    continue

                combined_confidence = _mean_confidence(segment)
                if combined_confidence >= ROUTING_MAX_COMBINED_CONFIDENCE:
                    record["combined_confidence"] = round(combined_confidence, 4)
                    keep("combined-already-confident")
                    continue

                bbox = _run_bbox(segment, block.get("bbox"))
                record["bbox"] = bbox
                crop = _crop_box(bbox, page_width, page_height)
                if crop is None:
                    keep("crop-too-small")
                    continue

                budget.spend()
                handle, temporary = tempfile.mkstemp(suffix=".png")
                os.close(handle)
                try:
                    page.crop(crop).save(temporary, format="PNG")
                    # The budget bounds the subprocess itself, not just the
                    # decision to start one: a single wedged call must not be
                    # able to spend the whole page's allowance.
                    tsv = _run_tesseract(
                        binary,
                        Path(temporary),
                        model_dir,
                        "grc",
                        psm,
                        timeout=min(ENGINE_TIMEOUT_SECONDS, remaining),
                    )
                    candidate_tree = _parse_tsv(tsv)
                except subprocess.TimeoutExpired:
                    keep("greek-pass-timed-out")
                    continue
                except (RuntimeError, OSError):
                    keep("greek-pass-failed")
                    continue
                finally:
                    try:
                        os.unlink(temporary)
                    except OSError:
                        pass

                candidate = _restrict_to_span(
                    _offset_lines(
                        [line for part in candidate_tree for line in part["lines"]],
                        crop[0],
                        crop[1],
                    ),
                    segment,
                )
                candidate, preserved = _merge_by_geometry(segment, candidate)
                record["lines_kept_as_mixed"] = preserved
                record["combined_confidence"] = round(_mean_confidence(segment), 4)
                record["candidate_confidence"] = round(_mean_confidence(candidate), 4)
                before_letters = sum(1 for c in _text_of(segment) if c.isalpha())
                after_letters = sum(1 for c in _text_of(candidate) if c.isalpha())
                record["coverage"] = round(
                    after_letters / before_letters if before_letters else 0.0, 4
                )

                rejection = _accept_replacement(segment, candidate, crop)
                if rejection is not None:
                    keep(f"rejected:{rejection}")
                    continue

                record["selected_profile"] = "grc"
                record["reason"] = "replaced"
                decisions.append(record)
                new_lines.extend(candidate)

            routed_tree.append({"bbox": block.get("bbox"), "lines": new_lines})

    return routed_tree, decisions


# ---------------------------------------------------------------------------
# PaddleOCR / RapidOCR engine (evaluation candidate)
# ---------------------------------------------------------------------------


def _run_paddle(image: Path, model_dir: Path, languages: str) -> list[list[list[dict]]]:
    """Detector-box recognizer adapter, kept for bake-off comparability.

    PaddleOCR/RapidOCR return one rectangle per detected text region with no
    word segmentation.  This adapter therefore emits **one line per detected
    region and one word per region**, and marks the response
    ``word_segmentation: "region"`` so the caller knows word boxes are region
    boxes rather than real words.  The Rust logical-line assembler is what
    rebuilds rows from these regions; this script does not pretend to have
    information it was never given.
    """
    from paddleocr import PaddleOCR  # noqa: PLC0415

    engine = PaddleOCR(
        lang=languages,
        use_doc_orientation_classify=False,
        use_doc_unwarping=False,
        use_textline_orientation=False,
    )
    result = engine.predict(str(image))
    regions: list[dict] = []
    for page in result or []:
        texts = page.get("rec_texts", [])
        scores = page.get("rec_scores", [])
        boxes = page.get("rec_polys", page.get("dt_polys", []))
        for text, score, box in zip(texts, scores, boxes):
            text = normalize(str(text))
            if not text.strip():
                continue
            xs = [float(point[0]) for point in box]
            ys = [float(point[1]) for point in box]
            regions.append(
                {
                    "text": text,
                    "left": min(xs),
                    "top": min(ys),
                    "width": max(xs) - min(xs),
                    "height": max(ys) - min(ys),
                    "confidence": max(0.0, min(1.0, float(score))),
                    "word_num": 0,
                }
            )
    regions.sort(key=lambda region: (round(region["top"]), region["left"]))
    return [{"bbox": None, "lines": [[region]]} for region in regions]


# ---------------------------------------------------------------------------
# Response assembly
# ---------------------------------------------------------------------------


def _bbox(items: list[dict]) -> dict:
    left = min(item["left"] for item in items)
    top = min(item["top"] for item in items)
    right = max(item["left"] + item["width"] for item in items)
    bottom = max(item["top"] + item["height"] for item in items)
    return {
        "x": float(left),
        "y": float(top),
        "width": float(right - left),
        "height": float(bottom - top),
    }


def _build_blocks(tree: list, page_width: int, page_height: int) -> tuple[list[dict], str]:
    blocks: list[dict] = []
    line_ordinal = 0
    all_text: list[str] = []
    for block_ordinal, block in enumerate(tree):
        lines = block["lines"]
        block_lines = []
        for words in lines:
            word_records = []
            for word_ordinal, word in enumerate(words):
                word_records.append(
                    {
                        "text": word["text"],
                        "normalized_text": " ".join(word["text"].split()),
                        "bbox": _clamp(
                            {
                                "x": float(word["left"]),
                                "y": float(word["top"]),
                                "width": float(word["width"]),
                                "height": float(word["height"]),
                            },
                            page_width,
                            page_height,
                        ),
                        "confidence": word["confidence"],
                        "reading_order": word_ordinal,
                    }
                )
                all_text.append(word["text"])
            block_lines.append(
                {
                    "bbox": _clamp(_bbox(words), page_width, page_height),
                    "confidence": min(word["confidence"] for word in words),
                    "reading_order": line_ordinal,
                    "words": word_records,
                }
            )
            line_ordinal += 1
        if not block_lines:
            continue
        flat = [word for line in lines for word in line]
        blocks.append(
            {
                "bbox": _clamp(_bbox(flat), page_width, page_height),
                "confidence": min(word["confidence"] for word in flat),
                "reading_order": block_ordinal,
                "lines": block_lines,
            }
        )
    return blocks, " ".join(all_text)


def _unrotate_blocks(
    blocks: list[dict], clockwise: int, width: int, height: int
) -> list[dict]:
    for block in blocks:
        block["bbox"] = _clamp(
            unrotate_box(block["bbox"], clockwise, width, height), width, height
        )
        for line in block["lines"]:
            line["bbox"] = _clamp(
                unrotate_box(line["bbox"], clockwise, width, height), width, height
            )
            for word in line["words"]:
                word["bbox"] = _clamp(
                    unrotate_box(word["bbox"], clockwise, width, height), width, height
                )
    return blocks


@contextlib.contextmanager
def _upright(image_path: Path, clockwise: int):
    """Yields a path to the page rotated upright, cleaning up afterwards."""
    if not clockwise:
        yield image_path
        return
    from PIL import Image  # noqa: PLC0415

    handle, temporary = tempfile.mkstemp(suffix=".png")
    os.close(handle)
    try:
        with Image.open(image_path) as image:
            # PIL rotates counter-clockwise, so negate to turn clockwise.
            image.rotate(-clockwise, expand=True).save(temporary, format="PNG")
        yield Path(temporary)
    finally:
        try:
            os.unlink(temporary)
        except OSError:
            pass


def _clamp(bbox: dict, width: int, height: int) -> dict:
    x = max(0.0, min(bbox["x"], float(width)))
    y = max(0.0, min(bbox["y"], float(height)))
    return {
        "x": x,
        "y": y,
        "width": max(0.0, min(bbox["width"], float(width) - x)),
        "height": max(0.0, min(bbox["height"], float(height) - y)),
    }


def run(args: argparse.Namespace) -> int:
    if args.protocol != PROTOCOL or args.protocol_version != PROTOCOL_VERSION:
        print("unsupported OCR protocol", file=sys.stderr)
        return EXIT_UNAVAILABLE

    request_line = sys.stdin.readline()
    if not request_line:
        print("provider request is missing", file=sys.stderr)
        return EXIT_UNAVAILABLE
    request = json.loads(request_line)

    profile_name = request.get("language_profile") or args.language_profile
    try:
        profile = _profile(profile_name)
    except ValueError:
        print("unsupported language profile", file=sys.stderr)
        return EXIT_UNAVAILABLE

    model_dir = Path(args.model_dir)
    if not model_dir.is_dir():
        print("model directory is missing", file=sys.stderr)
        return EXIT_UNAVAILABLE

    from PIL import Image  # noqa: PLC0415

    with Image.open(args.input) as image:
        page_width, page_height = image.size

    orientation_degrees = 0
    orientation_confidence = 0.0
    routing_enabled = False
    small_type_enabled = False
    detached_enabled = False
    routing_decisions: list[dict] = []
    if args.engine == "tesseract":
        binary = args.engine_binary or "tesseract"
        languages = profile["tesseract"]
        try:
            # `osd` is always verified: the orientation pass consults it on
            # every page, so a missing or swapped orientation model would
            # silently change what the recognizer sees without appearing in
            # the provenance.
            required = languages.split("+")
            if "osd" not in required:
                required = required + ["osd"]
            # Routing only applies where the profile genuinely spans both
            # scripts; a single-script profile has nothing to route between.
            profile_langs = set(languages.split("+"))
            routing_enabled = (
                args.routing == ROUTING_MODE
                and "grc" in profile_langs
                and bool(profile_langs & {"deu", "eng", "lat"})
            )
            small_type_enabled = args.small_type_latin == "on" and bool(
                profile_langs & {"deu", "eng", "lat"}
            )
            detached_enabled = (
                args.detached_greek_accents == "on" and "grc" in profile_langs
            )
            provenance = _verify_models(
                model_dir, _load_manifest(model_dir), "tesseract", required
            )
        except (FileNotFoundError, ValueError, json.JSONDecodeError):
            print("local OCR models are not provisioned", file=sys.stderr)
            return EXIT_UNAVAILABLE
        try:
            orientation_degrees, orientation_confidence = detect_orientation(
                binary, Path(args.input), model_dir
            )
            if orientation_degrees in (90, 270):
                upright_width, upright_height = page_height, page_width
            else:
                upright_width, upright_height = page_width, page_height
            with _upright(Path(args.input), orientation_degrees) as upright:
                # The combined pass always runs and always decides layout.
                tsv = _run_tesseract(binary, upright, model_dir, languages, args.psm)
                tree = _parse_tsv(tsv)
                if routing_enabled or small_type_enabled or detached_enabled:
                    # One allowance shared by both passes, so enabling both
                    # cannot double the worst-case cost of a page.
                    budget = _Budget(
                        ROUTING_MAX_SEGMENTS, ROUTING_TIME_BUDGET_SECONDS
                    )
                    if routing_enabled:
                        tree, routing_decisions = route_greek_blocks(
                            binary,
                            upright,
                            model_dir,
                            tree,
                            args.psm,
                            upright_width,
                            upright_height,
                            budget,
                        )
                    if small_type_enabled:
                        # Small Latin type -- footnotes, apparatus,
                        # bibliographies -- which the page-wide pass reads
                        # under the body text's assumptions.
                        tree, small_decisions = route_small_latin_regions(
                            binary,
                            upright,
                            model_dir,
                            tree,
                            args.psm,
                            upright_width,
                            upright_height,
                            languages,
                            budget,
                        )
                        routing_decisions.extend(small_decisions)
                    if detached_enabled:
                        tree, detached_decisions = route_detached_greek_accents(
                            binary,
                            upright,
                            model_dir,
                            tree,
                            upright_width,
                            upright_height,
                            budget,
                        )
                        routing_decisions.extend(detached_decisions)
        except FileNotFoundError:
            print("recognition engine is not installed", file=sys.stderr)
            return EXIT_UNAVAILABLE
        except subprocess.TimeoutExpired:
            print("recognition engine timed out", file=sys.stderr)
            return EXIT_FAILED
        engine_name = "tesseract"
        engine_version = _tesseract_version(binary)
        word_segmentation = "word"
        model_label = languages
    elif args.engine == "paddleocr":
        languages = args.paddle_lang or "en"
        try:
            tree = _run_paddle(Path(args.input), model_dir, languages)
        except ImportError:
            print("recognition engine is not installed", file=sys.stderr)
            return EXIT_UNAVAILABLE
        provenance = {
            "model_set": "paddleocr-evaluation",
            "model_set_version": languages,
            "files": [],
            # An evaluation-only path: nothing here is cleared for
            # distribution, so no license claim is manufactured.
            "license": "unverified-evaluation-only",
        }
        engine_name = "paddleocr"
        engine_version = _paddle_version()
        word_segmentation = "region"
        model_label = f"PP-OCR:{languages}"
    else:
        print("unsupported OCR engine", file=sys.stderr)
        return EXIT_UNAVAILABLE

    # Recognition happened on the upright raster; the response must be in the
    # coordinate space of the image the runner actually supplied.
    if orientation_degrees in (90, 270):
        upright_width, upright_height = page_height, page_width
    else:
        upright_width, upright_height = page_width, page_height
    blocks, joined = _build_blocks(tree, upright_width, upright_height)
    if orientation_degrees:
        blocks = _unrotate_blocks(blocks, orientation_degrees, page_width, page_height)
    violations = script_violations(joined, profile["scripts"])

    engine_binary_sha256 = ""
    if args.engine == "tesseract":
        # Bundled runs pass an absolute executable and must bind its bytes to
        # provenance. Legacy developer runs may still pass the PATH name
        # tesseract; resolve it when possible without changing execution
        # semantics or making PATH resolution a production fallback.
        resolved_binary = shutil.which(binary) or binary
        try:
            engine_binary_sha256 = hashlib.sha256(
                Path(resolved_binary).read_bytes()
            ).hexdigest()
        except OSError:
            engine_binary_sha256 = ""

    parameters = {
        "language_profile": profile_name,
        "language_model_set": model_label,
        "model_license": provenance["license"],
        "model_set": provenance["model_set"],
        "model_set_version": provenance["model_set_version"],
        "model_files_sha256": ",".join(
            f"{item['language']}:{item['sha256']}" for item in provenance["files"]
        ),
        "word_segmentation": word_segmentation,
        "script_violations": str(violations),
        "psm": str(args.psm),
        "orientation_degrees": str(orientation_degrees),
        "orientation_confidence": f"{orientation_confidence:.2f}",
        "engine_mode": "lstm" if args.engine == "tesseract" else "detector-recognizer",
        "engine_binary": binary if args.engine == "tesseract" else "",
        "engine_binary_sha256": engine_binary_sha256,
        "routing_mode": ROUTING_MODE if routing_enabled else "off",
        "small_type_latin": "on" if small_type_enabled else "off",
        "detached_greek_accents": "on" if detached_enabled else "off",
        "detached_bands_seen": str(
            sum(1 for item in routing_decisions
                if item.get("kind") == "detached-greek-accent")
        ),
        "detached_bands_replaced": str(
            sum(1 for item in routing_decisions
                if item.get("kind") == "detached-greek-accent"
                and item["reason"] == "replaced")
        ),
        "detached_bands_suppressed": str(
            sum(1 for item in routing_decisions
                if item.get("kind") == "detached-greek-accent"
                and item["reason"].startswith("suppressed:"))
        ),
        "detached_bands_kept": str(
            sum(1 for item in routing_decisions
                if item.get("kind") == "detached-greek-accent"
                and not item["reason"].startswith(("suppressed:", "replaced")))
        ),
        "routing_thresholds_version": (
            ROUTING_THRESHOLDS_VERSION
            if (routing_enabled or small_type_enabled or detached_enabled)
            else ""
        ),
        "routing_segments_total": str(len(routing_decisions)),
        "routing_segments_rerouted": str(
            sum(1 for item in routing_decisions if item["reason"] == "replaced")
        ),
        "routing_segments_small_latin": str(
            sum(1 for item in routing_decisions if item.get("kind") == "small-latin")
        ),
        "routing_segments_fallback": str(
            sum(1 for item in routing_decisions if item["reason"].startswith("rejected:"))
        ),
        "routing_models": ",".join(
            part for part in (
                "grc" if routing_enabled else "",
                _latin_languages(languages) if small_type_enabled else "",
            ) if part
        ),
    }

    response = {
        "protocol": PROTOCOL,
        "protocol_version": PROTOCOL_VERSION,
        "page_index": request["page_index"],
        "input_asset_sha256": request["input_asset_sha256"],
        "width": page_width,
        "height": page_height,
        "blocks": blocks,
        "engine": engine_name,
        "model": model_label,
        "version": engine_version,
        "parameters": parameters,
        "execution_location": "local",
    }
    # The audit trail lives at the top level of the response rather than in
    # `parameters`, for two reasons: `parameters` is promoted into the page
    # provenance that downstream digests compare, and the whole response is
    # already retained verbatim as the raw provider artifact.  It carries
    # geometry, counts, and the decision — never candidate text.  A rejected
    # Greek reading must not be recoverable from the evidence.
    if routing_enabled or small_type_enabled or detached_enabled:
        response["routing_decisions"] = routing_decisions
    sys.stdout.write(json.dumps(response, ensure_ascii=False, separators=(",", ":")))
    sys.stdout.write("\n")
    return EXIT_OK


def _tesseract_version(binary: str) -> str:
    completed = subprocess.run(
        [binary, "--version"], capture_output=True, timeout=30, check=False
    )
    first = completed.stdout.decode("utf-8", errors="replace").splitlines()
    match = re.search(r"tesseract\s+(\S+)", first[0]) if first else None
    return match.group(1) if match else "unknown"


def _paddle_version() -> str:
    try:
        import paddleocr  # noqa: PLC0415

        return getattr(paddleocr, "__version__", "unknown")
    except ImportError:
        return "unknown"


def main() -> int:
    parser = argparse.ArgumentParser(description="mpdf offline OCR sidecar")
    parser.add_argument("--protocol", required=True)
    parser.add_argument("--protocol-version", required=True)
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--input", required=True)
    parser.add_argument("--engine", default="tesseract",
                        choices=("tesseract", "paddleocr"))
    parser.add_argument("--language-profile", default=DEFAULT_PROFILE)
    parser.add_argument("--engine-binary", default=None)
    parser.add_argument("--paddle-lang", default=None)
    # PSM 6 = "a uniform block of text". Orientation is handled by its own
    # pass above, so recognition can use the mode that actually reads a page:
    # --psm 1 silently drops the right-flush page number from printed
    # contents rows, which the gold set measures (toc-* samples).
    parser.add_argument("--psm", type=int, default=6)
    # Default OFF, on the evidence.  Enabling it is a deliberate opt-in.
    #
    # On a 16-page slice of the CGPG holdout, routing removed every Greek to
    # Latin confusion (139 -> 0).  On the *full* 63-page holdout it removes
    # 1.9% of them (951 -> 933) and leaves CER slightly worse (0.1766 ->
    # 0.1797), because the guards that stop it destroying German and Latin
    # also stop it firing: 18 regions of 197.  The earlier headline was mostly
    # the damage, not the repair.  It stays available and measurable behind
    # this flag until it earns a default on a corpus that size.
    parser.add_argument(
        "--routing", default="off", choices=("off", ROUTING_MODE)
    )
    # Default OFF, like Greek routing, and for the same reason: the evidence
    # is not yet broad enough to justify changing what every user gets.
    #
    # It does help where it was measured -- on one real scanned page the 8 pt
    # footnote goes from 5 umlaut errors of 16 to 3, and the 11 pt body stays
    # at 0 of 24 -- but that is a single page, it misses the 2/16 target, and
    # it repairs only two of the four known failing words (`Zwölfzahl` and
    # `gegründete`; `diesbezüglichen` and `Stählin` still lose the diaeresis).
    # One page of evidence is enough to keep a feature and measure it, not
    # enough to turn it on for every document.
    parser.add_argument(
        "--small-type-latin", default="off", choices=("on", "off")
    )
    # Default OFF: measured on one Teubner-set source only. It is a large gain
    # there (raw CER 0.1756 -> 0.1086 over four pages) but a single typeface is
    # not evidence about Greek editions in general.
    parser.add_argument(
        "--detached-greek-accents", default="off", choices=("on", "off")
    )
    args = parser.parse_args()
    try:
        return run(args)
    except Exception:
        # Never emit a traceback or recognized text to the parent or a log.
        print("local OCR provider failed", file=sys.stderr)
        return EXIT_FAILED


if __name__ == "__main__":
    raise SystemExit(main())
