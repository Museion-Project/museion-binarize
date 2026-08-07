# Testing the desktop application

This is a companion to [`desktop.md`](desktop.md) (architecture) and
[`testing-pdf-pipeline.md`](testing-pdf-pipeline.md) (the sibling
core/CLI testing document). It states exactly what has been verified for
Milestone 4, and — just as importantly — what has not, so this document
cannot be mistaken for evidence of manual GUI testing that did not
happen.

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

## What has **not** been verified

Concretely, none of the following — all required by the Milestone 4
specification's manual verification section — has been performed:

- launching `pnpm --dir apps/desktop tauri dev` (the real native window,
  not the plain browser check above) and confirming PDFium-backed
  behavior;
- opening a real synthetic PDF through the native file dialog and
  confirming thumbnails, page selection, and preview render correctly
  on screen;
- visually confirming Otsu/Sauvola/Manual and 300/400/600 DPI actually
  look right in the preview pane;
- the rotation regression (0°/90°/180°/270°, square markers staying
  square, thumbnail and main preview agreeing) as an on-screen check —
  only the underlying core geometry tests (unchanged by this milestone)
  are known to pass;
- watching a real conversion's progress bar update, confirming the UI
  stays responsive, and confirming the completion panel renders
  correctly;
- cancelling a real job mid-conversion and confirming the UI settles
  into "Cancelled" with no partial output on disk;
- the alias-safety manual click-through (`input.pdf` vs `./input.pdf` vs
  an absolute path vs a symlink chosen via the save dialog);
- the password-protected-PDF prompt against a real generated encrypted
  fixture;
- screenshots of idle / document-loaded / preview / Sauvola settings /
  processing / completion / error states, and the visual review of them
  for clipping, overflow, tiny controls, contrast, and spacing;
- a real 100-page document's sidebar scrolling and memory behavior.

**Do not mark Milestone 4 "complete" in [`roadmap.md`](roadmap.md) on
the strength of this document alone.** The automated coverage above is
real and passing, and it covers the parts of the system that are
mechanically checkable without a display (state transitions, IPC
contracts, settings validation, output byte-identity, error structure).
It is not a substitute for someone actually running the app. Whoever
next has access to a graphical macOS session should work through the
scenario list in the Milestone 4 specification (§46–47) before that
status changes.
