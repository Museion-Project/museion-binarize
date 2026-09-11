# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project intends to adhere to [Semantic Versioning](https://semver.org/)
once a first tagged release is published.

## [0.2.0-beta.1] - 2026-09-11

- Rename the desktop app to Museion PDF (中文：Museion PDF 处理器).
- Ship the Apple Silicon local desktop: black-and-white processing, editable
  contents bookmarks, and Chinese/English interface.
- Bundle the pinned local runtime; disable body OCR and remote OCR commands.
- Require bookmark review; retain basic hierarchy when Apple models are unavailable.
- Preserve the bundle identifier and existing language preference.

## [Unreleased] — historical rc.3 development notes (not the Beta feature list)

This section prepares `0.1.0-rc.3`; it is not a published release and has
no release date or download link.

### Preparing 0.1.0-rc.3

- Consolidates the MDP 0.1 evidence package, persistent jobs/provider
  contracts, local OCR, AI-ready exports/revisions, evidence bookmarks, and
  searchable-PDF output now present after rc.2.
- Adds consented, opt-in remote API OCR alongside the offline local OCR and
  conversion paths; no source is uploaded without an explicit declared
  consent record.
- Adds automatic bookmark v2 with deterministic evidence alignment and safe
  refusal. Bookmark generation is local; cloud bookmark generation does not
  exist. The adopted Tesseract runtime and pinned `tessdata_best` files remain
  explicitly provisioned in source builds; bundling them is still an rc.3
  release gate.
- Adds a provider-neutral OCR contract with three explicit execution modes —
  `local` (default, offline, unchanged), `gemini-byok` (the user's own key,
  held in the OS credential store), and `mpdf-credits` (brokered). Every mode
  produces the same canonical block/line/word tree, so one bookmark and
  searchable-PDF pipeline serves all three.
  - Cloud pages take their **coordinates from the local detector** and their
    characters from a whole-page transcription, joined by a monotone,
    injective alignment with coverage/order gates. Model-returned rectangles
    are parsed and gated but never used as a coordinate source.
  - Cloud failure falls back per page to the local reading by default and
    reports the affected page numbers in the evidence, the CLI JSON report and
    the desktop completion panel. No failure path can produce an empty text
    layer.
  - Checkpoints bind provider mode, model and pinned version, prompt digest,
    local layout identity, alignment version and fallback policy — and nothing
    derived from a secret. A **local** run's fingerprint is byte-identical to
    before this change, so no existing local checkpoint was invalidated.
  - No `--api-key` flag exists anywhere; keys are read from stdin or a
    password field and handed to the OS credential store. A canary key is run
    through every failure path in the tests and asserted absent from evidence,
    reports, serialized state and written files.
  - **M PDF Credits has no production service**: the reservation, settlement,
    refund and idempotency state machine is complete and tested against an
    in-process fake, and the outstanding blockers are printed verbatim by
    `mpdf provider list`, the desktop provider picker and the release-readiness
    report. Neither front end will start a brokered run.
- Hardens rc.3 release validation: identity/version invariants, fail-closed
  Developer ID/notarytool orchestration, deterministic SPDX SBOMs, typed
  release manifests, checksums, license notices, and readiness records.

### Added

- **Automatic table of contents (bookmarks v2).** `mpdf bookmark auto` and a
  single desktop button compile a PDF outline from the document's own
  evidence: an existing native outline is preserved exactly, or a printed
  contents list is detected, parsed, and aligned against the headings in the
  text and the printed page labels. Only entries where title, page mapping,
  numbering, layout, OCR confidence, and a monotone position all agree are
  written; anything ambiguous is kept for review.
- A document with no reliable structure produces an explained **safe
  refusal** and invents no title. Standalone `bookmark auto` writes no PDF;
  the combined `mpdf run` flow still delivers its verified searchable bilevel
  PDF with an empty outline. There is no mode in which a model composes a
  table of contents.
- Bookmark snapshot schema **0.2** with an integer score breakdown, typed
  alignment evidence, and a new `auto_confirmed` status that stays
  distinguishable from a human `confirmed`; plus a separate
  `bookmarks/generation-report.json` explaining every run. Schema 0.1 records
  remain readable, reviewable, and buildable, and cannot acquire an automatic
  status.
- Local and consented API OCR are the same typed evidence to the engine:
  identical records produce identical decisions, and provider identity never
  branches the algorithm.
- Output verification now also re-reads the written `/Outlines` tree with
  lopdf and compares its titles, nesting, and destination pages against the
  effective bookmark tree, alongside the existing PDFium reopen check.

### Changed

