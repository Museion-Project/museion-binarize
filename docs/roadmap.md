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

- **Milestone 0 — Repository initialization.** Rust workspace, Tauri 2 +
  React + TypeScript desktop scaffold, bilingual documentation, dual
  licensing, citation metadata, contributor guidelines, and initial CI.
  *(This milestone — no PDF processing yet.)*
- **Milestone 1 — Core image I/O and rasterization.** Integrate the PDFium
  boundary; rasterize PDF pages to in-memory images in
  `museion-binarize-core`, with bounded memory use.
- **Milestone 2 — Deterministic thresholding.** Implement Otsu, Sauvola, and
  manual thresholding in the core, with unit tests against known reference
  outputs.
- **Milestone 3 — Bilevel PDF reconstruction.** Implement CCITT Group 4
  encoding and true 1-bit PDF writing, producing a valid, standards-compliant
  output PDF from thresholded pages.
- **Milestone 4 — CLI feature completeness.** Wire the core pipeline into
  `museion-binarize-cli` with a stable command surface suitable for
  scripting and benchmarking.
- **Milestone 5 — Desktop GUI feature completeness.** Wire the same pipeline
  into the Tauri desktop app: file selection, parameter controls, progress
  reporting, and output preview.
- **Milestone 6 — Reproducible benchmarking framework.** Implement the
  metrics and reporting pipeline described in
  [`benchmarking.md`](benchmarking.md), runnable in CI on non-copyrighted
  fixtures.
- **Milestone 7 — Cross-platform packaging and release.** DMG, MSI, and
  Linux packages (AppImage/deb/rpm), built and signed as appropriate, plus
  a first tagged release.

Milestone boundaries may shift as implementation reveals new constraints;
this document will be updated accordingly rather than treated as a fixed
contract.
