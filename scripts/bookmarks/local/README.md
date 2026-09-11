# Local bookmark MVP (development delivery)

Contract: `intent.md §8.34 TG1–TG4` amends §8.33: editable book-level review, three compiler repairs only. No network, cloud recognition, model downloads, or searchable OCR output. Zero-based PDF indices in JSON; one-based page numbers in the editor and command-line page selection.

## Components

- `BookmarkEvidence` is defined in `crates/mpdf-core/src/bookmarks/local.rs`. Visible top-left PDF points, source hash/ROI/affine transform, raw references, line/group identity, measured bounds/size proxy, optional confidence and explicit source/state. Raw Vision observations and native glyphs remain separate files.
- `bookmarks.py native` extracts actual glyph rectangles, reconstructs visual bands and words, and preserves raw glyphs. It never calls `.text().all()` or generates evenly spaced synthetic boxes.
- `vision_fast.swift` is the Apple Vision `.fast` worker. Full-page observation and token boxes/confidence are saved without correction. Run outside the Codex sandbox; no persistent approval rule is needed.
- `bookmarks.py project` performs generic lane and visual grouping; labels are detected before grouping. `compile` calls the existing Rust TOC parser and the new whole-book local hierarchy grammar through `mpdf-bookmarks`; the old 0.4 engine/scoring/gates are unchanged.
- Printed number lanes are independent of section labels and titles. Missing/low-confidence/invalid/nonmonotonic/out-of-range/large-jump labels stay review. `numeric_lane.py` is now a mandatory normal second read for all title-bearing visual rows, before final grouping, using the frozen Vision fast worker. `retry_numbers.py` remains historical and is not the normal path. Full-page raw and second-read raw remain separate.
- Pagination uses explicit PDF PageLabels or exact native header candidates. Header candidates remain review; no inferred global offset. `map --anchors FILE` accepts exact inspected anchors with evidence references. **No source outlines or assessment files are read by this path.**
- The current compiler requests Surya only for double-column reading-order faults, bbox fragmentation or unstable physical line groups. Missing numbers and semantic hierarchy do not trigger it. The frozen legacy `geometry_fallback.py` utility requires a source/page-bound diagnostic and is not called by the normal intake. It calls cached Surya **detection only** and the unchanged existing adapter. Its supplemental measured supports enter the same compiler; ambiguous/multiple associations remain review. It does not repair missing text or reliably resegment mixed observations yet.
- `editor.py` writes a standalone offline HTML editor. It supports title/parent/target edits, add/delete, explicit item confirmation, and source-page/position links. It downloads an auditable edit patch. Apply with the backend, then export. This is a basic two-command save workflow, not an integrated desktop release UI.
- `export` creates only new outline objects and changes Catalog/Outlines. All original object dictionaries and stream bytes, complete page geometry/text/glyphs/rendering, tree and actual destinations are checked before atomic no-clobber installation. Existing source outline objects remain untouched. No native-font literal decoder runs. Encrypted/signed inputs currently refuse explicitly.

## Run

Development uses Python with the pinned pypdf, pypdfium2 and Pillow dependencies. The macOS release carries a frozen local runtime and precompiled Apple Vision helpers; it does not discover Python installations or invoke a compiler at runtime. See `distribution/local-runtime/` and `scripts/distribution/build_local_runtime.py`.

```sh
CARGO_INCREMENTAL=0 cargo build -p mpdf-core --offline --bin mpdf-bookmarks
swiftc scripts/bookmarks/local/vision_fast.swift -o /private/tmp/mpdf-vision-fast
python3 scripts/bookmarks/local/intake.py SOURCE.pdf NEW_DIRECTORY --pages 6,7 --vision-worker /private/tmp/mpdf-vision-fast
```

Intake prefers native glyphs when available. It runs Vision only for selected pages without native glyphs. TOC page selection is explicit in this MVP; no full-book OCR or model-driven discovery. A sparse/incorrect pre-existing text layer is currently an intake limitation, not an automatic reason to run another OCR engine.

For the full-page Vision experiment independent of native preference:

```sh
cargo build -p mpdf-core --offline --example render_bookmark_pages
# MPDF_PDFIUM_LIBRARY must reference the installed project PDFium library.
target/debug/examples/render_bookmark_pages CASES.json NEW_RENDER_DIR
/private/tmp/mpdf-vision-fast NEW_RENDER_DIR/pages.json NEW_VISION_DIR
python3 scripts/bookmarks/local/bookmarks.py project NEW_EVIDENCE.json NEW_VISION_DIR/page.json
python3 scripts/bookmarks/local/bookmarks.py compile NEW_EVIDENCE.json NEW_TABLE.json
python3 scripts/bookmarks/local/bookmarks.py map NEW_TABLE.json NEW_MAPPED.json --anchors INSPECTED_ANCHORS.json
python3 scripts/bookmarks/local/editor.py NEW_MAPPED.json NEW_EDITOR.html
```

