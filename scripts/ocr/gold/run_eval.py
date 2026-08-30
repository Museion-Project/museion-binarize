#!/usr/bin/env python3
"""Runs OCR candidates over the gold corpus and scores them.

This is the only gate that may promote an engine/model/profile to a production
default.  It talks to candidates through the exact same argv + stdin protocol
the Rust runner uses, so a candidate that scores here is a candidate the
application can actually run -- not a library call that happens to work in a
notebook.

Usage::

    python3 scripts/ocr/gold/run_eval.py \\
        --work-dir /tmp/gold \\
        --candidate tesseract-auto:tesseract:auto:/path/to/tessdata_best \\
        --candidate tesseract-eng:tesseract:english:/path/to/tessdata_best \\
        --json out.json

A candidate is ``name:engine:language_profile:model_dir[:extra=value,...]``.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import corpus  # noqa: E402
import fixtures  # noqa: E402
import metrics as metrics_module  # noqa: E402

SIDECAR = Path(__file__).resolve().parents[1] / "mpdf_ocr_sidecar.py"
PROTOCOL = "mpdf-ocr"
PROTOCOL_VERSION = "0.1"


@dataclass
class Candidate:
    name: str
    engine: str
    language_profile: str
    model_dir: Path
    extra: dict[str, str]

    @staticmethod
    def parse(spec: str) -> "Candidate":
        parts = spec.split(":")
        if len(parts) < 4:
            raise ValueError(f"malformed candidate: {spec!r}")
        name, engine, profile = parts[0], parts[1], parts[2]
        # A model dir may itself contain ':' only if quoted by the shell; the
        # remaining segments after the 4th are key=value extras.
        model_dir = parts[3]
        extra: dict[str, str] = {}
        for chunk in parts[4:]:
            for item in chunk.split(","):
                if "=" in item:
                    key, value = item.split("=", 1)
                    extra[key] = value
        return Candidate(name, engine, profile, Path(model_dir), extra)


@dataclass
class PageResult:
    sample_id: str
    text: str
    lines: list[str]
    elapsed_seconds: float
    unavailable: bool = False
    failure: str | None = None
    parameters: dict | None = None
    engine: str = ""
    model: str = ""
    version: str = ""
    #: Recognized rows as (title, trailing page number) for TOC scoring.
    row_pairs: list[tuple[str, str]] | None = None


def _invoke(candidate: Candidate, image: Path, page_index: int) -> tuple[dict | None, str | None, float]:
    request = json.dumps(
        {
            "protocol": PROTOCOL,
            "protocol_version": PROTOCOL_VERSION,
            "page_index": page_index,
            # The gold harness is not a package writer; the digest binding is
            # exercised by the Rust integration tests, so a placeholder that
            # satisfies the shape is enough here.
            "input_asset_sha256": "0" * 64,
            "language_profile": candidate.language_profile,
        }
    )
    argv = [
        sys.executable,
        str(SIDECAR),
        "--protocol",
        PROTOCOL,
        "--protocol-version",
        PROTOCOL_VERSION,
        "--model-dir",
        str(candidate.model_dir),
        "--input",
        str(image),
        "--engine",
        candidate.engine,
        "--language-profile",
        candidate.language_profile,
    ]
    for key, value in candidate.extra.items():
        argv.extend([f"--{key.replace('_', '-')}", value])
    started = time.perf_counter()
    completed = subprocess.run(
        argv, input=request + "\n", capture_output=True, text=True, timeout=300, check=False
    )
    elapsed = time.perf_counter() - started
    if completed.returncode == 78:
        return None, "unavailable", elapsed
    if completed.returncode != 0:
        return None, completed.stderr.strip() or "provider failed", elapsed
    line = completed.stdout.strip().splitlines()
    if not line:
        return None, "no response", elapsed
    return json.loads(line[0]), None, elapsed


def _lines_from(response: dict) -> list[str]:
    """Flattens the typed response into recognized logical lines, in order."""
    rows: list[tuple[int, int, str]] = []
    for block in response.get("blocks", []):
        for line in block.get("lines", []):
            text = " ".join(word["text"] for word in line.get("words", []))
            if text.strip():
                rows.append((block["reading_order"], line["reading_order"], text))
    rows.sort(key=lambda row: (row[0], row[1]))
    return [row[2] for row in rows]


def run_candidate(candidate: Candidate, samples, images: dict[str, Path]) -> list[PageResult]:
    results: list[PageResult] = []
    for index, sample in enumerate(samples):
        response, failure, elapsed = _invoke(candidate, images[sample.sample_id], index)
        if response is None:
            results.append(
                PageResult(
                    sample_id=sample.sample_id,
                    text="",
                    lines=[],
                    elapsed_seconds=elapsed,
                    unavailable=failure == "unavailable",
                    failure=failure,
                )
            )
            continue
        lines = _lines_from(response)
        results.append(
            PageResult(
                sample_id=sample.sample_id,
                text="\n".join(lines),
                lines=lines,
                elapsed_seconds=elapsed,
                parameters=response.get("parameters"),
                engine=response.get("engine", ""),
                model=response.get("model", ""),
                version=response.get("version", ""),
            )
        )
    return results


def score_candidate(candidate: Candidate, samples, results: list[PageResult]) -> dict:
    by_id = {result.sample_id: result for result in results}
    per_script: dict[str, metrics_module.Metrics] = {}
    per_sample = []
    must_pass_failures: list[str] = []
    total_seconds = 0.0
    unavailable = 0

    for sample in samples:
        result = by_id[sample.sample_id]
        total_seconds += result.elapsed_seconds
        if result.unavailable or result.failure:
            unavailable += 1
            if sample.must_pass:
                must_pass_failures.append(
                    f"{sample.sample_id}: provider {result.failure or 'unavailable'}"
                )
            per_sample.append(
                {
                    "sample_id": sample.sample_id,
                    "script": sample.script,
                    "failure": result.failure or "unavailable",
                }
            )
            continue

        scored = metrics_module.score(sample.text, result.text)
        bucket = per_script.setdefault(sample.script, metrics_module.Metrics())
        per_script[sample.script] = bucket.merge(scored)

        recognized = metrics_module.nfc(result.text)
        missing = [
            token for token in sample.must_contain
            if metrics_module.nfc(token) not in recognized
        ]
        leaked = [
            token for token in sample.must_not_contain
            if metrics_module.nfc(token) in recognized
        ]
        toc = _score_toc(sample, result) if sample.kind == "toc" else None
        sample_failed = bool(missing or leaked) or (
            toc is not None and toc["rows_assembled"] < toc["rows_expected"]
        )
        if sample.must_pass and sample_failed:
            detail = []
            if missing:
                detail.append(f"missing {missing}")
            if leaked:
                detail.append(f"leaked {leaked}")
            if toc is not None and toc["rows_assembled"] < toc["rows_expected"]:
                detail.append(
                    f"toc rows assembled {toc['rows_assembled']}/{toc['rows_expected']}"
                )
            must_pass_failures.append(f"{sample.sample_id}: {'; '.join(detail)}")

        entry = {
            "sample_id": sample.sample_id,
            "script": sample.script,
            "kind": sample.kind,
            "must_pass": sample.must_pass,
            "missing_required": missing,
            "leaked_forbidden": leaked,
            "seconds": round(result.elapsed_seconds, 3),
            **scored.as_dict(),
        }
        if toc is not None:
            entry["toc"] = toc
        if scored.diacritic_error_examples:
            entry["diacritic_examples"] = scored.diacritic_error_examples[:8]
        if scored.script_confusion_examples:
            entry["script_confusion_examples"] = scored.script_confusion_examples[:8]
        per_sample.append(entry)

    overall = metrics_module.aggregate(per_script.values())
    sample_result = next(
        (by_id[s.sample_id] for s in samples if not by_id[s.sample_id].unavailable
         and not by_id[s.sample_id].failure),
        None,
    )
    return {
        "candidate": candidate.name,
        "engine": candidate.engine,
        "language_profile": candidate.language_profile,
        "model_dir": str(candidate.model_dir),
        "engine_reported": sample_result.engine if sample_result else "",
        "model_reported": sample_result.model if sample_result else "",
        "version_reported": sample_result.version if sample_result else "",
        "model_license": (sample_result.parameters or {}).get("model_license", "")
        if sample_result
        else "",
        "word_segmentation": (sample_result.parameters or {}).get("word_segmentation", "")
        if sample_result
        else "",
        "unavailable_samples": unavailable,
        "seconds_total": round(total_seconds, 3),
        "seconds_per_page": round(total_seconds / max(1, len(samples)), 3),
        "overall": overall.as_dict(),
        "by_script": {name: value.as_dict() for name, value in sorted(per_script.items())},
        "must_pass_failures": must_pass_failures,
        "gate": "pass" if not must_pass_failures else "fail",
        "samples": per_sample,
    }


_TRAILING_NUMBER = re.compile(r"([0-9]+|[ivxlcdmIVXLCDM]+)[.\s]*$")


def _score_toc(sample, result: PageResult) -> dict:
    """Two separate printed-contents facts, deliberately not merged.

    ``rows_assembled`` — the row's title text and *a* trailing page-number
    token came back on the same recognized line.  This is precisely what the
    "one detector rectangle published as a block, a line, and a word" defect
    destroys, and it is what the must-pass gate checks.

    ``rows_exact`` — that trailing token also matches the printed number.
    Reported as a quality metric, not gated: a misread numeral is caught
    downstream by the bookmark engine's printed-page mapping, whereas a row
    that never became a row cannot be caught by anything.
    """
    assembled = 0
    exact = 0
    detail = []
    for title, page_number in sample.toc_rows:
        title_key = metrics_module.nfc(title).split()[0].lower()
        hit = None
        for line in result.lines:
            normalized = metrics_module.nfc(line).rstrip()
            if title_key in normalized.lower() and _TRAILING_NUMBER.search(normalized):
                hit = normalized
                break
        row_exact = False
        if hit is not None:
            assembled += 1
            match = _TRAILING_NUMBER.search(hit)
            row_exact = bool(match) and match.group(1) == page_number
            exact += int(row_exact)
        detail.append(
            {
                "title": title,
                "page": page_number,
                "assembled": hit is not None,
                "exact": row_exact,
            }
        )
    return {
        "rows_expected": len(sample.toc_rows),
        "rows_assembled": assembled,
        "rows_exact": exact,
        "rows": detail,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work-dir", required=True, type=Path)
    parser.add_argument("--candidate", action="append", required=True)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--scripts", nargs="*", default=None)
    args = parser.parse_args()

    samples = corpus.samples_for(tuple(args.scripts) if args.scripts else None)
    images = fixtures.render_all(samples, args.work_dir / "fixtures")

    report = {
        "schema": "mpdf-ocr-gold-eval",
        "schema_version": "1.0",
        "corpus_samples": len(samples),
        "must_pass_samples": sum(1 for sample in samples if sample.must_pass),
        "fixture_digests": {
            sample_id: fixtures.fixture_digest(path)
            for sample_id, path in sorted(images.items())
        },
        "candidates": [],
    }
    for spec in args.candidate:
        candidate = Candidate.parse(spec)
        results = run_candidate(candidate, samples, images)
        report["candidates"].append(score_candidate(candidate, samples, results))

    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(
            json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )

    _print_table(report)
    return 0 if any(c["gate"] == "pass" for c in report["candidates"]) else 1


def _print_table(report: dict) -> None:
    header = (
        f"{'candidate':<26} {'gate':<5} {'CER':>7} {'WER':>7} {'exactW':>7} "
        f"{'diacErr':>8} {'scriptCf':>9} {'NFC!':>5} {'s/pg':>6}"
    )
    print(header)
    print("-" * len(header))
    for entry in report["candidates"]:
        overall = entry["overall"]
        print(
            f"{entry['candidate']:<26} {entry['gate']:<5} "
            f"{overall['cer']:>7.4f} {overall['wer']:>7.4f} "
            f"{overall['exact_word_rate']:>7.4f} "
            f"{overall['diacritic_error_rate']:>8.4f} "
            f"{overall['script_confusion_rate']:>9.4f} "
            f"{overall['nfc_violations']:>5} {entry['seconds_per_page']:>6.2f}"
        )
    print()
    for entry in report["candidates"]:
        if entry["must_pass_failures"]:
            print(f"{entry['candidate']} must-pass failures:")
            for failure in entry["must_pass_failures"]:
                print(f"  - {failure}")


if __name__ == "__main__":
    raise SystemExit(main())
