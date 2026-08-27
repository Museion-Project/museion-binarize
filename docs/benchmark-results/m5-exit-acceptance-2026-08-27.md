# M5 exit acceptance — 2026-08-27

## Verdict

**The engineering conformance subset passes; the strict M5 product exit gate
is blocked. M6 must not start under the strict interpretation selected for
this audit.**

The machine-readable companion report is
[`m5-exit-acceptance-2026-08-27.json`](m5-exit-acceptance-2026-08-27.json).
This report separates candidate-generation quality from the correctness of a
PDF written after human confirmation. Combining those values would make the
accuracy result misleading.

## Reproduction

Run the automated engine matrix on macOS with the pinned local PDFium library:

```sh
MPDF_PDFIUM_LIBRARY='/path/to/libpdfium.dylib' \
  scripts/m5/check_reader_matrix.sh /tmp/mpdf-m5-reader-matrix
```

The script refuses to reuse an output directory. It emits the generated source
and searchable PDFs, ground truth, hashes, qpdf outline JSON, Poppler output,
Ghostscript renders, and PDFKit result. Generated acceptance artifacts remain
outside Git; only the report and reproduction tools are versioned.

## Conformance-fixture metrics

The fixture is one synthetic four-page PDF with rotations 0/90/180/270, three
evidence-backed candidates, polytonic Greek, one parent-child edge, an exact
printed-label TOC target, and one deliberately low-confidence candidate.

| Metric | Result | Interpretation |
|---|---:|---|
| Candidate target-page accuracy | 3/3 (100%) | Synthetic conformance only |
| Candidate title precision | 3/3 (100%) | Synthetic conformance only |
| Hierarchy edge-F1 | 1.00 | One true parent-child edge |
| Candidate coverage | 3/3 (100%) | All fixture truth entries proposed |
| Automatically confirmed coverage | 0/3 (0%) | Expected: M5 never auto-confirms |
| Zero-edit document success | 0/1 (0%) | One candidate is `needs_review` |
| Unsupported-evidence titles | 0 | No untraceable title generated |
| Unicode outline title round trip | 3/3 (100%) | Includes polytonic Greek |

These numbers cannot be compared with the research release thresholds. The
repository does not yet contain the required minimum 20 real documents with
two independent annotations and adjudication. In particular, the 0% automatic
confirmation coverage is an intentional M5 contract, not an accuracy failure:
only human-confirmed candidates are eligible for PDF write-back.

## Reader and engine matrix

| Reader or engine | Result | Evidence |
|---|---|---|
| PDFium 151.0.7920.0 | Pass | Page count/boxes/rotation, Unicode extraction, hierarchy, page/XYZ destinations, pixel-identical visible rendering |
| Apple PDFKit on macOS 27.0 | Pass | Open, Unicode extraction, hierarchy, page and destination coordinates; this is the engine used by Preview |
| qpdf 12.3.2 | Pass | No syntax/stream warnings; outline hierarchy and `/XYZ` destinations parsed |
| Poppler 25.08.0 | Pass | Page geometry/rotation and Unicode extraction with empty diagnostic stream |
| Ghostscript 10.07.1 | Pass | Four pages render; source and searchable render files are byte-identical |
| Preview UI | Partial | PDFKit engine passes; UI navigation was not manually exercised |
| Adobe Acrobat 25.001.20476 | Blocked | Installed, but AppleEvent automation timed out; no manual result claimed |
| PDF.js | Not run | No pinned/vendored test runtime |
| Foxit/iOS | Not run | Reader/device unavailable |

The audit found two issues that PDFium alone did not expose and fixed both:
synthetic pages lacked explicit empty `/Resources` dictionaries, and the
CIDFont descendant wrote `/Registry` and `/Ordering` as PDF names instead of
required PDF strings. qpdf and Poppler now complete without repair or font
diagnostics.

## Work required to clear the strict gate

1. Build and license a versioned minimum 20-document real corpus spanning
   native outline, digital TOC, scanned TOC, and safe-refusal documents.
2. Obtain two independent annotations per document and adjudicate titles,
   hierarchy, physical target page, target y coordinate, and printed label.
3. Freeze dev/test splits and thresholds, then report target-page accuracy,
   title precision, edge-F1, coverage, zero-edit success, and hallucinations.
4. Complete UI-level checks in Acrobat, Preview, a pinned PDF.js runtime, and
   Foxit/iOS, recording versions and results.

Until those items are complete, the truthful result is: M5 implementation is
merged and its automated conformance matrix passes, but its strict product
exit condition is not yet satisfied.
