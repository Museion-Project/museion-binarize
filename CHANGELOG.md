# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project intends to adhere to [Semantic Versioning](https://semver.org/)
once a first tagged release is published.

## [Unreleased]

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
