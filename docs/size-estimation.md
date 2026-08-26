# Experimental output-size estimation

Milestone 5 adds a way to ask, before running a full conversion, "roughly
how big will the output PDF be?" This document explains what the
estimator actually measures, why it is built the way it is, and — just as
important — what it does not promise.

**The estimate is experimental.** It is a sampled, engineering-grade
approximation, not a guarantee. The converted file is always the
authoritative answer; the estimate exists to help a user decide DPI/method
settings before spending minutes or hours on a large scan.

## What it does

`estimate_output_size` (core), `mpdf-cli estimate` (CLI), and
the desktop app's "Estimate" button all do the same thing:

1. Pick a small, deterministic set of pages to sample (see below).
2. Render, binarize, and CCITT-encode *only those pages*, through
   [`analyze_one_page`](../crates/mpdf-core/src/pipeline.rs) —
   the exact same per-page function `process` and `analyze` use. There is
   no second, faster, approximate page-processing path. If the estimate
   is wrong, it is wrong for the same reasons a full conversion would be
   wrong, not because of a shortcut unique to estimation.
3. Measure each sampled page's compressed bytes per rendered pixel
   (`bytes_per_pixel = ccitt_bytes / (raster_width * raster_height)`).
4. Extrapolate a document-wide total from that per-pixel rate, using the
   *true* pixel count of every page in the document (not just the sampled
   ones — page geometry is known without rendering), plus a measured,
   deterministic correction for the PDF container structure every page
   costs regardless of its image content (see "Container overhead"
   below).

## Sampling policy

Sampling is deterministic — never random — so the same document and
sample count always sample the same pages and produce the same estimate.

- Default: 8 samples ([`estimation::DEFAULT_SAMPLE_COUNT`](../crates/mpdf-core/src/estimation.rs)).
- Evenly spaced across `[0, page_count - 1]` by rounded integer division;
  first and last page are included whenever the sample count allows it.
- A request larger than the document's page count quietly clamps to
  "sample every page" — not an error, since that's a reasonable thing to
  ask for on a short document. A request outside `[1, 32]`
  (`MIN_SAMPLE_COUNT`/`MAX_SAMPLE_COUNT`) is rejected before anything
  renders.

## Central estimate: mean, not median

The obvious per-sample statistic to extrapolate from is the median
bytes-per-pixel — it's outlier-resistant and was the first thing tried.
It turned out to be the wrong choice for this job.

The estimate needs to approximate a **sum** across every page
(`total ≈ representative_rate × page_count`). The mean has a direct
mathematical relationship to a sum; the median does not. On a document
with a mix of near-empty and heavily-inked pages — exactly the kind of
heterogeneity real scanned books have — the median of a skewed or
bimodal distribution can sit far from the value that actually reproduces
the total. This was not a theoretical concern: measuring the
median-based estimator against real PDFium output on synthetic
heterogeneous/homogeneous fixtures showed a consistent 40–46%
*underestimate*, well outside the acceptance threshold.

`estimated_output_bytes` therefore extrapolates from
`mean_bytes_per_pixel`. `median_bytes_per_pixel` is still reported, as a
useful outlier-resistant data point, but it no longer drives the central
estimate.

## Range: quartiles or min/max

The "likely range" (`estimated_lower_bytes`..`estimated_upper_bytes`) is
deliberately *not* called a confidence interval — it has no statistical
guarantee behind it, just the spread of what was actually observed in the
sample:

- With 4 or more samples, the range is the P25–P75 spread of sampled
  bytes-per-pixel, extrapolated the same way as the central estimate.
- With fewer than 4 samples, quartiles are not meaningful, so the range
  falls back to min/max.

Report text always uses phrases like "likely range" or "observed-sample
range," never "confidence interval."

## Container overhead

A CCITT-encoded page's compressed bytes are only part of what ends up in
the output file. Every page the writer emits also carries a page object,
an image XObject dictionary (`/Filter`, `/DecodeParms`, `/Width`,
`/Height`, ...), and a content stream; the document itself carries a
header, catalog, page-tree object, and trailer. For a page that compresses
to only a few dozen bytes — a mostly-blank page, for instance — this fixed
structure can dwarf the image data itself.

The estimator accounts for this with
[`pdf_writer::measure_container_overhead`](../crates/mpdf-core/src/pdf_writer.rs),
which builds two trivial one-pixel reference pages through the real
`BilevelPdfBuilder` — the same writer `process` uses — and reads the byte
deltas to get:

- a fixed, one-time document cost (header/catalog/page-tree/trailer), and
- a fixed per-page cost (page object + image dictionary + content
  stream).

This is a **direct measurement of the writer's own output shape**, not a
statistical fit to any corpus of documents: rerun it and you get the same
numbers, because the writer's structure doesn't change. It is added once
(document cost) and once per document page (per-page cost) on top of the
extrapolated image bytes. Before this correction was added, the estimator
underestimated real output by 35–46% on the synthetic accuracy fixtures,
entirely because of this uncounted structural overhead — switching from
median to mean alone did not fix it (see the commit history around
`estimation.rs`/`pdf_writer.rs` for the measurements that led here).

## Accuracy

These are engineering acceptance thresholds for the automated test suite,
not a promise about any particular real document:

| Fixture | Sample count | Threshold | Observed relative error | Status |
|---|---|---|---|---|
| 24-page heterogeneous synthetic (6 mixed page types) | 8 (default) | ≤ 25% | 3.7% (estimated 44,746 vs. actual 46,485 bytes) | Passing |
| 24-page homogeneous synthetic (uniform page type) | 8 (default) | ≤ 15% | 1.2% (estimated 37,308 vs. actual 37,757 bytes) | Passing |

These are single observed runs against the fixtures as committed
(`crates/mpdf-core/src/test_fixtures.rs`), Otsu/300 DPI —
not a statistical distribution across many runs, and the pass margin is
comfortably inside the threshold rather than right at the boundary. The
threshold, not the observed number, is the actual engineering
commitment; re-running
`cargo test --test pdf_pipeline estimate_accuracy -- --ignored --nocapture`
against a provisioned PDFium library reprints the current numbers.

Real scanned books are not synthetic fixtures. A document whose page
content or per-page compressibility varies far more than these fixtures —
extreme outlier pages, unusual scan artifacts, unusually large images —
can miss by more than these thresholds. When a completed conversion's
actual size differs meaningfully from its own prior estimate, that
comparison is reported back (`estimate_comparison` in the processing
report) rather than hidden, precisely so this kind of miss is visible
instead of silently assumed accurate.

## What the estimate is not

- **Not a confidence interval.** "Likely range" describes what was
  observed in a small sample, not a statistically derived bound.
- **Not a quality judgment.** A page flagged by outlier detection
  (`large_output`, `slow_processing`, `high_ink_ratio`, `low_ink_ratio`
  — see `docs/reporting.md`) is *different from the document's other
  pages* by a simple relative threshold. That is not the same claim as
  "bad scan" or "poor preservation," and the estimator makes no attempt
  at that judgment.
- **Not a recommendation engine.** The estimator reports measured
  numbers; it does not suggest "better" settings. That is out of scope
  for this milestone.
- **Not free.** Estimation renders and encodes real pages through the
  real pipeline — it is fast because it only touches a handful of pages,
  not because it takes a shortcut.

## Where it runs

Estimation is entirely local — the same process, same PDFium library, and
same settings parsing as `process`/`analyze`. Nothing about a document or
its estimate leaves the machine.
