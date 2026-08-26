# Roadmap

## Phases

### Phase 1 — Cross-platform deterministic binarization

Build a reliable, local, cross-platform tool that converts scanned PDFs into
clean, compact, true 1-bit PDFs using deterministic thresholding (Otsu,
Sauvola, manual) and CCITT Group 4 compression, with both a GUI and a CLI.
No OCR, no AI, no generative restoration. This is the phase this repository
is currently in.

### Phase 2 — Benchmark construction for Ancient Greek preservation

Before any claim is made about preserving polytonic Ancient Greek text,
critical apparatuses, or other small typographic detail, build a reproducible
benchmark: representative (permission-cleared or synthetic) sample pages,
ground truth, and the metrics described in
[`benchmarking.md`](benchmarking.md). Use this benchmark to measure how
Phase 1's deterministic methods perform on this material, and to identify
concretely where they fail.

### Phase 3 — Optional AI-assisted methods, only after evaluation

Only after Phase 2 produces benchmark data may AI-assisted or learned
methods be evaluated as optional, clearly-labeled alternatives to the
deterministic Phase 1 pipeline — and only if benchmarks show a real,
reproducible improvement for the material Phase 2 targets. Nothing in this
phase is scoped or committed to yet.

## Phase 1 milestones

> **Note on numbering.** Earlier drafts of this document numbered the
> milestones differently (rasterization as Milestone 1, thresholding as
> Milestone 2). The list below reflects what was actually built, in the
> order it was built, and is the authoritative numbering.

- **Milestone 0 — Repository initialization.** Rust workspace, Tauri 2 +
  React + TypeScript desktop scaffold, bilingual documentation, dual
  licensing, citation metadata, contributor guidelines, and initial CI.
  *(Complete.)*
- **Milestone 1 — Deterministic image-processing core.** Grayscale
  conversion and contrast, Otsu / Sauvola / manual thresholding,
  conservative preprocessing, despeckle cleanup, bilevel packing, and
  CCITT Group 4 encoding in `mpdf-core`, with unit tests.
  *(Complete.)*
- **Milestone 2 — End-to-end PDF pipeline.** PDFium rasterization, page
  inspection and geometry, the single page-processing orchestrator,
  deterministic 1-bit CCITT Group 4 PDF reconstruction, bounded-memory
  sequential processing, cancellation, temporary-file and atomic
  persistence, output validation, and enough CLI wiring (`inspect`,
  `process`, `preview`) to exercise and verify it. *(Complete; end-to-end
  behaviour verified only on a provisioned Apple Silicon macOS environment
  — CI runs the PDFium tests as ignored and verifies nothing about the
  pipeline. See [`limitations.md`](limitations.md) and
  [`testing-pdf-pipeline.md`](testing-pdf-pipeline.md).)*
- **Milestone 3 — CLI feature completeness and analysis commands.** The
  full command surface (`info`, `inspect`, `analyze`, `process`,
  `preview`), machine-readable versioned JSON reports for scripting, and a
  persistent PDFium document session. *(Complete; end-to-end behaviour —
  including `analyze` and the source-mutation-immunity test — verified on
  the same provisioned Apple Silicon macOS environment as Milestone 2.
  See [`limitations.md`](limitations.md) and
  [`testing-pdf-pipeline.md`](testing-pdf-pipeline.md).)*

  This milestone resolved the persistent-session problem Milestone 2
  deferred: `PdfRenderer` reopened and reparsed the source file on every
  `render_page` call, because `PdfDocument` was believed to require a
  self-referential struct to persist across calls. That belief was wrong
  — see [`pdf-pipeline-session.md`](pdf-pipeline-session.md) for why the
  pinned `pdfium-render` API makes a real, safe, single-open session
  possible without `unsafe` code. The replacement,
  `document_session.rs`'s `PdfDocumentSession`, opens the source exactly
  once per operation (an "open-bytes snapshot": the whole file is read
  into memory once and PDFium loads from that owned buffer), which also
  resolves the time-of-check/time-of-use concern by construction — there
  is no second read of the filesystem to race against a mutation. The
  memory-model consequence (the source's bytes are now held for the whole
  operation, not just one page) is documented honestly in
  [`limitations.md`](limitations.md) rather than left as the old, no
  longer accurate, per-page-only bound.
