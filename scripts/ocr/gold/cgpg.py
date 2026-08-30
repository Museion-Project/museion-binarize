#!/usr/bin/env python3
"""Loader for the Patrologia Graeca OCR ground truth (CGPG), PAGE XML.

The corpus is an *external* stress test.  Everything in ``scripts/ocr/gold``
besides this module is synthetic and rendered by this repository, which means
it shares its own blind spots: it was the synthetic set that reported a Greek
to Latin confusion rate of 0.0010 while real pages measured 0.0096.  CGPG is
human-transcribed material this project did not produce, and it labels Greek
and Latin columns separately -- so a script router can be checked against a
human's idea of where the Greek is rather than against its own.

What this module deliberately does not do
-----------------------------------------
* It never downloads anything.  The archive is provisioned out of band into a
  temporary directory; the application has no code path that reaches it.
* It never writes into the repository.  Images and transcriptions are third
  party content under CC BY 4.0 and stay in the temporary directory.

Provenance
----------
DOI ``10.5281/zenodo.20008699``; ``data-v2.zip`` SHA-256
``2ee5d79f3c781dc1b64fa386f0f194a762ab183cd36b97f5d873ce0a3004e1f7``;
CC BY 4.0.  304 image/PAGE XML pairs, of which 266 carry line transcriptions
and 38 are predominantly layout annotation.

The public package is **not** the 30-page test split used in the paper that
accompanies the corpus, so results here may not be compared against that
paper's headline accuracy.
"""

from __future__ import annotations

import hashlib
import re
import unicodedata
import xml.etree.ElementTree as ElementTree
from dataclasses import dataclass, field
from pathlib import Path

PAGE_NAMESPACE = "http://schema.primaresearch.org/PAGE/gts/pagecontent/2013-07-15"
_NS = {"p": PAGE_NAMESPACE}

#: A transcription file far larger than the corpus's own worst case is a sign
#: of a substituted or hostile file, not of a dense page.
MAX_XML_BYTES = 8 * 1024 * 1024
#: Page scans in this corpus top out near 3 MB; the cap leaves generous room.
MAX_IMAGE_BYTES = 64 * 1024 * 1024
MAX_REGIONS_PER_PAGE = 512
MAX_LINES_PER_PAGE = 4096
MAX_POINTS_PER_POLYGON = 512
#: Extensions accepted when the declared image filename is not the one that
#: shipped.  See ``_resolve_image``.
IMAGE_EXTENSIONS = (".jpg", ".jpeg", ".png", ".tif", ".tiff")
#: Pages larger than this in either axis are rejected rather than rendered.
MAX_PAGE_PIXELS = 20000

#: Region types the corpus uses for the two body columns.
#:
#: These labels are *not* usable as script ground truth for scoring, and the
#: reason is worth recording: the corpus carries two disjoint annotation
#: layers.  109 ``MainText_ColGreek`` and 70 ``MainText_ColLatin`` regions
#: exist, but every one of them is on a layout-only page, and all 268
#: transcribed regions are typed generically as ``text``.  Nothing carries
#: both a script label and a transcription.  A scorer must therefore derive
#: the script from the human transcription itself — see
#: :func:`script_of_text`, which reads the reference, never the OCR output.
GREEK_REGION_TYPES = frozenset({"MainText_ColGreek"})
LATIN_REGION_TYPES = frozenset({"MainText_ColLatin"})

#: Share of a reference region's letters that must be Greek (or Latin) before
#: the region counts as that script for reporting.  Regions in between are
#: reported as ``mixed`` and never silently folded into either side.
SCRIPT_LABEL_RATIO = 0.70

_TYPE_PATTERN = re.compile(r"type:\s*([^;}]*)")
_DOCTYPE_PATTERN = re.compile(rb"<!\s*(DOCTYPE|ENTITY)", re.IGNORECASE)


class CgpgError(Exception):
    """A corpus file was missing, malformed, or unsafe."""


@dataclass(frozen=True)
class Line:
    text: str
    bbox: tuple[int, int, int, int]


@dataclass(frozen=True)
class Region:
    region_id: str
    region_type: str
    bbox: tuple[int, int, int, int]
    lines: tuple[Line, ...] = field(default_factory=tuple)

    @property
    def is_greek(self) -> bool:
        return self.region_type in GREEK_REGION_TYPES

    @property
    def is_latin(self) -> bool:
        return self.region_type in LATIN_REGION_TYPES

    @property
    def text(self) -> str:
        return "\n".join(line.text for line in self.lines if line.text)


