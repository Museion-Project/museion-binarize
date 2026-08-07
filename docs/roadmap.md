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
  CCITT Group 4 encoding in `museion-binarize-core`, with unit tests.
  *(Complete.)*
- **Milestone 2 — End-to-end PDF pipeline.** PDFium rasterization, page
  inspection and geometry, the single page-processing orchestrator,
  deterministic 1-bit CCITT Group 4 PDF reconstruction, bounded-memory
  sequential processing, cancellation, temporary-file and atomic
  persistence, output validation, and enough CLI wiring (`inspect`,
  `process`, `preview`) to exercise and verify it. *(Complete; verified on
  Apple Silicon macOS only — see [`limitations.md`](limitations.md).)*
- **Milestone 3 — CLI feature completeness and analysis commands.** The
  full command surface, including `analyze` and machine-readable JSON
  reports, suitable for scripting and benchmarking.
- **Milestone 4 — Desktop GUI feature completeness.** Wire the same
  pipeline into the Tauri desktop app: file selection, thumbnails,
  before/after preview, parameter controls, presets, progress, and
  cancellation.
- **Milestone 5 — Size estimation and processing reports.** Sampled output
  size prediction (clearly labelled experimental) and richer processing
  reports.
- **Milestone 6 — Reproducible benchmarking framework.** The metrics and
  reporting pipeline described in [`benchmarking.md`](benchmarking.md),
  runnable on non-copyrighted fixtures.
- **Milestone 7 — Cross-platform packaging and release.** Verified Windows
  and Linux builds, DMG / MSI / AppImage / deb packaging, PDFium bundling,
  and a first tagged release.

Milestone boundaries may shift as implementation reveals new constraints;
this document will be updated accordingly rather than treated as a fixed
contract.
