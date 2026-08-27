# ADR 0007: evidence-based bookmarks and searchable PDF write-back

Status: accepted for M5.

## Decision

M5 adds a versioned `mpdf-bookmarks` 0.1 extension below `bookmarks/` in an
MDP. Generated candidates are immutable evidence records. Human review is an
append-only operation log; an effective view is computed from the generated
record plus that log. Regeneration is deterministic and refuses to apply a
review log whose candidate or generation digest is stale.

Every candidate contains a stable ID, immutable `source_title`, effective
title, source and effective level/parent, target page ID and zero-based physical
page index, a master-space target bbox when available, confidence, review
status, generator kind/name/version, reason codes, and typed evidence
references. An evidence reference resolves either to a stable DerivedDocument
page/region/block/line/word reference with page and bbox, or to an explicitly
indexed MDP existing-outline/page-label/typography/region record. Candidates
without a resolvable source are invalid. Human edits never replace source
title, source hierarchy, generator provenance, evidence, or rule trace.

The deterministic generator uses, in priority order, existing outline
evidence, traceable table-of-contents entries, title-region and typography
evidence, numbering patterns, reading order, OCR confidence, printed page
labels, effective M4 human text, and repeated header/footer suppression. Its
fixed rule and scoring versions are part of the generation digest. Sorting is
by physical page, target position, level, normalized source title, and
candidate ID. Identical inputs and revisions therefore produce identical JSON
bytes and IDs. Low-confidence and ambiguous candidates are `needs_review`;
automatic generation never creates a human `confirmed` result. AI is only a
reserved generator/proposal kind in M5 and is never invoked or applied.

The searchable-PDF command accepts an MDP, an explicit source PDF, and a
distinct output PDF. The source bytes must match `source.content_sha256`, its
page count and effective page geometry must match the MDP, and the MDP, OCR,
DerivedDocument inputs, bookmark generation, and review log must all validate
before output construction. Only effective human-confirmed bookmark candidates
are written to the outline. The text layer uses effective human-revised word
text and retains word evidence references in the build report.

`pdf-writer` remains the deterministic builder for new bilevel PDFs, but it is
not a PDF parser or editor. M5 therefore uses a bounded low-level PDF editing
layer to load and rewrite the matching source document without rasterizing its
existing page contents. It preserves page objects, inherited MediaBox/CropBox,
page count, `/Rotate`, and visible image streams, then appends one invisible
text content stream per page and a new outline tree. Existing visible content
is not modified.

The text layer uses one document-scoped embedded, redistributable Unicode
TrueType font exposed as a Type-0 font with Identity-H encoding, a CIDFontType2
descendant, deterministic CID-to-GID mapping, widths, and a ToUnicode CMap.
Character-to-CID assignment is the sorted set of Unicode scalar values used by
effective OCR text. Missing glyphs may use `.notdef`, but ToUnicode must still
map the CID back to the original UTF-16BE sequence so extraction remains
lossless. More than 65,534 distinct scalars, invalid font data, or an invalid
mapping fails closed. Font license and attribution travel with the bundled
font. No font or model is downloaded at runtime.

Each OCR word bbox is converted from top-left master coordinates through the
inverse declared affine transform to the page's visible PDF-point space. The
writer then maps visible coordinates into the source page's default user space
using the resolved CropBox/MediaBox origin and `/Rotate` (0/90/180/270). Text is
placed independently per word with text rendering mode 3 (neither fill nor
stroke); its baseline, font size, and horizontal scaling are bounded by the
word box. Outline destinations use the same mapping and explicit `/XYZ`
coordinates. Non-finite, singular, out-of-page, or geometry-mismatched values
are rejected rather than clamped silently.

Output is no-clobber by default. Input/output aliases, symlink inputs or
destinations, directory targets, and stale/partial/corrupt state are rejected.
Construction and PDFium reopen validation happen in a temporary regular file
in the output directory. Cancellation or any failure removes the temporary
file. Only a validated file is atomically installed; `--overwrite` is required
to replace an existing regular file and never permits replacing the source.

## Persistent layout and commands

```text
book.mdp/
└── bookmarks/
    ├── candidates.json   # deterministic immutable generation snapshot
    └── reviews.json      # append-only human/AI-proposal operations
```

The CLI contract is:

```text
mpdf bookmark generate <MDP> [--overwrite] [--json]
mpdf bookmark list <MDP> [--json]
mpdf bookmark confirm <MDP> --candidate <ID>
mpdf bookmark reject <MDP> --candidate <ID>
mpdf bookmark edit <MDP> --candidate <ID> --title <TEXT>
mpdf bookmark reparent <MDP> --candidate <ID> [--parent <ID>] --level <N>
mpdf pdf build-searchable <MDP> --source <PDF> --output <PDF> [--overwrite] [--json]
```

Mutation commands append a deterministic review record and atomically replace
only `bookmarks/reviews.json`. They reject missing candidates, invalid parents,
cycles, stale base generation digests, duplicate operation IDs, and invalid
state transitions. The desktop invokes the same core APIs and persists the
same records; it is not an in-memory mock. If an MDP has no preview asset, the
UI shows page, coordinate space, and bbox text only.

## Validation

Unit and contract tests cover existing-outline fidelity, printed/physical page
separation, numbering and typography hierarchy, header/footer suppression,
TOC traceability, low-confidence review routing, immutable source evidence,
stable regeneration, master/PDF transforms, rotated destinations, Unicode and
polytonic Greek, outline round trips, and all output safety cases. A real
PDFium test opens a fixture, builds MDP/OCR/DerivedDocument/candidates/PDF,
reopens it, and verifies page geometry/rotation, extracted Unicode text,
outline hierarchy and destination, and unchanged visible rendering/polarity.

## Consequences and limits

This design preserves the supplied matching source PDF rather than producing a
fresh binarization. Callers that want a bilevel searchable PDF first create the
bilevel PDF and its matching MDP, then build the searchable derivative from
that file. M5 does not shape complex scripts or infer missing reading order;
the invisible layer preserves logical Unicode and word boxes. Unsupported font
glyphs remain extractable through ToUnicode but may not have useful visible
outlines (the layer is intentionally invisible). Ambiguous TOC targets,
unusual rotations/page boxes, and low-confidence headings remain review items.
Cloud OCR/LLM calls, generated titles without evidence, runtime downloads,
signing, notarization, packaging, branding, and repository renaming are out of
scope.
