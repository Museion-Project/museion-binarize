# Benchmark metrics

This document defines every fidelity metric the benchmark framework
(`crates/mpdf-core/src/benchmark/metrics.rs`) computes: the
exact formula, the polarity/alignment assumptions, edge-case behavior,
and — where the metric has a canonical external definition — its
source. See [`benchmark-datasets.md`](benchmark-datasets.md) for dataset
manifests and [`benchmark-running.md`](benchmark-running.md) for how to
actually run a benchmark.

## Not the same thing as `analyze`

`analyze` (Milestone 3) reports real measurements from the production
pipeline — grayscale statistics, the chosen threshold, ink ratio, timing
— but has **no ground truth** and computes **none** of the metrics
below. A low `black_pixel_ratio` or small file size from `analyze` is
not a fidelity or quality claim. The benchmark framework exists
specifically because that gap needed a real answer.

## Benchmark levels

- **Level A — raster benchmark** (`BenchmarkLevel::Raster`, implemented
  in this milestone). `degraded input raster -> M PDF image pipeline
  -> binary output`, compared directly against ground truth. This never
  touches PDFium: [`image_pipeline::process_rendered_page`](../crates/mpdf-core/src/image_pipeline.rs)
  — the exact function `process`/`analyze`/`estimate` already use — runs
  directly on the input PNG. This is the **primary** fidelity benchmark:
  it isolates binarization-pipeline quality from PDF-rendering behavior,
  so a result cannot be confused for a PDFium artifact or vice versa.
- **Level B — end-to-end PDF benchmark** (`BenchmarkLevel::Pdf`,
  **not implemented in this milestone**). `PDF -> PDFium render ->
  M PDF pipeline -> binary output`, would measure the full
  application path and would be PDFium/platform-sensitive.
  **Deferred, not silently dropped**: this milestone's own priority
  ("correct and tested over more features" — see the milestone
  specification) applies equally to a second benchmark level as it does
  to a metric like pseudo-F below. Level A already gives the framework a
  real, PDFium-independent fidelity benchmark that hosted CI can run;
  adding a second, PDFium-dependent execution path with its own
  PDF-fixture-generation, alignment, and geometry-normalization concerns
  is real, separate scope that deserves its own dedicated
  implementation and test pass rather than a rushed addition here. The
  report schema's `environment.benchmark_level` field already
  distinguishes the two levels so a Level B implementation can be added
  later without a schema break.

**Never conflate the two.** A report's `environment.benchmark_level`
field says which one produced it; nothing in this framework computes
"one fidelity number" across both.

## Polarity

Unchanged from the rest of the crate: `true` = black / foreground / ink,
`false` = white / background (see
[`bilevel.rs`](../crates/mpdf-core/src/bilevel.rs)). Every
metric function is defined against this convention:

```
TP = output black AND ground-truth black
FP = output black AND ground-truth white
FN = output white AND ground-truth black
TN = output white AND ground-truth white
```

Ground-truth PNGs are loaded and normalized to this convention by
[`mask_io.rs`](../crates/mpdf-core/src/benchmark/mask_io.rs)
*before* any metric function sees them — a file's own pixel-value
convention (`0`/`255`, or reversed via `ground_truth_polarity` in the
dataset manifest) never leaks into the metric math itself.

## Alignment

Every metric requires `output` and `ground_truth` to have **identical**
`width`/`height`. There is no cropping, resampling, or best-effort
alignment — a mismatch is a structured `BenchmarkDimensionMismatch`
error, never silently approximated, because an approximated alignment
would invalidate the very fidelity number it claims to produce.

## Confusion matrix

`u64` counts (`true_positive`, `false_positive`, `false_negative`,
`true_negative`) with checked accumulation, so a pathologically large
benchmark image cannot silently overflow into a wrong answer.

## Precision, recall, F1

```
precision = TP / (TP + FP)
recall    = TP / (TP + FN)
F1        = 2 * precision * recall / (precision + recall)
```

**Edge-case policy** (applied consistently, never `NaN`):

