"""Offline contract tests for the Gemini Developer API primary run."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import run_cloud_complete_ocr_bakeoff as common  # noqa: E402
import run_gemini_complete_ocr_primary as primary  # noqa: E402


class FakeTransport:
    def __init__(self, response: common.HttpResponse):
        self.response = response
        self.calls: list[tuple[str, str, dict[str, str], bytes | None]] = []

    def request(self, method, url, headers, body, timeout_seconds):  # noqa: ANN001
        self.calls.append((method, url, dict(headers), body))
        return self.response


def _gemini_response() -> bytes:
    structured = {
        "lines": [
            {
                "text": "ἀρετή",
                "bbox": [100, 100, 200, 900],
                "reading_order": 0,
            }
        ]
    }
    return json.dumps(
        {
            "modelVersion": "gemini-3.7-flash",
            "responseId": "fixture-response-id",
            "candidates": [
                {
                    "finishReason": "STOP",
                    "content": {"parts": [{"text": json.dumps(structured)}]},
                }
            ],
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 20,
                "thoughtsTokenCount": 5,
                "cachedContentTokenCount": 0,
            },
        },
        ensure_ascii=False,
    ).encode()


def test_primary_configuration_is_frozen_and_primary_only():
    configuration = primary.primary_configuration()
    assert common.canonical_digest(configuration) == (
        "d826b29637d6abc0c2952b046d96dbc5ee5b37f0228c6978d410b1413965cb91"
    )
    assert configuration["planned_calls"] == 36
    assert configuration["decision"] == "inconclusive_missing_control"
    assert configuration["wire_maximum_lines"] == 64
    assert configuration["thinking_level"] == "LOW"
    assert primary._planned_cost() == primary.PLANNED_TARIFF_CEILING_USD


def test_v3_preregistration_binds_revised_developer_api_contract():
    binding = primary.verify_preregistration(
        primary.DEFAULT_PREREGISTRATION, require_committed=False
    )
    assert binding.preregistration_sha256 == primary.PREREGISTRATION_SHA256
    assert binding.configuration_sha256 == (
        "d826b29637d6abc0c2952b046d96dbc5ee5b37f0228c6978d410b1413965cb91"
    )


def test_key_is_only_accepted_from_dedicated_environment_mapping():
    assert primary._api_key({primary.API_KEY_ENV: "test-secret"}) == "test-secret"
    with pytest.raises(common.BakeoffError, match="gemini_api_key_unavailable"):
        primary._api_key({})
    with pytest.raises(common.BakeoffError, match="gemini_api_key_unavailable"):
        primary._api_key({primary.API_KEY_ENV: "secret with whitespace"})


def test_request_uses_header_not_url_and_preserves_structured_contract():
    transport = FakeTransport(common.HttpResponse(200, {}, _gemini_response()))
    client = primary.GeminiApiClient("test-secret", transport, clock=lambda: 1.0)
    result = client.recognize(b"jpeg", 100, 100)
    method, url, headers, raw_body = transport.calls[0]
    body = json.loads(raw_body)
    assert method == "POST"
    assert "test-secret" not in url
    assert headers["x-goog-api-key"] == "test-secret"
    assert body["generationConfig"]["responseMimeType"] == "application/json"
    assert "responseSchema" not in body["generationConfig"]
    assert body["generationConfig"]["responseJsonSchema"] == (
        primary.GEMINI_API_RESPONSE_SCHEMA
    )
    assert body["generationConfig"]["thinkingConfig"] == {
        "thinkingLevel": "LOW"
    }
    assert result.lines[0].text == "ἀρετή"
    assert result.lines[0].bbox == common.Box(10.0, 10.0, 90.0, 20.0)


def test_client_allows_frozen_probe_output_budget_override():
    transport = FakeTransport(common.HttpResponse(200, {}, _gemini_response()))
    client = primary.GeminiApiClient(
        "test-secret", transport, clock=lambda: 1.0, max_output_tokens=8192
    )
    client.recognize(b"jpeg", 100, 100)
    body = json.loads(transport.calls[0][3])
    assert body["generationConfig"]["maxOutputTokens"] == 8192
    with pytest.raises(ValueError, match="invalid max_output_tokens"):
        primary.GeminiApiClient(
            "test-secret", transport, max_output_tokens=True
        )


def test_http_failure_keeps_private_provider_body_for_raw_store():
    raw = b'{"error":{"status":"INVALID_ARGUMENT"}}'
    transport = FakeTransport(common.HttpResponse(400, (), raw))
    client = primary.GeminiApiClient(
        "test-secret", transport, clock=lambda: 1.0
    )
    with pytest.raises(common.BakeoffError, match="gemini_http_400") as raised:
        client.recognize(b"jpeg", 100, 100)
    assert raised.value.raw_response == raw
    assert raised.value.http_status == 400
    assert raised.value.provider_response_success is False


def test_primary_raw_store_preserves_private_failed_response(
    monkeypatch, tmp_path
):
    root = tmp_path / "raw"
    monkeypatch.setattr(common, "RAW_OUTPUT_ROOT", root)
    store = primary.PrimaryRawStore(
        common.RunBinding("a" * 64, "b" * 64, (("runner", "c" * 64),))
    )
    error = common.BakeoffError("gemini_finish_reason_invalid")
    error.provider_response_success = True
    error.latency_seconds = 1.5
    error.raw_response = _gemini_response()
    outcome = primary._outcome_from_error(error)
    store.write(
        "page",
        0,
        outcome,
        failure_raw_response=error.raw_response,
    )
    response_path = store.root / "google_gemini_api-page-r0.response.json"
    meta_path = store.root / "google_gemini_api-page-r0.meta.json"
    assert response_path.read_bytes() == error.raw_response
    assert response_path.stat().st_mode & 0o777 == 0o600
    meta = json.loads(meta_path.read_text())
    assert meta["status"] == "failure"
    assert meta["response_present"] is True
    assert meta["response_sha256"] == common.sha256_bytes(error.raw_response)


def test_readiness_is_content_free_and_requires_positive_token_count():
    transport = FakeTransport(
        common.HttpResponse(200, {}, json.dumps({"totalTokens": 1}).encode())
    )
    assert primary._readiness("test-secret", transport)
    _, url, headers, raw_body = transport.calls[0]
    assert url.endswith(":countTokens")
    assert "test-secret" not in url
    assert headers["x-goog-api-key"] == "test-secret"
    assert json.loads(raw_body) == {
        "contents": [{"parts": [{"text": "readiness"}]}]
    }


def test_result_evidence_forbids_provider_selection_without_control():
    page = common.cgpg.Page(
        name="page",
        image_path=Path("page.jpg"),
        xml_path=Path("page.xml"),
        width=100,
        height=100,
        regions=(
            common.cgpg.Region(
                "r",
                "text",
                (0, 0, 100, 100),
                (common.cgpg.Line("ἀρετή", (10, 10, 90, 20)),),
            ),
        ),
    )
    result = common.ProviderResult(
        lines=(common.ProviderLine("ἀρετή", common.Box(10, 10, 90, 20), 0),),
        usage=common.ProviderUsage(),
        latency_seconds=1.0,
        raw_response=_gemini_response(),
    )
    outcomes = {
        (page.name, repetition): common.InvocationOutcome(result)
        for repetition in range(common.REPETITIONS)
    }
    binding = common.RunBinding("a" * 64, "b" * 64, (("runner", "c" * 64),))
    evidence = primary.build_evidence([page], outcomes, binding, "d" * 64)
    assert evidence["status"] == "completed_primary_only"
    assert evidence["contract_control"]["status"] == "not_run_missing_credentials"
    assert evidence["decision"] == {
        "outcome": "inconclusive_missing_control",
        "candidate_retained": None,
        "production_eligibility": "not_established_by_primary_only_run",
    }
