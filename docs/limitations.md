# Limitations

## Current state (as of the `0.1.0-rc.2` release candidate)

M6 does not ship a vendor integration or make paid calls. The reusable client
speaks the provider-neutral `mpdf-api` 0.1 contract and CI validates it with a
deterministic loopback HTTP fixture. It uses the platform-native
Keychain/Credential Manager/Secret Service; an unavailable or locked store
fails visibly and never falls back to a plaintext token. Endpoint discovery,
OAuth, telemetry, cloud bookmark generation, and automatic upload remain out
of scope.

### Automatic table of contents (bookmarks v2)

Automatic bookmarks depend on the document's own evidence: **either a valid
native PDF outline, or a complete OCR run containing a recognizable printed
contents list.** There is no claim that an arbitrary PDF yields a correct
table of contents, and there is no mode in which a model reads the book and
composes one.

Known boundaries of this feature:

- A document with no printed contents list is a **safe refusal**: nothing is
  written to a PDF. Heading-like lines may be proposed for human review, but
  large or bold text alone is never confirmed automatically.
- A partial OCR run is refused rather than padded with guesses; the automatic
  contents mode needs evidence for every page.
- The scan window for contents pages is the front matter only
  (`min(pages, min(40, max(8, ceil(pages × 15%))))`). A contents list printed
  only at the back of the book is not found.
- Native-text pages (born-digital, no OCR) have approximate line and word
  boxes. They are usable as text evidence but not as strong column or
  font-size evidence, so a two-column contents list on such a page is read as
  a single column and its entries are marked for review.
- Column handling covers one and two columns; three or more are not modelled.
- The thresholds shipped here are a deliberately conservative frozen
  baseline, **not calibrated against a real annotated corpus**. Expect
  entries that a person would accept to arrive as `needs_review`. Loosening
  them requires a new rule version, which invalidates prior automatic
  decisions rather than silently reinterpreting them.
- Accuracy metrics for this feature have **not** been measured against an
  external human-gold corpus in this change; the evaluation entry point
  (`scripts/bookmarks/auto_bookmark_eval.py`) reports `not_run`/`pending`
  when the corpus or its annotations are absent, and CI only verifies the
  metric formulas against synthetic data.

The desktop does not retain PDF passwords. Consequently, it rejects remote
OCR for a password-protected open session before upload; the CLI can still be
used with its existing environment-only password input when appropriate.

M PDF Processor can perform a complete local PDF conversion, can analyze
a PDF without converting it, can produce an experimental sampled
estimate of a conversion's output size before running it, and can
benchmark binarization fidelity against pixel-accurate ground truth:

```
input.pdf -> PDFium rasterization -> image-processing core
          -> true bilevel image -> CCITT Group 4
          -> rebuilt 1-bit output.pdf -> reopened and validated   [process]

input.pdf -> PDFium rasterization -> image-processing core
          -> per-page/document measurements -> JSON report        [analyze]

input.pdf -> deterministic page sample -> real pipeline on the sample
          -> bytes-per-pixel extrapolation + container overhead
          -> experimental size estimate                           [estimate]

degraded raster + ground truth -> real image-processing core
          -> confusion matrix / F1 / PSNR / DRD -> versioned report [benchmark]
```

**Implemented:**

- the deterministic image-processing algorithms (Otsu, Sauvola, manual
  thresholding, conservative preprocessing, despeckle cleanup);
- PDF input, page inspection, and rasterization at 300 / 400 / 600 DPI;
- bilevel PDF reconstruction as true 1-bit `/CCITTFaxDecode` image
  XObjects (see [`pdf-output.md`](pdf-output.md));
- a persistent, single-open-per-operation PDFium document session (see
  [`pdf-pipeline-session.md`](pdf-pipeline-session.md)) — `inspect`,
  `analyze`, `process`, and `preview` each open the source exactly once,
  not once per page;
- a full CLI: `info`, `inspect`, `analyze`, `process`, `preview`, each with
  human-readable and versioned `--json` output (see
  [`cli.md`](cli.md) and [`reporting.md`](reporting.md));
- `analyze`: real rendering and binarization measurements (grayscale
  statistics, the actual threshold selected, ink ratios, per-stage
  timing, optional CCITT size) without writing an output PDF;
- `estimate`: an **experimental** sampled output-size estimate — real
  rendering/binarization/CCITT-encoding of a small, deterministic page
  sample, extrapolated to the whole document; richer per-page and
  aggregate metrics and simple document-relative outlier flags on
  `process`'s own report; see [`size-estimation.md`](size-estimation.md)
  for the full methodology and its accuracy thresholds;