| Case | Precision | Recall | F1 |
|---|---|---|---|
| Normal (`TP+FP>0`, `TP+FN>0`) | `TP/(TP+FP)` | `TP/(TP+FN)` | harmonic mean |
| Output empty, GT empty | `1.0` (vacuously correct) | `1.0` (nothing to find) | `1.0` |
| Output empty, GT has foreground | `0.0` | `0.0` | `0.0` |
| Output has foreground, GT empty | `0.0` | `1.0` (nothing to find) | `0.0` |

"Output empty" means the output raster has zero black pixels;
"GT empty" means the ground truth does. See
`crates/mpdf-core/src/benchmark/metrics.rs`'s
`f_measure` doc comment for the reasoning behind each branch, and its
tests for one worked example per row of the table above, each computed
by hand — not by calling the production function to generate its own
expected value.

### Macro vs. micro F1

Two valid ways to aggregate F1 across pages exist and are **both**
reported, never conflated under one bare `f1`:

- **Macro F1**: the mean of each page's own F1.
- **Micro F1**: sum confusion counts across every page first, then
  compute one F1 from the total.

These can differ substantially when page sizes vary — a page-size-
insensitive summary (macro) versus a pixel-volume-weighted one (micro).

## PSNR

Each pixel is treated as a normalized binary sample (`black = 1.0`,
`white = 0.0`, so `MAX_I = 1`):

```
MSE  = mismatched_pixels / total_pixels
PSNR = 10 * log10(1 / MSE)
```

A perfect match gives `MSE = 0`, which is mathematically infinite PSNR
— not representable in JSON (`serde_json` refuses to serialize
`Infinity`, by design; see `docs/reporting.md`). This is represented as
a tagged result rather than an arbitrary clamp:

```json
{"perfect_match": true}
```

versus, for a real (non-perfect) page:

```json
{"psnr_db": 17.97}
```

`AggregateMetrics.mean_psnr_db` is the mean over **non-perfect** pages
only (perfect-match pages have no finite dB value to average in);
`perfect_psnr_page_count` reports how many pages were excluded that way,
so a reader is never misled into thinking every page contributed to the
mean.

## DRD (Distance Reciprocal Distortion)

Source: Lu, H., Kot, A.C., Shi, Y.Q., "Distance-Reciprocal Distortion
Measure for Binary Document Images," *IEEE Signal Processing Letters*
11(2), 2004 — the definition used throughout the DIBCO document-
binarization-competition literature.

For every pixel where `output` disagrees with `ground_truth` (a
"flipped" pixel at `(x, y)`), DRD measures how much the surrounding 5x5
neighborhood of ground truth disagrees with the *output's own value* at
`(x, y)`, weighted by a distance-reciprocal weight matrix:

```
WM(i, j) = 1 / distance((i, j), center)   for every non-center cell
WM normalized so sum(WM) = 1; the center cell itself is excluded.

DRD_k = sum over the 5x5 neighborhood of |GT_neighbor - output_value| * WM(i, j)
DRD   = sum(DRD_k for every flipped pixel k) / NUBN
```

`NUBN` (Number of Uniform (i.e. non-uniform, in the original paper's
naming) Blocks) is the count of non-overlapping 8x8 blocks of the
ground truth that contain **both** black and white pixels. This
normalizes for how much of the page has actual content: a mostly-blank
page has few non-uniform blocks, so the same handful of errors there
counts for more per block than on a densely inked page.

**Boundary convention** (documented, not the only one in the
literature): a neighborhood pixel outside the image is treated as
background (white / `0`). If `NUBN == 0` (e.g. a fully blank ground
truth) and there were no flipped pixels, `DRD = 0.0`; if there *were*
flipped pixels against a ground truth with no measurable block-content
scale, the raw (undivided) distortion sum is returned rather than
dividing by zero.

Verified against three hand-checkable micro-fixtures (not by calling
the implementation to generate its own expected value):

1. A perfect match: `DRD = 0`.
2. A single isolated flipped pixel whose entire 5x5 neighborhood is
   otherwise blank: contributes `0` distortion (every weighted term is
   `|white - white| = 0`).
3. A flipped pixel inside a solid black column, where the expected
   per-neighbor contribution is computed independently in the test
   itself from the weight matrix and the known column geometry, then
   compared to the function's output.

See `drd_weight_matrix_is_normalized_and_center_excluded`,
`drd_single_isolated_flip_in_one_non_uniform_block_matches_hand_calculation`,
and `drd_flip_that_disagrees_with_dense_neighborhood_is_positive_and_matches_hand_calculation`
in `metrics.rs` for the exact worked cases.

