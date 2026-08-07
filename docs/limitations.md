# Limitations

## Current state (Milestone 3)

Museion Binarize can perform a complete local PDF conversion and can
analyze a PDF without converting it:

```
input.pdf -> PDFium rasterization -> image-processing core
          -> true bilevel image -> CCITT Group 4
          -> rebuilt 1-bit output.pdf -> reopened and validated   [process]

input.pdf -> PDFium rasterization -> image-processing core
          -> per-page/document measurements -> JSON report        [analyze]
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
- documented, tested exit codes and a stdout/stderr contract that keeps
  `--json` output free of progress text or prose;
- cancellation, safe temporary files with atomic persistence, and output
  validation that reopens and renders the finished file.

**Not implemented yet:**

- **The desktop GUI is not connected to the pipeline.** It still shows a
  static screen with a disabled "Open PDF" control. Use the CLI.
- **`process` does not support a partial page selection** (`--pages` is
  `analyze`-only in this milestone); see [`cli.md`](cli.md) for the
  narrower-scope decision and rationale.
- Output size estimation, the reproducible benchmarking framework, and
  release packaging do not exist yet (Milestones 5–7).
- No benchmark data or fixtures beyond synthetic generated ones.
- No automatic, checksum-verified PDFium provisioning in CI (Milestone 7);
  the PDFium-dependent tests remain `#[ignore]`d there.

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
