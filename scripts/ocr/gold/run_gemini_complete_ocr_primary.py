#!/usr/bin/env python3
"""Run the preregistered Gemini 3.7 Flash primary-only OCR measurement.

This is the primary half of the cloud Complete OCR bakeoff.  Azure remains the
contract-control and is deliberately recorded as not run, so this command can
measure Gemini but can never select a production provider.  The API key is read
only from the child process environment and is never accepted as an argument,
written to disk, or included in evidence.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
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

import run_cloud_complete_ocr_bakeoff as common  # noqa: E402


PROVIDER = "google_gemini_api"
BACKEND = "gemini_developer_api"
API_VERSION = "v1beta"
API_ORIGIN = "https://generativelanguage.googleapis.com"
API_KEY_ENV = "MPDF_BAKEOFF_GEMINI_API_KEY"
LIVE_ENV = common.LIVE_ENV
MAX_COST_USD = Decimal("0.70")
PLANNED_TARIFF_CEILING_USD = Decimal("0.6920640")
UPLOAD_ACKNOWLEDGEMENT = (
    "I acknowledge CGPG JPEG upload to Google Gemini API under the "
    "configured retention policy."
)
DEFAULT_PREREGISTRATION = (
    common.REPO_ROOT
    / "docs/evidence/gemini-complete-ocr-primary-preregistration-v3-2026-08-30.json"
)
PREREGISTRATION_SHA256 = (
    "01e103561b52ca4770c74454519b4de245720c07bed842c3d70b9cddf3759f62"
)
GEMINI_API_MAX_LINES = 64
GEMINI_API_THINKING_LEVEL = "LOW"
GEMINI_API_RESPONSE_SCHEMA = {
    "type": "object",
    "properties": {
        "lines": {
            "type": "array",
            "minItems": 1,
            "maxItems": GEMINI_API_MAX_LINES,
            "items": {
                "type": "object",
                "properties": {
                    "text": {"type": "string"},
                    "bbox": {
                        "type": "array",
                        "minItems": 4,
                        "maxItems": 4,
                        "items": {"type": "integer"},
                    },
                    "reading_order": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 255,
                    },
                },
                "required": ["text", "bbox", "reading_order"],
            },
        }
    },
    "required": ["lines"],
}


def _endpoint(method: str) -> str:
    if method not in {"generateContent", "countTokens"}:
        raise ValueError("unsupported Gemini API method")
    return (
        f"{API_ORIGIN}/{API_VERSION}/models/{common.GEMINI_MODEL}:{method}"
    )


def primary_configuration() -> dict[str, object]:
    return {
        "provider": PROVIDER,
        "backend": BACKEND,
        "api_version": API_VERSION,
        "model": common.GEMINI_MODEL,
        "prompt_sha256": common.GEMINI_PROMPT_SHA256,
        "response_schema_sha256": common.canonical_digest(
            GEMINI_API_RESPONSE_SCHEMA
        ),
        "wire_maximum_lines": GEMINI_API_MAX_LINES,
        "local_coordinate_range_validation": "integer_0_to_1000",
        "thinking_level": GEMINI_API_THINKING_LEVEL,
        "max_output_tokens": common.GEMINI_MAX_OUTPUT_TOKENS,
        "sampling_parameters": "omitted_provider_defaults",
        "page_count": common.SCORABLE_PAGE_COUNT,
        "repetitions": common.REPETITIONS,
        "planned_calls": common.SCORABLE_PAGE_COUNT * common.REPETITIONS,
        "input_manifest_sha256": common.FORMAL_INPUT_MANIFEST_SHA256,
        "automatic_retries": 0,
        "concurrency": 1,
        "thresholds": {
            "polytonic_cer_max": common.MAX_POLYTONIC_CER,
            "base_letter_cer_max": common.MAX_BASE_LETTER_CER,
            "box_f1_at_0_5_min": common.MIN_BOX_F1_AT_050,
            "box_f1_at_0_75_min": common.MIN_BOX_F1_AT_075,
            "matched_mean_iou_min": common.MIN_MATCHED_MEAN_IOU,
            "reading_order_pairwise_accuracy_min": (
                common.MIN_READING_ORDER_PAIRWISE_ACCURACY
            ),
            "reading_order_gt_coverage_min": (
                common.MIN_READING_ORDER_GT_COVERAGE
            ),
            "stability_box_f1_at_0_5_min": (
                common.MIN_STABILITY_BOX_F1_AT_050
            ),
            "stability_order_pairwise_accuracy_min": (
                common.MIN_STABILITY_ORDER_PAIRWISE_ACCURACY
            ),
            "stability_inter_run_text_cer_max": (
                common.MAX_STABILITY_INTER_RUN_TEXT_CER
            ),
            "stability_relative_line_count_delta_max": (
                common.MAX_STABILITY_RELATIVE_LINE_COUNT_DELTA
            ),
        },
        "tariff": {
            "effective_date": "2026-08-30",
            "input_usd_per_million": 0.75,
            "cached_input_usd_per_million": 0.075,
            "output_and_thinking_usd_per_million": 3.75,
            "planned_ceiling_usd": str(PLANNED_TARIFF_CEILING_USD),
            "hard_maximum_usd": str(MAX_COST_USD),
            "actual_billed_cost_default": None,
        },
        "decision": "inconclusive_missing_control",
        "production_eligibility": "not_established_by_primary_only_run",
        "single_column_limitation": True,
    }


def _code_paths() -> dict[str, Path]:
    return {
        **common._code_paths(),
        "gemini_primary_runner": Path(__file__).resolve(),
    }


def _code_digests() -> dict[str, str]:
    return {
        name: common.sha256_file(path) for name, path in _code_paths().items()
    }


def verify_preregistration(
    path: Path, *, require_committed: bool = True
) -> common.RunBinding:
    try:
        raw = path.read_bytes()
    except OSError:
        raise common.BakeoffError("primary_preregistration_missing") from None
    if common.sha256_bytes(raw) != PREREGISTRATION_SHA256:
        raise common.BakeoffError(
            "primary_preregistration_digest_mismatch"
        )
    value = common.strict_json_loads(raw, maximum_bytes=1024 * 1024)
    expected_configuration = common.canonical_digest(primary_configuration())
    try:
        valid = (
            isinstance(value, dict)
            and value["schema"]
            == "mpdf-gemini-complete-ocr-primary-preregistration"
            and value["schema_version"] == "1.2"
            and value["status"] == "preregistered_not_executed"
            and value["registered_before_live_execution"] is True
            and value["provider"]["backend"] == BACKEND
            and value["provider"]["model"] == common.GEMINI_MODEL
            and value["corpus"]["formal_input_manifest_sha256"]
            == common.FORMAL_INPUT_MANIFEST_SHA256
            and value["design"]["planned_calls"]
            == common.SCORABLE_PAGE_COUNT * common.REPETITIONS
            and value["harness_binding"]["configuration_sha256"]
            == expected_configuration
            and value["decision"]["outcome_without_control"]
            == "inconclusive_missing_control"
        )
    except (KeyError, TypeError):
        valid = False
    if not valid:
        raise common.BakeoffError("primary_preregistration_invalid")
    paths = [path, *_code_paths().values()]
    if require_committed and not common._git_paths_are_committed(paths):
        raise common.BakeoffError("primary_preregistration_or_code_not_committed")
    return common.RunBinding(
        preregistration_sha256=common.sha256_bytes(raw),
        configuration_sha256=expected_configuration,
        code_sha256=tuple(sorted(_code_digests().items())),
    )


def _api_key(environment: Mapping[str, str]) -> str:
    key = environment.get(API_KEY_ENV, "")
    if (
        not key
        or len(key) > 4096
        or any(character.isspace() for character in key)
    ):
        raise common.BakeoffError("gemini_api_key_unavailable")
    return key


class GeminiApiClient:
    def __init__(
        self,
        api_key: str,
        transport: common.HttpTransport,
        clock=time.perf_counter,
    ) -> None:
        self._api_key = api_key
        self._transport = transport
        self._clock = clock

    @staticmethod
    def _request_body(image_bytes: bytes) -> bytes:
        return common.canonical_json_bytes(
            {
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            {"text": common.GEMINI_PROMPT},
                            {
                                "inlineData": {
                                    "mimeType": "image/jpeg",
                                    "data": base64.b64encode(image_bytes).decode(
                                        "ascii"
                                    ),
                                }
                            },
                        ],
                    }
                ],
                "generationConfig": {
                    "maxOutputTokens": common.GEMINI_MAX_OUTPUT_TOKENS,
                    "responseMimeType": "application/json",
                    "responseJsonSchema": GEMINI_API_RESPONSE_SCHEMA,
                    "thinkingConfig": {
                        "thinkingLevel": GEMINI_API_THINKING_LEVEL
                    },
                },
            }
        )

    def recognize(
        self, image_bytes: bytes, width: int, height: int
    ) -> common.ProviderResult:
        started = self._clock()
        try:
            response = self._transport.request(
                "POST",
                _endpoint("generateContent"),
                {
                    "x-goog-api-key": self._api_key,
                    "Content-Type": "application/json; charset=utf-8",
                    "Accept": "application/json",
                },
                self._request_body(image_bytes),
                common.HTTP_TIMEOUT_SECONDS,
            )
        except common.BakeoffError as error:
            if error.code == "transport_failed":
                error = common.BakeoffError("gemini_outcome_ambiguous")
            error.latency_seconds = self._clock() - started
            raise error from None
        if response.status != 200:
            error = common.BakeoffError(
                f"gemini_http_{response.status}"
                if response.status
                in (400, 401, 403, 404, 408, 409, 429, 500, 502, 503, 504)
                else "gemini_http_error"
            )
            error.latency_seconds = self._clock() - started
            raise error
        try:
            parsed = common.parse_gemini_response(response.body, width, height)
        except common.BakeoffError as error:
            error.raw_response = response.body
            error.provider_response_success = True
            error.latency_seconds = self._clock() - started
            try:
                envelope = common.strict_json_loads(response.body)
                error.usage = common.parse_gemini_usage(envelope)
                error.provenance = common.parse_gemini_provenance(envelope)
            except common.BakeoffError:
                pass
            raise
        if len(parsed.lines) > GEMINI_API_MAX_LINES:
            error = common.BakeoffError("response_schema_invalid")
            error.raw_response = response.body
            error.provider_response_success = True
            error.latency_seconds = self._clock() - started
            error.usage = parsed.usage
            error.provenance = parsed.provenance
            raise error
        return replace(parsed, latency_seconds=self._clock() - started)


def _readiness(api_key: str, transport: common.HttpTransport) -> bool:
    response = transport.request(
        "POST",
        _endpoint("countTokens"),
        {
            "x-goog-api-key": api_key,
            "Content-Type": "application/json; charset=utf-8",
            "Accept": "application/json",
        },
        common.canonical_json_bytes(
            {"contents": [{"parts": [{"text": "readiness"}]}]}
        ),
        30.0,
    )
    if response.status != 200:
        raise common.BakeoffError(
            f"gemini_readiness_http_{response.status}"
            if response.status in (400, 401, 403, 404, 429, 500, 503)
            else "gemini_readiness_http_error"
        )
    value = common.strict_json_loads(response.body, maximum_bytes=64 * 1024)
    if not isinstance(value, dict) or common._strict_int(
        value.get("totalTokens")
    ) <= 0:
        raise common.BakeoffError("gemini_readiness_response_invalid")
    return True


def _planned_cost() -> Decimal:
    return sum(
        (
            common.conservative_call_cost("google_vertex_gemini", record)
            for record in common.FIXED_INPUT_RECORDS
            for _ in range(common.REPETITIONS)
        ),
        Decimal(0),
    )


class PrimaryRawStore:
    def __init__(self, binding: common.RunBinding) -> None:
        timestamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
        self.root = (
            common.RAW_OUTPUT_ROOT
            / f"gemini-primary-{timestamp}-{os.urandom(6).hex()}"
        )
        common._assert_private_tree(self.root)
        common.RAW_OUTPUT_ROOT.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(common.RAW_OUTPUT_ROOT, 0o700)
        self.root.mkdir(mode=0o700)
        os.chmod(self.root, 0o700)
        self.binding = binding
        self.records: list[dict[str, object]] = []

    @staticmethod
    def _stem(page_id: str, repetition: int) -> str:
        if not re.fullmatch(r"[A-Za-z0-9._-]{1,128}", page_id):
            raise common.BakeoffError("raw_output_identifier_invalid")
        return f"{PROVIDER}-{page_id}-r{repetition}"

    def write(
        self,
        page_id: str,
        repetition: int,
        outcome: common.InvocationOutcome,
    ) -> None:
        stem = self._stem(page_id, repetition)
        result = outcome.result
        raw = result.raw_response if result is not None else None
        if raw is not None:
            common._private_write(self.root / f"{stem}.response.json", raw)
        meta = {
            "schema": "mpdf-gemini-primary-raw-meta",
            "schema_version": "1.0",
            "provider": PROVIDER,
            "page_id": page_id,
            "repetition": repetition,
            "status": "success" if result is not None else "failure",
            "error_code": None if result is not None else outcome.error_code,
            "provider_response_success": (
                True if result is not None else outcome.provider_response_success
            ),
            "latency_seconds": (
                result.latency_seconds
                if result is not None
                else outcome.latency_seconds
            ),
            "usage": common._usage_record(
                result.usage if result is not None else outcome.usage
            ),
            "provenance": common._provenance_record(
                result.provenance if result is not None else outcome.provenance
            ),
            "response_present": raw is not None,
            "response_size_bytes": len(raw) if raw is not None else None,
            "response_sha256": (
                common.sha256_bytes(raw) if raw is not None else None
            ),
        }
        meta_raw = common.canonical_json_bytes(meta)
        common._private_write(self.root / f"{stem}.meta.json", meta_raw)
        self.records.append(
            {
                "provider": PROVIDER,
                "page_id": page_id,
                "repetition": repetition,
                "meta_sha256": common.sha256_bytes(meta_raw),
            }
        )

    def finalize(self) -> str:
        expected = common.SCORABLE_PAGE_COUNT * common.REPETITIONS
        if len(self.records) != expected:
            raise common.BakeoffError("primary_raw_output_incomplete")
        manifest = {
            "schema": "mpdf-gemini-primary-raw-run-manifest",
            "schema_version": "1.0",
            "formal_input_manifest_sha256": (
                common.FORMAL_INPUT_MANIFEST_SHA256
            ),
            "run_binding": self.binding.as_record(),
            "completed_records": expected,
            "records": self.records,
        }
        raw = common.canonical_json_bytes(manifest)
        common._private_write(self.root / "run-manifest.json", raw)
        return common.sha256_bytes(raw)


def _outcome_from_error(error: common.BakeoffError) -> common.InvocationOutcome:
    return common.InvocationOutcome(
        None,
        error.code,
        provider_response_success=error.provider_response_success,
        latency_seconds=error.latency_seconds,
        usage=error.usage,
        provenance=error.provenance,
        invalid_boxes=error.invalid_boxes,
        degenerate_boxes=error.degenerate_boxes,
        out_of_page_boxes=error.out_of_page_boxes,
        duplicate_boxes=error.duplicate_boxes,
    )


def execute(
    pages: Sequence[common.cgpg.Page],
    client: GeminiApiClient,
    binding: common.RunBinding,
) -> tuple[
    dict[tuple[str, int], common.InvocationOutcome], str, Path
]:
    outcomes: dict[tuple[str, int], common.InvocationOutcome] = {}
    budget = common.BudgetGuard(MAX_COST_USD)
    store = PrimaryRawStore(binding)
    for page in pages:
        record = common._record_for_page(page.name)
        image_bytes = page.image_path.read_bytes()
        if (
            len(image_bytes) != record["image"]["size_bytes"]
            or common.sha256_bytes(image_bytes) != record["image"]["sha256"]
            or image_bytes[:3] != b"\xff\xd8\xff"
        ):
            raise common.BakeoffError("fixed_page_changed_after_preflight")
        for repetition in range(common.REPETITIONS):
            budget.reserve("google_vertex_gemini", record)
            try:
                result = client.recognize(
                    image_bytes, page.width, page.height
                )
                outcome = common.InvocationOutcome(result)
                status = "ok"
            except common.BakeoffError as error:
                outcome = _outcome_from_error(error)
                status = error.code
            store.write(page.name, repetition, outcome)
            outcomes[(page.name, repetition)] = outcome
            print(
                json.dumps(
                    {
                        "provider": PROVIDER,
                        "page_id": page.name,
                        "repetition": repetition,
                        "status": status,
                    },
                    separators=(",", ":"),
                ),
                file=sys.stderr,
                flush=True,
            )
    return outcomes, store.finalize(), store.root


def build_evidence(
    pages: Sequence[common.cgpg.Page],
    outcomes: Mapping[tuple[str, int], common.InvocationOutcome],
    binding: common.RunBinding,
    raw_manifest_sha256: str,
) -> dict[str, object]:
    evaluation = common.evaluate_provider(
        "google_vertex_gemini", pages, outcomes
    )
    evaluation["provider"] = PROVIDER
    evaluation["backend"] = BACKEND
    evaluation["cost"]["tariff_snapshot"]["backend"] = BACKEND
    report: dict[str, object] = {
        "schema": "mpdf-cloud-complete-ocr-bakeoff",
        "schema_version": "1.0",
        "date": date.today().isoformat(),
        "status": "completed_primary_only",
        "content_policy": (
            "Content-free metrics, usage, latency, digests, stable error codes, "
            "and public tariff calculations only."
        ),
        "corpus": {
            "doi": common.CORPUS_DOI,
            "license": common.CORPUS_LICENSE,
            "archive_sha256": common.CORPUS_ARCHIVE_SHA256,
            "formal_input_manifest_sha256": (
                common.FORMAL_INPUT_MANIFEST_SHA256
            ),
            "page_ids": [page.name for page in pages],
            "single_column_limitation": True,
        },
        "execution": {
            "provider": PROVIDER,
            "backend": BACKEND,
            "model": common.GEMINI_MODEL,
            "repetitions_per_page": common.REPETITIONS,
            "planned_calls": len(pages) * common.REPETITIONS,
            "automatic_retries": 0,
            "concurrency": 1,
            "identical_jpeg_bytes_enforced": True,
        },
        "preregistration_sha256": binding.preregistration_sha256,
        "configuration_sha256": binding.configuration_sha256,
        "code_sha256": dict(binding.code_sha256),
        "raw_run_manifest_sha256": raw_manifest_sha256,
        "providers": {PROVIDER: evaluation},
        "contract_control": {
            "provider": "azure_document_intelligence",
            "status": "not_run_missing_credentials",
        },
        "comparison": {"status": "deferred_until_control_run"},
        "decision": {
            "outcome": "inconclusive_missing_control",
            "candidate_retained": None,
            "production_eligibility": "not_established_by_primary_only_run",
        },
        "limitations": [
            "Azure Document Intelligence control was not run, so no provider selection is permitted.",
            "The fixed CGPG slice is single-column Greek and does not validate complex-layout reading order.",
            "Tariff cost is calculated from response-reported usage; actual billed cost remains null without a billing export.",
            "A structural pass remains experimental evidence and does not override ADR 0011 production eligibility rules.",
        ],
    }
    report["run_sha256"] = common.canonical_digest(report)
    return report


def _parse_arguments(argv: Sequence[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--archive", required=True, type=Path)
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
    args = _parse_arguments(argv)
    errors: list[str] = []
    if os.environ.get(LIVE_ENV) != "1":
        errors.append("live_environment_gate_missing")
    if not args.allow_upload:
        errors.append("upload_permission_missing")
    if args.acknowledge_upload != UPLOAD_ACKNOWLEDGEMENT:
        errors.append("upload_acknowledgement_mismatch")
    if args.max_cost_usd != "0.70":
        errors.append("max_cost_gate_mismatch")
    if _planned_cost() != PLANNED_TARIFF_CEILING_USD:
        errors.append("planned_cost_contract_mismatch")
    if not common._output_destinations_ready(args.evidence):
        errors.append("evidence_destination_invalid")
    try:
        binding = verify_preregistration(args.preregistration)
    except common.BakeoffError as error:
        errors.append(error.code)
        binding = None
    try:
        pages = common.select_and_verify_fixed_inputs(
            args.corpus, args.archive
        )
    except common.BakeoffError as error:
        errors.append(error.code)
        pages = []
    try:
        key = _api_key(os.environ)
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
        _readiness(key, transport)
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
                {
                    "status": "preflight_ready",
                    "model_calls": 0,
                    "readiness_requests": 1,
                },
                separators=(",", ":"),
            )
        )
        return 0
    if binding is None or not pages:  # pragma: no cover
        raise common.BakeoffError("primary_preflight_internal_error")
    outcomes, raw_manifest, raw_root = execute(
        pages, GeminiApiClient(key, transport), binding
    )
    report = build_evidence(pages, outcomes, binding, raw_manifest)
    common.write_evidence(
        args.evidence,
        report,
        forbidden_values=common.evidence_scan_values(
            pages,
            {"google_vertex_gemini": outcomes},
            (
                key,
                str(args.corpus),
                str(args.archive),
                str(args.preregistration),
                str(raw_root),
            ),
        ),
    )
    print(
        json.dumps(
            {
                "status": "completed_primary_only",
                "decision": "inconclusive_missing_control",
            },
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
