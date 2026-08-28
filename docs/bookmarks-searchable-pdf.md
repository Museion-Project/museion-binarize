# Evidence bookmarks and searchable PDF

## What the automatic path does (bookmarks v2)

`mpdf bookmark auto` — and the desktop application's single "Add bookmarks
automatically" button — compile a table of contents from the document's own
evidence and produce exactly one of three outcomes:

1. **`existing_outline`**: the source PDF already has a valid outline. It is
   preserved exactly (title, level, parent, physical target page) and never
   mixed with inferred entries. No OCR is required.
2. **`toc_aligned`**: there is no usable native outline, but the document has
   a printed contents list. The engine finds the contents pages in the front
   matter, parses their entries (single and double column, dot leaders,
   wrapped titles, arabic and roman page labels), locates the matching
   headings in the body text, solves printed-label to physical-page mapping
   in piecewise-constant segments per numbering family, and confirms only the
   entries where title, page mapping, numbering, layout, OCR confidence, and
   a monotone position all agree.
3. **`safe_refusal`**: no contents list, incomplete OCR, or evidence too
   ambiguous. Nothing is written to a PDF and no title is invented. This is a
   normal result with an explanation, not an error.

**This feature depends on either a valid native outline or a complete OCR run
containing a recognizable printed contents list.** It does not claim to
produce a correct table of contents for an arbitrary PDF, and it will not have
a model read the book and compose one. Where there is no printed contents
list, heading-like lines may be proposed for human review; they are never
confirmed automatically.

Every automatic decision is an integer score out of 10,000 across six capped
components (title match, printed-page mapping, numbering/level, heading
layout, OCR quality, sequence/uniqueness), with frozen thresholds bound into
the snapshot's `rule_config_digest`. `bookmarks/generation-report.json`
records the scanned front-page window, the detected contents pages and their
signals, the mapping segments, the reason-code counts, and any resource
truncation — without copying document text or any raw provider artifact.

Local (M3) and consented API (M6) OCR are the same typed evidence to this
engine: identical typed records yield identical titles, levels, targets,
statuses, and scores. Provider identity appears only in the report's
provenance summary and the input digests, never in a branch.

## Statuses and schema versions

`auto_confirmed` is produced only by the deterministic gate; `confirmed` is
produced only by a human review. Both reach the PDF outline; `proposed`,
`needs_review`, `skipped`, and `rejected` do not. A human confirm, edit, or
reparent of an automatic entry makes it `confirmed` while retaining its
automatic score, reason, rule version, and rule-config digest.

New generations write `mpdf-bookmarks` **0.2**
(`schemas/mpdf-bookmarks-0.2.schema.json`) plus a generation report
(`schemas/mpdf-bookmark-generation-report-0.1.schema.json`). Existing **0.1**
snapshots and review logs stay readable, listable, reviewable, and buildable
exactly as they are; nothing migrates them in place, and a 0.1 file carrying a
0.2 field or status is rejected rather than reinterpreted. Regenerating over a
non-empty review log is refused with an explanation.

## Evidence contract (M5, unchanged)

M5 stores deterministic bookmark candidates under `bookmarks/` in the MDP.
Candidates are proposals backed by typed references to MDP or DerivedDocument
evidence. Human confirm, reject, title edit, and reparent operations are
append-only. The original candidate title, hierarchy, evidence, confidence,
and generator trace remain available after every review operation.

Only human-confirmed effective candidates are written to a PDF outline.
Unreviewed and low-confidence candidates remain visible in the CLI and desktop
review queue but cannot silently become document navigation.

When an MDP is created directly from a PDF session, actionable native outline
items are imported as `source-pdf` evidence in source tree order. Items without
a resolvable destination are hierarchy containers only: they are not emitted
as candidates, and actionable descendants attach to the nearest actionable
ancestor. Invalid page destinations fail the package build closed. Source
titles remain byte-for-byte equivalent to PDFium's Unicode result; a trimmed
effective title is used only for display and review.

Searchable output is always a new file unless `--overwrite` explicitly permits
replacing an existing output. The source PDF must be the exact source recorded
by the MDP. Existing page content and image streams are preserved, while an
embedded Unicode font, invisible per-word text, and confirmed outline entries
are appended. All word and destination coordinates are derived from the MDP's
declared affine transform and the source page's box and rotation.

The build fails closed for a mismatched source, partial OCR, stale derived or
bookmark state, unresolved evidence, invalid/cyclic review hierarchy, unsafe
paths, unsupported geometry, missing font data, cancellation, or PDFium reopen
validation failure. Temporary output is created beside the destination and is
removed on failure.

M5 is offline. It does not call an LLM, upload a document, or download a model
or font. AI generator provenance is reserved for a later milestone and cannot
change the effective bookmark tree in M5.

The v2 engine keeps that property: `mpdf-core` makes no network request, loads
no model, and treats OCR text strictly as untrusted document content — never
as an instruction, path, URL, command, or provider selector. An AI-suggested
M4 revision is still never applied automatically.

## Output verification

After the derivative is written to a same-directory temporary file, two
independent checks run before it is installed: PDFium reopens it and re-reads
page count, page geometry, and rotation; lopdf walks the written `/Outlines`
tree and compares its titles, nesting depth, and destination pages against the
effective bookmark tree. Only then is the file atomically moved into place,
and the source bytes are re-read to confirm they did not change. Cancellation
or any failure leaves no output and no temporary file.
