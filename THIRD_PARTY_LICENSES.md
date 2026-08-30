# Third-Party Licenses

M PDF Processor is dual-licensed under MIT OR Apache-2.0 (see
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE)). This
project also uses third-party open-source software. This file records
attributions for all bundled or statically linked dependencies. The complete
Rust and Node transitive inventory is generated per target in the
deterministic SPDX 2.3 SBOM shipped with release assets.

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
| `reqwest` (rustls) | HTTPS transport for opt-in cloud modes | MIT OR Apache-2.0 |
| `keyring` | OS credential store (Keychain / Credential Manager / Secret Service) | MIT OR Apache-2.0 |
| `zeroize` | Wiping secret buffers on drop | MIT OR Apache-2.0 |
| `base64` | Encoding page images for the cloud OCR request body | MIT OR Apache-2.0 |

Transitive Rust dependencies are resolved from `Cargo.lock`; `cargo deny
check` is the license-policy gate. They are not vendored as source here.

## Cloud OCR providers

No model, SDK, or vendor client library is bundled for either cloud mode.
`gemini-byok` is a direct HTTPS request over the `reqwest`/`rustls` stack
already listed above; there is no Google SDK in the dependency graph, and no
weights are downloaded, cached, or redistributed by this project.

Using a cloud mode makes **you** the API customer. Google's terms, pricing and
data-handling policy govern your key and the page images you choose to upload;
this project neither accepts them on your behalf nor proxies your traffic.
Review them before uploading material you do not own. Local OCR is the default
and involves none of this.

M PDF Cloud OCR has no production service in this build; see
[`docs/ocr-providers.md`](docs/ocr-providers.md) for the outstanding blockers.

## Local OCR: Tesseract and tessdata_best

Local text recognition runs through an out-of-process sidecar
(`scripts/ocr/mpdf_ocr_sidecar.py`) that drives **Tesseract 5** in LSTM mode
with the **`tessdata_best` 4.1.0** trained data. Both are Apache-2.0:

| Component | Upstream | License |
|---|---|---|
| Tesseract OCR engine | <https://github.com/tesseract-ocr/tesseract> | Apache-2.0 |
| `tessdata_best` trained data (`grc`, `deu`, `eng`, `lat`, `ell`, `osd`) | <https://github.com/tesseract-ocr/tessdata_best> | Apache-2.0 |

Every model file the application will load is pinned by URL, byte size and
SHA-256 in [`distribution/ocr-models/manifest.toml`](distribution/ocr-models/manifest.toml),
which also records each set's license and whether it may be redistributed.
`scripts/ocr/provision_models.py` verifies those digests before writing the
sidecar-facing `manifest.json`, and the sidecar re-verifies every file it is
about to use on every page.

**No OCR model is committed to this repository, and the application never
downloads one at runtime.** Models are provisioned out of band by an operator
or staged into a release bundle at packaging time.

Evaluated but **not** shipped, with reasons recorded in
[`docs/ocr-engines.md`](docs/ocr-engines.md):

- **RapidOCR + `ch_PP-OCRv4`** (Apache-2.0 code; model license not separately
  pinned) — a Chinese/English recognizer, rejected as the direct cause of the
  Greek and German failures it was producing.
- **PaddleOCR PP-OCRv5** (Apache-2.0 code; weights fetched by the library and
  not pinned or hashed here) — its `el` model is Modern Greek and cannot
  represent polytonic text. Development comparator only.
- **Kraken** (Apache-2.0 code; **per-model licenses unconfirmed**) — not
  evaluated. Each published model's license, version and SHA-256 must be
  confirmed individually before it could be considered.

## Fonts

[`crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf`](crates/mpdf-core/assets/fonts/NotoSans-Regular.ttf)
is used for the invisible text layer written into searchable output, and by
the OCR gold evaluation to render its fixtures. It is licensed under the SIL
Open Font License 1.1; the full text is committed beside it as
[`OFL.txt`](crates/mpdf-core/assets/fonts/OFL.txt).

## PDFium

M PDF Processor uses [PDFium](https://pdfium.googlesource.com/pdfium/) to
rasterize source PDFs. PDFium is licensed BSD-3-Clause with Apache-2.0
components; prebuilt binaries commonly come from the
[`pdfium-binaries`](https://github.com/bblanchon/pdfium-binaries) project,
whose packaging is MIT.

**No PDFium binary is committed to this repository, and the application
never downloads one at runtime.** The library is supplied separately by a
developer or packager; official builds fetch it only at build time. See [`docs/pdfium.md`](docs/pdfium.md).

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

Frontend dependency declarations are captured in `apps/desktop/package.json`
and `pnpm-lock.yaml`; the release SBOM records the actual installed desktop
build graph (runtime, development, and optional dependencies) resolved by
pnpm, with package metadata merged from the workspace installation. It is
not a claim that every platform-specific or optional lockfile entry is
installed in every build. Packaged CLI and desktop artifacts carry the root
MIT/Apache notices,
the PDFium license texts, and this notice file.
