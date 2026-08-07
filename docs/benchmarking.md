# Benchmarking

**Status: planned.** No benchmarking framework or benchmark data exists in
this repository yet. This document describes the intended reporting
framework so that future benchmark work has an agreed structure from the
start, and so no quality or preservation claim is made without it.

**Not to be confused with `analyze` (Milestone 3, implemented).** The
`analyze` command (see [`cli.md`](cli.md) and [`reporting.md`](reporting.md))
reports real per-page measurements — grayscale statistics, the actual
threshold selected, ink pixel ratios, processing time, CCITT byte size —
from the real pipeline. It is a diagnostic and scripting tool for choosing
settings and finding difficult pages, not the benchmark described below: it
has no ground truth, computes none of the fidelity metrics in this
document (F-measure, PSNR, DRD, ...), and its numbers are not evidence of
preservation quality on their own. A low `black_pixel_ratio` or small
output size from `analyze` is not a quality claim.

## Purpose

Museion Binarize aims to make quantitative, reproducible claims about output
quality instead of relying on visual inspection of a handful of pages. The
benchmarking framework (planned for Milestone 6, see
[`roadmap.md`](roadmap.md)) is what will let the project say, with evidence,
how a given thresholding method and parameter set performs — including,
eventually, on material like polytonic Ancient Greek and critical
apparatuses, which is the focus of the Phase 2 research direction.

## Planned metrics

- **F-measure.** Harmonic mean of precision and recall of foreground
  (ink) pixel classification against a pixel-accurate ground truth.
- **Pseudo-F-measure.** A variant of F-measure that weights errors near
  character skeletons/contours more heavily, commonly used in document
  binarization competitions (e.g. DIBCO) where a small positional
  error near a stroke edge is less severe than one in open background.
- **PSNR (Peak Signal-to-Noise Ratio).** Standard image-fidelity metric
  between the binarized output and ground truth, included for
  comparability with prior binarization literature.
- **DRD (Distance Reciprocal Distortion).** A perceptually motivated
  metric for bilevel image quality that accounts for the visual impact of
  errors based on their distance from the nearest edge, widely used
  alongside F-measure in binarization benchmarks.
- **Processing time.** Wall-clock time per page and per document, measured
  on documented reference hardware, for each thresholding method.
- **Peak memory.** Peak resident memory during processing of a reference
  document, to validate the bounded-memory design goal in
  [`architecture.md`](architecture.md).
- **Compressed bytes per megapixel.** Output file size normalized by page
  pixel count, to make compression efficiency comparable across page sizes
  and DPI settings.
- **OCR CER (Character Error Rate) — optional, downstream only.** Phase 1
  does not perform OCR. CER against a third-party OCR engine run
  separately on the output may optionally be reported as a downstream
  quality signal, but it is not a target Museion Binarize optimizes for,
  and it must always be reported as a downstream measurement, not a claim
  about the tool's own OCR capability (which does not exist).

## Ground truth and datasets

- Benchmark datasets **must not be committed to this repository without
  license review.** Scanned book pages are frequently under copyright even
  when very old editions are in the public domain, and critical editions in
  particular may carry their own separate copyright on the apparatus and
  editorial matter.
- Preferred sources for benchmark material are: pages explicitly in the
  public domain with documented provenance, pages the contributor holds
  rights to and explicitly licenses for this purpose, or synthetically
  generated test pages (see [`test-data/README.md`](../test-data/README.md)).
- Any dataset that is committed must include a note on its source, license,
  and how ground truth was produced (e.g. manually annotated, derived from
  a known-clean digital edition, or synthetically generated).

## Reporting

Benchmark results will be produced by running the CLI (or
`museion-binarize-core` directly) against a fixed, documented dataset and
parameter set, and recording the metrics above in a structured, versioned
format alongside the tool version and hardware/OS used. Until this framework
exists, no specific numeric performance or preservation claim should appear
in project documentation, release notes, or marketing material.
