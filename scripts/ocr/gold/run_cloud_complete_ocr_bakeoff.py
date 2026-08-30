#!/usr/bin/env python3
"""Run the isolated CGPG cloud Complete OCR bake-off.

This is an evaluation harness, not a product OCR adapter.  It compares the
Vertex AI publisher model ``gemini-3.7-flash`` with Azure Document
Intelligence ``prebuilt-read`` on the exact twelve-page CGPG holdout used by
the earlier Greek-model bake-off.  Thresholds, repetition count, call order,
tariffs, and decision rules are constants rather than CLI options.

Live calls are fail-closed behind one all-or-nothing preflight.  A failed
preflight performs no provider request and reports stable, redacted codes.
Successful raw provider responses are kept only below the ignored, private
``test-output/cloud-ocr-bakeoff`` tree.  The JSON evidence writer emits only
content-free metrics and provenance: never OCR/ground-truth text, response
bodies, paths, endpoints, project/account identifiers, or credentials.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import http.client
import io
import json
import math
import os
import re
import shutil
import stat
import statistics
import subprocess
import sys
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ElementTree
from dataclasses import dataclass, field, replace
from datetime import date
from decimal import Decimal, InvalidOperation
from fractions import Fraction
from pathlib import Path
from typing import Callable, Iterable, Mapping, Protocol, Sequence

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cgpg  # noqa: E402
import metrics as gold_metrics  # noqa: E402
import run_cgpg  # noqa: E402
import run_cgpg_model_bakeoff as prior_bakeoff  # noqa: E402


REPO_ROOT = Path(__file__).resolve().parents[3]
RAW_OUTPUT_ROOT = REPO_ROOT / "test-output/cloud-ocr-bakeoff"
DEFAULT_PREREGISTRATION = (
    REPO_ROOT
    / "docs/evidence/cloud-complete-ocr-bakeoff-preregistration-2026-08-30.json"
)
PREREGISTRATION_SHA256 = (
    "a5112e276378c264d9896ff9038501dabaf6d25a5ad5aa77938d53300d0f0e3d"
)

SCORABLE_PAGE_COUNT = 12
CORPUS_PAGE_COUNT = 304
CORPUS_SCORABLE_PAGE_COUNT = 264
CORPUS_DEVELOPMENT_PAGE_COUNT = 227
CORPUS_HOLDOUT_PAGE_COUNT = 77
CORPUS_SCORABLE_HOLDOUT_PAGE_COUNT = 63
REPETITIONS = 3
PROVIDERS = ("google_vertex_gemini", "azure_document_intelligence")

CORPUS_DOI = "10.5281/zenodo.20008699"
CORPUS_ARCHIVE_SHA256 = (
    "2ee5d79f3c781dc1b64fa386f0f194a762ab183cd36b97f5d873ce0a3004e1f7"
)
CORPUS_LICENSE = "CC BY 4.0"
LEGACY_AUDIT_MANIFEST_SHA256 = (
    "07b330a2da29193c5cfebdd3c807649eb2b53ff50a89f260f1abe7f9c98d77fa"
)
FORMAL_INPUT_MANIFEST_SHA256 = (
    "6ab387adc9da8c55d227d3ef88dfe47af9e9f1c757b2f974ba06d8bf0eb4a751"
)

GEMINI_MODEL = "gemini-3.7-flash"
GEMINI_LOCATION = "global"
GEMINI_API_VERSION = "v1"
GEMINI_MAX_OUTPUT_TOKENS = 4096
GEMINI_THINKING_LEVEL = "MEDIUM"
AZURE_MODEL = "prebuilt-read"
AZURE_API_VERSION = "2024-11-30"

MAX_RESPONSE_BYTES = 32 * 1024 * 1024
MAX_PAGE_TEXT_CHARS = 4096
MAX_CER_CELL_UPDATES = 20_000_000
MAX_FIXED_REFERENCE_TEXT_CHARS = 2040
MAX_LINES = 256
MAX_GEMINI_PARTS = 8
MAX_WORDS = 4096
HTTP_TIMEOUT_SECONDS = 180.0
AZURE_POLL_TIMEOUT_SECONDS = 300.0
AZURE_MAX_POLLS = 120
AZURE_POLL_INTERVAL_SECONDS = 1.0

MAX_POLYTONIC_CER = 0.15
MAX_BASE_LETTER_CER = 0.12
MIN_BOX_F1_AT_050 = 0.85
MIN_BOX_F1_AT_075 = 0.60
MIN_MATCHED_MEAN_IOU = 0.70
MIN_READING_ORDER_PAIRWISE_ACCURACY = 0.99
MIN_READING_ORDER_GT_COVERAGE = 0.85
MIN_STABILITY_BOX_F1_AT_050 = 0.90
MIN_STABILITY_ORDER_PAIRWISE_ACCURACY = 0.995
MAX_STABILITY_INTER_RUN_TEXT_CER = 0.02
MAX_STABILITY_RELATIVE_LINE_COUNT_DELTA = 0.05

GEMINI_INPUT_USD_PER_MILLION = Decimal("0.75")
GEMINI_CACHED_INPUT_USD_PER_MILLION = Decimal("0.075")
GEMINI_OUTPUT_AND_THINKING_USD_PER_MILLION = Decimal("3.75")
AZURE_READ_USD_PER_THOUSAND_PAGES = Decimal("1.50")
REQUIRED_MAX_COST_USD = Decimal("1.00")
PLANNED_TARIFF_CEILING_USD = Decimal("0.746064")

LIVE_ENV = "MPDF_CLOUD_BAKEOFF_LIVE"
GOOGLE_PROJECT_ENV = "MPDF_BAKEOFF_GOOGLE_PROJECT"
AZURE_ENDPOINT_ENV = "MPDF_BAKEOFF_AZURE_ENDPOINT"
AZURE_KEY_ENV = "MPDF_BAKEOFF_AZURE_KEY"
UPLOAD_ACKNOWLEDGEMENT = (
    "I acknowledge CGPG JPEG upload to Google Vertex AI and Microsoft Azure "
    "Document Intelligence under the configured retention policies."
)

_PROJECT_ID_PATTERN = re.compile(r"[a-z][a-z0-9-]{4,28}[a-z0-9]\Z")
_STABLE_CODE_PATTERN = re.compile(r"[a-z][a-z0-9_]{1,95}\Z")
_AZURE_RESOURCE_HOST_PATTERN = re.compile(
    r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.cognitiveservices\.azure\.com\Z"
)


class BakeoffError(RuntimeError):
    """A stable, redacted bake-off failure."""

    def __init__(
        self,
        code: str,
        *,
        invalid_boxes: int = 0,
        degenerate_boxes: int = 0,
        out_of_page_boxes: int = 0,
        duplicate_boxes: int = 0,
    ):
        if not _STABLE_CODE_PATTERN.fullmatch(code):
            raise ValueError("BakeoffError requires a stable code")
        super().__init__(code)
        self.code = code
        self.degenerate_boxes = degenerate_boxes
        self.out_of_page_boxes = out_of_page_boxes
        self.duplicate_boxes = duplicate_boxes
        self.invalid_boxes = (
            invalid_boxes
            + degenerate_boxes
            + out_of_page_boxes
            + duplicate_boxes
        )
        self.raw_response: bytes | None = None
        self.latency_seconds: float | None = None
        self.usage: ProviderUsage | None = None
        self.provenance: ProviderProvenance | None = None
        self.provider_response_success = False
        self.http_status: int | None = None


@dataclass(frozen=True)
class Box:
    left: float
    top: float
    right: float
    bottom: float

    @property
    def width(self) -> float:
        return self.right - self.left

    @property
    def height(self) -> float:
        return self.bottom - self.top

    @property
    def centre(self) -> tuple[float, float]:
        return ((self.left + self.right) / 2.0, (self.top + self.bottom) / 2.0)

    def as_xywh(self) -> dict[str, float]:
        return {
            "x": self.left,
            "y": self.top,
            "width": self.width,
            "height": self.height,
        }


@dataclass(frozen=True)
class ProviderLine:
    text: str
    bbox: Box
    reading_order: int


@dataclass(frozen=True)
class ProviderUsage:
    prompt_tokens: int = 0
    candidate_tokens: int = 0
    thought_tokens: int = 0
    cached_tokens: int = 0
    accepted_pages: int = 0
    billable_pages: int = 0
    native_words: int = 0
    confidence_count: int = 0
    confidence_sum: float = 0.0
    confidence_min: float | None = None
    confidence_max: float | None = None


@dataclass(frozen=True)
class ProviderProvenance:
    model_version: str | None = None
    response_id_sha256: str | None = None
    model_id: str | None = None
    api_version: str | None = None


@dataclass(frozen=True)
class ProviderResult:
    lines: tuple[ProviderLine, ...]
    usage: ProviderUsage
    latency_seconds: float
    raw_response: bytes = field(repr=False)
    provenance: ProviderProvenance = field(default_factory=ProviderProvenance)


@dataclass(frozen=True)
class HttpResponse:
    status: int
    headers: tuple[tuple[str, str], ...]
    body: bytes = field(repr=False)

    def header_values(self, name: str) -> list[str]:
        lowered = name.lower()
        return [value for key, value in self.headers if key.lower() == lowered]


class HttpTransport(Protocol):
    def request(
        self,
        method: str,
        url: str,
        headers: Mapping[str, str],
        body: bytes | None,
        timeout_seconds: float,
    ) -> HttpResponse: ...


class NoRedirectHandler(urllib.request.HTTPRedirectHandler):
    """Reject every redirect, including same-origin redirects."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: ANN001
        return None


class UrllibTransport:
    """Bounded HTTPS transport with redirects and retries disabled."""

    def __init__(self) -> None:
        self._opener = urllib.request.build_opener(NoRedirectHandler())

    def request(
        self,
        method: str,
        url: str,
        headers: Mapping[str, str],
        body: bytes | None,
        timeout_seconds: float,
    ) -> HttpResponse:
        if (
            not url
            or len(url) > 16_384
            or any(
                character.isspace() or ord(character) < 32 or ord(character) == 127
                for character in url
            )
        ):
            raise BakeoffError("transport_failed")
        response = None
        try:
            request = urllib.request.Request(url, data=body, method=method)
            for name, value in headers.items():
                if "\r" in value or "\n" in value:
                    raise BakeoffError("unsafe_request_header")
                if name.lower() in ("authorization", "ocp-apim-subscription-key"):
                    request.add_unredirected_header(name, value)
                else:
                    request.add_header(name, value)
            response = self._opener.open(request, timeout=timeout_seconds)
            payload = _read_bounded(response, MAX_RESPONSE_BYTES)
            return HttpResponse(
                int(response.status), tuple(response.headers.items()), payload
            )
        except urllib.error.HTTPError as error:
            # Provider error bodies are deliberately never read or surfaced.
            try:
                code = int(error.code)
            finally:
                try:
                    error.close()
                except (OSError, http.client.HTTPException):
                    raise BakeoffError("transport_failed") from None
            return HttpResponse(code, (), b"")
        except BakeoffError as error:
            if response is not None:
                try:
                    error.http_status = int(response.status)
                except (AttributeError, TypeError, ValueError):
                    pass
            raise
        except (
            OSError,
            urllib.error.URLError,
            TimeoutError,
            ValueError,
            http.client.HTTPException,
        ):
            raise BakeoffError("transport_failed") from None
        finally:
            if response is not None:
                try:
                    response.close()
                except (OSError, http.client.HTTPException):
                    raise BakeoffError("transport_failed") from None


def _read_bounded(handle, limit: int) -> bytes:  # noqa: ANN001
    chunks: list[bytes] = []
    remaining = limit + 1
    while remaining:
        chunk = handle.read(min(1024 * 1024, remaining))
        if not chunk:
            break
        chunks.append(chunk)
        remaining -= len(chunk)
    payload = b"".join(chunks)
    if len(payload) > limit:
        raise BakeoffError("response_too_large")
    return payload


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError:
        raise BakeoffError("input_file_unreadable") from None
    return digest.hexdigest()


def canonical_json_bytes(value: object) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")


def canonical_digest(value: object) -> str:
    return sha256_bytes(canonical_json_bytes(value))


def _reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def _reject_nonfinite_json(_value: str) -> None:
    raise ValueError("non-finite JSON number")


def strict_json_loads(raw: bytes, *, maximum_bytes: int = MAX_RESPONSE_BYTES) -> object:
    if len(raw) > maximum_bytes:
        raise BakeoffError("response_too_large")
    try:
        text = raw.decode("utf-8", errors="strict")
        return json.loads(
            text,
            object_pairs_hook=_reject_duplicate_keys,
            parse_constant=_reject_nonfinite_json,
        )
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError, ValueError):
        raise BakeoffError("response_json_invalid") from None


def _strict_int(value: object) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise BakeoffError("response_schema_invalid")
    return value


def _strict_number(value: object) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise BakeoffError("response_schema_invalid")
    try:
        number = float(value)
    except (OverflowError, ValueError):
        raise BakeoffError("response_schema_invalid") from None
    if not math.isfinite(number):
        raise BakeoffError("response_schema_invalid")
    return number


FIXED_INPUT_RECORDS: tuple[dict, ...] = (
    {"page_id": "grc_grna_or_399", "width": 1652, "height": 3945, "image": {"mime_type": "image/jpeg", "sha256": "4520bcb8c92c420b78d3e74e15d0f28a80d67a723ff87cc92ec3231137ebd5bd", "size_bytes": 962559}, "ground_truth_page_xml": {"sha256": "ac3d20a8f79820f4be3404a2c1037885b568d4ca4bff68ede3d6a3a418057555", "size_bytes": 36067}},
    {"page_id": "grc_grna_or_409", "width": 1686, "height": 3506, "image": {"mime_type": "image/jpeg", "sha256": "cdfaa107c3d44aef0668dc0d2a0d72f63190baba9dbcffe589aac7accc2adb03", "size_bytes": 930799}, "ground_truth_page_xml": {"sha256": "6d874dd3645b4556b2c0cd7bb3220b11a16cdc6f1675a1b53c1efad82114f367", "size_bytes": 31178}},
    {"page_id": "grc_grna_or_423", "width": 1760, "height": 3534, "image": {"mime_type": "image/jpeg", "sha256": "8e67f7681393b3b32e778f7a04bef33a4feaf54842500bfc9d8ac354e601c38e", "size_bytes": 1028036}, "ground_truth_page_xml": {"sha256": "f0a9eb5da57c5ce75ec7103106d56cea9b9299db976bd2b6cd98b6b7eaf5212b", "size_bytes": 31924}},
    {"page_id": "grc_grna_or_429", "width": 1804, "height": 3933, "image": {"mime_type": "image/jpeg", "sha256": "bc687db3bfc90957e7cbffe3ae0c97881acd49be1934bea4e496f401b0ae1b72", "size_bytes": 1054137}, "ground_truth_page_xml": {"sha256": "d44075dd3c9c694c1829275f55ed15e9a4b9a8451266b62316c0d64febc58290", "size_bytes": 36739}},
    {"page_id": "grc_grna_or_443", "width": 1717, "height": 3478, "image": {"mime_type": "image/jpeg", "sha256": "dca6f5ab1849394ed76110b0eb4cad8d567a967083fdb0ba24a1621241704b8f", "size_bytes": 991279}, "ground_truth_page_xml": {"sha256": "e93cde9b5aa46fbfef8b98274c383458e9dc620be60bfda758af377108df1dbc", "size_bytes": 30812}},
    {"page_id": "grc_grna_or_447", "width": 1745, "height": 3189, "image": {"mime_type": "image/jpeg", "sha256": "34bb2be6e9e5f5452398845aa607bc93bb98fe7c58672c8dbc114b227399691a", "size_bytes": 1032274}, "ground_truth_page_xml": {"sha256": "f5758518a1bd359ce66da131b4b9efe7122c74924bc66db8a7fea47fe1d21c0f", "size_bytes": 29703}},
    {"page_id": "grc_grna_or_461", "width": 1740, "height": 3940, "image": {"mime_type": "image/jpeg", "sha256": "56f3f91ebcf82b586e2664c68b4bbb1fd8744dd8d4bb100d7905df26299e7450", "size_bytes": 1063234}, "ground_truth_page_xml": {"sha256": "1b5dcac95617de8bac8b29c498bf677d6fae8bfd7358ac886ae1f5fe45a80d58", "size_bytes": 37042}},
    {"page_id": "grc_grna_or_475", "width": 1723, "height": 4022, "image": {"mime_type": "image/jpeg", "sha256": "aceba49059317b99885d1bcd32f25f8f5a68d4026639b47e0e0db24929571dc1", "size_bytes": 1243126}, "ground_truth_page_xml": {"sha256": "bfeb76a24b02e0e768fa3d57f6da8c7a6f2d9f6fb0ba2c3cc4b8f61c69dd613d", "size_bytes": 36373}},
    {"page_id": "grc_grna_or_487", "width": 1719, "height": 3297, "image": {"mime_type": "image/jpeg", "sha256": "70debdd430ad2e88d165709bb66dffff9b8380320996ed070a45fc970ccaf52a", "size_bytes": 820307}, "ground_truth_page_xml": {"sha256": "9550bf1cd87bd7b086bcf4a10d8b734f52a311607b28e6097d1c9e0078d74d1d", "size_bytes": 27561}},
    {"page_id": "grc_grna_or_489", "width": 1767, "height": 3716, "image": {"mime_type": "image/jpeg", "sha256": "090c588132f4a10def4c237446717fb02370244163dff1e1c160abd4820cdd15", "size_bytes": 928220}, "ground_truth_page_xml": {"sha256": "4036476568581561ff84ba45beea3dccd3f43a45f942b93c278b1f4e50e84192", "size_bytes": 31524}},
    {"page_id": "grc_grna_or_503", "width": 1779, "height": 3654, "image": {"mime_type": "image/jpeg", "sha256": "4019cdcf28f5891b72d9ca0140c425a42d3b14dd573266de8302ab2775b5ed9f", "size_bytes": 1013152}, "ground_truth_page_xml": {"sha256": "a4307689ea5e07e3d6c9cf3f8ca319f9f57bf81694d04f8b7de6ad80a3721fb9", "size_bytes": 32906}},
    {"page_id": "grc_grna_or_513", "width": 1767, "height": 3606, "image": {"mime_type": "image/jpeg", "sha256": "467d513fdc6bf6ba1e371abd51d8c7ca52f18ee91739736e3e077a959d3e2fe4", "size_bytes": 1150767}, "ground_truth_page_xml": {"sha256": "f12ceff71e4c3b9b78d21d7b174e28d09ed88d9aacfdd3c8026d76a580f571c7", "size_bytes": 34314}},
)


