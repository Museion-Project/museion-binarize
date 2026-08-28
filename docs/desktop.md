# API route and cross-device tasks (M6)

The desktop exposes Local, Cloud enhanced, and Cloud then local as explicit
route choices. Local never constructs an API client. Cloud enhanced never
falls back silently; Cloud then local records the user-selected fallback
reason. Before upload, the consent summary shows endpoint origin, provider,
model, source digest, integer micros budget, and retention. Only credential
presence is displayed—tokens never enter frontend state or IPC responses.

Portable task receipts can be imported on another device after selecting a
credential profile for the same origin. Progress, cost, cancellation, resume,
and retention acknowledgement/pending/failure are durable states.

# The desktop application

This document describes the Milestone 4 desktop GUI: its architecture,
what it actually does today, and its current limitations. It complements
[`architecture.md`](architecture.md) (the workspace as a whole) and
[`cli.md`](cli.md) (the sibling command-line interface, which shares the
same core).

## Status

**Implemented and passing ordinary and provisioned-PDFium automated
tests as of this writing. Not yet manually exercised as a running desktop
application on a physical machine** — see
[`desktop-testing.md`](desktop-testing.md) for exactly what has and has
not been verified, and why. Do not treat this document as evidence that
someone has clicked through the running app.

## Workflow

```
Open PDF -> Preview -> Configure -> Convert -> Validate
```

Opening a PDF creates one document session in the backend. The sidebar
shows lazily-loaded page thumbnails; selecting a page renders an
original and a processed preview through the real core pipeline.
Settings changes invalidate only the processed preview. Choosing an
output destination and clicking Convert starts an asynchronous job with
live progress; the job can be cancelled; completion shows a summary
report or a structured error.

## Persistent document ownership

**One active document per window** (see the Milestone 3 spec's suggested
simplification, adopted here). A window that opens a second document
first closes whatever it had open; opening is rejected outright while a
processing job is running, so a session is never replaced out from under
an in-flight conversion.

Rationale: multi-document tabs are real scope, not a small addition —
they would need per-tab job/cancellation state, per-tab thumbnail
caches, and a UI affordance for switching between them, none of which
this milestone's mission (feature-complete single-document workflow)
requires. Deferred, not forgotten.

## The PDFium worker thread

`apps/desktop/src-tauri/src/worker.rs` spawns one dedicated OS thread
that owns at most one open `PdfDocumentSession` for the lifetime of the
document. Every PDFium-touching operation — opening a document, rendering
a page, running a conversion — is a message sent to this thread and
executed there, one at a time, in the order received.

This is deliberate and stronger than "assume `pdfium-render`'s
`thread_safe` feature makes everything fine": the session and the
`PdfDocument` it holds **never leave the thread they were opened on**, so
the backend makes no `Send`/`Sync` claim about `pdfium-render` types at
all. `WorkerCommand` variants that need PDFium hold plain owned data
(paths, settings, a boxed `ProgressReporter`); only the worker thread's
own `run` loop ever touches the session.

A `Process` command occupies the worker thread for the whole conversion.
This is intentional, not an oversight: cancellation is delivered
out-of-band (see below), not as a queued message, so it does not wait
behind the job. Preview and thumbnail requests issued while a job is
running do queue behind it and are answered once the job finishes — the
frontend's own state machine already disables settings and preview
interaction during `processing`, so this is not user-visible as a stall
in the cases the UI allows.

## Not blocking the Tauri event loop

Tauri commands that touch the worker (`open_document`, `render_preview`,
and the fire-and-forget dispatch inside `start_processing`) are `async
fn`s that send a message to the worker thread and then bridge the
worker's blocking `std::sync::mpsc::Receiver::recv()` through
`tauri::async_runtime::spawn_blocking`. This keeps the async command
itself non-blocking without introducing a second async runtime or a
broad Tokio dependency beyond what Tauri 2 already provides — see
`WorkerHandle::call` in `worker.rs`.

`start_processing` returns as soon as the job is handed to the worker
thread, carrying a `jobId`. The actual conversion continues in the
background; its outcome (completion, cancellation, or failure) is
delivered later as a Tauri event, not as the command's return value.

## Cancellation

