# ADR 0002 — Rebuilt 1-bit CCITT Group 4 PDF output

**Status:** Accepted (2026-08-07). Proposed during Milestone 2 and accepted
only after the end-to-end polarity, orientation, geometry, and determinism
tests passed locally.

## Context

The product goal is a *clean, compact bilevel* PDF. That requires deciding
how output pages are produced and exactly what goes into them.

## Decision

### Output pages are rebuilt, not saved through PDFium

PDFium can save a document, but doing so would carry along source objects
this project does not model, cannot test, and does not claim to preserve.
Rebuilding from scratch means every byte is one we chose and can assert on.
It also makes byte-for-byte determinism achievable.

The cost is honest and documented: everything not re-created is **lost**.
Bookmarks, links, annotations, form fields, signatures, optional-content
layers, attachments, XMP metadata, and hidden OCR text layers do not
survive. Pages are rasterized.

### True 1-bit image XObjects

Each page is one image XObject with `/BitsPerComponent 1` and
`/ColorSpace /DeviceGray`. Writing an 8-bit grayscale image that merely
*looks* black-and-white would defeat the entire point: an 8-bit page is
roughly eight times larger and no more faithful.

### CCITT Group 4, and why not the alternatives

CCITT Group 4 (ITU-T T.6) is lossless for bilevel rasters, is the standard
encoding for compact scanned-document PDFs, and is supported by every PDF
reader. It adds no loss beyond what thresholding already committed to.

* **JPEG (`/DCTDecode`) is excluded**: it is lossy and designed for
  continuous-tone images. On 1-bit text it produces ringing artefacts
  around every stroke while being *larger* than G4.
* **JBIG2 is excluded**: lossy/generic-region JBIG2 substitutes visually
  similar glyphs for one another. That has caused real-world digit
  substitution in scanned documents — unacceptable for scholarly text, and
  directly contrary to this project's "no generative rewriting" principle.

`validation::assert_bilevel_ccitt_structure` fails the build if
`/DCTDecode`, `/JBIG2Decode`, or `/JPXDecode` ever appears in output.

### PDF writing crate

`pdf-writer` 0.15 (MIT OR Apache-2.0, from the Typst project): a low-level,
step-by-step writer that gives explicit control over objects and streams,
which is what deterministic output requires.

### Image XObject dictionary

```
/Type /XObject
/Subtype /Image
/Width  <pixel width>
/Height <pixel height>
/ColorSpace /DeviceGray
/BitsPerComponent 1
/Interpolate false
/Filter /CCITTFaxDecode
/Decode [1 0]
/DecodeParms << /K -1 /Columns <w> /Rows <h> /BlackIs1 true >>
```

### Bit polarity — determined empirically

This crate's `BilevelImage` uses **1 = black** (`black_is_one == true`).
Mapping that into PDF took two entries, and the combination was found by
rendering output back through PDFium, not by reading the dictionary and
assuming it looked right:

* `/BlackIs1 true` makes the CCITT decoder emit a 1 bit per black pixel,
  matching the crate convention.
* But in `/DeviceGray`, sample value 1 is **white**. With `/BlackIs1 true`
  alone the page renders **inverted** — this actually happened, and the
  orientation/polarity test caught it.
* `/Decode [1 0]` flips the sample-to-colour mapping so a 1 bit means
  black.

The internal `BilevelImage` convention was *not* changed to paper over the
writer; the writer was fixed. Removing either entry re-inverts the page and
fails `end_to_end_conversion_preserves_polarity_and_orientation`.

### Page geometry and rotation

**One strategy, applied uniformly: rotation is normalized into the
geometry, and every output page is written upright with no `/Rotate`.**

PDFium renders a page in its *visible* orientation, applying the source
`/Rotate`. The resulting raster is therefore already upright, and the
rebuilt page uses the visible rectangle (`display_width_points` x
`display_height_points`). The alternative — preserving the original
rectangle plus `/Rotate` and counter-rotating the content — was rejected
because mixing both strategies is where orientation bugs breed.

Visible page dimensions are preserved within a documented 0.1 pt tolerance
(a page size makes a round trip through `f32` points and a rounded integer
pixel count, so exact equality would be the wrong assertion).

Page box policy: the **CropBox** when valid, otherwise the **MediaBox** —
this is what PDFium reports as the page size. TrimBox, BleedBox, and ArtBox
are not preserved.

The content stream draws the image exactly once under an explicit
transformation matrix `[w 0 0 h 0 0]`. PDF image space is the unit square
with its first sample row at the top, so scaling to the page rectangle
places raster row 0 at the top with no flip. Nothing relies on a default
transformation.

### Deterministic object ordering

Object ids are allocated in a fixed sequence — catalog, page tree, then
per page (page, image, content) — so the same input and settings always
produce identical bytes. **No `/CreationDate` or `/ModDate` is written**,
because a wall-clock timestamp would break exactly that property. Metadata
(Title, Author, Subject, Keywords) is copied only when present in the
source, sanitized of control characters and length-capped; nothing is
invented.

`repeated_conversions_are_byte_for_byte_identical` enforces this.

### Temporary file and atomic persistence

The destination is never written incrementally. The document is built in
memory, written to a temporary file **in the destination's directory** (so
the later rename stays on one filesystem and is atomic), flushed, synced,
reopened, validated, and only then renamed into place. Any failure or
cancellation drops the temporary file, which deletes it; the source and any
pre-existing destination are left untouched.

Overwrite is off by default. When enabled, the old destination is removed
only after the replacement has been built and validated. On Windows a
rename onto an existing path fails, so the old file is unlinked immediately
before the rename — leaving a very small window in which neither name
exists. That platform-specific caveat is documented rather than hidden.

### Output validation

Validation lives in `validation.rs`, separate from the writer, so a writer
bug cannot also define what "valid" means. It always **reopens the finished
file with PDFium** and renders from it — checking the page count, every
page's visible dimensions within tolerance, that output pages carry no
`/Rotate`, and that pages actually rasterize. `Structural` renders the
first and last page; `RenderAll` renders every page.

Byte-level inspection for `/CCITTFaxDecode` is a supplementary assertion
only. Searching for strings in PDF bytes is not proof that a reader can
open the file, so it is never the sole check.

### Memory characteristics — stated honestly

Pages are processed strictly one at a time; the rendered bitmap, grayscale
buffer, and binary mask for page N are dropped before page N+1 is rendered.
Only *compressed* CCITT streams are retained, because `pdf-writer`
assembles the document in memory.

The truthful bound is:

> approximately one uncompressed working page
> + algorithm working buffers
> + the growing compressed output PDF

This is **not** O(1) in document length, and the documentation does not
claim it is. Writing a custom streaming PDF serializer to remove the last
term was judged not worth the risk at this milestone.
