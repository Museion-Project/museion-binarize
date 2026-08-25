English | [简体中文](README.zh-CN.md)

# Museion Binarize

**Museion Binarize** is an open-source, cross-platform application for
converting scanned scholarly books into clean and compact bilevel PDFs.

**Status: Phase 1 feature-complete — current public release candidate:
`v0.1.0-rc.2`.** A complete local CLI pipeline exists
(`inspect`, `analyze`, `estimate`, `process`, `preview`, `benchmark`,
with versioned JSON reports), and the desktop GUI is wired to the same
pipeline (open, preview, configure, an experimental size estimate,
convert, cancel — see [`docs/desktop.md`](docs/desktop.md)). `estimate`
produces an experimental, sampled output-size prediction — see
[`docs/size-estimation.md`](docs/size-estimation.md); it is not a
guarantee. `benchmark` is a reproducible ground-truth
binarization-fidelity benchmarking framework — see
[`docs/benchmarking.md`](docs/benchmarking.md); its committed synthetic
fixture suite validates the framework and is **not** a representative
corpus of real scanned documents, and is not evidence for a preservation
claim about historical polytonic Greek editions. Human end-to-end
runtime acceptance exists for **macOS (Apple Silicon) only** — see
[`docs/desktop-testing.md`](docs/desktop-testing.md) for the native
desktop acceptance record; Windows and Linux packages build and package
successfully but do not yet have human runtime acceptance (see
"Download" below). See [`docs/limitations.md`](docs/limitations.md) for
exactly what this repository can and cannot do today.

## Core principles

- **Transparent processing. Reproducible results. No generative rewriting of
  source documents.**
- Deterministic, explainable algorithms over black-box models. Every
  binarization decision should be traceable to a documented method and
  parameters.
- Local-first. Your scans and your output stay on your machine.
- Bounded, predictable resource usage on large scanned books.
- Cross-platform by default: macOS, Windows, and Linux are first-class
  targets, not afterthoughts.

Museion Binarize is not described as "AI-powered." Phase 1 uses classical,
deterministic image-processing methods, not machine learning models.

## Phase 1 features

- macOS, Windows, and Linux desktop support.
- Fully local PDF processing — no upload, no network dependency for
  conversion.
- Deterministic **Otsu**, **Sauvola**, and **manual** thresholding methods
  for binarization.
- True 1-bit (bilevel) PDF reconstruction from scanned page images.
- **CCITT Group 4** compression for compact bilevel output.
- Both a graphical desktop application and a command-line interface, sharing
  the same processing core.
- Native single-PDF drag-and-drop in the desktop application.
- A reproducible benchmarking framework for evaluating output quality.

All of the above is implemented in this repository today. See
[`docs/roadmap.md`](docs/roadmap.md) for the milestone-by-milestone
history of how it was built, and "Download" below for how to get a
packaged build.

## Download