- documented, tested exit codes and a stdout/stderr contract that keeps
  `--json` output free of progress text or prose;
- cancellation, safe temporary files with atomic persistence, and output
  validation that reopens and renders the finished file;
- **the desktop GUI**: native file selection and single-PDF drag-and-drop,
  a persistent per-window
  document session, lazily-loaded page thumbnails, before/after preview
  through the real pipeline, settings and deterministic presets,
  asynchronous processing with progress events and real cancellation,
  an experimental pre-conversion size estimate, and structured
  error/completion presentation (see [`desktop.md`](desktop.md)). Covered
  by automated tests and by native macOS acceptance testing against the
  real running application — see [`desktop-testing.md`](desktop-testing.md)
  for the full record, including the one observed real-world processing
  baseline (not a performance guarantee).
- **a reproducible, ground-truth binarization-fidelity benchmark
  framework** (Milestone 6): confusion matrix / precision / recall / F1
  / PSNR / DRD against pixel-accurate ground truth, versioned
  dataset/profile manifests with path containment, region-of-interest
  metrics, and a `benchmark run`/`benchmark validate` CLI. See
  [`benchmarking.md`](benchmarking.md) and
  [`benchmark-metrics.md`](benchmark-metrics.md). **This is measurement
  infrastructure, not a preservation claim** — see "Benchmark evidence
  is not a preservation claim" below.

**Not implemented yet:**

- **`process` does not support a partial page selection** (`--pages` is
  `analyze`-only in this milestone); see [`cli.md`](cli.md) for the
  narrower-scope decision and rationale.
- **Pseudo-F-measure and the end-to-end PDF (Level B) benchmark** are
  deliberately deferred — see [`benchmark-metrics.md`](benchmark-metrics.md)
  for why (no trustworthy reference/test oracle for pseudo-F yet; Level
  B is real, separate scope this milestone did not rush).
- No real-world (non-synthetic) benchmark corpus — see
  [`benchmark-datasets.md`](benchmark-datasets.md), "Real scholarly
  corpus plan," for the documented future protocol.
- **No public release exists.** Milestone 7A built the packaging
  infrastructure (see [`distribution.md`](distribution.md) and
  [`releasing.md`](releasing.md)) but did not publish a GitHub Release
  or create a tag.
