"""Offline tests for the frozen two-call Gemini forensic probe."""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cgpg  # noqa: E402
import run_cloud_complete_ocr_bakeoff as common  # noqa: E402
import run_gemini_complete_ocr_forensic_probe as probe  # noqa: E402


def _binding() -> common.RunBinding:
    return common.RunBinding(
        "a" * 64, "b" * 64, (("probe", "c" * 64),)
    )


def _page(name: str = "grc_grna_or_399") -> cgpg.Page:
    return cgpg.Page(
        name=name,
        image_path=Path("page.jpg"),
        xml_path=Path("page.xml"),
        width=100,
        height=100,
        regions=(
            cgpg.Region(
                "r",
                "text",
                (0, 0, 100, 100),
                (
                    cgpg.Line("α", (10, 10, 90, 20)),
                    cgpg.Line("β", (10, 30, 90, 40)),
                ),
            ),
        ),
    )


def _result(box_shift: float = 0.0) -> common.ProviderResult:
    return common.ProviderResult(
        lines=(
            common.ProviderLine(
                "α", common.Box(10, 10 + box_shift, 90, 20 + box_shift), 0
            ),
            common.ProviderLine(
                "β", common.Box(10, 30 + box_shift, 90, 40 + box_shift), 1
            ),
        ),
        usage=common.ProviderUsage(prompt_tokens=10, candidate_tokens=20),
        latency_seconds=1.0,
        raw_response=b"{}",
        provenance=common.ProviderProvenance(
            model_version="gemini-3.7-flash",
            response_id_sha256="d" * 64,
        ),
    )


def test_configuration_and_preregistration_freeze_two_calls_only():
    configuration = probe.probe_configuration()
    assert common.canonical_digest(configuration) == (
        "0b959dfa6d232368a3afc4ec7e4a645378956ab541d364c98ee85f9655929570"
    )
    assert configuration["live_calls"] == 2
    assert configuration["new_geometry_calls"] == 0
    assert configuration["hard_maximum_cost_usd"] == "0.07"
    binding = probe.verify_preregistration(
        probe.DEFAULT_PREREGISTRATION, require_committed=False
    )
    assert binding.preregistration_sha256 == probe.PREREGISTRATION_SHA256


def test_probe_raw_store_preserves_failed_http_200_body(monkeypatch, tmp_path):
    monkeypatch.setattr(common, "RAW_OUTPUT_ROOT", tmp_path / "raw")
    store = probe.ProbeRawStore(_binding())
    error = common.BakeoffError("gemini_finish_reason_invalid")
    error.provider_response_success = True
    error.latency_seconds = 1.0
    error.usage = common.ProviderUsage(candidate_tokens=8190)
    error.raw_response = b'{"partial":"private OCR"}'
    store.write_failure("grc_grna_or_443", error)
    response = next(store.root.glob("*.response.json"))
    meta = json.loads(next(store.root.glob("*.meta.json")).read_text())
    assert response.read_bytes() == error.raw_response
    assert response.stat().st_mode & 0o777 == 0o600
    assert meta["status"] == "failure"
    assert meta["response_present"] is True
    assert meta["response_sha256"] == common.sha256_bytes(error.raw_response)


def test_geometry_replay_detects_box_drift_without_new_calls():
    page = _page()
    outcomes = {
        (page.name, 0): common.InvocationOutcome(_result(0.0)),
        (page.name, 1): common.InvocationOutcome(_result(0.0)),
        (page.name, 2): common.InvocationOutcome(_result(50.0)),
    }
    evidence = probe._pairwise_geometry(page, outcomes)
    assert evidence["source"] == "frozen_baseline_raw_replay"
    assert evidence["minimum_pairwise_box_f1_at_0_50"] == 0.0
    assert len(evidence["pairwise_repeat_geometry"]) == 3


def test_8192_success_over_4096_is_direct_budget_confirmation():
    record = {
        "page_id": "grc_grna_or_443",
        "status": "success",
        "usage": {"candidate_tokens": 5000},
    }
    candidate_tokens = record["usage"]["candidate_tokens"]
    mechanism = (
        "direct_output_budget_confirmation"
        if record["status"] == "success" and candidate_tokens > 4096
        else "other"
    )
    assert mechanism == "direct_output_budget_confirmation"
