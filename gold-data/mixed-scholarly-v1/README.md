# Mixed scholarly closed-world OCR gold

## Current Dev reference QA (2026-09-06)

The 25 non-frozen pages have a new visual preannotation candidate in
`candidates/visual-dev-qa-2026-09-06/`. Its `README.md` records the review scope,
source-preservation policy and page-specific QA notes; `open-workbench.command`
opens exactly those candidates. Original page records and the 16 frozen pages
are unchanged. AI preannotation never creates human verification.

The annotator now accepts `--review-scope ocr-geometry` for the approved Dev
reference scope: geometry, transcription, reading order and exhaustive visible
line coverage. Completion leaves the record as a draft and writes a separate
hash-bound `dev-qa-receipts/` receipt. Full structure/typography semantics remain
deferred under `intent.md` §8.1. The default `full-gold` mode retains strict Gold
validation. Failed completion preserves edits without setting a false completed
state, and saving refuses to overwrite a record changed externally since load.

## Full Gold annotation

This is the first-party evaluation set for pages that may mix German, French,
English, Latin, and polytonic Greek. It starts from blank human annotation; no
OCR provider output is treated as gold or silently used as annotation truth.

Launch the local annotator and select exported page images:

```bash
python3 scripts/ocr/gold/annotate_closed_world_gold.py
```

Or pass images explicitly:

```bash
python3 scripts/ocr/gold/annotate_closed_world_gold.py \
  --images /path/to/page-001.png /path/to/page-002.png
```

Draft JSON is written under `pages/` by default. A draft may contain zero lines
while annotation is beginning. A verified page must contain every visible text
line with an original-image pixel box, exact NFC transcription, language,
content class, and explicit reading order. The page-level exhaustive-coverage
gate must also pass.

The workbench exposes the two independent parts of that contract as sequential
modules (schema 1.2):

1. **Geometry + structure module:** review/add/delete/resize line boxes,
   establish exact reading order, and label paragraph role, leaf, column, and
   canonical marginal references. The module is complete only when both
   `geometry_status` and `structure_status` are `human_verified`.
2. **Character + typography module:** correct exact NFC transcription,
   language, content class, and footnote `note_id`; select character ranges to
   retain italics, bold, superscript, subscript, underline, small caps, and
   semantic links such as a superscript marker to its footnote. The module is
   complete only when both `transcription_status` and `typography_status` are
   `human_verified`.

Blue boxes have verified geometry/structure only. Green boxes have both
modules verified. Resizing or reordering invalidates geometry; editing
structural fields invalidates structure; editing text or labels invalidates
transcription and typography. A page becomes gold only when every line is green
and all three page-level exhaustive gates (visible lines, structure, and
typography) pass.

Strict verification also checks the footnote graph in both directions. A note
whose first `footnote` line on the page has `paragraph_role: "start"` must have
at least one `footnote_marker` from a non-note line. A page containing only a
cross-page `continuation` does not start a new note and therefore does not need
a repeated marker. A genuinely markerless note must be declared explicitly in
the optional page coverage field:

```json
"footnote_marker_exceptions": [
  {"note_id": "page-note-editorial", "reason": "Unnumbered editorial note in the source"}
]
```

Exception reasons must be non-empty. Unknown, duplicate, or stale exceptions
fail strict validation. A superscript number introducing a footnote definition
is typography, not a reference marker: retain `styles: ["superscript"]` with
`semantic_role: "none"` and `target_id: null`; it must not self-link to the
line's own `note_id`. A real marker whose definition is outside the current
page remains a `footnote_marker` and declares the absent target rather than
being weakened to `semantic_role: "other"`:

```json
"external_footnote_targets": [
  {"target_id": "next-page-note-45", "reason": "Definition is outside this page image"}
]
```

Strict Gold also rejects numeric superscripts left as `semantic_role: "other"`;
they must be classified as a linked marker or as non-reference typography.

CGPG scaffolding remains available only through the explicit legacy mode:

