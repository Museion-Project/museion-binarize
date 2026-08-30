#!/usr/bin/env python3
"""Run the frozen two-call Gemini truncation and geometry forensic probe."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
from dataclasses import replace
from datetime import date
from decimal import Decimal
from pathlib import Path
from typing import Mapping, Sequence

sys.path.insert(0, str(Path(__file__).resolve().parent))

import complete_ocr_coverage as coverage  # noqa: E402
import run_cloud_complete_ocr_bakeoff as common  # noqa: E402
import run_gemini_complete_ocr_primary as primary  # noqa: E402


SCHEMA = "mpdf-gemini-complete-ocr-forensic-probe"
SCHEMA_VERSION = "1.0"
TRUNCATION_PAGE_IDS = ("grc_grna_or_443", "grc_grna_or_503")
GEOMETRY_PAGE_IDS = ("grc_grna_or_399", "grc_grna_or_489")
PROBE_MAX_OUTPUT_TOKENS = 8192
PROBE_THINKING_LEVEL = "LOW"
PROBE_CALLS = 2
PLANNED_COST_USD = Decimal("0.0687810")
MAX_COST_USD = Decimal("0.07")
BASELINE_RAW_MANIFEST_SHA256 = (
    "b931ccf05124c20c27107cca4c7e5472defb911df6d362047e7814beb4ca0d9b"
)
BASELINE_CONFIGURATION_SHA256 = (
    "d826b29637d6abc0c2952b046d96dbc5ee5b37f0228c6978d410b1413965cb91"
)
UPLOAD_ACKNOWLEDGEMENT = (
    "I acknowledge two CGPG JPEG uploads to Google Gemini API for the frozen "
    "forensic probe."
)
DEFAULT_PREREGISTRATION = (
    common.REPO_ROOT
    / "docs/evidence/gemini-complete-ocr-forensic-probe-preregistration-2026-08-30.json"
)
PREREGISTRATION_SHA256 = (
    "7c5078e2874354a6025b704fecbe1a262c828deeac76c6fd8e542d626138ff60"
)


def probe_configuration() -> dict[str, object]:
    return {
        "provider": primary.PROVIDER,
        "backend": primary.BACKEND,
        "model": common.GEMINI_MODEL,
        "prompt_sha256": common.GEMINI_PROMPT_SHA256,
        "response_schema_sha256": common.canonical_digest(
            primary.GEMINI_API_RESPONSE_SCHEMA
        ),
        "coverage_contract_sha256": coverage.CONTRACT_SHA256,
        "formal_input_manifest_sha256": common.FORMAL_INPUT_MANIFEST_SHA256,
        "baseline_raw_manifest_sha256": BASELINE_RAW_MANIFEST_SHA256,
        "baseline_configuration_sha256": BASELINE_CONFIGURATION_SHA256,
        "truncation_page_ids": list(TRUNCATION_PAGE_IDS),
        "geometry_page_ids": list(GEOMETRY_PAGE_IDS),
        "live_calls": PROBE_CALLS,
        "repetitions_per_truncation_page": 1,
        "new_geometry_calls": 0,
        "max_output_tokens": PROBE_MAX_OUTPUT_TOKENS,
        "thinking_level": PROBE_THINKING_LEVEL,
        "automatic_retries": 0,
        "concurrency": 1,
        "planned_cost_usd": str(PLANNED_COST_USD),
        "hard_maximum_cost_usd": str(MAX_COST_USD),
        "production_provider_selection_permitted": False,
    }


def _code_paths() -> dict[str, Path]:
    return {
        "forensic_probe": Path(__file__).resolve(),
        "primary_runner": Path(primary.__file__).resolve(),
        "common_harness": Path(common.__file__).resolve(),
        "coverage_scorer": Path(coverage.__file__).resolve(),
        "cgpg_loader": Path(common.cgpg.__file__).resolve(),
    }


def verify_preregistration(
    path: Path, *, require_committed: bool = True
) -> common.RunBinding:
    raw = path.read_bytes()
    if common.sha256_bytes(raw) != PREREGISTRATION_SHA256:
        raise common.BakeoffError("probe_preregistration_digest_mismatch")
    value = common.strict_json_loads(raw, maximum_bytes=1024 * 1024)
    configuration_sha256 = common.canonical_digest(probe_configuration())
    try:
        valid = (
            isinstance(value, dict)
            and value["schema"]
            == "mpdf-gemini-complete-ocr-forensic-probe-preregistration"
            and value["schema_version"] == "1.0"
            and value["status"] == "preregistered_not_executed"
            and value["registered_before_live_execution"] is True
            and value["configuration_sha256"] == configuration_sha256
            and value["coverage_contract_sha256"]
            == coverage.CONTRACT_SHA256
            and value["design"]["live_calls"] == PROBE_CALLS
            and value["decision"]["provider_selection_permitted"] is False
        )
    except (KeyError, TypeError):
        valid = False
    if not valid:
        raise common.BakeoffError("probe_preregistration_invalid")
    paths = [path, coverage.DEFAULT_CONTRACT_PATH, *_code_paths().values()]
    if require_committed and not common._git_paths_are_committed(paths):
        raise common.BakeoffError("probe_preregistration_or_code_not_committed")
    return common.RunBinding(
        preregistration_sha256=PREREGISTRATION_SHA256,
        configuration_sha256=configuration_sha256,
        code_sha256=tuple(
            sorted(
                (name, common.sha256_file(code_path))
                for name, code_path in _code_paths().items()
            )
        ),
    )


def _read_private(path: Path, maximum_bytes: int) -> bytes:
    try:
        invalid = (
            not path.is_file()
            or path.is_symlink()
            or bool(path.stat().st_mode & 0o077)
            or path.stat().st_size > maximum_bytes
        )
    except OSError:
        invalid = True
    if invalid:
        raise common.BakeoffError("probe_baseline_raw_invalid")
    return path.read_bytes()


def _baseline_stem(page_id: str, repetition: int) -> str:
    return f"{primary.PROVIDER}-{page_id}-r{repetition}"


def load_baseline(
    root: Path,
    pages: Mapping[str, common.cgpg.Page],
) -> dict[tuple[str, int], common.InvocationOutcome]:
    try:
        invalid_root = (
            not root.is_dir()
            or root.is_symlink()
            or bool(root.stat().st_mode & 0o077)
        )
    except OSError:
        invalid_root = True
    if invalid_root:
        raise common.BakeoffError("probe_baseline_raw_invalid")
    manifest_raw = _read_private(root / "run-manifest.json", 256 * 1024)
    if common.sha256_bytes(manifest_raw) != BASELINE_RAW_MANIFEST_SHA256:
        raise common.BakeoffError("probe_baseline_manifest_mismatch")
    manifest = common.strict_json_loads(manifest_raw, maximum_bytes=256 * 1024)
    try:
        valid_manifest = (
            isinstance(manifest, dict)
            and manifest["schema"] == "mpdf-gemini-primary-raw-run-manifest"
            and manifest["formal_input_manifest_sha256"]
            == common.FORMAL_INPUT_MANIFEST_SHA256
            and manifest["run_binding"]["configuration_sha256"]
            == BASELINE_CONFIGURATION_SHA256
            and manifest["completed_records"]
            == common.SCORABLE_PAGE_COUNT * common.REPETITIONS
        )
        manifest_records = {
            (item["page_id"], item["repetition"]): item["meta_sha256"]
            for item in manifest["records"]
            if item["provider"] == primary.PROVIDER
        }
    except (KeyError, TypeError):
        valid_manifest = False
        manifest_records = {}
    if not valid_manifest:
        raise common.BakeoffError("probe_baseline_raw_invalid")
    outcomes: dict[tuple[str, int], common.InvocationOutcome] = {}
    for page_id in (*GEOMETRY_PAGE_IDS, *TRUNCATION_PAGE_IDS):
        page = pages[page_id]
        for repetition in range(common.REPETITIONS):
            stem = _baseline_stem(page_id, repetition)
            meta_raw = _read_private(root / f"{stem}.meta.json", 64 * 1024)
            if common.sha256_bytes(meta_raw) != manifest_records.get(
                (page_id, repetition)
            ):
                raise common.BakeoffError("probe_baseline_raw_invalid")
            meta = common.strict_json_loads(meta_raw, maximum_bytes=64 * 1024)
            try:
                response_present = meta["response_present"] is True
                if meta["status"] == "success":
                    response_raw = _read_private(
                        root / f"{stem}.response.json",
                        common.MAX_RESPONSE_BYTES,
                    )
                    valid_response = (
                        response_present
                        and len(response_raw) == meta["response_size_bytes"]
                        and common.sha256_bytes(response_raw)
                        == meta["response_sha256"]
                    )
                    if not valid_response:
                        raise common.BakeoffError(
                            "probe_baseline_raw_invalid"
                        )
                    result = common.parse_gemini_response(
                        response_raw, page.width, page.height
                    )
                    result = replace(
                        result, latency_seconds=float(meta["latency_seconds"])
                    )
                    outcomes[(page_id, repetition)] = (
                        common.InvocationOutcome(result)
                    )
                elif meta["status"] == "failure":
                    outcomes[(page_id, repetition)] = common.InvocationOutcome(
                        None,
                        str(meta["error_code"]),
                        provider_response_success=bool(
                            meta["provider_response_success"]
                        ),
                        latency_seconds=float(meta["latency_seconds"]),
                        usage=common._usage_from_record(meta["usage"]),
                        provenance=common._provenance_from_record(
                            meta["provenance"], "google_vertex_gemini"
                        ),
                    )
                else:
                    raise common.BakeoffError("probe_baseline_raw_invalid")
            except (KeyError, TypeError, ValueError):
                raise common.BakeoffError("probe_baseline_raw_invalid") from None
    return outcomes


def _pairwise_geometry(
    page: common.cgpg.Page,
    outcomes: Mapping[tuple[str, int], common.InvocationOutcome],
) -> dict[str, object]:
    results = [outcomes[(page.name, repetition)].result for repetition in range(3)]
    if any(result is None for result in results):
        raise common.BakeoffError("probe_geometry_baseline_incomplete")
    typed = [result for result in results if result is not None]
    per_run = [coverage.score_open_world_target(page, item.lines) for item in typed]
    pairs: list[dict[str, object]] = []
    for left_index in range(len(typed)):
        for right_index in range(left_index + 1, len(typed)):
            left = common.lines_in_scorable_regions(page, typed[left_index].lines)
            right = common.lines_in_scorable_regions(page, typed[right_index].lines)
            metric = common._aggregate_box_metrics(
                [([line.bbox for line in left], [line.bbox for line in right])],
                0.5,
            )
            pairs.append(
                {
                    "left_repetition": left_index,
                    "right_repetition": right_index,
                    "line_count_delta": len(right) - len(left),
                    "box_f1_at_0_50": metric["f1"],
                }
            )
    return {
        "page_id": page.name,
        "source": "frozen_baseline_raw_replay",
        "per_run_target_metrics": per_run,
        "pairwise_repeat_geometry": pairs,
        "minimum_pairwise_box_f1_at_0_50": min(
            float(item["box_f1_at_0_50"]) for item in pairs
        ),
        "mechanism_rule": (
            "If target text CER and line counts remain stable while pairwise "
            "box F1 changes materially, classify as coordinate-generation "
            "stochasticity rather than OCR text instability."
        ),
    }


class ProbeRawStore:
    def __init__(self, binding: common.RunBinding) -> None:
        timestamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
        common.RAW_OUTPUT_ROOT.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(common.RAW_OUTPUT_ROOT, 0o700)
        self.root = (
            common.RAW_OUTPUT_ROOT
            / f"gemini-forensic-{timestamp}-{os.urandom(6).hex()}"
        )
        self.root.mkdir(mode=0o700)
        os.chmod(self.root, 0o700)
        self.binding = binding
        self.records: list[dict[str, object]] = []

    def _write(
        self,
        page_id: str,
        *,
        result: common.ProviderResult | None,
        error: common.BakeoffError | None,
    ) -> None:
        if (result is None) == (error is None):
            raise common.BakeoffError("probe_raw_outcome_invalid")
        stem = f"{primary.PROVIDER}-{page_id}-r0"
        raw = result.raw_response if result is not None else error.raw_response
        if raw is not None:
            common._private_write(self.root / f"{stem}.response.json", raw)
        usage = result.usage if result is not None else error.usage
        provenance = result.provenance if result is not None else error.provenance
        latency = (
            result.latency_seconds if result is not None else error.latency_seconds
        )
        meta = {
            "schema": "mpdf-gemini-forensic-raw-meta",
            "schema_version": "1.0",
            "provider": primary.PROVIDER,
            "page_id": page_id,
            "repetition": 0,
            "status": "success" if result is not None else "failure",
            "error_code": None if result is not None else error.code,
            "provider_response_success": (
                True if result is not None else error.provider_response_success
            ),
            "latency_seconds": latency,
            "usage": common._usage_record(usage),
            "provenance": common._provenance_record(provenance),
            "response_present": raw is not None,
            "response_size_bytes": len(raw) if raw is not None else None,
            "response_sha256": common.sha256_bytes(raw) if raw is not None else None,
        }
        meta_raw = common.canonical_json_bytes(meta)
        common._private_write(self.root / f"{stem}.meta.json", meta_raw)
        self.records.append(
            {
                "page_id": page_id,
                "meta_sha256": common.sha256_bytes(meta_raw),
            }
        )

    def write_success(self, page_id: str, result: common.ProviderResult) -> None:
        self._write(page_id, result=result, error=None)

    def write_failure(self, page_id: str, error: common.BakeoffError) -> None:
        self._write(page_id, result=None, error=error)

    def finalize(self) -> str:
        if len(self.records) != PROBE_CALLS:
            raise common.BakeoffError("probe_raw_output_incomplete")
        manifest = {
            "schema": "mpdf-gemini-forensic-raw-run-manifest",
            "schema_version": "1.0",
            "run_binding": self.binding.as_record(),
            "coverage_contract_sha256": coverage.CONTRACT_SHA256,
            "records": self.records,
        }
        raw = common.canonical_json_bytes(manifest)
        common._private_write(self.root / "run-manifest.json", raw)
        return common.sha256_bytes(raw)


def _tariff_cost(usages: Sequence[common.ProviderUsage]) -> Decimal:
    prompt = sum(item.prompt_tokens for item in usages)
    cached = sum(item.cached_tokens for item in usages)
    output = sum(item.candidate_tokens + item.thought_tokens for item in usages)
    return (
        Decimal(prompt - cached)
        * common.GEMINI_INPUT_USD_PER_MILLION
        / Decimal(1_000_000)
        + Decimal(cached)
        * common.GEMINI_CACHED_INPUT_USD_PER_MILLION
        / Decimal(1_000_000)
        + Decimal(output)
        * common.GEMINI_OUTPUT_AND_THINKING_USD_PER_MILLION
        / Decimal(1_000_000)
    )


def execute_probe(
    pages: Mapping[str, common.cgpg.Page],
    client: primary.GeminiApiClient,
    binding: common.RunBinding,
) -> tuple[list[dict[str, object]], str, Path]:
    store = ProbeRawStore(binding)
    records: list[dict[str, object]] = []
    for page_id in TRUNCATION_PAGE_IDS:
        page = pages[page_id]
        record = common._record_for_page(page_id)
        image_bytes = page.image_path.read_bytes()
        if (
            common.sha256_bytes(image_bytes) != record["image"]["sha256"]
            or len(image_bytes) != record["image"]["size_bytes"]
        ):
            raise common.BakeoffError("fixed_page_changed_after_preflight")
        try:
            result = client.recognize(image_bytes, page.width, page.height)
        except common.BakeoffError as error:
            store.write_failure(page_id, error)
            usage = error.usage
            records.append(
                {
                    "page_id": page_id,
                    "status": "failure",
                    "error_code": error.code,
                    "provider_response_success": error.provider_response_success,
                    "latency_seconds": common._display_float(
                        float(error.latency_seconds or 0.0)
                    ),
                    "usage": common._usage_record(usage),
                    "raw_failure_response_preserved": error.raw_response is not None,
                    "target_metrics": None,
                }
            )
            status = error.code
        else:
            store.write_success(page_id, result)
            records.append(
                {
                    "page_id": page_id,
                    "status": "success",
                    "error_code": None,
                    "provider_response_success": True,
                    "latency_seconds": common._display_float(
                        result.latency_seconds
                    ),
                    "usage": common._usage_record(result.usage),
                    "raw_failure_response_preserved": None,
                    "target_metrics": coverage.score_open_world_target(
                        page, result.lines
                    ),
                }
            )
            status = "ok"
        print(
            json.dumps(
                {"page_id": page_id, "max_output_tokens": 8192, "status": status},
                separators=(",", ":"),
            ),
            file=sys.stderr,
            flush=True,
        )
    return records, store.finalize(), store.root


def _baseline_truncation(
    page_id: str,
    outcomes: Mapping[tuple[str, int], common.InvocationOutcome],
) -> dict[str, object]:
    page_outcomes = [outcomes[(page_id, repetition)] for repetition in range(3)]
    return {
        "max_output_tokens": 4096,
        "attempts": 3,
        "successes": sum(item.result is not None for item in page_outcomes),
        "finish_reason_failures": sum(
            item.error_code == "gemini_finish_reason_invalid"
            for item in page_outcomes
        ),
        "candidate_tokens": [
            (item.result.usage if item.result is not None else item.usage).candidate_tokens
            for item in page_outcomes
            if (item.result is not None or item.usage is not None)
        ],
    }


def build_evidence(
    pages: Mapping[str, common.cgpg.Page],
    baseline: Mapping[tuple[str, int], common.InvocationOutcome],
    probe_records: Sequence[dict[str, object]],
    binding: common.RunBinding,
    raw_manifest_sha256: str,
) -> dict[str, object]:
    usages: list[common.ProviderUsage] = []
    for item in probe_records:
        usage = common._usage_from_record(item["usage"])
        if usage is not None:
            usages.append(usage)
    truncation = []
    for item in probe_records:
        usage_record = item["usage"] if isinstance(item["usage"], dict) else {}
        candidate_tokens = int(usage_record.get("candidate_tokens", 0))
        if item["status"] == "success" and candidate_tokens > 4096:
            mechanism = "direct_output_budget_confirmation"
        elif item["status"] == "success":
            mechanism = "stochastic_overgeneration_not_reproduced"
        elif item["raw_failure_response_preserved"] is True:
            mechanism = "failure_preserved_for_private_forensics"
        else:
            mechanism = "unresolved"
        truncation.append(
            {
                "page_id": item["page_id"],
                "baseline_4096": _baseline_truncation(
                    str(item["page_id"]), baseline
                ),
                "probe_8192": item,
                "mechanism_classification": mechanism,
            }
        )
    report: dict[str, object] = {
        "schema": SCHEMA,
        "schema_version": SCHEMA_VERSION,
        "date": date.today().isoformat(),
        "status": "completed",
        "provider": primary.PROVIDER,
        "model": common.GEMINI_MODEL,
        "coverage_contract_sha256": coverage.CONTRACT_SHA256,
        "coverage_mode": coverage.COVERAGE_MODE,
        "preregistration_sha256": binding.preregistration_sha256,
        "configuration_sha256": binding.configuration_sha256,
        "code_sha256": dict(binding.code_sha256),
        "baseline_raw_manifest_sha256": BASELINE_RAW_MANIFEST_SHA256,
        "probe_raw_manifest_sha256": raw_manifest_sha256,
        "design": {
            "live_calls": PROBE_CALLS,
            "truncation_pages": list(TRUNCATION_PAGE_IDS),
            "geometry_pages_replayed_without_new_calls": list(
                GEOMETRY_PAGE_IDS
            ),
            "max_output_tokens": PROBE_MAX_OUTPUT_TOKENS,
            "thinking_level": PROBE_THINKING_LEVEL,
            "automatic_retries": 0,
        },
        "truncation": truncation,
        "geometry": [
            _pairwise_geometry(pages[page_id], baseline)
            for page_id in GEOMETRY_PAGE_IDS
        ],
        "cost": {
            "tariff_cost_usd": float(_tariff_cost(usages)),
            "planned_ceiling_usd": str(PLANNED_COST_USD),
            "hard_maximum_usd": str(MAX_COST_USD),
            "actual_billed_cost_usd": None,
        },
        "decision": {
            "provider_selection_permitted": False,
            "complete_ocr_competition_permitted": False,
            "reason": "closed_world_complete_gold_not_ready",
        },
        "limitations": [
            "One 8192-token probe call per truncation page cannot estimate a failure rate.",
            "Geometry mechanism uses the frozen prior three-repetition raw responses and makes no new geometry calls.",
            "Open-world annotated-target gold cannot establish provider completeness or a production winner.",
        ],
    }
    report["run_sha256"] = common.canonical_digest(report)
    return report


def _parse_args(argv: Sequence[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--baseline-raw", required=True, type=Path)
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument(
        "--preregistration", type=Path, default=DEFAULT_PREREGISTRATION
    )
    parser.add_argument("--allow-upload", action="store_true")
    parser.add_argument("--acknowledge-upload")
    parser.add_argument("--max-cost-usd")
    parser.add_argument("--preflight-only", action="store_true")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = _parse_args(argv)
    errors: list[str] = []
    if os.environ.get(common.LIVE_ENV) != "1":
        errors.append("live_environment_gate_missing")
    if not args.allow_upload:
        errors.append("upload_permission_missing")
    if args.acknowledge_upload != UPLOAD_ACKNOWLEDGEMENT:
        errors.append("upload_acknowledgement_mismatch")
    if args.max_cost_usd != "0.07":
        errors.append("max_cost_gate_mismatch")
    if not common._output_destinations_ready(args.evidence):
        errors.append("evidence_destination_invalid")
    try:
        coverage.verify_contract()
    except (OSError, common.BakeoffError) as error:
        errors.append(
            error.code
            if isinstance(error, common.BakeoffError)
            else "coverage_contract_missing"
        )
    try:
        binding = verify_preregistration(args.preregistration)
    except (OSError, common.BakeoffError) as error:
        errors.append(
            error.code
            if isinstance(error, common.BakeoffError)
            else "probe_preregistration_missing"
        )
        binding = None
    try:
        selected = common.select_and_verify_fixed_inputs(args.corpus, args.archive)
        pages = {page.name: page for page in selected}
        baseline = load_baseline(args.baseline_raw, pages)
    except common.BakeoffError as error:
        errors.append(error.code)
        pages = {}
        baseline = {}
    try:
        key = primary._api_key(os.environ)
    except common.BakeoffError as error:
        errors.append(error.code)
        key = ""
    if errors:
        print(
            json.dumps(
                {
                    "status": "preflight_failed",
                    "model_calls": 0,
                    "error_codes": sorted(set(errors)),
                },
                separators=(",", ":"),
            )
        )
        return 2
    transport = common.UrllibTransport()
    try:
        primary._readiness(key, transport)
    except common.BakeoffError as error:
        print(
            json.dumps(
                {
                    "status": "preflight_failed",
                    "model_calls": 0,
                    "error_codes": [error.code],
                },
                separators=(",", ":"),
            )
        )
        return 2
    if args.preflight_only:
        print(
            json.dumps(
                {"status": "preflight_ready", "model_calls": 0},
                separators=(",", ":"),
            )
        )
        return 0
    if binding is None:  # pragma: no cover
        raise common.BakeoffError("probe_preflight_internal_error")
    client = primary.GeminiApiClient(
        key,
        transport,
        max_output_tokens=PROBE_MAX_OUTPUT_TOKENS,
    )
    probe_records, raw_manifest, raw_root = execute_probe(
        pages, client, binding
    )
    report = build_evidence(
        pages, baseline, probe_records, binding, raw_manifest
    )
    forbidden_values = [
        key,
        str(args.corpus),
        str(args.archive),
        str(args.baseline_raw),
        str(args.preregistration),
        str(raw_root),
    ]
    for outcome in baseline.values():
        if outcome.result is not None:
            forbidden_values.extend(line.text for line in outcome.result.lines)
    common.write_evidence(
        args.evidence, report, forbidden_values=forbidden_values
    )
    print(
        json.dumps(
            {"status": "completed", "model_calls": PROBE_CALLS},
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except common.BakeoffError as error:
        print(
            json.dumps({"status": "failed", "error_code": error.code}),
            file=sys.stderr,
        )
        raise SystemExit(2) from None
