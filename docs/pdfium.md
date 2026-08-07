# Providing PDFium

Museion Binarize uses [PDFium](https://pdfium.googlesource.com/pdfium/) to
rasterize source PDFs. PDFium is a large C++ library that is **not** bundled
with the `pdfium-render` crate, **not** committed to this repository, and
**never downloaded by the application at runtime**. You provide it once; the
application then loads it dynamically.

See [`adr/0001-pdfium-runtime-binding.md`](adr/0001-pdfium-runtime-binding.md)
for why.

## Where the application looks

In order:

1. an explicit path you pass (`--pdfium-library <path>`);
2. the `MUSEION_PDFIUM_LIBRARY` environment variable;
3. `resources/<library>` next to the executable (packaged applications);
4. `<library>` next to the executable;
5. the system library search path — **only** with `--allow-system-pdfium`.

Options 1 and 2 are exact. If the file you named is missing, the run fails
and lists what it tried; it will never quietly load some other PDFium.

The library file is `libpdfium.dylib` on macOS, `pdfium.dll` on Windows, and
`libpdfium.so` on Linux.

Every command that uses PDFium reports which library it actually loaded, so
you can always tell.

## Developer setup

Obtain a PDFium dynamic library for your platform, then either place it at
`target/pdfium/<target-triple>/<library>` or point
`MUSEION_PDFIUM_LIBRARY` at it:

```bash
export MUSEION_PDFIUM_LIBRARY=/path/to/libpdfium.dylib
cargo run -p museion-binarize-cli -- inspect some.pdf
```

Common sources of a prebuilt binary:

* the [`pdfium-binaries`](https://github.com/bblanchon/pdfium-binaries)
  project, which publishes builds for each platform;
* an existing local install — many Python and CLI PDF tools (for example
  `pypdfium2`, which `ocrmypdf` depends on) already ship a `libpdfium`
  you can reuse for development.

**Record what you used.** Before relying on a binary, compute its checksum:

```bash
shasum -a 256 /path/to/libpdfium.dylib
```

and add an entry to [`../third_party/pdfium/manifest.toml`](../third_party/pdfium/manifest.toml)
if that asset is not already listed. Never add an entry for a platform
whose asset you have not actually obtained and hashed — an invented
checksum is worse than none.

## Verified platforms

| Target | Built | Run against real PDFium |
|---|---|---|
| `aarch64-apple-darwin` | yes | **yes** (build 7920) |
| `x86_64-apple-darwin` | not verified | no |
| `x86_64-pc-windows-msvc` | not verified | no |
| `x86_64-unknown-linux-gnu` | not verified | no |

The architecture is cross-platform and the resolution logic covers all
three operating systems, but only Apple Silicon macOS has actually been
exercised end to end. See [`limitations.md`](limitations.md).

## Licensing

PDFium is BSD-3-Clause with Apache-2.0 components; the `pdfium-binaries`
packaging is MIT. Both texts are committed under
[`../third_party/pdfium/`](../third_party/pdfium/). If you redistribute a
PDFium binary with this application, you must ship those notices.
