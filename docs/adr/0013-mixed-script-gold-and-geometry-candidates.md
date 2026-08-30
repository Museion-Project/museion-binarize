# ADR 0013: Own mixed-script gold before selecting geometry

Status: partially superseded by ADR 0014 for geometry selection

This decision refines the gold and provider-selection consequences of
[ADR 0012](0012-deterministic-geometry-and-gemini-transcription.md).

## Decision

The production OCR composition is:

```text
original page raster
  -> deterministic GeometryProvider
       -> immutable line_id + bbox + reading_order
  -> Gemini 3.7 Flash transcription
       -> text keyed by every supplied line_id exactly once
  -> strict compositor
       -> coordinate-bearing OcrPage -> hidden PDF text layer
```

Geometry and transcription are now evaluated independently. ADR 0014 selects a
provisional GeometryProvider from a text-free clean-native control; it does not
wait for mixed-script transcription gold and it explicitly remains unvalidated
on historical material. Tesseract, Orli/Kraken, PaddleOCR detection plus a
deterministic orderer, and Apple Vision remain replaceable implementations of
the same contract.

## Gold policy

CGPG is a Greek transcription stress test, not the project closed-world gold.
GT4HistComment is a useful Greek/German/English/Latin transcription control,
and BHL-IMPACT is a useful English/French/German/Latin layout control. Neither
covers the target contract on the same pages.

The project therefore creates a human-verified sample of real scholarly pages
containing German, French, English, and polytonic Greek. Every selected page
must exhaustively annotate all visible text lines, original-image pixel boxes,
reading order, exact NFC transcription, language, and content class.

## Selection gate

After the gold manifest is frozen, candidates run on identical images and are
compared on line detection/alignment, reading order, determinism, latency,
package/runtime size, Mac compatibility, and redistribution license. Gemini
transcription quality is evaluated separately with gold geometry so geometry
errors cannot be misattributed to transcription.

This gate remains applicable to transcription claims. Geometry promotion now
uses the separate historical-scan holdout defined by ADR 0014.
