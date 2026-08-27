# ADR 0005: Local OCR routing and evidence extension

Status: accepted for Milestone 3

## Decision

M3 routes each page through PDFium's native text layer first. A page is sent
to OCR only when that layer is empty, too short, or contains replacement or
unexpected control characters. The page is rendered at a canonical 300 DPI
and released after the provider call; no whole-document raster is retained.

OCR output is a typed extension beneath `ocr/`: `OcrPage` contains a route,
dimensions, blocks, lines, words, typed optional revisions, raw provider
artifact, original text and a whitespace-normalized text value. Bounding
boxes, confidence, text bytes, and provider output have explicit limits. The base MDP 0.1 package remains
readable by consumers that do not understand the extension.

The first adapter is a direct argv-only RapidOCR/ONNX sidecar runner. It must
be configured with an executable and model directory; it never invokes a
shell, contacts a network, or downloads models. The deterministic Reference
provider is used for fixtures and local development. Missing model/executable
state is reported as `provider_unavailable`, and a partial run is not marked
successful.

The M2 SQLite job store is used for per-page checkpoint records. The CLI
ensures a source/provider-matching job, writes each typed page and raw
artifact before its checkpoint, and resumes only after validating both the
database record and on-disk file. Cancellation is checked between pages and
retains committed pages. The desktop currently exposes durable status,
cancel, and page-error queries; CLI remains the controller for starting work.
Provider failures use the M2 retryable transition. A new job id may adopt
valid page records left in the same source-matching package after cancellation
or a crash, while malformed/orphan page files fail closed and raw-only orphans
are safely verified or replaced. RapidOCR job identity includes the bytes of
all three provisioned ONNX model files.

## Consequences

Native born-digital pages avoid unnecessary OCR and preserve the original
Unicode string in the evidence record. Their line/word boxes are explicitly
page-relative approximations until the PDFium seam exposes stable glyph
rectangles; they are not presented as measured glyph geometry. Scanned pages remain searchable only
through provider output; a local RapidOCR installation is intentionally an
operator concern and CI does not download models. OCR records are additive,
so existing binarization and MDP source/page validation continue to work when
OCR is unavailable.
