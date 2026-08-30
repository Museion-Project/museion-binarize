# Limitations

## Current state (rc.3 source under preparation; public release remains rc.2)

M6 does not ship a vendor integration or make paid calls. The reusable client
speaks the provider-neutral `mpdf-api` 0.1 contract and CI validates it with a
deterministic loopback HTTP fixture. It uses the platform-native
Keychain/Credential Manager/Secret Service; an unavailable or locked store
fails visibly and never falls back to a plaintext token. Endpoint discovery,
OAuth, telemetry, cloud bookmark generation, and automatic upload remain out
of scope. This explicit-endpoint developer/private-service protocol is not a
v2 OCR product mode, Gemini BYOK, or the unavailable `mpdf-credits` service.

### Local text recognition

The base release has no OCR runtime. Tesseract 5 (LSTM) with the pinned
`tessdata_best` 4.1.0 model set is the adopted local-plugin candidate and the
only candidate that cleared the recorded gold evaluation; see
[`ocr-engines.md`](ocr-engines.md) for the full bake-off. It is not a required
dependency of the deterministic base application.

Known boundaries:

- **Models are never downloaded by the application.** Without the optional
  local OCR plugin, native-text PDFs and non-OCR conversion still work. A
  scanned page that needs recognition reports provider unavailable; it does
  not silently degrade to a worse recognizer or to a successful empty layer.
- **OCR is an independent optional artifact.** Tesseract, its sidecar, pinned
  models, runtime closure, licenses, SBOM, and installed smoke belong to the
  `optional-local-ocr-plugin` profile. Their pending cross-platform evidence
  does not block or become a hidden dependency of the base profile.
- **Isolated accented characters are unreliable.** Accented Greek vowels
  printed on their own, with no surrounding word, are recognized at a
  measurably worse rate (diacritic error ≈0.26 on the `grc-codepoints`
  sample) than the same characters inside words (0.000 on both Greek prose
  samples). This is a property of the recognizer, not a threshold.
- **Roman front-matter page numbers are less reliable than Arabic ones.** In
  the gold contents samples every row is assembled with a trailing number,
  but lowercase Roman numerals after a leader run are misread often enough
  that 2 of 4 were wrong. A misread number is caught downstream by the
  bookmark engine's printed-page mapping, which refuses rather than guesses.
- **Wrapped contents titles are not joined across lines.** A contents entry
  whose title wraps onto a second line is recovered as the line that carries
  the page number; the continuation is not merged into the title.
- **Three or more columns are not modelled**, in the logical-line assembler
  as in the bookmark engine.
- **Rotation is handled by an orientation pass**, which is skipped when its
  confidence is low. A page whose orientation cannot be determined is
  recognized as it arrived rather than being rotated on a guess.
- **A language profile is a promise about the page, not a detector.** Running
  a Greek document under `english` produces Latin lookalikes; the evidence
  records a `script_violations` count so this is visible, but nothing rewrites
  the text.
- The gold corpus is **synthetic**: text rendered from a single OFL font. It
  is a regression gate for script, diacritic and layout handling, not a claim
  about accuracy on real scans. Real-corpus figures must be produced on the
  operator's own machine and are not committed.

### The final PDF

`mpdf run` (and the desktop app's main button) produce one file that is
binarized, searchable, and outlined, by OCR-ing the original pages first and
binarizing only afterwards. Boundaries:

- **Bookmark refusal does not discard a valid conversion.** If the document
  yields no reliable structure, the combined `mpdf run` flow still writes and
  verifies the searchable bilevel PDF, reports `bookmark_status =
  safe_refusal`, and leaves its outline empty rather than inventing entries.
  The standalone `mpdf bookmark auto` command keeps its narrower contract and
  writes no PDF on safe refusal.
- **Rotation is normalized into the output.** The binarized carrier is written
  upright with no `/Rotate`, and the text layer is placed accordingly. Readers
  see the same visible page; a tool that inspects `/Rotate` will not see the
  source's value.
- **`/CropBox` is not carried onto the binarized output.** The visible crop is
  baked into the raster and the output's `MediaBox` is the source's *visible*
  size with its origin at (0, 0). Page count, order, and visible geometry are
  preserved and verified on reopen.
- **Links, page labels, and source metadata are not carried onto the
  binarized output.** The bilevel writer reconstructs the document; only the
  pages, the invisible text layer, and the confirmed outline are written. The
  source-preserving path (`mpdf pdf build-searchable`) is the one that keeps
  the original objects.
- **A resumed run reuses committed OCR pages only.** Changing the source,
  engine, sidecar bytes, model-file bytes, language profile, or OCR DPI changes
  the job fingerprint, so evidence from different configurations is rejected
  rather than mixed. Moving byte-identical models to another directory does
  not change their identity.

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
- **No rc.3 public release exists.** The public download remains rc.2;
  rc.3 packaging infrastructure is under owner review and has no rc.3 tag.
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

**Ordinary PR CI does not verify the PDF pipeline.** Its GitHub-hosted
runners do not provision PDFium, so end-to-end tests are *ignored* there.
The manual distribution workflow now runs release PDFium smoke gates after
fetching the pinned library. A green ordinary CI run still says nothing about
whether a PDF can be converted; see [`testing-pdf-pipeline.md`](testing-pdf-pipeline.md).

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

Conversion and the optional local OCR plugin run offline. Provider discovery
has no available remote OCR product mode; there is no cloud bookmark
generation, telemetry, or account requirement. The retained generic M6 API
client can upload only when an operator separately supplies an endpoint,
credential, and matching consent digest; it is not an OCR provider fallback.

## Cloud OCR providers

- **M PDF Cloud OCR (`mpdf-credits`) is unavailable.** It is the only planned
  cloud product mode and would be paid and brokered, with task-level consent
  and a hard maximum-credit ceiling. No production backend independently
  returns the complete-coordinate OCR contract; payment, production signing
  keys, provider-key custody, privacy/retention, and deletion are also absent.
  Protocol fixtures are not a production service, no credits can be bought,
  and neither front end can start a run.
- **Gemini BYOK is disabled.** The legacy value remains deserializable so old
  settings fail predictably, but it is omitted from provider lists/pickers and
  no credential command reads a key. It will be reconsidered only for an API
  that independently returns complete coordinate OCR.
- **Text-only/model-rectangle experiments are not complete OCR.** The retained
  local-geometry plus transcription path is `experimental-composite`.
  `TextEnhancer` output may become a reviewable revision over existing
  canonical evidence; it cannot recognize an empty scan, supply authoritative
  coordinates, or decide bookmarks.

## Phase 1 non-goals and remaining limitations

Unlike the items above, which are simply not built yet, the following are
explicitly **out of scope for all of Phase 1**, not just this milestone:

- **Hidden OCR layer preservation.** If an input PDF already contains a
  hidden/invisible OCR text layer, Phase 1 does not preserve it in the
  ordinary conversion output. The separate searchable-PDF export may
  intentionally add typed text and bookmarks from typed evidence.
- **Generative or black-box models for binarization/bookmark decisions.**
  Those decisions use deterministic, classical image-processing and explicit
  evidence rules (see [`algorithms.md`](algorithms.md)). An optional OCR plugin
  may use explicitly provisioned models; it is a separate artifact and is not
  used by the binarization or bookmark decision engine.
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
