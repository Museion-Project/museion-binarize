# synthetic-document-v1

A deterministic, procedurally generated benchmark suite for
document-binarization fidelity — see [`docs/benchmark-metrics.md`](../../../docs/benchmark-metrics.md)
and [`docs/benchmark-datasets.md`](../../../docs/benchmark-datasets.md)
for the full framework this dataset feeds into.

## What this is

- **Dataset ID**: `synthetic-document-v1`
- **License**: CC0-1.0 (public domain dedication)
- **Provenance**: every pixel is drawn procedurally by
  `crates/mpdf-core/src/benchmark_fixtures.rs`, via
  `cargo run -p mpdf-core --example gen_benchmark_fixtures --
  test-data/benchmark/synthetic-v1`. No font, scan, screenshot, or
  third-party image was used anywhere in this dataset.
- **Ground truth**: the exact procedural shape drawn *before* any
  degradation is applied. It is stored losslessly (strictly binary PNG,
  `0` = black/foreground, `255` = white/background) and is never derived
  by running M PDF Processor's own binarization pipeline — doing so
  would make the benchmark circular (see `docs/benchmark-metrics.md`).
- **Dimensions**: every page is 160x200 pixels.

## Categories (12 pages)

| Page id | What it stresses |
|---|---|
| `clean_text` | Baseline: well-separated procedural glyph strokes, no degradation. |
| `faint_text` | Same glyphs as `clean_text`, ink faded 65% toward the paper color. |
| `small_text` | Much smaller/thinner strokes, tightly packed. |
| `uneven_background` | Left-to-right background gradient (180→250) behind clean glyphs. |
| `salt_pepper_noise` | 3% of pixels flipped to pure black or white (deterministic PRNG, fixed seed). |
| `blur` | Clean glyphs through a radius-1 box blur. |
| `low_contrast` | Clean glyphs with contrast compressed 65% toward mid-gray. |
| `broken_strokes` | Ground truth itself has small gaps punched into alternating glyphs — the *source* content is broken, not a later degradation of clean ground truth. |
| `thick_strokes` | Wider glyph strokes than `clean_text`. |
| `mixed_text_and_lines` | Clean glyph rows plus long thin horizontal rule lines. |
| `dense_apparatus_density` | Many small, tightly packed marks and isolated punctuation-like dots; every third row is tagged as an `apparatus` ROI. Purpose is scale/density stress, not philological content. |
| `synthetic_diacritic_detail` | Base glyph strokes each paired with one of: an accent-like diagonal stroke, a breathing-like arc, an iota-subscript-like dot, or a stacked accent+breathing combination — each tagged as a `diacritic` ROI so small-mark recall can be scored separately from whole-page F1. These are **polytonic-diacritic-*like* stress shapes**, not a claim of typographic representativeness of real Greek. |

## Known limitations

- **Not a representative corpus of historical Greek editions, or of any
  real scanned book.** This dataset validates the benchmark framework
  and measures defined synthetic stress cases; it is not evidence for a
  broad claim about preservation quality on real scholarly material. See
  `docs/limitations.md`.
- Pages are small (160x200) by design — large enough to exercise every
  metric meaningfully, small enough to keep the repository modest. This
  is not a claim about performance at real page/DPI scale.
- `synthetic_diacritic_detail`'s marks are geometric stand-ins (short
  strokes, arcs, dots), not actual polytonic Greek diacritics rendered
  from a real font — see `docs/benchmark-datasets.md` for why (font
  redistribution/licensing, and reproducible rendering across machines
  without relying on a system font).

## Regenerating

```bash
cargo run -p mpdf-core --example gen_benchmark_fixtures -- /tmp/regen
```

Rerunning against the same generator version reproduces every PNG in
this directory byte-for-byte; this is verified automatically by
`benchmark_fixture_suite_regenerates_byte_identical_pngs` in
`crates/mpdf-core/tests/benchmark_suite.rs`.
