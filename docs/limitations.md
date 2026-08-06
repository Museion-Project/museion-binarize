# Limitations

## Current state (Milestone 0)

This repository, at Milestone 0, is a project scaffold. It establishes the
Rust workspace, a minimal Tauri 2 + React + TypeScript desktop shell,
documentation, licensing, and CI — **it does not yet process PDFs in any
way.**

Specifically, as of this milestone:

- There is no PDF rasterization, thresholding, encoding, or PDF-writing
  implementation. `museion-binarize-core` exposes only placeholder project
  metadata (see [`architecture.md`](architecture.md)).
- The CLI (`museion-binarize-cli`) supports `--version`, `--help`, and an
  `info` command only. It does not accept or convert PDF files.
- The desktop application displays a static "Phase 1 — under development"
  screen and calls one Tauri command to confirm the frontend/backend bridge
  works. Its "Open PDF" control is present but disabled, and does not open
  or process any file.
- No thresholding algorithm (Otsu, Sauvola, or manual) is implemented.
- No CCITT Group 4 encoding is implemented.
- No benchmarking framework or benchmark data exists yet.

Early Phase 1 builds that follow this milestone will incrementally add real
processing capability per [`roadmap.md`](roadmap.md); this document should
be updated as that happens so it never overstates what the software does.

## Phase 1 non-goals (final, not just "not yet implemented")

Unlike the items above, which are simply not built yet, the following are
explicitly **out of scope for all of Phase 1**, not just this milestone:

- **OCR.** Museion Binarize does not perform optical character recognition
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
