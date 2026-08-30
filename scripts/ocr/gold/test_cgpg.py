"""Tests for the CGPG PAGE XML loader.

These run without the corpus: every fixture is written inline, so the suite
gates the loader's safety properties on a machine that has never downloaded
the 400 MB archive.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cgpg  # noqa: E402

NS = cgpg.PAGE_NAMESPACE


def page_xml(
    *,
    image: str = "page.jpg",
    width: int = 200,
    height: int = 300,
    region_points: str = "10,10 190,10 190,290 10,290",
    line_points: str = "12,12 188,12 188,60 12,60",
    text: str = "ἐκεῖνο",
    region_type: str = "MainText_ColGreek",
    doctype: str = "",
) -> str:
    return (
        '<?xml version="1.0" ?>\n'
        f"{doctype}"
        f'<PcGts xmlns="{NS}">'
        f'<Page imageFilename="{image}" imageHeight="{height}" imageWidth="{width}">'
        f'<TextRegion id="r1" custom="structure {{type:{region_type};}}">'
        f'<Coords points="{region_points}"/>'
        f'<TextLine id="l1"><Coords points="{line_points}"/>'
        f"<TextEquiv><Unicode>{text}</Unicode></TextEquiv>"
        "</TextLine></TextRegion></Page></PcGts>"
    )


def write_page(directory: Path, xml: str, *, name: str = "page") -> Path:
    path = directory / f"{name}.xml"
    path.write_text(xml, encoding="utf-8")
    return path


def test_a_well_formed_page_loads_with_its_regions_and_lines(tmp_path):
    path = write_page(tmp_path, page_xml())
    page = cgpg.load_page(path, verify_image=False)
    assert page.width == 200 and page.height == 300
    assert len(page.regions) == 1
    region = page.regions[0]
    assert region.is_greek and not region.is_latin
    assert region.bbox == (10, 10, 190, 290)
    assert region.text == "ἐκεῖνο"
    assert page.transcribed_lines == 1


def test_greek_and_latin_columns_are_distinguished(tmp_path):
    greek = cgpg.load_page(write_page(tmp_path, page_xml(), name="g"), verify_image=False)
    latin = cgpg.load_page(
        write_page(tmp_path, page_xml(region_type="MainText_ColLatin"), name="l"),
        verify_image=False,
    )
    assert greek.greek_regions and not greek.latin_regions
    assert latin.latin_regions and not latin.greek_regions


def test_ground_truth_is_normalized_to_nfc(tmp_path):
    # Composed vs decomposed alpha with a rough breathing: the loader must
    # hand back NFC so a comparison measures recognition, not normalization.
    decomposed = "ἁ"
    path = write_page(tmp_path, page_xml(text=decomposed))
    page = cgpg.load_page(path, verify_image=False)
    assert page.regions[0].text == "ἁ"


def test_a_doctype_is_refused(tmp_path):
    path = write_page(tmp_path, page_xml(doctype="<!DOCTYPE PcGts [ ]>\n"))
    with pytest.raises(cgpg.CgpgError, match="DOCTYPE"):
        cgpg.load_page(path, verify_image=False)


def test_an_entity_declaration_is_refused(tmp_path):
    path = write_page(
        tmp_path, page_xml(doctype='<!ENTITY xxe SYSTEM "file:///etc/passwd">\n')
    )
    with pytest.raises(cgpg.CgpgError):
        cgpg.load_page(path, verify_image=False)


@pytest.mark.parametrize(
    "image",
    ["../escape.jpg", "/etc/passwd", "sub/dir.jpg", "", "..", "."],
)
def test_an_image_filename_may_not_escape_the_corpus_directory(tmp_path, image):
    path = write_page(tmp_path, page_xml(image=image))
    with pytest.raises(cgpg.CgpgError):
        cgpg.load_page(path, verify_image=False)


def test_a_polygon_outside_the_page_is_refused(tmp_path):
    path = write_page(tmp_path, page_xml(region_points="10,10 400,10 400,290 10,290"))
    with pytest.raises(cgpg.CgpgError, match="outside"):
        cgpg.load_page(path, verify_image=False)


def test_a_negative_polygon_point_is_refused(tmp_path):
    path = write_page(tmp_path, page_xml(line_points="-1,12 188,12 188,60 12,60"))
    with pytest.raises(cgpg.CgpgError, match="outside"):
        cgpg.load_page(path, verify_image=False)


def test_an_empty_polygon_is_skipped_not_treated_as_corruption(tmp_path):
    # 188 of the corpus's 16,347 Coords elements are empty. Rejecting the file
    # over one of them would throw away 300 otherwise usable pages.
    path = write_page(tmp_path, page_xml(line_points=""))
    page = cgpg.load_page(path, verify_image=False)
    assert page.regions[0].lines == ()
    assert page.transcribed_lines == 0


def test_a_region_with_empty_geometry_is_skipped(tmp_path):
    path = write_page(tmp_path, page_xml(region_points=""))
    page = cgpg.load_page(path, verify_image=False)
    assert page.regions == ()


def test_too_many_polygon_points_is_refused(tmp_path, monkeypatch):
    monkeypatch.setattr(cgpg, "MAX_POINTS_PER_POLYGON", 2)
    path = write_page(tmp_path, page_xml())
    with pytest.raises(cgpg.CgpgError, match="too many points"):
        cgpg.load_page(path, verify_image=False)


def test_a_malformed_polygon_point_is_refused(tmp_path):
    path = write_page(tmp_path, page_xml(line_points="12;12 188,12"))
    with pytest.raises(cgpg.CgpgError, match="malformed"):
        cgpg.load_page(path, verify_image=False)


def test_absurd_page_dimensions_are_refused(tmp_path):
    path = write_page(tmp_path, page_xml(width=999999, height=10))
    with pytest.raises(cgpg.CgpgError, match="out of range"):
        cgpg.load_page(path, verify_image=False)


def test_an_oversized_transcription_is_refused(tmp_path, monkeypatch):
    monkeypatch.setattr(cgpg, "MAX_XML_BYTES", 16)
    path = write_page(tmp_path, page_xml())
    with pytest.raises(cgpg.CgpgError, match="too large"):
        cgpg.load_page(path, verify_image=False)


def test_too_many_lines_is_refused(tmp_path, monkeypatch):
    monkeypatch.setattr(cgpg, "MAX_LINES_PER_PAGE", 0)
    path = write_page(tmp_path, page_xml())
    with pytest.raises(cgpg.CgpgError, match="too many lines"):
        cgpg.load_page(path, verify_image=False)


def test_a_page_image_that_disagrees_with_the_transcription_is_refused(tmp_path):
    pytest.importorskip("PIL")
    from PIL import Image

    Image.new("L", (32, 32), 255).save(tmp_path / "page.jpg")
    path = write_page(tmp_path, page_xml())
    with pytest.raises(cgpg.CgpgError, match="declares"):
        cgpg.load_page(path, verify_image=True)


def test_a_matching_page_image_is_accepted(tmp_path):
    pytest.importorskip("PIL")
    from PIL import Image

    Image.new("L", (200, 300), 255).save(tmp_path / "page.jpg")
    path = write_page(tmp_path, page_xml())
    assert cgpg.load_page(path, verify_image=True).width == 200


def test_a_tif_declared_but_shipped_as_jpg_is_accepted(tmp_path):
    # The corpus's own inconsistency: transcribed against TIFFs, published as
    # JPEGs. The dimension check is what still proves it is the right image.
    pytest.importorskip("PIL")
    from PIL import Image

    Image.new("L", (200, 300), 255).save(tmp_path / "page.jpg")
    path = write_page(tmp_path, page_xml(image="page.tif"))
    assert cgpg.load_page(path, verify_image=True).image_path.suffix == ".jpg"


def test_a_substituted_image_of_the_wrong_size_is_still_refused(tmp_path):
    pytest.importorskip("PIL")
    from PIL import Image

    Image.new("L", (64, 64), 255).save(tmp_path / "page.jpg")
    path = write_page(tmp_path, page_xml(image="page.tif"))
    with pytest.raises(cgpg.CgpgError, match="declares"):
        cgpg.load_page(path, verify_image=True)


def test_a_missing_page_image_is_refused(tmp_path):
    path = write_page(tmp_path, page_xml())
    with pytest.raises(cgpg.CgpgError, match="missing"):
        cgpg.load_page(path, verify_image=True)


def test_the_split_is_deterministic_and_independent_of_order():
    names = [f"PG{index}_col_{index}-{index + 1}" for index in range(400)]
    first = {name: cgpg.split_name(name) for name in names}
    second = {name: cgpg.split_name(name) for name in reversed(names)}
    assert first == second
    holdout = sum(1 for value in first.values() if value == "holdout")
    # Roughly a quarter, and neither split may be empty or swallow everything.
    assert 0 < holdout < len(names)
    assert 0.15 < holdout / len(names) < 0.35


def test_splitting_a_corpus_partitions_it_exactly(tmp_path):
    pages = []
    for index in range(12):
        path = write_page(tmp_path, page_xml(), name=f"page{index}")
        pages.append(cgpg.load_page(path, verify_image=False))
    dev, holdout = cgpg.split_corpus(pages)
    assert len(dev) + len(holdout) == len(pages)
    assert {page.name for page in dev}.isdisjoint({page.name for page in holdout})


def test_load_corpus_refuses_a_missing_directory(tmp_path):
    with pytest.raises(cgpg.CgpgError, match="corpus directory"):
        cgpg.load_corpus(tmp_path / "absent")