After editing and downloading `bookmark-edits.json`:

```sh
python3 scripts/bookmarks/local/bookmarks.py edit NEW_MAPPED.json bookmark-edits.json REVIEWED.json
python3 scripts/bookmarks/local/bookmarks.py export REVIEWED.json NEW_BOOKMARKED.pdf NEW_RECEIPT.json
```

All outputs must be new paths. Export refuses unresolved entries, invalid parents/cycles, out-of-range targets, source hash changes and output collisions. Editing a title or target invalidates that entry's confirmation; confirm after inspecting it. Delete removes the subtree. Parent edits are canonicalized into preorder and levels recomputed.

## Validation

```sh
cargo test -p mpdf-core --offline bookmarks --lib
cargo test -p mpdf-core --offline --test auto_bookmarks
python3 -m unittest discover -s scripts/bookmarks/local -p 'test_*.py' -v
cargo build -p mpdf-core --offline --example verify_bookmark_outline
MPDF_PDFIUM_LIBRARY=/ABSOLUTE/PATH/libpdfium.dylib target/debug/examples/verify_bookmark_outline REVIEWED.json NEW_BOOKMARKED.pdf
```

The development report and source-bound quality assessment are in `docs/evidence/vision-bookmarks-2026-09-10/report.zh-CN.md`. Assessment data is agent source-image review, not human Gold. A successful manual-edit round trip is not an automatically correct directory.

Current repair regression and book-scope edit burden: `docs/evidence/toc-compiler-repair-2026-09-10/report.zh-CN.md`. No GUI/writer change is included in this repair.


### Source tree and navigation projection

The additive table fields `source_entries` and `navigation_projection` retain the
compiler's source-tree interpretation and each `omit_container_promote_children`
decision. `entries` is the editable navigation tree. Only unpaginated numbered
containers with actual children are projected out; missing-page leaves remain.
An omitted container remains in `source_entries` with its source/evidence IDs.
Projection is a reviewable convention, not evidence that the image has no page
number. Promoted children carry `container_projection_requires_review`.
To inspect or edit the source-tree alternative, create a separate table with
`entries` copied from `source_entries`; never overwrite the original table.
Manual edits affect that table's active `entries`; `source_entries` stays the
original compiler interpretation and is not a synchronized second editor.

`label_hypotheses` retains the interpreted label plus `null` (literal, unparsed
reading) for normalized labels; `section_label` is the selected interpretation.
An isolated confusable decimal is not selected without another explicit book
prefix/child/sibling. `parent_hypotheses` records the selected and alternative
scope/root parents for named headings. Alternatives are not automatically
confirmed and the editor does not yet offer a dedicated hypothesis chooser.
Measured `title_start_x` and `label_bbox` are derived only from existing token
boxes. An unavailable substring boundary falls back to the original line box;
no character-width geometry is synthesized. These are parser alternatives, not
Vision n-best candidates; the existing worker/raw contract remains unchanged.

### Explicit-TOC recovery development entry point

`recover.py SOURCE.pdf NEW_OUTPUT --vision-worker /absolute/vision-fast --compiler
/absolute/mpdf-bookmarks` accepts a complete PDF without annotated page numbers.
It scans every page using native observations or the existing Vision fast worker,
then compiles discovered navigation spans independently. `prediction.json` retains
source entries, complete printed references, group pages and review diagnostics.
Raw discovery observations, selected-page recognition and numeric rereads remain
alongside it. It does not perform pagination mapping or write a bookmarked PDF.

This is the development CLI evaluated under
`evaluation/toc-generalization-2026-09-10/`; it is not an automatic desktop page
selection feature. Discovery can miss unrecognized headings or admit unrelated
navigation-like pages. Results require review, and a completed process is not an
exact-document pass. No body-heading style model or body-derived TOC is used.

The PDFium native extraction path has schema `mpdf-native-glyph-raw/2`; historical MuPDF measurements are not rewritten or relabelled. Bookmark output uses pypdf object serialization with a classic incremental xref table and validates original byte-prefix/object/stream preservation plus complete PDFium glyphs, pixels and actual destinations.
