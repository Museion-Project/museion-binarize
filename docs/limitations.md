# Limitations

## Current state (Milestone 2)

Museion Binarize can now perform a complete local PDF conversion:

```
input.pdf -> PDFium rasterization -> image-processing core
          -> true bilevel image -> CCITT Group 4
          -> rebuilt 1-bit output.pdf -> reopened and validated
```

**Implemented:**

- the deterministic image-processing algorithms (Otsu, Sauvola, manual
  thresholding, conservative preprocessing, despeckle cleanup);
- PDF input, page inspection, and rasterization at 300 / 400 / 600 DPI;
- bilevel PDF reconstruction as true 1-bit `/CCITTFaxDecode` image
  XObjects (see [`pdf-output.md`](pdf-output.md));
- a minimal CLI that can `inspect`, `process`, and `preview`;
- bounded-memory sequential page processing, cancellation, safe temporary
  files with atomic persistence, and output validation that reopens and
  renders the finished file.

**Not implemented yet:**

- **The desktop GUI is not connected to the pipeline.** It still shows a
  static screen with a disabled "Open PDF" control. Use the CLI.
- The full CLI surface (`analyze`, JSON reports) is Milestone 3.
- Output size estimation, the benchmarking framework, and release
  packaging do not exist yet.
- No benchmark data or fixtures beyond synthetic generated ones.

**PDFium must be supplied separately.** It is not bundled with the crate,
not committed to this repository, and never downloaded at runtime. See
[`pdfium.md`](pdfium.md). Release bundling of PDFium is Milestone 7.

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

**Memory.** Uncompressed page buffers are bounded — one working page at a
time. The *compressed* output PDF is assembled in memory, so total usage
grows with document length. The honest bound is: one uncompressed working
page + algorithm buffers + the growing compressed output. This is not O(1)
in page count.

**What conversion loses.** Output pages are rasterized. Hidden OCR text
layers, bookmarks, links, annotations, form fields, signatures, layers, and
attachments are **not** preserved. Text in the output is not selectable or
searchable.

**No preservation claim for Ancient Greek.** No claim is made that this
tool preserves polytonic Ancient Greek, critical apparatuses, or other fine
typographic detail. That will only be claimed if and when reproducible
benchmark data supports it (Phase 2).

All processing is local. There is no networking, telemetry, account, OCR,
or AI of any kind.

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
