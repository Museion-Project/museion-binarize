# Testing the PDF pipeline

## Two tiers

**Ordinary tests** (`cargo test --workspace`) need no PDFium. They cover
page geometry, the image-processing orchestrator, bit packing, CCITT
round-trips, PDF dictionary construction, destination safety, atomic
persistence, and CLI argument validation. They must always pass on a clean
machine.

**PDFium integration tests** (`crates/museion-binarize-core/tests/pdf_pipeline.rs`)
need a real PDFium library. Without `MUSEION_PDFIUM_LIBRARY` set, each one
prints `SKIPPED: ...` and returns rather than failing, so a contributor
with no PDFium still gets a green run. Run them with:

```bash
MUSEION_PDFIUM_LIBRARY=/path/to/libpdfium.dylib \
  cargo test -p museion-binarize-core --test pdf_pipeline
```

A skipped run is *not* a passing run. Before declaring PDF-pipeline work
done, run them with a real library and say so.

## Synthetic fixtures only

Fixtures are generated programmatically by `test_fixtures.rs` from vector
shapes this project draws. **No scanned or copyrighted page is ever
committed.** Fixtures are written into temporary directories at test time.

| Fixture | Purpose |
|---|---|
| `orientation_and_polarity()` | One page, deliberately asymmetric: large black square top-left, small black square bottom-right, a horizontal bar near the top and a vertical bar near the left. Detects inversion, mirroring, flipping, wrong rotation, and scaling errors. |
| `mixed_page_sizes()` | Portrait, landscape, and a smaller custom page — proves mixed sizes survive. |
| `page_rotations()` | Four pages with `/Rotate` 0, 90, 180, 270. |
| `threshold_patterns()` | Grayscale bands, thin strokes, and isolated specks — proves the thresholding algorithms actually run. |

## What the integration tests assert

* the pinned library loads, and is reported as an explicit path;
* a generated PDF opens, with correct page count and geometry;
* pages render at 300, 400, and 600 DPI with the expected pixel sizes;
* **polarity**: after a full conversion, the output is reopened and
  rendered, and black marker regions really are black while open areas
  really are white;
* **orientation**: the top-left marker is darker than both the bottom-left
  (no vertical flip) and the top-right (no horizontal mirror);
* **rotation**: for each rotated page, the output's rendering is compared
  against the *source's* rendering on a coarse grid. A `/Rotate 90` page
  legitimately shows its markers in a different corner, so comparing
  against a fixed corner would be wrong; comparing source to output
  catches any *extra* rotation the pipeline introduces;
* page count and visible dimensions survive, including mixed sizes;
* cancellation leaves no destination file and no temporary file;
* the same input and settings produce **byte-identical** output;
* an existing destination is refused without overwrite, and honoured with;
* input equals output is refused.

## PDFium must be driven sequentially

PDFium is initialized once per process and this project drives it
sequentially. Exercising documents from several test threads at once
crashes inside the C++ library, so every PDFium-touching test takes a
shared mutex. Do not remove that lock to make tests faster.

## Manual end-to-end check

```bash
export MUSEION_PDFIUM_LIBRARY=/path/to/libpdfium.dylib
cargo run -q -p museion-binarize-core --example gen_fixtures -- /tmp/fx
cargo run -p museion-binarize-cli -- inspect /tmp/fx/mixed.pdf
cargo run -p museion-binarize-cli -- process /tmp/fx/mixed.pdf \
  --output /tmp/fx/out.pdf --method otsu --dpi 300 --validate render-all
cargo run -p museion-binarize-cli -- preview /tmp/fx/out.pdf \
  --page 1 --output /tmp/fx/page1.png --method manual --threshold 128
```

Processing the source and processing the reopened output should yield
byte-identical preview PNGs — that is the strongest available evidence
that the round trip is lossless.
