# Limitations

## Current state (Milestone 1)

This repository has completed Milestone 1: the image-processing core
(grayscale conversion, Otsu/Sauvola/manual binarization, conservative
preprocessing, despeckle cleanup, bilevel packing, and CCITT Group 4
encoding) is implemented and unit-tested in `museion-binarize-core`. **It
still does not process a PDF file end to end** — there is no PDF
rasterization or PDF-writing yet; that is Milestone 2.

Specifically, as of this milestone:

- `museion-binarize-core` can binarize an in-memory grayscale image and
  pack/CCITT-encode the result, but has no PDFium binding, no PDF page
  rendering, and no PDF reconstruction (`docs/pdf-output.md`-equivalent
  functionality does not exist yet).
- The CLI (`museion-binarize-cli`) supports `--version`, `--help`, and an
  `info` command only. It does not accept or convert PDF files.
- The desktop application displays a static "Phase 1 — under development"
  screen and calls one Tauri command to confirm the frontend/backend bridge
  works. Its "Open PDF" control is present but disabled, and does not open
  or process any file.
- No benchmarking framework or benchmark data exists yet.

Early Phase 1 builds that follow this milestone will incrementally add real
PDF processing capability per [`roadmap.md`](roadmap.md); this document
should be updated as that happens so it never overstates what the software
does.

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
