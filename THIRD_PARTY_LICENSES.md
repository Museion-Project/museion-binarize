# Third-Party Licenses

Museion Binarize is dual-licensed under MIT OR Apache-2.0 (see
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE)). This
project also uses third-party open-source software. This file will track
attributions for all bundled or statically linked dependencies.

## Status

At Milestone 0 (initial scaffolding), the workspace has no runtime
dependencies beyond `clap` (CLI argument parsing, MIT OR Apache-2.0) and the
standard Tauri/React/Vite toolchain used to build the desktop application.
None of these are bundled or redistributed as source in this repository.

A complete, automatically-checked list of third-party licenses will be
generated as real dependencies (image processing, PDF generation, PDFium
bindings, etc.) are introduced in later milestones. License compliance is
enforced in CI via [`deny.toml`](deny.toml) and `cargo deny check`.

## PDFium

Museion Binarize's architecture anticipates using [PDFium](https://pdfium.googlesource.com/pdfium/)
(BSD-3-Clause and other permissive licenses) for PDF rendering/decoding in a
later milestone. PDFium binaries, if used, will be fetched through a
separate, documented, controlled build process — never committed to this
repository — and their license terms will be reproduced here at that time.

## Node.js / frontend dependencies

Frontend dependency licenses are captured in `apps/desktop/package.json` and
its lockfile. A generated third-party notice for the frontend bundle will be
added once the desktop application has a real dependency tree beyond the
Tauri/React/Vite starter template.
