# Research Query: one-click PDF OCR applications with machine-readable artifacts

**Date:** 2026-08-26
**Window:** 2026-02-26 through 2026-08-26
**Status:** Complete for product/architecture scoping

## Scope correction

The target is end-user software that accepts a PDF, runs OCR, and returns a machine-readable artifact.
Large OCR/document parsing frameworks are dependencies, not peer products.

## Search strategy

Started from `Ivan666jjj/guji-tools`, `Ivan666jjj/xiaohuan-tools`, and `1ampa55ag3/octo-ocr`.
Expanded through GitHub `pdf-ocr`, `searchable-pdf`, `pdf-to-markdown`, and `paddleocr-vl` topics plus
combinations of Web UI, local, desktop, FastAPI, Gradio, Streamlit, Markdown, and JSON. Reviewed README,
repository tree, current commit, license, and the core pipeline/server/output/job source of ten repositories.

## Main findings

1. `octo-ocr` is the closest peer: it has a versioned page/project JSON with geometry and editing evidence.
2. `Folio-OCR` has the strongest proofreading UX; `pdf-converter` has the strongest durable-job design;
   `MDFlux` is the strongest Tauri + sidecar packaging reference.
3. Most products expose Markdown or vendor JSON, not a provider-neutral, versioned document IR.
4. Page-level checkpoint/restart, explicit coordinate spaces, source digests, and revision provenance are
   rarely implemented together.
5. No reviewed product ships TOC/heading/printed-page alignment evidence as a first-class artifact for
   automatic bookmarks.

## Recommendation

Build a local-first OCR workbench around a page-sharded MDP package. Probe each page, preserve native text,
OCR only pages that need it, commit each page atomically, and derive Markdown/searchable PDF/chunks/bookmark
evidence from one canonical IR. Keep the OCR provider behind a versioned sidecar contract; add API providers
after the local job and artifact semantics are stable.

Full synthesis:
[`docs/one-click-pdf-ocr-app-research.zh-CN.md`](../../docs/one-click-pdf-ocr-app-research.zh-CN.md)

Revised architecture:
[`docs/ocr-ai-ready-layer.zh-CN.md`](../../docs/ocr-ai-ready-layer.zh-CN.md)

## Limitations

This was a source/architecture review, not an OCR accuracy benchmark. Models were not installed and the
projects were not run against a common corpus. Performance and accuracy claims in READMEs remain unverified.
