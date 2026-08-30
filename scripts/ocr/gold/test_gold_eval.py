"""Tests for the OCR gold-evaluation harness and the offline sidecar.

These run without any OCR engine installed: they cover the metrics, the
corpus contract, the deterministic fixture renderer, and the sidecar's pure
functions (coordinate mapping, normalization, TSV parsing, profiles).
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import unicodedata
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import corpus  # noqa: E402
import fixtures  # noqa: E402
import metrics  # noqa: E402
import run_eval  # noqa: E402


def _sidecar():
    path = HERE.parent / "mpdf_ocr_sidecar.py"
    spec = importlib.util.spec_from_file_location("mpdf_ocr_sidecar", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# ---------------------------------------------------------------------------
# Metrics
# ---------------------------------------------------------------------------


def test_a_perfect_reading_scores_zero_error():
    result = metrics.score("ὀξέων ἄνθρωπος", "ὀξέων ἄνθρωπος")
    assert result.cer == 0.0
    assert result.wer == 0.0
    assert result.exact_word_rate == 1.0
    assert result.diacritic_error_rate == 0.0
    assert result.script_confusion_rate == 0.0
    assert result.nfc_violations == 0


def test_transliteration_is_caught_as_both_diacritic_loss_and_script_confusion():
    # The exact reported failure: ὀξέων -> ogeov.
    result = metrics.score("ὀξέων", "ogeov")
    assert result.diacritic_error_rate == 1.0
    assert result.script_confusions > 0
    assert result.exact_word_rate == 0.0


def test_dropping_only_the_accents_is_a_diacritic_error_not_a_script_confusion():
    # A monotonic-Greek model reading polytonic text: right letters, no marks.
    result = metrics.score("ῥητορικῆς", "ρητορικης")
    assert result.diacritic_errors > 0
    assert result.script_confusions == 0
    assert result.cer > 0.0


def test_german_umlaut_loss_is_measured():
    result = metrics.score("Über für", "Uber fir")
    assert result.diacritic_error_rate > 0.0
    assert result.exact_word_rate == 0.0


def test_nfd_output_is_reported_even_when_the_text_is_otherwise_correct():
    reference = "ὀξέων"
    hypothesis = unicodedata.normalize("NFD", reference)
    result = metrics.score(reference, hypothesis)
    assert result.nfc_violations == 1
    # Accuracy is still measured on the normalized forms.
    assert result.cer == 0.0


def test_metrics_aggregate_additively():
    first = metrics.score("abc", "abc")
    second = metrics.score("def", "xyz")
    total = metrics.aggregate([first, second])
    assert total.samples == 2
    assert total.reference_chars == first.reference_chars + second.reference_chars
    assert total.char_edits == first.char_edits + second.char_edits


def test_levenshtein_is_symmetric_and_zero_on_equality():
    assert metrics.levenshtein(list("kitten"), list("sitting")) == 3
    assert metrics.levenshtein(list("abc"), list("abc")) == 0
    assert metrics.levenshtein(list("abc"), []) == 3


# ---------------------------------------------------------------------------
# Corpus
# ---------------------------------------------------------------------------


def test_every_acceptance_codepoint_appears_in_the_greek_corpus():
    text = "\n".join(sample.text for sample in corpus.samples_for(("grc",)))
    for codepoint in ["ὀξέων", "ἄνθρωπος", "ῥ", "ᾶ", "ΐ", "ΰ", "ᾳ", "ῃ", "ῳ"]:
        assert codepoint in text, codepoint


def test_every_german_acceptance_codepoint_appears():
    text = "\n".join(sample.text for sample in corpus.samples_for(("deu",)))
    for codepoint in ["Ä", "Ö", "Ü", "ä", "ö", "ü", "ß", "für", "Über"]:
        assert codepoint in text, codepoint


def test_corpus_ground_truth_is_nfc():
    for sample in corpus.ALL_SAMPLES:
        assert metrics.is_nfc(sample.text), sample.sample_id


def test_sample_ids_are_unique():
    ids = [sample.sample_id for sample in corpus.ALL_SAMPLES]
    assert len(ids) == len(set(ids))


def test_the_corpus_covers_rotation_dpi_noise_and_columns():
    kinds = {
        "rotation": any(sample.rotation for sample in corpus.ALL_SAMPLES),
        "noise": any(sample.noise for sample in corpus.ALL_SAMPLES),
        "columns": any(sample.columns > 1 for sample in corpus.ALL_SAMPLES),
        "dpi": len({sample.dpi for sample in corpus.ALL_SAMPLES}) > 1,
        "toc": any(sample.kind == "toc" for sample in corpus.ALL_SAMPLES),
    }
    assert all(kinds.values()), kinds


def test_toc_samples_declare_their_rows():
    for sample in corpus.ALL_SAMPLES:
        if sample.kind == "toc":
            assert sample.toc_rows, sample.sample_id


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


def test_fixture_rendering_is_byte_deterministic(tmp_path):
    sample = corpus.by_id("grc-prose-heraclitus")
    first = fixtures.render_sample(sample, tmp_path / "a.png")
    second = fixtures.render_sample(sample, tmp_path / "b.png")
    assert fixtures.fixture_digest(first) == fixtures.fixture_digest(second)


def test_rotated_fixture_swaps_the_page_dimensions(tmp_path):
    from PIL import Image

    upright = corpus.by_id("deu-prose-umlauts")
    rotated = corpus.by_id("deu-prose-umlauts-rot90")
    with Image.open(fixtures.render_sample(upright, tmp_path / "u.png")) as image:
        upright_size = image.size
    with Image.open(fixtures.render_sample(rotated, tmp_path / "r.png")) as image:
        rotated_size = image.size
    assert rotated_size == (upright_size[1], upright_size[0])


def test_the_fixture_font_is_the_repository_ofl_font():
    assert fixtures.FONT_PATH.is_file()
    assert fixtures.FONT_LICENSE.is_file()
    assert "assets/fonts" in fixtures.FONT_PATH.as_posix()


# ---------------------------------------------------------------------------
# Sidecar: pure functions
# ---------------------------------------------------------------------------


def test_normalization_is_nfc_and_never_strips_marks():
    module = _sidecar()
    decomposed = unicodedata.normalize("NFD", "ὀξέων Über")
    result = module.normalize(decomposed)
    assert result == "ὀξέων Über"
    assert metrics.is_nfc(result)


def test_normalization_does_not_repair_or_transliterate():
    module = _sidecar()
    # Whatever the engine produced is what comes out, NFC-normalized only.
    assert module.normalize("ogeov") == "ogeov"


@pytest.mark.parametrize("clockwise", [90, 180, 270])
def test_unrotating_a_box_round_trips_through_the_page(clockwise):
    module = _sidecar()
    width, height = 1000, 600
    if clockwise in (90, 270):
        upright_width, upright_height = height, width
    else:
        upright_width, upright_height = width, height
    box = {"x": 10.0, "y": 20.0, "width": 30.0, "height": 40.0}
    mapped = module.unrotate_box(box, clockwise, width, height)
    # The mapped box must lie inside the original page.
    assert 0 <= mapped["x"] <= width
    assert 0 <= mapped["y"] <= height
    assert mapped["x"] + mapped["width"] <= width + 1e-6
    assert mapped["y"] + mapped["height"] <= height + 1e-6
    # A quarter turn swaps the box's own extents; a half turn preserves them.
    if clockwise in (90, 270):
        assert (mapped["width"], mapped["height"]) == (box["height"], box["width"])
    else:
        assert (mapped["width"], mapped["height"]) == (box["width"], box["height"])
    assert (upright_width, upright_height) != (0, 0)


def test_unrotating_by_zero_is_the_identity():
    module = _sidecar()
    box = {"x": 1.0, "y": 2.0, "width": 3.0, "height": 4.0}
    assert module.unrotate_box(box, 0, 100, 100) == box


def test_a_top_left_box_lands_where_a_clockwise_turn_puts_it():
    module = _sidecar()
    # Original page 1000x600. Rotating the *image* 90 degrees clockwise makes
    # the original top-left corner the upright top-right corner, so a box at
    # the upright top-right must map back to the original top-left.
    width, height = 1000, 600
    upright_box = {"x": 560.0, "y": 0.0, "width": 40.0, "height": 20.0}
    mapped = module.unrotate_box(upright_box, 90, width, height)
    assert mapped["x"] == pytest.approx(0.0, abs=1e-6)
    assert mapped["y"] == pytest.approx(0.0, abs=1e-6)


def test_language_profiles_separate_ancient_from_modern_greek():
    module = _sidecar()
    assert module.PROFILES["greek-ancient"]["tesseract"] == "grc"
    assert module.PROFILES["greek-modern"]["tesseract"] == "ell"
    assert module.PROFILES["greek-ancient"] != module.PROFILES["greek-modern"]


def test_the_default_profile_covers_greek_and_latin_together():
    module = _sidecar()
    profile = module.PROFILES[module.DEFAULT_PROFILE]
    assert "grc" in profile["tesseract"]
    assert "deu" in profile["tesseract"]
    assert set(profile["scripts"]) == {"Greek", "Latin"}


def test_script_violations_count_letters_outside_the_profile():
    module = _sidecar()
    # A Greek-only profile that emitted Cyrillic is reported, not rewritten.
    assert module.script_violations("δικαιοσύνη", ["Greek"]) == 0
    assert module.script_violations("привет", ["Greek"]) > 0


def test_tsv_parsing_keeps_the_engine_line_structure():
    module = _sidecar()
    header = "\t".join(
        "level page_num block_num par_num line_num word_num "
        "left top width height conf text".split()
    )
    rows = [
        header,
        "5\t1\t1\t1\t1\t1\t10\t20\t50\t12\t95.0\tEinleitung",
        "5\t1\t1\t1\t1\t2\t400\t20\t10\t12\t90.0\t2",
        "5\t1\t1\t1\t2\t1\t10\t40\t60\t12\t93.0\tAnhang",
    ]
    tree = module._parse_tsv("\n".join(rows))
    assert len(tree) == 1                      # one block
    assert len(tree[0]["lines"]) == 2          # two lines
    assert [word["text"] for word in tree[0]["lines"][0]] == ["Einleitung", "2"]
    assert [word["text"] for word in tree[0]["lines"][1]] == ["Anhang"]


def test_tsv_parsing_drops_only_negative_confidence_and_blank_words():
    module = _sidecar()
    header = "\t".join(
        "level page_num block_num par_num line_num word_num "
        "left top width height conf text".split()
    )
    rows = [
        header,
        "5\t1\t1\t1\t1\t1\t10\t20\t50\t12\t-1\t",
        "5\t1\t1\t1\t1\t2\t10\t20\t50\t12\t0\tzero",
        "4\t1\t1\t1\t1\t0\t0\t0\t0\t0\t-1\t",
    ]
    tree = module._parse_tsv("\n".join(rows))
    assert [word["text"] for word in tree[0]["lines"][0]] == ["zero"]


def test_confidences_are_rescaled_into_the_protocol_range():
    module = _sidecar()
    header = "\t".join(
        "level page_num block_num par_num line_num word_num "
        "left top width height conf text".split()
    )
    rows = [header, "5\t1\t1\t1\t1\t1\t10\t20\t50\t12\t95.0\tword"]
    tree = module._parse_tsv("\n".join(rows))
    assert 0.0 <= tree[0]["lines"][0][0]["confidence"] <= 1.0
    assert tree[0]["lines"][0][0]["confidence"] == pytest.approx(0.95)


# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------


def test_candidate_specs_parse_into_engine_profile_and_model_dir():
    candidate = run_eval.Candidate.parse("name:tesseract:auto:/models:paddle_lang=el")
    assert candidate.name == "name"
    assert candidate.engine == "tesseract"
    assert candidate.language_profile == "auto"
    assert candidate.model_dir == Path("/models")
    assert candidate.extra == {"paddle_lang": "el"}


def test_a_malformed_candidate_spec_is_refused():
    with pytest.raises(ValueError):
        run_eval.Candidate.parse("too:few")


def test_toc_scoring_separates_row_assembly_from_numeral_accuracy():
    sample = corpus.by_id("toc-roman-front-matter")
    # Every row came back as one line, but two numerals were misread.
    lines = [
        "Preface . . . . . . vii",
        "Acknowledgements . . . . . . XI",
        "Introduction . . . . . . 1",
        "Chapter One: The Method . . . . . . 23",
    ]
    result = run_eval.PageResult(
        sample_id=sample.sample_id, text="\n".join(lines), lines=lines,
        elapsed_seconds=0.0,
    )
    score = run_eval._score_toc(sample, result)
    assert score["rows_assembled"] == 4
    assert score["rows_exact"] == 3     # "XI" != "xi" as printed


def test_a_row_whose_number_landed_on_another_line_is_not_assembled():
    sample = corpus.by_id("toc-single-column")
    # The defect this whole round is about: title and number on separate lines.
    lines = ["Einleitung", "1", "1. Die ἀρετή bei Aristoteles", "17"]
    result = run_eval.PageResult(
        sample_id=sample.sample_id, text="\n".join(lines), lines=lines,
        elapsed_seconds=0.0,
    )
    score = run_eval._score_toc(sample, result)
    assert score["rows_assembled"] < score["rows_expected"]


# ---------------------------------------------------------------------------
# Block-level script routing
# ---------------------------------------------------------------------------


def _word(text, left=0, top=0, width=40, height=30, confidence=0.80):
    return {
        "text": text, "left": left, "top": top, "width": width,
        "height": height, "confidence": confidence, "word_num": 0,
    }


def _greek_line(top, words=4):
    return [_word("ἐκεῖνο", left=50 * index, top=top) for index in range(words)]


def _latin_line(top, words=4):
    return [_word("Kausalität", left=50 * index, top=top) for index in range(words)]


def test_script_counts_separate_greek_latin_and_other():
    module = _sidecar()
    greek, latin, other = module._script_counts("ἐκεῖνο Kausalität 123")
    assert greek == 6
    assert latin == 10
    assert other == 0


def test_a_greek_run_and_a_latin_run_are_split_not_merged():
    module = _sidecar()
    lines = [_latin_line(0), _greek_line(50), _greek_line(100), _latin_line(150)]
    runs = module._script_runs(lines)
    assert [(leans, first, last) for leans, first, last in runs] == [
        (False, 0, 1), (True, 1, 3), (False, 3, 4),
    ]


def test_a_latin_running_head_is_never_greek_dominant():
    module = _sidecar()
    greek, latin, other = module._script_counts("I. Metaphysische Kausalität")
    assert not module._is_greek_dominant(greek, latin, other)


def test_a_short_greek_fragment_is_left_to_the_combined_pass():
    module = _sidecar()
    # Ratio alone would reroute this; the character floor is what stops it.
    greek, latin, other = module._script_counts("τὸ")
    assert greek < module.ROUTING_MIN_GREEK_CHARS
    assert not module._is_greek_dominant(greek, latin, other)


def test_a_solidly_greek_run_is_routed():
    module = _sidecar()
    greek, latin, other = module._script_counts("ἐκεῖνο ἀνηρτημένου νοῦς φύσει")
    assert module._is_greek_dominant(greek, latin, other)


def test_a_candidate_may_not_shrink_or_balloon_the_region():
    module = _sidecar()
    crop = (0, 0, 1000, 1000)
    original = [_greek_line(0), _greek_line(50)]
    collapsed = [[_word("ἐ", top=0)]]
    assert module._accept_replacement(original, collapsed, crop) == "coverage-collapsed"
    ballooned = [_greek_line(0, words=40)]
    wide = (0, 0, 4000, 1000)
    assert module._accept_replacement(original, ballooned, wide) == "coverage-expanded"


def test_a_candidate_outside_the_crop_is_rejected():
    module = _sidecar()
    original = [_greek_line(0)]
    stray = [[_word("ἐκεῖνο", left=5000, top=5000)]]
    assert module._accept_replacement(original, stray, (0, 0, 100, 100)) == (
        "bbox-outside-crop"
    )


def test_a_candidate_that_lost_the_greek_is_rejected():
    module = _sidecar()
    original = [_greek_line(0)]
    latinised = [[_word("ekeino", top=0), _word("ekeino", top=0),
                  _word("ekeino", top=0), _word("ekeino", top=0)]]
    assert module._accept_replacement(original, latinised, (0, 0, 1000, 1000)) == (
        "greek-lost"
    )


def test_an_empty_candidate_is_rejected():
    module = _sidecar()
    assert module._accept_replacement([_greek_line(0)], [], (0, 0, 100, 100)) == "empty"


def test_neighbouring_lines_pulled_in_by_crop_padding_are_dropped():
    module = _sidecar()
    # The crop is padded so tall Greek accents survive, which lets the Greek
    # pass also read the line above and below. Keeping those would duplicate
    # text the combined pass still holds.
    segment = [_greek_line(100), _greek_line(150)]
    candidate = [_greek_line(20), _greek_line(100), _greek_line(150), _greek_line(230)]
    kept = module._restrict_to_span(candidate, segment)
    assert len(kept) == 2
    assert [module._line_centre(line) for line in kept] == [115.0, 165.0]


def test_routing_counters_and_thresholds_are_reported():
    module = _sidecar()
    assert module.ROUTING_MODE == "block-script-v1"
    assert module.ROUTING_THRESHOLDS_VERSION
    # A threshold change without a version bump would let evidence produced
    # under different calibrations be compared as though it matched.
    assert 0.0 < module.ROUTING_GREEK_RATIO <= 1.0
    assert module.ROUTING_MIN_GREEK_CHARS >= 1


# ---------------------------------------------------------------------------
# Routing safety: Latin must survive
# ---------------------------------------------------------------------------


def _latin_line(top, words=4, text="Kausalität", confidence=0.80):
    return [
        _word(text, left=50 * index, top=top, confidence=confidence)
        for index in range(words)
    ]


def _confident_latin_line(top, words=4, text="Kausalität"):
    """A Latin line the combined pass read well - the case worth preserving."""
    return _latin_line(top, words=words, text=text, confidence=0.97)


def test_the_latin_retention_floor_is_reachable_from_the_routing_ceiling():
    module = _sidecar()
    # The defect this pins: the floor used to trigger only above the ceiling
    # that selection already enforces, so no routable run could ever reach it.
    assert (
        module.ROUTING_SUBSTANTIAL_LATIN_CHARS < module.ROUTING_MAX_LATIN_CHARS
    ), "a retention floor at or above the routing ceiling can never fire"


def test_a_candidate_that_destroys_substantial_latin_is_rejected():
    module = _sidecar()
    # A run that is Greek enough to route but carries real Latin words.
    original = [_greek_line(0, words=14), _latin_line(50, words=2)]
    greek, latin, other = module._script_counts(module._text_of(original))
    assert module._is_greek_dominant(greek, latin, other), "must be routable"
    assert latin >= module.ROUTING_SUBSTANTIAL_LATIN_CHARS

    # The Greek-only pass returns Greek where the Latin was, at the same
    # length, so coverage passes and only the Latin loss can reject it.
    latinless = [_greek_line(0, words=14), _greek_line(50, words=3)]
    assert module._accept_replacement(
        original, latinless, (0, 0, 4000, 4000)
    ) == "latin-lost"


def test_retention_tolerates_a_small_latin_loss_but_not_a_majority():
    module = _sidecar()
    original = [_latin_line(0, words=3)]          # 30 Latin letters
    _, latin_before, _ = module._script_counts(module._text_of(original))
    # Both candidates keep the character count, so coverage cannot be what
    # rejects them; only the Latin retention differs.
    keeps_most = [_latin_line(0, words=2) + _greek_line(0, words=2)[:2]]
    loses_most = [_greek_line(0, words=5)]        # 0 of 30 kept
    crop = (0, 0, 4000, 4000)
    assert latin_before >= module.ROUTING_SUBSTANTIAL_LATIN_CHARS
    assert module._accept_replacement(original, keeps_most, crop) != "latin-lost"
    assert module._accept_replacement(original, loses_most, crop) == "latin-lost"


def test_a_mixed_line_inside_a_greek_region_keeps_its_combined_reading():
    module = _sidecar()
    # A German running head sitting inside a Greek column.
    segment = [_confident_latin_line(0, words=2), _greek_line(50), _greek_line(100)]
    candidate = [_greek_line(0, words=2), _greek_line(50), _greek_line(100)]
    merged, preserved = module._merge_by_geometry(segment, candidate)
    assert preserved == 1
    assert module._text_of([merged[0]]) == module._text_of([segment[0]])
    # ... while the genuinely Greek lines are taken from the Greek pass.
    assert merged[1] is candidate[1]
    assert merged[2] is candidate[2]


@pytest.mark.parametrize(
    "text,keeps",
    [
        ("Kausalität", True),      # a German word: real Latin
        ("Vgl", False),            # three letters: below the floor
        ("ab", False),             # a stray pair
    ],
)
def test_only_lines_with_substantial_latin_are_preserved(text, keeps):
    module = _sidecar()
    segment = [[_word(text, top=0, confidence=0.97)], _greek_line(50)]
    candidate = [_greek_line(0, words=1), _greek_line(50)]
    merged, preserved = module._merge_by_geometry(segment, candidate)
    assert (preserved == 1) is keeps


def test_a_page_number_and_a_reference_survive_a_greek_region():
    module = _sidecar()
    reference = [_word("Doxographi", top=0, confidence=0.97),
                 _word("Diels", top=0, confidence=0.97)]
    segment = [reference, _greek_line(50), _greek_line(100)]
    candidate = [_greek_line(0, words=2), _greek_line(50), _greek_line(100)]
    merged, preserved = module._merge_by_geometry(segment, candidate)
    assert preserved == 1
    assert "Doxographi" in module._text_of([merged[0]])


# ---------------------------------------------------------------------------
# Routing safety: the time budget is a real budget
# ---------------------------------------------------------------------------


def test_the_engine_timeout_is_explicit_and_overridable(monkeypatch):
    module = _sidecar()
    seen = {}

    class Completed:
        returncode = 0
        stdout = b"level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\t" \
                 b"left\ttop\twidth\theight\tconf\ttext\n"

    def fake_run(argv, **kwargs):
        seen["timeout"] = kwargs.get("timeout")
        return Completed()

    monkeypatch.setattr(module.subprocess, "run", fake_run)
    module._run_tesseract("tesseract", Path("x.png"), Path("m"), "grc", 6, timeout=3.5)
    assert seen["timeout"] == 3.5
    module._run_tesseract("tesseract", Path("x.png"), Path("m"), "grc", 6)
    assert seen["timeout"] == module.ENGINE_TIMEOUT_SECONDS


def test_a_rerouted_region_never_outlives_the_routing_budget(monkeypatch):
    module = _sidecar()
    # No real sleeping: the clock is faked and only the ceiling is asserted.
    clock = {"now": 0.0}
    monkeypatch.setattr(module.time, "monotonic", lambda: clock["now"])
    timeouts = []

    def fake_run(binary, image, model_dir, languages, psm, timeout=None):
        timeouts.append(timeout)
        clock["now"] += 20.0        # each call burns 20s of the budget
        raise RuntimeError("recognition engine failed")

    monkeypatch.setattr(module, "_run_tesseract", fake_run)

    class FakeImage:
        size = (4000, 4000)
        def convert(self, _mode): return self
        def crop(self, _box): return self
        def save(self, *_a, **_k): pass
        def __enter__(self): return self
        def __exit__(self, *_a): return False

    import PIL.Image
    monkeypatch.setattr(PIL.Image, "open", lambda *_a, **_k: FakeImage())

    # Several separate Greek regions, split by Latin lines, so the router has
    # more work available than the budget can pay for.
    lines = []
    for index in range(8):
        lines.append(_greek_line(index * 200, words=14))
        lines.append(_latin_line(index * 200 + 100, words=3))
    tree = [{"bbox": {"x": 0.0, "y": 0.0, "width": 3000.0, "height": 3000.0},
             "lines": lines}]
    _, decisions = module.route_greek_blocks(
        "tesseract", Path("p.png"), Path("m"), tree, 6, 4000, 4000,
        module._Budget(module.ROUTING_MAX_SEGMENTS,
                       module.ROUTING_TIME_BUDGET_SECONDS),
    )
    # Each launched subprocess was capped by what actually remained, never by
    # the flat engine ceiling, and never exceeded the routing budget.
    assert timeouts, "the regions must have been routable"
    for value in timeouts:
        assert 0 < value <= module.ROUTING_TIME_BUDGET_SECONDS
        assert value <= module.ENGINE_TIMEOUT_SECONDS
    # 45s of budget spent 20s at a time: three calls, with a shrinking cap.
    assert timeouts == [45.0, 25.0, 5.0]
    # Once the budget is gone, no further subprocess is started.
    assert any(d["reason"] == "time-budget-exhausted" for d in decisions)


def test_a_timed_out_greek_pass_keeps_the_combined_reading(monkeypatch):
    module = _sidecar()

    def fake_run(*_a, **_k):
        raise module.subprocess.TimeoutExpired(cmd="tesseract", timeout=1)

    monkeypatch.setattr(module, "_run_tesseract", fake_run)

    class FakeImage:
        size = (4000, 4000)
        def convert(self, _mode): return self
        def crop(self, _box): return self
        def save(self, *_a, **_k): pass
        def __enter__(self): return self
        def __exit__(self, *_a): return False

    import PIL.Image
    monkeypatch.setattr(PIL.Image, "open", lambda *_a, **_k: FakeImage())

    lines = [_greek_line(0, words=6), _greek_line(60, words=6)]
    tree = [{"bbox": {"x": 0.0, "y": 0.0, "width": 3000.0, "height": 300.0},
             "lines": lines}]
    routed, decisions = module.route_greek_blocks(
        "tesseract", Path("p.png"), Path("m"), tree, 6, 4000, 4000,
        module._Budget(module.ROUTING_MAX_SEGMENTS,
                       module.ROUTING_TIME_BUDGET_SECONDS),
    )
    assert routed[0]["lines"] == lines, "a timeout must not lose text"
    assert any(d["reason"] == "greek-pass-timed-out" for d in decisions)


def test_a_region_the_combined_pass_read_confidently_is_left_alone(monkeypatch):
    module = _sidecar()
    # The measured lesson: rerouting only helped where the combined pass was
    # unsure. Where it was already confident (0.95-0.96 on Greek quotations
    # inside a German page) rerouting made the page worse.
    called = []
    monkeypatch.setattr(
        module, "_run_tesseract",
        lambda *a, **k: called.append(1) or (_ for _ in ()).throw(RuntimeError("x")),
    )

    class FakeImage:
        size = (4000, 4000)
        def convert(self, _mode): return self
        def crop(self, _box): return self
        def save(self, *_a, **_k): pass
        def __enter__(self): return self
        def __exit__(self, *_a): return False

    import PIL.Image
    monkeypatch.setattr(PIL.Image, "open", lambda *_a, **_k: FakeImage())

    confident = [
        [_word("ἐκεῖνο", left=50 * i, top=0, confidence=0.96) for i in range(14)]
    ]
    tree = [{"bbox": {"x": 0.0, "y": 0.0, "width": 3000.0, "height": 100.0},
             "lines": confident}]
    routed, decisions = module.route_greek_blocks(
        "tesseract", Path("p.png"), Path("m"), tree, 6, 4000, 4000,
        module._Budget(module.ROUTING_MAX_SEGMENTS,
                       module.ROUTING_TIME_BUDGET_SECONDS),
    )
    assert not called, "a confidently-read region must not be re-recognized"
    assert routed[0]["lines"] == confident
    assert any(d["reason"] == "combined-already-confident" for d in decisions)


def test_a_low_confidence_greek_region_is_still_routed(monkeypatch):
    module = _sidecar()
    called = []
    monkeypatch.setattr(
        module, "_run_tesseract",
        lambda *a, **k: called.append(1) or (_ for _ in ()).throw(RuntimeError("x")),
    )

    class FakeImage:
        size = (4000, 4000)
        def convert(self, _mode): return self
        def crop(self, _box): return self
        def save(self, *_a, **_k): pass
        def __enter__(self): return self
        def __exit__(self, *_a): return False

    import PIL.Image
    monkeypatch.setattr(PIL.Image, "open", lambda *_a, **_k: FakeImage())

    unsure = [
        [_word("ἐκεῖνο", left=50 * i, top=0, confidence=0.72) for i in range(14)]
    ]
    tree = [{"bbox": {"x": 0.0, "y": 0.0, "width": 3000.0, "height": 100.0},
             "lines": unsure}]
    module.route_greek_blocks(
        "tesseract", Path("p.png"), Path("m"), tree, 6, 4000, 4000,
        module._Budget(module.ROUTING_MAX_SEGMENTS,
                       module.ROUTING_TIME_BUDGET_SECONDS),
    )
    assert called, "an unsure region is exactly what routing exists for"


# ---------------------------------------------------------------------------
# argv contract: every sidecar must parse what the Rust runner actually builds
# ---------------------------------------------------------------------------


def _runner_argv(script: Path, model_dir: Path, image: Path) -> list:
    """The exact argument list `SidecarOcrProvider::recognize` constructs.

    Kept in one place so a change on the Rust side that this file does not
    mirror shows up as a failing contract test rather than as a sidecar
    exiting 2 and being reported as "provider unavailable".
    """
    return [
        sys.executable, str(script),
        "--protocol", "mpdf-ocr",
        "--protocol-version", "0.1",
        "--model-dir", str(model_dir),
        "--input", str(image),
        "--engine", "tesseract",
        "--language-profile", "auto",
        "--routing", "off",
        "--small-type-latin", "off",
        "--detached-greek-accents", "off",
    ]


@pytest.mark.parametrize("script_name", ["mpdf_ocr_sidecar.py", "rapidocr_sidecar.py"])
def test_every_sidecar_parses_the_runners_argv(tmp_path, script_name):
    """No sidecar may reject the runner's argv with an argparse usage error.

    This starts the real process with the real argument list. A sidecar that
    does not know an option exits 2 before reading stdin, and the runner
    surfaces that as an unavailable provider -- a silent break of a CLI path
    that is still reachable.
    """
    script = HERE.parent / script_name
    model_dir = tmp_path / "models"
    model_dir.mkdir()
    image = tmp_path / "page.png"
    image.write_bytes(b"")

    completed = subprocess.run(
        _runner_argv(script, model_dir, image),
        input="", capture_output=True, text=True, timeout=120, check=False,
    )
    assert completed.returncode != 2, (
        f"{script_name} rejected the runner's argv: {completed.stderr.strip()[:300]}"
    )
    assert "unrecognized arguments" not in completed.stderr
    assert "invalid choice" not in completed.stderr


def test_the_contract_test_would_catch_an_unknown_option(tmp_path):
    """The test above is only meaningful if argparse really exits 2 here."""
    script = HERE.parent / "rapidocr_sidecar.py"
    model_dir = tmp_path / "models"
    model_dir.mkdir()
    image = tmp_path / "page.png"
    image.write_bytes(b"")
    argv = _runner_argv(script, model_dir, image) + ["--not-a-real-option", "x"]
    completed = subprocess.run(
        argv, input="", capture_output=True, text=True, timeout=120, check=False
    )
    assert completed.returncode == 2
    assert "unrecognized arguments" in completed.stderr


# ---------------------------------------------------------------------------
# Detached Greek accent bands
# ---------------------------------------------------------------------------


def _band(top, height, left=100, width=900, confidence=0.2, text="~ \\ / 3 \\"):
    """A short, uncertain, mark-only line - the shape of an accent band."""
    return [_word(text, left=left, top=top, width=width, height=height,
                  confidence=confidence)]


def _greek_base(top, height=74, left=100, width=1000, words=8):
    return [
        _word("ἐκεῖνο", left=left + index * 120, top=top,
              width=width // words, height=height, confidence=0.6)
        for index in range(words)
    ]


def test_a_detached_band_over_a_greek_line_is_detected():
    module = _sidecar()
    band = _band(top=100, height=20)
    assert module._is_detached_band(band, 74.0)


@pytest.mark.parametrize(
    "name,line",
    [
        ("legitimate standalone punctuation", [_word(".", top=100, height=20,
                                                     confidence=0.95)]),
        ("a page number", [_word("135", top=100, height=70, confidence=0.9)]),
        ("apparatus siglum", [_word("D.", top=100, height=70, confidence=0.9)]),
        ("a real Greek line", None),
    ],
)
def test_legitimate_short_content_is_not_treated_as_a_band(name, line):
    module = _sidecar()
    if line is None:
        line = _greek_base(top=100)
    assert not module._is_detached_band(line, 74.0), name


def test_a_confident_short_line_is_never_a_band():
    module = _sidecar()
    # Confidence is part of the signal: the engine is not unsure about any
    # part of this line.
    assert not module._is_detached_band(
        _band(top=100, height=20, confidence=0.95), 74.0
    )


def test_a_band_over_a_latin_line_has_no_greek_base():
    module = _sidecar()
    band = _band(top=100, height=20)
    latin = [_word("Kausalität", left=100 + i * 120, top=110, width=120,
                   height=74, confidence=0.9) for i in range(8)]
    assert module._base_line_for(band, [band, latin], 74.0) is None


def test_a_band_at_the_top_of_the_page_with_no_base_line_is_left_alone():
    module = _sidecar()
    band = _band(top=5, height=20)
    assert module._base_line_for(band, [band], 74.0) is None


def test_a_band_pairs_with_the_greek_line_it_sits_on():
    module = _sidecar()
    band = _band(top=100, height=20)
    base = _greek_base(top=98)
    other = _greek_base(top=400)
    lines = [band, base, other]
    assert module._base_line_for(band, lines, 74.0) == 1


def test_a_band_does_not_reach_across_to_a_distant_line():
    module = _sidecar()
    band = _band(top=100, height=20)
    far = _greek_base(top=900)
    assert module._base_line_for(band, [band, far], 74.0) is None


def test_a_horizontally_disjoint_band_is_not_paired():
    module = _sidecar()
    band = _band(top=100, height=20, left=3000, width=200)
    base = _greek_base(top=98, left=100, width=1000)
    assert module._base_line_for(band, [band, base], 74.0) is None


@pytest.mark.parametrize(
    "reason,candidate",
    [
        # Same letter count, Greek replaced by Latin: isolates the Greek
        # retention gate from the coverage gate.
        ("greek-lost", [[_word("ekeinos", left=i * 120, top=100, height=70,
                               confidence=0.8) for i in range(8)]]),
        ("coverage-collapsed", [[_word("ἐ", top=100, height=70, confidence=0.8)]]),
        ("empty", []),
    ],
)
def test_a_detached_candidate_that_loses_content_is_rejected(reason, candidate):
    module = _sidecar()
    base = _greek_base(top=100)
    assert module._accept_detached(base, candidate, (0, 0, 4000, 4000)) == reason


def test_a_detached_candidate_may_not_drop_latin_or_digits():
    module = _sidecar()
    base = [_word("ἐκεῖνο", top=100, height=70, confidence=0.7),
            _word("Vgl", top=100, left=200, height=70, confidence=0.7),
            _word("704", top=100, left=400, height=70, confidence=0.7)]
    without_digits = [[_word("ἐκεῖνο", top=100, height=70, confidence=0.7),
                       _word("Vgl", top=100, left=200, height=70, confidence=0.7)]]
    assert module._accept_detached(base, without_digits, (0, 0, 4000, 4000)) in (
        "digits-lost", "coverage-collapsed",
    )


def test_a_detached_candidate_outside_the_crop_is_rejected():
    module = _sidecar()
    base = _greek_base(top=100)
    stray = [[_word("ἐκεῖνο", left=9000, top=9000, height=70, confidence=0.8)]]
    assert module._accept_detached(base, stray, (0, 0, 500, 500)) == "bbox-outside-crop"


def test_the_detached_pass_shares_the_one_budget(monkeypatch):
    module = _sidecar()
    clock = {"now": 0.0}
    monkeypatch.setattr(module.time, "monotonic", lambda: clock["now"])
    calls = []

    def fake_run(binary, image, model_dir, languages, psm, timeout=None):
        calls.append((languages, psm, timeout))
        clock["now"] += 20.0
        raise RuntimeError("engine failed")

    monkeypatch.setattr(module, "_run_tesseract", fake_run)

    class FakeImage:
        size = (4000, 4000)
        def convert(self, _m): return self
        def crop(self, _b): return self
        def resize(self, *_a, **_k): return self
        def save(self, *_a, **_k): pass
        def __enter__(self): return self
        def __exit__(self, *_a): return False

    import PIL.Image
    monkeypatch.setattr(PIL.Image, "open", lambda *_a, **_k: FakeImage())

    lines = []
    for i in range(8):
        lines.append(_band(top=i * 200, height=20))
        lines.append(_greek_base(top=i * 200 - 2))
    tree = [{"bbox": {"x": 0.0, "y": 0.0, "width": 3000.0, "height": 3000.0},
             "lines": lines}]
    budget = module._Budget(module.ROUTING_MAX_SEGMENTS,
                            module.ROUTING_TIME_BUDGET_SECONDS)
    routed, decisions = module.route_detached_greek_accents(
        "tesseract", Path("p.png"), Path("m"), tree, 4000, 4000, budget
    )
    assert calls, "bands must have been detected"
    for _, psm, timeout in calls:
        assert psm == module.DETACHED_CROP_PSM
        assert 0 < timeout <= module.ROUTING_TIME_BUDGET_SECONDS
    assert any(d["reason"] == "time-budget-exhausted" for d in decisions)
    # A failure keeps every line the combined pass produced.
    assert routed[0]["lines"] == lines


def test_a_failed_detached_pass_keeps_the_combined_lines(monkeypatch):
    module = _sidecar()
    monkeypatch.setattr(
        module, "_run_tesseract",
        lambda *a, **k: (_ for _ in ()).throw(module.subprocess.TimeoutExpired("t", 1)),
    )

    class FakeImage:
        size = (4000, 4000)
        def convert(self, _m): return self
        def crop(self, _b): return self
        def resize(self, *_a, **_k): return self
        def save(self, *_a, **_k): pass
        def __enter__(self): return self
        def __exit__(self, *_a): return False

    import PIL.Image
    monkeypatch.setattr(PIL.Image, "open", lambda *_a, **_k: FakeImage())

    lines = [_band(top=100, height=20), _greek_base(top=98)]
    tree = [{"bbox": {"x": 0.0, "y": 0.0, "width": 3000.0, "height": 300.0},
             "lines": lines}]
    routed, decisions = module.route_detached_greek_accents(
        "tesseract", Path("p.png"), Path("m"), tree, 4000, 4000,
        module._Budget(12, 45),
    )
    assert routed[0]["lines"] == lines, "a timeout must never lose text"
    assert any(d["reason"] == "greek-pass-timed-out" for d in decisions)


def test_a_rejected_candidate_suppresses_only_the_band_not_the_base_line(monkeypatch):
    module = _sidecar()
    base = _greek_base(top=98)
    band = _band(top=100, height=20)

    def fake_run(*_a, **_k):
        # A candidate that destroys the Greek: must never replace the line.
        header = "\t".join(
            "level page_num block_num par_num line_num word_num "
            "left top width height conf text".split()
        )
        return "\n".join([header,
                           "5\t1\t1\t1\t1\t1\t0\t0\t50\t60\t80.0\tabcdefgh"])

    monkeypatch.setattr(module, "_run_tesseract", fake_run)

    class FakeImage:
        size = (4000, 4000)
        def convert(self, _m): return self
        def crop(self, _b): return self
        def resize(self, *_a, **_k): return self
        def save(self, *_a, **_k): pass
        def __enter__(self): return self
        def __exit__(self, *_a): return False

    import PIL.Image
    monkeypatch.setattr(PIL.Image, "open", lambda *_a, **_k: FakeImage())

    tree = [{"bbox": {"x": 0.0, "y": 0.0, "width": 3000.0, "height": 300.0},
             "lines": [band, base]}]
    routed, decisions = module.route_detached_greek_accents(
        "tesseract", Path("p.png"), Path("m"), tree, 4000, 4000,
        module._Budget(12, 45),
    )
    # The base line survives untouched; only the mark-only band is dropped.
    assert routed[0]["lines"] == [base]
    assert any(d["reason"].startswith("suppressed:") for d in decisions)


def test_the_audit_record_carries_no_candidate_text(monkeypatch):
    module = _sidecar()
    monkeypatch.setattr(
        module, "_run_tesseract",
        lambda *a, **k: (_ for _ in ()).throw(RuntimeError("x")),
    )

    class FakeImage:
        size = (4000, 4000)
        def convert(self, _m): return self
        def crop(self, _b): return self
        def resize(self, *_a, **_k): return self
        def save(self, *_a, **_k): pass
        def __enter__(self): return self
        def __exit__(self, *_a): return False

    import PIL.Image
    monkeypatch.setattr(PIL.Image, "open", lambda *_a, **_k: FakeImage())

    lines = [_band(top=100, height=20), _greek_base(top=98)]
    tree = [{"bbox": {"x": 0.0, "y": 0.0, "width": 3000.0, "height": 300.0},
             "lines": lines}]
    _, decisions = module.route_detached_greek_accents(
        "tesseract", Path("p.png"), Path("m"), tree, 4000, 4000,
        module._Budget(12, 45),
    )
    blob = json.dumps(decisions, ensure_ascii=False)
    assert "ἐκεῖνο" not in blob
    assert not [c for c in blob if 0x370 <= ord(c) <= 0x3FF]