## Pseudo-F-measure: deferred

**Not implemented in this milestone.** Pseudo-F-measure (a DIBCO-style
variant of F-measure that weights errors near character skeletons/
contours more heavily than errors in open background) requires
correctly reproducing skeleton/contour-weighting semantics from a
trustworthy reference. At the time of this milestone, no reference
implementation or worked test example was identified with enough
confidence to build hand-checkable micro-fixtures the way DRD's
definition allowed above — and the milestone's own stated priority is
explicit: *"correct + tested F1/PSNR/DRD is better than
F1/PSNR/DRD/pseudo-F/... with uncertain formulas."* Rather than ship a
plausible-looking formula that might not match the literature's actual
pseudo-F, `pseudo_f_measure` is left absent from the report entirely
(no field, not a null placeholder) until it can be implemented against
a verified specification and tested the same rigorous way DRD was. This
is a documented, deliberate scope decision, not an oversight.

## Compressed bytes per megapixel

```
bytes_per_megapixel = ccitt_bytes / (pixel_count / 1_000_000)
```

`1 MB = 1,000,000 pixels` here (not `1,048,576`). `ccitt_bytes` is the
raw CCITT Group 4 payload for that one page — deliberately **not**
including the PDF container overhead described in
[`size-estimation.md`](size-estimation.md), so this metric isolates
bilevel-compression efficiency from PDF structural overhead. Do not
compare it directly to a whole converted PDF's file size.

## Timing

`processing_duration_us` sums the same per-stage timings
`process`/`analyze` already measure (grayscale/contrast/preprocessing,
binarization, cleanup, CCITT encoding — see
[`timing.rs`](../crates/mpdf-core/src/timing.rs)).
`render_duration_us` is `null` (not `0`) at the raster level, since
there is no PDFium render stage to measure — `null` and `0` are
different claims, and this framework never conflates "not applicable"
with "measured zero cost."

### Performance measurement policy

Timing numbers are only comparable when the environment producing them
is documented (OS, architecture, release vs. debug build) — see
`docs/benchmark-results/synthetic-v1.md`'s "Environment" section for the
pattern to follow. Do not compare a debug-build timing to a
`--release` one, or timings from different machines, as if they were
hardware-independent. Content-fidelity fields (F1, PSNR, DRD, confusion
counts, compressed bytes) are deterministic and *are* safely comparable
across machines, since they depend only on the pipeline's arithmetic.

**Always use `--release` for timing comparisons.** Debug-build timings
must never be published as performance evidence.

## Peak memory

Not implemented as an observed, OS-level measurement in this milestone
— that is inherently platform/runtime-specific, and faking it from the
sum of buffers this crate happens to allocate would risk being read as
real resident-memory evidence when it is not. No `peak_memory` field
exists in the current report; a future milestone could add either a
modelled working-set figure (explicitly labeled as such, never called
"peak resident memory") or genuine OS-level RSS collection behind a
platform-specific runner hook, but neither is claimed here.

## OCR CER

Not implemented. M PDF Processor performs no OCR of its own (see
`docs/limitations.md`); OCR CER, if ever added, would be an explicitly
labeled downstream/external measurement (running a third-party OCR
engine on the *output*), never a claim about this project's own
capability.

## Quality limitation vs. correctness bug

A benchmark result that shows one method scoring worse than another is
**not automatically a bug**. Distinguish:

- **Correctness bug**: something is objectively wrong regardless of
  content — inverted polarity, a systematic geometry error, cleanup
  removing content it should not. These would be fixed with a
  regression test, same as any other core defect.
- **Quality limitation**: a real, expected difference in how an
  algorithm performs on certain content — e.g. a fixed manual threshold
  failing on faded ink that an adaptive method recovers correctly (see
  `docs/benchmark-results/synthetic-v1.md`'s `manual-300-128` /
  `faint_text` result). This is exactly the kind of thing the benchmark
  exists to surface, not something to "fix" by retuning the algorithm
  mid-benchmark and presenting only the post-tuning picture.

This milestone made no algorithm changes in response to any benchmark
result — see the results document for the actual first-run numbers.
