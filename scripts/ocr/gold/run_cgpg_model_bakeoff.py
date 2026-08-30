#!/usr/bin/env python3
"""Pre-registered CGPG bake-off for Greek Tesseract model choices.

The evaluation is intentionally fixed: the lexicographically first twelve
scorable pages in the corpus loader's deterministic holdout split, Tesseract
PSM 6 / OEM 1, and four candidates (``ell``, ``ell`` plus the conservative
text-only enhancer, ``grc``, and ``ell+grc``). Candidate thresholds are
constants, not command-line knobs, so a failed run cannot be made to pass by
rerunning it with friendlier criteria.

Only metrics and provenance are written to JSON.  OCR text, human
transcriptions, page images, and image paths are never serialized.
"""

from __future__ import annotations

import argparse
import copy
import csv
import difflib
import hashlib
import io
import json
import os
import platform
import subprocess
import sys
import time
import unicodedata
from dataclasses import dataclass
from datetime import date
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cgpg  # noqa: E402
import greek_text_enhancer as enhancer  # noqa: E402
import metrics as metrics_module  # noqa: E402
import run_cgpg  # noqa: E402

try:  # Exact same distance, with a compiled implementation when available.
    import rapidfuzz  # noqa: E402
    from rapidfuzz.distance import Levenshtein as _RapidLevenshtein  # noqa: E402

    LEVENSHTEIN_BACKEND = f"rapidfuzz {rapidfuzz.__version__}"

    def _levenshtein(left: str, right: str) -> int:
        return int(_RapidLevenshtein.distance(left, right))

except ImportError:  # pragma: no cover - exercised on dependency-minimal hosts.
    LEVENSHTEIN_BACKEND = "scripts/ocr/gold/metrics.py"

    def _levenshtein(left: str, right: str) -> int:
        return metrics_module.levenshtein(left, right)

SCORABLE_PAGE_COUNT = 12
PSM = 6
OEM = 1
ENGINE_TIMEOUT_SECONDS = 900
MAX_ENGINE_OUTPUT_BYTES = 64 * 1024 * 1024

# Pre-registered by the task before the evaluation is run.  These are not CLI
# options and must not be adjusted in response to observed output.
MIN_POLYTONIC_CER_IMPROVEMENT_PERCENT = 30.0
MAX_BASE_LETTER_CER_DEGRADATION_PERCENTAGE_POINTS = 0.5
MAX_CORRECT_BASE_REWRITE_PERCENT = 0.5
REQUIRED_LINEAGE_PERCENT = 100.0

CORPUS_DOI = "10.5281/zenodo.20008699"
CORPUS_ARCHIVE_SHA256 = (
    "2ee5d79f3c781dc1b64fa386f0f194a762ab183cd36b97f5d873ce0a3004e1f7"
)
CORPUS_LICENSE = "CC BY 4.0"

TSV_FIELDS = (
    "level page_num block_num par_num line_num word_num "
    "left top width height conf text"
).split()

RAW_CANDIDATES: tuple[tuple[str, str], ...] = (
    ("ell", "ell"),
    ("grc", "grc"),
    ("ell+grc", "ell+grc"),
)
CANDIDATE_ORDER = ("ell", "ell+enhancer", "grc", "ell+grc")
REPO_ROOT = Path(__file__).resolve().parents[3]
SHIPPED_RUST_ENHANCER = (
    REPO_ROOT / "crates/mpdf-core/src/ocr_provider/text_enhancer.rs"
)


class BakeoffError(RuntimeError):
    """The fixed evaluation could not be executed as specified."""


@dataclass(frozen=True)
class Invocation:
    response: dict
    seconds: float


@dataclass(frozen=True)
class CerCounts:
    reference_chars: int = 0
    char_edits: int = 0
    samples: int = 0
    nfc_violations: int = 0

    @property
    def cer(self) -> float:
        return self.char_edits / self.reference_chars if self.reference_chars else 0.0

    def merge(self, other: "CerCounts") -> "CerCounts":
        return CerCounts(
            reference_chars=self.reference_chars + other.reference_chars,
            char_edits=self.char_edits + other.char_edits,
            samples=self.samples + other.samples,
            nfc_violations=self.nfc_violations + other.nfc_violations,
        )

    def as_dict(self) -> dict[str, int | float]:
        return {
            "samples": self.samples,
            "reference_chars": self.reference_chars,
            "char_edits": self.char_edits,
            "cer": round(self.cer, 6),
            "nfc_violations": self.nfc_violations,
        }


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_digest(value: object) -> str:
    encoded = json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def select_fixed_pages(pages: list[cgpg.Page]) -> list[cgpg.Page]:
    """Returns the fixed first 12 scorable holdout pages.

    ``cgpg.split_corpus`` is the repository's pre-existing deterministic
    document-level split. Selecting only after that split prevents development
    pages used while writing the harness from leaking into this decision set.
    """
    _development, holdout = cgpg.split_corpus(pages)
    scorable = sorted(
        (page for page in holdout if page.transcribed_lines), key=lambda page: page.name
    )
    if len(scorable) < SCORABLE_PAGE_COUNT:
        raise BakeoffError(
            f"CGPG holdout has only {len(scorable)} scorable pages; "
            f"{SCORABLE_PAGE_COUNT} are required"
        )
    return scorable[:SCORABLE_PAGE_COUNT]


