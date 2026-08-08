# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project intends to adhere to [Semantic Versioning](https://semver.org/)
once a first tagged release is published.

## [Unreleased]

### Changed

- Permanent application identifier finalized to `me.museion.binarize`
  (previously `org.museionproject.binarize`), owner-approved, ahead of
  any Apple App ID / App Store Connect registration. Declared once in
  `tauri.conf.json`; every distribution overlay inherits it.

### Added

- Mac App Store readiness (Milestone 7B1): a separate, App
  Sandbox-enabled build path (`tauri.mas.conf.json`, a minimal
  entitlements template, `scripts/distribution/package_mas.py`) beside
  the existing GitHub Developer-ID distribution — not a replacement for
  it. A sandboxed-output-save strategy
  (`OutputWriteStrategy::DirectWriteToDestination`, MAS build only, via
  the `mas-sandbox` Cargo feature) writes directly to a user-selected
  destination instead of the same-directory-rename pattern the CLI and
  GitHub build keep using unchanged, because a real, credential-free
  local App Sandbox test showed a sandboxed save panel's grant does not
  obviously cover a sibling temp file. See
  `docs/mac-app-store-readiness.md`.

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

### Added

- Milestone 0: repository initialization — Rust workspace scaffolding
  (`museion-binarize-core`, `museion-binarize-cli`), a minimal Tauri 2 +
  React + TypeScript desktop shell, bilingual project documentation,
  dual MIT/Apache-2.0 licensing, citation metadata, contributor
  guidelines, and an initial CI workflow.
- No PDF processing functionality is implemented yet.
