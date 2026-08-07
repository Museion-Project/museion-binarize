# Testing the PDF pipeline

## Two tiers

**Ordinary tests** (`cargo test --workspace`) need no PDFium. They cover
page geometry, the image-processing orchestrator, bit packing, CCITT
round-trips, PDF dictionary construction, destination safety, atomic
persistence, and CLI argument validation. They must always pass on a clean
machine.

**PDFium integration tests** (`crates/museion-binarize-core/tests/pdf_pipeline.rs`)
need a real PDFium library, so every one of them is marked `#[ignore]`.
An ordinary run reports them as **ignored** — never as passed:

```text
Running tests/pdf_pipeline.rs
test result: ok. 0 passed; 0 failed; 13 ignored
```

Run them explicitly, with a library:

```bash
MUSEION_PDFIUM_LIBRARY=/absolute/path/to/libpdfium.dylib \
  cargo test --test pdf_pipeline -- --ignored
```

If `MUSEION_PDFIUM_LIBRARY` is unset or does not point at a file, these
tests **fail** with a message telling you how to fix it. They cannot pass
without exercising PDFium.

> **Why `#[ignore]` and not an early return.** Rust's built-in test
> harness has no dynamic "skip" result: a test that returns early is
> recorded as **passed**. An earlier version of this suite returned early
> when no library was configured, so ordinary CI reported twelve passing
> end-to-end tests that had never run. Ignored tests are counted and
> displayed separately, which is the only truthful way to say "not
> verified here".

An ignored run is *not* a passing run. Before declaring PDF-pipeline work
done, run the provisioned command above and report both numbers
separately.

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
* **rotation**, asserted against an *independent* oracle. Expected values
  come from the fixture's own constants (`A4_PORTRAIT`, the marker sizes)
  and the definition of a point (72 per inch) — never from
  `PageGeometry`, `points_to_pixels`, or any other production helper, so
  the test can detect an error in those helpers instead of inheriting it:
  * an A4 page with `/Rotate 90` or `270` must report a **visible**
    842x595 pt and rasterize to 3508x2479 px at 300 DPI;
  * `/Rotate 0` and `180` must stay 595x842 pt;
  * the source `/Rotate` must survive as informational metadata;
  * the fixture's **square** markers must still be square after
    conversion (connected-component segmentation, aspect within 6% and
    size within 10% of the value implied by the fixture constants), and
    must remain diagonally opposite — a transposed page turns them into
    rectangles, a mirrored page moves them off the diagonal.

  > An earlier version compared the source's rendering against the
  > output's. Both sides were computed by the same production code, so the
  > test agreed with itself while both were wrong, and missed a real
  > double-rotation defect. Validation may compare source against output,
  > but the integration tests must establish independently that the source
  > interpretation is correct in the first place.
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

## Continuous integration

CI runs `cargo test --workspace` on GitHub-hosted runners, which have no
PDFium, so the integration tests above are reported as **ignored** there.
**CI does not verify the end-to-end PDF pipeline.**

There is deliberately no PDFium job in the workflow: a `workflow_dispatch`
job that cannot obtain a library would present a button in the Actions UI
that can only fail, and adding an automatic binary download is exactly
what `adr/0001-pdfium-runtime-binding.md` rules out. The end-to-end tests
are run on a provisioned local macOS environment with the command above.
Automatic, checksum-verified provisioning from
`third_party/pdfium/manifest.toml` is deferred to the packaging/release
milestone.

## Manual end-to-end check

```bash
export MUSEION_PDFIUM_LIBRARY=/absolute/path/to/libpdfium.dylib
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