def formal_input_manifest(records: Sequence[dict] = FIXED_INPUT_RECORDS) -> dict:
    return {
        "schema": "mpdf-cloud-complete-ocr-input-manifest",
        "schema_version": "1.0",
        "pages": list(records),
    }


def legacy_audit_manifest(pages: Sequence[cgpg.Page]) -> list[list[object]]:
    records: list[list[object]] = []
    for page in pages:
        for kind, path in (("image", page.image_path), ("xml", page.xml_path)):
            records.append(
                [page.name, kind, sha256_file(path), path.stat().st_size]
            )
    return records


def legacy_manifest_digest(pages: Sequence[cgpg.Page]) -> str:
    # This deliberately reproduces the earlier ad-hoc audit serialization.
    encoded = json.dumps(
        legacy_audit_manifest(pages), separators=(",", ":")
    ).encode("utf-8")
    return sha256_bytes(encoded)


def _parse_verified_page_xml(
    xml_path: Path, raw: bytes, expected: Mapping[str, object]
) -> cgpg.Page:
    """Parse the exact XML byte buffer whose manifest digest was verified."""
    try:
        cgpg._reject_unsafe_xml(raw)
        root = ElementTree.fromstring(raw)
        page_element = root.find("p:Page", cgpg._NS)
        if page_element is None:
            raise cgpg.CgpgError("missing PAGE Page element")
        width = int(page_element.get("imageWidth", ""))
        height = int(page_element.get("imageHeight", ""))
        if (
            width != expected["width"]
            or height != expected["height"]
            or not (0 < width <= cgpg.MAX_PAGE_PIXELS)
            or not (0 < height <= cgpg.MAX_PAGE_PIXELS)
        ):
            raise cgpg.CgpgError("PAGE dimensions differ from manifest")
        image_path = cgpg._resolve_image(
            xml_path, page_element.get("imageFilename", "")
        )
        if image_path is None:
            raise cgpg.CgpgError("PAGE image is missing")

        regions: list[cgpg.Region] = []
        total_lines = 0
        for region_element in page_element.findall("p:TextRegion", cgpg._NS):
            if len(regions) >= cgpg.MAX_REGIONS_PER_PAGE:
                raise cgpg.CgpgError("too many PAGE regions")
            coords = region_element.find("p:Coords", cgpg._NS)
            if coords is None:
                continue
            region_box = cgpg._polygon_bbox(
                coords.get("points", ""), width, height
            )
            if region_box is None:
                continue
            lines: list[cgpg.Line] = []
            for line_element in region_element.findall("p:TextLine", cgpg._NS):
                total_lines += 1
                if total_lines > cgpg.MAX_LINES_PER_PAGE:
                    raise cgpg.CgpgError("too many PAGE lines")
                line_coords = line_element.find("p:Coords", cgpg._NS)
                if line_coords is None:
                    continue
                line_box = cgpg._polygon_bbox(
                    line_coords.get("points", ""), width, height
                )
                if line_box is None:
                    continue
                unicode_element = line_element.find(
                    ".//p:TextEquiv/p:Unicode", cgpg._NS
                )
                text = (
                    unicode_element.text or ""
                    if unicode_element is not None
                    else ""
                )
                lines.append(
                    cgpg.Line(
                        unicodedata.normalize("NFC", text).strip(), line_box
                    )
                )
            regions.append(
                cgpg.Region(
                    region_element.get("id", ""),
                    cgpg._region_type(region_element),
                    region_box,
                    tuple(lines),
                )
            )
    except (ElementTree.ParseError, TypeError, ValueError, cgpg.CgpgError):
        raise BakeoffError("fixed_page_xml_invalid") from None
    return cgpg.Page(
        name=xml_path.stem,
        image_path=image_path,
        xml_path=xml_path,
        width=width,
        height=height,
        regions=tuple(regions),
    )


def _legacy_digest_from_verified_records(records: Sequence[Mapping[str, object]]) -> str:
    rows: list[list[object]] = []
    for record in records:
        image = record["image"]
        xml = record["ground_truth_page_xml"]
        if not isinstance(image, Mapping) or not isinstance(xml, Mapping):
            raise BakeoffError("formal_input_manifest_digest_mismatch")
        rows.extend(
            [
                [
                    record["page_id"],
                    "image",
                    image["sha256"],
                    image["size_bytes"],
                ],
                [
                    record["page_id"],
                    "xml",
                    xml["sha256"],
                    xml["size_bytes"],
                ],
            ]
        )
    return sha256_bytes(json.dumps(rows, separators=(",", ":")).encode("utf-8"))


def select_and_verify_fixed_inputs(
    corpus_root: Path, archive_path: Path
) -> list[cgpg.Page]:
    try:
        if sha256_file(archive_path) != CORPUS_ARCHIVE_SHA256:
            raise BakeoffError("corpus_archive_digest_mismatch")
        all_pages = cgpg.load_corpus(corpus_root, verify_image=True)
        development, holdout = cgpg.split_corpus(all_pages)
        pages = prior_bakeoff.select_fixed_pages(all_pages)
    except BakeoffError:
        raise
    except (cgpg.CgpgError, prior_bakeoff.BakeoffError, OSError):
        raise BakeoffError("corpus_validation_failed") from None

    corpus_counts = (
        len(all_pages),
        sum(bool(page.transcribed_lines) for page in all_pages),
        len(development),
        len(holdout),
        sum(bool(page.transcribed_lines) for page in holdout),
    )
    if corpus_counts != (
        CORPUS_PAGE_COUNT,
        CORPUS_SCORABLE_PAGE_COUNT,
        CORPUS_DEVELOPMENT_PAGE_COUNT,
        CORPUS_HOLDOUT_PAGE_COUNT,
        CORPUS_SCORABLE_HOLDOUT_PAGE_COUNT,
    ):
        raise BakeoffError("corpus_inventory_mismatch")

    expected_ids = [record["page_id"] for record in FIXED_INPUT_RECORDS]
    if [page.name for page in pages] != expected_ids:
        raise BakeoffError("fixed_page_selection_mismatch")
    actual_records: list[dict] = []
    verified_pages: list[cgpg.Page] = []
    for expected in FIXED_INPUT_RECORDS:
        xml_path = corpus_root / f"{expected['page_id']}.xml"
        try:
            xml_bytes = xml_path.read_bytes()
        except OSError:
            raise BakeoffError("fixed_page_unreadable") from None
        if (
            len(xml_bytes)
            != expected["ground_truth_page_xml"]["size_bytes"]
            or sha256_bytes(xml_bytes)
            != expected["ground_truth_page_xml"]["sha256"]
        ):
            raise BakeoffError("fixed_page_manifest_mismatch")
        page = _parse_verified_page_xml(xml_path, xml_bytes, expected)
        try:
            image_bytes = page.image_path.read_bytes()
        except OSError:
            raise BakeoffError("fixed_page_unreadable") from None
        actual = {
            "page_id": page.name,
            "width": page.width,
            "height": page.height,
            "image": {
                "mime_type": "image/jpeg",
                "sha256": sha256_bytes(image_bytes),
                "size_bytes": len(image_bytes),
            },
            "ground_truth_page_xml": {
                "sha256": sha256_bytes(xml_bytes),
                "size_bytes": len(xml_bytes),
            },
        }
        if actual != expected:
            raise BakeoffError("fixed_page_manifest_mismatch")
        if image_bytes[:3] != b"\xff\xd8\xff":
            raise BakeoffError("fixed_page_not_jpeg")
        try:
            from PIL import Image  # noqa: PLC0415

            with Image.open(io.BytesIO(image_bytes)) as image:
                if image.size != (page.width, page.height):
                    raise BakeoffError("fixed_page_dimension_mismatch")
        except BakeoffError:
            raise
        except (OSError, ValueError):
            raise BakeoffError("fixed_page_not_jpeg") from None
        actual_records.append(actual)
        verified_pages.append(page)

    if canonical_digest(formal_input_manifest(actual_records)) != FORMAL_INPUT_MANIFEST_SHA256:
        raise BakeoffError("formal_input_manifest_digest_mismatch")
    legacy_digest = _legacy_digest_from_verified_records(actual_records)
    if legacy_digest != LEGACY_AUDIT_MANIFEST_SHA256:
        raise BakeoffError("legacy_input_manifest_digest_mismatch")
    maximum_reference_chars = max(
        len(
            "\n".join(
                line.text
                for region in page.regions
                if region.text.strip()
                for line in region.lines
                if line.text
            )
        )
        for page in verified_pages
    )
    if maximum_reference_chars != MAX_FIXED_REFERENCE_TEXT_CHARS:
        raise BakeoffError("fixed_reference_inventory_mismatch")
    return verified_pages


GEMINI_PROMPT = """Act as the sole OCR and geometry source for this one JPEG page.
Return every visible printed text line exactly once, preserving polytonic Greek,
punctuation, capitalization, and natural single-page reading order. Do not use
Markdown, commentary, inferred corrections, or prose outside the requested JSON.
For each complete line return its text, a tight axis-aligned bounding box as
[ymin,xmin,ymax,xmax] integers normalized to 0..1000 from the top-left image
origin, and a zero-based contiguous reading_order. Do not merge separate lines.
Do not split one printed line merely because of inter-word spacing.
"""
GEMINI_PROMPT_SHA256 = sha256_bytes(GEMINI_PROMPT.encode("utf-8"))

GEMINI_RESPONSE_SCHEMA = {
    "type": "OBJECT",
    "propertyOrdering": ["lines"],
    "properties": {
        "lines": {
            "type": "ARRAY",
            "minItems": 1,
            "maxItems": MAX_LINES,
            "items": {
                "type": "OBJECT",
                "propertyOrdering": ["text", "bbox", "reading_order"],
                "properties": {
                    "text": {"type": "STRING"},
                    "bbox": {
                        "type": "ARRAY",
                        "minItems": 4,
                        "maxItems": 4,
                        "items": {
                            "type": "INTEGER",
                            "minimum": 0,
                            "maximum": 1000,
                        },
                    },
                    "reading_order": {
                        "type": "INTEGER",
                        "minimum": 0,
                        "maximum": MAX_LINES - 1,
                    },
                },
                "required": ["text", "bbox", "reading_order"],
            },
        }
    },
    "required": ["lines"],
}


def _validate_line_set(
    lines: Sequence[ProviderLine], width: int, height: int
) -> tuple[ProviderLine, ...]:
    if not lines or len(lines) > MAX_LINES:
        raise BakeoffError("response_line_count_invalid")
    invalid = 0
    degenerate = 0
    out_of_page = 0
    seen_boxes: set[tuple[float, float, float, float]] = set()
    duplicate_boxes = 0
    total_chars = 0
    for expected_order, line in enumerate(lines):
        if line.reading_order != expected_order:
            raise BakeoffError("reading_order_schema_invalid")
        total_chars += len(line.text)
        if (
            not line.text.strip()
            or any(
                character in "\r\n"
                or unicodedata.category(character) in {"Cc", "Zl", "Zp"}
                for character in line.text
            )
            or total_chars > MAX_PAGE_TEXT_CHARS
        ):
            raise BakeoffError("response_text_invalid")
        box = line.bbox
        coordinates = (box.left, box.top, box.right, box.bottom)
        if any(not math.isfinite(value) for value in coordinates):
            invalid += 1
        elif box.width <= 0.0 or box.height <= 0.0:
            degenerate += 1
        elif (
            box.left < 0.0
            or box.top < 0.0
            or box.right > width
            or box.bottom > height
        ):
            out_of_page += 1
        if coordinates in seen_boxes:
            duplicate_boxes += 1
        seen_boxes.add(coordinates)
    if invalid or degenerate or out_of_page:
        raise BakeoffError(
            "invalid_provider_boxes",
            invalid_boxes=invalid,
            degenerate_boxes=degenerate,
            out_of_page_boxes=out_of_page,
            duplicate_boxes=duplicate_boxes,
        )
    if duplicate_boxes:
        raise BakeoffError(
            "exact_duplicate_provider_boxes", duplicate_boxes=duplicate_boxes
        )
    return tuple(lines)


def parse_gemini_lines(value: object, width: int, height: int) -> tuple[ProviderLine, ...]:
    if not isinstance(value, dict) or set(value) != {"lines"}:
        raise BakeoffError("response_schema_invalid")
    records = value["lines"]
    if not isinstance(records, list) or not (1 <= len(records) <= MAX_LINES):
        raise BakeoffError("response_line_count_invalid")
    lines: list[ProviderLine] = []
    for record in records:
        if not isinstance(record, dict) or set(record) != {
            "text",
            "bbox",
            "reading_order",
        }:
            raise BakeoffError("response_schema_invalid")
        text = record["text"]
        bbox = record["bbox"]
        if not isinstance(text, str) or not isinstance(bbox, list) or len(bbox) != 4:
            raise BakeoffError("response_schema_invalid")
        ymin, xmin, ymax, xmax = (_strict_int(item) for item in bbox)
        if not all(0 <= item <= 1000 for item in (ymin, xmin, ymax, xmax)):
            raise BakeoffError(
                "invalid_provider_boxes", out_of_page_boxes=1
            )
        order = _strict_int(record["reading_order"])
        lines.append(
            ProviderLine(
                text=text,
                bbox=Box(
                    left=xmin * width / 1000.0,
                    top=ymin * height / 1000.0,
                    right=xmax * width / 1000.0,
                    bottom=ymax * height / 1000.0,
                ),
                reading_order=order,
            )
        )
    return _validate_line_set(lines, width, height)


def _usage_integer(metadata: dict, key: str, *, required: bool = False) -> int:
    if key not in metadata and not required:
        return 0
    value = _strict_int(metadata.get(key))
    if value < 0:
        raise BakeoffError("response_usage_invalid")
    return value


def parse_gemini_usage(envelope: object) -> ProviderUsage:
    if not isinstance(envelope, dict):
        raise BakeoffError("response_usage_invalid")
    usage_metadata = envelope.get("usageMetadata")
    if not isinstance(usage_metadata, dict):
        raise BakeoffError("response_usage_invalid")
    prompt = _usage_integer(usage_metadata, "promptTokenCount", required=True)
    candidate_tokens = _usage_integer(
        usage_metadata, "candidatesTokenCount", required=True
    )
    thoughts = _usage_integer(usage_metadata, "thoughtsTokenCount")
    cached = _usage_integer(usage_metadata, "cachedContentTokenCount")
    if cached > prompt:
        raise BakeoffError("response_usage_invalid")
    return ProviderUsage(
        prompt_tokens=prompt,
        candidate_tokens=candidate_tokens,
        thought_tokens=thoughts,
        cached_tokens=cached,
    )


_GEMINI_MODEL_VERSION_PATTERN = re.compile(
    rf"{re.escape(GEMINI_MODEL)}(?:-[A-Za-z0-9][A-Za-z0-9._-]{{0,127}})?"
)


def _valid_gemini_model_version(value: object) -> bool:
    return isinstance(value, str) and bool(
        _GEMINI_MODEL_VERSION_PATTERN.fullmatch(value)
    )


def parse_gemini_provenance(envelope: object) -> ProviderProvenance:
    if not isinstance(envelope, dict):
        raise BakeoffError("gemini_response_provenance_invalid")
    model_version = envelope.get("modelVersion")
    response_id = envelope.get("responseId")
    if (
        not _valid_gemini_model_version(model_version)
        or not isinstance(response_id, str)
        or not response_id
        or len(response_id) > 1024
        or any(character.isspace() for character in response_id)
    ):
        raise BakeoffError("gemini_response_provenance_invalid")
    return ProviderProvenance(
        model_version=model_version,
        response_id_sha256=sha256_bytes(response_id.encode("utf-8")),
    )


def parse_gemini_response(
    raw: bytes, width: int, height: int, latency_seconds: float = 0.0
) -> ProviderResult:
    envelope = strict_json_loads(raw)
    if not isinstance(envelope, dict):
        raise BakeoffError("response_schema_invalid")
    candidates = envelope.get("candidates")
    if not isinstance(candidates, list) or len(candidates) != 1:
        raise BakeoffError("gemini_candidate_invalid")
    candidate = candidates[0]
    if not isinstance(candidate, dict):
        raise BakeoffError("gemini_candidate_invalid")
    if candidate.get("finishReason") != "STOP":
        raise BakeoffError("gemini_finish_reason_invalid")
    provenance = parse_gemini_provenance(envelope)
    content = candidate.get("content")
    if not isinstance(content, dict):
        raise BakeoffError("gemini_candidate_invalid")
    parts = content.get("parts")
    if not isinstance(parts, list) or not (1 <= len(parts) <= MAX_GEMINI_PARTS):
        raise BakeoffError("gemini_candidate_invalid")
    payloads: list[str] = []
    for part_index, part in enumerate(parts):
        if not isinstance(part, dict) or not part or not set(part).issubset(
            {"text", "thought", "thoughtSignature"}
        ):
            raise BakeoffError("gemini_candidate_invalid")
        thought = part.get("thought", False)
        if not isinstance(thought, bool):
            raise BakeoffError("gemini_candidate_invalid")
        if "thoughtSignature" in part and (
            not isinstance(part["thoughtSignature"], str)
            or not part["thoughtSignature"]
            or len(part["thoughtSignature"]) > 1024 * 1024
        ):
            raise BakeoffError("gemini_candidate_invalid")
        if "text" in part:
            if not isinstance(part["text"], str):
                raise BakeoffError("gemini_candidate_invalid")
            if part["text"] and not thought:
                payloads.append(part["text"])
            elif not part["text"] and not thought and part_index != len(parts) - 1:
                raise BakeoffError("gemini_candidate_invalid")
        elif "thoughtSignature" not in part and not thought:
            raise BakeoffError("gemini_candidate_invalid")
    if len(payloads) != 1:
        raise BakeoffError("gemini_candidate_invalid")
    structured = strict_json_loads(
        payloads[0].encode("utf-8"), maximum_bytes=MAX_RESPONSE_BYTES
    )
    lines = parse_gemini_lines(structured, width, height)

    usage = parse_gemini_usage(envelope)
    return ProviderResult(
        lines=lines,
        usage=usage,
        latency_seconds=latency_seconds,
        raw_response=raw,
        provenance=provenance,
    )


