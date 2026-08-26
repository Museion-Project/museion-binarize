# Algorithms

**Status: implemented and wired into the end-to-end PDF pipeline.** Every
algorithm described below is implemented and unit-tested in
`mpdf-core`, and as of Milestone 2 they run on real rasterized
PDF pages through a single orchestrator (`image_pipeline.rs`), reachable
from the CLI. The desktop UI is not connected yet.

The orchestrator applies stages in exactly this order: validate settings →
grayscale → contrast → background normalization → median denoise →
binarization → despeckle → bilevel packing.

## Thresholding methods

Phase 1 supports exactly three deterministic thresholding methods. All three
convert a grayscale page image into a bilevel (1-bit) image by classifying
each pixel as foreground (ink) or background (page).

### Otsu

A global, automatic threshold computed from the page's grayscale histogram,
maximizing between-class variance between foreground and background pixel
populations. Implemented in-house with 64-bit accumulators: an earlier
version delegated to `imageproc`, whose implementation accumulates in
`u32` and panics with an arithmetic overflow on a full page at 600 DPI
(roughly 35 megapixels). Panicking on user-supplied documents is not
acceptable, so the algorithm is now this project's own. Fast and parameter-free, but a single global threshold can
perform poorly on pages with uneven illumination or bleed-through from the
reverse side — common in scans of older books.

### Sauvola

A local, adaptive threshold computed per pixel from the mean and standard
deviation of a surrounding window, parameterized by window size and a
sensitivity constant `k`. Generally more robust than Otsu on scans with
uneven lighting or degraded paper, at higher computational cost and with
more parameters that affect output.

Implemented in-house (not via a third-party CV library) using summed-area
tables (integral images) for the sum and sum-of-squares of pixel values, so
the local mean and standard deviation for every pixel's window are computed
in amortized constant time rather than re-scanning the window per pixel.
The optimized implementation is cross-checked in tests against a
brute-force reference implementation that recomputes each window directly.

### Manual

A single, user-specified global threshold value, with no automatic
adaptation. Useful when a user has already determined (e.g. by inspection or
prior experience with a particular scan batch) a threshold that works well,
or when reproducibility of a specific documented threshold matters more than
adaptivity.

## Conservative preprocessing

Some preprocessing before thresholding (e.g. mild denoising) can improve
results, but preprocessing is also where information is most easily and
irreversibly destroyed — particularly the fine strokes, breathing marks, and
accents of polytonic Ancient Greek, and the small type often used in
critical apparatuses. Phase 1's design intent is:

- Preprocessing steps must be individually documented, deterministic, and
  optional (able to be disabled).
- No preprocessing step should be enabled by default if it has not been
  evaluated against the benchmark described in
  [`benchmarking.md`](benchmarking.md).
- Preprocessing parameters must be recorded alongside output so that a run
  can be reproduced exactly.

## Despeckling risks

Despeckling (removing small isolated dark regions, presumed to be scanner
noise) is a common step in scan-cleanup tools, but it is also one of the
most likely steps to delete real content that happens to be small: diacritic
marks, punctuation, apparatus symbols, and fine serifs. For this reason:

- Despeckling is not assumed to be part of the default Phase 1 pipeline.
- If offered, it must be optional, clearly labeled with its risk to small
  typographic detail, and evaluated against benchmark data (including,
  eventually, Ancient Greek text) before being recommended as a default.

## CCITT Group 4

CCITT Group 4 (ITU-T T.6) is a lossless, run-length-based compression scheme
designed specifically for bilevel (1-bit) images, and is the standard
encoding for compact scanned-document PDFs (`/Filter /CCITTFaxDecode`).
Because it is lossless with respect to the bilevel raster it is given,
CCITT Group 4 itself introduces no additional information loss beyond
whatever the thresholding step already committed to. This is why Phase 1
treats thresholding quality — not the compression step — as the primary
lever for output fidelity.

Implemented via the pure-Rust `fax` crate (MIT-licensed, part of the
`pdf-rs` project), wrapped in a project-owned interface (`src/ccitt.rs`) so
callers never depend on `fax` directly. Round-trip (encode-then-decode)
correctness is covered by tests, including odd image widths not divisible
by eight and fully white/black/random-sparse pages.

## What is intentionally out of scope for Phase 1 algorithms

- No OCR or text recognition of any kind runs as part of thresholding.
- No AI or machine-learned models are used to choose thresholds or classify
  pixels.
- No generative inpainting or restoration is applied to damaged or missing
  content.