@dataclass(frozen=True)
class Page:
    name: str
    image_path: Path
    xml_path: Path
    width: int
    height: int
    regions: tuple[Region, ...]

    @property
    def transcribed_lines(self) -> int:
        return sum(1 for region in self.regions for line in region.lines if line.text)

    @property
    def greek_regions(self) -> tuple[Region, ...]:
        return tuple(region for region in self.regions if region.is_greek)

    @property
    def latin_regions(self) -> tuple[Region, ...]:
        return tuple(region for region in self.regions if region.is_latin)


def script_of_text(text: str) -> str:
    """Labels a *reference* transcription as greek, latin, or mixed.

    The input must always be ground truth.  Labelling by OCR output would make
    the Greek bucket mean "what the recognizer thought was Greek", which is
    precisely the quantity under test.
    """
    greek = latin = 0
    for character in text:
        if not character.isalpha():
            continue
        code = ord(unicodedata.normalize("NFD", character)[0])
        if (0x0370 <= code <= 0x03FF) or (0x1F00 <= code <= 0x1FFF):
            greek += 1
        elif (0x0041 <= code <= 0x024F) or (0x1E00 <= code <= 0x1EFF):
            latin += 1
    letters = greek + latin
    if not letters:
        return "mixed"
    if greek / letters >= SCRIPT_LABEL_RATIO:
        return "greek"
    if latin / letters >= SCRIPT_LABEL_RATIO:
        return "latin"
    return "mixed"


def _reject_unsafe_xml(raw: bytes) -> None:
    """Refuses any document type declaration or entity definition.

    ElementTree does not resolve external entities, but a DOCTYPE is still the
    entry point for entity-expansion attacks against other parsers that might
    later read the same file, and a corpus file has no legitimate reason to
    carry one.  Rejecting outright is cheaper than reasoning about it.
    """
    if _DOCTYPE_PATTERN.search(raw):
        raise CgpgError("transcription declares a DOCTYPE or entity")


def _safe_sibling(root: Path, name: str) -> Path:
    """Resolves an image filename declared inside the XML, without escaping.

    ``imageFilename`` is attacker-controlled data as far as this loader is
    concerned: it comes from a file, not from us.
    """
    if not name or name != Path(name).name or name in (".", ".."):
        raise CgpgError(f"unsafe image filename: {name!r}")
    candidate = (root / name).resolve()
    if candidate.parent != root.resolve():
        raise CgpgError(f"image filename escapes the corpus directory: {name!r}")
    return candidate


def _resolve_image(xml_path: Path, declared: str) -> Path | None:
    """Finds the page image, tolerating the corpus's own naming inconsistency.

    158 of the 304 transcriptions name a ``.tif`` that the published archive
    ships as a ``.jpg`` of the same stem: the pages were transcribed against
    TIFFs and distributed as JPEGs. Insisting on the declared name would
    discard half the corpus.

    This is not a licence to load an arbitrary neighbour. The declared name is
    still resolved without escaping the corpus directory, any substitute must
    be a sibling with the transcription's own stem, and the caller still checks
    that the image's pixel dimensions equal the ones the transcription
    declares — which is what actually establishes that it is the right image.
    """
    root = xml_path.parent
    exact = _safe_sibling(root, declared)
    if exact.is_file():
        return exact
    for extension in IMAGE_EXTENSIONS:
        candidate = _safe_sibling(root, xml_path.stem + extension)
        if candidate.is_file():
            return candidate
    return None


def _polygon_bbox(
    points: str, width: int, height: int
) -> tuple[int, int, int, int] | None:
    """Returns the bounding box, or ``None`` when the polygon is empty.

    188 of the corpus's 16,347 ``Coords`` elements carry no points at all.
    That is real annotation data, not corruption, so it is skipped rather than
    treated as a malformed file — a loader that rejected the page would
    discard 300 usable pages over an empty attribute. Malformed points,
    points outside the page, and absurd point counts remain hard errors.
    """
    pairs = points.split()
    if not pairs:
        return None
    if len(pairs) > MAX_POINTS_PER_POLYGON:
        raise CgpgError("polygon has too many points")
    xs: list[int] = []
    ys: list[int] = []
    for pair in pairs:
        try:
            x_text, y_text = pair.split(",")
            x, y = int(x_text), int(y_text)
        except ValueError as error:
            raise CgpgError(f"malformed polygon point: {pair!r}") from error
        if not (0 <= x <= width and 0 <= y <= height):
            raise CgpgError(
                f"polygon point {x},{y} lies outside the {width}x{height} page"
            )
        xs.append(x)
        ys.append(y)
    return min(xs), min(ys), max(xs), max(ys)


def _region_type(element: ElementTree.Element) -> str:
    match = _TYPE_PATTERN.search(element.get("custom") or "")
    return (match.group(1) if match else "").strip().strip("'\"")


