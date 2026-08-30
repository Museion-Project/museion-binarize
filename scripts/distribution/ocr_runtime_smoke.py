#!/usr/bin/env python3
"""Execute bundled CLI OCR and emit strict evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import verify_ocr_runtime  # noqa: E402

REQUIRED_CHECKS = ("runtime_structure", "runner_identity", "execution", "output_created",
                   "source_unchanged", "four_pages", "qpdf", "searchable_text")


def _sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _pages(path: Path) -> int | None:
    if not shutil.which("pdfinfo"):
        return None
    try:
        result = subprocess.run(["pdfinfo", str(path)], capture_output=True, text=True, check=True)
    except (OSError, subprocess.SubprocessError):
        return None
    for line in result.stdout.splitlines():
        if line.startswith("Pages:"):
            try:
                return int(line.split(":", 1)[1].strip())
            except ValueError:
                return None
    return None


def _runner(layout: Path) -> Path | None:
    for candidate in (layout / "mpdf", layout / "mpdf.exe", layout / "Contents/MacOS/mpdf"):
        if candidate.is_file() and not candidate.is_symlink() and os.access(candidate, os.X_OK):
            return candidate
    return None


def smoke(*, layout: Path, target: str, source_pdf: Path | None = None,
          output_pdf: Path | None = None, artifact: Path | None = None,
          pinned_models: Path | None = None, release: str = "0.1.0-rc.3",
          timeout_seconds: int = 240) -> dict:
    runtime = layout / "ocr-runtime"
    if not runtime.is_dir() and (layout / "Contents/Resources/ocr-runtime").is_dir():
        runtime = layout / "Contents/Resources/ocr-runtime"
    evidence: dict = {
        "schema": "mpdf-ocr-runtime-smoke-evidence", "schema_version": "1.0",
        "release": release, "target": target, "status": "blocked", "artifact": {},
        "runner": {}, "runtime": {}, "source": {}, "output": {}, "execution": {},
        "checks": {name: "not_run" for name in REQUIRED_CHECKS}, "reasons": [],
    }
    if artifact and artifact.is_file():
        evidence["artifact"] = {"path_basename": artifact.name, "sha256": _sha(artifact),
                                 "size_bytes": artifact.stat().st_size}
    else:
        evidence["reasons"].append("artifact_not_provided")
    runner = _runner(layout)
    if runner is None:
        evidence["reasons"].append("packaged_cli_runner_not_found")
    else:
        evidence["runner"] = {"path_basename": runner.name, "sha256": _sha(runner)}
        evidence["checks"]["runner_identity"] = "pass"
    if source_pdf is None or not source_pdf.is_file():
        evidence["reasons"].append("four_page_source_fixture_not_provided")
    elif _pages(source_pdf) != 4:
        evidence["reasons"].append("source_fixture_is_not_four_pages")
    else:
        evidence["source"] = {"path_basename": source_pdf.name, "sha256_before": _sha(source_pdf), "pages": 4}
    if not runtime.is_dir():
        evidence["reasons"].append("bundled_ocr_runtime_not_found")
    else:
        try:
            manifest_path = runtime / verify_ocr_runtime.RUNTIME_MANIFEST
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            verify_ocr_runtime.verify(runtime, target, pinned_models or verify_ocr_runtime.DEFAULT_PINNED_MODELS,
                                      expected_release=release)
            evidence["runtime"] = {
                "manifest_sha256": _sha(manifest_path), "engine": manifest.get("engine"),
                "engine_version": manifest.get("engine_version"), "model_set": manifest.get("model_set"),
                "model_set_version": manifest.get("model_set_version"),
                "engine_sha256": next(i["sha256"] for i in manifest["files"] if i.get("role") == "engine"),
                "sidecar_sha256": next(i["sha256"] for i in manifest["files"] if i.get("role") == "sidecar"),
                "model_sha256": {i["language"]: i["sha256"] for i in manifest["files"] if i.get("role") == "model"},
            }
            evidence["checks"]["runtime_structure"] = "pass"
        except (OSError, ValueError, KeyError, StopIteration, json.JSONDecodeError) as exc:
            evidence["reasons"].append(f"structure_failed:{exc}")
    if any(evidence["checks"][name] != "pass" for name in ("runner_identity", "runtime_structure")):
        return evidence
    if source_pdf is None or evidence["source"].get("pages") != 4:
        return evidence
    if not all(shutil.which(tool) for tool in ("pdfinfo", "qpdf", "pdftotext")):
        evidence["reasons"].append("pdfinfo_qpdf_pdftotext_required")
        return evidence
    if output_pdf is None:
        fd, generated = tempfile.mkstemp(prefix="mpdf-ocr-smoke-", suffix=".pdf")
        os.close(fd)
        Path(generated).unlink()
        output_pdf = Path(generated)
    if output_pdf.exists():
        evidence["reasons"].append("output_path_must_not_preexist")
        return evidence
    started = time.monotonic()
    # Do not pass sidecar/model overrides here: the purpose of the installed
    # smoke is to exercise the executable-adjacent bundled resolver, including
    # its binding to bin/tesseract via --engine-binary.
    argv = [str(runner), "run", str(source_pdf), "--output", str(output_pdf), "--overwrite",
            "--provider", "tesseract", "--on-review", "confirmed", "--language", "auto"]
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=timeout_seconds)
        duration = time.monotonic() - started
    except (OSError, subprocess.TimeoutExpired) as exc:
        evidence["execution"] = {"exit_status": None, "duration_seconds": time.monotonic() - started,
                              "protocol": "mpdf run + executable-adjacent bundled runtime"}
        evidence["reasons"].append(f"runner_failed:{type(exc).__name__}")
        return evidence
    evidence["execution"] = {"exit_status": result.returncode, "duration_seconds": round(duration, 3),
                              "protocol": "mpdf run + executable-adjacent bundled runtime",
                              "runner_sha256": evidence["runner"]["sha256"]}
    evidence["checks"]["execution"] = "pass" if result.returncode == 0 else "fail"
    evidence["checks"]["output_created"] = "pass" if output_pdf.is_file() else "fail"
    if not output_pdf.is_file():
        evidence["reasons"].append("runner_did_not_create_output")
        return evidence
    evidence["output"] = {"path_basename": output_pdf.name, "sha256": _sha(output_pdf),
                           "size_bytes": output_pdf.stat().st_size, "pages": _pages(output_pdf),
                           "created_by_harness": True, "preexisting": False}
    evidence["checks"]["source_unchanged"] = "pass" if _sha(source_pdf) == evidence["source"]["sha256_before"] else "fail"
    evidence["checks"]["four_pages"] = "pass" if evidence["output"]["pages"] == 4 else "fail"
    evidence["checks"]["qpdf"] = "pass" if subprocess.run(["qpdf", "--check", str(output_pdf)], capture_output=True).returncode == 0 else "fail"
    text_path = output_pdf.with_suffix(".txt")
    text_result = subprocess.run(["pdftotext", str(output_pdf), str(text_path)], capture_output=True)
    searchable = text_result.returncode == 0 and text_path.is_file() and bool(text_path.read_text(errors="ignore").strip())
    evidence["checks"]["searchable_text"] = "pass" if searchable else "fail"
    if all(evidence["checks"][name] == "pass" for name in REQUIRED_CHECKS):
        evidence["status"] = "pass"
    else:
        evidence["status"] = "not_run"
        evidence["reasons"].append("one_or_more_execution_checks_not_pass")
    evidence["source"]["sha256_after"] = _sha(source_pdf)
    return evidence


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--layout", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--source-pdf", type=Path)
    parser.add_argument("--output-pdf", type=Path)
    parser.add_argument("--artifact", type=Path)
    parser.add_argument("--pinned-models", type=Path)
    parser.add_argument("--release", default="0.1.0-rc.3")
    parser.add_argument("--timeout-seconds", type=int, default=240)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    evidence = smoke(layout=args.layout, target=args.target, source_pdf=args.source_pdf,
                     output_pdf=args.output_pdf, artifact=args.artifact,
                     pinned_models=args.pinned_models, release=args.release,
                     timeout_seconds=args.timeout_seconds)
    args.evidence.parent.mkdir(parents=True, exist_ok=True)
    args.evidence.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": evidence["status"], "evidence": str(args.evidence)}, sort_keys=True))
    return 0 if evidence["status"] == "pass" else 2


if __name__ == "__main__":
    raise SystemExit(main())
