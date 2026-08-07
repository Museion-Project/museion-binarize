English | [简体中文](README.zh-CN.md)

# Museion Binarize

**Museion Binarize** is an open-source, cross-platform application for
converting scanned scholarly books into clean and compact bilevel PDFs.

**Status: Phase 1 — early development.** A complete local CLI pipeline
exists (`inspect`, `analyze`, `process`, `preview`, with versioned JSON
reports), and the desktop GUI is now wired to the same pipeline (open,
preview, configure, convert, cancel — see [`docs/desktop.md`](docs/desktop.md)).
End-to-end behavior is currently verified only on a provisioned macOS
environment; see [`docs/desktop-testing.md`](docs/desktop-testing.md) for
the native desktop acceptance record. See
[`docs/limitations.md`](docs/limitations.md) for exactly what this
repository can and cannot do today.

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

## Planned Phase 1 features

- macOS, Windows, and Linux desktop support.
- Fully local PDF processing — no upload, no network dependency for
  conversion.
- Deterministic **Otsu**, **Sauvola**, and **manual** thresholding methods
  for binarization.
- True 1-bit (bilevel) PDF reconstruction from scanned page images.
- **CCITT Group 4** compression for compact bilevel output.
- Both a graphical desktop application and a command-line interface, sharing
  the same processing core.
- A reproducible benchmarking framework for evaluating output quality.

None of these features are implemented yet in this repository; Phase 1 is
just beginning. See [`docs/roadmap.md`](docs/roadmap.md) for the
milestone-by-milestone plan.

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

PDF rendering needs a PDFium dynamic library. It is not bundled, not committed to this repository, and never downloaded at runtime — you supply it once. See [docs/pdfium.md](docs/pdfium.md).

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

# Convert to a bilevel CCITT Group 4 PDF
museion-binarize process input.pdf --output output.pdf \
  --method sauvola --dpi 400 --validate render-all

# Save a PNG preview of one processed page (one-based page numbers)
museion-binarize preview input.pdf --page 12 --output preview.png
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
