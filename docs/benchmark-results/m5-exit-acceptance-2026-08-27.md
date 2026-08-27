# M5 exit acceptance — 2026-08-27

## Verdict

**The engineering conformance and real native-outline subsets pass; the strict
M5 product exit gate is blocked. M6 must not start under the strict
interpretation selected for this audit.**

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
native-outline corpus below has 20 documents. A separate 12-document
single-authoritative-annotator pack now covers the three missing evidence
classes, but its human tables are not filled yet. In particular, the
0% automatic confirmation coverage is an intentional M5 contract, not an
accuracy failure: only human-confirmed candidates are eligible for PDF
write-back.

## Real native-outline corpus

The versioned manifest selects 20 PDFs (4,869 pages) from Translation Agent
2's existing `input/` tree without copying copyrighted source bytes into Git.
Source SHA-256 values pin the inputs. qpdf independently supplies an
actionable-outline semantic digest; PDFium then feeds the MDP and bookmark
generator.

| Metric | Result |
|---|---:|
| Documents exercised | 20 |
| Accepted documents | 19 |
| Safely rejected invalid-outline documents | 1 |
| Accepted outline entries | 574 |
| Exact source titles preserved | 574/574 (100%) |
| Exact hierarchy levels preserved | 574/574 (100%) |
| Exact physical target pages preserved | 574/574 (100%) |
| Unresolved evidence | 0 |
| Automatically confirmed | 0 |
| Deterministic repeat generations | 19/19 (100%) |

The rejected PDF contains a bookmark destination for which PDFium cannot
resolve a page index; the MDP build fails closed instead of inventing a
target. The machine-readable result is
[`m5-real-outline-corpus-2026-08-27.json`](m5-real-outline-corpus-2026-08-27.json),
and the reproduction contract is documented with the
[manifest](../../test-data/benchmark/m5-real-outline-v1/README.md).

This is a real-document preservation benchmark, not the full product-quality
benchmark: all accepted documents already have native outlines. It does not
measure title inference from digital/scanned TOCs, target-y accuracy, or
no-outline safe refusal.

## Single-authoritative-annotator pack

The product owner chose one authoritative annotator and will perform the
Acrobat, Preview, and iOS checks. The repository therefore does not claim an
inter-annotator agreement score or adjudication. The frozen pack adds 12
different PDFs from the same Translation Agent 2 input tree: four digital-TOC,
four scanned-TOC, and four preliminary safe-refusal cases (2,149 pages total).
Together with the existing-outline corpus, M5 now has 32 distinct real-PDF
cases; the original 20-document preservation result is unchanged.

Source PDF bytes remain external. The manifest pins relative paths, hashes,
page counts, and non-authoritative TOC hints. `human_acceptance.py prepare`
verifies those pins and atomically creates a no-clobber working pack. Its
validator requires one annotator, complete document decisions, traceable
bookmark coordinates and hierarchy, genuine safe refusals, and passing manual
results from all three target readers. Empty templates fail closed.

The local working pack is intentionally outside Git. Until the owner fills it
and `validation-report.json` reports `pass`, these 12 documents are an
acceptance assignment, not a completed gold-standard result.

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
| PDF.js 5.4.624 | Pass | Pinned automated runtime: page count, 0/90/180/270 rotation, Unicode text, hierarchy, page and `/XYZ` destinations |
| Foxit PDF Editor 2026.1.0.70169 | Pass | Manual UI: open, four pages, Unicode outline hierarchy, child bookmark navigation to page 2, polytonic Greek text search |
| iOS | Not run | Device/runtime unavailable |

The audit found two issues that PDFium alone did not expose and fixed both:
synthetic pages lacked explicit empty `/Resources` dictionaries, and the
CIDFont descendant wrote `/Registry` and `/Ordering` as PDF names instead of
required PDF strings. qpdf and Poppler now complete without repair or font
diagnostics.

## Work required to clear the strict gate

1. Complete the single authoritative annotation of titles, hierarchy,
   physical target page, target y coordinate, printed label, and safe refusals
   in the frozen 12-document pack.
2. Freeze dev/test splits and thresholds, then report target-page accuracy,
   title precision, edge-F1, coverage, zero-edit success, and hallucinations.
3. Complete UI-level checks in Acrobat, Preview, and iOS. PDF.js and Foxit are
   now covered with recorded versions and passing evidence.

Until those items are complete, the truthful result is: M5 implementation is
merged and its automated conformance matrix passes, but its strict product
exit condition is not yet satisfied.
