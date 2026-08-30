# M PDF Processor development handoff — 2026-08-30

This file is the starting point for the next Codex conversation.

## Repository state

- Repository: `/Users/theo/AI 工作流/museion-binarize`
- Branch: `codex/cloud-complete-ocr-bakeoff`
- Geometry implementation commit: `04d918b feat: select provisional Tesseract geometry`
- Earlier architecture commits:
  - `8a78d67 feat: prepare mixed-script OCR gold and geometry seam`
  - `fef5157 feat: split OCR geometry and transcription`
  - `e0d2be5 test: record Gemini OCR forensic probe`
  - `b98047d test: freeze OCR coverage forensic probe`
- Working tree at handoff: only the user's existing untracked draft
  `gold-data/closed-world-cgpg-v1/pages/` remains. Do not delete, stage or
  relabel it without reviewing it with the user.
- No push, tag or public release was made in this work.

## Product and architecture decisions

1. OCR is a three-layer product:
   - base local/native-text and deterministic PDF processing;
   - paid brokered complete OCR;
   - an optional offline local OCR plugin.
2. BYOK is disabled. Reconsider it only if one API independently provides
   complete coordinate OCR.
3. The paid complete-OCR pipeline is:

   ```text
   deterministic GeometryProvider
     -> immutable line ids, boxes and reading order
   Gemini 3.7 Flash transcription
     -> exactly one text response per line id, no coordinates
   strict compositor
     -> coordinate-bearing OcrPage
   independent PDF text-layer verification
   ```

4. Text recognition and coordinate generation are replaceable steps. Gemini
   cannot modify geometry; Tesseract text is discarded on the split path.
5. The provisional GeometryProvider is Tesseract 5.5.3, OEM 1, PSM 3, using
   pinned `tessdata_best` 4.1.0. Every page remains stamped
   `historical_material_not_validated`.

Authoritative design documents:

- [`adr/0012-deterministic-geometry-and-gemini-transcription.md`](adr/0012-deterministic-geometry-and-gemini-transcription.md)
- [`adr/0014-provisional-tesseract-geometry.md`](adr/0014-provisional-tesseract-geometry.md)
- [`ocr-providers.md`](ocr-providers.md)
- [`architecture.md`](architecture.md)

## GeometryProvider comparison completed

The supplied Gerson PDF is a 327-page born-digital document with native vector
text. It is a useful clean geometry control, not historical-scan gold and not
human-verified closed-world gold.

Eight pages were rendered at 300 DPI. Native visible PDF text objects supplied
421 reference line boxes and content-stream reading order. Every provider ran
twice on identical images.

| Candidate | Line F1 | Mean IoU | Reading-order τ | Median s/page | Exact repeat |
|---|---:|---:|---:|---:|---|
| Tesseract 5.5.3 PSM 3 | **0.9721** | 0.8147 | **0.9291** | 4.898 | yes |
| PP-OCRv5 server detector | 0.9689 | **0.8320** | 0.8333 | 3.350 | yes |
| Apple Vision text rectangles | 0.9514 | 0.8100 | 0.8915 | **0.168** | yes |

Paddle had tighter boxes but its simple deterministic orderer interleaved the
two-column index pages. Apple was fastest but over-segmented more lines.
Tesseract won the frozen primary F1 and reading-order comparison.

Evidence and reproduction harness:

- [`evidence/geometry-provider-clean-native-2026-08-30.json`](evidence/geometry-provider-clean-native-2026-08-30.json)
- [`../scripts/ocr/geometry/README.md`](../scripts/ocr/geometry/README.md)

## Pipeline state

- `SidecarOcrConfig::tesseract_geometry()` fixes PSM 3.
- `GeometryTranscriptionPageProvider::from_selected_tesseract_sidecar()`
  rejects ordinary PSM 6 configurations so unmeasured evidence cannot be
  mislabeled as the selected candidate.
- Geometry validation status participates in the geometry digest, pipeline
  fingerprint and composed-page provenance.
