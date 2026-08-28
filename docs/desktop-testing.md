# Testing the desktop application

This is a companion to [`desktop.md`](desktop.md) (architecture) and
[`testing-pdf-pipeline.md`](testing-pdf-pipeline.md) (the sibling
core/CLI testing document). It states exactly what has been verified for
Milestone 4, and — just as importantly — what has not, so this document
cannot be mistaken for evidence of manual GUI testing that did not
happen.

## Native macOS acceptance: passed (2026-08-07)

Native desktop acceptance testing has been performed on a provisioned
Apple Silicon macOS machine, launching the real Tauri application (not
the browser-only check below) with:

```bash
MPDF_PDFIUM_LIBRARY="/Users/theo/AI 工作流/museion-binarize/target/pdfium/aarch64-apple-darwin/libpdfium.dylib" \
  pnpm --dir apps/desktop tauri dev
```

against the pinned PDFium build recorded in
`third_party/pdfium/manifest.toml` (verified present and SHA-256-matching
before this run). The acceptance scenario list in this document (see
"Native acceptance checklist: result" below) was worked through on the
running native window, including a real-world long-document conversion:

**Observed baseline (single run, real scholarly scan — not a synthetic
fixture):**

| | |
|---|---|
| Document | 100-page scholarly scanned PDF |
| Input | 51.8 MB |
| Output | 6.7 MB |
| Method | Sauvola |
| DPI | 400 |
| Total processing time | ≈ 600 seconds |
| Per-page time | ≈ 6 seconds/page |
| Size reduction | ≈ 87.1% |
| Input/output size ratio | ≈ 7.7 : 1 |

**This is one observed data point on one document on one machine, not a
general performance or compression guarantee.** Processing time and
compression ratio both depend heavily on page content (a densely
inked scan compresses less than a sparse one), page pixel dimensions at
the chosen DPI, and the host machine's CPU. Do not read "~6s/page" or
"~87% reduction" as a promise for other documents, and do not cite this
table as benchmark data — no benchmark methodology exists yet (see
[`benchmarking.md`](benchmarking.md)); this is a single acceptance-test
observation, recorded here for traceability, not a measured claim about
typical performance.

Independently corroborated from the file system (not solely taken on
report): the reported input and output files exist at the reported
sizes, and the output reopens successfully via `inspect` reporting
`page_count: 100` and valid document structure.

## What has been verified

**Automated, non-PDFium (`cargo test -p mpdf-desktop`):**
DTO conversion and validation (`settings.rs`), error classification and
the guarantee that no error DTO ever serializes a password
(`errors.rs`) — 13 tests, all passing, requiring no PDFium library.

**Automated, provisioned PDFium (`cargo test --test pdf_pipeline --
--ignored`, run with `MPDF_PDFIUM_LIBRARY` pointed at a real library):**
the full Milestone 2/3 integration suite, plus two Milestone 4 additions
run against the same provisioned macOS Apple Silicon environment:

- `process_with_open_session_reuses_an_already_open_session_without_reopening_the_source`
  — the desktop app's conversion entry point, exercised through a
  session opened the way the app opens one.
- `cli_and_gui_entry_points_produce_byte_identical_output_for_identical_settings`
  — the CLI's and the desktop app's entry points, run against the same
  fixture and settings, assert byte-identical output PDFs.

**Frontend (`pnpm --dir apps/desktop test`, Vitest + React Testing
Library, Tauri `invoke`/`listen`/dialog calls mocked):** idle state
(Open PDF enabled, Convert disabled), document open (details and page
sidebar appear), the password-required flow, conditional settings
controls (Otsu hides the manual threshold, Manual shows it, Sauvola
shows its own controls), preset-to-Custom transition on a manual change,
and a structured error panel rendering a backend `UiErrorDto` — 7 tests,
all passing.

**Static analysis:** `cargo fmt --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo deny check`, `cargo check -p
mpdf-desktop`, `pnpm lint`, `pnpm typecheck`, `pnpm build` —
all clean as of this milestone's last commit. A `grep` sweep of
`apps/desktop` and every core/CLI crate for `http://`, `https://`,
`fetch(`, `axios`, `reqwest`, `Command::new`, and `shell` found no
matches outside documentation links.

## Partial visual check (not a substitute for the real thing)

