# M5 real native-outline corpus manifest

This manifest identifies 20 PDFs already present under the Translation Agent
2 project's `input/` directory. The PDF bytes are not copied into this
repository. Each entry pins its relative path, SHA-256, page count, actionable
outline count, and an independent qpdf semantic-outline digest.

The semantic digest is the SHA-256 of newline-delimited JSON tuples
`[level, physical_page_index, title]` in source tree order. Targetless
container nodes do not count as actionable entries and do not increase the
actionable level. Only trailing whitespace and NUL padding are ignored for the
cross-reader digest because PDFium and qpdf decode malformed padding
differently. MDP evidence still preserves PDFium's complete source title.

Run the corpus gate with:

```sh
MPDF_PDFIUM_LIBRARY='/path/to/libpdfium.dylib' \
MPDF_M5_REAL_CORPUS_ROOT='/path/to/translation-agent-2/input' \
MPDF_M5_REAL_REPORT='/tmp/mpdf-m5-real-outline-report.json' \
cargo test --offline -p mpdf-core --test m5_real_outline_corpus -- --nocapture
```

The test validates source hashes and safe paths before opening a document. One
manifest entry intentionally records an invalid outline destination and must
be rejected. This is a fail-closed corpus case, not a successful mapping.

This corpus measures preservation of existing native outlines. It is not a
substitute for independently annotated digital-TOC, scanned-TOC, or
no-outline inference benchmarks.
