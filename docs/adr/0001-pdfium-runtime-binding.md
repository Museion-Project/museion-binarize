# ADR 0001 — PDFium as a dynamically bound input renderer

**Status:** Accepted (2026-08-07). Proposed during Milestone 2 and accepted
only after the end-to-end tests in `crates/mpdf-core/tests/pdf_pipeline.rs`
passed against a real PDFium build on the verifying host.

## Context

M PDF Processor must rasterize arbitrary scanned PDFs. Writing a PDF
parser and rasterizer is out of scope; PDFium (the renderer inside
Chromium) is the mature, permissively licensed option.

## Decision

### PDFium is used only as the input renderer

PDFium reads and rasterizes source pages. It never writes output: output
PDFs are rebuilt by this project (see [ADR 0002](0002-bilevel-pdf-output.md)).
Keeping PDFium on the input side alone means the bytes we publish are ones
we fully control and can test.

### `pdfium-render` rather than hand-written FFI

`pdfium-render` 0.9.3 (MIT OR Apache-2.0) wraps PDFium's C API with
lifetimes and `Result`s. Hand-writing this FFI would mean maintaining a
large `unsafe` surface for no benefit. All of it is confined behind
`pdfium_backend.rs` and `document_session.rs` (Milestone 2's
`renderer.rs`, replaced in Milestone 3 — see
[`pdf-pipeline-session.md`](../pdf-pipeline-session.md)); no
`pdfium-render` type appears in any public signature elsewhere in the
core.

Feature selection is explicit rather than defaulted:

```toml
pdfium-render = { version = "0.9.3", default-features = false,
                  features = ["pdfium_7881", "image_025", "thread_safe"] }
```

* `image_025` pins the same `image` 0.25 generation the core already uses,
  so no second, incompatible `image` version enters the tree.
* `thread_safe` guards the bindings with a mutex.
* `static`, WASM, V8/XFA, Skia, and bindgen features are all left off.

### Dynamic, not static, binding

The library is loaded at runtime from a path this project resolves. Static
linking would require building PDFium from source per platform — a large,
slow toolchain problem that would dominate this milestone. Dynamic binding
also lets a user point the application at an audited binary of their own.

### Selected build

PDFium build **7920** (`151.0.7920.0`), from the `pdfium-binaries`
distribution project. `pdfium-render`'s newest binding set is
`pdfium_7881`; 7920 is newer. Rather than assume the ABI matched, it was
verified empirically — a smoke test bound the library and the full
integration suite then exercised open, inspect, render, and re-open. See
`third_party/pdfium/manifest.toml` for the exact asset and its checksum.

### Library file names

| Platform | File name |
|---|---|
| macOS | `libpdfium.dylib` |
| Windows | `pdfium.dll` |
| Linux | `libpdfium.so` |

### Resolution order

1. an explicit path passed by the caller (`--pdfium-library`, GUI setting);
2. the `MPDF_PDFIUM_LIBRARY` environment variable;
3. an application-relative bundled resource directory (`resources/`);
4. a library next to the running executable;
5. the system library search path — **only** when explicitly allowed.

Steps 1 and 2 are exact: if the named file is missing, resolution fails.
It never falls back to a different binary, because silently running an
unknown PDFium is worse than a clear error. Failures list every location
tried.

### Provenance and verification

`third_party/pdfium/manifest.toml` records, for each asset actually
verified: upstream project, build identifier, filename, target triple,
where the copy came from, its SHA-256, size, and both licenses. Checksums
are computed locally with `shasum -a 256`; no value is transcribed from a
release page, and no entry exists for a platform whose asset has not been
hashed on that platform.

### Licensing obligations

PDFium is BSD-3-Clause plus Apache-2.0 components; the `pdfium-binaries`
packaging is MIT. Both texts are committed as
`third_party/pdfium/LICENSE-PDFIUM` and `LICENSE-DISTRIBUTION`. Neither is
copyleft, so both are compatible with this project's MIT OR Apache-2.0
dual license. Any redistribution of a PDFium binary must ship these
notices.

### No runtime downloading

The application never downloads a library. A binary fetched silently at
runtime is unverifiable executable code from the network. Provisioning is
a deliberate, documented developer or packaging step
(see [`../pdfium.md`](../pdfium.md)).

### Binaries are not committed

A ~7.7 MB platform-specific binary per target would bloat the repository
permanently, and Git is the wrong place to distribute signed executables.
The development copy lives in the gitignored `target/pdfium/<triple>/`.

That path is resolved against the **current working directory**, which
makes it a library-injection vector: anyone who can write into a
directory the user runs the tool from could choose the native code that
gets loaded. It is therefore not part of ordinary resolution. It is
consulted only in a debug build *and* only when `MPDF_ALLOW_CWD_PDFIUM=1`
is set explicitly; release builds ignore it entirely. Every other search
location is anchored to the running executable or the operating system.

### Thread safety and concurrency

**PDFium is initialized once per process.** `FPDF_InitLibrary` /
`FPDF_DestroyLibrary` are process-global: binding twice returns
`PdfiumLibraryBindingsAlreadyInitialized`, and dropping one binding while
another is live tears the library out from under it. This was not
theoretical — an earlier design that bound per `PdfRenderer::open` failed
exactly this way, first as that error and then as a `SIGSEGV`.

The core therefore holds the session in a process-wide `OnceLock`, guarded
by a mutex during initialization. This is an *immutable* singleton, not
global mutable configuration: it is written once and never mutated, and
all caller-facing configuration still travels explicitly through
`PdfiumConfig`. Asking for a *different* explicit library once one is
bound is reported as an error rather than silently satisfied.

Rendering is **sequential**. No page-level parallelism is introduced in
this milestone, because PDFium's safety and memory behaviour under
concurrency have not been measured here. Integration tests serialize
PDFium access with a mutex for the same reason.

### Current platform limitations

Only **aarch64-apple-darwin** has been built *and run* against a real
PDFium binary. The code is written to be cross-platform and the resolution
logic covers macOS, Windows, and Linux names, but Windows and Linux are
**unverified at runtime** — no asset has been obtained or hashed for them,
so `manifest.toml` deliberately has no entry for them. See
[`../limitations.md`](../limitations.md).
