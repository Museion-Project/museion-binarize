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
MUSEION_PDFIUM_LIBRARY="/Users/theo/AI 工作流/museion-binarize/target/pdfium/aarch64-apple-darwin/libpdfium.dylib" \
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

**Automated, non-PDFium (`cargo test -p museion-binarize-desktop`):**
DTO conversion and validation (`settings.rs`), error classification and
the guarantee that no error DTO ever serializes a password
(`errors.rs`) — 13 tests, all passing, requiring no PDFium library.

**Automated, provisioned PDFium (`cargo test --test pdf_pipeline --
--ignored`, run with `MUSEION_PDFIUM_LIBRARY` pointed at a real library):**
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
museion-binarize-desktop`, `pnpm lint`, `pnpm typecheck`, `pnpm build` —
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
MUSEION_PDFIUM_LIBRARY="/Users/theo/AI 工作流/museion-binarize/target/pdfium/aarch64-apple-darwin/libpdfium.dylib" \
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