The GUI's Cancel button flips an `Arc<AtomicBool>` shared with the
`ProgressReporter` implementation (`TauriProgressReporter` in
`commands/processing.rs`) running inside the worker thread — no channel
round-trip, no queued message. The core pipeline's existing
`ProgressReporter::is_cancelled` check (already exercised by Milestone
2/3's cancellation tests) is what actually stops work between pages and
cleans up the temporary output file; **no core cancellation semantics
changed for this milestone.** This is why cancellation is real rather
than a UI-only "hide the progress bar": the same mechanism the CLI's
`process` command has always used is what the desktop app also uses.

## Progress events

Namespaced `mpdf://processing-progress` / `-completed` / `-cancelled`
/ `-failed` events, one job at a time. Progress granularity is
stage-level (rendering / binarizing / encoding / writing / validating
per page), not pixel-level — for a long book this is on the order of a
few events per page, not thousands total. `fraction` is estimated from
completed pages plus a fixed weight for the current stage; it is not a
precise measurement.

## Preview

Both the "original" (untouched rasterization) and "processed" (through
the real `image_pipeline::process_rendered_page`, the same function
`process_pdf` and `analyze_pdf` use) previews are rendered at the
**selected conversion DPI**, then optionally downscaled server-side
(`Triangle` filter) to a `maxDimension` the frontend requests — 1400px
for the main preview pane, 160px for thumbnails. The processed preview
therefore reflects the real algorithm at the real settings; only the
*display* resolution is reduced. `PreviewResultDto.isReducedResolution`
tells the frontend when this happened, so it is never presented as if it
were a lower-fidelity approximation of the algorithm itself — only of
the image.

Preview requests are debounced (200ms) and tagged with a
frontend-assigned, monotonically increasing `requestId`; the reducer
only applies a `PREVIEW_SUCCEEDED`/`PREVIEW_FAILED` action if its
`requestId` still matches the latest one issued, so a slow response to
an old request can never overwrite what a newer request already
produced (`hooks/usePreview.ts`, `app/reducer.ts`).

## Size estimation

The "Estimate" panel next to the settings controls calls the same
sampled, real-pipeline estimator described in
[`size-estimation.md`](size-estimation.md), via a dedicated `Estimate`
worker command that (unlike preview) is a direct request/response — an
estimate is bounded and fast enough not to need the event-based
progress/cancellation machinery `Process` uses.

- **Manually triggered, not auto-run on every settings change.** Running
  a real sample through the pipeline on every keystroke would make
  settings controls feel laggy and would burn CPU nobody asked for; the
  user clicks "Estimate" (or "Re-estimate").
- **Never discards the last value.** `EstimateState` is a discriminated
  union (`idle | running | ready | stale | failed`); when settings change
  after a successful estimate, the state moves to `stale` — the previous
  number stays visible (dimmed, labeled "Estimate outdated") instead of
  disappearing, so the panel is never blank right when the user might
  want it most.
- **Never blocks Convert.** A conversion can start with no estimate ever
  requested; the estimate is informational only.
- **Cancellable and staleness-guarded.** Each estimate request carries a
  monotonically increasing id, the same pattern `usePreview` established
  for preview requests — a slow response to a superseded request can
  never overwrite a newer one.
- **Serialized with conversion on the one PDFium worker thread.**
  Starting a real conversion cancels any in-flight estimate (flips its
  shared cancellation flag) so a `Convert` click is never stuck behind an
  estimate; an estimate cannot be started while a conversion is running.
- **Cached by document + settings.** The backend (`AppState::
  estimate_cache`) remembers the last successful estimate's document id
  and settings fingerprint. If a `process` call's settings still match, the
  resulting `ProcessingCompleted` report's `estimate_comparison` field is
  populated automatically — the frontend does not need to thread the
  prior estimate through itself.

## Image transfer

Preview and thumbnail images cross the IPC boundary as base64-encoded
PNG bytes in the command's own return value — not as a filesystem path,
not as a raw pixel array over a separate channel. This needs no
temp-file lifecycle (creation, cleanup on document close, cleanup on
crash) and exposes no filesystem path to the frontend for the renderer
to reach through. The tradeoff is base64/JSON overhead on top of the PNG
bytes themselves, which is acceptable at preview/thumbnail sizes (a few
hundred KB at most) but would not be the right choice for, say, exporting
the full converted PDF back through IPC — that never happens; the output
PDF is written directly to disk by the backend and the frontend only
ever learns its path and size.

## Memory model

```
source PDF bytes (held once, by the worker thread's session)
+ one uncompressed working page (during preview render or conversion)
+ a bounded per-document thumbnail cache (small PNGs, cleared on document change)
+ the growing compressed output PDF (during a conversion job)
```

The frontend never receives the source PDF's bytes at all — only
metadata (`DocumentSummaryDto`), preview/thumbnail PNGs, and reports.
Thumbnails are fetched lazily (`IntersectionObserver`) as they scroll
into view and cached per document id; a 100-page document does not
eagerly render 100 full-resolution pages, and the cache is cleared
(dropped) when the document changes. This is the same non-O(1)-in-source-
size honesty Milestone 3 already established for the CLI (see
[`pdf-pipeline-session.md`](pdf-pipeline-session.md)); the desktop app
does not make it worse by duplicating the source elsewhere.

## Settings, presets, and CLI parity

`ProcessingSettingsDto` mirrors `mpdf_core::settings::
ProcessingSettings` field-for-field. `settings.rs`'s
`to_processing_settings` is the one conversion point, and it re-validates
every field server-side (unsupported DPI, out-of-range contrast, an even
Sauvola window, `backgroundRadius` without `backgroundNormalization`,
...) regardless of what the frontend's own controls already constrain —
a frontend range is a convenience, never the actual limit.

Presets (`src/lib/settings.ts`) are three fixed, deterministic
`ProcessingSettings` values — Default, Fine detail, Noisy scan — each
with a plain description of what it changes. None claims to be "best",
"optimal", or tuned for a specific script or language: no benchmark in
this repository supports a claim like that (see
[`benchmarking.md`](benchmarking.md)). Changing any control switches the
preset indicator to "Custom".

A dedicated integration test
(`cli_and_gui_entry_points_produce_byte_identical_output_for_identical_settings`
in `crates/mpdf-core/tests/pdf_pipeline.rs`) converts the
same fixture through `process_pdf` (the CLI's entry point) and
`process_with_open_session` (the desktop app's entry point, added this
milestone specifically so the app can reuse an already-open session) and
asserts the two output PDFs are byte-for-byte identical.

## IPC boundary and security

Tauri capabilities (`capabilities/default.json`) grant only
`core:default`, `core:event:default`, `opener:default` (used for
"Reveal in Finder"), and the dialog plugin's `allow-open`/`allow-save`
permissions — no filesystem, shell, or HTTP capability of any kind. The
window's CSP (`tauri.conf.json`) is `default-src 'self'` with `img-src`
additionally allowing `data:` for inline preview PNGs; there is no
network capability for the frontend to use even if it wanted to, and the
processing core itself makes no network calls (verified by grep — see
`desktop-testing.md`).

No password is ever returned to the frontend: `open_document` accepts
one as a plain argument, uses it for exactly one PDFium open call inside
the worker thread, and nothing in `dto.rs` has a field capable of
carrying it back out. The frontend's own `PasswordPrompt` component holds
the password only in local component state, sends it once, and clears it
after use; there is no "remember password" feature and nothing is
written to `localStorage`.

## Application state

`apps/desktop/src/app/reducer.ts` models the whole UI as one
discriminated union (`idle | opening | passwordRequired | ready |
processing | completed | cancelled | failed`), driven by a pure
`reducer(state, action)` function — not scattered `isLoading`/
`isProcessing`/`hasError` booleans that could combine into an invalid
state. Every event carries an id (`documentId`, `jobId`, preview
`requestId`) and the reducer checks it against the id currently in state
before applying the update, so a stale async response or a stale Tauri
event from a previous job/document can never corrupt newer state.

## M5 bookmark review

The workbench can load the persisted bookmark tree and invoke the core-backed
`confirm_bookmark`, `reject_bookmark`, `edit_bookmark`, and
`reparent_bookmark` commands. Candidate source title, page, master bbox,
evidence count, confidence, and rule trace remain visible; no preview asset
means no overlay is drawn. Every mutation is an append-only review record.

## Known limitations (honest, as of this milestone)

- **Not yet run as a live application.** See
  [`desktop-testing.md`](desktop-testing.md).
- One document per window; no tabs.
- No thumbnail/preview virtualization library — lazy loading via
  `IntersectionObserver` is the only scaling strategy for long books.
  Adequate for the automated coverage this milestone has, unverified at
  real scale (hundreds of pages) without a live run.
- No settings UI for an explicit PDFium library path; only the
  `MPDF_PDFIUM_LIBRARY` environment variable (development-only, same
  as the CLI).
- No packaging, code signing, or notarization — this milestone is GUI
  feature completeness, not release engineering (Milestone 7).
- The M3 local OCR controller remains CLI-owned, but the desktop exposes
  provider readiness plus durable status, cancellation, and page-error query
  commands. It does not claim to start OCR from a background desktop worker;
  partial output and restart semantics are defined by the CLI/job store.
  M4 adds a local three-column review workbench for loading typed review
  issues and submitting human or AI-suggested revision records. It shows
  page/bbox coordinates rather than inventing an image overlay when no MDP
  preview asset is available; persistence is performed by the registered
  `load_review_queue` and `add_review_revision` commands.
  (bookmarks, annotations, forms, attachments) — unchanged from prior
  milestones.
- No Ancient Greek / polytonic typography preservation claim — unchanged
  from prior milestones; no benchmark exists yet (Milestone 6).
