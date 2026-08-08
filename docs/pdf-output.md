# PDF output format

What Museion Binarize writes, and what it does not preserve.
Rationale lives in [`adr/0002-bilevel-pdf-output.md`](adr/0002-bilevel-pdf-output.md).

## Every output page

One page object, one image XObject, one content stream, one resource
dictionary. The image is a true bilevel raster:

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

`/K -1` is Group 4. `/BlackIs1 true` makes the decoder emit a 1 bit per
black pixel; because a `/DeviceGray` sample of 1 is *white*, `/Decode [1 0]`
is required to map that 1 back to black. Both entries are needed — dropping
either inverts the page.

## Geometry

Pixel dimensions come from `pixels = round(points * DPI / 72)`.

Page rotation is **normalized into the geometry**: PDFium renders each page
in its visible orientation, so the rebuilt page uses the visible rectangle
and carries no `/Rotate`. Visible width, height, and orientation are
preserved within 0.1 pt.

The visible box is the **CropBox** when valid, otherwise the **MediaBox**.

The image is drawn once under an explicit matrix `[w 0 0 h 0 0]`, placing
raster row 0 at the top of the page.

## Metadata

Title, Author, Subject, and Keywords are copied when present in the source,
stripped of control characters and capped in length. Nothing is invented,
and **no `/CreationDate` or `/ModDate` is written** — a timestamp would make
repeated runs differ byte-for-byte.

## Not preserved

Pages are **rasterized**. The following do not survive, and the project does
not claim otherwise:

* hidden OCR text layers (output is image-only — text is not selectable or
  searchable);
* bookmarks, outlines, links;
* annotations and form fields;
* digital signatures;
* optional-content groups (layers);
* file attachments;
* XMP metadata;
* TrimBox, BleedBox, ArtBox.

## Determinism

The same input, settings, and version produce byte-identical output. Object
ids are allocated in a fixed order and no clock, filesystem, or random
source contributes to the bytes.

## Safe writing

The CLI and the GitHub-distributed desktop app — neither ever runs under
App Sandbox — write output through
`OutputWriteStrategy::AtomicSameDirectoryRename`: a temporary file in the
destination's directory, flushed, synced, reopened, and validated, and
only then atomically renamed into place. Failure or cancellation removes
the temporary file and leaves both the source and any existing
destination untouched. Overwriting is off by default.

On Windows, renaming onto an existing file is not permitted, so with
`--overwrite` the old file is unlinked immediately before the rename. There
is a brief window in which neither name exists.

A Mac App Store build (only) instead uses
`OutputWriteStrategy::DirectWriteToDestination`, because a sandboxed
process's Powerbox grant for a user-selected save destination is scoped
to that exact path, not a sibling temp file in the same directory — see
`docs/mac-app-store-readiness.md`, "Sandboxed output-save architecture,"
for the full evidence and trade-off. Validation still happens before the
destination is ever touched (in the app's own always-writable container
temp directory), so a validation failure or cancellation still leaves an
existing destination completely untouched — but the final write to the
destination is a plain write, not a same-filesystem atomic rename, so it
does not have the same crash-window guarantee: a process killed mid-write
to the destination can leave it holding a partial file. This applies only
to the Mac App Store build, never to the CLI or the GitHub desktop
build.

## Size expectations

Output is smaller than a scanned source for real scans, where the input
carries continuous-tone page images. For the tiny synthetic vector fixtures
used in tests the output is *larger* than the input, because a few hundred
bytes of vector drawing commands become a real rasterized page. That is
expected and is not a regression.