- `mpdf bookmark generate` now runs the same engine as `bookmark auto`; there
  is no second, simplified generator. `--regenerate` is refused while human
  review decisions exist, and is independent of `--overwrite`.
- The CLI, the desktop application, and `pdf build-searchable` share one safe
  output boundary in `mpdf_core::searchable_output`.

## [0.1.0-rc.2] - 2026-08-25

### Added

- Native desktop drag-and-drop: dropping exactly one PDF anywhere on the
  application window opens it directly, with a visible drop target and a
  clear validation message for unsupported drops.
- GitHub Sponsors support for the open-source project.

### Fixed

- File drag-and-drop now uses Tauri's native window event API. Native
  operating-system drops are intercepted before ordinary HTML drag events,
  which made the earlier webview-style approach ineffective in packaged
  desktop builds.
- The Intel macOS distribution job now targets GitHub's current
  `macos-15-intel` runner instead of the retired `macos-13` label.

## [0.1.0-rc.1] - 2026-08-08

The first public release candidate. Summarized here as a delivered
product, not as a log of every internal commit — see this repository's
own commit history and `docs/roadmap.md` for the full milestone-by-milestone
build record.

### Added

- Deterministic **Otsu**, **Sauvola**, and **manual** thresholding
  binarization methods.
- True 1-bit (bilevel) PDF reconstruction from scanned page images, with
  **CCITT Group 4** compression for compact output.
- A command-line interface (`inspect`, `analyze`, `estimate`, `process`,
  `preview`, `benchmark`) with versioned JSON reports.
- A desktop GUI (macOS, Windows, Linux) wired to the same processing
  core: open, preview, configure, an experimental sampled output-size
  estimate, convert, and cancel a running conversion.
- A reproducible, ground-truth binarization-fidelity benchmarking
  framework, with a committed synthetic fixture suite that validates the
  framework itself (not a corpus of real scanned documents).
- GitHub distribution packaging: every packaged desktop/CLI artifact
  bundles its own pinned, checksum-verified copy of PDFium — no separate
  PDFium install needed to run a downloaded release.
- Mac App Store technical sandbox readiness: a separate, App
  Sandbox-enabled build path exists and has passed local sandboxed
  human-acceptance testing, as groundwork for a possible **future**
  paid Mac App Store distribution. **Nothing has been submitted to
  Apple, and no App Store listing exists** — this is packaging-path
  readiness only, not a release channel.

### Known limitations

- No OCR (optical character recognition), and no preservation of hidden
  OCR text layers from source PDFs — output is image-only.
- No AI, machine-learning, or generative restoration/inpainting of any
  kind.
- No page dewarping or geometric correction.
- No claim is made that this software preserves polytonic Ancient Greek
  typography, critical apparatuses, or other small typographic detail —
  that claim will only be made once reproducible benchmark data exists
  (see `docs/roadmap.md`, Phase 2).
- **macOS (Apple Silicon)** is the only platform with human runtime
  acceptance for this release; the packaged `.app`/`.dmg` is ad-hoc
  signed, **not** Developer ID signed or notarized.
- **Windows and Linux** packages build and package successfully but do
  **not** yet have human runtime acceptance — treat them as
  release-candidate builds.

### Changed

- Permanent application identifier finalized to `me.museion.binarize`
  (previously `org.museionproject.binarize`), owner-approved, ahead of
  any Apple App ID / App Store Connect registration. Declared once in
  `tauri.conf.json`; every distribution overlay inherits it.

### Fixed

- macOS arm64 packaged `.app`/`.dmg` reported "is damaged and can't be
  opened" in Finder instead of launching, because the bundle's
  `Contents/_CodeSignature/CodeResources` resource seal was never
  generated (no `signingIdentity` configured, so `tauri-bundler` never
  resigned the whole bundle after Rust's linker ad-hoc-signs the
  individual Mach-O binaries). The build pipeline now always ad-hoc
  signs the whole `.app` bundle and packages the `.dmg` from that signed
  bundle directly, fixing the defect for unsigned (current) builds. See
  `docs/desktop-testing.md`, "macOS arm64: 'is damaged' bug found by
  human runtime testing."

### Added (Milestone 0, historical)

- Repository initialization — Rust workspace scaffolding
  (`museion-binarize-core`, `museion-binarize-cli`), a minimal Tauri 2 +
  React + TypeScript desktop shell, bilingual project documentation,
  dual MIT/Apache-2.0 licensing, citation metadata, contributor
  guidelines, and an initial CI workflow. At this point in the project's
  history, no PDF processing functionality existed yet — see "Added"
  above for what this `0.1.0-rc.1` release actually delivers.
