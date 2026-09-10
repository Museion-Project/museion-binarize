# Current local desktop workflow (2026-09-10)

The main application now uses one workspace: open a PDF, choose black-and-white
processing and/or contents bookmarks, preview and edit, then save a new PDF.
OCR body recognition has a visible disabled entry. The older API, provider and
OCR panels described below remain compatibility code and are not mounted in
the current main workspace.

The contents service reads native text or the unchanged Apple Vision fast
worker, renders full pages through the open PDFium session, performs the normal
numeric-lane read, calls the shared Rust `compile_local` directly, and writes
reviewed outlines through the independently validating writer. The desktop
never shells out to the CLI. Bookmark-only export preserves original page
content; combined export binarizes first and then adds outlines to the staged
PDF. Existing output files are refused.

Current evidence: [desktop workflow report](evidence/desktop-workflow-2026-09-10/report.zh-CN.md).
Three existing books passed service-level generation/review/save/reopening;
Menn also passed connected native UI editing and full 202-page combined export.
OS file pickers in that automated UI run were supplied by development fixtures.
The `desktop-ui-test` Cargo feature is opt-in and debug-only; the delivered
local app excludes it, and production frontend builds remove dialog fixtures.

The local app bundles frontend, PDFium and scripts. Bookmark generation still
requires this machine's Python with PyMuPDF/Pillow; image recognition also uses
macOS Vision and the Swift compiler on its first run. Portable distribution,
independent human review and new-book generalization are not established.

The sections below document earlier milestones; their main-screen workflows
and historical verification statuses do not describe the current UI.

---

# Generic API route and cross-device tasks (M6 compatibility surface)

The desktop retains a generic `mpdf-api/0.1` developer/private-service panel
with Local, Cloud enhanced, and Cloud then local routes. This panel predates
the v2 OCR product contract: it is not a `CompleteOcrProvider`, Gemini BYOK,
or the unavailable `mpdf-credits` service, and it is not the provider picker
described below. Local never constructs an API client. An explicit remote
route never falls back silently; Cloud then local records the user-selected
fallback reason. Before an operator-directed upload, the consent summary shows
endpoint origin, provider, model, source digest, integer micros budget, and
retention. Only credential presence is displayed—tokens never enter frontend
state or IPC responses.

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

## The main flow: OCR → bookmarks → binarize

`LocalPipelinePanel` is the app's primary action, and it maps to exactly one
core entry point: `mpdf_core::orchestrator::run`, the same one `mpdf run`
uses. The desktop backend does not have a second implementation of the
pipeline order, because a second implementation is how "OCR the binarized
output" gets reintroduced.

A person using it chooses where the finished PDF goes and a working folder,
then presses **Start**. `local` is the stable default route. The base app may
start without OCR-plugin readiness: a PDF with reliable native text completes,
while the first scanned/image-only page reports that the optional local OCR
plugin is required. Advanced settings can point at an explicitly staged
sidecar and model directory. Plugin readiness gates only the independent
plugin artifact, never the base application's Start button.

### Commands and events

| Command | Purpose |
|---|---|
| `local_ocr_readiness` | Whether local OCR can run, and exactly which model files are missing. Inspects configured paths only: it never discovers or downloads anything. |
| `start_local_pipeline` | Starts (or resumes) the run on the PDFium worker thread. Returns the job id, the workspace, **the stage list**, and whether reusable evidence was found. |
| `cancel_local_pipeline` | Flips the shared in-process/provider cancellation flag and marks the durable OCR job cancelled. |
| `cancel_local_pipeline_job` | Recovery/admin form that marks a named durable OCR job cancelled. |

Events: `mpdf://pipeline-stage`, `-completed`, `-failed`, `-cancelled`.

The stage list comes **from the backend**, so the UI cannot drift out of step
with the core's real order. The frontend renders
`PIPELINE_STAGE_LABELS[stage]` and nothing else.

### The source is never named by the frontend

`LocalPipelineRequestDto` carries no source path. The backend takes it from
`OpenDocumentState.input_path` — the original file the user opened. There is
therefore no request the frontend can construct that points OCR at a
binarized output.

### Start, cancel, resume

Resume is not a separate command. The run is keyed to a durable workspace, so
starting again with the same workspace reuses every OCR page that was already
committed and digest-verified in the same fingerprint namespace, and continues
from there. Cancelling propagates through the desktop worker, durable SQLite
job, and selected provider loop. It keeps committed pages: the panel says so,
and the button changes to **Resume**. No current product cloud mode can create
an in-flight HTTP OCR call.

Changing the source, engine, sidecar bytes, model-file bytes, language profile,
or OCR DPI changes the durable job's fingerprint, so evidence from different
configurations is rejected rather than mixed. Moving byte-identical models
does not change their content identity.

### Review is a normal outcome

When entries need a human decision the run stops at `awaiting_review` and
writes **nothing**. The panel says so plainly and offers two ways forward:
apply the decisions made in the review workbench below, or write only the
entries already confirmed. A safe refusal is likewise shown as a result, not
as an error.

### Completion wording

`formatSizeChange` reports the direction the output size actually went. The
core reports `sizeReductionFraction = 1 - output / input`, which is negative
when the output grew; rendering that straight through produced
"-311% smaller" for a file that had grown to 4.11× its input. A conversion
that grows is a normal outcome for a photographic scan binarized at a high
DPI, so it is described as "311% larger".

## Choosing where recognition runs

