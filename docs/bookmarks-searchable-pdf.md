# Evidence bookmarks and searchable PDF

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