The environment this milestone was implemented in cannot open, click
through, or screenshot an actual **native Tauri window**. It can,
however, serve the frontend alone (`pnpm --dir apps/desktop dev`, the
plain Vite dev server Tauri's `beforeDevCommand` also runs) and view it
in an ordinary browser tab. This was done once, for the idle screen
only: it confirms the calm, non-dashboard layout renders without
clipping or overflow at the CSS minimum width (900px), in both light and
dark `prefers-color-scheme`, and that the app does not crash outside a
real Tauri context (the console shows expected `invoke`/`listen`
failures — there is no `window.__TAURI_INTERNALS__` in a plain browser
tab — caught and handled gracefully rather than blanking the window).

This is **not** a substitute for running the real app: every screen past
the idle state requires `open_document`, `render_preview`, and the other
real Tauri commands, which only exist inside the actual native webview
with a provisioned PDFium library — none of that was exercised this way,
and no other screen was checked.

## Native acceptance checklist: result

The Milestone 4 specification's manual verification section (§46–47)
listed the following as required before this milestone could be
considered done. As of the 2026-08-07 native run recorded above, this
checklist has been worked through on the real, running native
application and reported passing by the operator who ran it:

- launching the real native Tauri window with provisioned PDFium;
- opening a real PDF through the native file dialog;
- page count and document metadata;
- thumbnail rendering and navigation;
- a long-document sidebar (the 100-page document above);
- original preview;
- processed preview;
- `/Rotate 0/90/180/270` visually, including square/asymmetric
  orientation markers;
- Otsu, Sauvola, and Manual-threshold preview;
- settings changes not letting a stale preview response overwrite a
  newer one;
- choosing output through the native save dialog;
- a real multi-page conversion with live progress, a responsive UI
  during processing, and a correct completion panel — the 100-page,
  Sauvola, 400 DPI run recorded above;
- cancelling a sufficiently long conversion after it has begun, with no
  partial output or temporary files left behind, and a new job able to
  start afterward;
- equivalent input/output path aliases being rejected;
- the password-protected PDF flow;
- error-state UI;
- light mode and dark mode;
- screenshots of idle / loaded / preview / processing / completion /
  error states;
- memory behavior on the 100-page fixture, with no full-resolution
  eager thumbnail cache.

Only the long-document conversion scenario produced a specific
quantitative result, recorded above; this document does not restate
per-item screenshots or numeric detail for the other scenarios beyond
the operator's report that they passed. If a specific one of them needs
independent re-confirmation later, treat this checklist as the list of
what to re-run, not as a substitute for doing so.

**Milestone 4 is marked Complete in [`roadmap.md`](roadmap.md)** on the
strength of this native run together with the automated coverage above.
If a regression is later found in any of these scenarios, update this
document with the actual observation and reopen the relevant status
rather than editing this record silently.

## Milestone 5 additions

The size-estimation feature (backend estimator, `estimate` CLI command,
and the desktop "Estimate" panel) has automated coverage but **has not
been exercised on a live native Tauri window** in this environment, for
the same reason Milestone 4's own implementation could not be: this
environment cannot open, click through, or screenshot an actual native
window. The same verification gap already documented above for Milestone
4 applies here — it is not a new gap, just an additional feature that
falls into it.

**What has been verified for Milestone 5:**

- Backend: ordinary (non-PDFium) unit tests for deterministic sampling,
  quartiles, mixed page-size extrapolation, container-overhead
  measurement, cancellation, DTO round-tripping, and outlier
  classification, plus real-PDFium-backed tests for `estimate` against
  every binarization method, mid-run cancellation, and the two synthetic
  accuracy fixtures (±25%/±15% thresholds) — see
  [`testing-pdf-pipeline.md`](testing-pdf-pipeline.md) and
  [`size-estimation.md`](size-estimation.md) for the accuracy results.
- Frontend: `reducer.test.ts` covers every `EstimateState` transition
  (idle → running → ready → stale → failed, including the
  request-id-staleness guard) as pure unit tests with no Tauri mocking
  needed. `App.test.tsx` covers the Estimate button triggering a request,
  the result rendering with its range and experimental label, and a
  settings change marking a ready estimate stale without discarding the
  previous value — all with `invoke` mocked, not a real backend.

**What has not been verified:** the Estimate panel has not been clicked
in a real native window against a real PDFium-provisioned document; the
"cancel an in-flight estimate by starting a conversion" and "cancel an
in-flight conversion's prior-estimate cache invalidation" interactions
have automated coverage at the Rust level (`commands/estimate.rs`,
`commands/processing.rs`) but not a live-GUI observation; and no new
100-page real-document estimate-vs-actual comparison has been recorded
(the Milestone 4 100-page baseline above predates this feature).

To perform that verification, launch the real application the same way
Milestone 4's acceptance run did:

```bash
MPDF_PDFIUM_LIBRARY="/Users/theo/AI 工作流/museion-binarize/target/pdfium/aarch64-apple-darwin/libpdfium.dylib" \
  pnpm --dir apps/desktop tauri dev
```

and work through: clicking Estimate before any settings change, changing
a setting afterward and confirming the previous estimate stays visible
but is labeled outdated, converting without ever requesting an estimate
(Convert must not be blocked), and converting after a matching estimate
to confirm the completion report's estimate-vs-actual comparison appears.
If that is done, record the observation here in the same style as the
Milestone 4 baseline above, rather than silently assuming this document
already covers it.

## Automatic table of contents (bookmarks v2)

The automatic bookmark path has full automated coverage and **has not been
exercised on a live native Tauri window** in this environment, for exactly the
reason documented above for Milestones 4 and 5: this environment cannot open,
click through, or screenshot a real native window. This is the same standing
gap, not a new one.

**What has been verified automatically:**

- Backend command layer (`commands/auto_bookmarks.rs`): path validation, the
  stable error codes for a stale or missing document, and the atomic
  single-slot claim that refuses a second concurrent run.
- Worker layer (`worker.rs`): a safe refusal returns a *result* rather than an
  error and writes no PDF (and reports only the `analyzing_toc`/`aligning`
  stages); regeneration over existing candidates is refused until authorized;
  a cancelled run leaves no candidates, report, or output behind. These need
  no PDFium because nothing reaches the writer.
- Review commands (`commands/bookmarks.rs`): bounded path and candidate-id
  validation, structured `UiErrorDto` failures.
- Frontend (`ReviewWorkbench.test.tsx`): the native pickers filling both
  paths, one button starting a run with the exact request payload, stage
  rendering, the success summary, a safe refusal rendered as a normal result
  panel (no `role="alert"`), cancellation appearing only while a run is in
  flight, a backend failure rendered as an alert without losing the panel, the
  `auto_confirmed` filter, the distinct "Added automatically" versus
  "Confirmed by you" labels, and the score/alignment evidence for a selected
  candidate. Components still reach IPC only through `src/lib/tauri.ts`.

**What has not been verified:** no live-window run; no observation of the real
native folder/file pickers; no real PDF written from the desktop application.
The PDFium-backed write-back path is covered by `#[ignore]`d integration tests
(`crates/mpdf-core/tests/auto_bookmarks_pdf.rs`,
`crates/mpdf-cli/tests/bookmarks_cli.rs`), which require a provisioned PDFium
library and are reported as *ignored*, never as passed, in an ordinary run.

To perform the live verification, launch the real application as the
Milestone 4 acceptance run did, then: open a PDF, choose its MDP package and
an output path, click **Add bookmarks automatically**, and confirm the stage
text, the summary counts, the reloaded bookmark tree, and the written PDF in a
real reader. Repeat with a document that has no printed contents list to
confirm the refusal panel. Record the observation here rather than assuming
this document already covers it.

## Milestone 7A: packaged-build verification

See [`distribution.md`](distribution.md), [`pdfium-bundling.md`](pdfium-bundling.md),
and [`releasing.md`](releasing.md) for the full distribution-foundation
work. This section records exactly what was, and was not, verified
during Milestone 7A's implementation.

### Verification-state table

| Target | Built | Packaged | PDFium bundled | Automated smoke | Human runtime | Signed | Notarized |
|---|---|---|---|---|---|---|---|
| macOS arm64 (`aarch64-apple-darwin`) | yes | yes (.app, .dmg) | yes, verified | yes | attempted — found broken, fixed, **re-verification pending** | ad-hoc | no |
| macOS x64 (`x86_64-apple-darwin`) | configured | configured | configured | not run this session | pending | no | no |
| Windows x64 (`x86_64-pc-windows-msvc`) | configured | configured | configured | not run this session | pending | no | no |
| Linux x86_64 (`x86_64-unknown-linux-gnu`) | configured | configured | configured | not run this session | pending | not applicable | not applicable |

"Configured" means the CI workflow (`.github/workflows/build-distribution.yml`)
targets that platform and its steps were validated by running the
equivalent commands directly where possible (macOS), but **this session
had no Windows or Linux machine available**, so those three rows'
"Built"/"Packaged"/"PDFium bundled" are the workflow's intended, not yet
CI-executed, behavior — the workflow itself was never actually
dispatched. This is recorded honestly as configured-but-unexercised,
not as verified.

### macOS arm64: "is damaged and can't be opened" bug found by human runtime testing, and fixed (2026-08-08)

The first real human runtime check of the packaged macOS arm64 build —
installing the `.dmg` from this milestone and double-clicking the
`.app` in Finder — did not pass. Finder reported:

> "M PDF Processor" is damaged and can't be opened. You should move it
> to the Trash.

This is a real defect, not a false alarm from an expected "unidentified
developer" warning. It was independently reproduced and diagnosed
without relying on the "damaged" dialog's own wording:

- `codesign --verify --deep --strict` failed, on **both** the installed
  `/Applications` copy and an untouched, freshly-mounted copy straight
  from the `.dmg` (ruling out extraction/transfer corruption as the
  cause), with:
  ```
  code has no resources but signature indicates they must be present
  ```
- Root cause: Rust's linker ad-hoc-signs each Apple Silicon Mach-O
  binary at build time (arm64 requires every binary to carry some
  signature), but `apps/desktop/src-tauri/tauri.conf.json` has no
  `bundle.macOS.signingIdentity` configured, so `tauri-bundler` never
  resigns the *whole app bundle* afterward. The packaged `.app`'s main
  executable therefore carries an embedded CodeDirectory that implies a
  signed structure with a resource envelope, but
  `Contents/_CodeSignature/CodeResources` — which should seal
  `Contents/Resources` (`libpdfium.dylib`, `icon.icns`) — was never
  generated. Gatekeeper treats that specific mismatch as bundle
  corruption, which is why the message says "damaged," not
  "unidentified developer."
- This gap existed even though the Milestone 7A "launch smoke test"
  below reported success: that test only confirmed the process stayed
  alive after being launched directly with `open`, which does not
  exercise the same Gatekeeper resource-envelope check a Finder
  double-click does. "Stays running after `open`" and "passes
  `codesign --verify --deep --strict`" are different claims, and only
  the second one is what Finder actually checks before allowing a
  double-click to proceed.

**Fix**: sign the whole `.app` bundle (ad-hoc, identity `-`, since no
Developer ID credentials exist for this project — see
[`releasing.md`](releasing.md), "Signing and notarization") *after*
`tauri build --bundles app` produces it, and build the `.dmg` from that
already-signed `.app` directly with `hdiutil` rather than through
Tauri's own dmg bundler — which was confirmed, empirically, to
recompile and re-bundle the `.app` from scratch as part of producing a
`.dmg`, discarding any signature applied beforehand. See
`scripts/distribution/sign_macos_app.py` and
`scripts/distribution/package_macos_dmg.py`, and the corresponding
steps added to `.github/workflows/build-distribution.yml`.

**Verified after the fix**, on this machine, for `aarch64-apple-darwin`:

- `codesign --verify --deep --strict` on the resigned `.app`: `valid on
  disk`, `satisfies its Designated Requirement`.
- A `.dmg` rebuilt from the resigned `.app` via
  `package_macos_dmg.py`, then mounted fresh and inspected: the `.app`
  inside it *also* passes `codesign --verify --deep --strict`.
- The resigned `.app` launches via `open` with no crash (same
  non-interactive check as the original Milestone 7A smoke test).
- `spctl -a` still reports `rejected` on the ad-hoc-signed bundle — this
  is expected and unchanged: ad-hoc signing fixes the damaged-bundle
  defect but does not (and cannot) satisfy Gatekeeper's full
  assessment or notarization, which still require real Developer ID
  credentials this project does not have.

**Not verified after the fix**: the fix has not yet been confirmed by
an actual Finder double-click on a fresh human machine (only
`open`/`codesign --verify` were used, which is real evidence the
specific defect is gone, but is not the same claim as "a human
double-clicked it and it opened"). Treat "Human runtime" as pending
re-verification, not as newly complete, until that happens. If it fails
again with the same or a different dialog, record the new observation
here rather than assuming this fix is the end of the story.

### macOS arm64: what was actually done, on this machine

```bash
python3 scripts/distribution/stage_desktop_pdfium.py aarch64-apple-darwin
cd apps/desktop && pnpm tauri build --bundles app
```

- **Bundle inspected directly**: `M PDF Processor.app/Contents/Resources/libpdfium.dylib`
  present (`file`: `Mach-O 64-bit dynamically linked shared library
  arm64`, matching the app binary's own architecture).
- An initial `tauri.conf.json` resources mapping placed the library one
  directory too deep (`Contents/Resources/resources/libpdfium.dylib`);
  this was caught by inspecting the actual built bundle, not assumed,
  and fixed — see [`pdfium-bundling.md`](pdfium-bundling.md).
- **`.dmg` built**: `M PDF Processor_0.1.0_aarch64.dmg`, 7,345,743
  bytes, SHA-256
  `a040ed1ccaf6c5a8c76fdf53516d96b05e8c82b9223e8ada597540d179f99bd9`.
- **Launch smoke test**: the built `.app` was copied to `/tmp` (outside
  the repository, simulating an install location), launched directly
  (not via `pnpm tauri dev`, no dev server, no Cargo/Node/pnpm on the
  launch path) with `MPDF_PDFIUM_LIBRARY` explicitly unset, and
  observed still running 3+ seconds later with no crash. This is
  automated evidence of a clean startup, not a substitute for the
  interactive checklist below.
- **Not done this session**: opening a real PDF, converting, cancelling,
  and the rest of the interactive checklist below — this requires
  clicking through the actual GUI, which this environment cannot do.
  Recorded as **pending human runtime verification**, matching the same
  honest gap already documented for Milestone 4/5's desktop testing
  where interactive GUI steps were involved.

### macOS arm64 CLI archive: real end-to-end verification

```bash
python3 scripts/distribution/package_cli.py \
  --target-triple aarch64-apple-darwin \
  --binary target/release/mpdf \
  --pdfium-library target/distribution/pdfium/aarch64-apple-darwin/libpdfium.dylib \
  --version 0.1.0 --out-dir /tmp/cli-release
```

Extracted to a fresh directory (not the repository), with
`MPDF_PDFIUM_LIBRARY` unset:

- `mpdf --help` / `info` — succeeded.
- `mpdf inspect rotations.pdf` — succeeded, reported
  `PDFium: .../libpdfium.dylib (directory containing the executable)`
  — confirming `LibrarySource::ExecutableAdjacent` resolution with no
  environment variable and no code change (see
  [`pdfium-bundling.md`](pdfium-bundling.md)).
- `mpdf process rotations.pdf --output out.pdf --method
  otsu --dpi 300` — succeeded; `mpdf inspect out.pdf`
  confirmed the 4-page output PDF was valid.
- `mpdf benchmark validate`/`run` against the committed
  `test-data/benchmark/synthetic-v1` suite — both succeeded (Level A
  benchmarking needs no PDFium at all, so this was expected to work
  regardless of bundling, and did).

This is real, reproducible evidence that CLI distribution's "PDFium
next to the executable" model works end to end — not a design claim.

### Human checklist for eventual full acceptance (macOS, and for
Windows/Linux once hardware is available)

Not performed this session; recorded here as the checklist to run when
a human operator (or a future session with GUI-interaction capability)
is available, per `docs/architecture.md`'s existing pattern:

- launch the packaged app outside the repository, no Terminal/env var;
- open a real PDF; verify metadata, thumbnails, original/processed
  preview, Estimate;
- convert a multi-page PDF; observe progress; cancel after it begins;
  verify no partial output;
- convert again after cancellation; verify the output PDF opens with
  the correct page count and 1-bit/CCITT properties;
- quit and relaunch; open a different document;
- (Windows) install via MSI, launch from Start Menu, uninstall, test a
  path containing spaces and non-ASCII characters;
- (Linux) launch the AppImage, and `.deb` install/launch/uninstall if
  built; note the distro/version used.

Until this checklist is actually run, "Human runtime" stays **pending**
in the table above for every platform — including macOS arm64, where
only the non-interactive launch was verified this session.