```bash
python3 scripts/ocr/gold/annotate_closed_world_gold.py \
  --corpus-root /path/to/cgpg/data
```

Existing CGPG drafts use schema 1.0 and are neither migrated nor overwritten by
the mixed-gold tool. New first-party drafts use schema 1.2. Opening a first-party
1.1 draft in the workbench upgrades it to 1.2 and deliberately reopens the new
structure and typography evidence for review.

## Historical-scan geometry selection (2026-08-30)

The local 11-page Brisson / Ueberweg / Burnet selection is recorded in
`selection-2026-08-30.json`. It contains immutable source and rendered-image
digests, PDF page indices, 300-DPI dimensions, and the feature rationale for
each page. The selected images deliberately cover French/German/polytonic
Greek/Latin mixtures, two-page gutters, two-column bibliography, bleed-through,
small apparatus and historical type.

The selection includes a Brisson contents leaf for hierarchy, indentation,
dot-leader, and destination-page annotation. The verified Brisson PDF page 30
spread is retained; PDF pages 60 and 100 were replaced by denser mixed-script
pages 110 and 170. The malformed/folded Ueberweg scan representing printed page
148 (PDF page 150) was excluded and replaced by the normal two-column
bibliography page 143.

The selection manifest remains
`candidate_drafts_ready_for_human_review_not_frozen` until every page has
passed the workbench's exhaustive page-coverage gate. The orange Tesseract
scaffolding (PSM 3 generally, PSM 6 for the single-column Burnet pages) is useful
precisely because it exposes material misses on the hardest pages; it is not gold. Rendered images
and page records are local evaluation material; do not publish or redistribute
them merely because the selection manifest is versioned.

On 2026-08-31 the 11 page records received a visual-model preannotation pass.
It removed obvious OCR artefact boxes, restored the missing Kramer equation
line, classified page furniture/body/notes/apparatus/bibliography, populated
paragraph and column structure, linked obvious footnote markers, and encoded
the contents hierarchy and printed-page destinations. This pass remains
`candidate_unverified`: it deliberately reset every page-level coverage gate
and every line's four review statuses. Only the subsequent workbench review may
promote those fields to human gold. The page-specific bootstrap decisions are
recorded in `scripts/ocr/gold/bootstrap_visual_preannotations.py`.

## Second visual batch (2026-08-31)

`selection-2026-08-31.json` adds 30 local candidate pages: four facing-page
spreads from the Budé *Sophiste*, five OSAP pages, five *Phronesis* pages,
Ueberweg/Krämer PDF pages 3–4, eight pages from Primavesi's
*Aristoteles und Speusipp über die Platonische Zwei-Elementen-Lehre*, and six
high-density pages from Menn's *The Aim and the Argument of Aristotle's
Metaphysics*. This batch covers French/Greek critical text and apparatus,
classic journal typography, two-column German scanning, modern German
text-critical discussion, parallel quotations, and unusually long scholarly
footnotes.

All 30 records have received a visual-model preannotation pass, with native PDF
font/baseline evidence used where available and multilingual Tesseract used only
as scaffolding for the two scanned sources. Native italics, bold, and
superscript candidates are retained as inline spans. Obvious page furniture,
Stephanus labels, body/footnote/apparatus boundaries, two-column reading order,
and paragraph starts are prefilled. The records remain fail-closed schema 1.2
drafts: every line and all four evidence statuses are `candidate_unverified`,
and only a human workbench review may turn them into gold.

The reproducible local preparation and page-specific visual decisions are in
`scripts/ocr/gold/prepare_visual_batch_2026_08_31.py`. The source PDFs and
rendered page images are local evaluation material and are not publication
assets.

## Strict Gold freeze (2026-09-05)

`freeze-manifest-2026-09-05.json` formally freezes the 16 pages from the
second visual batch that had passed strict, exhaustive human verification at
freeze time. The manifest records the exact closed-world page set plus every
page-record and rendered-image SHA-256 digest. The other 14 selected pages
remain drafts and are intentionally excluded from this freeze.