The current public release is
[**v0.1.0-rc.2**](https://github.com/Museion-Project/museion-binarize/releases/tag/v0.1.0-rc.2)
— the second public release candidate. It is a **prerelease**: use the
direct links below or the release page itself, not
`/releases/latest` (which only ever points at a stable, non-prerelease
version and will not list this one). Every packaged build — desktop app
and CLI, on every platform — bundles its own pinned copy of PDFium;
**you do not need to install PDFium separately to run a downloaded
release.** (Running from source is different — see "Provide PDFium"
below.)

| Platform | Download | Human runtime tested | Signing |
|---|---|---|---|
| macOS (Apple Silicon / arm64) | [`.dmg`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/Museion-Binarize-0.1.0-rc.2-macos-arm64.dmg) (desktop app) · [CLI `.tar.gz`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/museion-binarize-cli-0.1.0-rc.2-macos-arm64.tar.gz) | Yes — the primary validated platform | Ad-hoc signed, **not** Developer ID signed or notarized (see below) |
| Windows x64 | [`.msi`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/Museion-Binarize-0.1.0-rc.2-windows-x64.msi) installer · [CLI `.zip`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/museion-binarize-cli-0.1.0-rc.2-windows-x64.zip) | Not yet — release-candidate build only | Unsigned |
| Linux x86_64 | [`.AppImage`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/Museion-Binarize-0.1.0-rc.2-linux-x86_64.AppImage) · [`.deb`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/Museion-Binarize-0.1.0-rc.2-linux-x86_64.deb) · [CLI `.tar.gz`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/museion-binarize-cli-0.1.0-rc.2-linux-x86_64.tar.gz) | Not yet — release-candidate build only | Not applicable |

The [release page](https://github.com/Museion-Project/museion-binarize/releases/tag/v0.1.0-rc.2)
also has [`SHA256SUMS`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/SHA256SUMS)
and [`release-manifest.json`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/release-manifest.json)
for verifying every asset above, plus all CLI archives listed in the
table.

**macOS Gatekeeper**: the desktop app is signed with a complete, valid
ad-hoc signature (not a Developer ID certificate, and not notarized), so
first launch shows the standard "Apple could not verify... is free of
malware" prompt for an app from an unidentified developer. Right-click
(or Control-click) the app and choose **Open**, then confirm — this is
the normal, expected macOS flow for an unsigned/ad-hoc-signed
distribution, once per machine. **Do not** disable Gatekeeper
system-wide (`sudo spctl --master-disable`) to work around this; that
turns off a real security feature for every app, not just this one, and
is never necessary here.

**Windows SmartScreen**: the installer is unsigned; Windows may warn
accordingly. No trust or reputation claim is made for it.

See [`docs/releasing.md`](docs/releasing.md) for exactly what "ad-hoc
signed" means technically and what real Developer ID signing/notarization
would additionally require, and
[`docs/desktop-testing.md`](docs/desktop-testing.md) for the full,
per-platform verification-state record — "built" and "packaged" are not
the same claim as "human runtime tested," and this repository does not
conflate them.

## Research direction

A later, benchmark-driven research phase will evaluate methods intended to
better preserve **polytonic Ancient Greek**, **critical apparatuses**, and
other small typographic details that aggressive binarization can destroy.
No claim is made today that Museion Binarize preserves this kind of
typography — that claim will only be made once reproducible benchmark data
exists. See [`docs/roadmap.md`](docs/roadmap.md) and
[`docs/benchmarking.md`](docs/benchmarking.md).

## Current non-goals

Phase 1 does **not** include:

- OCR (optical character recognition).
- Preservation of hidden OCR text layers from source PDFs.
- AI or machine-learning models of any kind.
- Generative restoration or inpainting of damaged/missing content.
- Page dewarping or geometric correction.
- Annotation or form-field preservation.

See [`docs/limitations.md`](docs/limitations.md) for the complete list and
rationale.

## Distribution

Museion Binarize's source remains open source (MIT OR Apache-2.0), and
GitHub builds remain fully functional; a future paid Mac App Store
edition is planned as a convenience distribution, not a separate
closed-source tier — technical sandbox readiness for that path exists
(see [`docs/mac-app-store-readiness.md`](docs/mac-app-store-readiness.md)),
but nothing has been submitted to Apple and no App Store listing exists.
See [`docs/distribution.md`](docs/distribution.md) for the full model.
Packaged GitHub releases are published starting with `v0.1.0-rc.1` — see
"Download" above. Building a package yourself from source is also
possible — see [`docs/releasing.md`](docs/releasing.md).

## Open source and supporting the project

Museion Binarize is free and open source. The official GitHub builds are
fully functional and freely available — nothing is held back for a paid
tier.

If Museion Binarize is useful to you, you can support its continued
development through [GitHub Sponsors](https://github.com/sponsors/pei-haoran).

A paid Mac App Store edition is also planned for the future. It is
intended as a convenient way to install and update the application
while supporting continued development — not as a feature-gated
replacement for the free GitHub version. No price has been set, and no
App Store submission exists yet (see [`docs/distribution.md`](docs/distribution.md)).

## Privacy

Museion Binarize is designed to process files entirely on your own machine.
The core processing pipeline does not upload scans, page images, or output
files to any network service. The desktop application and CLI operate on
local files you choose.

## Repository architecture

```
museion-binarize/
├── crates/
│   ├── museion-binarize-core/   # Tauri-independent Rust processing core
│   └── museion-binarize-cli/    # Command-line interface built on the core
├── apps/
│   └── desktop/                 # Tauri 2 + React + TypeScript desktop app
├── docs/                        # Architecture, roadmap, algorithms, benchmarking
├── benchmarks/                  # Reproducible benchmarking framework (planned)
├── test-data/                   # Synthetic and provenance-documented fixtures
└── .github/                     # CI workflows, issue and PR templates
```

See [`docs/architecture.md`](docs/architecture.md) for the full design,
including why the processing core is kept independent of Tauri, and the
planned PDF pipeline.

## Development instructions

### Prerequisites

- Rust (pinned via [`rust-toolchain.toml`](rust-toolchain.toml))
- Node.js (version pinned in [`.nvmrc`](.nvmrc))
- [pnpm](https://pnpm.io/) (`corepack enable pnpm`)

### Build and test the Rust workspace

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

### Run the CLI

```bash
cargo run -p museion-binarize-cli -- --help
```

### Provide PDFium

This section is for **running from source**. If you downloaded a
packaged release instead (see "Download" above), PDFium is already
bundled inside it — skip this section entirely.

PDF rendering needs a PDFium dynamic library. When building from
source, it is not bundled, not committed to this repository, and never
downloaded at runtime — you supply it once. See
[docs/pdfium.md](docs/pdfium.md).

```bash
export MUSEION_PDFIUM_LIBRARY=/path/to/libpdfium.dylib
```

### Command-line usage

```bash
# Inspect a document: pages, geometry, rotation, render sizes
museion-binarize inspect input.pdf

# Measure a document through the real pipeline without writing an output
# PDF — useful for choosing settings before a full conversion
museion-binarize analyze input.pdf --dpi 300 --method otsu --json --pretty

# Sample a handful of pages through the real pipeline and extrapolate an
# experimental output-size estimate, without a full conversion
museion-binarize estimate input.pdf --dpi 400 --method sauvola --samples 8

# Convert to a bilevel CCITT Group 4 PDF
museion-binarize process input.pdf --output output.pdf \
  --method sauvola --dpi 400 --validate render-all

# Save a PNG preview of one processed page (one-based page numbers)
museion-binarize preview input.pdf --page 12 --output preview.png

# Benchmark binarization fidelity against pixel-accurate ground truth
# (no PDF/PDFium needed for the raster benchmark level)
museion-binarize benchmark run \
  --dataset test-data/benchmark/synthetic-v1/dataset.toml \
  --profile test-data/benchmark/profiles/baseline.toml
```

Useful options: `--method otsu|sauvola|manual`, `--threshold`, `--sauvola-k`, `--sauvola-window`, `--contrast`, `--median-denoise`, `--background-normalization`, `--despeckle off|conservative|strong`, `--overwrite`, `--pdfium-library`, `--pages` (`analyze` only, e.g. `1,3,8-12`), `--json`/`--pretty`/`--quiet`/`--report`. Progress goes to stderr; the result (human or `--json`) goes to stdout. See [`docs/cli.md`](docs/cli.md) for the full command surface, exit codes, and stdout/stderr contract, and [`docs/reporting.md`](docs/reporting.md) for JSON report schemas.

The source file is never modified, and the destination is only written after a complete, validated document exists.

### Run the desktop application

```bash
pnpm install
pnpm --filter museion-binarize-desktop tauri dev
```

## Contributing

Contributions are welcome. Please read [`CONTRIBUTING.md`](CONTRIBUTING.md)
before opening a pull request — it covers formatting, testing, and rules
around fixture provenance and unsubstantiated claims. See
[`SECURITY.md`](SECURITY.md) to report a vulnerability.

## Citation

If you use this software, please cite it using the metadata in
[`CITATION.cff`](CITATION.cff).

## Author and maintainer

Museion Binarize is created and maintained by **Pei Haoran** under the
**Museion Project** organization. See [`AUTHORS.md`](AUTHORS.md).

## License

Licensed under either of

- MIT License ([`LICENSE-MIT`](LICENSE-MIT))
- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))

at your option.
