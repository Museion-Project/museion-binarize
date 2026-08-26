# Benchmarking

**Status: implemented (Milestone 6).** A reproducible, versioned
ground-truth benchmarking framework exists — see
[`benchmark-metrics.md`](benchmark-metrics.md) for metric definitions,
[`benchmark-datasets.md`](benchmark-datasets.md) for the dataset/profile
manifest formats, [`benchmark-running.md`](benchmark-running.md) for how
to run one, and [`benchmark-results/synthetic-v1.md`](benchmark-results/synthetic-v1.md)
for the first recorded results.

**This is framework evidence, not a broad preservation claim.** The
committed `synthetic-document-v1` dataset validates the framework and
measures defined synthetic stress cases. It is explicitly **not** a
representative corpus of real scanned or printed documents, and it is
**not** evidence for a claim about preservation quality on historical
polytonic Greek editions — see `docs/limitations.md` and
`docs/benchmark-datasets.md`, "Real scholarly corpus plan," for what
would actually be needed to support that broader claim.

**Not to be confused with `analyze` (Milestone 3, implemented).** The
`analyze` command (see [`cli.md`](cli.md) and [`reporting.md`](reporting.md))
reports real per-page measurements — grayscale statistics, the actual
threshold selected, ink pixel ratios, processing time, CCITT byte size —
from the real pipeline. It is a diagnostic and scripting tool for choosing
settings and finding difficult pages, not the benchmark: it has no ground
truth and computes none of the fidelity metrics below. A low
`black_pixel_ratio` or small file size from `analyze` is not a quality
claim on its own.

## What exists

- **Metrics** (`crates/mpdf-core/src/benchmark/metrics.rs`):
  a foreground confusion matrix, precision/recall/F1 (with an explicit,
  documented, tested edge-case policy — never `NaN`), PSNR (perfect
  match represented as a tagged result, never `Infinity`), and DRD
  (Distance Reciprocal Distortion, per Lu/Kot/Shi 2004, the definition
  used throughout the DIBCO binarization-competition literature). See
  [`benchmark-metrics.md`](benchmark-metrics.md) for exact formulas,
  sources, and edge cases, and for **pseudo-F-measure's deliberate
  deferral** (not implemented — no trustworthy reference/test oracle was
  identified with enough confidence in this milestone; see that
  document for the full rationale).
- **Two benchmark levels**, kept explicitly distinct in every report:
  Level A (raster — the primary, implemented benchmark, no PDFium
  involved) and Level B (end-to-end PDF via PDFium — not implemented in
  this milestone, deferred for the same "correct and tested over more
  features" reason as pseudo-F).
- **Versioned manifests**: `mpdf-benchmark-dataset` and
  `mpdf-benchmark-profile`, both schema `1.0`, with
  dataset-root path containment (rejects traversal and symlink escape),
  resource limits on untrusted manifest input, and required
  license/provenance/ground-truth-method fields.
- **Region-of-interest (ROI) support**, so small critical detail (a
  diacritic, a punctuation mark) can be scored separately from a whole
  page's F1 — see [`benchmark-datasets.md`](benchmark-datasets.md), "Why
  ROIs matter."
- **A committed, deterministic synthetic fixture suite**
  (`test-data/benchmark/synthetic-v1/`, CC0-1.0, 12 procedurally
  generated categories including polytonic-diacritic-*like* and dense-
  apparatus-*like* stress shapes — geometric stand-ins, not real
  rendered Greek text; see that directory's own README for exactly why).
- **CLI**: `mpdf benchmark run`/`benchmark validate`.
- **Reproducibility digests** (SHA-256 of dataset/profile manifests and
  referenced files) recorded in every report.

## Reporting

Benchmark results are produced by running `mpdf benchmark
run` against a dataset and profile manifest, and are recorded in the
versioned `mpdf-benchmark` JSON schema (see
[`reporting.md`](reporting.md)) alongside the tool version and
environment used. See
[`benchmark-results/synthetic-v1.md`](benchmark-results/synthetic-v1.md)
for the first such recording and its explicit interpretation caveats.