def tesseract_argv(
    binary: Path, image: Path | str, tessdata: Path, languages: str
) -> list[str]:
    """Builds the exact recognition argv shared by every candidate."""
    return [
        str(binary),
        str(image),
        "stdout",
        "--tessdata-dir",
        str(tessdata),
        "-l",
        languages,
        "--psm",
        str(PSM),
        "--oem",
        str(OEM),
        "-c",
        "preserve_interword_spaces=1",
        "-c",
        "tessedit_create_tsv=1",
    ]


def _bbox(record: dict[str, str]) -> dict[str, float]:
    try:
        return {
            "x": float(record["left"]),
            "y": float(record["top"]),
            "width": float(record["width"]),
            "height": float(record["height"]),
        }
    except ValueError as error:
        raise BakeoffError("Tesseract emitted a malformed TSV box") from error


def _union_boxes(boxes: list[dict[str, float]]) -> dict[str, float]:
    left = min(box["x"] for box in boxes)
    top = min(box["y"] for box in boxes)
    right = max(box["x"] + box["width"] for box in boxes)
    bottom = max(box["y"] + box["height"] for box in boxes)
    return {"x": left, "y": top, "width": right - left, "height": bottom - top}


def parse_tsv(tsv: str) -> dict:
    """Parses Tesseract TSV into the response shape used by ``run_cgpg``."""
    # Tesseract's final text field may itself be a bare ``"`` glyph.  TSV is
    # not CSV-quoted, so enabling csv's default quote handling would consume
    # subsequent physical rows into that glyph and catastrophically inflate
    # the recognized line.  Treat every character literally.
    rows = list(
        csv.reader(io.StringIO(tsv), delimiter="\t", quoting=csv.QUOTE_NONE)
    )
    if not rows or rows[0] != TSV_FIELDS:
        raise BakeoffError("unexpected Tesseract TSV header")

    blocks: dict[int, dict] = {}
    for parts in rows[1:]:
        if len(parts) != len(TSV_FIELDS):
            continue
        record = dict(zip(TSV_FIELDS, parts))
        try:
            level = int(record["level"])
            block_num = int(record["block_num"])
            paragraph_num = int(record["par_num"])
            line_num = int(record["line_num"])
            word_num = int(record["word_num"])
        except ValueError as error:
            raise BakeoffError("Tesseract emitted malformed TSV identifiers") from error

        if level == 2:
            block = blocks.setdefault(block_num, {"bbox": None, "lines": {}})
            block["bbox"] = _bbox(record)
            continue
        if level not in (4, 5):
            continue
        block = blocks.setdefault(block_num, {"bbox": None, "lines": {}})
        line_key = (paragraph_num, line_num)
        line = block["lines"].setdefault(line_key, {"bbox": None, "words": {}})
        if level == 4:
            line["bbox"] = _bbox(record)
            continue

        text = record["text"].strip()
        try:
            confidence = float(record["conf"])
        except ValueError as error:
            raise BakeoffError("Tesseract emitted a malformed confidence") from error
        if not text or confidence < 0.0:
            continue
        line["words"][word_num] = {
            "lineage_id": (
                f"p{record['page_num']}:b{block_num}:p{paragraph_num}:"
                f"l{line_num}:w{word_num}"
            ),
            "text": text,
            "bbox": _bbox(record),
            "confidence": max(0.0, min(1.0, confidence / 100.0)),
        }

    response_blocks: list[dict] = []
    for block_order, block_num in enumerate(sorted(blocks)):
        source_block = blocks[block_num]
        response_lines: list[dict] = []
        for line_order, line_key in enumerate(sorted(source_block["lines"])):
            source_line = source_block["lines"][line_key]
            words = [
                source_line["words"][number]
                for number in sorted(source_line["words"])
            ]
            if not words:
                continue
            line_box = source_line["bbox"] or _union_boxes(
                [word["bbox"] for word in words]
            )
            response_lines.append(
                {
                    "reading_order": line_order,
                    "bbox": line_box,
                    "words": words,
                }
            )
        if response_lines:
            response_blocks.append(
                {
                    "reading_order": block_order,
                    "bbox": source_block["bbox"],
                    "lines": response_lines,
                }
            )
    return {"blocks": response_blocks}


