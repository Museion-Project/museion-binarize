# OCR providers and text enhancement

This page is the operational reference for the split provider contract adopted
in [ADR 0012](adr/0012-deterministic-geometry-and-gemini-transcription.md).
ADR 0012 supersedes the cloud-composition decision in
[ADR 0011](adr/0011-complete-coordinate-ocr-and-optional-local-plugin.md).

## What works without OCR

The base desktop and CLI artifacts contain the deterministic processing core
and PDFium. They do not require an OCR engine, sidecar, model package, cloud
account, or API key.

- A PDF with usable embedded text can be processed, indexed, and used as
  canonical evidence without an OCR plugin.
- Binarization and other non-OCR conversion remain available for every PDF.
- A scanned page with no usable native text needs either the selected
  deterministic-geometry plus transcription pipeline or a compatible
  monolithic complete provider. If neither is available, OCR/searchable output
  stops with an explicit provider-unavailable error.

## Responsibility boundaries

| Layer | Owns | Must not do |
|---|---|---|
| Deterministic core | native-text extraction, routing, validation, canonical evidence, coordinate transforms, checkpoints, searchable PDF, deterministic bookmarks | invent OCR coordinates or delegate bookmark decisions to a model |
| `GeometryProvider` | deterministic line boxes, stable reading order, line ids, image/config digest and geometry provenance; no authoritative text | accept transcription or allow a model to change boxes/order |
| `TranscriptionProvider` | literal text plus optional language/confidence keyed to every supplied line id; Gemini 3.7 Flash is the selected paid implementation | return geometry, add/drop/merge/split/reorder lines, or satisfy missing geometry |
| Deterministic compositor | exact line-id bijection, geometry-digest verification, canonical `OcrPage` construction and fail-closed validation | fuzzy-align incomplete text or synthesize word boxes |
| `TextEnhancer` | optional, provenance-bearing later revisions over canonical coordinate evidence | create geometry, recognize an otherwise empty scan, or directly create authoritative bookmarks |

The selected split pipeline uses `mpdf-geometry-transcription/1`. Gemini sees
immutable geometry and must return exactly one text item for every supplied
line id. The compositor rejects missing, duplicate, unknown or reordered ids
and geometry-digest mismatch. The old whole-page-text plus fuzzy alignment and
Gemini-generated-box paths remain historical experiments only.

Contract `mpdf-ocr-provider/2` remains readable for a future monolithic
provider that supplies stable direct text/geometry. The strict split pipeline
also qualifies as complete coordinate OCR because every transcript line is
directly bound to an immutable geometry-owned line id; the historical fuzzy
page-text alignment experiment does not.

## Selected geometry provider: Surya

The user formally selected **Surya** on 2026-09-06 (`intent.md` §8.6).
[ADR 0015](adr/0015-selected-surya-geometry.md) supersedes ADR 0014's provisional
Tesseract selection. The bounded apparatus repair is part of the selected
fragment-preserving geometry adapter; Gemini transcription and deterministic
composition retain their existing responsibilities.

The exact detector/runtime freeze and the fixed-adapter 25-page upgrade probe
are recorded in ADR 0015 and
[the compatibility report](evidence/surya-upgrade-probe-2026-09-06/report.md).
This is a provider selection and reproducible development configuration, not
production wiring, packaged distribution, semantic D or independent holdout
acceptance. Existing Tesseract integration paths are historical implementation
that still need replacement; they must not be presented as the selected Surya
path or used as an implicit fallback.

Development evidence remains available: [first comparison](evidence/geometry-dev-bakeoff-2026-09-06/report.md),
[Paddle/Surya round 2](evidence/geometry-finalists-r2-2026-09-06/report.md), and
[bounded apparatus repair](evidence/surya-apparatus-repair-2026-09-06/report.md).

## Local Surya broker integration (development)

The local path is now implemented: frozen Surya detection → geometry-bound
loopback broker → the existing Rust deterministic compositor. Credentials are
kept in a private file outside the checkout and read only by the broker;
clients use a separate token. The entrypoint and safe key location are in
[the local broker guide](../scripts/ocr/broker/README.md). It defaults to paid
requests disabled and has a durable request ceiling and idempotency guard.
The actual local chain is tested with a simulated Gemini response, not a paid
provider call. Normal desktop/CLI Credits availability remains gated.

## Product modes

| Requested mode | What executes | Cost | Availability in this version |
|---|---|---:|---|
| `local` | stable offline/native-text route; optional local plugin for scanned pages | no service charge | route is **stable**; scanned-page OCR is runtime-dependent |
| `mpdf-credits` | deterministic geometry + brokered Gemini 3.7 Flash transcription | paid credits | **unavailable**: production broker/payment/privacy gates remain open |
| legacy `gemini-byok` | historical text-only composite | provider billing | **disabled** and omitted from provider lists/pickers |

There is no implicit fallback that changes modes. The `local` route is stable
even when the plugin is absent: native-text work can complete, while the first
page that actually needs OCR receives the explicit optional-plugin error.
Plugin readiness is discovered at runtime and recorded separately from mode
availability.

