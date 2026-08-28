# ADR 0009: deterministic automatic table-of-contents compilation

Status: accepted for the automatic bookmark feature (bookmarks v2), on the
M6 baseline.

## Context

M5 froze an evidence-backed bookmark contract: immutable candidates, typed
evidence references, an append-only human review log, and a searchable-PDF
writer that only materializes what a person confirmed. Its generator,
however, leaned on MDP `region_evidence`, `printed_page_label`, and
`typography_evidence` fields that an ordinary M3/M6 OCR run leaves empty. In
practice a user who had run OCR still got no bookmarks: the path from OCR
evidence to a written outline was never closed.

M6 then added a consented remote OCR provider whose validated result is
installed into the very same `ocr/` extension. Any automatic navigation
feature must therefore treat local and API OCR as one kind of typed
evidence, not as two code paths.

## Decision

### One engine, three business outcomes

`mpdf-core::bookmarks` compiles bookmarks from evidence in strict priority
order and reports exactly one of three outcomes:

1. **`existing_outline`** — the source PDF's own validated outline is
   preserved exactly (title, level, parent, physical target page) and is
   never mixed with, or overridden by, inferred entries. This mode needs no
   OCR at all.
2. **`toc_aligned`** — no usable native outline, but the document has a
   printed contents list. The engine detects the contents pages, parses
   their entries, finds the corresponding headings in the body text, solves
   printed-label to physical-page mapping, and confirms only entries with
   full multi-signal agreement.
3. **`safe_refusal`** — no contents list, incomplete OCR, or evidence too
   ambiguous. The run returns an explained report, writes no PDF, and
   invents no title. A refusal is a normal business result, not an error.

There is no fourth mode in which a model reads the book and composes a
table of contents, and no mode in which arbitrary large or bold text becomes
a reliable bookmark. Without a printed contents list, heading-like lines may
be proposed for human review only; they can never be confirmed
automatically.

### Provider-neutral evidence, no network in core

The engine consumes typed `OcrRun` records — `page_index`, route,
block/line/word structure, bounding boxes, reading order, and confidence —
plus the M4 `DerivedDocument` overlay that carries human-revised effective
text in master coordinates. Whether those records came from the M3 local
route or from `remote_api::install_remote_ocr_result` changes nothing about
the decisions: provider `engine`/`model`/`version`/`execution_location`
enter only the report's deduplicated provenance summary and the input
digests. `provider_raw_artifact` is never parsed; the algorithm consumes
only schema-validated typed records, and OCR text is always untrusted
document content, never an instruction, path, URL, or command.

`OcrRoute` does carry one structural consequence: a `NativeText` page's line
and word boxes are approximate, so they may not drive a multi-column or
font-size decision. That is a property of the evidence, not of a vendor.

### `auto_confirmed` is not `confirmed`

Bookmark snapshot 0.2 adds a status that only the frozen deterministic gate
can produce. `confirmed` continues to mean "a person decided this". Both are
written to a PDF outline; `proposed`, `needs_review`, `skipped`, and
`rejected` are not. Confirming, editing, or reparenting an automatic entry
makes it `confirmed` — a human decision — while its automatic score, reason,
rule version, and rule-config digest are retained for audit.

### Frozen integer scoring

Every decision is an integer on a 0–10,000 scale, split into six capped
components: title match (4,000), printed-page mapping (2,000), numbering and
level agreement (1,000), body heading layout (1,000), OCR quality (1,000),
and monotone sequence/uniqueness (1,000). Automatic confirmation requires
9,200 total, 3,600 title, ≥0.80 minimum word confidence on both sides, a
600-point margin over the runner-up, real body-heading line and bbox
evidence, a satisfied printed-page residual, an unambiguous level, and a
monotone position. Anything else is `needs_review` or `skipped`, with reason
codes. Floating point never decides anything; the public `confidence` is the
total divided by 10,000.

All thresholds live in `AutoBookmarkConfig`, are serialized into a
`rule_config_digest`, and are locked by unit tests. Recalibrating them
changes the rule version and therefore every generation digest — old
automatic decisions are invalidated rather than silently reinterpreted.

### Printed pages map piecewise, and targets move forward only

Printed labels are not offset by one document-wide constant. Anchors are
grouped by numbering family (arabic, roman) and segmented by a deterministic
dynamic program over the observed offsets; opening a segment costs more than
one anchor disagreeing with it and less than two, so a stray anchor cannot
fork the mapping while a real inserted plate or numbering restart does. A
second dynamic program then chooses one target per entry such that targets
never move backwards through the document; an entry that cannot be placed
monotonically is left unassigned rather than reordered.

### PDF write-back and verification

Write-back keeps ADR 0007's model — new file only, source digest bound, no
clobber, same-directory temporary file, atomic install, unchanged source
bytes — and extends verification: after writing, PDFium reopens the file to
re-check page count, geometry, and rotation, and lopdf independently walks
the written `/Outlines` tree to compare its titles, nesting, and destination
pages against the effective tree. A documented claim is never accepted where
a check can be made. Destinations prefer the top of the body heading's
master bbox; existing-outline entries keep their original target semantics.

### Compatibility

0.1 snapshots and review logs remain readable, listable, reviewable, and
buildable exactly as they are. Nothing migrates them, and no 0.1 record can
acquire an automatic status: a 0.1 file carrying a 0.2 field or status is
rejected. New generations always write 0.2, alongside a separately schema'd
`bookmarks/generation-report.json`. Regeneration over a non-empty review log
is refused with an explanation, never performed silently.

## Consequences

- A user with a complete OCR run and a recognizable printed contents list
  gets reliable bookmarks from one command or one button, without knowing
  anything about providers, thresholds, page offsets, or schemas.
- A document without that evidence gets an honest refusal instead of a
  plausible-looking but unreliable outline.
- The engine's cost is bounded: contents entries are matched through an
  inverted index with a 32-target shortlist per entry, never against every
  line in the book, and the report states how much index work was actually
  performed.
- Better accuracy on hard documents now requires a corpus-calibrated rule
  version, not a quiet threshold change: the digests make that explicit.