- **Milestone 4 — Desktop GUI feature completeness.** Wire the same
  pipeline into the Tauri desktop app: file selection, a persistent
  per-window document session with a dedicated PDFium worker thread,
  lazily-loaded thumbnails, before/after preview (through the real core
  pipeline, at the real conversion DPI), settings and deterministic
  presets, asynchronous processing with progress events and real
  cancellation, and structured error/completion presentation.
  *(Complete. Automated tests — ordinary, frontend, and
  provisioned-PDFium, including a test proving the CLI and the desktop
  app produce byte-identical output for identical settings — all pass,
  and native macOS acceptance testing has been performed against the
  real running Tauri application with a provisioned PDFium library,
  including a 100-page real-world scanned-document conversion (Sauvola,
  400 DPI). See [`desktop.md`](desktop.md) and
  [`desktop-testing.md`](desktop-testing.md) for the full acceptance
  record, including the observed processing-time/compression baseline —
  which is a single data point from one document on one machine, not a
  performance guarantee.)*
- **Milestone 5 — Output size estimation.** Sampled output size prediction
  (clearly labelled experimental, and not to be implemented as if it were
  reliable). Basic process and analysis reports already exist as of
  Milestone 3; see [`reporting.md`](reporting.md). *(Complete. Adds a
  deterministic, evenly-spaced page sample estimated through the real
  pipeline (no separate fast/approximate estimator), a
  `mpdf-size-estimate` report schema, an `estimate` CLI
  command reusing `analyze`/`process`'s own settings parsing, an
  "Estimate" panel in the desktop app with caching/staleness/cancellation
  and automatic estimate-vs-actual comparison on the next matching
  conversion, and richer per-page/aggregate metrics plus simple
  document-relative outlier flags on `process`'s own report. Central
  estimate and range methodology, and why they are not called a
  confidence interval, are documented in
  [`size-estimation.md`](size-estimation.md). Accuracy against synthetic
  heterogeneous/homogeneous fixtures meets the milestone's own engineering
  thresholds (±25%/±15% at the default 8 samples) — engineering
  acceptance thresholds, not product guarantees about any real document.
  See [`reporting.md`](reporting.md) for the schema and
  [`limitations.md`](limitations.md) for what the estimate is not.)*
- **Milestone 6 — Reproducible benchmarking framework.** The metrics and
  reporting pipeline described in [`benchmarking.md`](benchmarking.md),
  runnable on non-copyrighted fixtures. *(Complete. Adds a reproducible,
  versioned ground-truth benchmark framework with deterministic
  non-copyrighted synthetic fixtures, standard document-binarization
  fidelity metrics, ROI/category reporting, dataset and profile
  digests, and CLI benchmark execution. The committed synthetic
  benchmark validates the framework and measures defined stress cases;
  it is not yet evidence for a broad claim about preservation quality
  across historical polytonic Greek editions — see
  [`benchmark-metrics.md`](benchmark-metrics.md),
  [`benchmark-datasets.md`](benchmark-datasets.md), and
  [`benchmark-results/synthetic-v1.md`](benchmark-results/synthetic-v1.md).
  Pseudo-F-measure and the end-to-end PDF (Level B) benchmark level are
  deliberately deferred, each for a documented reason, rather than
  shipped with uncertain correctness.)*
- **Milestone 7A — Distribution foundation.** Self-contained production
  packaging for official GitHub builds: trusted bundled PDFium (desktop
  and CLI) with a pinned, checksum-verified provenance manifest,
  production Tauri packaging (macOS `.app`/`.dmg`, Windows, Linux
  AppImage/`.deb`), deterministic artifact naming, release checksums
  and a versioned release-manifest, a `workflow_dispatch`-only GitHub
  Actions build workflow that never auto-publishes, Developer-ID
  signing/notarization integration points, and a Mac App Store
  readiness audit for the future M7B. *(Complete. macOS arm64 built,
  packaged, and non-interactively launch-verified from outside the
  repository with no environment variable; the standalone CLI archive
  was verified end to end — inspect/process/benchmark all succeeded
  against a fresh extraction with bundled PDFium and no
  `MPDF_PDFIUM_LIBRARY`. Windows/Linux packaging is configured and
  CI-targeted but not exercised on real hardware this milestone;
  signing/notarization integration exists but no artifact is actually
  signed or notarized (no credentials available). No public release or
  tag was created. See [`distribution.md`](distribution.md),
  [`releasing.md`](releasing.md), [`pdfium-bundling.md`](pdfium-bundling.md),
  and [`desktop-testing.md`](desktop-testing.md)'s verification-state
  table for the honest, per-platform breakdown.)*
- **Milestone 7B — Mac App Store distribution.** Paid Mac App Store
  edition: App Sandbox migration, Mac App Store signing/provisioning,
  App Store Connect metadata and submission, store privacy/review
  requirements. Scope only — see
  [`mac-app-store-readiness.md`](mac-app-store-readiness.md) for the
  M7A-era audit of what this would require. **Not started.**

Milestone boundaries may shift as implementation reveals new constraints;
this document will be updated accordingly rather than treated as a fixed
contract.