def load_page(xml_path: Path, verify_image: bool = True) -> Page:
    """Parses one PAGE XML file and validates it against its image."""
    xml_path = Path(xml_path)
    if not xml_path.is_file():
        raise CgpgError(f"transcription is missing: {xml_path}")
    if xml_path.stat().st_size > MAX_XML_BYTES:
        raise CgpgError(f"transcription is too large: {xml_path}")
    raw = xml_path.read_bytes()
    _reject_unsafe_xml(raw)
    try:
        root = ElementTree.fromstring(raw)
    except ElementTree.ParseError as error:
        raise CgpgError(f"transcription is not well-formed: {error}") from error

    page_element = root.find("p:Page", _NS)
    if page_element is None:
        raise CgpgError("transcription has no Page element")
    try:
        width = int(page_element.get("imageWidth", ""))
        height = int(page_element.get("imageHeight", ""))
    except ValueError as error:
        raise CgpgError("page declares no usable dimensions") from error
    if not (0 < width <= MAX_PAGE_PIXELS and 0 < height <= MAX_PAGE_PIXELS):
        raise CgpgError(f"page dimensions out of range: {width}x{height}")

    image_path = _resolve_image(xml_path, page_element.get("imageFilename", ""))
    if verify_image:
        if image_path is None or not image_path.is_file():
            raise CgpgError(
                f"page image is missing: {page_element.get('imageFilename', '')!r}"
            )
        if image_path.stat().st_size > MAX_IMAGE_BYTES:
            raise CgpgError(f"page image is too large: {image_path.name}")
        from PIL import Image  # noqa: PLC0415

        with Image.open(image_path) as handle:
            actual = handle.size
        if actual != (width, height):
            raise CgpgError(
                f"page image is {actual[0]}x{actual[1]} but the transcription "
                f"declares {width}x{height}"
            )

    regions: list[Region] = []
    total_lines = 0
    for region_element in page_element.findall("p:TextRegion", _NS):
        if len(regions) >= MAX_REGIONS_PER_PAGE:
            raise CgpgError("page declares too many regions")
        coords = region_element.find("p:Coords", _NS)
        if coords is None:
            continue
        bbox = _polygon_bbox(coords.get("points", ""), width, height)
        if bbox is None:
            continue
        lines: list[Line] = []
        for line_element in region_element.findall("p:TextLine", _NS):
            total_lines += 1
            if total_lines > MAX_LINES_PER_PAGE:
                raise CgpgError("page declares too many lines")
            line_coords = line_element.find("p:Coords", _NS)
            if line_coords is None:
                continue
            line_bbox = _polygon_bbox(line_coords.get("points", ""), width, height)
            if line_bbox is None:
                continue
            unicode_element = line_element.find(".//p:TextEquiv/p:Unicode", _NS)
            text = (unicode_element.text or "") if unicode_element is not None else ""
            # Ground truth is normalized exactly the way the sidecar normalizes
            # its own output, so a comparison measures recognition rather than
            # a normalization mismatch.
            lines.append(Line(unicodedata.normalize("NFC", text).strip(), line_bbox))
        regions.append(
            Region(
                region_id=region_element.get("id", ""),
                region_type=_region_type(region_element),
                bbox=bbox,
                lines=tuple(lines),
            )
        )

    return Page(
        name=xml_path.stem,
        image_path=image_path,
        xml_path=xml_path,
        width=width,
        height=height,
        regions=tuple(regions),
    )


def load_corpus(root: Path, verify_image: bool = True) -> list[Page]:
    """Loads every transcription in ``root``, sorted by name for determinism."""
    root = Path(root)
    if not root.is_dir():
        raise CgpgError(f"corpus directory is missing: {root}")
    return [
        load_page(path, verify_image=verify_image)
        for path in sorted(root.glob("*.xml"))
    ]


def split_name(name: str, holdout_every: int = 4) -> str:
    """Assigns a page to ``dev`` or ``holdout`` from a hash of its name.

    Hashing the name rather than taking a slice keeps the split stable when
    pages are added, removed, or reordered, and makes it impossible to widen
    the dev set by rerunning with a different sort.
    """
    digest = hashlib.sha256(name.encode("utf-8")).digest()
    return "holdout" if digest[0] % holdout_every == 0 else "dev"


def split_corpus(
    pages: list[Page], holdout_every: int = 4
) -> tuple[list[Page], list[Page]]:
    dev = [page for page in pages if split_name(page.name, holdout_every) == "dev"]
    holdout = [
        page for page in pages if split_name(page.name, holdout_every) == "holdout"
    ]
    return dev, holdout