def run_tesseract(
    binary: Path, image: Path, tessdata: Path, languages: str
) -> Invocation:
    environment = dict(os.environ)
    environment["TESSDATA_PREFIX"] = str(tessdata)
    environment["OMP_THREAD_LIMIT"] = "1"
    argv = tesseract_argv(binary, image, tessdata, languages)
    started = time.perf_counter()
    try:
        completed = subprocess.run(
            argv,
            capture_output=True,
            timeout=ENGINE_TIMEOUT_SECONDS,
            env=environment,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise BakeoffError(f"Tesseract invocation failed: {type(error).__name__}") from error
    seconds = time.perf_counter() - started
    if completed.returncode != 0:
        raise BakeoffError(f"Tesseract exited with status {completed.returncode}")
    if len(completed.stdout) > MAX_ENGINE_OUTPUT_BYTES:
        raise BakeoffError("Tesseract output exceeded the evaluation limit")
    try:
        decoded = completed.stdout.decode("utf-8", errors="strict")
    except UnicodeDecodeError as error:
        raise BakeoffError("Tesseract TSV was not UTF-8") from error
    return Invocation(parse_tsv(decoded), seconds)


def _flatten_lineage(response: dict) -> list[tuple[str, tuple[float, ...]]]:
    flattened: list[tuple[str, tuple[float, ...]]] = []
    for block in response.get("blocks", []):
        for line in block.get("lines", []):
            for word in line.get("words", []):
                box = word["bbox"]
                flattened.append(
                    (
                        word["lineage_id"],
                        (box["x"], box["y"], box["width"], box["height"]),
                    )
                )
    return flattened


def lineage_stats(before: dict, after: dict) -> dict[str, int | float]:
    source = _flatten_lineage(before)
    output = _flatten_lineage(after)
    source_ids = [item[0] for item in source]
    output_ids = [item[0] for item in output]
    token_preserved = sum(
        left == right for left, right in zip(source_ids, output_ids)
    ) if len(source_ids) == len(output_ids) else 0
    box_preserved = sum(
        left == right for left, right in zip(source, output)
    ) if len(source) == len(output) else 0
    denominator = len(source)
    token_rate = (
        token_preserved / denominator if denominator else float(not output)
    )
    box_rate = box_preserved / denominator if denominator else float(not output)
    return {
        "source_tokens": denominator,
        "output_tokens": len(output),
        "token_lineage_preserved": token_preserved,
        "box_lineage_preserved": box_preserved,
        "token_lineage_rate": token_rate,
        "box_lineage_rate": box_rate,
    }


def enhance_response(response: dict) -> tuple[dict, dict[str, int | float]]:
    """Changes token text only and returns aggregate, content-free audit data."""
    enhanced = copy.deepcopy(response)
    rewrites = normalized_tokens = 0
    for block in enhanced.get("blocks", []):
        for line in block.get("lines", []):
            for word in line.get("words", []):
                result = enhancer.enhance_token(
                    word["text"], float(word["confidence"])
                )
                word["text"] = result.text
                rewrites += result.rewrites
                normalized_tokens += int(result.normalized)
    audit = lineage_stats(response, enhanced)
    audit["lookalike_rewrites"] = rewrites
    audit["nfc_normalized_tokens"] = normalized_tokens
    return enhanced, audit


def base_letter_text(text: str) -> str:
    """Removes combining marks for the explicitly secondary CER view."""
    decomposed = unicodedata.normalize("NFD", text)
    stripped = "".join(
        character for character in decomposed if not unicodedata.combining(character)
    )
    return unicodedata.normalize("NFC", stripped)


def _cer_normal_form(text: str) -> str:
    return " ".join(unicodedata.normalize("NFC", text).split())


def cer_counts(reference: str, hypothesis: str) -> CerCounts:
    """Returns the exact CER counts needed by this pre-registered bake-off.

    The general gold metrics also compute WER, exact-line rate, diacritic
    examples, and a full backtracked alignment.  None of those quantities is a
    gate here.  Avoiding that matrix for multi-thousand-character CGPG regions
    leaves the CER definition unchanged while making the fixed run practical.
    """
    reference_norm = _cer_normal_form(reference)
    hypothesis_norm = _cer_normal_form(hypothesis)
    return CerCounts(
        reference_chars=len(reference_norm),
        char_edits=_levenshtein(reference_norm, hypothesis_norm),
        samples=1,
        nfc_violations=int(not unicodedata.is_normalized("NFC", hypothesis)),
    )


def score_outputs(
    pages: list[cgpg.Page],
    outputs: dict[str, dict | None],
    timings: dict[str, float],
) -> dict:
    raw_total = CerCounts()
    base_total = CerCounts()
    per_script_raw: dict[str, CerCounts] = {}
    per_script_base: dict[str, CerCounts] = {}
    failures: list[dict[str, str]] = []
    per_page: list[dict] = []
    missing_lines = extra_lines = 0

    for page in pages:
        response = outputs.get(page.name)
        if response is None:
            failures.append({"page_id": page.name, "reason": "recognition_failed"})
            continue
        page_raw = CerCounts()
        page_base = CerCounts()
        page_missing = page_extra = transcribed_regions = 0
        for region in page.regions:
            if not region.text.strip():
                continue
            transcribed_regions += 1
            script = cgpg.script_of_text(region.text)
            hypothesis_lines = run_cgpg.lines_in_region(response, region)
            hypothesis = "\n".join(hypothesis_lines)
            raw = cer_counts(region.text, hypothesis)
            folded = cer_counts(
                base_letter_text(region.text), base_letter_text(hypothesis)
            )
            page_raw = page_raw.merge(raw)
            page_base = page_base.merge(folded)
            per_script_raw[script] = per_script_raw.get(script, CerCounts()).merge(raw)
            per_script_base[script] = per_script_base.get(
                script, CerCounts()
            ).merge(folded)

            reference_lines = len([line for line in region.lines if line.text])
            page_missing += max(0, reference_lines - len(hypothesis_lines))
            page_extra += max(0, len(hypothesis_lines) - reference_lines)

        raw_total = raw_total.merge(page_raw)
        base_total = base_total.merge(page_base)
        missing_lines += page_missing
        extra_lines += page_extra

        per_page.append(
            {
                "page_id": page.name,
                "transcribed_regions": transcribed_regions,
                "reference_chars": page_raw.reference_chars,
                "polytonic_cer": round(page_raw.cer, 6),
                "base_letter_cer": round(page_base.cer, 6),
                "missing_lines": page_missing,
                "extra_lines": page_extra,
            }
        )

    return {
        "pages_attempted": len(pages),
        "pages_succeeded": len(pages) - len(failures),
        "failures": failures,
        "seconds_total": round(sum(timings.values()), 3),
        "seconds_per_page": round(
            sum(timings.values()) / max(1, len(pages)), 3
        ),
        "missing_lines": missing_lines,
        "extra_lines": extra_lines,
        "polytonic": raw_total.as_dict(),
        "base_letter": base_total.as_dict(),
        "by_script": {
            script: {
                "polytonic": per_script_raw[script].as_dict(),
                "base_letter": per_script_base[script].as_dict(),
            }
            for script in sorted(per_script_raw)
        },
        "per_page": per_page,
    }


def _score_normal_form(text: str) -> str:
    return " ".join(base_letter_text(text).split())


def correct_base_rewrite_stats(
    reference: str, before: str, after: str
) -> dict[str, int]:
    """Counts changed base letters that were exactly correct before repair.

    Equal blocks in a deterministic sequence alignment identify hypothesis
    base letters that exactly matched the reference.  Because the enhancer's
    repairs are one-to-one, a change at one of those positions is a rewrite of
    an already-correct base.  The denominator is all exactly correct baseline
    base letters, making the reported percentage an error rate, not a count
    divided by a hand-picked set of rewrites.
    """
    reference_base = _score_normal_form(reference)
    before_base = _score_normal_form(before)
    after_base = _score_normal_form(after)
    if len(before_base) != len(after_base):
        raise BakeoffError("enhancer changed base-letter string length")

    correct_before: set[int] = set()
    matcher = difflib.SequenceMatcher(
        None, reference_base, before_base, autojunk=False
    )
    for block in matcher.get_matching_blocks():
        for offset in range(block.size):
            hypothesis_index = block.b + offset
            if before_base[hypothesis_index].isalpha():
                correct_before.add(hypothesis_index)

    changed = {
        index
        for index, (old, new) in enumerate(zip(before_base, after_base))
        if old != new and (old.isalpha() or new.isalpha())
    }
    return {
        "correct_base_letters_before": len(correct_before),
        "base_changing_rewrites": len(changed),
        "correct_base_rewrites": len(changed & correct_before),
    }


def aggregate_rewrite_safety(
    pages: list[cgpg.Page],
    baseline: dict[str, dict | None],
    enhanced: dict[str, dict | None],
) -> dict[str, int | float]:
    totals = {
        "correct_base_letters_before": 0,
        "base_changing_rewrites": 0,
        "correct_base_rewrites": 0,
    }
    for page in pages:
        before_response = baseline.get(page.name)
        after_response = enhanced.get(page.name)
        if before_response is None or after_response is None:
            continue
        for region in page.regions:
            if not region.text.strip():
                continue
            before = "\n".join(run_cgpg.lines_in_region(before_response, region))
            after = "\n".join(run_cgpg.lines_in_region(after_response, region))
            result = correct_base_rewrite_stats(region.text, before, after)
            for key in totals:
                totals[key] += result[key]
    denominator = totals["correct_base_letters_before"]
    totals["correct_base_rewrite_rate"] = (
        totals["correct_base_rewrites"] / denominator if denominator else 0.0
    )
    totals["correct_base_rewrite_percent"] = round(
        100.0 * totals["correct_base_rewrite_rate"], 6
    )
    return totals


def aggregate_lineage(audits: list[dict[str, int | float]]) -> dict:
    summed = {
        "source_tokens": sum(int(item["source_tokens"]) for item in audits),
        "output_tokens": sum(int(item["output_tokens"]) for item in audits),
        "token_lineage_preserved": sum(
            int(item["token_lineage_preserved"]) for item in audits
        ),
        "box_lineage_preserved": sum(
            int(item["box_lineage_preserved"]) for item in audits
        ),
        "lookalike_rewrites": sum(int(item["lookalike_rewrites"]) for item in audits),
        "nfc_normalized_tokens": sum(
            int(item["nfc_normalized_tokens"]) for item in audits
        ),
    }
    denominator = summed["source_tokens"]
    summed["token_lineage_percent"] = round(
        100.0 * summed["token_lineage_preserved"] / denominator
        if denominator else 100.0,
        6,
    )
    summed["box_lineage_percent"] = round(
        100.0 * summed["box_lineage_preserved"] / denominator
        if denominator else 100.0,
        6,
    )
    return summed


def exact_candidate_cer(candidate: dict, view: str) -> float:
    metric = candidate[view]
    denominator = metric["reference_chars"]
    return metric["char_edits"] / denominator if denominator else 0.0


def comparisons_to_ell(evaluations: dict[str, dict]) -> dict[str, dict[str, float]]:
    """Computes all displayed deltas from unrounded integer metric counts."""
    baseline = evaluations["ell"]
    baseline_polytonic = exact_candidate_cer(baseline, "polytonic")
    baseline_base = exact_candidate_cer(baseline, "base_letter")
    comparisons: dict[str, dict[str, float]] = {}
    for name in ("ell+enhancer", "grc", "ell+grc"):
        candidate_polytonic = exact_candidate_cer(evaluations[name], "polytonic")
        candidate_base = exact_candidate_cer(evaluations[name], "base_letter")
        relative_improvement = (
            (baseline_polytonic - candidate_polytonic)
            / baseline_polytonic
            * 100.0
            if baseline_polytonic
            else 0.0
        )
        comparisons[name] = {
            "polytonic_cer_delta_percentage_points": round(
                (candidate_polytonic - baseline_polytonic) * 100.0, 6
            ),
            "polytonic_cer_relative_improvement_percent": round(
                relative_improvement, 6
            ),
            "base_letter_cer_delta_percentage_points": round(
                (candidate_base - baseline_base) * 100.0, 6
            ),
        }
    return comparisons


def evaluate_gate(
    baseline: dict,
    enhanced: dict,
    rewrite_safety: dict,
    lineage: dict,
) -> dict:
    # Gate on integer edit/reference counts, never on the rounded display CER.
    baseline_polytonic_cer = exact_candidate_cer(baseline, "polytonic")
    enhanced_polytonic_cer = exact_candidate_cer(enhanced, "polytonic")
    if baseline_polytonic_cer:
        improvement_percent = (
            (baseline_polytonic_cer - enhanced_polytonic_cer)
            / baseline_polytonic_cer
            * 100.0
        )
    else:
        improvement_percent = 0.0
    base_degradation_pp = (
        exact_candidate_cer(enhanced, "base_letter")
        - exact_candidate_cer(baseline, "base_letter")
    ) * 100.0
    correct_rewrite_percent = rewrite_safety["correct_base_rewrite_percent"]

    checks = {
        "polytonic_cer_improvement": {
            "operator": ">=",
            "threshold_percent": MIN_POLYTONIC_CER_IMPROVEMENT_PERCENT,
            "actual_percent": round(improvement_percent, 6),
            "passed": improvement_percent
            >= MIN_POLYTONIC_CER_IMPROVEMENT_PERCENT,
        },
        "base_letter_cer_degradation": {
            "operator": "<=",
            "threshold_percentage_points": (
                MAX_BASE_LETTER_CER_DEGRADATION_PERCENTAGE_POINTS
            ),
            "actual_percentage_points": round(base_degradation_pp, 6),
            "passed": base_degradation_pp
            <= MAX_BASE_LETTER_CER_DEGRADATION_PERCENTAGE_POINTS,
        },
        "correct_base_rewrite": {
            "operator": "<=",
            "threshold_percent": MAX_CORRECT_BASE_REWRITE_PERCENT,
            "actual_percent": correct_rewrite_percent,
            "numerator": rewrite_safety["correct_base_rewrites"],
            "denominator": rewrite_safety["correct_base_letters_before"],
            "passed": correct_rewrite_percent <= MAX_CORRECT_BASE_REWRITE_PERCENT,
        },
        "token_lineage": {
            "operator": "==",
            "threshold_percent": REQUIRED_LINEAGE_PERCENT,
            "actual_percent": lineage["token_lineage_percent"],
            "passed": lineage["token_lineage_percent"]
            == REQUIRED_LINEAGE_PERCENT,
        },
        "box_lineage": {
            "operator": "==",
            "threshold_percent": REQUIRED_LINEAGE_PERCENT,
            "actual_percent": lineage["box_lineage_percent"],
            "passed": lineage["box_lineage_percent"]
            == REQUIRED_LINEAGE_PERCENT,
        },
        "all_pages_completed": {
            "operator": "==",
            "threshold_pages": SCORABLE_PAGE_COUNT,
            "actual_pages": min(
                baseline["pages_succeeded"], enhanced["pages_succeeded"]
            ),
            "passed": (
                baseline["pages_succeeded"] == SCORABLE_PAGE_COUNT
                and enhanced["pages_succeeded"] == SCORABLE_PAGE_COUNT
            ),
        },
    }
    return {
        "outcome": "pass" if all(item["passed"] for item in checks.values()) else "fail",
        "checks": checks,
    }


def load_model_provenance(tessdata: Path) -> tuple[dict, dict[str, dict]]:
    manifest_path = tessdata / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BakeoffError("tessdata manifest is missing or invalid") from error
    records = {item["language"]: item for item in manifest.get("models", [])}
    selected: dict[str, dict] = {}
    for language in ("ell", "grc"):
        record = records.get(language)
        if record is None:
            raise BakeoffError(f"manifest does not declare {language}.traineddata")
        path = tessdata / record["filename"]
        actual_digest = sha256_file(path)
        if actual_digest != record.get("sha256"):
            raise BakeoffError(f"{record['filename']} does not match its manifest")
        selected[language] = {
            "filename": record["filename"],
            "sha256": actual_digest,
            "size_bytes": path.stat().st_size,
            "model_set": manifest.get("model_set"),
            "model_set_version": manifest.get("model_set_version"),
            "license": record.get("license"),
        }
    return manifest, selected


def tesseract_version(binary: Path) -> str:
    try:
        completed = subprocess.run(
            [str(binary), "--version"], capture_output=True, timeout=30, check=False
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise BakeoffError("bundled Tesseract is not executable") from error
    first = completed.stdout.decode("utf-8", errors="replace").splitlines()
    if completed.returncode != 0 or not first:
        raise BakeoffError("bundled Tesseract did not report a version")
    return first[0].strip()


def candidate_provenance(
    binary_digest: str,
    models: dict[str, dict],
    enhancer_digest: str,
    shipped_enhancer_digest: str,
    tessdata: Path,
    binary: Path,
) -> dict[str, dict]:
    records: dict[str, dict] = {}
    for name in CANDIDATE_ORDER:
        languages = "ell" if name == "ell+enhancer" else name
        language_parts = languages.split("+")
        model_records = [models[language] for language in language_parts]
        enhancement = None
        if name == "ell+enhancer":
            enhancement = {
                "enhancer_id": enhancer.ALGORITHM_ID,
                "enhancer_version": enhancer.ALGORITHM_VERSION,
                "minimum_confidence": enhancer.MINIMUM_CONFIDENCE,
                "reference_mirror_source_sha256": enhancer_digest,
                "shipped_rust_source_sha256": shipped_enhancer_digest,
                "mapping_sha256": canonical_digest(enhancer.LATIN_TO_GREEK),
            }
        digest_input = {
            "engine_binary_sha256": binary_digest,
            "languages": languages,
            "model_sha256": [record["sha256"] for record in model_records],
            "psm": PSM,
            "oem": OEM,
            "preserve_interword_spaces": 1,
            "tessedit_create_tsv": 1,
            "enhancement": enhancement,
        }
        records[name] = {
            "candidate_sha256": canonical_digest(digest_input),
            "languages": languages,
            "models": model_records,
            "enhancement": enhancement,
            "tesseract_argv_template": tesseract_argv(
                Path("<TESSERACT_BIN>"),
                "<CGPG_PAGE_IMAGE>",
                Path("<TESSDATA_DIR>"),
                languages,
            ),
        }
    return records


def reproducible_harness_argv() -> list[str]:
    """Portable command template recorded in evidence instead of host paths."""
    return [
        "python3",
        "scripts/ocr/gold/run_cgpg_model_bakeoff.py",
        "--corpus",
        "<CGPG_DATA_DIR>",
        "--tesseract",
        "<TESSERACT_BIN>",
        "--tessdata",
        "<TESSDATA_DIR>",
        "--json",
        "docs/evidence/ocr-cgpg-greek-model-bakeoff-2026-08-30.json",
    ]


def _code_digests() -> dict[str, str]:
    root = Path(__file__).resolve().parent
    paths = {
        "scripts/ocr/gold/cgpg.py": root / "cgpg.py",
        "scripts/ocr/gold/metrics.py": root / "metrics.py",
        "scripts/ocr/gold/run_cgpg.py": root / "run_cgpg.py",
        "scripts/ocr/gold/greek_text_enhancer.py": (
            root / "greek_text_enhancer.py"
        ),
        "scripts/ocr/gold/run_cgpg_model_bakeoff.py": Path(__file__).resolve(),
        "crates/mpdf-core/src/ocr_provider/text_enhancer.rs": (
            SHIPPED_RUST_ENHANCER
        ),
    }
    return {name: sha256_file(path) for name, path in paths.items()}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--tesseract", required=True, type=Path)
    parser.add_argument("--tessdata", required=True, type=Path)
    parser.add_argument("--json", required=True, type=Path)
    args = parser.parse_args()

    if not args.tesseract.is_file():
        raise BakeoffError("bundled Tesseract binary is missing")
    if not args.tessdata.is_dir():
        raise BakeoffError("bundled tessdata directory is missing")

    all_pages = cgpg.load_corpus(args.corpus)
    development_pages, holdout_pages = cgpg.split_corpus(all_pages)
    scorable_total = sum(bool(page.transcribed_lines) for page in all_pages)
    holdout_scorable_total = sum(bool(page.transcribed_lines) for page in holdout_pages)
    pages = select_fixed_pages(all_pages)
    manifest, models = load_model_provenance(args.tessdata)
    binary_digest = sha256_file(args.tesseract)
    enhancer_digest = sha256_file(Path(enhancer.__file__).resolve())
    if not SHIPPED_RUST_ENHANCER.is_file():
        raise BakeoffError("shipped Rust enhancer implementation is missing")
    shipped_enhancer_digest = sha256_file(SHIPPED_RUST_ENHANCER)
    provenance = candidate_provenance(
        binary_digest,
        models,
        enhancer_digest,
        shipped_enhancer_digest,
        args.tessdata,
        args.tesseract,
    )

    outputs: dict[str, dict[str, dict | None]] = {
        name: {} for name in CANDIDATE_ORDER
    }
    timings: dict[str, dict[str, float]] = {name: {} for name in CANDIDATE_ORDER}
    invocation_failures: dict[str, list[dict[str, str]]] = {
        name: [] for name in CANDIDATE_ORDER
    }
    audits: list[dict[str, int | float]] = []
    enhancer_seconds = 0.0

    for page in pages:
        for name, languages in RAW_CANDIDATES:
            try:
                invocation = run_tesseract(
                    args.tesseract, page.image_path, args.tessdata, languages
                )
            except BakeoffError as error:
                outputs[name][page.name] = None
                timings[name][page.name] = 0.0
                invocation_failures[name].append(
                    {"page_id": page.name, "reason": str(error)}
                )
                print(f"  ! {name:12s} {page.name}: {error}", file=sys.stderr)
                continue
            outputs[name][page.name] = invocation.response
            timings[name][page.name] = invocation.seconds
            print(
                f"  {name:12s} {page.name:34s} {invocation.seconds:6.2f}s",
                file=sys.stderr,
                flush=True,
            )

        ell_response = outputs["ell"].get(page.name)
        if ell_response is None:
            outputs["ell+enhancer"][page.name] = None
            timings["ell+enhancer"][page.name] = timings["ell"].get(page.name, 0.0)
            invocation_failures["ell+enhancer"].append(
                {"page_id": page.name, "reason": "ell baseline failed"}
            )
        else:
            started = time.perf_counter()
            enhanced_response, audit = enhance_response(ell_response)
            enhancer_elapsed = time.perf_counter() - started
            enhancer_seconds += enhancer_elapsed
            outputs["ell+enhancer"][page.name] = enhanced_response
            timings["ell+enhancer"][page.name] = (
                timings["ell"][page.name] + enhancer_elapsed
            )
            audits.append(audit)

    evaluations: dict[str, dict] = {}
    for name in CANDIDATE_ORDER:
        evaluation = score_outputs(pages, outputs[name], timings[name])
        if invocation_failures[name]:
            evaluation["invocation_failures"] = invocation_failures[name]
        evaluations[name] = {**provenance[name], **evaluation}

    lineage = aggregate_lineage(audits)
    rewrite_safety = aggregate_rewrite_safety(
        pages, outputs["ell"], outputs["ell+enhancer"]
    )
    evaluations["ell+enhancer"]["enhancer_seconds_total"] = round(
        enhancer_seconds, 6
    )
    evaluations["ell+enhancer"]["enhancer_audit"] = {
        **lineage,
        **rewrite_safety,
    }
    gate = evaluate_gate(
        evaluations["ell"],
        evaluations["ell+enhancer"],
        rewrite_safety,
        lineage,
    )

    report = {
        "schema": "mpdf-cgpg-greek-model-bakeoff",
        "schema_version": "1.0",
        "date": date.today().isoformat(),
        "content_policy": (
            "Metrics and provenance only; no CGPG transcription, OCR text, "
            "page image, page image path, corpus directory, or local runtime "
            "path is serialized."
        ),
        "host": {
            "platform": platform.platform(),
            "machine": platform.machine(),
            "python": platform.python_version(),
        },
        "corpus": {
            "name": "Corpus of Ground-truthed Patrologia Graeca",
            "doi": CORPUS_DOI,
            "archive_sha256": CORPUS_ARCHIVE_SHA256,
            "license": CORPUS_LICENSE,
            "pages_in_archive": len(all_pages),
            "scorable_pages_under_loader": scorable_total,
            "development_pages": len(development_pages),
            "holdout_pages": len(holdout_pages),
            "scorable_holdout_pages": holdout_scorable_total,
            "selection_rule": (
                "lexicographically first 12 pages with at least one non-empty "
                "transcribed line from cgpg.split_corpus's deterministic "
                "holdout split"
            ),
            "page_ids": [page.name for page in pages],
        },
        "runtime": {
            "tesseract_version": tesseract_version(args.tesseract),
            "binary_sha256": binary_digest,
            "binary_size_bytes": args.tesseract.stat().st_size,
            "tessdata_manifest_sha256": sha256_file(
                args.tessdata / "manifest.json"
            ),
            "model_set": manifest.get("model_set"),
            "model_set_version": manifest.get("model_set_version"),
            "model_license": manifest.get("license"),
            "environment": {
                "TESSDATA_PREFIX": "<TESSDATA_DIR>",
                "OMP_THREAD_LIMIT": "1",
            },
        },
        "pre_registration": {
            "registered_before_candidate_execution": True,
            "page_count": SCORABLE_PAGE_COUNT,
            "psm": PSM,
            "oem": OEM,
            "thresholds": {
                "ell_plus_enhancer_polytonic_cer_improvement_percent_min": (
                    MIN_POLYTONIC_CER_IMPROVEMENT_PERCENT
                ),
                "base_letter_cer_degradation_percentage_points_max": (
                    MAX_BASE_LETTER_CER_DEGRADATION_PERCENTAGE_POINTS
                ),
                "correct_base_rewrite_percent_max": (
                    MAX_CORRECT_BASE_REWRITE_PERCENT
                ),
                "token_lineage_percent_required": REQUIRED_LINEAGE_PERCENT,
                "box_lineage_percent_required": REQUIRED_LINEAGE_PERCENT,
            },
            "thresholds_are_not_cli_options": True,
        },
        "metric_definitions": {
            "polytonic_cer": (
                "Unicode character Levenshtein edits divided by reference "
                "characters after NFC and whitespace normalization; all "
                "polytonic marks remain significant"
            ),
            "base_letter_cer": (
                "the same CER after NFD decomposition and removal of all "
                "combining marks; case and script remain significant"
            ),
            "correct_base_rewrite_percent": (
                "enhancer-changed base-letter positions that exactly matched "
                "the reference before enhancement, divided by all exactly "
                "matching baseline base-letter positions"
            ),
            "token_lineage_percent": (
                "ordered enhanced token lineage IDs exactly equal to the "
                "ordered ell-source IDs, divided by the ell-source token count; "
                "a token-count difference preserves zero tokens"
            ),
            "box_lineage_percent": (
                "ordered (lineage ID, x, y, width, height) tuples exactly equal "
                "between enhanced and ell-source responses, divided by the "
                "ell-source token count; a token-count difference preserves "
                "zero boxes"
            ),
            "aggregation": (
                "micro-average from summed integer edit and reference counts "
                "across reference-defined PAGE regions"
            ),
            "gate_precision": (
                "checks use unrounded integer edit/reference counts; JSON CER "
                "values are rounded to six decimal places for display"
            ),
        },
        "enhancer_contract": {
            "input": (
                "OCR token text plus that token's normalized Tesseract word "
                "confidence only"
            ),
            "normalization": "NFC",
            "enhancer_id": enhancer.ALGORITHM_ID,
            "enhancer_version": enhancer.ALGORITHM_VERSION,
            "minimum_confidence": enhancer.MINIMUM_CONFIDENCE,
            "confidence_normalization": (
                "Tesseract TSV conf is parsed as a decimal and divided by "
                "100.0, then clamped to [0.0, 1.0]; the candidate requires "
                "normalized confidence >= 0.95"
            ),
            "token_predicate": (
                "after NFC, at least one character is in Greek/Greek Extended "
                "and every alphabetic character is Greek or a key in the "
                "fixed Latin-lookalike mapping"
            ),
            "eligible_position": "every position in an eligible token, including edges",
            "repair": (
                "fixed Latin-lookalike to Greek mapping, one code point to "
                "one code point"
            ),
            "mapping": enhancer.LATIN_TO_GREEK,
            "mapping_sha256": canonical_digest(enhancer.LATIN_TO_GREEK),
            "reference_mirror_source_sha256": enhancer_digest,
            "shipped_rust_source_sha256": shipped_enhancer_digest,
            "excluded_inputs": [
                "page image",
                "token geometry",
                "other tokens",
                "ground truth",
                "dictionary",
                "language model",
                "sample IDs",
                "sample-specific rules",
            ],
        },
        "execution": {
            "harness_argv": reproducible_harness_argv(),
            "levenshtein_backend": LEVENSHTEIN_BACKEND,
            "candidate_environment": {
                "TESSDATA_PREFIX": "<TESSDATA_DIR>",
                "OMP_THREAD_LIMIT": "1",
            },
            "candidate_invocation_note": (
                "Replace <CGPG_PAGE_IMAGE> with the image paired to each "
                "page_id; ell+enhancer reuses the ell TSV and performs no "
                "second OCR invocation."
            ),
            "evaluated_code_sha256": _code_digests(),
        },
        "candidates": evaluations,
        "comparisons_to_ell": comparisons_to_ell(evaluations),
        "gate": gate,
        "limitations": [
            (
                "The public CGPG package is not the paper's 30-page test "
                "split; these metrics must not be compared with the paper's "
                "headline accuracy."
            ),
            (
                "The fixed slice is the first 12 scorable pages in the "
                "repository's deterministic holdout split, not a random or "
                "blinded sample, and all scored regions in this slice are "
                "reference-labelled Greek."
            ),
            (
                "The slice comes from one corpus and typography family; it "
                "does not establish generalization to other editions."
            ),
            (
                "CGPG provides no clean/degraded condition label used by this "
                "harness, so the fixed slice cannot report separate clean and "
                "degraded strata or attribute errors to degradation severity."
            ),
            (
                "Latin-language safety is represented only by the "
                "correct-base rewrite and lineage gates because this slice "
                "contains no reference-labelled Latin regions."
            ),
            (
                "The enhancer cannot restore missing accents or infer words; "
                "it can only normalize NFC and repair the pre-registered "
                "internal lookalike set."
            ),
        ],
    }

    args.json.parent.mkdir(parents=True, exist_ok=True)
    serialized = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    args.json.write_text(serialized, encoding="utf-8")
    print(serialized, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