- **No Developer ID signed or notarized artifact exists.** The macOS
  build is ad-hoc signed (a real, complete signature that satisfies
  `codesign --verify --deep --strict` and launches normally — see
  `docs/desktop-testing.md`, "macOS arm64: 'is damaged' bug found by
  human runtime testing") but not signed with a Developer ID
  certificate and not notarized; Windows and Linux artifacts are
  unsigned. Real Developer ID/notarization credentials were not
  available. See [`releasing.md`](releasing.md), "Signing and
  notarization."
- **Windows and Linux packaging builds and packages successfully in
  CI, but has no human runtime acceptance.** No Windows or Linux
  machine has exercised an actual built package interactively. See
  [`desktop-testing.md`](desktop-testing.md)'s verification-state table.
- **Mac App Store technical sandbox readiness is complete** (Milestone
  7B1) — App Sandbox, entitlements, and the sandboxed output-save path
  have passed local human sandbox-acceptance testing — but production
  Apple Developer signing/provisioning is still pending owner
  credentials, and no App Store Connect submission has been made. See
  [`mac-app-store-readiness.md`](mac-app-store-readiness.md).

**PDFium is not bundled with the crate or committed to this repository,
and the running application never downloads one at runtime.** See
[`pdfium.md`](pdfium.md) for the unchanged developer-setup story. As of
Milestone 7A, an *officially packaged* build (desktop app or CLI
archive) does carry its own trusted, checksum-verified PDFium, fetched
and staged at *build/package time only* — see
[`pdfium-bundling.md`](pdfium-bundling.md). No public package has
actually been distributed yet (see above), so this bundling exists in
the release infrastructure but has not reached an end user.

**Platform verification.** The architecture is cross-platform, but only
**aarch64-apple-darwin** has actually been built *and run* against a real
PDFium binary. Windows and Linux are unverified at runtime. The project
does not claim working support for all three operating systems merely
because the Rust code compiles.

**CI does not verify the PDF pipeline.** GitHub-hosted runners have no
PDFium, so every end-to-end integration test is reported as *ignored*
there. A green CI run means the code compiles, is formatted, passes
clippy, passes the PDFium-independent unit tests, and satisfies
`cargo-deny` — it says nothing about whether a PDF can actually be
converted. That evidence currently comes only from a provisioned local
macOS run; see [`testing-pdf-pipeline.md`](testing-pdf-pipeline.md).

**Output replacement atomicity.** On Unix and macOS, replacing an existing
output is a single atomic `rename(2)`. On Windows the old file must be
unlinked immediately before the rename, leaving a narrow window in which
neither name exists. No cross-platform atomicity is claimed; see
[`pdf-output.md`](pdf-output.md).

**Memory.** As of Milestone 3's persistent document session, the *entire
source file* is held in memory for the duration of an operation (the
open-bytes snapshot policy — see
[`pdf-pipeline-session.md`](pdf-pipeline-session.md)), in addition to one
uncompressed working page, algorithm buffers, and — for `process` — the
growing compressed output PDF assembled in memory. The honest bound is:

> source PDF bytes + one uncompressed working page
> + algorithm buffers + the growing compressed output (`process` only)

This is **not** O(1) in either source size or output size. Earlier
Milestone 2 documentation described only the per-page bound because that
milestone reopened the source file per page instead of holding it in
memory; that design no longer exists, and this section has been corrected
rather than left describing removed behavior.

**Size estimation is experimental, not a guarantee.** It is a sampled
approximation calibrated only against synthetic fixtures (±25% for
heterogeneous documents, ±15% for homogeneous ones, at the default 8
samples — engineering acceptance thresholds, not product guarantees). It
is not a statistical confidence interval, not a quality judgement about
the source scan, and not a "best settings" recommendation — the
estimator only reports measured numbers. A real scanned book with more
extreme per-page variation than the synthetic fixtures can miss by more
than these thresholds; the converted file's real size is always
authoritative. See [`size-estimation.md`](size-estimation.md).

**What conversion loses.** Output pages are rasterized. Hidden OCR text
layers, bookmarks, links, annotations, form fields, signatures, layers, and
attachments are **not** preserved. Text in the output is not selectable or
searchable.

**No preservation claim for Ancient Greek.** No claim is made that this
tool preserves polytonic Ancient Greek, critical apparatuses, or other fine
typographic detail. That will only be claimed if and when reproducible
benchmark data supports it (Phase 2).

**Benchmark evidence is not a preservation claim.** Milestone 6 adds a
real, reproducible benchmark *framework* and runs it once against a
committed synthetic fixture suite (`synthetic-document-v1`). That
suite validates the framework and measures defined synthetic stress
cases, including polytonic-diacritic-*like* and dense-apparatus-*like*
procedural shapes — it is explicitly **not** a representative corpus of
real scanned or printed material, and its results are **not** evidence
for a broad claim about preservation quality on historical polytonic
Greek editions. See [`benchmark-results/synthetic-v1.md`](benchmark-results/synthetic-v1.md)
for the actual first-run numbers and their interpretation, and
[`benchmark-datasets.md`](benchmark-datasets.md) for what a
rights-cleared real corpus would need before that broader claim could
be made.

All processing is local. There is no networking, telemetry, account, OCR,
or AI of any kind.

## Phase 1 non-goals (final, not just "not yet implemented")

Unlike the items above, which are simply not built yet, the following are
explicitly **out of scope for all of Phase 1**, not just this milestone:

- **OCR.** M PDF Processor does not perform optical character recognition
  and does not plan to in Phase 1.
- **Hidden OCR layer preservation.** If an input PDF already contains a
  hidden/invisible OCR text layer, Phase 1 does not preserve it in the
  output. Output PDFs are image-only, bilevel documents.
- **AI or machine-learned models.** Phase 1 uses only deterministic,
  classical image-processing algorithms (see [`algorithms.md`](algorithms.md)).
- **Generative restoration.** No inpainting, super-resolution, or other
  generative reconstruction of damaged, faded, or missing content.
- **Dewarping.** No geometric correction for curved or skewed page scans.
- **Annotation and form preservation.** Interactive form fields, comments,
  and other non-image PDF content in the source are not preserved in the
  output.

Whether and how any of these might be addressed is a question for later
phases (see [`roadmap.md`](roadmap.md)) — most notably Phase 2's benchmark
work on preserving Ancient Greek typography — and no commitment is made
here about if or when that will happen.
M6 desktop `api_then_local` uses the deterministic bundled reference OCR
provider when the remote service cannot complete. It is a safe, offline,
audited fallback and never downloads a model, but production-quality scanned
OCR still requires an explicitly configured RapidOCR/ONNX installation.
