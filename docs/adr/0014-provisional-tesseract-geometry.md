# ADR 0014: Provisional Tesseract geometry provider

Status: superseded for provider selection by [ADR 0015](0015-selected-surya-geometry.md); retained as historical integration evidence

## Context

ADR 0012 split complete OCR into deterministic geometry and geometry-bound
Gemini 3.7 Flash transcription. ADR 0013 deferred geometry selection until a
mixed-script transcription gold set existed. That coupled two different
questions: line geometry does not require correct transcription or any
particular language mix.

The supplied 327-page born-digital Gerson volume has native text objects and
bookmarks. It is useful as a clean geometry control after rendering, but it is
not human closed-world gold and it is not evidence about historical scans.

## Decision

Use Tesseract 5.5.3 with PSM 3 and the pinned `tessdata_best` 4.1.0 model set as
the provisional `GeometryProvider`. Discard all Tesseract text at the geometry
boundary. Preserve only line rectangles, deterministic reading order,
confidence, provider identity, and the raster/config digest. Gemini 3.7 Flash
returns transcription keyed to the immutable line ids; it cannot return or
change coordinates.

Every geometry page and composed OCR provenance record carries
`geometry_validation_status=historical_material_not_validated`. The status is
also part of the geometry digest and pipeline fingerprint. Wiring or smoke
testing cannot promote it.

The selected sidecar mode is PSM 3, not the historical all-text OCR default
PSM 6. Multi-column index pages were part of the control, and the evaluated
candidate configuration must match the configuration that is shipped.

## Evidence

Eight pages at 300 DPI supplied 421 native PDF line boxes. Each candidate was
run twice on identical images. The frozen primary metric was line F1, with box
IoU, reading order, and latency as tie-breakers.

| Candidate | F1 | Mean IoU | Reading-order τ | Median s/page | Exact repeat |
|---|---:|---:|---:|---:|---|
| Tesseract 5.5.3 PSM 3 | **0.9721** | 0.8147 | **0.9291** | 4.898 | yes |
| PP-OCRv5 server detector | 0.9689 | **0.8320** | 0.8333 | 3.350 | yes |
| Apple Vision text rectangles | 0.9514 | 0.8100 | 0.8915 | **0.168** | yes |

Paddle produced slightly tighter boxes, but its frozen deterministic orderer
interleaved the two-column index pages (τ 0.526 and 0.490). Apple Vision was
fastest but over-segmented more lines. Tesseract won the frozen primary metric
and the reading-order comparison.

The machine-readable result is
[`geometry-provider-clean-native-2026-08-30.json`](../evidence/geometry-provider-clean-native-2026-08-30.json).

## Consequences and promotion gate

This decision is sufficient to connect and test the pipeline, not to declare a
production-quality historical OCR engine. Promotion requires a separate,
frozen, exhaustive line-geometry holdout built from representative historical
scans. It must cover skew, bleed-through, damaged or uneven type, marginalia,
small apparatus, columns, ornaments, and blank/non-text regions. Provider
selection is reopened if Tesseract fails that holdout; Paddle remains the
nearest accuracy control and Apple Vision the macOS latency/control path.
