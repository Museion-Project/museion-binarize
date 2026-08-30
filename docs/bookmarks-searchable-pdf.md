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

## Provider-neutral evidence

Bookmarks are compiled from `OcrPage` records and never from a provider. It
does not matter whether a page was recognized locally, by a cloud model under
the user's own key, or by a brokered service: the engine sees the same
block/line/word tree with the same measured rectangles, applies the same
rules, and refuses in the same cases.

Two consequences follow, and both are enforced by tests:

- **A cloud model never decides a bookmark's target page.** It supplies
  validated OCR material; the contents parser, the printed-page mapping, the
  body-heading verification and the safe-refusal blockers are unchanged and
  provider-neutral. A page whose target cannot be mapped, whose sequence is
  non-monotonic, or whose body carries no heading evidence is refused exactly
  as before — a cloud provider cannot buy its way past a blocker.
- **Changing the provider invalidates the candidates.** Provider mode, model
  and pinned version, prompt digest, geometry source, alignment version and
  any per-page fallback are recorded in the page's provenance, so they are
  inside the `ocr_digest` the generation report carries. Re-running the same
  document under a different mode, a different prompt, or with different pages
  falling back produces a different digest, and the previous snapshot is stale
  rather than silently reused.

See [`ocr-providers.md`](ocr-providers.md).

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

## Automatic confirmation: two routes (rule 0.4)

The **numeric gate** is unchanged from 0.2: total ≥ 9,200, title ≥ 3,600,
runner-up margin ≥ 600, every word confidence ≥ 0.80, and every structural
condition clear.

Rule 0.3 adds a second, narrower **structural consensus** route. It exists
because the numeric gate was refusing entries whose placement was
demonstrably right. On a real 160-page volume it auto-confirmed 1 of 14 valid
candidates: the rest were blocked by a total a few hundred points short, or by
OCR word confidence on a page the engine had nonetheless located correctly and
uniquely.

The consensus route is not the numeric gate with smaller numbers. It requires
evidence the numeric gate never consults — a printed-to-physical page mapping
that at least `consensus_min_anchors` other shortlist entries independently
agree with — and it holds every safety condition at full strength:

- printed-page residual exactly `0` (not merely inside tolerance);
- corroboration from **independent exact anchors**: other entries whose own
  observed printed-to-physical offset equals this entry's segment offset. Not
  the segment's member count — the mapping solver deliberately keeps
  disagreeing anchors inside a run, paying a mismatch penalty, so that one
  stray heading cannot fork the mapping, and counting those would count the
  anchors it overruled. The entry being judged is excluded, so nothing
  corroborates itself. On the measured volume the single segment has 14
  members, 12 exact and 2 disagreeing, giving any member 11 independent
  exact anchors;
- runner-up margin ≥ the *same* 600 the numeric gate demands, because target
  uniqueness is a safety property and is not traded;
- body heading present, sequence monotone, level unambiguous, primary key
  match, no repeated header/footer, measured (not approximate) geometry, no
  resource truncation.

Only three blockers may be outweighed — `total_score_below_gate`,
`title_score_below_gate`, `low_word_confidence` — and only down to floors
(`consensus_min_total` 8,000, `consensus_min_title` 3,200,
`consensus_min_word_confidence_permille` 300). OCR uncertainty is
**compensated, never ignored**: no amount of agreement confirms text the
engine could not read.

Every consensus confirmation records what carried it and what was
compensated: `printed_page_mapping_consensus`,
`mapping_exact_independent_anchors_<n>`, `mapping_disagreeing_anchors_<m>`,
`unique_target_margin_clear`, `monotonic_target`, `body_heading_present`, and
one `compensated_<blocker>` code per outweighed blocker.

Measured on that volume: 1 → 9 auto-confirmed, 12 → 4 needs review, skipped
unchanged at 11, and **all 9 targets verified correct against the written PDF
with zero false confirmations**. The four still needing review are honest
refusals — a 216-point margin, a 0.04 TOC word confidence, and two totals
below the consensus floor — and were left as review rather than reached for.

Changing any of these values changes `rule_version` and the rule-config
digest, which invalidates existing automatic decisions instead of silently
reinterpreting them.

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
validation failure. The writer receives an explicit `OutputWriteStrategy`:
ordinary CLI and desktop builds stage beside the destination and atomically
rename after validation; the MAS build validates in container/system temp and
writes directly to the exact NSSavePanel-authorized destination.

M5 is offline. It does not call an LLM, upload a document, or download a model
or font. AI generator provenance is reserved for a later milestone and cannot
change the effective bookmark tree in M5.

The v2 engine keeps that property: `mpdf-core` makes no network request, loads
no model, and treats OCR text strictly as untrusted document content — never
as an instruction, path, URL, command, or provider selector. An AI-suggested
M4 revision is still never applied automatically.

## Output verification

After the derivative is written to a strategy-specific temporary file, two
independent checks run before it is installed: PDFium reopens it and re-reads
page count, page geometry, and rotation; lopdf walks the written `/Outlines`
tree and compares its titles, nesting depth, and destination pages against the
effective bookmark tree. Source bytes are checked again immediately before the
final commit. Cancellation or validation failure leaves an existing
destination untouched and removes the temporary file. Candidates and report
are each fully staged and safely replaced, then loaded and checked as one
matching pair; if the subsequent PDF operation fails, the old pair is
restored byte-for-byte. A process crash between the two replacements can
expose a half-pair, which the next run rejects closed rather than repairing or
guessing.
