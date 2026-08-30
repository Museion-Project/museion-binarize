"""Offline tests for the cloud Complete OCR bake-off harness."""

from __future__ import annotations

import base64
import http.client
import json
import math
import stat
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cgpg  # noqa: E402
import run_cloud_complete_ocr_bakeoff as bakeoff  # noqa: E402


def _line(
    text: str = "ἀρετή",
    box: tuple[float, float, float, float] = (10, 10, 90, 20),
    order: int = 0,
) -> bakeoff.ProviderLine:
    return bakeoff.ProviderLine(text, bakeoff.Box(*box), order)


def _page(name: str = "page", texts: tuple[str, ...] = ("ἀρετή",)) -> cgpg.Page:
    lines = tuple(
        cgpg.Line(text, (10, 10 + index * 20, 90, 20 + index * 20))
        for index, text in enumerate(texts)
    )
    return cgpg.Page(
        name=name,
        image_path=Path(f"{name}.jpg"),
        xml_path=Path(f"{name}.xml"),
        width=100,
        height=100,
        regions=(cgpg.Region("r1", "text", (0, 0, 100, 100), lines),),
    )


def _gemini_raw(
    lines: list[dict] | None = None,
    *,
    prompt: int = 100,
    candidate: int = 20,
    thoughts: int = 5,
    cached: int = 10,
) -> bytes:
    structured = {
        "lines": lines
        or [
            {
                "text": "ἀρετή",
                "bbox": [100, 100, 200, 900],
                "reading_order": 0,
            }
        ]
    }
    return json.dumps(
        {
            "modelVersion": "gemini-3.7-flash-001",
            "responseId": "response-fixture-id",
            "candidates": [
                {
                    "finishReason": "STOP",
                    "content": {"parts": [{"text": json.dumps(structured)}]},
                }
            ],
            "usageMetadata": {
                "promptTokenCount": prompt,
                "candidatesTokenCount": candidate,
                "thoughtsTokenCount": thoughts,
                "cachedContentTokenCount": cached,
            },
        },
        ensure_ascii=False,
    ).encode()


def _azure_raw(
    *,
    width: int = 100,
    height: int = 100,
    lines: list[dict] | None = None,
    status: str = "succeeded",
) -> bytes:
    return json.dumps(
        {
            "status": status,
            "analyzeResult": {
                "modelId": "prebuilt-read",
                "apiVersion": "2024-11-30",
                "pages": [
                    {
                        "width": width,
                        "height": height,
                        "unit": "pixel",
                        "lines": lines
                        or [
                            {
                                "content": "ἀρετή",
                                "polygon": [10, 10, 90, 10, 90, 20, 10, 20],
                            }
                        ],
                        "words": [
                            {
                                "content": "ἀρετή",
                                "polygon": [10, 10, 90, 10, 90, 20, 10, 20],
                                "confidence": 0.98,
                            }
                        ],
                    }
                ]
            },
        },
        ensure_ascii=False,
    ).encode()


class FakeTransport:
    def __init__(self, responses: list[bakeoff.HttpResponse]):
        self.responses = list(responses)
        self.calls: list[tuple[str, str, dict[str, str], bytes | None]] = []

    def request(self, method, url, headers, body, timeout_seconds):  # noqa: ANN001
        self.calls.append((method, url, dict(headers), body))
        if not self.responses:
            raise AssertionError("unexpected transport call")
        return self.responses.pop(0)


def _success_result(
    lines: tuple[bakeoff.ProviderLine, ...] | None = None,
    *,
    raw: bytes | None = None,
    usage: bakeoff.ProviderUsage | None = None,
    latency: float = 1.0,
) -> bakeoff.ProviderResult:
    return bakeoff.ProviderResult(
        lines or (_line(),),
        usage or bakeoff.ProviderUsage(),
        latency,
        raw or _gemini_raw(),
    )


def _run_binding() -> bakeoff.RunBinding:
    return bakeoff.RunBinding(
        "a" * 64,
        "b" * 64,
        (("cloud_complete_ocr_bakeoff_harness", "c" * 64),),
    )


def _outcomes(
    page: cgpg.Page, results: list[bakeoff.ProviderResult]
) -> dict[tuple[str, int], bakeoff.InvocationOutcome]:
    return {
        (page.name, repetition): bakeoff.InvocationOutcome(result)
        for repetition, result in enumerate(results)
    }


def test_formal_and_legacy_input_manifest_constants_are_frozen():
    assert bakeoff.canonical_digest(bakeoff.formal_input_manifest()) == (
        "6ab387adc9da8c55d227d3ef88dfe47af9e9f1c757b2f974ba06d8bf0eb4a751"
    )
    assert [item["page_id"] for item in bakeoff.FIXED_INPUT_RECORDS] == [
        "grc_grna_or_399",
        "grc_grna_or_409",
        "grc_grna_or_423",
        "grc_grna_or_429",
        "grc_grna_or_443",
        "grc_grna_or_447",
        "grc_grna_or_461",
        "grc_grna_or_475",
        "grc_grna_or_487",
        "grc_grna_or_489",
        "grc_grna_or_503",
        "grc_grna_or_513",
    ]
    assert bakeoff.LEGACY_AUDIT_MANIFEST_SHA256.startswith("07b330a2")


def test_frozen_schedule_has_72_calls_and_balances_first_position():
    pages = [_page(record["page_id"]) for record in bakeoff.FIXED_INPUT_RECORDS]
    schedule = bakeoff.call_schedule(pages)
    assert len(schedule) == 72
    first_positions = {provider: 0 for provider in bakeoff.PROVIDERS}
    for index in range(0, len(schedule), 2):
        first_positions[schedule[index][2]] += 1
    assert first_positions == {
        "google_vertex_gemini": 18,
        "azure_document_intelligence": 18,
    }