The provider picker sits above the Start button and offers the two current
product modes: `local` and `mpdf-credits`. Legacy `gemini-byok` state remains
deserializable but is not listed. The base app can process native-text PDFs
without OCR; scanned pages need the separately installed local plugin.

Three rules shape the component:

1. **Local is preselected and no error path falls forward into cloud.** The UI
   explains that the base app handles native text while scanned pages require
   the optional offline plugin.
2. **M PDF Credits is visibly unavailable.** It is the future paid, brokered
   complete-OCR path, and the core supplies the exact production-service
   blockers. The control cannot start a run.
3. **Consent and cost are backend gates.** A future enabled brokered run must
   name the page-image upload and require a positive credit ceiling. The
   backend re-checks both; a disabled button is only a courtesy.

Retained credential IPC command names exist only so an older renderer receives
the stable BYOK-disabled error. They do not inspect the supplied payload,
consult a credential store, or reactivate a key workflow. See
[`ocr-providers.md`](ocr-providers.md).

## Bookmark review and automatic table of contents

The workbench loads the persisted bookmark tree through `load_bookmark_tree`
and invokes the core-backed `confirm_bookmark`, `reject_bookmark`,
`edit_bookmark`, and `reparent_bookmark` commands. Candidate source title,
page, master bbox, evidence count, confidence, score breakdown, alignment
evidence, and reason codes remain visible; no preview asset means no overlay
is drawn. Every mutation is an append-only review record.

`auto_confirmed` (added by the deterministic engine) and `confirmed` (decided
by a person) are separate filter values and are labelled distinctly —
"Added automatically" versus "Confirmed by you" — in the tree, the status
filter, and the selected-candidate panel.

### The automatic path

One button, `Add bookmarks automatically`, maps to `start_auto_bookmark`. The
user picks the MDP package folder and where to save the new PDF through native
pickers; the raw text field beside them is retained and labelled
"(advanced)". The user is never asked for an OCR provider, a threshold, a page
offset, or a contents page.

- The request DTO is project-owned and carries the current `document_id`, the
  package path, the output path, and explicit `overwrite`/`regenerate`
  booleans. The **source PDF path comes from `AppState::OpenDocumentState`**,
  never from the frontend, and no source path or byte ever travels back out.
- A request for a document that is no longer open is rejected
  (`document_stale`/`document_not_open`) rather than applied to whatever is
  open now.
- A single `AppState` operation gate atomically arbitrates processing, remote
  API execution/install, and automatic bookmarks. A second claim is refused
  with an actionable `operation_active` structured error, never queued
  silently; the per-operation slots remain cancellation handles only.
- The run executes on the single PDFium worker thread, so the UI never
  freezes while a long book is matched or a PDF is written. Stages arrive as
  `mpdf://auto-bookmark-stage` events (`analyzing_toc`, `aligning`,
  `writing_pdf`, `validating`), and the result, failure, or cancellation as
  `mpdf://auto-bookmark-completed` / `-failed` / `-cancelled`.
- `cancel_auto_bookmark` takes both the job id and the document id, so a stale
  window cannot cancel a newer document's run. Cancellation leaves no new
  candidates/report pair, output, or temporary file behind; an older matching
  generation/output remains intact.
- The result panel reports mode, status, contents pages found, and the
  automatically-added / needs-review / skipped counts, then reloads the
  bookmark tree. A **safe refusal is a normal result panel**, not a red error:
  it states plainly that no reliable structure was found and that nothing was
  written.
- Errors are classified through the existing `UiErrorDto`; no OCR text, token,
  credential, or raw API response reaches the frontend.

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

## Local practical fixes (2026-09-10)

The local unified workspace now defaults to original-page preview, renders it
at screen size and caches a bounded 16 MiB of previews. Processed preview still
uses the requested output DPI, and runs only on selection. Thumbnails are
serialized and pause behind the main preview.

Binarization accepts physical PDF page ranges and skip-current exclusions.
Unselected pages remain in place, retaining their original PDF objects/streams;
selected pages use the shared core image pipeline. Original page IDs remain
stable for bookmarks. Partial conversion saves a new file only, checks the open
source hash and independently validates the mixed result. It retains original
resources, so partial output may grow. Encrypted partial input is rejected.

Finder launches discover an installed Python with both PyMuPDF and Pillow via
absolute candidates, including versioned Homebrew installations. This is local
runtime discovery, not a bundled Python distribution. Use optimized release
builds for delivery. See the performance evidence report for measured timings,
connected native UI coverage and the boundary between debug test instrumentation
and the ordinary release bundle.

## Open-time pagination and explicit processing (2026-09-10)

Opening or dropping a PDF starts a document-scoped local text-layer/pagination
pass. Native folios are read directly; scanned margins receive bounded random
Apple Vision samples and nearby checks. Observed folios and piecewise inferred
rules remain separate, including Roman/Arabic numbering, restarts and paired
folios on spreads. The footer reports progress. Generation reuses this model,
excludes contents pages as independent witnesses, and retains ambiguous targets
for review. Sampling cannot guarantee detection of every short numbering reset
or unobserved inserted page. Encrypted inputs and unavailable runtimes report
unavailability rather than a successful reconstruction.

The black-and-white heading has a Start action; the contents heading has a
Generate action. Processing presets live inside Advanced settings. Start caches
a validated PDF for the current source hash, settings and range. Save reuses
that result; changed parameters require another Start. Automatic pagination is
separate from the serialized PDFium preview worker and is cancelled on close
or document replacement. It is not a persistent background service.
