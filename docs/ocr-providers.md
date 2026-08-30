# OCR providers and text enhancement

This page is the operational reference for the provider contract adopted in
[ADR 0011](adr/0011-complete-coordinate-ocr-and-optional-local-plugin.md).
ADR 0011 supersedes the default-provider and cloud-composition decisions in
[ADR 0010](adr/0010-provider-neutral-ocr-and-cloud-modes.md).

## What works without OCR

The base desktop and CLI artifacts contain the deterministic processing core
and PDFium. They do not require an OCR engine, sidecar, model package, cloud
account, or API key.

- A PDF with usable embedded text can be processed, indexed, and used as
  canonical evidence without an OCR plugin.
- Binarization and other non-OCR conversion remain available for every PDF.
- A scanned page with no usable native text needs a complete OCR provider. If
  none is installed or available, the OCR/searchable-output request stops with
  an explicit provider-unavailable error. It does not invent text or silently
  mark the page as recognized.

## The three responsibility layers

| Layer | Owns | Must not do |
|---|---|---|
| Deterministic core | native-text extraction, routing, validation, canonical evidence, coordinate transforms, checkpoints, searchable PDF, deterministic bookmarks | invent OCR coordinates or delegate bookmark decisions to a model |
| `CompleteOcrProvider` | complete block/line/word text, measured coordinates, stable reading order, direct text/geometry identity, declared language/confidence support, engine identity, provenance | return text alone and call it OCR |
| `TextEnhancer` | optional, provenance-bearing text revisions over existing canonical coordinate evidence | create geometry, recognize an otherwise empty scan, or directly create authoritative bookmarks |

Complete providers use contract `mpdf-ocr-provider/2` and schema
`mpdf-ocr-provider` version `0.2`. A valid page result is complete coordinate
OCR: its evidence can stand on its own without another OCR engine supplying
the rectangles.

The older local-detection plus cloud-transcription implementation is an
`experimental-composite`. It is useful for evaluation, but it is neither a
complete provider nor an available product mode. Deterministic alignment can
attach revised characters to measured lines; it cannot make generated or
missing geometry observational evidence.

## Product modes

| Requested mode | What executes | Cost | Availability in this version |
|---|---|---:|---|
| `local` | stable offline/native-text route; optional local plugin for scanned pages | no service charge | route is **stable**; scanned-page OCR is runtime-dependent |
| `mpdf-credits` | future M PDF brokered complete-OCR service | paid credits | **unavailable**: no production complete-OCR backend exists |
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

`mpdf-credits` is the only planned cloud OCR product mode. It is brokered: the
platform's provider credential remains server-side, while the client receives
a short-lived job token. Every job requires explicit upload consent and an
explicit maximum-credit ceiling.

The client protocol and development fixtures do not constitute a service.
Availability remains `unavailable` because all of the following production
requirements are still absent:

- a deployed backend that independently returns complete coordinate OCR;
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

No key setup, test, or run workflow is supported. BYOK will be reconsidered
only if an API independently returns complete coordinate OCR under the v2
contract. A text-only response, or rectangles generated by a model rather than
measured by an OCR system, does not qualify.

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

See also [`limitations.md`](limitations.md),
[`distribution.md`](distribution.md), and
[ADR 0011](adr/0011-complete-coordinate-ocr-and-optional-local-plugin.md).