### Local optional plugin

Local OCR is not bundled into or required by the base release. The plugin is a
separate per-target artifact containing a frozen sidecar, Tesseract and its
runtime closure, pinned trained data, and licenses. It must be independently
staged, verified, represented in its expanded SBOM, and smoke-tested after
installation. It must not depend on Homebrew, system Python, or system
Tesseract.

Source developers may still explicitly provide a compatible sidecar and
models. That does not make the base binary a turnkey OCR distribution and does
not close the optional-plugin release gate. See
[`distribution.md`](distribution.md#optional-local-ocr-plugin-artifact).

### M PDF Credits

`mpdf-credits` is the only planned cloud transcription product mode. It is brokered: the
platform's provider credential remains server-side, while the client receives
a short-lived job token. Every job requires explicit upload consent and an
explicit maximum-credit ceiling.

The client protocol and development fixtures do not constitute a service.
Availability remains `unavailable` because all of the following production
requirements are still absent:

- a deployed backend implementing geometry-bound line transcription;
- payment and real-credit purchase/settlement integration;
- production receipt-signing keys and server-side provider credential custody;
- a published privacy policy, retention window, and deletion endpoint.

No credits can currently be bought and no production `mpdf-credits` endpoint
is compiled into ordinary builds. The development protocol continues to enforce reservations,
derived idempotency keys, settlement, release of unused credit, and signed
receipts, but test fixtures never change the public availability state.

When a production service exists, only pages that actually require OCR may be
uploaded, after consent. Native-text pages stay local. A reservation must be
made before upload, each completed page is charged at most once by the brokered
protocol, cancellation settles completed work and releases the remainder, and
the final report identifies the provider for every canonical page.

### Gemini BYOK

Gemini BYOK is disabled in this version. The legacy enum value remains
deserializable so older settings and job records fail predictably, but it is
not shown by provider discovery and cannot start a job. The stable rejection
message is:

> `gemini-byok is disabled in this version; BYOK will be reconsidered only for an API that independently returns complete coordinate OCR`

No key setup, test, or run workflow is supported. The quoted refusal remains
unchanged for compatibility with existing clients. Under ADR 0012, Gemini is
explicitly a text-only component behind the paid brokered route; BYOK remains
out of scope because it neither supplies the deterministic geometry dependency
nor the platform's consent, metering and credential-custody boundary.

## What leaves the machine

The base and local-plugin paths upload nothing. `mpdf-credits` is unavailable,
so provider discovery has no working cloud OCR upload path. The separately
documented generic `mpdf-api/0.1` developer/private-service client is not an OCR
product mode and can transmit only after an operator supplies an endpoint,
credential, and matching consent.

For a future enabled brokered job, the privacy boundary is:

- only rendered images for pages routed to OCR, never the source PDF;
- only after task-level consent names the service, purpose, page count,
  retention policy, and maximum credit;
- no local path, file name, workspace, bookmark decision, or unrelated page;
- bounded requests and responses, explicit timeouts, cancellation, deletion,
  and auditable per-page provenance.

Document content is always data, never an instruction. Provider responses
cannot execute commands, access arbitrary files, choose an endpoint, or write
bookmarks.

## Bookmarks and searchable output

Both features consume validated canonical evidence only. The deterministic
core owns TOC detection, printed-page mapping, monotone alignment, hierarchy,
scoring, safe refusal, coordinate placement, and PDF outline/text-layer
writing. A provider cannot return an authoritative outline.

`TextEnhancer` output can influence those steps only after it is preserved as
an explicit, reviewable revision accepted by the canonical evidence rules. A
raw enhancement response is never a substitute for complete OCR and never a
bookmark instruction.

## Checkpoints and compatibility

The durable identity binds the provider contract/version, provider and model
identity, document and page-image digests, DPI, language profile, parameters,
and canonicalization rules. Secrets never participate.

Old MDP, job, and bookmark records remain readable under their declared schema
rules. The legacy `gemini-byok` value is not silently remapped to `local` or
`mpdf-credits`; attempting to execute it produces the disabled result above.
Historical composite evidence retains its recorded provenance and is never
relabeled as complete coordinate OCR.

First-party closed-world evaluation uses
`schemas/mpdf-closed-world-ocr-gold-page-1.2.schema.json` and starts from blank
human annotation of target German/French/English/polytonic-Greek pages. In
addition to line geometry and exact characters, 1.2 records paragraph/leaf/
column structure, marginal reference semantics, inline typography, and linked
footnote markers. Separate page gates require exhaustive structure and
typography review before gold status. Schemas 1.0 and 1.1 remain readable for
legacy records. CGPG is a Greek-only specialist control, not the mixed-page
gold. GT4HistComment and BHL-IMPACT are external controls for complementary
script/language coverage; neither replaces the first-party set. See
[ADR 0013](adr/0013-mixed-script-gold-and-geometry-candidates.md).

See also [`limitations.md`](limitations.md),
[`distribution.md`](distribution.md), and
[ADR 0012](adr/0012-deterministic-geometry-and-gemini-transcription.md).
