# PDFium bundling (Milestone 7A)

How official packaged builds carry their own trusted PDFium, so a normal
end user never sets `MPDF_PDFIUM_LIBRARY`, searches for a library, or
launches from a terminal. See [`pdfium.md`](pdfium.md) for the
unchanged developer-setup story and the resolver's full precedence
order, and [`releasing.md`](releasing.md) for how this fits into the
release pipeline.

## The resolver was already right

Milestone 2's `pdfium_backend::resolve_library` already implements
exactly the "explicit override -> packaged resource -> [system, if
allowed] -> reject" precedence Milestone 7A needs (see
`crates/mpdf-core/src/pdfium_backend.rs`), including a
`LibrarySource::BundledResource` candidate (`<exe_dir>/resources/<lib>`)
and a `LibrarySource::ExecutableAdjacent` candidate (`<exe_dir>/<lib>`)
— both already anchored to the running executable, never the current
working directory. Milestone 7A did not change this precedence or
reintroduce any CWD search; it only made sure packaging actually puts a
verified library where the resolver (or, on macOS, Tauri's own resource
API) can find it.

## CLI distribution: unchanged code, new packaging

The CLI resolver's `ExecutableAdjacent` candidate already matches the
milestone's desired archive layout exactly:

```
mpdf-cli-<version>-<os>-<arch>/
  mpdf            (or mpdf.exe)
  libpdfium.dylib              (or pdfium.dll / libpdfium.so)
  LICENSE-MIT
  LICENSE-APACHE
  THIRD-PARTY-NOTICES.md
  README.txt
```

No core code changed to support this — `scripts/distribution/package_cli.py`
only assembles the archive; `resolve_library` finds the sibling library
automatically. Verified directly: extracting a packaged archive to a
fresh location and running `inspect`/`process` with no
`MPDF_PDFIUM_LIBRARY` set actually works — see
`docs/desktop-testing.md`'s "Milestone 7A" section for the exact
transcript.

## Desktop distribution: one new precedence layer

The desktop app's macOS bundle places resources at
`Contents/Resources/`, which is a *different* location from
`Contents/MacOS/resources/` that the core resolver's generic
`BundledResource` candidate checks (that candidate assumes the simpler,
same-directory-as-executable layout Windows/Linux packaging typically
uses). Rather than teach the core resolver to guess every platform's
bundle layout, the desktop backend resolves its own bundled PDFium path
once at startup using **Tauri's own resource-directory API**
(`app.path().resolve(..., BaseDirectory::Resource)` — see
`apps/desktop/src-tauri/src/lib.rs`'s `.setup()` closure), which already
knows the correct location for whichever platform it's running on. That
resolved path (or `None` in a development run with no bundled resource)
is stored once in `AppState` and threaded into every worker-thread
`PdfiumConfig` construction (`apps/desktop/src-tauri/src/worker.rs`'s
`pdfium_config`).

**Precedence, implemented in `worker::pdfium_config`:**

1. `MPDF_PDFIUM_LIBRARY`, if set — unchanged development/support
   override, still honored even in a packaged build (useful for
   diagnosing a bad bundled copy without needing a whole new release).
2. The trusted bundled resource, if Tauri found one at startup.
3. Otherwise, `PdfiumConfig::default()` — the core resolver's own
   unchanged search (executable-adjacent, `MPDF_ALLOW_CWD_PDFIUM`
   development tree, system library only if explicitly allowed).

Verified directly against a real build: `cargo tauri build --bundles
app` produces
`M PDF Processor.app/Contents/Resources/libpdfium.dylib`; a copy of
that `.app` launched outside the repository, with no environment
variable set, starts without crashing (see
`docs/desktop-testing.md`). Full interactive click-through (open a
document, convert, verify output) remains a human-runtime-verification
step — see that document for exactly what is and is not covered by
automated evidence.

## `tauri.conf.json` vs. `tauri.dist.conf.json`

`tauri::generate_context!` (invoked from `src-tauri/src/main.rs`) reads
`bundle.resources` at **compile time**, for every `cargo build` /
`cargo clippy` / `cargo test` of the desktop crate — not only for
`tauri build`. If `bundle.resources` names a glob, that glob must match
on disk or compilation fails, even for an ordinary lint/test run that
has no reason to know about PDFium at all. Ordinary CI (`ci.yml`) runs
`cargo clippy --workspace --all-targets` / `cargo test --workspace` on a
clean checkout with nothing staged under `resources/pdfium/`, so the
base config must never require that glob to match.

The `resources.pdfium/*` entry therefore lives in a separate overlay,
`apps/desktop/src-tauri/tauri.dist.conf.json`, not in the base
`tauri.conf.json`:

```json
// tauri.dist.conf.json
{
  "bundle": {
    "resources": {
      "resources/pdfium/*": "./"
    }
  }
}
```

Only the distribution packaging workflow (`build-distribution.yml`)
merges it in, via Tauri's own config-overlay mechanism:

```
pnpm tauri build --target <target-triple> --config src-tauri/tauri.dist.conf.json
```

`--config` merges the given file over the base `tauri.conf.json` (a
deep merge, last value wins) for that one invocation only — plain
`cargo build`/`clippy`/`test` never see it, since they read
`tauri.conf.json` directly and no `--config`/`TAURI_CONFIG` is ever set
outside the distribution workflow.

`resources/pdfium/` (gitignored — never commit a staged binary) is
populated by `scripts/distribution/stage_desktop_pdfium.py
<target-triple>` immediately before `tauri build`, which fetches the
pinned, checksum-verified library for the target being built (via
`fetch_pdfium.py`) and copies it there under its own platform-correct
filename, failing closed (non-zero exit) if the target isn't pinned or
a checksum doesn't match. Only one target's library is staged at a
time, so the same static glob works for every platform without per-OS
config duplication. The target mapping `"./"` places the file directly
at the bundle's resource root — verified by inspecting the actual built
bundle (see above), not assumed from documentation, after an initial
attempt at a `"resources/"` subfolder mapping was found (by inspection)
to nest the file one level too deep.

Regression coverage for this split lives in
`scripts/distribution/test_distribution.py`'s `TauriResourceConfigTests`:
it asserts the base config has no `bundle.resources`, the dist overlay
does reference `resources/pdfium/*` and carries nothing else, staging
fails closed for an unpinned target, and no `.dylib`/`.dll`/`.so` is
ever tracked in git.

## No production runtime dependency on an environment variable

A packaged build's primary path never requires
`MPDF_PDFIUM_LIBRARY` to be set — the bundled resource is used
automatically. If the bundled resource is somehow missing or invalid
(a broken package), the failure is the core resolver's existing
structured `PdfiumNotFound`/`PdfiumLoadFailed` error, surfaced through
the desktop app's existing error presentation
(`commands/document.rs::pdfium_status`, `errors::classify_core_error`)
— never a request to "set an environment variable and relaunch from
Terminal."