- A real sidecar probe on control page 100 returned 4 blocks, 46 lines and 599
  words; its 46 lines matched the native control line count. Protocol, page
  identity, input digest, engine version and PSM round-tripped correctly.
- The Rust vertical-slice test runs selected Tesseract-shaped geometry through
  the real `GeminiGeometryTranscriber` contract with a fake network transport,
  then verifies the composed `OcrPage` preserves the original boxes.
- The production paid broker, payment path and real service transport remain
  unavailable. “Pipeline connected” does not mean the cloud product is live.

Validation completed:

- `python3 -m pytest scripts/ocr/geometry/test_geometry_bakeoff.py -q`: 2 passed
- `cargo test -p mpdf-core --lib`: 530 passed
- `cargo check` passed for `mpdf-api-client`, `mpdf-cli` and `mpdf-desktop`
- `cargo fmt --all -- --check`: passed
- `git diff --check`: passed

## `target/` disk audit

No files were deleted during this audit. Current sizes are approximate because
Cargo builds and APFS accounting can move object files between directories.
At audit time the data volume reported only about 2.8 GiB available and 100%
capacity after rounding, so another normal parallel debug build can fail.

| Path | Size | Classification | Recommendation |
|---|---:|---|---|
| `target/debug/` | 59 GiB | entirely rebuildable development output | primary cleanup target |
| `target/debug/incremental/` | 31 GiB | 926 stale/current incremental sessions | safest large deletion; next builds lose incremental reuse |
| `target/debug/deps/` | 25 GiB | about 403k files, including about 398k `.o` files | rebuildable; remove with all of `target/debug/` for maximum recovery |
| `target/debug/build/` | 2.3 GiB | dependency build-script output | rebuildable |
| `target/release/` | 1.8 GiB | release build plus intermediate deps | preserve for now; `deps/` 1.4 GiB and `build/` 350 MiB are rebuildable later |
| `target/aarch64-apple-darwin/release/` | 1.8 GiB | arm64 app/runtime/bundle plus deps | preserve until release evidence is archived |
| `target/distribution/` | 813 MiB | DMG/tar, frozen OCR runtime, wheels, smoke files and evidence | do not clean indiscriminately |
| `target/distribution/ocr-release/` | 337 MiB | final DMG/CLI archive plus two unpacked verification copies | preserve final artifacts; unpacked copies can be reconsidered after archival |
| `target/pdfium/` | 7.4 MiB | provisioned PDFium | keep |

The growth is development-cache accumulation, not corpus/gold data. The most
conservative recovery is deleting only `target/debug/incremental/` (about
31 GiB). The strongest low-risk recovery is deleting all `target/debug/`
(about 59 GiB); this forces a cold rebuild but does not remove source or the
separate release/distribution trees.

Do **not** run an unqualified `cargo clean` before archiving the release
artifacts: Cargo's target directory also contains `target/distribution/` and
the verified arm64 runtime staging. Any deletion still requires explicit user
authorization. For low-space verification, the proven command pattern is:

```text
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo check ...
```

Longer-term, place `CARGO_TARGET_DIR` on a larger development volume or adopt a
periodic debug-only cleanup policy. Do not move release evidence without also
updating its paths/hashes and readiness documentation.

## Next minimum milestone

Build a frozen historical-scan **geometry** holdout. Language-balanced exact
transcription is not required for this gate; exhaustive visible-line boxes,
reading order, non-text/blank coverage, image digest and annotation provenance
are required. Include representative skew, bleed-through, damaged type,
marginalia, small apparatus, columns and ornaments.

Then rerun the same candidates on identical images:

1. Tesseract PSM 3 (current provisional choice).
2. PP-OCRv5 detector with a column-aware deterministic orderer.
3. Apple Vision as a macOS control.
4. A new local candidate only after installability, redistribution and runtime
   constraints are established; do not download several providers at once.

Promote `historical_material_not_validated` only if the frozen holdout passes.
If it fails, keep the split contract and replace only `GeometryProvider`.
