# Deterministic derived document and review data

M4 derives an AI-ready typed document from an MDP directory and its optional
`ocr/` extension. `mpdf-derived-document` 0.1 is an intermediate
representation, not Markdown: pages, regions, blocks, lines, words and chunks
carry stable references, page ids, master-space bboxes and structural paths.
OCR pixel boxes are scaled through the page's declared MDP master dimensions.

The derived manifest records source/package/OCR/revision digests and, for an
`all` export, the SHA-256 and relative path of every generated artifact. A
changed input makes a previous manifest stale; rebuilds write a temporary
directory and install it atomically. Existing destinations are refused unless
`--overwrite` is explicit. Bundle verification checks the manifest size, exact
artifact basename set, relative paths, regular-file status, byte lengths and
lowercase SHA-256 digests; extra or missing files are corrupt rather than
partially usable. Overwrite moves the complete old directory to a private
backup and restores it if installation fails.

```bash
mpdf export book.mdp --format all --output book-derived
mpdf review book.mdp --json
mpdf revision add book.mdp --target-ref word-... \
  --base-evidence-digest <page-evidence-sha256> --text corrected
mpdf revision list book.mdp --json
```

Human revisions affect only `effective_text`; immutable source text remains in
the derived record. AI suggestions are append-only and not applied by default.
A missing target or stale base digest fails closed. Existing outline/page-label/
typography/region evidence is retained as typed records, including original
outline source and each evidence bounds space; M4 does not infer a bookmark
tree. A present but incomplete `ocr/` directory is rejected (only a completely
absent extension means no-OCR input). HTML, hOCR and ALTO escape document text
and include page/bbox attributes. No exporter writes a PDF or calls a network
service.

M5 consumes this derived record as bookmark evidence. Bookmark candidates
retain both source/effective title and typed line/word references; only human
confirmed effective candidates are eligible for searchable-PDF outlines.
