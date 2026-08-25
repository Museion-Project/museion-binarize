# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project intends to adhere to [Semantic Versioning](https://semver.org/)
once a first tagged release is published.

## [Unreleased]

Nothing yet since `0.1.0-rc.2`.

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