def _azure_polygon_box(value: object) -> Box:
    if not isinstance(value, list) or len(value) != 8:
        raise BakeoffError("azure_polygon_invalid", invalid_boxes=1)
    numbers = [_strict_number(item) for item in value]
    xs = numbers[0::2]
    ys = numbers[1::2]
    return Box(min(xs), min(ys), max(xs), max(ys))


def parse_azure_provenance(envelope: object) -> ProviderProvenance:
    if not isinstance(envelope, dict):
        raise BakeoffError("azure_response_provenance_invalid")
    result = envelope.get("analyzeResult")
    if (
        not isinstance(result, dict)
        or result.get("modelId") != AZURE_MODEL
        or result.get("apiVersion") != AZURE_API_VERSION
    ):
        raise BakeoffError("azure_response_provenance_invalid")
    return ProviderProvenance(model_id=AZURE_MODEL, api_version=AZURE_API_VERSION)


def parse_azure_response(
    raw: bytes, width: int, height: int, latency_seconds: float = 0.0
) -> ProviderResult:
    envelope = strict_json_loads(raw)
    if not isinstance(envelope, dict) or envelope.get("status") != "succeeded":
        raise BakeoffError("azure_terminal_response_invalid")
    result = envelope.get("analyzeResult")
    if not isinstance(result, dict):
        raise BakeoffError("azure_terminal_response_invalid")
    provenance = parse_azure_provenance(envelope)
    pages = result.get("pages")
    if not isinstance(pages, list) or len(pages) != 1:
        raise BakeoffError("azure_page_count_invalid")
    page = pages[0]
    if not isinstance(page, dict):
        raise BakeoffError("azure_page_schema_invalid")
    page_width = _strict_number(page.get("width"))
    page_height = _strict_number(page.get("height"))
    if (
        str(page.get("unit", "")).lower() != "pixel"
        or abs(page_width - width) > 0.5
        or abs(page_height - height) > 0.5
    ):
        raise BakeoffError("azure_coordinate_space_invalid")
    records = page.get("lines")
    if not isinstance(records, list) or not (1 <= len(records) <= MAX_LINES):
        raise BakeoffError("response_line_count_invalid")
    lines: list[ProviderLine] = []
    for order, record in enumerate(records):
        if not isinstance(record, dict) or not isinstance(record.get("content"), str):
            raise BakeoffError("azure_line_schema_invalid")
        lines.append(
            ProviderLine(
                text=record["content"],
                bbox=_azure_polygon_box(record.get("polygon")),
                reading_order=order,
            )
        )
    validated = _validate_line_set(lines, width, height)

    words = page.get("words", [])
    if not isinstance(words, list) or len(words) > MAX_WORDS:
        raise BakeoffError("azure_words_invalid")
    confidences: list[float] = []
    word_characters = 0
    for word in words:
        if not isinstance(word, dict) or not isinstance(word.get("content"), str):
            raise BakeoffError("azure_words_invalid")
        # Retain native polygons and confidence through strict validation. Raw
        # response storage preserves the complete provider-native records.
        word_characters += len(word["content"])
        if (
            not word["content"].strip()
            or any(
                character in "\r\n"
                or unicodedata.category(character) in {"Cc", "Zl", "Zp"}
                for character in word["content"]
            )
            or word_characters > MAX_PAGE_TEXT_CHARS
        ):
            raise BakeoffError("azure_words_invalid")
        word_box = _azure_polygon_box(word.get("polygon"))
        if word_box.width <= 0.0 or word_box.height <= 0.0:
            raise BakeoffError(
                "azure_word_polygon_invalid", degenerate_boxes=1
            )
        if (
            word_box.left < 0.0
            or word_box.top < 0.0
            or word_box.right > width
            or word_box.bottom > height
        ):
            raise BakeoffError(
                "azure_word_polygon_invalid", out_of_page_boxes=1
            )
        if "confidence" in word:
            confidence = _strict_number(word["confidence"])
            if not 0.0 <= confidence <= 1.0:
                raise BakeoffError("azure_confidence_invalid")
            confidences.append(confidence)
    usage = ProviderUsage(
        accepted_pages=1,
        billable_pages=len(pages),
        native_words=len(words),
        confidence_count=len(confidences),
        confidence_sum=sum(confidences),
        confidence_min=min(confidences) if confidences else None,
        confidence_max=max(confidences) if confidences else None,
    )
    return ProviderResult(
        validated,
        usage,
        latency_seconds,
        raw,
        provenance,
    )


def _gemini_endpoint(project_id: str) -> str:
    quoted_project = urllib.parse.quote(project_id, safe="")
    return (
        "https://aiplatform.googleapis.com/v1/projects/"
        f"{quoted_project}/locations/{GEMINI_LOCATION}/publishers/google/models/"
        f"{GEMINI_MODEL}:generateContent"
    )


def _gemini_model_resource(project_id: str) -> str:
    return (
        f"projects/{project_id}/locations/{GEMINI_LOCATION}/publishers/google/"
        f"models/{GEMINI_MODEL}"
    )


def _gemini_count_tokens_endpoint(project_id: str) -> str:
    return f"{_gemini_endpoint(project_id).removesuffix(':generateContent')}:countTokens"


class GeminiClient:
    def __init__(
        self,
        project_id: str,
        bearer_token: str,
        transport: HttpTransport,
        clock: Callable[[], float] = time.perf_counter,
    ) -> None:
        self._project_id = project_id
        self._bearer_token = bearer_token
        self._transport = transport
        self._clock = clock

    def recognize(self, image_bytes: bytes, width: int, height: int) -> ProviderResult:
        request_body = canonical_json_bytes(
            {
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            {"text": GEMINI_PROMPT},
                            {
                                "inlineData": {
                                    "mimeType": "image/jpeg",
                                    "data": base64.b64encode(image_bytes).decode("ascii"),
                                }
                            },
                        ],
                    }
                ],
                "generationConfig": {
                    "maxOutputTokens": GEMINI_MAX_OUTPUT_TOKENS,
                    "responseMimeType": "application/json",
                    "responseSchema": GEMINI_RESPONSE_SCHEMA,
                    "thinkingConfig": {"thinkingLevel": GEMINI_THINKING_LEVEL},
                },
            }
        )
        endpoint = _gemini_endpoint(self._project_id)
        headers = {
            "Authorization": f"Bearer {self._bearer_token}",
            "Content-Type": "application/json; charset=utf-8",
            "Accept": "application/json",
        }
        started = self._clock()
        try:
            response = self._transport.request(
                "POST",
                endpoint,
                headers,
                request_body,
                HTTP_TIMEOUT_SECONDS,
            )
        except BakeoffError as error:
            if error.code == "transport_failed":
                error = BakeoffError("gemini_outcome_ambiguous")
            elif error.http_status == 200:
                error.provider_response_success = True
            error.latency_seconds = self._clock() - started
            raise error from None
        if response.status != 200:
            error = BakeoffError(
                f"gemini_http_{response.status}"
                if response.status
                in (400, 401, 403, 404, 408, 409, 429, 500, 502, 503, 504)
                else "gemini_http_error"
            )
            error.latency_seconds = self._clock() - started
            raise error
        try:
            result = parse_gemini_response(response.body, width, height)
        except BakeoffError as error:
            error.raw_response = response.body
            error.provider_response_success = True
            error.latency_seconds = self._clock() - started
            try:
                envelope = strict_json_loads(response.body)
                error.usage = parse_gemini_usage(envelope)
                error.provenance = parse_gemini_provenance(envelope)
            except BakeoffError:
                pass
            raise
        return replace(result, latency_seconds=self._clock() - started)


def _validated_azure_base(endpoint: str) -> urllib.parse.SplitResult:
    if (
        not endpoint
        or len(endpoint) > 2048
        or any(
            character.isspace()
            or ord(character) < 32
            or ord(character) == 127
            for character in endpoint
        )
    ):
        raise BakeoffError("azure_endpoint_invalid")
    try:
        parsed = urllib.parse.urlsplit(endpoint)
        port = parsed.port
    except ValueError:
        raise BakeoffError("azure_endpoint_invalid") from None
    if (
        parsed.scheme.lower() != "https"
        or not parsed.hostname
        or not _AZURE_RESOURCE_HOST_PATTERN.fullmatch(parsed.hostname.lower())
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
        or parsed.path not in ("", "/")
        or port not in (None, 443)
    ):
        raise BakeoffError("azure_endpoint_invalid")
    return parsed


def _origin(parsed: urllib.parse.SplitResult) -> tuple[str, str, int]:
    try:
        port = parsed.port
    except ValueError:
        raise BakeoffError("azure_operation_location_invalid") from None
    if not parsed.hostname:
        raise BakeoffError("azure_operation_location_invalid")
    return (parsed.scheme.lower(), parsed.hostname.lower(), port or 443)


def validate_operation_location(location: str, endpoint: str) -> str:
    base = _validated_azure_base(endpoint)
    if (
        not location
        or len(location) > 16_384
        or any(
            character.isspace()
            or ord(character) < 32
            or ord(character) == 127
            for character in location
        )
    ):
        raise BakeoffError("azure_operation_location_invalid")
    try:
        parsed = urllib.parse.urlsplit(location)
    except ValueError:
        raise BakeoffError("azure_operation_location_invalid") from None
    if (
        parsed.scheme.lower() != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.fragment
        or _origin(parsed) != _origin(base)
    ):
        raise BakeoffError("azure_operation_location_invalid")
    return location


def _azure_analyze_url(endpoint: str) -> str:
    base = _validated_azure_base(endpoint)
    authority = base.hostname
    if base.port:
        authority = f"{authority}:{base.port}"
    return (
        f"https://{authority}/documentintelligence/documentModels/"
        f"{AZURE_MODEL}:analyze?api-version={AZURE_API_VERSION}"
    )


def _azure_model_url(endpoint: str) -> str:
    base = _validated_azure_base(endpoint)
    authority = base.hostname
    if base.port:
        authority = f"{authority}:{base.port}"
    return (
        f"https://{authority}/documentintelligence/documentModels/{AZURE_MODEL}"
        f"?api-version={AZURE_API_VERSION}"
    )