def test_prompt_schema_repetitions_thresholds_and_cost_ceiling_are_not_cli_options():
    assert bakeoff.GEMINI_PROMPT_SHA256 == (
        "ce534ab2f3fcfd8ea8a45607730e892e9996d26fceb5f693f35361447c2ff4af"
    )
    assert bakeoff.canonical_digest(bakeoff.GEMINI_RESPONSE_SCHEMA) == (
        "fa065d5e0c268af610cb58f0f3f7d7baca0ffd87226a3183747940f24ab4624f"
    )
    assert bakeoff.REPETITIONS == 3
    assert bakeoff.MAX_POLYTONIC_CER == 0.15
    assert bakeoff.MAX_BASE_LETTER_CER == 0.12
    assert bakeoff.conservative_planned_cost() == bakeoff.Decimal("0.7460640")
    help_text = subprocess.run(
        [sys.executable, str(Path(bakeoff.__file__)), "--help"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    assert "--threshold" not in help_text.lower()
    assert "--provider" not in help_text
    assert "--page" not in help_text
    assert "--repetitions" not in help_text


def test_strict_json_rejects_duplicate_keys_nonfinite_and_oversize():
    with pytest.raises(bakeoff.BakeoffError, match="response_json_invalid"):
        bakeoff.strict_json_loads(b'{"a":1,"a":2}')
    with pytest.raises(bakeoff.BakeoffError, match="response_json_invalid"):
        bakeoff.strict_json_loads(b'{"a":NaN}')
    with pytest.raises(bakeoff.BakeoffError, match="response_too_large"):
        bakeoff.strict_json_loads(b"[]", maximum_bytes=1)
    with pytest.raises(bakeoff.BakeoffError, match="response_schema_invalid"):
        bakeoff._strict_number(10**10_000)
    deeply_nested = b"[" * 2_000 + b"]" * 2_000
    with pytest.raises(bakeoff.BakeoffError, match="response_json_invalid"):
        bakeoff.strict_json_loads(deeply_nested)


def test_verified_page_parser_scores_the_hashed_buffer_not_changed_path(tmp_path):
    (tmp_path / "page.jpg").write_bytes(b"image-placeholder")
    xml_path = tmp_path / "page.xml"

    def xml(text):  # noqa: ANN001
        return f'''<PcGts xmlns="{cgpg.PAGE_NAMESPACE}"><Page imageFilename="page.jpg" imageWidth="100" imageHeight="100"><TextRegion id="r"><Coords points="0,0 100,0 100,100 0,100"/><TextLine id="l"><Coords points="10,10 90,10 90,20 10,20"/><TextEquiv><Unicode>{text}</Unicode></TextEquiv></TextLine></TextRegion></Page></PcGts>'''.encode()

    xml_path.write_bytes(xml("changed-on-disk"))
    expected = {"width": 100, "height": 100}
    page = bakeoff._parse_verified_page_xml(
        xml_path, xml("verified-buffer"), expected
    )
    assert page.regions[0].lines[0].text == "verified-buffer"


def test_corpus_derived_text_and_cer_work_limits_are_bounded():
    assert bakeoff.MAX_FIXED_REFERENCE_TEXT_CHARS == 2040
    assert bakeoff.MAX_PAGE_TEXT_CHARS == 4096
    with pytest.raises(bakeoff.BakeoffError, match="cer_work_limit_exceeded"):
        bakeoff._bounded_cer_counts("a" * 5000, "b" * 5000)
    oversized = [
        {
            "text": "a" * (bakeoff.MAX_PAGE_TEXT_CHARS + 1),
            "bbox": [1, 1, 2, 2],
            "reading_order": 0,
        }
    ]
    with pytest.raises(bakeoff.BakeoffError, match="response_text_invalid"):
        bakeoff.parse_gemini_response(_gemini_raw(oversized), 100, 100)


def test_gemini_parser_is_strict_and_retains_usage():
    result = bakeoff.parse_gemini_response(_gemini_raw(), 100, 100, 1.25)
    assert result.lines == (_line(),)
    assert result.usage.prompt_tokens == 100
    assert result.usage.candidate_tokens == 20
    assert result.usage.thought_tokens == 5
    assert result.usage.cached_tokens == 10
    assert result.latency_seconds == 1.25
    assert result.provenance.model_version == "gemini-3.7-flash-001"
    assert result.provenance.response_id_sha256 == bakeoff.sha256_bytes(
        b"response-fixture-id"
    )


@pytest.mark.parametrize(
    "model_version",
    [
        "gemini-3.7-flash-OCR text",
        "gemini-3.7-flash-\u03b1",
        "gemini-3.7-flash-/etc/passwd",
        "gemini-3.7-flash-" + "a" * 129,
    ],
)
def test_gemini_provenance_rejects_content_bearing_model_versions(model_version):
    value = json.loads(_gemini_raw())
    value["modelVersion"] = model_version
    with pytest.raises(
        bakeoff.BakeoffError, match="gemini_response_provenance_invalid"
    ):
        bakeoff.parse_gemini_response(json.dumps(value).encode(), 100, 100)


def test_gemini_parser_allows_bounded_opaque_thought_signature_metadata():
    value = json.loads(_gemini_raw())
    value["candidates"][0]["content"]["parts"][0]["thoughtSignature"] = (
        "opaque-signature"
    )
    result = bakeoff.parse_gemini_response(json.dumps(value).encode(), 100, 100)
    assert result.lines == (_line(),)


def test_gemini_parser_ignores_thought_and_empty_metadata_parts():
    value = json.loads(_gemini_raw())
    payload = value["candidates"][0]["content"]["parts"][0]
    value["candidates"][0]["content"]["parts"] = [
        {"text": "internal thought", "thought": True, "thoughtSignature": "sig"},
        payload,
        {"text": "", "thoughtSignature": "final-sig"},
    ]
    result = bakeoff.parse_gemini_response(json.dumps(value).encode(), 100, 100)
    assert result.lines == (_line(),)


@pytest.mark.parametrize(
    "lines,code",
    [
        ([{"text": "x", "bbox": [1, 2, 3, 4], "reading_order": 1}], "reading_order_schema_invalid"),
        ([{"text": "x", "bbox": [1, 2, 1, 4], "reading_order": 0}], "invalid_provider_boxes"),
        ([{"text": "x", "bbox": [-1, 2, 3, 4], "reading_order": 0}], "invalid_provider_boxes"),
        ([{"text": "", "bbox": [1, 2, 3, 4], "reading_order": 0}], "response_text_invalid"),
        ([{"text": "x\ny", "bbox": [1, 2, 3, 4], "reading_order": 0}], "response_text_invalid"),
        ([{"text": "x", "bbox": [1, 2, 3, 4], "reading_order": 0, "extra": 1}], "response_schema_invalid"),
    ],
)
def test_gemini_parser_rejects_bad_schema_and_boxes(lines, code):
    with pytest.raises(bakeoff.BakeoffError, match=code):
        bakeoff.parse_gemini_response(_gemini_raw(lines), 100, 100)


def test_gemini_parser_rejects_exact_duplicate_boxes():
    lines = [
        {"text": "a", "bbox": [10, 10, 20, 20], "reading_order": 0},
        {"text": "b", "bbox": [10, 10, 20, 20], "reading_order": 1},
    ]
    with pytest.raises(bakeoff.BakeoffError) as failure:
        bakeoff.parse_gemini_response(_gemini_raw(lines), 100, 100)
    assert failure.value.code == "exact_duplicate_provider_boxes"
    assert failure.value.invalid_boxes == 1
    assert failure.value.duplicate_boxes == 1


def test_invalid_box_error_also_reports_duplicate_count():
    lines = [
        {"text": "a", "bbox": [10, 10, 10, 20], "reading_order": 0},
        {"text": "b", "bbox": [10, 10, 10, 20], "reading_order": 1},
    ]
    with pytest.raises(bakeoff.BakeoffError) as failure:
        bakeoff.parse_gemini_response(_gemini_raw(lines), 100, 100)
    assert failure.value.degenerate_boxes == 2
    assert failure.value.duplicate_boxes == 1
    assert failure.value.invalid_boxes == 3


def test_gemini_parser_rejects_a_non_stop_finish_reason():
    value = json.loads(_gemini_raw())
    value["candidates"][0]["finishReason"] = "MAX_TOKENS"
    with pytest.raises(bakeoff.BakeoffError, match="gemini_finish_reason_invalid"):
        bakeoff.parse_gemini_response(json.dumps(value).encode(), 100, 100)


def test_azure_parser_uses_native_polygons_order_words_and_confidence():
    result = bakeoff.parse_azure_response(_azure_raw(), 100, 100, 2.5)
    assert result.lines == (_line(),)
    assert result.usage.accepted_pages == 1
    assert result.usage.billable_pages == 1
    assert result.usage.native_words == 1
    assert result.usage.confidence_count == 1
    assert result.usage.confidence_min == 0.98
    assert result.provenance.model_id == "prebuilt-read"
    assert result.provenance.api_version == "2024-11-30"


@pytest.mark.parametrize(
    "raw,code",
    [
        (_azure_raw(width=99), "azure_coordinate_space_invalid"),
        (_azure_raw(lines=[{"content": "x", "polygon": [1, 2]}]), "azure_polygon_invalid"),
        (_azure_raw(lines=[{"content": "x", "polygon": [-1, 1, 2, 1, 2, 2, -1, 2]}]), "invalid_provider_boxes"),
        (_azure_raw(lines=[{"content": "", "polygon": [1, 1, 2, 1, 2, 2, 1, 2]}]), "response_text_invalid"),
    ],
)
def test_azure_parser_rejects_invalid_coordinate_contract(raw, code):
    with pytest.raises(bakeoff.BakeoffError, match=code):
        bakeoff.parse_azure_response(raw, 100, 100)


def test_hungarian_assignment_beats_greedy_and_is_deterministic():
    # Greedy takes weight 9 then zero; the optimum is 8 + 8.
    assert bakeoff._hungarian_max([[9, 8], [8, 0]]) == [(0, 1), (1, 0)]
    assert bakeoff._hungarian_max([[5, 5], [5, 5]]) == bakeoff._hungarian_max(
        [[5, 5], [5, 5]]
    )


def test_matching_uses_exact_threshold_subnanounit_iou_and_frozen_tie_break():
    reference = [bakeoff.Box(0, 0, 1, 1)]
    below = bakeoff.Box(0, 0, math.nextafter(0.5, 0.0), 1)
    boundary = bakeoff.Box(0, 0, 0.5, 1)
    above = bakeoff.Box(0, 0, math.nextafter(0.5, 1.0), 1)
    assert bakeoff.maximum_weight_matching(
        reference, [below], minimum_iou=0.5
    ) == []
    assert bakeoff.maximum_weight_matching(
        reference, [boundary], minimum_iou=0.5
    )[0].right_index == 0
    assert bakeoff.maximum_weight_matching(
        reference, [boundary, above], minimum_iou=0.5
    )[0].right_index == 1

    duplicate_references = [
        bakeoff.Box(0, 0, 1, 1),
        bakeoff.Box(0, 0, 1, 1),
    ]
    duplicate_predictions = [
        bakeoff.Box(0, 0, 1, 1),
        bakeoff.Box(0, 0, 1, 1),
    ]
    tied = bakeoff.maximum_weight_matching(
        duplicate_references, duplicate_predictions, minimum_iou=0.5
    )
    assert [(item.left_index, item.right_index) for item in tied] == [
        (0, 0),
        (1, 1),
    ]
    assert bakeoff.maximum_weight_matching(
        reference, duplicate_predictions, minimum_iou=0.5
    )[0].right_index == 0
    assert bakeoff.maximum_weight_matching(
        duplicate_references, [duplicate_predictions[0]], minimum_iou=0.5
    )[0].left_index == 0


def test_box_alignment_reports_precision_recall_f1_and_unmatched_counts():
    references = [bakeoff.Box(0, 0, 10, 10), bakeoff.Box(0, 20, 10, 30)]
    hypotheses = [bakeoff.Box(0, 0, 10, 10), bakeoff.Box(50, 50, 60, 60)]
    metric = bakeoff.box_alignment(references, hypotheses, 0.5)
    assert metric == {
        "reference_lines": 2,
        "provider_lines": 2,
        "matched_lines": 1,
        "unmatched_reference_lines": 1,
        "unmatched_provider_lines": 1,
        "precision": 0.5,
        "recall": 0.5,
        "f1": 0.5,
    }


def test_reading_order_reports_inversions_pairs_accuracy_and_coverage():
    matches = [
        bakeoff.Match(0, 1, 1.0),
        bakeoff.Match(1, 0, 1.0),
        bakeoff.Match(2, 2, 1.0),
    ]
    lines = (_line(order=0), _line(order=1), _line(order=2))
    metric = bakeoff.reading_order_metrics(matches, 4, lines)
    assert metric["inversions"] == 1
    assert metric["comparable_pairs"] == 3
    assert metric["pairwise_accuracy"] == pytest.approx(2 / 3)
    assert metric["matched_gt_coverage"] == 0.75


def test_text_scoring_reuses_polytonic_and_base_letter_definitions():
    page = _page(texts=("ἄνθρωπος",))
    poly, base = bakeoff.text_counts_for_page(
        page, (_line("ανθρωπος"),)
    )
    assert poly.char_edits > base.char_edits
    assert base.char_edits == 0


def test_stability_splits_structural_failure_from_text_only_failure():
    page = _page(texts=("alpha",))
    good_box = bakeoff.Box(10, 10, 90, 20)
    results = [
        _success_result((bakeoff.ProviderLine(text, good_box, 0),))
        for text in ("alpha", "alpha", "omega")
    ]
    metric = bakeoff.stability_metrics([page], _outcomes(page, results))
    assert metric["structural_all_pages_passed"] is True
    assert metric["recognition_quality_all_pages_passed"] is False


def test_stability_empty_lower_run_fails_closed_against_nonempty_run():
    page = _page(texts=("alpha",))
    outside = bakeoff.ProviderLine(
        "alpha", bakeoff.Box(200, 200, 210, 210), 0
    )
    results = [
        _success_result((outside,)),
        _success_result((_line("alpha"),)),
        _success_result((_line("alpha"),)),
    ]
    metric = bakeoff.stability_metrics([page], _outcomes(page, results))
    assert metric["per_page"][0]["maximum_inter_run_text_cer"] == 1.0
    assert metric["recognition_quality_all_pages_passed"] is False


def _gate_record(structural: str, quality: str) -> dict:
    return {
        "structural_contract_gate": {"outcome": structural},
        "recognition_quality_gate": {"outcome": quality},
    }


@pytest.mark.parametrize(
    "gemini,azure,outcome",
    [
        ((_gate_record("pass", "pass")), _gate_record("fail", "pass"), "inconclusive_control_failed"),
        ((_gate_record("fail", "pass")), _gate_record("pass", "pass"), "gemini_not_complete_ocr_consider_azure"),
        ((_gate_record("pass", "fail")), _gate_record("pass", "pass"), "gemini_coordinate_candidate_quality_failed"),
        ((_gate_record("pass", "pass")), _gate_record("pass", "fail"), "gemini_complete_ocr_candidate_retained"),
    ],
)
def test_decision_logic_separates_control_structure_and_gemini_quality(
    gemini, azure, outcome
):
    assert bakeoff.decide(gemini, azure)["outcome"] == outcome


def test_latency_uses_median_and_nearest_rank_p95():
    metric = bakeoff.latency_statistics([float(value) for value in range(1, 37)])
    assert metric["total_seconds"] == 666.0
    assert metric["mean_seconds"] == 18.5
    assert metric["median_seconds"] == 18.5
    assert metric["p95_seconds"] == 35.0
    assert metric["max_seconds"] == 36.0


def test_tariff_cost_uses_measured_usage_and_never_invents_actual_bill():
    gemini = bakeoff.tariff_cost(
        "google_vertex_gemini",
        bakeoff.ProviderUsage(
            prompt_tokens=1_000_000,
            cached_tokens=100_000,
            candidate_tokens=100_000,
            thought_tokens=100_000,
        ),
    )
    assert gemini["tariff_cost_usd"] == 1.4325
    assert gemini["actual_billed_cost"] is None
    azure = bakeoff.tariff_cost(
        "azure_document_intelligence", bakeoff.ProviderUsage(billable_pages=36)
    )
    assert azure["tariff_cost_usd"] == 0.054
    assert azure["actual_billed_cost"] is None


def test_incomplete_potentially_billable_usage_nulls_total_and_reports_lower_bound():
    usage = bakeoff.ProviderUsage(prompt_tokens=1_000)
    cost = bakeoff.tariff_cost(
        "google_vertex_gemini", usage, usage_complete=False
    )
    assert cost["tariff_cost_usd"] is None
    assert cost["known_measured_usage_tariff_cost_lower_bound_usd"] == 0.00075
    assert cost["cost_status"] == "incomplete_known_lower_bound"
    assert cost["usage_complete"] is False

    page = _page()
    outcomes = {
        (page.name, 0): bakeoff.InvocationOutcome(
            _success_result(usage=usage)
        ),
        (page.name, 1): bakeoff.InvocationOutcome(_success_result()),
        (page.name, 2): bakeoff.InvocationOutcome(
            None,
            "gemini_outcome_ambiguous",
            latency_seconds=0.2,
        ),
    }
    record = bakeoff.evaluate_provider(
        "google_vertex_gemini", [page], outcomes
    )
    assert record["usage"]["incomplete_cost_usage_invocations"] == 1
    assert record["cost"]["tariff_cost_usd"] is None
    comparison = bakeoff._comparison(record, record)["google_vertex_gemini"]
    assert comparison["cost_status"] == "incomplete_known_lower_bound"
    assert comparison["usage_complete"] is False


def test_evaluation_distinguishes_provider_success_from_schema_and_costs_failures():
    page = _page(texts=("ἀρετή",))
    outcomes = {
        (page.name, 0): bakeoff.InvocationOutcome(_success_result(latency=0.1)),
        (page.name, 1): bakeoff.InvocationOutcome(_success_result(latency=0.2)),
        (page.name, 2): bakeoff.InvocationOutcome(
            None,
            "response_schema_invalid",
            provider_response_success=True,
            latency_seconds=0.3,
            usage=bakeoff.ProviderUsage(prompt_tokens=99),
        ),
    }
    record = bakeoff.evaluate_provider("google_vertex_gemini", [page], outcomes)
    checks = record["structural_contract_gate"]["checks"]
    assert checks["response_success"]["actual"] == 1.0
    assert checks["response_success"]["passed"] is True
    assert checks["strict_schema_valid"]["actual"] == pytest.approx(2 / 3, abs=1e-6)
    assert checks["strict_schema_valid"]["passed"] is False
    assert record["usage"]["prompt_tokens"] == 99
    assert record["latency"]["samples"] == 3


def test_box_gate_uses_unrounded_counts_not_rounded_display_value():
    metric = {
        "matched_lines": 1_699_999,
        "provider_lines": 2_000_000,
        "reference_lines": 2_000_000,
        "f1": 0.85,
    }
    assert bakeoff._unrounded_aggregate_f1(metric) == pytest.approx(0.8499995)
    assert bakeoff._unrounded_aggregate_f1(metric) < bakeoff.MIN_BOX_F1_AT_050


def test_gemini_transport_request_uses_vertex_schema_and_exact_jpeg_bytes():
    transport = FakeTransport(
        [bakeoff.HttpResponse(200, (("Content-Type", "application/json"),), _gemini_raw())]
    )
    result = bakeoff.GeminiClient(
        "safe-project", "bearer-canary", transport
    ).recognize(b"\xff\xd8\xffsame-jpeg", 100, 100)
    assert result.lines
    method, url, headers, body = transport.calls[0]
    assert method == "POST"
    assert url.endswith("/gemini-3.7-flash:generateContent")
    assert headers["Authorization"] == "Bearer bearer-canary"
    request = json.loads(body)
    config = request["generationConfig"]
    assert "temperature" not in config
    assert "topP" not in config
    assert "topK" not in config
    assert config["thinkingConfig"] == {"thinkingLevel": "MEDIUM"}
    assert config["responseMimeType"] == "application/json"
    assert config["responseSchema"] == bakeoff.GEMINI_RESPONSE_SCHEMA
    serialized_schema = json.dumps(config["responseSchema"])
    assert "additionalProperties" not in serialized_schema
    assert "minLength" not in serialized_schema
    assert "maxLength" not in serialized_schema
    inline = request["contents"][0]["parts"][1]["inlineData"]
    assert inline["mimeType"] == "image/jpeg"
    assert base64.b64decode(inline["data"]) == b"\xff\xd8\xffsame-jpeg"


def test_gemini_latency_excludes_request_encoding_and_includes_validation(
    monkeypatch,
):
    events = []
    original_encode = bakeoff.canonical_json_bytes
    original_parse = bakeoff.parse_gemini_response

    def encode(value):  # noqa: ANN001
        events.append("encode")
        return original_encode(value)

    def parse(*args, **kwargs):  # noqa: ANN002, ANN003
        events.append("validate")
        return original_parse(*args, **kwargs)

    class EventTransport(FakeTransport):
        def request(self, *args, **kwargs):  # noqa: ANN002, ANN003
            events.append("submit")
            return super().request(*args, **kwargs)

    ticks = iter((5.0, 7.0))

    def clock():
        events.append("clock")
        return next(ticks)

    monkeypatch.setattr(bakeoff, "canonical_json_bytes", encode)
    monkeypatch.setattr(bakeoff, "parse_gemini_response", parse)
    transport = EventTransport([bakeoff.HttpResponse(200, (), _gemini_raw())])
    result = bakeoff.GeminiClient(
        "safe-project", "bearer", transport, clock=clock
    ).recognize(b"jpeg", 100, 100)
    assert result.latency_seconds == 2.0
    assert events == ["encode", "clock", "submit", "validate", "clock"]


def test_gemini_http_error_body_is_not_exposed():
    canary = b"provider-body-secret-canary"
    transport = FakeTransport([bakeoff.HttpResponse(401, (), canary)])
    with pytest.raises(bakeoff.BakeoffError) as failure:
        bakeoff.GeminiClient("safe-project", "bearer", transport).recognize(
            b"jpeg", 100, 100
        )
    assert failure.value.code == "gemini_http_401"
    assert canary.decode() not in str(failure.value)


def test_gemini_schema_failure_retains_raw_usage_provenance_and_latency():
    value = json.loads(_gemini_raw(prompt=123, candidate=45, thoughts=6, cached=7))
    value["candidates"][0]["finishReason"] = "MAX_TOKENS"
    raw = json.dumps(value).encode()
    transport = FakeTransport([bakeoff.HttpResponse(200, (), raw)])
    ticks = iter((10.0, 11.5))
    with pytest.raises(bakeoff.BakeoffError) as failure:
        bakeoff.GeminiClient(
            "safe-project", "bearer", transport, clock=lambda: next(ticks)
        ).recognize(b"jpeg", 100, 100)
    error = failure.value
    assert error.code == "gemini_finish_reason_invalid"
    assert error.provider_response_success is True
    assert error.raw_response == raw
    assert error.latency_seconds == 1.5
    assert error.usage == bakeoff.ProviderUsage(
        prompt_tokens=123,
        candidate_tokens=45,
        thought_tokens=6,
        cached_tokens=7,
    )
    assert error.provenance.model_version == "gemini-3.7-flash-001"


def test_gemini_http_200_oversize_is_successful_potentially_billable_unknown_usage():
    class OversizeTransport:
        def request(self, *_args, **_kwargs):
            error = bakeoff.BakeoffError("response_too_large")
            error.http_status = 200
            raise error

    ticks = iter((5.0, 6.0))
    with pytest.raises(bakeoff.BakeoffError) as failure:
        bakeoff.GeminiClient(
            "safe-project",
            "bearer",
            OversizeTransport(),
            clock=lambda: next(ticks),
        ).recognize(b"jpeg", 100, 100)
    error = failure.value
    assert error.code == "response_too_large"
    assert error.provider_response_success is True
    assert error.usage is None

    page = _page()
    outcomes = {
        (page.name, 0): bakeoff.InvocationOutcome(_success_result()),
        (page.name, 1): bakeoff.InvocationOutcome(_success_result()),
        (page.name, 2): bakeoff.InvocationOutcome(
            None,
            error.code,
            provider_response_success=error.provider_response_success,
            latency_seconds=error.latency_seconds,
        ),
    }
    evaluation = bakeoff.evaluate_provider(
        "google_vertex_gemini", [page], outcomes
    )
    assert evaluation["cost"]["tariff_cost_usd"] is None
    assert evaluation["cost"]["cost_status"] == "incomplete_known_lower_bound"


def test_azure_client_posts_raw_jpeg_and_polls_same_origin_without_retry():
    operation = "https://trusted.cognitiveservices.azure.com/operations/123?api-version=2024-11-30"
    transport = FakeTransport(
        [
            bakeoff.HttpResponse(202, (("Operation-Location", operation),), b""),
            bakeoff.HttpResponse(200, (), b'{"status":"running"}'),
            bakeoff.HttpResponse(200, (), _azure_raw()),
        ]
    )
    result = bakeoff.AzureClient(
        "https://trusted.cognitiveservices.azure.com", "azure-key-canary", transport, sleep=lambda _: None
    ).recognize(b"\xff\xd8\xffsame-jpeg", 100, 100)
    assert result.usage.billable_pages == 1
    assert [call[0] for call in transport.calls] == ["POST", "GET", "GET"]
    assert transport.calls[0][3] == b"\xff\xd8\xffsame-jpeg"
    assert transport.calls[0][2]["Content-Type"] == "image/jpeg"
    assert all(call[2]["Ocp-Apim-Subscription-Key"] == "azure-key-canary" for call in transport.calls)


@pytest.mark.parametrize(
    "location",
    [
        "http://trusted.cognitiveservices.azure.com/op/1",
        "https://trusted.cognitiveservices.azure.com.evil/op/1",
        "https://trusted.cognitiveservices.azure.com:444/op/1",
        "https://user@trusted.cognitiveservices.azure.com/op/1",
        "/relative/op/1",
        "https://trusted.cognitiveservices.azure.com/op/1#fragment",
        "https://trusted.cognitiveservices.azure.com:bad/op/1",
        "https://trusted.cognitiveservices.azure.com/op/with space",
    ],
)
def test_azure_operation_location_rejects_malicious_origins(location):
    with pytest.raises(bakeoff.BakeoffError, match="azure_operation_location_invalid"):
        bakeoff.validate_operation_location(
            location, "https://trusted.cognitiveservices.azure.com"
        )


def test_azure_endpoint_rejects_arbitrary_https_recipients():
    with pytest.raises(bakeoff.BakeoffError, match="azure_endpoint_invalid"):
        bakeoff.AzureClient("https://attacker.example", "key", FakeTransport([]))


def test_azure_rejects_duplicate_operation_location_before_any_poll():
    transport = FakeTransport(
        [
            bakeoff.HttpResponse(
                202,
                (
                    ("Operation-Location", "https://trusted.cognitiveservices.azure.com/op/1"),
                    ("operation-location", "https://trusted.cognitiveservices.azure.com/op/2"),
                ),
                b"",
            )
        ]
    )
    with pytest.raises(bakeoff.BakeoffError, match="azure_operation_location_invalid"):
        bakeoff.AzureClient(
            "https://trusted.cognitiveservices.azure.com", "key", transport
        ).recognize(b"jpeg", 100, 100)
    assert len(transport.calls) == 1


def test_azure_rejects_an_operation_location_that_echoes_its_key():
    key = "long-secret-key-canary"
    transport = FakeTransport(
        [
            bakeoff.HttpResponse(
                202,
                (
                    (
                        "Operation-Location",
                        f"https://trusted.cognitiveservices.azure.com/op/1?value={key}",
                    ),
                ),
                b"",
            )
        ]
    )
    with pytest.raises(
        bakeoff.BakeoffError,
        match="azure_operation_location_contains_secret",
    ):
        bakeoff.AzureClient(
            "https://trusted.cognitiveservices.azure.com", key, transport
        ).recognize(b"jpeg", 100, 100)
    assert len(transport.calls) == 1


def test_azure_rejects_a_percent_encoded_key_in_operation_location():
    key = "secret/key"
    transport = FakeTransport(
        [
            bakeoff.HttpResponse(
                202,
                (
                    (
                        "Operation-Location",
                        "https://trusted.cognitiveservices.azure.com/op/1?value=secret%2Fkey",
                    ),
                ),
                b"",
            )
        ]
    )
    with pytest.raises(
        bakeoff.BakeoffError,
        match="azure_operation_location_contains_secret",
    ):
        bakeoff.AzureClient(
            "https://trusted.cognitiveservices.azure.com", key, transport
        ).recognize(b"jpeg", 100, 100)


def test_azure_accepted_terminal_failure_retains_cost_latency_and_raw():
    operation = "https://trusted.cognitiveservices.azure.com/operations/123"
    raw = b'{"status":"failed"}'
    transport = FakeTransport(
        [
            bakeoff.HttpResponse(202, (("Operation-Location", operation),), b""),
            bakeoff.HttpResponse(200, (), raw),
        ]
    )
    ticks = iter((20.0, 20.1, 20.2, 21.25))
    with pytest.raises(bakeoff.BakeoffError) as failure:
        bakeoff.AzureClient(
            "https://trusted.cognitiveservices.azure.com",
            "key",
            transport,
            clock=lambda: next(ticks),
            sleep=lambda _seconds: None,
        ).recognize(b"jpeg", 100, 100)
    error = failure.value
    assert error.code == "azure_analysis_failed"
    assert error.usage.accepted_pages == 1
    assert error.usage.billable_pages == 1
    assert error.latency_seconds == pytest.approx(1.25)
    assert error.raw_response == raw


def test_azure_rechecks_deadline_after_poll_sleep_and_sends_no_late_poll():
    operation = "https://trusted.cognitiveservices.azure.com/operations/123"
    transport = FakeTransport(
        [
            bakeoff.HttpResponse(202, (("Operation-Location", operation),), b""),
            bakeoff.HttpResponse(200, (), b'{"status":"running"}'),
        ]
    )
    now = [0.0]

    def sleep(_seconds):  # noqa: ANN001
        now[0] = bakeoff.AZURE_POLL_TIMEOUT_SECONDS

    with pytest.raises(bakeoff.BakeoffError) as failure:
        bakeoff.AzureClient(
            "https://trusted.cognitiveservices.azure.com",
            "key",
            transport,
            clock=lambda: now[0],
            sleep=sleep,
        ).recognize(b"jpeg", 100, 100)
    assert failure.value.code == "azure_poll_timeout_ambiguous"
    assert [call[0] for call in transport.calls] == ["POST", "GET"]


@pytest.mark.parametrize("redirect_status", [301, 302, 303, 307, 308])
def test_urllib_transport_never_forwards_secret_across_redirect(
    redirect_status,
):
    received: list[str | None] = []

    class Destination(BaseHTTPRequestHandler):
        def do_POST(self):  # noqa: N802
            received.append(self.headers.get("Authorization"))
            self.send_response(200)
            self.end_headers()

        do_GET = do_POST

        def log_message(self, *_args):
            pass

    destination = ThreadingHTTPServer(("127.0.0.1", 0), Destination)
    destination_thread = threading.Thread(target=destination.serve_forever, daemon=True)
    destination_thread.start()
    location = f"http://127.0.0.1:{destination.server_port}/sink"

    class Redirect(BaseHTTPRequestHandler):
        def do_POST(self):  # noqa: N802
            self.send_response(redirect_status)
            self.send_header("Location", location)
            self.end_headers()

        def log_message(self, *_args):
            pass

    origin = ThreadingHTTPServer(("127.0.0.1", 0), Redirect)
    origin_thread = threading.Thread(target=origin.serve_forever, daemon=True)
    origin_thread.start()
    try:
        response = bakeoff.UrllibTransport().request(
            "POST",
            f"http://127.0.0.1:{origin.server_port}/start",
            {"Authorization": "Bearer redirect-secret"},
            b"payload",
            5,
        )
        assert response.status == redirect_status
        assert received == []
    finally:
        origin.shutdown()
        destination.shutdown()
        origin.server_close()
        destination.server_close()


@pytest.mark.parametrize(
    "failure",
    [
        http.client.InvalidURL("url-canary"),
        http.client.IncompleteRead(b"partial", 10),
    ],
)
def test_urllib_transport_maps_open_and_read_failures_to_redacted_code(failure):
    class BrokenResponse:
        status = 200
        headers = {}

        def read(self, _size):
            raise failure

        def close(self):
            pass

    class BrokenOpener:
        def open(self, *_args, **_kwargs):
            if isinstance(failure, http.client.InvalidURL):
                raise failure
            return BrokenResponse()

    transport = bakeoff.UrllibTransport()
    transport._opener = BrokenOpener()
    with pytest.raises(bakeoff.BakeoffError) as caught:
        transport.request("GET", "https://safe.example/path", {}, None, 1.0)
    assert caught.value.code == "transport_failed"
    assert "url-canary" not in str(caught.value)


def test_urllib_transport_preserves_received_status_on_bounded_read_failure(
    monkeypatch,
):
    class Response:
        status = 200
        headers = {}

        def close(self):
            pass

    class Opener:
        def open(self, *_args, **_kwargs):
            return Response()

    def fail_bounded_read(*_args, **_kwargs):
        raise bakeoff.BakeoffError("response_too_large")

    transport = bakeoff.UrllibTransport()
    transport._opener = Opener()
    monkeypatch.setattr(bakeoff, "_read_bounded", fail_bounded_read)
    with pytest.raises(bakeoff.BakeoffError) as failure:
        transport.request("GET", "https://safe.example/path", {}, None, 1.0)
    assert failure.value.code == "response_too_large"
    assert failure.value.http_status == 200


def test_preflight_requires_all_local_gates_before_credentials(monkeypatch, tmp_path):
    credential_calls = []
    monkeypatch.setattr(
        bakeoff, "verify_preregistration", lambda *a, **k: _run_binding()
    )
    monkeypatch.setattr(
        bakeoff, "select_and_verify_fixed_inputs", lambda *a, **k: [_page()]
    )
    monkeypatch.setattr(
        bakeoff,
        "_live_credentials",
        lambda _env: credential_calls.append(True),
    )
    result = bakeoff.preflight_live(
        corpus_root=tmp_path,
        archive_path=tmp_path / "archive.zip",
        preregistration_path=tmp_path / "pre.json",
        evidence_path=tmp_path / "evidence.json",
        allow_upload=False,
        acknowledgement=None,
        max_cost_text=None,
        environment={},
        require_committed=False,
    )
    assert not result.ready
    assert credential_calls == []
    assert set(result.error_codes) >= {
        "live_environment_gate_missing",
        "upload_permission_missing",
        "upload_acknowledgement_mismatch",
        "max_cost_gate_mismatch",
    }


def test_preflight_fails_both_providers_when_azure_is_missing(monkeypatch, tmp_path):
    monkeypatch.setattr(
        bakeoff, "verify_preregistration", lambda *a, **k: _run_binding()
    )
    monkeypatch.setattr(
        bakeoff, "select_and_verify_fixed_inputs", lambda *a, **k: [_page()]
    )
    monkeypatch.setattr(
        bakeoff, "assert_run_binding_unchanged", lambda *_args: None
    )
    environment = {
        bakeoff.LIVE_ENV: "1",
        bakeoff.GOOGLE_PROJECT_ENV: "safe-project",
    }
    result = bakeoff.preflight_live(
        corpus_root=tmp_path,
        archive_path=tmp_path / "archive.zip",
        preregistration_path=tmp_path / "pre.json",
        evidence_path=tmp_path / "evidence.json",
        allow_upload=True,
        acknowledgement=bakeoff.UPLOAD_ACKNOWLEDGEMENT,
        max_cost_text="1.00",
        environment=environment,
        require_committed=False,
    )
    assert result.public()["ocr_generatecontent_analyze_calls"] == 0
    assert result.public()["provider_readiness"] == {
        "google_vertex_gemini": False,
        "azure_document_intelligence": False,
    }
    assert result.error_codes == ("azure_endpoint_unavailable",)


def test_preflight_accepts_only_exact_ack_and_cost_text(monkeypatch, tmp_path):
    page = _page()
    credentials = bakeoff.LiveCredentials(
        "safe-project", "bearer-canary", "https://trusted.cognitiveservices.azure.com", "key-canary"
    )
    monkeypatch.setattr(
        bakeoff, "verify_preregistration", lambda *a, **k: _run_binding()
    )
    monkeypatch.setattr(
        bakeoff, "select_and_verify_fixed_inputs", lambda *a, **k: [page]
    )
    monkeypatch.setattr(bakeoff, "_live_credentials", lambda _env: credentials)
    monkeypatch.setattr(
        bakeoff, "assert_run_binding_unchanged", lambda *_args: None
    )
    kwargs = dict(
        corpus_root=tmp_path,
        archive_path=tmp_path / "archive.zip",
        preregistration_path=tmp_path / "pre.json",
        evidence_path=tmp_path / "evidence.json",
        allow_upload=True,
        acknowledgement=bakeoff.UPLOAD_ACKNOWLEDGEMENT,
        max_cost_text="1.00",
        environment={bakeoff.LIVE_ENV: "1"},
        require_committed=False,
        readiness_transport=FakeTransport(
            [
                bakeoff.HttpResponse(200, (), b'{"totalTokens":1}'),
                bakeoff.HttpResponse(
                    200,
                    (),
                    b'{"modelId":"prebuilt-read","apiVersion":"2024-11-30"}',
                ),
            ]
        ),
    )
    assert bakeoff.preflight_live(**kwargs).ready
    kwargs["max_cost_text"] = "1.0"
    assert not bakeoff.preflight_live(**kwargs).ready


def test_preflight_readiness_checks_both_and_uploads_no_corpus_bytes(
    monkeypatch, tmp_path
):
    page = _page()
    credentials = bakeoff.LiveCredentials(
        "safe-project", "bearer-canary", "https://trusted.cognitiveservices.azure.com", "key-canary"
    )
    monkeypatch.setattr(
        bakeoff, "verify_preregistration", lambda *a, **k: _run_binding()
    )
    monkeypatch.setattr(
        bakeoff, "select_and_verify_fixed_inputs", lambda *a, **k: [page]
    )
    monkeypatch.setattr(bakeoff, "_live_credentials", lambda _env: credentials)
    monkeypatch.setattr(
        bakeoff, "assert_run_binding_unchanged", lambda *_args: None
    )
    transport = FakeTransport(
        [
            bakeoff.HttpResponse(200, (), b'{"totalTokens":1}'),
            bakeoff.HttpResponse(401, (), b"never-read-error-body"),
        ]
    )
    result = bakeoff.preflight_live(
        corpus_root=tmp_path,
        archive_path=tmp_path / "archive.zip",
        preregistration_path=tmp_path / "pre.json",
        evidence_path=tmp_path / "evidence.json",
        allow_upload=True,
        acknowledgement=bakeoff.UPLOAD_ACKNOWLEDGEMENT,
        max_cost_text="1.00",
        environment={bakeoff.LIVE_ENV: "1"},
        require_committed=False,
        readiness_transport=transport,
    )
    assert not result.ready
    assert result.credentials is None
    assert result.public()["ocr_generatecontent_analyze_calls"] == 0
    assert result.public()["readiness_requests"] == 2
    assert result.public()["provider_readiness"] == {
        "google_vertex_gemini": True,
        "azure_document_intelligence": False,
    }
    assert result.error_codes == ("azure_readiness_http_401",)
    assert [call[0] for call in transport.calls] == ["POST", "GET"]
    assert transport.calls[0][1].endswith(":countTokens")
    assert b"readiness" in transport.calls[0][3]
    assert all(b"jpeg" not in (call[3] or b"") for call in transport.calls)


def test_subprocess_environment_scrubs_all_bakeoff_secrets(monkeypatch):
    monkeypatch.setenv(bakeoff.AZURE_KEY_ENV, "secret-key")
    monkeypatch.setenv(bakeoff.AZURE_ENDPOINT_ENV, "https://secret.example")
    monkeypatch.setenv(bakeoff.GOOGLE_PROJECT_ENV, "secret-project")
    monkeypatch.setenv(bakeoff.LIVE_ENV, "1")
    child = bakeoff._scrubbed_subprocess_environment()
    assert bakeoff.AZURE_KEY_ENV not in child
    assert bakeoff.AZURE_ENDPOINT_ENV not in child
    assert bakeoff.GOOGLE_PROJECT_ENV not in child
    assert bakeoff.LIVE_ENV not in child


class MemoryStore:
    def __init__(self):
        self.writes = []

    def write(self, provider, page_id, repetition, result):  # noqa: ANN001
        self.writes.append((provider, page_id, repetition, result))

    def write_failure(self, provider, page_id, repetition, error):  # noqa: ANN001
        self.writes.append((provider, page_id, repetition, error.code))

    def finalize_schedule(self, entries):  # noqa: ANN001
        self.schedule = list(entries)


class CapturingClient:
    def __init__(self, raw: bytes):
        self.raw = raw
        self.inputs: list[tuple[int, bytes]] = []

    def recognize(self, image_bytes, width, height):  # noqa: ANN001
        self.inputs.append((id(image_bytes), image_bytes))
        return _success_result(raw=self.raw)


def test_execute_schedule_interleaves_and_reuses_identical_bytes(
    monkeypatch, tmp_path, capsys
):
    image = b"\xff\xd8\xffidentical"
    page = _page("fixture")
    image_path = tmp_path / "fixture.jpg"
    image_path.write_bytes(image)
    page = cgpg.Page(
        page.name,
        image_path,
        page.xml_path,
        page.width,
        page.height,
        page.regions,
    )
    record = {
        "page_id": page.name,
        "width": 100,
        "height": 100,
        "image": {
            "size_bytes": len(image),
            "sha256": bakeoff.sha256_bytes(image),
        },
    }
    monkeypatch.setattr(bakeoff, "_record_for_page", lambda _name: record)
    order = []

    class OrderedClient(CapturingClient):
        def __init__(self, name, raw):
            super().__init__(raw)
            self.name = name

        def recognize(self, image_bytes, width, height):
            order.append(self.name)
            return super().recognize(image_bytes, width, height)

    gemini = OrderedClient("google_vertex_gemini", _gemini_raw())
    azure = OrderedClient("azure_document_intelligence", _azure_raw())
    store = MemoryStore()
    outcomes = bakeoff.execute_schedule(
        [page],
        {
            "google_vertex_gemini": gemini,
            "azure_document_intelligence": azure,
        },
        store,  # type: ignore[arg-type]
        bakeoff.BudgetGuard(bakeoff.Decimal("1.00")),
    )
    assert order == [
        "google_vertex_gemini",
        "azure_document_intelligence",
        "azure_document_intelligence",
        "google_vertex_gemini",
        "google_vertex_gemini",
        "azure_document_intelligence",
    ]
    for repetition in range(3):
        left = gemini.inputs[repetition]
        right = azure.inputs[repetition]
        assert left[0] == right[0]
        assert left[1] == right[1] == image
    assert all(len(records) == 3 for records in outcomes.values())
    assert len(store.writes) == 6
    capsys.readouterr()


def test_raw_store_is_ignored_private_and_replays_both_providers(
    monkeypatch, tmp_path
):
    root = tmp_path / "test-output/cloud-ocr-bakeoff"
    monkeypatch.setattr(bakeoff, "RAW_OUTPUT_ROOT", root)
    store = bakeoff.RawStore(
        root / "run-1", create=True, run_binding=_run_binding()
    )
    gemini = bakeoff.parse_gemini_response(_gemini_raw(), 100, 100, 0.5)
    azure = bakeoff.parse_azure_response(_azure_raw(), 100, 100, 0.75)
    store.write("google_vertex_gemini", "grc_grna_or_399", 0, gemini)
    store.write("azure_document_intelligence", "grc_grna_or_399", 0, azure)
    assert stat.S_IMODE(store.root.stat().st_mode) == 0o700
    assert all(stat.S_IMODE(path.stat().st_mode) == 0o600 for path in store.root.iterdir())
    replay = bakeoff.RawStore(store.root, create=False)
    assert replay.read(
        "google_vertex_gemini", "grc_grna_or_399", 0, 100, 100
    ).usage.prompt_tokens == 100
    assert replay.read(
        "azure_document_intelligence", "grc_grna_or_399", 0, 100, 100
    ).usage.native_words == 1


def test_raw_store_replays_a_failed_call_with_usage_and_no_error_body(
    monkeypatch, tmp_path
):
    root = tmp_path / "test-output/cloud-ocr-bakeoff"
    monkeypatch.setattr(bakeoff, "RAW_OUTPUT_ROOT", root)
    store = bakeoff.RawStore(root / "run-1", create=True)
    error = bakeoff.BakeoffError("gemini_http_401")
    error.latency_seconds = 0.25
    error.usage = bakeoff.ProviderUsage(prompt_tokens=9)
    store.write_failure(
        "google_vertex_gemini", "grc_grna_or_399", 0, error
    )
    outcome = store.read_outcome(
        "google_vertex_gemini", "grc_grna_or_399", 0, 100, 100
    )
    assert outcome.result is None
    assert outcome.error_code == "gemini_http_401"
    assert outcome.latency_seconds == 0.25
    assert outcome.usage.prompt_tokens == 9
    assert not any(store.root.glob("*.response.json"))


def test_private_replay_open_rejects_symlinks_and_provider_mismatched_provenance(
    tmp_path,
):
    target = tmp_path / "target.json"
    target.write_bytes(b"{}")
    target.chmod(0o600)
    link = tmp_path / "link.json"
    link.symlink_to(target)
    with pytest.raises(bakeoff.BakeoffError, match="replay_record_invalid"):
        bakeoff.RawStore._read_private_bytes(link, maximum_bytes=10)

    malicious = {
        "model_version": "gemini-3.7-flash-OCR text",
        "response_id_sha256": "a" * 64,
        "model_id": None,
        "api_version": None,
    }
    with pytest.raises(bakeoff.BakeoffError, match="replay_record_invalid"):
        bakeoff._provenance_from_record(
            malicious, "google_vertex_gemini"
        )
    azure_as_gemini = {
        "model_version": None,
        "response_id_sha256": None,
        "model_id": "prebuilt-read",
        "api_version": "2024-11-30",
    }
    with pytest.raises(bakeoff.BakeoffError, match="replay_record_invalid"):
        bakeoff._provenance_from_record(
            azure_as_gemini, "google_vertex_gemini"
        )


def test_raw_run_manifest_binds_exact_complete_order_and_failures(
    monkeypatch, tmp_path
):
    root = tmp_path / "test-output/cloud-ocr-bakeoff"
    monkeypatch.setattr(bakeoff, "RAW_OUTPUT_ROOT", root)
    store = bakeoff.RawStore(
        root / "run-1", create=True, run_binding=_run_binding()
    )
    pages = [_page(record["page_id"]) for record in bakeoff.FIXED_INPUT_RECORDS]
    schedule = bakeoff.call_schedule(pages)
    for page, repetition, provider in schedule:
        error = bakeoff.BakeoffError("transport_failed")
        error.latency_seconds = 0.1
        store.write_failure(provider, page.name, repetition, error)
    store.finalize_schedule(schedule)
    digest = store.manifest_sha256
    assert digest is not None
    store.verify_schedule(schedule, expected_manifest_sha256=digest)
    assert store.manifest_sha256 == bakeoff.sha256_file(
        store.root / "run-manifest.json"
    )
    with pytest.raises(bakeoff.BakeoffError, match="replay_schedule_invalid"):
        store.verify_schedule(
            schedule, expected_manifest_sha256="f" * 64
        )
    monkeypatch.setattr(
        store,
        "read_outcome",
        lambda *_args, **_kwargs: pytest.fail("replay reread an outcome"),
    )
    replay = bakeoff.replay_schedule(
        pages, store, expected_manifest_sha256=digest
    )
    assert sum(len(items) for items in replay.values()) == 72
    assert all(
        outcome.error_code == "transport_failed"
        for items in replay.values()
        for outcome in items.values()
    )
    first_page, first_repetition, first_provider = schedule[0]
    missing = store.root / (
        f"{store._stem(first_provider, first_page.name, first_repetition)}.meta.json"
    )
    original = missing.read_bytes()
    missing.write_bytes(original + b" ")
    with pytest.raises(bakeoff.BakeoffError, match="replay_schedule_invalid"):
        store.verify_schedule(schedule)
    missing.write_bytes(original)
    missing.unlink()
    with pytest.raises(bakeoff.BakeoffError, match="replay_schedule_invalid"):
        store.verify_schedule(schedule)


def test_repository_raw_output_location_is_git_ignored():
    relative = "test-output/cloud-ocr-bakeoff/probe.response.json"
    ignored = subprocess.run(
        ["git", "check-ignore", "-q", relative],
        cwd=bakeoff.REPO_ROOT,
        check=False,
    )
    assert ignored.returncode == 0


def test_evidence_scanner_rejects_content_paths_and_secrets():
    with pytest.raises(bakeoff.BakeoffError, match="evidence_content_policy_violation"):
        bakeoff.assert_evidence_safe({"text": "OCR"})
    with pytest.raises(bakeoff.BakeoffError, match="evidence_content_policy_violation"):
        bakeoff.assert_evidence_safe({"safe": "/Users/person/corpus"})
    with pytest.raises(bakeoff.BakeoffError, match="evidence_content_policy_violation"):
        bakeoff.assert_evidence_safe({"safe": "/etc/passwd"})
    with pytest.raises(bakeoff.BakeoffError, match="evidence_content_policy_violation"):
        bakeoff.assert_evidence_safe({"safe": r"\\server\share\secret"})
    with pytest.raises(bakeoff.BakeoffError, match="evidence_content_policy_violation"):
        bakeoff.assert_evidence_safe(
            {"safe": "prefix-secret-canary-suffix"},
            forbidden_values=["secret-canary"],
        )


def test_complete_evidence_is_content_free_and_preserves_null_actual_cost():
    sensitive_text = "sensitive-ground-truth-canary"
    page = _page(texts=(sensitive_text,))
    result = _success_result((_line(sensitive_text),))
    outcomes = {
        provider: _outcomes(page, [result, result, result])
        for provider in bakeoff.PROVIDERS
    }
    report = bakeoff.build_evidence(
        [page], outcomes, _run_binding(), "d" * 64
    )
    forbidden = bakeoff.evidence_scan_values(
        [page], outcomes, (sensitive_text, "/arbitrary/private/path")
    )
    bakeoff.assert_evidence_safe(report, forbidden_values=forbidden)
    encoded = json.dumps(report, ensure_ascii=False)
    assert sensitive_text not in encoded
    assert "page.jpg" not in encoded
    assert all(
        provider["cost"]["actual_billed_cost"] is None
        for provider in report["providers"].values()
    )


def test_evidence_writer_is_exclusive_content_free_and_mode_0644(tmp_path):
    report = {
        "schema": "safe",
        "metrics": {"cer": 0.1},
        "actual_billed_cost": None,
    }
    destination = tmp_path / "evidence.json"
    bakeoff.write_evidence(destination, report, forbidden_values=["secret"])
    assert stat.S_IMODE(destination.stat().st_mode) == 0o644
    assert json.loads(destination.read_text()) == report
    with pytest.raises(bakeoff.BakeoffError, match="evidence_destination_invalid"):
        bakeoff.write_evidence(destination, report)


def test_preregistration_matches_runner_contract_without_requiring_git_state():
    assert bakeoff.sha256_file(bakeoff.DEFAULT_PREREGISTRATION) == (
        bakeoff.PREREGISTRATION_SHA256
    )
    binding = bakeoff.verify_preregistration(
        bakeoff.DEFAULT_PREREGISTRATION, require_committed=False
    )
    assert dict(binding.code_sha256)["levenshtein_backend"] == (
        bakeoff.sha256_file(Path(bakeoff.gold_metrics.__file__))
    )


def test_reproducible_command_has_no_absolute_paths_or_threshold_overrides():
    command = bakeoff.reproducible_live_argv()
    assert command[0:2] == [
        "python3",
        "scripts/ocr/gold/run_cloud_complete_ocr_bakeoff.py",
    ]
    assert command[-1] == "1.00"
    assert bakeoff.UPLOAD_ACKNOWLEDGEMENT in command
    assert all(not value.startswith("/Users/") for value in command)
    assert all("threshold" not in value for value in command)
