# Third-Party Licenses

M PDF Processor is dual-licensed under MIT OR Apache-2.0 (see
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE)). This
project also uses third-party open-source software. This file will track
attributions for all bundled or statically linked dependencies.

## Status

As of Milestone 2 the processing core has real runtime dependencies. All of
them are permissively licensed and compatible with this project's MIT OR
Apache-2.0 dual license; no GPL or AGPL dependency is present. License
compliance is enforced by [`deny.toml`](deny.toml) and `cargo deny check`.

### Direct Rust dependencies

| Crate | Purpose | License |
|---|---|---|
| `clap` | CLI argument parsing | MIT OR Apache-2.0 |
| `image` | Image buffers, PNG encoding | MIT OR Apache-2.0 |
| `fax` | CCITT Group 3/4 encoding and decoding | MIT |
| `pdf-writer` | Output PDF construction | MIT OR Apache-2.0 |
| `pdfium-render` | Safe bindings to PDFium | MIT OR Apache-2.0 |
| `tempfile` | Safe temporary output files | MIT OR Apache-2.0 |
| `thiserror` | Error type derivation | MIT OR Apache-2.0 |

None of these are vendored or redistributed as source in this repository;
they are fetched from crates.io at build time.

## PDFium

M PDF Processor uses [PDFium](https://pdfium.googlesource.com/pdfium/) to
rasterize source PDFs. PDFium is licensed BSD-3-Clause with Apache-2.0
components; prebuilt binaries commonly come from the
[`pdfium-binaries`](https://github.com/bblanchon/pdfium-binaries) project,
whose packaging is MIT.

**No PDFium binary is committed to this repository, and the application
never downloads one at runtime.** The library is supplied separately by a
developer or packager; see [`docs/pdfium.md`](docs/pdfium.md).

Full license texts are committed under
[`third_party/pdfium/`](third_party/pdfium/):

- [`LICENSE-PDFIUM`](third_party/pdfium/LICENSE-PDFIUM) — PDFium itself;
- [`LICENSE-DISTRIBUTION`](third_party/pdfium/LICENSE-DISTRIBUTION) — the
  binary distribution packaging.

[`third_party/pdfium/manifest.toml`](third_party/pdfium/manifest.toml)
records the provenance and locally-verified SHA-256 of every PDFium asset
this project has actually used. **Anyone redistributing a PDFium binary
alongside M PDF Processor must ship these notices.**

**Official packaged builds** (Milestone 7A) bundle a PDFium library
fetched and checksum-verified at build/package time from a pinned
upstream release — see
[`distribution/pdfium/manifest.toml`](distribution/pdfium/manifest.toml)
and [`docs/pdfium-bundling.md`](docs/pdfium-bundling.md). This is a
distinct, release-pipeline-specific provenance record from the developer
manifest above, but both point at the same upstream PDFium/pdfium-binaries
projects and licenses.

## Node.js / frontend dependencies

Frontend dependency licenses are captured in `apps/desktop/package.json` and
its lockfile. A generated third-party notice for the frontend bundle will be
added once the desktop application has a real dependency tree beyond the
Tauri/React/Vite starter template.