class AzureClient:
    def __init__(
        self,
        endpoint: str,
        key: str,
        transport: HttpTransport,
        clock: Callable[[], float] = time.perf_counter,
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        _validated_azure_base(endpoint)
        self._endpoint = endpoint
        self._key = key
        self._transport = transport
        self._clock = clock
        self._sleep = sleep

    def recognize(self, image_bytes: bytes, width: int, height: int) -> ProviderResult:
        headers = {
            "Ocp-Apim-Subscription-Key": self._key,
            "Content-Type": "image/jpeg",
            "Accept": "application/json",
        }
        analyze_url = _azure_analyze_url(self._endpoint)
        started = self._clock()
        try:
            return self._recognize(
                image_bytes, width, height, started, analyze_url, headers
            )
        except BakeoffError as error:
            if (
                error.usage is None
                and error.code != "azure_submission_outcome_ambiguous"
                and not error.code.startswith("azure_submit_http_")
                and error.code != "azure_submit_http_error"
            ):
                error.usage = ProviderUsage(accepted_pages=1, billable_pages=1)
            if error.latency_seconds is None:
                error.latency_seconds = self._clock() - started
            raise

    def _recognize(
        self,
        image_bytes: bytes,
        width: int,
        height: int,
        started: float,
        analyze_url: str,
        headers: Mapping[str, str],
    ) -> ProviderResult:
        try:
            submitted = self._transport.request(
                "POST",
                analyze_url,
                headers,
                image_bytes,
                HTTP_TIMEOUT_SECONDS,
            )
        except BakeoffError as error:
            if error.code == "transport_failed" or error.http_status == 202:
                raise BakeoffError("azure_submission_outcome_ambiguous") from None
            raise
        if submitted.status != 202:
            raise BakeoffError(
                f"azure_submit_http_{submitted.status}"
                if submitted.status
                in (400, 401, 403, 404, 408, 409, 429, 500, 502, 503, 504)
                else "azure_submit_http_error"
            )
        locations = submitted.header_values("Operation-Location")
        if len(locations) != 1:
            raise BakeoffError("azure_operation_location_invalid")
        operation_url = validate_operation_location(locations[0], self._endpoint)
        if self._key in urllib.parse.unquote(operation_url):
            raise BakeoffError("azure_operation_location_contains_secret")

        deadline = started + AZURE_POLL_TIMEOUT_SECONDS
        for poll_index in range(AZURE_MAX_POLLS):
            if self._clock() >= deadline:
                raise BakeoffError("azure_poll_timeout_ambiguous")
            if poll_index:
                self._sleep(AZURE_POLL_INTERVAL_SECONDS)
            remaining = deadline - self._clock()
            if remaining <= 0.0:
                raise BakeoffError("azure_poll_timeout_ambiguous")
            try:
                polled = self._transport.request(
                    "GET",
                    operation_url,
                    {
                        "Ocp-Apim-Subscription-Key": self._key,
                        "Accept": "application/json",
                    },
                    None,
                    min(HTTP_TIMEOUT_SECONDS, remaining),
                )
            except BakeoffError as error:
                if error.code == "transport_failed":
                    raise BakeoffError("azure_poll_outcome_ambiguous") from None
                raise
            if polled.status != 200:
                raise BakeoffError(
                    f"azure_poll_http_{polled.status}"
                    if polled.status
                    in (400, 401, 403, 404, 408, 409, 429, 500, 502, 503, 504)
                    else "azure_poll_http_error"
                )
            try:
                status_envelope = strict_json_loads(polled.body)
            except BakeoffError as error:
                error.raw_response = polled.body
                raise
            if not isinstance(status_envelope, dict):
                raise BakeoffError("azure_poll_response_invalid")
            status = status_envelope.get("status")
            if status in ("notStarted", "running"):
                continue
            if status == "succeeded":
                try:
                    result = parse_azure_response(polled.body, width, height)
                except BakeoffError as error:
                    error.raw_response = polled.body
                    error.provider_response_success = True
                    try:
                        error.provenance = parse_azure_provenance(status_envelope)
                    except BakeoffError:
                        pass
                    raise
                return replace(result, latency_seconds=self._clock() - started)
            if status in ("failed", "canceled"):
                error = BakeoffError(f"azure_analysis_{status}")
                error.raw_response = polled.body
                raise error
            error = BakeoffError("azure_poll_status_invalid")
            error.raw_response = polled.body
            raise error
        raise BakeoffError("azure_poll_timeout_ambiguous")


def _exact_intersection_over_union(left: Box, right: Box) -> Fraction:
    left_left, left_top, left_right, left_bottom = (
        Fraction(value) for value in (left.left, left.top, left.right, left.bottom)
    )
    right_left, right_top, right_right, right_bottom = (
        Fraction(value)
        for value in (right.left, right.top, right.right, right.bottom)
    )
    intersection_width = max(
        Fraction(0), min(left_right, right_right) - max(left_left, right_left)
    )
    intersection_height = max(
        Fraction(0), min(left_bottom, right_bottom) - max(left_top, right_top)
    )
    intersection = intersection_width * intersection_height
    left_area = (left_right - left_left) * (left_bottom - left_top)
    right_area = (right_right - right_left) * (right_bottom - right_top)
    union = left_area + right_area - intersection
    return intersection / union if union > 0 else Fraction(0)


def intersection_over_union(left: Box, right: Box) -> float:
    return float(_exact_intersection_over_union(left, right))


@dataclass(frozen=True)
class Match:
    left_index: int
    right_index: int
    iou: float


@dataclass(frozen=True)
class _MatchingWeight:
    matched: int
    iou: Fraction
    tie: int

    def _key(self) -> tuple[int, Fraction, int]:
        return (self.matched, self.iou, self.tie)

    def __lt__(self, other: object) -> bool:
        if not isinstance(other, _MatchingWeight):
            return NotImplemented
        return self._key() < other._key()

    def __add__(self, other: object) -> _MatchingWeight:
        if not isinstance(other, _MatchingWeight):
            return NotImplemented
        return _MatchingWeight(
            self.matched + other.matched,
            self.iou + other.iou,
            self.tie + other.tie,
        )

    def __sub__(self, other: object) -> _MatchingWeight:
        if not isinstance(other, _MatchingWeight):
            return NotImplemented
        return _MatchingWeight(
            self.matched - other.matched,
            self.iou - other.iou,
            self.tie - other.tie,
        )


def _hungarian_max(weights: Sequence[Sequence[object]]) -> list[tuple[int, int]]:
    """Return a deterministic maximum-weight square assignment.

    This is the O(n^3) Hungarian algorithm. Callers pad through the square
    matrix constructed here and discard zero-weight dummy assignments. It
    operates over ordinary numbers or lexicographically ordered additive
    weights.
    """
    rows = len(weights)
    columns = max((len(row) for row in weights), default=0)
    size = max(rows, columns)
    if not size:
        return []
    sample = next((value for row in weights for value in row), 0)
    zero = sample - sample  # type: ignore[operator]
    padded = [
        list(row) + [zero] * (size - len(row)) for row in weights
    ] + [[zero] * size for _ in range(size - rows)]
    maximum = max(max(row) for row in padded)
    costs = [
        [maximum - value for value in row]  # type: ignore[operator]
        for row in padded
    ]

    u = [zero] * (size + 1)
    v = [zero] * (size + 1)
    p = [0] * (size + 1)
    way = [0] * (size + 1)
    for row_index in range(1, size + 1):
        p[0] = row_index
        column0 = 0
        minimum: list[object | None] = [None] * (size + 1)
        used = [False] * (size + 1)
        while True:
            used[column0] = True
            current_row = p[column0]
            delta = None
            column1 = 0
            for column in range(1, size + 1):
                if used[column]:
                    continue
                current = (
                    costs[current_row - 1][column - 1]
                    - u[current_row]
                    - v[column]
                )
                if minimum[column] is None or current < minimum[column]:
                    minimum[column] = current
                    way[column] = column0
                if delta is None or minimum[column] < delta:
                    delta = minimum[column]
                    column1 = column
            if delta is None:  # pragma: no cover - square matrix invariant.
                raise ValueError("Hungarian assignment has no reachable column")
            for column in range(size + 1):
                if used[column]:
                    u[p[column]] = u[p[column]] + delta  # type: ignore[operator]
                    v[column] = v[column] - delta  # type: ignore[operator]
                elif minimum[column] is not None:
                    minimum[column] = minimum[column] - delta  # type: ignore[operator]
            column0 = column1
            if p[column0] == 0:
                break
        while True:
            column1 = way[column0]
            p[column0] = p[column1]
            column0 = column1
            if column0 == 0:
                break
    assignment = [
        (p[column] - 1, column - 1) for column in range(1, size + 1)
    ]
    return sorted(assignment)


def maximum_weight_matching(
    left: Sequence[Box],
    right: Sequence[Box],
    *,
    minimum_iou: float = 0.0,
) -> list[Match]:
    """Maximum-cardinality then maximum-total-IoU one-to-one matching.

    Only edges at or above ``minimum_iou`` are eligible (zero-overlap edges
    are never eligible). Coordinates and IoUs remain exact Fractions through
    assignment. A lexicographic additive weight maximizes cardinality, exact
    total IoU, then the frozen prediction-index/reference-index ordering.
    """
    if not 0.0 <= minimum_iou <= 1.0:
        raise ValueError("minimum_iou must be between zero and one")
    if not left or not right:
        return []
    threshold = Fraction(minimum_iou)
    reference_count = len(left)
    prediction_count = len(right)
    tie_base = reference_count + 1
    tie_powers = [
        tie_base ** (prediction_count - 1 - prediction_index)
        for prediction_index in range(prediction_count)
    ]
    zero_weight = _MatchingWeight(0, Fraction(0), 0)
    ious: list[list[Fraction]] = []
    weights: list[list[_MatchingWeight]] = []
    for reference_index, left_box in enumerate(left):
        iou_row: list[Fraction] = []
        weight_row: list[_MatchingWeight] = []
        for prediction_index, right_box in enumerate(right):
            value = _exact_intersection_over_union(left_box, right_box)
            iou_row.append(value)
            eligible = value > 0 and value >= threshold
            weight_row.append(
                _MatchingWeight(
                    1,
                    value,
                    (reference_count - reference_index)
                    * tie_powers[prediction_index],
                )
                if eligible
                else zero_weight
            )
        ious.append(iou_row)
        weights.append(weight_row)
    pairs = _hungarian_max(weights)
    matches = [
        Match(left_index, right_index, float(ious[left_index][right_index]))
        for left_index, right_index in pairs
        if left_index < len(left)
        and right_index < len(right)
        and weights[left_index][right_index].matched > 0
    ]
    return sorted(matches, key=lambda item: (item.left_index, item.right_index))


def _ratio(numerator: int | float, denominator: int | float) -> float:
    return float(numerator / denominator) if denominator else 0.0


def _f1(precision: float, recall: float) -> float:
    return 2.0 * precision * recall / (precision + recall) if precision + recall else 0.0


def box_alignment(
    reference: Sequence[Box], hypothesis: Sequence[Box], threshold: float
) -> dict[str, int | float]:
    matches = maximum_weight_matching(
        reference, hypothesis, minimum_iou=threshold
    )
    matched = len(matches)
    precision = _ratio(matched, len(hypothesis))
    recall = _ratio(matched, len(reference))
    return {
        "reference_lines": len(reference),
        "provider_lines": len(hypothesis),
        "matched_lines": matched,
        "unmatched_reference_lines": len(reference) - matched,
        "unmatched_provider_lines": len(hypothesis) - matched,
        "precision": precision,
        "recall": recall,
        "f1": _f1(precision, recall),
    }


def reading_order_metrics(
    matches: Sequence[Match],
    reference_count: int,
    hypothesis_lines: Sequence[ProviderLine],
) -> dict[str, int | float]:
    ordered = sorted(matches, key=lambda item: item.left_index)
    hypothesis_orders = [
        hypothesis_lines[item.right_index].reading_order for item in ordered
    ]
    inversions = sum(
        1
        for left_index in range(len(hypothesis_orders))
        for right_index in range(left_index + 1, len(hypothesis_orders))
        if hypothesis_orders[left_index] > hypothesis_orders[right_index]
    )
    comparable_pairs = len(hypothesis_orders) * (len(hypothesis_orders) - 1) // 2
    accuracy = (
        1.0 - inversions / comparable_pairs if comparable_pairs else 1.0
    )
    return {
        "matched_lines": len(ordered),
        "reference_lines": reference_count,
        "matched_gt_coverage": _ratio(len(ordered), reference_count),
        "comparable_pairs": comparable_pairs,
        "inversions": inversions,
        "pairwise_accuracy": accuracy,
    }


def reference_lines(page: cgpg.Page) -> tuple[ProviderLine, ...]:
    lines: list[ProviderLine] = []
    for region in page.regions:
        if not region.text.strip():
            continue
        for line in region.lines:
            if not line.text:
                continue
            left, top, right, bottom = line.bbox
            lines.append(
                ProviderLine(
                    text=line.text,
                    bbox=Box(float(left), float(top), float(right), float(bottom)),
                    reading_order=len(lines),
                )
            )
    return tuple(lines)


def _scorable_regions(page: cgpg.Page) -> tuple[cgpg.Region, ...]:
    return tuple(region for region in page.regions if region.text.strip())


def lines_in_scorable_regions(
    page: cgpg.Page, lines: Sequence[ProviderLine]
) -> tuple[ProviderLine, ...]:
    regions = _scorable_regions(page)
    selected: list[ProviderLine] = []
    for line in lines:
        x, y = line.bbox.centre
        if any(
            region.bbox[0] <= x <= region.bbox[2]
            and region.bbox[1] <= y <= region.bbox[3]
            for region in regions
        ):
            selected.append(line)
    return tuple(sorted(selected, key=lambda item: item.reading_order))


def _run_cgpg_response(lines: Sequence[ProviderLine]) -> dict:
    return {
        "blocks": [
            {
                "reading_order": 0,
                "lines": [
                    {
                        "reading_order": line.reading_order,
                        "bbox": line.bbox.as_xywh(),
                        "words": [
                            {
                                "text": line.text,
                                "bbox": line.bbox.as_xywh(),
                            }
                        ],
                    }
                    for line in lines
                ],
            }
        ]
    }


def text_counts_for_page(
    page: cgpg.Page, lines: Sequence[ProviderLine]
) -> tuple[prior_bakeoff.CerCounts, prior_bakeoff.CerCounts]:
    response = _run_cgpg_response(lines)
    polytonic = prior_bakeoff.CerCounts()
    base = prior_bakeoff.CerCounts()
    for region in _scorable_regions(page):
        hypothesis = "\n".join(run_cgpg.lines_in_region(response, region))
        polytonic = polytonic.merge(_bounded_cer_counts(region.text, hypothesis))
        base = base.merge(
            _bounded_cer_counts(
                prior_bakeoff.base_letter_text(region.text),
                prior_bakeoff.base_letter_text(hypothesis),
            )
        )
    return polytonic, base


def _bounded_cer_counts(
    reference: str, hypothesis: str
) -> prior_bakeoff.CerCounts:
    reference_normalized = " ".join(
        unicodedata.normalize("NFC", reference).split()
    )
    hypothesis_normalized = " ".join(
        unicodedata.normalize("NFC", hypothesis).split()
    )
    if (
        (len(reference_normalized) + 1)
        * (len(hypothesis_normalized) + 1)
        > MAX_CER_CELL_UPDATES
    ):
        raise BakeoffError("cer_work_limit_exceeded")
    return prior_bakeoff.CerCounts(
        reference_chars=len(reference_normalized),
        char_edits=gold_metrics.levenshtein(
            reference_normalized, hypothesis_normalized
        ),
        samples=1,
        nfc_violations=int(not unicodedata.is_normalized("NFC", hypothesis)),
    )


def _provider_text(page: cgpg.Page, lines: Sequence[ProviderLine]) -> str:
    response = _run_cgpg_response(lines)
    return "\n".join(
        "\n".join(run_cgpg.lines_in_region(response, region))
        for region in _scorable_regions(page)
    )


@dataclass(frozen=True)
class InvocationOutcome:
    result: ProviderResult | None
    error_code: str | None = None
    provider_response_success: bool = False
    latency_seconds: float | None = None
    usage: ProviderUsage | None = None
    provenance: ProviderProvenance | None = None
    invalid_boxes: int = 0
    degenerate_boxes: int = 0
    out_of_page_boxes: int = 0
    duplicate_boxes: int = 0


def _sum_usage(usages_input: Iterable[ProviderUsage]) -> ProviderUsage:
    usages = list(usages_input)
    confidences = [
        usage.confidence_min for usage in usages if usage.confidence_min is not None
    ]
    maxima = [
        usage.confidence_max for usage in usages if usage.confidence_max is not None
    ]
    return ProviderUsage(
        prompt_tokens=sum(item.prompt_tokens for item in usages),
        candidate_tokens=sum(item.candidate_tokens for item in usages),
        thought_tokens=sum(item.thought_tokens for item in usages),
        cached_tokens=sum(item.cached_tokens for item in usages),
        accepted_pages=sum(item.accepted_pages for item in usages),
        billable_pages=sum(item.billable_pages for item in usages),
        native_words=sum(item.native_words for item in usages),
        confidence_count=sum(item.confidence_count for item in usages),
        confidence_sum=sum(item.confidence_sum for item in usages),
        confidence_min=min(confidences) if confidences else None,
        confidence_max=max(maxima) if maxima else None,
    )


def _display_float(value: float) -> float:
    return round(value, 6)


def latency_statistics(values: Sequence[float]) -> dict[str, int | float]:
    ordered = sorted(values)
    if not ordered:
        return {
            "samples": 0,
            "total_seconds": 0.0,
            "mean_seconds": 0.0,
            "median_seconds": 0.0,
            "p95_seconds": 0.0,
            "max_seconds": 0.0,
        }
    p95 = ordered[math.ceil(0.95 * len(ordered)) - 1]
    return {
        "samples": len(ordered),
        "total_seconds": _display_float(sum(ordered)),
        "mean_seconds": _display_float(statistics.fmean(ordered)),
        "median_seconds": _display_float(statistics.median(ordered)),
        "p95_seconds": _display_float(p95),
        "max_seconds": _display_float(ordered[-1]),
    }


def _decimal_cost(value: Decimal) -> float:
    return float(value.quantize(Decimal("0.000001")))


def tariff_cost(
    provider: str, usage: ProviderUsage, *, usage_complete: bool = True
) -> dict:
    million = Decimal(1_000_000)
    thousand = Decimal(1000)
    if provider == "google_vertex_gemini":
        uncached = max(usage.prompt_tokens - usage.cached_tokens, 0)
        amount = (
            Decimal(uncached) * GEMINI_INPUT_USD_PER_MILLION / million
            + Decimal(usage.cached_tokens)
            * GEMINI_CACHED_INPUT_USD_PER_MILLION
            / million
            + Decimal(usage.candidate_tokens + usage.thought_tokens)
            * GEMINI_OUTPUT_AND_THINKING_USD_PER_MILLION
            / million
        )
        snapshot = {
            "effective_date": "2026-08-30",
            "scope": "global_standard_paygo",
            "input_usd_per_million_tokens": 0.75,
            "cached_input_usd_per_million_tokens": 0.075,
            "output_and_thinking_usd_per_million_tokens": 3.75,
        }
    elif provider == "azure_document_intelligence":
        amount = (
            Decimal(usage.billable_pages)
            * AZURE_READ_USD_PER_THOUSAND_PAGES
            / thousand
        )
        snapshot = {
            "effective_date": "2026-08-30",
            "tier": "S0",
            "read_usd_per_thousand_pages": 1.50,
        }
    else:  # pragma: no cover - internal invariant.
        raise ValueError("unknown provider")
    measured_cost = _decimal_cost(amount)
    return {
        "tariff_cost_usd": measured_cost if usage_complete else None,
        "known_measured_usage_tariff_cost_lower_bound_usd": measured_cost,
        "usage_complete": usage_complete,
        "cost_status": (
            "complete_measured_usage"
            if usage_complete
            else "incomplete_known_lower_bound"
        ),
        "actual_billed_cost": None,
        "tariff_snapshot": snapshot,
    }


def _aggregate_box_metrics(
    page_results: Sequence[tuple[Sequence[Box], Sequence[Box]]], threshold: float
) -> dict:
    reference_total = provider_total = matched_total = 0
    for references, hypotheses in page_results:
        metric = box_alignment(references, hypotheses, threshold)
        reference_total += int(metric["reference_lines"])
        provider_total += int(metric["provider_lines"])
        matched_total += int(metric["matched_lines"])
    precision = _ratio(matched_total, provider_total)
    recall = _ratio(matched_total, reference_total)
    return {
        "reference_lines": reference_total,
        "provider_lines": provider_total,
        "matched_lines": matched_total,
        "unmatched_reference_lines": reference_total - matched_total,
        "unmatched_provider_lines": provider_total - matched_total,
        "precision": _display_float(precision),
        "recall": _display_float(recall),
        "f1": _display_float(_f1(precision, recall)),
    }


def _unrounded_aggregate_f1(metric: Mapping[str, object]) -> float:
    """Reconstruct aggregate F1 from integer counts for threshold decisions."""
    matched = int(metric["matched_lines"])
    precision = _ratio(matched, int(metric["provider_lines"]))
    recall = _ratio(matched, int(metric["reference_lines"]))
    return _f1(precision, recall)


def _inter_run_cer(reference: str, hypothesis: str) -> float:
    counts = _bounded_cer_counts(reference, hypothesis)
    if counts.reference_chars:
        return counts.cer
    return 0.0 if counts.char_edits == 0 else 1.0


def stability_metrics(
    pages: Sequence[cgpg.Page],
    results: Mapping[tuple[str, int], InvocationOutcome],
) -> dict:
    per_page: list[dict] = []
    for page in pages:
        outcomes = [results[(page.name, repetition)] for repetition in range(REPETITIONS)]
        if any(item.result is None for item in outcomes):
            per_page.append(
                {
                    "page_id": page.name,
                    "complete_repetitions": sum(item.result is not None for item in outcomes),
                    "pair_count": 0,
                    "minimum_box_f1_at_0_5": 0.0,
                    "minimum_pairwise_order_accuracy": 0.0,
                    "maximum_inter_run_text_cer": 1.0,
                    "maximum_relative_line_count_delta": 1.0,
                    "structural_passed": False,
                    "recognition_quality_passed": False,
                }
            )
            continue
        selected = [
            lines_in_scorable_regions(page, item.result.lines)  # type: ignore[union-attr]
            for item in outcomes
        ]
        box_f1s: list[float] = []
        order_accuracies: list[float] = []
        text_cers: list[float] = []
        count_deltas: list[float] = []
        for left_index, right_index in ((0, 1), (0, 2), (1, 2)):
            left_lines = selected[left_index]
            right_lines = selected[right_index]
            left_boxes = [line.bbox for line in left_lines]
            right_boxes = [line.bbox for line in right_lines]
            matches = maximum_weight_matching(
                left_boxes, right_boxes, minimum_iou=0.5
            )
            metric = box_alignment(left_boxes, right_boxes, 0.5)
            box_f1s.append(float(metric["f1"]))
            order_accuracies.append(
                float(
                    reading_order_metrics(matches, len(left_lines), right_lines)[
                        "pairwise_accuracy"
                    ]
                )
            )
            left_text = _provider_text(page, left_lines)
            right_text = _provider_text(page, right_lines)
            text_cers.append(_inter_run_cer(left_text, right_text))
            count_deltas.append(
                abs(len(left_lines) - len(right_lines))
                / max(len(left_lines), len(right_lines), 1)
            )
        page_record = {
            "page_id": page.name,
            "complete_repetitions": REPETITIONS,
            "pair_count": 3,
            "minimum_box_f1_at_0_5": _display_float(min(box_f1s)),
            "minimum_pairwise_order_accuracy": _display_float(
                min(order_accuracies)
            ),
            "maximum_inter_run_text_cer": _display_float(max(text_cers)),
            "maximum_relative_line_count_delta": _display_float(max(count_deltas)),
        }
        page_record["structural_passed"] = bool(
            min(box_f1s) >= MIN_STABILITY_BOX_F1_AT_050
            and min(order_accuracies) >= MIN_STABILITY_ORDER_PAIRWISE_ACCURACY
            and max(count_deltas) <= MAX_STABILITY_RELATIVE_LINE_COUNT_DELTA
        )
        page_record["recognition_quality_passed"] = bool(
            max(text_cers) <= MAX_STABILITY_INTER_RUN_TEXT_CER
        )
        per_page.append(page_record)
    return {
        "structural_pages_passed": sum(
            bool(item["structural_passed"]) for item in per_page
        ),
        "recognition_quality_pages_passed": sum(
            bool(item["recognition_quality_passed"]) for item in per_page
        ),
        "pages_required": len(pages),
        "structural_all_pages_passed": all(
            bool(item["structural_passed"]) for item in per_page
        ),
        "recognition_quality_all_pages_passed": all(
            bool(item["recognition_quality_passed"]) for item in per_page
        ),
        "per_page": per_page,
    }


def evaluate_provider(
    provider: str,
    pages: Sequence[cgpg.Page],
    outcomes: Mapping[tuple[str, int], InvocationOutcome],
) -> dict:
    expected_invocations = len(pages) * REPETITIONS
    successful_results = [
        outcome.result for outcome in outcomes.values() if outcome.result is not None
    ]
    successful_results_typed = [result for result in successful_results if result is not None]
    provider_response_successes = sum(
        outcome.result is not None or outcome.provider_response_success
        for outcome in outcomes.values()
    )
    invocation_latencies = [
        outcome.result.latency_seconds
        if outcome.result is not None
        else outcome.latency_seconds
        for outcome in outcomes.values()
    ]
    polytonic = prior_bakeoff.CerCounts()
    base = prior_bakeoff.CerCounts()
    box_inputs: list[tuple[Sequence[Box], Sequence[Box]]] = []
    all_positive_ious: list[float] = []
    reading_matches = reading_reference = reading_pairs = inversions = 0
    failures: list[dict] = []
    invalid_boxes = 0
    degenerate_boxes = 0
    out_of_page_boxes = 0
    duplicate_boxes = 0

    for page in pages:
        references = reference_lines(page)
        reference_boxes = [line.bbox for line in references]
        for repetition in range(REPETITIONS):
            outcome = outcomes[(page.name, repetition)]
            if outcome.result is None:
                lines: tuple[ProviderLine, ...] = ()
                failures.append(
                    {
                        "page_id": page.name,
                        "repetition": repetition,
                        "error_code": outcome.error_code or "unknown_failure",
                    }
                )
                invalid_boxes += outcome.invalid_boxes
                degenerate_boxes += outcome.degenerate_boxes
                out_of_page_boxes += outcome.out_of_page_boxes
                duplicate_boxes += outcome.duplicate_boxes
            else:
                lines = lines_in_scorable_regions(page, outcome.result.lines)
            poly_counts, base_counts = text_counts_for_page(page, lines)
            polytonic = polytonic.merge(poly_counts)
            base = base.merge(base_counts)
            hypothesis_boxes = [line.bbox for line in lines]
            box_inputs.append((reference_boxes, hypothesis_boxes))
            positive_matches = maximum_weight_matching(
                reference_boxes, hypothesis_boxes, minimum_iou=0.0
            )
            all_positive_ious.extend(item.iou for item in positive_matches)
            order_matches = maximum_weight_matching(
                reference_boxes, hypothesis_boxes, minimum_iou=0.5
            )
            order = reading_order_metrics(order_matches, len(references), lines)
            reading_matches += int(order["matched_lines"])
            reading_reference += len(references)
            reading_pairs += int(order["comparable_pairs"])
            inversions += int(order["inversions"])

    box_050 = _aggregate_box_metrics(box_inputs, 0.5)
    box_075 = _aggregate_box_metrics(box_inputs, 0.75)
    box_050_gate_f1 = _unrounded_aggregate_f1(box_050)
    box_075_gate_f1 = _unrounded_aggregate_f1(box_075)
    matched_mean_iou = (
        statistics.fmean(all_positive_ious) if all_positive_ious else 0.0
    )
    pairwise_accuracy = (
        1.0 - inversions / reading_pairs if reading_pairs else 0.0
    )
    coverage = _ratio(reading_matches, reading_reference)
    stability = stability_metrics(pages, outcomes)
    response_rate = _ratio(len(successful_results_typed), expected_invocations)
    usage = _sum_usage(
        outcome.result.usage
        if outcome.result is not None
        else outcome.usage
        for outcome in outcomes.values()
        if outcome.result is not None or outcome.usage is not None
    )
    provenances = [
        outcome.result.provenance
        if outcome.result is not None
        else outcome.provenance
        for outcome in outcomes.values()
        if outcome.result is not None or outcome.provenance is not None
    ]

    pages_with_three_valid_repetitions = sum(
        all(outcomes[(page.name, repetition)].result is not None for repetition in range(REPETITIONS))
        for page in pages
    )
    geometry_checks = {
        "response_success": {
            "operator": "==",
            "threshold": 1.0,
            "actual": _display_float(
                _ratio(provider_response_successes, expected_invocations)
            ),
            "passed": provider_response_successes == expected_invocations,
        },
        "strict_schema_valid": {
            "operator": "==",
            "threshold": 1.0,
            "actual": _display_float(response_rate),
            "passed": len(successful_results_typed) == expected_invocations,
        },
        "pages_with_three_valid_repetitions": {
            "operator": "==",
            "threshold": len(pages),
            "actual": pages_with_three_valid_repetitions,
            "passed": pages_with_three_valid_repetitions == len(pages),
        },
        "invalid_boxes": {
            "operator": "==",
            "threshold": 0,
            "actual": invalid_boxes,
            "passed": invalid_boxes == 0,
        },
        "degenerate_boxes": {
            "operator": "==",
            "threshold": 0,
            "actual": degenerate_boxes,
            "passed": degenerate_boxes == 0,
        },
        "out_of_page_boxes": {
            "operator": "==",
            "threshold": 0,
            "actual": out_of_page_boxes,
            "passed": out_of_page_boxes == 0,
        },
        "exact_duplicate_boxes": {
            "operator": "==",
            "threshold": 0,
            "actual": duplicate_boxes,
            "passed": duplicate_boxes == 0,
        },
        "box_f1_at_0_5": {
            "operator": ">=",
            "threshold": MIN_BOX_F1_AT_050,
            "actual": box_050["f1"],
            "passed": box_050_gate_f1 >= MIN_BOX_F1_AT_050,
        },
        "box_f1_at_0_75": {
            "operator": ">=",
            "threshold": MIN_BOX_F1_AT_075,
            "actual": box_075["f1"],
            "passed": box_075_gate_f1 >= MIN_BOX_F1_AT_075,
        },
        "matched_mean_iou": {
            "operator": ">=",
            "threshold": MIN_MATCHED_MEAN_IOU,
            "actual": _display_float(matched_mean_iou),
            "passed": matched_mean_iou >= MIN_MATCHED_MEAN_IOU,
        },
        "reading_order_pairwise_accuracy": {
            "operator": ">=",
            "threshold": MIN_READING_ORDER_PAIRWISE_ACCURACY,
            "actual": _display_float(pairwise_accuracy),
            "passed": pairwise_accuracy >= MIN_READING_ORDER_PAIRWISE_ACCURACY,
        },
        "reading_order_matched_gt_coverage": {
            "operator": ">=",
            "threshold": MIN_READING_ORDER_GT_COVERAGE,
            "actual": _display_float(coverage),
            "passed": coverage >= MIN_READING_ORDER_GT_COVERAGE,
        },
        "repeat_stability_all_pages": {
            "operator": "==",
            "threshold": len(pages),
            "actual": stability["structural_pages_passed"],
            "passed": stability["structural_all_pages_passed"],
        },
    }
    quality_checks = {
        "polytonic_cer": {
            "operator": "<=",
            "threshold": MAX_POLYTONIC_CER,
            "actual": _display_float(polytonic.cer),
            "passed": polytonic.cer <= MAX_POLYTONIC_CER,
        },
        "base_letter_cer": {
            "operator": "<=",
            "threshold": MAX_BASE_LETTER_CER,
            "actual": _display_float(base.cer),
            "passed": base.cer <= MAX_BASE_LETTER_CER,
        },
        "repeat_text_stability_all_pages": {
            "operator": "==",
            "threshold": len(pages),
            "actual": stability["recognition_quality_pages_passed"],
            "passed": stability["recognition_quality_all_pages_passed"],
        },
    }
    usage_record = {
        "prompt_tokens": usage.prompt_tokens,
        "candidate_tokens": usage.candidate_tokens,
        "thought_tokens": usage.thought_tokens,
        "cached_tokens": usage.cached_tokens,
        "accepted_pages": usage.accepted_pages,
        "billable_pages": usage.billable_pages,
        "native_words": usage.native_words,
        "confidence": {
            "samples": usage.confidence_count,
            "mean": _display_float(
                _ratio(usage.confidence_sum, usage.confidence_count)
            ),
            "minimum": (
                _display_float(usage.confidence_min)
                if usage.confidence_min is not None
                else None
            ),
            "maximum": (
                _display_float(usage.confidence_max)
                if usage.confidence_max is not None
                else None
            ),
        },
        "usage_unavailable_invocations": sum(
            outcome.result is None and outcome.usage is None
            for outcome in outcomes.values()
        ),
        "ambiguous_usage_invocations": sum(
            outcome.error_code
            in {"gemini_outcome_ambiguous", "azure_submission_outcome_ambiguous"}
            for outcome in outcomes.values()
        ),
    }
    incomplete_cost_usage_invocations = sum(
        outcome.result is None
        and outcome.usage is None
        and (
            outcome.provider_response_success
            or outcome.error_code
            in {
                "gemini_outcome_ambiguous",
                "azure_submission_outcome_ambiguous",
            }
        )
        for outcome in outcomes.values()
    )
    usage_record["incomplete_cost_usage_invocations"] = (
        incomplete_cost_usage_invocations
    )
    return {
        "provider": provider,
        "expected_invocations": expected_invocations,
        "successful_invocations": len(successful_results_typed),
        "failures": failures,
        "recognition_metrics": {
            "polytonic": polytonic.as_dict(),
            "base_letter": base.as_dict(),
        },
        "box_alignment": {
            "at_0_5": box_050,
            "at_0_75": box_075,
            "positive_iou_matches": len(all_positive_ious),
            "matched_mean_iou": _display_float(matched_mean_iou),
            "invalid_boxes": invalid_boxes,
            "degenerate_boxes": degenerate_boxes,
            "out_of_page_boxes": out_of_page_boxes,
            "exact_duplicate_boxes": duplicate_boxes,
        },
        "reading_order": {
            "matched_lines": reading_matches,
            "reference_lines": reading_reference,
            "matched_gt_coverage": _display_float(coverage),
            "comparable_pairs": reading_pairs,
            "inversions": inversions,
            "pairwise_accuracy": _display_float(pairwise_accuracy),
        },
        "repeat_stability": stability,
        "latency": latency_statistics(
            [value for value in invocation_latencies if value is not None]
        ),
        "usage": usage_record,
        "provider_response_provenance": {
            "records": len(provenances),
            "observed_model_versions": sorted(
                {
                    item.model_version
                    for item in provenances
                    if item is not None and item.model_version is not None
                }
            ),
            "response_id_sha256": sorted(
                [
                    item.response_id_sha256
                    for item in provenances
                    if item is not None and item.response_id_sha256 is not None
                ]
            ),
            "observed_model_ids": sorted(
                {
                    item.model_id
                    for item in provenances
                    if item is not None and item.model_id is not None
                }
            ),
            "observed_api_versions": sorted(
                {
                    item.api_version
                    for item in provenances
                    if item is not None and item.api_version is not None
                }
            ),
        },
        "cost": tariff_cost(
            provider,
            usage,
            usage_complete=incomplete_cost_usage_invocations == 0,
        ),
        "structural_contract_gate": {
            "outcome": (
                "pass" if all(item["passed"] for item in geometry_checks.values()) else "fail"
            ),
            "checks": geometry_checks,
        },
        "recognition_quality_gate": {
            "outcome": (
                "pass" if all(item["passed"] for item in quality_checks.values()) else "fail"
            ),
            "checks": quality_checks,
        },
    }


def decide(gemini: dict, azure: dict) -> dict[str, str | None]:
    gemini_structural = gemini["structural_contract_gate"]["outcome"] == "pass"
    azure_structural = azure["structural_contract_gate"]["outcome"] == "pass"
    gemini_quality = gemini["recognition_quality_gate"]["outcome"] == "pass"
    if not azure_structural:
        outcome = "inconclusive_control_failed"
        selected = None
    elif not gemini_structural:
        outcome = "gemini_not_complete_ocr_consider_azure"
        selected = None
    elif not gemini_quality:
        outcome = "gemini_coordinate_candidate_quality_failed"
        selected = None
    else:
        outcome = "gemini_complete_ocr_candidate_retained"
        selected = "google_vertex_gemini"
    return {
        "outcome": outcome,
        "candidate_retained": selected,
        "production_eligibility": "not_established_by_bakeoff",
    }


def frozen_configuration() -> dict:
    return {
        "providers": {
            "google_vertex_gemini": {
                "model": GEMINI_MODEL,
                "location": GEMINI_LOCATION,
                "api_version": GEMINI_API_VERSION,
                "prompt_sha256": GEMINI_PROMPT_SHA256,
                "response_schema_sha256": canonical_digest(GEMINI_RESPONSE_SCHEMA),
                "sampling_parameters": "omitted_provider_defaults",
                "thinking_level": GEMINI_THINKING_LEVEL,
                "max_output_tokens": GEMINI_MAX_OUTPUT_TOKENS,
                "maximum_lines": MAX_LINES,
                "maximum_response_bytes": MAX_RESPONSE_BYTES,
                "maximum_page_text_characters": MAX_PAGE_TEXT_CHARS,
                "required_finish_reason": "STOP",
                "required_response_provenance": ["modelVersion", "responseId"],
            },
            "azure_document_intelligence": {
                "model": AZURE_MODEL,
                "api_version": AZURE_API_VERSION,
                "maximum_lines": MAX_LINES,
                "maximum_response_bytes": MAX_RESPONSE_BYTES,
                "maximum_page_text_characters": MAX_PAGE_TEXT_CHARS,
                "maximum_native_words": MAX_WORDS,
                "required_response_provenance": ["modelId", "apiVersion"],
                "endpoint_hostname_policy": (
                    "single_label.cognitiveservices.azure.com_https_443"
                ),
            },
        },
        "input_manifest_sha256": FORMAL_INPUT_MANIFEST_SHA256,
        "legacy_audit_manifest_sha256": LEGACY_AUDIT_MANIFEST_SHA256,
        "page_count": SCORABLE_PAGE_COUNT,
        "repetitions": REPETITIONS,
        "concurrency": 1,
        "schedule": "page_then_repetition_parity_interleave",
        "automatic_retries": 0,
        "http_timeout_seconds": HTTP_TIMEOUT_SECONDS,
        "azure_poll_timeout_seconds": AZURE_POLL_TIMEOUT_SECONDS,
        "azure_max_polls": AZURE_MAX_POLLS,
        "azure_poll_interval_seconds": AZURE_POLL_INTERVAL_SECONDS,
        "preflight_readiness": {
            "google": "countTokens_literal_readiness_no_corpus_bytes",
            "azure": "get_exact_document_model_no_corpus_bytes",
            "required_requests": 2,
            "both_must_pass_before_inference": True,
        },
        "matching": (
            "exact_fraction_maximum_cardinality_then_total_iou_then_"
            "prediction_reference_index_v2"
        ),
        "line_and_word_text_contract": "nonempty_no_cr_lf_cc_zl_zp",
        "maximum_cer_cell_updates_per_comparison": MAX_CER_CELL_UPDATES,
        "maximum_fixed_reference_text_characters": MAX_FIXED_REFERENCE_TEXT_CHARS,
        "verified_input_snapshot": (
            "parse_and_score_exact_formal_manifest_xml_bytes_and_upload_"
            "exact_verified_jpeg_bytes_v2"
        ),
        "run_binding_snapshot": (
            "prereg_validation_returns_clean_read_once_code_config_snapshot_v2"
        ),
        "cer_backend": "bound_scripts_ocr_gold_metrics_py_no_dynamic_backend",
        "raw_replay": (
            "externally_anchored_manifest_single_read_scoring_snapshot_v3"
        ),
        "oversized_success_accounting": (
            "preserve_received_http_status_and_mark_usage_incomplete_v1"
        ),
        "strict_json_recursion": "stable_response_json_invalid_v1",
        "azure_poll_deadline": "recheck_after_sleep_no_minimum_timeout_floor_v1",
        "stability_zero_reference_cer": (
            "zero_only_when_both_normalized_runs_empty_else_one_v1"
        ),
        "thresholds": {
            "polytonic_cer_max": MAX_POLYTONIC_CER,
            "base_letter_cer_max": MAX_BASE_LETTER_CER,
            "box_f1_at_0_5_min": MIN_BOX_F1_AT_050,
            "box_f1_at_0_75_min": MIN_BOX_F1_AT_075,
            "matched_mean_iou_min": MIN_MATCHED_MEAN_IOU,
            "reading_order_pairwise_accuracy_min": MIN_READING_ORDER_PAIRWISE_ACCURACY,
            "reading_order_gt_coverage_min": MIN_READING_ORDER_GT_COVERAGE,
            "stability_box_f1_at_0_5_min": MIN_STABILITY_BOX_F1_AT_050,
            "stability_order_pairwise_accuracy_min": MIN_STABILITY_ORDER_PAIRWISE_ACCURACY,
            "stability_inter_run_text_cer_max": MAX_STABILITY_INTER_RUN_TEXT_CER,
            "stability_relative_line_count_delta_max": MAX_STABILITY_RELATIVE_LINE_COUNT_DELTA,
        },
        "tariffs": {
            "effective_date": "2026-08-30",
            "gemini_global_input_usd_per_million": 0.75,
            "gemini_global_cached_input_usd_per_million": 0.075,
            "gemini_global_output_and_thinking_usd_per_million": 3.75,
            "azure_s0_read_usd_per_thousand_pages": 1.50,
            "actual_billed_cost_default": None,
            "planned_tariff_ceiling_usd": str(PLANNED_TARIFF_CEILING_USD),
            "required_max_cost_usd": str(REQUIRED_MAX_COST_USD),
        },
        "cost_reporting": (
            "null_total_and_known_measured_lower_bound_when_potentially_"
            "billable_usage_is_incomplete_v1"
        ),
        "decision_precedence": [
            "azure_structural_control",
            "gemini_structural_contract",
            "gemini_recognition_quality",
        ],
        "decision_outcomes": {
            "azure_structural_fail": "inconclusive_control_failed",
            "gemini_structural_fail": "gemini_not_complete_ocr_consider_azure",
            "gemini_quality_only_fail": "gemini_coordinate_candidate_quality_failed",
            "gemini_structural_and_quality_pass": "gemini_complete_ocr_candidate_retained",
        },
        "single_column_limitation": True,
    }


def conservative_gemini_input_token_ceiling(record: dict) -> int:
    image_tiles = math.ceil(record["width"] / 768) * math.ceil(
        record["height"] / 768
    )
    return 1024 + 258 * image_tiles


def conservative_call_cost(provider: str, record: dict) -> Decimal:
    if provider == "google_vertex_gemini":
        return (
            Decimal(conservative_gemini_input_token_ceiling(record))
            * GEMINI_INPUT_USD_PER_MILLION
            / Decimal(1_000_000)
            + Decimal(GEMINI_MAX_OUTPUT_TOKENS)
            * GEMINI_OUTPUT_AND_THINKING_USD_PER_MILLION
            / Decimal(1_000_000)
        )
    if provider == "azure_document_intelligence":
        return AZURE_READ_USD_PER_THOUSAND_PAGES / Decimal(1000)
    raise ValueError("unknown provider")


def conservative_planned_cost() -> Decimal:
    return sum(
        (
            conservative_call_cost(provider, record)
            for record in FIXED_INPUT_RECORDS
            for _repetition in range(REPETITIONS)
            for provider in PROVIDERS
        ),
        Decimal(0),
    )


class BudgetGuard:
    """Reserve the conservative ceiling before every non-retried call."""

    def __init__(self, maximum_usd: Decimal) -> None:
        self.maximum_usd = maximum_usd
        self.reserved_usd = Decimal(0)

    def reserve(self, provider: str, record: dict) -> None:
        next_total = self.reserved_usd + conservative_call_cost(provider, record)
        if next_total > self.maximum_usd:
            raise BakeoffError("cost_ceiling_would_be_exceeded")
        self.reserved_usd = next_total


@dataclass(frozen=True)
class LiveCredentials:
    google_project: str = field(repr=False)
    google_bearer: str = field(repr=False)
    azure_endpoint: str = field(repr=False)
    azure_key: str = field(repr=False)


@dataclass(frozen=True)
class RunBinding:
    preregistration_sha256: str
    configuration_sha256: str
    code_sha256: tuple[tuple[str, str], ...]

    def as_record(self) -> dict[str, object]:
        return {
            "preregistration_sha256": self.preregistration_sha256,
            "configuration_sha256": self.configuration_sha256,
            "code_sha256": dict(self.code_sha256),
        }


@dataclass(frozen=True)
class PreflightResult:
    ready: bool
    error_codes: tuple[str, ...]
    pages: tuple[cgpg.Page, ...] = field(default=(), repr=False)
    credentials: LiveCredentials | None = field(default=None, repr=False)
    run_binding: RunBinding | None = field(default=None, repr=False)
    readiness_requests: int = 0
    google_ready: bool = False
    azure_ready: bool = False

    def public(self) -> dict:
        return {
            "schema": "mpdf-cloud-complete-ocr-bakeoff-preflight",
            "schema_version": "1.0",
            "ready": self.ready,
            "ocr_generatecontent_analyze_calls": 0,
            "readiness_requests": self.readiness_requests,
            "provider_readiness": {
                "google_vertex_gemini": self.google_ready,
                "azure_document_intelligence": self.azure_ready,
            },
            "error_codes": list(self.error_codes),
        }


def _scrubbed_subprocess_environment() -> dict[str, str]:
    environment = dict(os.environ)
    for name in (
        LIVE_ENV,
        GOOGLE_PROJECT_ENV,
        AZURE_ENDPOINT_ENV,
        AZURE_KEY_ENV,
    ):
        environment.pop(name, None)
    return environment


def _resolved_executable(name: str) -> str | None:
    candidate = shutil.which(name)
    if not candidate:
        return None
    try:
        resolved = Path(candidate).resolve(strict=True)
    except OSError:
        return None
    if not resolved.is_file() or not os.access(resolved, os.X_OK):
        return None
    return str(resolved)


def _git_paths_are_committed(paths: Sequence[Path]) -> bool:
    relative_paths: list[str] = []
    for path in paths:
        try:
            relative_paths.append(str(path.resolve().relative_to(REPO_ROOT)))
        except (OSError, ValueError):
            return False
    binary = _resolved_executable("git")
    if binary is None:
        return False
    child_environment = _scrubbed_subprocess_environment()
    try:
        tracked = subprocess.run(
            [binary, "ls-files", "--error-unmatch", "--", *relative_paths],
            cwd=REPO_ROOT,
            capture_output=True,
            timeout=30,
            check=False,
            env=child_environment,
        )
        worktree = subprocess.run(
            [binary, "diff", "--quiet", "--", *relative_paths],
            cwd=REPO_ROOT,
            capture_output=True,
            timeout=30,
            check=False,
            env=child_environment,
        )
        staged = subprocess.run(
            [binary, "diff", "--cached", "--quiet", "--", *relative_paths],
            cwd=REPO_ROOT,
            capture_output=True,
            timeout=30,
            check=False,
            env=child_environment,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    return tracked.returncode == worktree.returncode == staged.returncode == 0


def verify_preregistration(
    path: Path, *, require_committed: bool = True
) -> RunBinding:
    try:
        raw = path.read_bytes()
    except OSError:
        raise BakeoffError("preregistration_missing") from None
    if sha256_bytes(raw) != PREREGISTRATION_SHA256:
        raise BakeoffError("preregistration_digest_mismatch")
    value = strict_json_loads(raw, maximum_bytes=2 * 1024 * 1024)
    if not isinstance(value, dict):
        raise BakeoffError("preregistration_invalid")
    try:
        valid = (
            value["schema"]
            == "mpdf-cloud-complete-ocr-bakeoff-preregistration"
            and value["schema_version"] == "1.0"
            and value["status"] == "preregistered_not_executed"
            and value["registered_before_live_execution"] is True
            and value["input_manifest"]["formal_manifest"]["expected_digest"]
            == FORMAL_INPUT_MANIFEST_SHA256
            and value["input_manifest"]["legacy_audit_manifest"][
                "legacy_audit_manifest_digest"
            ]
            == LEGACY_AUDIT_MANIFEST_SHA256
            and value["provider_contracts"]["google_vertex_gemini"]["prompt"][
                "sha256"
            ]
            == GEMINI_PROMPT_SHA256
            and value["provider_contracts"]["google_vertex_gemini"][
                "response_contract"
            ]["canonical_schema_sha256"]
            == canonical_digest(GEMINI_RESPONSE_SCHEMA)
            and value["provider_contracts"]["google_vertex_gemini"][
                "response_contract"
            ]["lines"]["maximum_items"]
            == MAX_LINES
            and value["provider_contracts"]["google_vertex_gemini"][
                "generation_config"
            ]
            == {
                "sampling_parameters": (
                    "Omit temperature, topP, and topK because Gemini 3.7 does "
                    "not expose them as stable controls; the three frozen "
                    "repetitions measure actual backend stability under "
                    "provider-default sampling."
                ),
                "thinking_level": GEMINI_THINKING_LEVEL,
                "max_output_tokens": GEMINI_MAX_OUTPUT_TOKENS,
                "response_mime_type": "application/json",
            }
            and value["provider_contracts"]["azure_document_intelligence"][
                "model"
            ]
            == AZURE_MODEL
            and value["provider_contracts"]["azure_document_intelligence"][
                "api_version"
            ]
            == AZURE_API_VERSION
            and value["harness_binding"][
                "canonical_frozen_configuration_sha256"
            ]
            == canonical_digest(frozen_configuration())
            and Decimal(
                str(
                    value["cost_accounting"]["conservative_planned_ceiling"][
                        "combined_planned_tariff_ceiling_usd"
                    ]
                )
            )
            == PLANNED_TARIFF_CEILING_USD
        )
    except (InvalidOperation, KeyError, TypeError, ValueError):
        valid = False
    if not valid:
        raise BakeoffError("preregistration_invalid")
    code_digests = _code_digests()
    if require_committed and not _git_paths_are_committed(
        [path, *list(_code_paths().values())]
    ):
        raise BakeoffError("preregistration_or_harness_not_committed")
    return RunBinding(
        preregistration_sha256=sha256_bytes(raw),
        configuration_sha256=canonical_digest(frozen_configuration()),
        code_sha256=tuple(sorted(code_digests.items())),
    )


def _safe_google_project(environment: Mapping[str, str]) -> str:
    project = environment.get(GOOGLE_PROJECT_ENV, "").strip()
    if not project:
        binary = _resolved_executable("gcloud")
        if not binary:
            raise BakeoffError("google_project_unavailable")
        try:
            completed = subprocess.run(
                [binary, "config", "get-value", "project", "--quiet"],
                capture_output=True,
                timeout=30,
                check=False,
                env=_scrubbed_subprocess_environment(),
            )
        except (OSError, subprocess.TimeoutExpired):
            raise BakeoffError("google_project_unavailable") from None
        if completed.returncode != 0 or len(completed.stdout) > 1024:
            raise BakeoffError("google_project_unavailable")
        try:
            project = completed.stdout.decode("utf-8", errors="strict").strip()
        except UnicodeDecodeError:
            raise BakeoffError("google_project_unavailable") from None
    if not _PROJECT_ID_PATTERN.fullmatch(project):
        raise BakeoffError("google_project_invalid")
    return project


def _adc_bearer() -> str:
    binary = _resolved_executable("gcloud")
    if not binary:
        raise BakeoffError("google_adc_unavailable")
    try:
        completed = subprocess.run(
            [binary, "auth", "application-default", "print-access-token"],
            capture_output=True,
            timeout=30,
            check=False,
            env=_scrubbed_subprocess_environment(),
        )
    except (OSError, subprocess.TimeoutExpired):
        raise BakeoffError("google_adc_unavailable") from None
    if completed.returncode != 0 or not (20 <= len(completed.stdout) <= 16384):
        raise BakeoffError("google_adc_unavailable")
    try:
        token = completed.stdout.decode("utf-8", errors="strict").strip()
    except UnicodeDecodeError:
        raise BakeoffError("google_adc_unavailable") from None
    if not token or any(character.isspace() for character in token):
        raise BakeoffError("google_adc_unavailable")
    return token


def _live_credentials(environment: Mapping[str, str]) -> LiveCredentials:
    endpoint = environment.get(AZURE_ENDPOINT_ENV, "").strip()
    key = environment.get(AZURE_KEY_ENV, "").strip()
    if not endpoint:
        raise BakeoffError("azure_endpoint_unavailable")
    _validated_azure_base(endpoint)
    if not key or len(key) > 4096 or any(character.isspace() for character in key):
        raise BakeoffError("azure_key_unavailable")
    project = _safe_google_project(environment)
    bearer = _adc_bearer()
    return LiveCredentials(project, bearer, endpoint, key)


def _provider_readiness(
    credentials: LiveCredentials, transport: HttpTransport
) -> tuple[bool, bool, tuple[str, ...]]:
    """Perform two content-free, non-inference auth/model availability checks."""
    errors: list[str] = []
    google_ready = False
    azure_ready = False
    google_body = canonical_json_bytes(
        {
            "model": _gemini_model_resource(credentials.google_project),
            "contents": [
                {"role": "user", "parts": [{"text": "readiness"}]}
            ],
        }
    )
    try:
        response = transport.request(
            "POST",
            _gemini_count_tokens_endpoint(credentials.google_project),
            {
                "Authorization": f"Bearer {credentials.google_bearer}",
                "Content-Type": "application/json; charset=utf-8",
                "Accept": "application/json",
            },
            google_body,
            HTTP_TIMEOUT_SECONDS,
        )
        if response.status != 200:
            raise BakeoffError(
                f"gemini_readiness_http_{response.status}"
                if response.status
                in (400, 401, 403, 404, 408, 409, 429, 500, 502, 503, 504)
                else "gemini_readiness_http_error"
            )
        envelope = strict_json_loads(response.body, maximum_bytes=64 * 1024)
        if not isinstance(envelope, dict):
            raise BakeoffError("gemini_readiness_response_invalid")
        total_tokens = _strict_int(envelope.get("totalTokens"))
        if total_tokens <= 0:
            raise BakeoffError("gemini_readiness_response_invalid")
        google_ready = True
    except BakeoffError as error:
        errors.append(
            "gemini_readiness_transport_failed"
            if error.code == "transport_failed"
            else error.code
        )

    try:
        response = transport.request(
            "GET",
            _azure_model_url(credentials.azure_endpoint),
            {
                "Ocp-Apim-Subscription-Key": credentials.azure_key,
                "Accept": "application/json",
            },
            None,
            HTTP_TIMEOUT_SECONDS,
        )
        if response.status != 200:
            raise BakeoffError(
                f"azure_readiness_http_{response.status}"
                if response.status
                in (400, 401, 403, 404, 408, 409, 429, 500, 502, 503, 504)
                else "azure_readiness_http_error"
            )
        envelope = strict_json_loads(response.body, maximum_bytes=256 * 1024)
        if (
            not isinstance(envelope, dict)
            or envelope.get("modelId") != AZURE_MODEL
            or envelope.get("apiVersion") != AZURE_API_VERSION
        ):
            raise BakeoffError("azure_readiness_response_invalid")
        azure_ready = True
    except BakeoffError as error:
        errors.append(
            "azure_readiness_transport_failed"
            if error.code == "transport_failed"
            else error.code
        )
    return google_ready, azure_ready, tuple(sorted(set(errors)))


def preflight_live(
    *,
    corpus_root: Path,
    archive_path: Path,
    preregistration_path: Path,
    evidence_path: Path,
    allow_upload: bool,
    acknowledgement: str | None,
    max_cost_text: str | None,
    environment: Mapping[str, str] = os.environ,
    require_committed: bool = True,
    readiness_transport: HttpTransport | None = None,
) -> PreflightResult:
    """Validate every condition before reading either provider credential."""
    errors: list[str] = []
    pages: list[cgpg.Page] = []
    run_binding: RunBinding | None = None
    if environment.get(LIVE_ENV) != "1":
        errors.append("live_environment_gate_missing")
    if not allow_upload:
        errors.append("upload_permission_missing")
    if acknowledgement != UPLOAD_ACKNOWLEDGEMENT:
        errors.append("upload_acknowledgement_mismatch")
    if max_cost_text != "1.00":
        errors.append("max_cost_gate_mismatch")
    if not _output_destinations_ready(evidence_path):
        errors.append("evidence_destination_invalid")
    try:
        run_binding = verify_preregistration(
            preregistration_path, require_committed=require_committed
        )
    except BakeoffError as error:
        errors.append(error.code)
    try:
        pages = select_and_verify_fixed_inputs(corpus_root, archive_path)
    except BakeoffError as error:
        errors.append(error.code)
    planned_cost = conservative_planned_cost()
    if planned_cost != PLANNED_TARIFF_CEILING_USD:
        errors.append("planned_cost_contract_mismatch")
    if planned_cost > REQUIRED_MAX_COST_USD:
        errors.append("planned_cost_exceeds_ceiling")
    if errors:
        return PreflightResult(False, tuple(sorted(set(errors))))

    if run_binding is None:  # pragma: no cover - error collection invariant.
        return PreflightResult(False, ("preflight_internal_error",))
    try:
        assert_run_binding_unchanged(run_binding, preregistration_path)
    except BakeoffError as error:
        return PreflightResult(False, (error.code,))

    try:
        credentials = _live_credentials(environment)
    except BakeoffError as error:
        return PreflightResult(False, (error.code,))
    google_ready, azure_ready, readiness_errors = _provider_readiness(
        credentials, readiness_transport or UrllibTransport()
    )
    if readiness_errors:
        return PreflightResult(
            False,
            readiness_errors,
            readiness_requests=2,
            google_ready=google_ready,
            azure_ready=azure_ready,
        )
    return PreflightResult(
        True,
        (),
        tuple(pages),
        credentials,
        run_binding,
        readiness_requests=2,
        google_ready=True,
        azure_ready=True,
    )


def provider_order(page_index: int, repetition: int) -> tuple[str, str]:
    if (page_index + repetition) % 2 == 0:
        return PROVIDERS
    return tuple(reversed(PROVIDERS))  # type: ignore[return-value]


def call_schedule(pages: Sequence[cgpg.Page]) -> list[tuple[cgpg.Page, int, str]]:
    return [
        (page, repetition, provider)
        for page_index, page in enumerate(pages)
        for repetition in range(REPETITIONS)
        for provider in provider_order(page_index, repetition)
    ]


def _assert_private_tree(path: Path) -> None:
    lexical_root = Path(os.path.abspath(RAW_OUTPUT_ROOT))
    lexical_path = Path(os.path.abspath(path))
    try:
        relative = lexical_path.relative_to(lexical_root)
    except ValueError:
        raise BakeoffError("raw_output_location_invalid") from None
    if lexical_root.exists() and lexical_root.is_symlink():
        raise BakeoffError("raw_output_symlink_rejected")
    try:
        repository_relative = lexical_root.relative_to(REPO_ROOT.absolute())
    except ValueError:
        repository_relative = None
    if repository_relative is not None:
        repository_component = REPO_ROOT.absolute()
        for part in repository_relative.parts:
            repository_component = repository_component / part
            if repository_component.exists() and repository_component.is_symlink():
                raise BakeoffError("raw_output_symlink_rejected")
    current = lexical_root
    for part in relative.parts:
        current = current / part
        if current.exists() and current.is_symlink():
            raise BakeoffError("raw_output_symlink_rejected")


def _output_destinations_ready(evidence_path: Path) -> bool:
    if (
        evidence_path.suffix.lower() != ".json"
        or evidence_path.exists()
        or evidence_path.is_symlink()
        or not evidence_path.parent.is_dir()
        or evidence_path.parent.is_symlink()
        or not os.access(evidence_path.parent, os.W_OK)
    ):
        return False
    try:
        _assert_private_tree(RAW_OUTPUT_ROOT / "preflight-probe")
        relative_probe = str(
            (RAW_OUTPUT_ROOT / "preflight-probe")
            .absolute()
            .relative_to(REPO_ROOT.absolute())
        )
        binary = _resolved_executable("git")
        if binary is None:
            return False
        ignored = subprocess.run(
            [binary, "check-ignore", "-q", relative_probe],
            cwd=REPO_ROOT,
            capture_output=True,
            timeout=30,
            check=False,
            env=_scrubbed_subprocess_environment(),
        )
    except (BakeoffError, OSError, ValueError, subprocess.TimeoutExpired):
        return False
    if ignored.returncode != 0:
        return False
    nearest = RAW_OUTPUT_ROOT
    while not nearest.exists() and nearest != nearest.parent:
        nearest = nearest.parent
    return nearest.is_dir() and not nearest.is_symlink() and os.access(nearest, os.W_OK)


def _private_write(path: Path, payload: bytes) -> None:
    if path.exists() or path.is_symlink():
        raise BakeoffError("raw_output_exists")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags, 0o600)
        try:
            os.fchmod(descriptor, 0o600)
            with os.fdopen(descriptor, "wb", closefd=False) as handle:
                handle.write(payload)
                handle.flush()
                os.fsync(handle.fileno())
        finally:
            os.close(descriptor)
    except OSError:
        raise BakeoffError("raw_output_write_failed") from None


def _usage_record(usage: ProviderUsage | None) -> dict | None:
    if usage is None:
        return None
    return {
        "prompt_tokens": usage.prompt_tokens,
        "candidate_tokens": usage.candidate_tokens,
        "thought_tokens": usage.thought_tokens,
        "cached_tokens": usage.cached_tokens,
        "accepted_pages": usage.accepted_pages,
        "billable_pages": usage.billable_pages,
        "native_words": usage.native_words,
        "confidence_count": usage.confidence_count,
        "confidence_sum": usage.confidence_sum,
        "confidence_min": usage.confidence_min,
        "confidence_max": usage.confidence_max,
    }


def _usage_from_record(value: object) -> ProviderUsage | None:
    if value is None:
        return None
    keys = {
        "prompt_tokens",
        "candidate_tokens",
        "thought_tokens",
        "cached_tokens",
        "accepted_pages",
        "billable_pages",
        "native_words",
        "confidence_count",
        "confidence_sum",
        "confidence_min",
        "confidence_max",
    }
    if not isinstance(value, dict) or set(value) != keys:
        raise BakeoffError("replay_record_invalid")
    integers = {
        key: _strict_int(value[key])
        for key in keys
        if key
        not in {"confidence_sum", "confidence_min", "confidence_max"}
    }
    if any(item < 0 for item in integers.values()):
        raise BakeoffError("replay_record_invalid")
    confidence_sum = _strict_number(value["confidence_sum"])
    confidence_min = (
        None
        if value["confidence_min"] is None
        else _strict_number(value["confidence_min"])
    )
    confidence_max = (
        None
        if value["confidence_max"] is None
        else _strict_number(value["confidence_max"])
    )
    if (
        confidence_sum < 0.0
        or (confidence_min is None) != (confidence_max is None)
        or (
            integers["confidence_count"] == 0
            and (confidence_sum != 0.0 or confidence_min is not None)
        )
        or (
            integers["confidence_count"] > 0
            and (confidence_min is None or confidence_sum > integers["confidence_count"])
        )
        or (
            confidence_min is not None
            and not 0.0 <= confidence_min <= confidence_max <= 1.0
        )
    ):
        raise BakeoffError("replay_record_invalid")
    return ProviderUsage(
        **integers,
        confidence_sum=confidence_sum,
        confidence_min=confidence_min,
        confidence_max=confidence_max,
    )


def _provenance_record(provenance: ProviderProvenance | None) -> dict | None:
    if provenance is None:
        return None
    return {
        "model_version": provenance.model_version,
        "response_id_sha256": provenance.response_id_sha256,
        "model_id": provenance.model_id,
        "api_version": provenance.api_version,
    }


def _provenance_from_record(
    value: object, provider: str
) -> ProviderProvenance | None:
    if value is None:
        return None
    keys = {"model_version", "response_id_sha256", "model_id", "api_version"}
    if not isinstance(value, dict) or set(value) != keys:
        raise BakeoffError("replay_record_invalid")
    for key in keys:
        if value[key] is not None and (
            not isinstance(value[key], str) or len(value[key]) > 1024
        ):
            raise BakeoffError("replay_record_invalid")
    digest = value["response_id_sha256"]
    if digest is not None and not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise BakeoffError("replay_record_invalid")
    provenance = ProviderProvenance(**value)
    if provider == "google_vertex_gemini":
        valid = (
            _valid_gemini_model_version(provenance.model_version)
            and provenance.response_id_sha256 is not None
            and provenance.model_id is None
            and provenance.api_version is None
        )
    elif provider == "azure_document_intelligence":
        valid = (
            provenance.model_version is None
            and provenance.response_id_sha256 is None
            and provenance.model_id == AZURE_MODEL
            and provenance.api_version == AZURE_API_VERSION
        )
    else:  # pragma: no cover - _stem validates the provider first.
        valid = False
    if not valid:
        raise BakeoffError("replay_record_invalid")
    return provenance


def _schedule_record(
    entries: Sequence[tuple[cgpg.Page, int, str]],
) -> list[dict[str, str | int]]:
    return [
        {
            "provider": provider,
            "page_id": page.name,
            "repetition": repetition,
        }
        for page, repetition, provider in entries
    ]


class RawStore:
    def __init__(
        self,
        root: Path,
        *,
        create: bool,
        run_binding: RunBinding | None = None,
    ) -> None:
        _assert_private_tree(root)
        self.root = root
        self.run_binding = run_binding
        self.manifest_sha256: str | None = None
        if create:
            if root.exists() or root.is_symlink():
                raise BakeoffError("raw_output_exists")
            RAW_OUTPUT_ROOT.mkdir(parents=True, exist_ok=True, mode=0o700)
            os.chmod(RAW_OUTPUT_ROOT, 0o700)
            root.mkdir(mode=0o700)
            os.chmod(root, 0o700)
        else:
            try:
                invalid = (
                    not root.is_dir()
                    or root.is_symlink()
                    or bool(root.stat().st_mode & 0o077)
                )
            except OSError:
                invalid = True
            if invalid:
                raise BakeoffError("replay_directory_invalid")

    @staticmethod
    def _stem(provider: str, page_id: str, repetition: int) -> str:
        if (
            provider not in PROVIDERS
            or page_id not in {item["page_id"] for item in FIXED_INPUT_RECORDS}
            or repetition not in range(REPETITIONS)
        ):
            raise BakeoffError("raw_record_identifier_invalid")
        return f"{provider}--{page_id}--r{repetition}"

    @staticmethod
    def _response_fields(raw: bytes | None) -> dict[str, object]:
        return {
            "response_present": raw is not None,
            "response_size_bytes": len(raw) if raw is not None else None,
            "response_sha256": sha256_bytes(raw) if raw is not None else None,
        }

    def _write_meta(
        self,
        provider: str,
        page_id: str,
        repetition: int,
        *,
        status: str,
        error_code: str | None,
        provider_response_success: bool,
        latency_seconds: float | None,
        usage: ProviderUsage | None,
        provenance: ProviderProvenance | None,
        raw: bytes | None,
        invalid_boxes: int,
        degenerate_boxes: int,
        out_of_page_boxes: int,
        duplicate_boxes: int,
    ) -> None:
        stem = self._stem(provider, page_id, repetition)
        if (
            latency_seconds is None
            or not math.isfinite(latency_seconds)
            or latency_seconds < 0.0
        ):
            raise BakeoffError("raw_outcome_invalid")
        if raw is not None:
            _private_write(self.root / f"{stem}.response.json", raw)
        meta = {
            "schema": "mpdf-cloud-complete-ocr-raw-meta",
            "schema_version": "3.0",
            "provider": provider,
            "page_id": page_id,
            "repetition": repetition,
            "status": status,
            "error_code": error_code,
            "provider_response_success": provider_response_success,
            "latency_seconds": latency_seconds,
            "usage": _usage_record(usage),
            "provenance": _provenance_record(provenance),
            "invalid_boxes": invalid_boxes,
            "degenerate_boxes": degenerate_boxes,
            "out_of_page_boxes": out_of_page_boxes,
            "duplicate_boxes": duplicate_boxes,
            **self._response_fields(raw),
        }
        _private_write(
            self.root / f"{stem}.meta.json", canonical_json_bytes(meta)
        )

    def write(
        self,
        provider: str,
        page_id: str,
        repetition: int,
        result: ProviderResult,
    ) -> None:
        self._write_meta(
            provider,
            page_id,
            repetition,
            status="success",
            error_code=None,
            provider_response_success=True,
            latency_seconds=result.latency_seconds,
            usage=result.usage,
            provenance=result.provenance,
            raw=result.raw_response,
            invalid_boxes=0,
            degenerate_boxes=0,
            out_of_page_boxes=0,
            duplicate_boxes=0,
        )

    def write_failure(
        self,
        provider: str,
        page_id: str,
        repetition: int,
        error: BakeoffError,
    ) -> None:
        self._write_meta(
            provider,
            page_id,
            repetition,
            status="failure",
            error_code=error.code,
            provider_response_success=error.provider_response_success,
            latency_seconds=error.latency_seconds,
            usage=error.usage,
            provenance=error.provenance,
            raw=error.raw_response,
            invalid_boxes=error.invalid_boxes,
            degenerate_boxes=error.degenerate_boxes,
            out_of_page_boxes=error.out_of_page_boxes,
            duplicate_boxes=error.duplicate_boxes,
        )

    def finalize_schedule(
        self, entries: Sequence[tuple[cgpg.Page, int, str]]
    ) -> None:
        expected = _schedule_record(entries)
        expected_meta = {
            f"{self._stem(item['provider'], item['page_id'], item['repetition'])}.meta.json"
            for item in expected
        }
        expected_response = {
            name.removesuffix(".meta.json") + ".response.json"
            for name in expected_meta
        }
        try:
            actual_meta = {path.name for path in self.root.glob("*.meta.json")}
            actual_names = {path.name for path in self.root.iterdir()}
        except OSError:
            raise BakeoffError("raw_output_completeness_failed") from None
        if (
            self.run_binding is None
            or len(expected)
            != SCORABLE_PAGE_COUNT * REPETITIONS * len(PROVIDERS)
            or actual_meta != expected_meta
            or not actual_names.issubset(expected_meta | expected_response)
        ):
            raise BakeoffError("raw_output_completeness_failed")
        records: list[dict[str, object]] = []
        try:
            for item in expected:
                stem = self._stem(
                    str(item["provider"]),
                    str(item["page_id"]),
                    int(item["repetition"]),
                )
                meta_raw = self._read_private_bytes(
                    self.root / f"{stem}.meta.json", maximum_bytes=64 * 1024
                )
                records.append({**item, "meta_sha256": sha256_bytes(meta_raw)})
        except BakeoffError:
            raise BakeoffError("raw_output_completeness_failed") from None
        manifest = {
            "schema": "mpdf-cloud-complete-ocr-raw-run-manifest",
            "schema_version": "3.0",
            "formal_input_manifest_sha256": FORMAL_INPUT_MANIFEST_SHA256,
            "run_binding": self.run_binding.as_record(),
            "completed_records": len(expected),
            "records": records,
        }
        encoded = canonical_json_bytes(manifest)
        _private_write(self.root / "run-manifest.json", encoded)
        self.manifest_sha256 = sha256_bytes(encoded)

    def verify_schedule(
        self,
        entries: Sequence[tuple[cgpg.Page, int, str]],
        *,
        expected_manifest_sha256: str | None = None,
    ) -> dict[str, dict[tuple[str, int], InvocationOutcome]]:
        expected = _schedule_record(entries)
        anchor = expected_manifest_sha256 or self.manifest_sha256
        if (
            self.run_binding is None
            or anchor is None
            or not re.fullmatch(r"[0-9a-f]{64}", anchor)
        ):
            raise BakeoffError("replay_schedule_invalid")
        records: list[dict[str, object]] = []
        outcomes: dict[str, dict[tuple[str, int], InvocationOutcome]] = {
            provider: {} for provider in PROVIDERS
        }
        try:
            manifest_raw = self._read_private_bytes(
                self.root / "run-manifest.json", maximum_bytes=256 * 1024
            )
            if sha256_bytes(manifest_raw) != anchor:
                raise BakeoffError("replay_schedule_invalid")
            manifest = strict_json_loads(
                manifest_raw, maximum_bytes=256 * 1024
            )
            for (page, repetition, provider), item in zip(entries, expected):
                stem = self._stem(
                    str(item["provider"]),
                    str(item["page_id"]),
                    int(item["repetition"]),
                )
                meta_raw = self._read_private_bytes(
                    self.root / f"{stem}.meta.json", maximum_bytes=64 * 1024
                )
                records.append({**item, "meta_sha256": sha256_bytes(meta_raw)})
                outcomes[provider][(page.name, repetition)] = (
                    self._outcome_from_meta_bytes(
                        provider,
                        page.name,
                        repetition,
                        page.width,
                        page.height,
                        meta_raw,
                    )
                )
        except BakeoffError:
            raise BakeoffError("replay_schedule_invalid") from None
        if not isinstance(manifest, dict) or manifest != {
            "schema": "mpdf-cloud-complete-ocr-raw-run-manifest",
            "schema_version": "3.0",
            "formal_input_manifest_sha256": FORMAL_INPUT_MANIFEST_SHA256,
            "run_binding": self.run_binding.as_record(),
            "completed_records": len(expected),
            "records": records,
        }:
            raise BakeoffError("replay_schedule_invalid")
        expected_meta = {
            f"{self._stem(item['provider'], item['page_id'], item['repetition'])}.meta.json"
            for item in expected
        }
        expected_response = {
            name.removesuffix(".meta.json") + ".response.json"
            for name in expected_meta
        }
        try:
            actual_meta = {path.name for path in self.root.glob("*.meta.json")}
            actual_names = {path.name for path in self.root.iterdir()}
        except OSError:
            raise BakeoffError("replay_schedule_invalid") from None
        if actual_meta != expected_meta or not actual_names.issubset(
            expected_meta | expected_response | {"run-manifest.json"}
        ):
            raise BakeoffError("replay_schedule_invalid")
        self.manifest_sha256 = anchor
        return outcomes

    @staticmethod
    def _read_private_bytes(path: Path, *, maximum_bytes: int) -> bytes:
        descriptor: int | None = None
        try:
            if not hasattr(os, "O_NOFOLLOW"):
                raise BakeoffError("replay_record_invalid")
            descriptor = os.open(
                path,
                os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
            )
            stat_result = os.fstat(descriptor)
            if (
                not stat.S_ISREG(stat_result.st_mode)
                or stat_result.st_mode & 0o077
                or stat_result.st_size > maximum_bytes
            ):
                raise BakeoffError("replay_record_invalid")
            with os.fdopen(descriptor, "rb", closefd=False) as handle:
                return _read_bounded(handle, maximum_bytes)
        except (BakeoffError, OSError):
            raise BakeoffError("replay_record_invalid") from None
        finally:
            if descriptor is not None:
                try:
                    os.close(descriptor)
                except OSError:
                    pass

    @classmethod
    def _read_private_json(cls, path: Path, *, maximum_bytes: int) -> object:
        return strict_json_loads(
            cls._read_private_bytes(path, maximum_bytes=maximum_bytes),
            maximum_bytes=maximum_bytes,
        )

    def read_outcome(
        self, provider: str, page_id: str, repetition: int, width: int, height: int
    ) -> InvocationOutcome:
        stem = self._stem(provider, page_id, repetition)
        meta_raw = self._read_private_bytes(
            self.root / f"{stem}.meta.json", maximum_bytes=64 * 1024
        )
        return self._outcome_from_meta_bytes(
            provider, page_id, repetition, width, height, meta_raw
        )

    def _outcome_from_meta_bytes(
        self,
        provider: str,
        page_id: str,
        repetition: int,
        width: int,
        height: int,
        meta_raw: bytes,
    ) -> InvocationOutcome:
        stem = self._stem(provider, page_id, repetition)
        meta = strict_json_loads(meta_raw, maximum_bytes=64 * 1024)
        expected_keys = {
            "schema",
            "schema_version",
            "provider",
            "page_id",
            "repetition",
            "status",
            "error_code",
            "provider_response_success",
            "latency_seconds",
            "usage",
            "provenance",
            "invalid_boxes",
            "degenerate_boxes",
            "out_of_page_boxes",
            "duplicate_boxes",
            "response_present",
            "response_size_bytes",
            "response_sha256",
        }
        if (
            not isinstance(meta, dict)
            or set(meta) != expected_keys
            or meta.get("schema") != "mpdf-cloud-complete-ocr-raw-meta"
            or meta.get("schema_version") != "3.0"
            or meta.get("provider") != provider
            or meta.get("page_id") != page_id
            or meta.get("repetition") != repetition
            or not isinstance(meta.get("provider_response_success"), bool)
            or not isinstance(meta.get("response_present"), bool)
        ):
            raise BakeoffError("replay_record_invalid")
        latency = (
            None
            if meta["latency_seconds"] is None
            else _strict_number(meta["latency_seconds"])
        )
        if latency is None or latency < 0.0:
            raise BakeoffError("replay_record_invalid")
        usage = _usage_from_record(meta["usage"])
        provenance = _provenance_from_record(meta["provenance"], provider)
        box_counts = {
            key: _strict_int(meta[key])
            for key in (
                "invalid_boxes",
                "degenerate_boxes",
                "out_of_page_boxes",
                "duplicate_boxes",
            )
        }
        if any(item < 0 for item in box_counts.values()):
            raise BakeoffError("replay_record_invalid")
        response_path = self.root / f"{stem}.response.json"
        raw: bytes | None = None
        if meta["response_present"]:
            raw = self._read_private_bytes(
                response_path, maximum_bytes=MAX_RESPONSE_BYTES
            )
            if (
                meta["response_size_bytes"] != len(raw)
                or meta["response_sha256"] != sha256_bytes(raw)
            ):
                raise BakeoffError("replay_record_invalid")
        elif (
            meta["response_size_bytes"] is not None
            or meta["response_sha256"] is not None
            or response_path.exists()
            or response_path.is_symlink()
        ):
            raise BakeoffError("replay_record_invalid")
        if meta["status"] == "success":
            if (
                raw is None
                or latency is None
                or meta["error_code"] is not None
                or meta["provider_response_success"] is not True
                or usage is None
                or provenance is None
                or any(box_counts.values())
            ):
                raise BakeoffError("replay_record_invalid")
            parsed = (
                parse_gemini_response(raw, width, height)
                if provider == "google_vertex_gemini"
                else parse_azure_response(raw, width, height)
            )
            if parsed.usage != usage or parsed.provenance != provenance:
                raise BakeoffError("replay_record_invalid")
            return InvocationOutcome(replace(parsed, latency_seconds=latency))
        if (
            meta["status"] != "failure"
            or not isinstance(meta["error_code"], str)
            or not _STABLE_CODE_PATTERN.fullmatch(meta["error_code"])
        ):
            raise BakeoffError("replay_record_invalid")
        return InvocationOutcome(
            None,
            meta["error_code"],
            provider_response_success=meta["provider_response_success"],
            latency_seconds=latency,
            usage=usage,
            provenance=provenance,
            **box_counts,
        )

    def read(
        self, provider: str, page_id: str, repetition: int, width: int, height: int
    ) -> ProviderResult:
        outcome = self.read_outcome(provider, page_id, repetition, width, height)
        if outcome.result is not None:
            return outcome.result
        error = BakeoffError(
            outcome.error_code or "replay_record_invalid",
            invalid_boxes=max(
                0,
                outcome.invalid_boxes
                - outcome.degenerate_boxes
                - outcome.out_of_page_boxes
                - outcome.duplicate_boxes,
            ),
            degenerate_boxes=outcome.degenerate_boxes,
            out_of_page_boxes=outcome.out_of_page_boxes,
            duplicate_boxes=outcome.duplicate_boxes,
        )
        error.provider_response_success = outcome.provider_response_success
        error.latency_seconds = outcome.latency_seconds
        error.usage = outcome.usage
        error.provenance = outcome.provenance
        raise error


class Recognizer(Protocol):
    def recognize(self, image_bytes: bytes, width: int, height: int) -> ProviderResult: ...


def _record_for_page(page_id: str) -> dict:
    for record in FIXED_INPUT_RECORDS:
        if record["page_id"] == page_id:
            return record
    raise BakeoffError("fixed_page_selection_mismatch")


def execute_schedule(
    pages: Sequence[cgpg.Page],
    clients: Mapping[str, Recognizer],
    raw_store: RawStore,
    budget: BudgetGuard,
) -> dict[str, dict[tuple[str, int], InvocationOutcome]]:
    outcomes: dict[str, dict[tuple[str, int], InvocationOutcome]] = {
        provider: {} for provider in PROVIDERS
    }
    for page_index, page in enumerate(pages):
        record = _record_for_page(page.name)
        for repetition in range(REPETITIONS):
            try:
                image_bytes = page.image_path.read_bytes()
            except OSError:
                raise BakeoffError("fixed_page_became_unreadable") from None
            if (
                len(image_bytes) != record["image"]["size_bytes"]
                or sha256_bytes(image_bytes) != record["image"]["sha256"]
                or image_bytes[:3] != b"\xff\xd8\xff"
            ):
                raise BakeoffError("fixed_page_changed_after_preflight")
            # Both recognizers receive this exact immutable bytes object.
            for provider in provider_order(page_index, repetition):
                budget.reserve(provider, record)
                try:
                    result = clients[provider].recognize(
                        image_bytes, page.width, page.height
                    )
                    raw_store.write(provider, page.name, repetition, result)
                    outcome = InvocationOutcome(result)
                    status = "ok"
                except BakeoffError as error:
                    raw_store.write_failure(
                        provider, page.name, repetition, error
                    )
                    outcome = InvocationOutcome(
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
                    status = error.code
                outcomes[provider][(page.name, repetition)] = outcome
                print(
                    json.dumps(
                        {
                            "provider": provider,
                            "page_id": page.name,
                            "repetition": repetition,
                            "status": status,
                        },
                        separators=(",", ":"),
                    ),
                    file=sys.stderr,
                    flush=True,
                )
    raw_store.finalize_schedule(call_schedule(pages))
    return outcomes


def replay_schedule(
    pages: Sequence[cgpg.Page],
    raw_store: RawStore,
    *,
    expected_manifest_sha256: str | None = None,
) -> dict[str, dict[tuple[str, int], InvocationOutcome]]:
    return raw_store.verify_schedule(
        call_schedule(pages),
        expected_manifest_sha256=expected_manifest_sha256,
    )


def _code_paths() -> dict[str, Path]:
    return {
        "cgpg_loader": Path(cgpg.__file__).resolve(),
        "cgpg_region_scorer": Path(run_cgpg.__file__).resolve(),
        "prior_selector_and_cer_metrics": Path(
            prior_bakeoff.__file__
        ).resolve(),
        "levenshtein_backend": Path(gold_metrics.__file__).resolve(),
        "cloud_complete_ocr_bakeoff_harness": Path(__file__).resolve(),
    }


def _code_digests() -> dict[str, str]:
    return {name: sha256_file(path) for name, path in _code_paths().items()}


def capture_run_binding(preregistration_path: Path) -> RunBinding:
    return RunBinding(
        preregistration_sha256=sha256_file(preregistration_path),
        configuration_sha256=canonical_digest(frozen_configuration()),
        code_sha256=tuple(sorted(_code_digests().items())),
    )


def assert_run_binding_unchanged(
    expected: RunBinding, preregistration_path: Path
) -> None:
    if capture_run_binding(preregistration_path) != expected:
        raise BakeoffError("run_binding_changed")


def reproducible_live_argv() -> list[str]:
    return [
        "python3",
        "scripts/ocr/gold/run_cloud_complete_ocr_bakeoff.py",
        "--corpus",
        "<CGPG_DATA_DIR>",
        "--archive",
        "<CGPG_DATA_V2_ZIP>",
        "--preregistration",
        "docs/evidence/cloud-complete-ocr-bakeoff-preregistration-2026-08-30.json",
        "--evidence",
        "docs/evidence/cloud-complete-ocr-bakeoff-2026-08-30.json",
        "--allow-upload",
        "--acknowledge-upload",
        UPLOAD_ACKNOWLEDGEMENT,
        "--max-cost-usd",
        "1.00",
    ]


def _comparison(gemini: dict, azure: dict) -> dict:
    def summary(record: dict) -> dict:
        return {
            "polytonic_cer": record["recognition_metrics"]["polytonic"]["cer"],
            "base_letter_cer": record["recognition_metrics"]["base_letter"]["cer"],
            "box_f1_at_0_5": record["box_alignment"]["at_0_5"]["f1"],
            "box_f1_at_0_75": record["box_alignment"]["at_0_75"]["f1"],
            "matched_mean_iou": record["box_alignment"]["matched_mean_iou"],
            "reading_order_pairwise_accuracy": record["reading_order"][
                "pairwise_accuracy"
            ],
            "reading_order_matched_gt_coverage": record["reading_order"][
                "matched_gt_coverage"
            ],
            "mean_latency_seconds": record["latency"]["mean_seconds"],
            "tariff_cost_usd": record["cost"]["tariff_cost_usd"],
            "known_measured_usage_tariff_cost_lower_bound_usd": record["cost"][
                "known_measured_usage_tariff_cost_lower_bound_usd"
            ],
            "cost_status": record["cost"]["cost_status"],
            "usage_complete": record["cost"]["usage_complete"],
            "actual_billed_cost": record["cost"]["actual_billed_cost"],
        }

    return {
        "google_vertex_gemini": summary(gemini),
        "azure_document_intelligence": summary(azure),
    }


_FORBIDDEN_EVIDENCE_KEYS = frozenset(
    {
        "text",
        "body",
        "raw_response",
        "response_body",
        "endpoint",
        "project",
        "project_id",
        "account",
        "account_id",
        "credential",
        "authorization",
        "operation_location",
        "image_path",
        "xml_path",
        "corpus_path",
    }
)
_ABSOLUTE_PATH_PATTERN = re.compile(
    r"(?:^|[\s\"'(=])(?:/(?!/)[^\s\"']*|[A-Za-z]:[\\/][^\s\"']*|"
    r"\\\\[^\\/\s]+[\\/][^\s\"']*)"
)


def assert_evidence_safe(
    value: object, *, forbidden_values: Iterable[str] = ()
) -> None:
    """Reject secrets, paths, and content-bearing field names before write."""
    forbidden = [item for item in forbidden_values if item]

    def visit(item: object) -> None:
        if isinstance(item, dict):
            for key, child in item.items():
                if str(key).lower() in _FORBIDDEN_EVIDENCE_KEYS:
                    raise BakeoffError("evidence_content_policy_violation")
                visit(child)
        elif isinstance(item, list):
            for child in item:
                visit(child)
        elif isinstance(item, str):
            if _ABSOLUTE_PATH_PATTERN.search(item):
                raise BakeoffError("evidence_content_policy_violation")
            if any(secret in item for secret in forbidden):
                raise BakeoffError("evidence_content_policy_violation")

    visit(value)


def build_evidence(
    pages: Sequence[cgpg.Page],
    outcomes: Mapping[str, Mapping[tuple[str, int], InvocationOutcome]],
    run_binding: RunBinding,
    raw_run_manifest_sha256: str,
) -> dict:
    evaluations = {
        provider: evaluate_provider(provider, pages, outcomes[provider])
        for provider in PROVIDERS
    }
    gemini = evaluations["google_vertex_gemini"]
    azure = evaluations["azure_document_intelligence"]
    report: dict = {
        "schema": "mpdf-cloud-complete-ocr-bakeoff",
        "schema_version": "1.0",
        "date": date.today().isoformat(),
        "status": "completed",
        "content_policy": (
            "Metrics, usage, latency, tariffs, stable error codes, page IDs, "
            "and provenance digests only. No OCR or ground-truth strings, "
            "provider response bodies, images, filesystem paths, endpoints, "
            "project/account identifiers, or credentials are serialized."
        ),
        "scope": "isolated_evaluation_only",
        "corpus": {
            "doi": CORPUS_DOI,
            "license": CORPUS_LICENSE,
            "archive_sha256": CORPUS_ARCHIVE_SHA256,
            "formal_input_manifest_sha256": FORMAL_INPUT_MANIFEST_SHA256,
            "legacy_audit_manifest_sha256": LEGACY_AUDIT_MANIFEST_SHA256,
            "selection_rule": (
                "first 12 scorable pages by page ID from the deterministic "
                "CGPG holdout"
            ),
            "page_ids": [page.name for page in pages],
        },
        "execution": {
            "repetitions_per_provider_page": REPETITIONS,
            "planned_provider_calls": len(pages) * REPETITIONS * len(PROVIDERS),
            "concurrency": 1,
            "interleaving": "page_repetition_parity",
            "identical_jpeg_bytes_enforced": True,
            "automatic_retries": 0,
            "raw_responses_stored_in_ignored_private_output": True,
        },
        "preregistration_sha256": run_binding.preregistration_sha256,
        "configuration_sha256": run_binding.configuration_sha256,
        "code_sha256": dict(run_binding.code_sha256),
        "raw_run_manifest_sha256": raw_run_manifest_sha256,
        "providers": evaluations,
        "comparison": _comparison(gemini, azure),
        "decision": decide(gemini, azure),
        "limitations": [
            "The fixed CGPG slice is single-column Greek material and does not validate multi-column or complex-layout reading order.",
            "The public CGPG package is not the paper's 30-page test split, so these results cannot be compared with the paper's headline accuracy.",
            "Tariff cost is calculated from response-reported measured usage; if potentially billable usage is incomplete, only a known lower bound is reported and the total is null. Actual billed cost remains null without a billing export or receipt.",
            "A passing bake-off is comparative evidence only and does not make model-generated boxes production-eligible under ADR 0011.",
        ],
    }
    report["run_sha256"] = canonical_digest(report)
    return report


def evidence_scan_values(
    pages: Sequence[cgpg.Page],
    outcomes: Mapping[str, Mapping[tuple[str, int], InvocationOutcome]],
    extras: Iterable[str] = (),
) -> tuple[str, ...]:
    values = [item for item in extras if item]
    values.extend(
        line.text
        for page in pages
        for region in page.regions
        for line in region.lines
        if len(line.text) >= 16
    )
    values.extend(
        line.text
        for provider_outcomes in outcomes.values()
        for outcome in provider_outcomes.values()
        if outcome.result is not None
        for line in outcome.result.lines
        if len(line.text) >= 16
    )
    return tuple(values)


def write_evidence(
    path: Path, report: dict, *, forbidden_values: Iterable[str] = ()
) -> None:
    assert_evidence_safe(report, forbidden_values=forbidden_values)
    if path.exists() or path.is_symlink() or not path.parent.is_dir():
        raise BakeoffError("evidence_destination_invalid")
    payload = json.dumps(report, ensure_ascii=False, indent=2).encode("utf-8") + b"\n"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags, 0o644)
        try:
            os.fchmod(descriptor, 0o644)
            with os.fdopen(descriptor, "wb", closefd=False) as handle:
                handle.write(payload)
                handle.flush()
                os.fsync(handle.fileno())
        finally:
            os.close(descriptor)
    except OSError:
        raise BakeoffError("evidence_write_failed") from None


def _new_raw_run_directory() -> Path:
    timestamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    nonce = os.urandom(6).hex()
    return RAW_OUTPUT_ROOT / f"run-{timestamp}-{nonce}"


def _parse_arguments(argv: Sequence[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument(
        "--preregistration", type=Path, default=DEFAULT_PREREGISTRATION
    )
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument("--allow-upload", action="store_true")
    parser.add_argument("--acknowledge-upload")
    parser.add_argument("--max-cost-usd")
    parser.add_argument("--preflight-only", action="store_true")
    parser.add_argument("--replay-dir", type=Path)
    parser.add_argument("--expected-raw-manifest-sha256")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = _parse_arguments(argv)
    if args.replay_dir is not None:
        if (
            args.preflight_only
            or not isinstance(args.expected_raw_manifest_sha256, str)
            or not re.fullmatch(
                r"[0-9a-f]{64}", args.expected_raw_manifest_sha256
            )
        ):
            raise BakeoffError("replay_arguments_invalid")
        run_binding = verify_preregistration(
            args.preregistration, require_committed=True
        )
        pages = select_and_verify_fixed_inputs(args.corpus, args.archive)
        if args.evidence.exists() or args.evidence.suffix.lower() != ".json":
            raise BakeoffError("evidence_destination_invalid")
        store = RawStore(
            args.replay_dir, create=False, run_binding=run_binding
        )
        outcomes = replay_schedule(
            pages,
            store,
            expected_manifest_sha256=args.expected_raw_manifest_sha256,
        )
        assert_run_binding_unchanged(run_binding, args.preregistration)
        if store.manifest_sha256 is None:  # pragma: no cover - invariant.
            raise BakeoffError("replay_schedule_invalid")
        report = build_evidence(
            pages, outcomes, run_binding, store.manifest_sha256
        )
        assert_run_binding_unchanged(run_binding, args.preregistration)
        write_evidence(
            args.evidence,
            report,
            forbidden_values=evidence_scan_values(
                pages,
                outcomes,
                (
                    str(args.corpus),
                    str(args.archive),
                    str(args.preregistration),
                    str(store.root),
                ),
            ),
        )
        print(json.dumps({"status": "completed_replay", "decision": report["decision"]["outcome"]}))
        return 0

    if args.expected_raw_manifest_sha256 is not None:
        raise BakeoffError("replay_arguments_invalid")

    preflight = preflight_live(
        corpus_root=args.corpus,
        archive_path=args.archive,
        preregistration_path=args.preregistration,
        evidence_path=args.evidence,
        allow_upload=args.allow_upload,
        acknowledgement=args.acknowledge_upload,
        max_cost_text=args.max_cost_usd,
    )
    if not preflight.ready:
        print(json.dumps(preflight.public(), separators=(",", ":")))
        return 2
    if args.preflight_only:
        print(json.dumps(preflight.public(), separators=(",", ":")))
        return 0
    if (
        preflight.credentials is None or preflight.run_binding is None
    ):  # pragma: no cover - dataclass invariant.
        raise BakeoffError("preflight_internal_error")

    raw_store = RawStore(
        _new_raw_run_directory(),
        create=True,
        run_binding=preflight.run_binding,
    )
    transport = UrllibTransport()
    clients: dict[str, Recognizer] = {
        "google_vertex_gemini": GeminiClient(
            preflight.credentials.google_project,
            preflight.credentials.google_bearer,
            transport,
        ),
        "azure_document_intelligence": AzureClient(
            preflight.credentials.azure_endpoint,
            preflight.credentials.azure_key,
            transport,
        ),
    }
    budget = BudgetGuard(REQUIRED_MAX_COST_USD)
    assert_run_binding_unchanged(
        preflight.run_binding, args.preregistration
    )
    execute_schedule(preflight.pages, clients, raw_store, budget)
    if raw_store.manifest_sha256 is None:  # pragma: no cover - invariant.
        raise BakeoffError("raw_output_completeness_failed")
    outcomes = raw_store.verify_schedule(
        call_schedule(preflight.pages),
        expected_manifest_sha256=raw_store.manifest_sha256,
    )
    assert_run_binding_unchanged(preflight.run_binding, args.preregistration)
    if raw_store.manifest_sha256 is None:  # pragma: no cover - invariant.
        raise BakeoffError("raw_output_completeness_failed")
    report = build_evidence(
        preflight.pages,
        outcomes,
        preflight.run_binding,
        raw_store.manifest_sha256,
    )
    assert_run_binding_unchanged(preflight.run_binding, args.preregistration)
    write_evidence(
        args.evidence,
        report,
        forbidden_values=evidence_scan_values(
            preflight.pages,
            outcomes,
            (
                preflight.credentials.google_project,
                preflight.credentials.google_bearer,
                preflight.credentials.azure_endpoint,
                preflight.credentials.azure_key,
                str(args.corpus),
                str(args.archive),
                str(args.preregistration),
                str(raw_store.root),
            ),
        ),
    )
    print(
        json.dumps(
            {"status": "completed_live", "decision": report["decision"]["outcome"]},
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BakeoffError as error:
        print(json.dumps({"status": "failed", "error_code": error.code}), file=sys.stderr)
        raise SystemExit(2) from None
