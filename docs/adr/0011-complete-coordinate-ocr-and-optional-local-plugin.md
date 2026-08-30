# ADR 0011: Complete coordinate OCR and an optional local plugin

Status: accepted on 2026-08-30. Supersedes ADR 0010.

## Context

ADR 0010 treated local detection followed by cloud transcription and
alignment as another implementation of one OCR provider contract. That
experiment established useful security, provenance, and checkpoint rules, but
it also joined two materially different jobs:

- OCR observes text *and its page geometry* and must return complete canonical
  block/line/word evidence;
- text enhancement proposes better characters for evidence whose geometry
  already exists.

A text-only model cannot independently OCR a scanned page. It first needs a
local OCR engine to discover the lines, and deterministic alignment can reject
or retain text but cannot turn generated rectangles into observations. Calling
that composition a complete cloud OCR provider made the cloud mode depend on
the local runtime, obscured which component supplied the evidence, and made a
heavy OCR runtime appear mandatory for the base application.

The product also needs a coherent paid-cloud boundary. Shipping a user's
provider key in a desktop workflow makes the user responsible for changing
third-party APIs, terms, prices, and data handling, while a product-operated
service must keep the platform credential server-side and enforce consent,
budgets, retention, and receipts. No such production service exists today.

## Decision

### Three responsibility layers

The document pipeline has three explicit layers. They are not interchangeable.

1. **Deterministic core.** The network-free core owns native-text extraction,
   page routing, coordinate transforms, schema validation, canonical evidence,
   checkpoints, searchable-PDF assembly, and bookmark solving. It may validate
   or reject provider output; it does not ask a model to invent coordinates or
   bookmark decisions.
2. **`CompleteOcrProvider`.** A complete OCR provider accepts an authorized page
   asset and independently returns the complete canonical block/line/word tree,
   including measured coordinates, stable reading order, direct text-to-geometry
   identity, engine identity, and provenance. It declares independently whether
   confidence and language metadata are available at page, line, or word level.
   Contract `mpdf-ocr-provider/2` serializes schema `mpdf-ocr-provider` version
   `0.2`. A component that returns only text is not a complete OCR provider.
3. **`TextEnhancer`.** An enhancer may propose text revisions over existing
   canonical coordinate evidence. It cannot create or replace geometry, fill a
   page that has no complete OCR/native-text evidence, directly produce an
   outline, or satisfy an OCR-provider availability check. Enhancements retain
   their own provenance and remain distinguishable from the immutable source
   recognition.

The old local-geometry plus Gemini-transcription path is therefore classified
as `experimental-composite`, not complete OCR. It may remain as internal
evaluation code, but it is not a product mode or a release capability.

### Product modes and availability

The product exposes only these requested modes:

- `local`: use the stable offline/native-text route and, when a scanned page
  needs recognition, a discovered compatible complete-OCR plugin;
- `mpdf-credits`: use the future M PDF brokered complete-OCR service.

`mpdf-credits` is a paid, brokered mode. It always requires task-level upload
consent and an explicit maximum-credit ceiling. The client receives only a
short-lived job token; the platform provider credential must remain on the
service. Its availability is `unavailable` in this version because there is no
production complete-coordinate OCR backend, payment path, signing-key
provisioning, published retention/deletion policy, or production service to
contact. Protocol fixtures do not change product availability.

Gemini BYOK is disabled. The legacy `gemini-byok` enum value remains
deserializable so old settings and job records produce a controlled result,
but it has availability `disabled`, is omitted from provider lists and pickers,
and cannot start a job. The stable refusal is:

> `gemini-byok is disabled in this version; BYOK will be reconsidered only for an API that independently returns complete coordinate OCR`

BYOK may be reconsidered only for an API that independently returns complete
coordinate OCR under the v2 contract. A text-only or model-invented-rectangle
API does not qualify.

### Local OCR is an optional plugin artifact

The base desktop and CLI release contains the deterministic converter and its
PDFium dependency. It neither contains nor requires an OCR engine, sidecar,
models, or their transitive libraries. Consequently:

- a PDF with usable native text can complete the canonical evidence,
  searchable-output, and deterministic bookmark paths without the plugin;
- a scanned page that needs recognition fails with a specific OCR-provider
  unavailable error when no complete provider is installed; it never fabricates
  text, silently treats the scan as recognized, or blocks unrelated conversion;
- the `local` route remains available without changing the base artifact;
  installing a compatible OCR plugin makes scanned-page recognition available
  within that route.

Each target's local OCR plugin is a separate, self-contained artifact. The
existing network-free stager, desktop overlay, CLI staging input, manifest
verifier, SBOM expansion, and installed-runtime smoke remain the distribution
boundary for that artifact. The plugin must not depend on Homebrew, a system
Python, or a system Tesseract. Base-release readiness must not require plugin
evidence; the independent `optional-local-ocr-plugin` profile must require
structure, provenance, license, and installed OCR smoke evidence.

### Bookmarks consume canonical evidence only

Bookmark generation reads only validated canonical native-text/OCR evidence
and user revisions. The deterministic core performs TOC detection, printed-page
mapping, alignment, hierarchy, scoring, safe refusal, and PDF outline writing.
Neither a complete OCR provider nor a `TextEnhancer` may return an authoritative
bookmark tree or bypass those gates. Enhancer text can affect bookmarks only
after it has been stored as an explicit, reviewable revision accepted by the
canonical evidence rules.

### Legacy compatibility

- Existing MDP, job, bookmark snapshot, and report schemas remain readable
  under their declared compatibility rules.
- Legacy provider records and `gemini-byok` settings deserialize; unsupported
  execution fails explicitly instead of becoming `local` or `mpdf-credits`.
- The v1 sidecar/provider surface remains a compatibility boundary for old
  records and development fixtures. Only an adapter that validates a complete
  coordinate result may enter the v2 canonical pipeline.
- Existing native-text and local OCR evidence keeps its recorded provenance.
  Migration must not relabel experimental composite output as complete OCR.
- Base conversion, native-text PDFs, and legacy deterministic bookmark inputs
  do not acquire a plugin, network, account, or payment dependency.

## Consequences

- A base release is useful and releasable without bundling a large OCR closure.
- Local OCR requires a separately installed, verified plugin and is never
  advertised merely because staging scripts or source models exist.
- There is one future cloud product path: paid, brokered `mpdf-credits`. It is
  visibly unavailable until the production service and policy gates exist.
- BYOK UI and execution are disabled while legacy state remains safe to read.
- Accuracy experiments may continue through `TextEnhancer`, but their output
  cannot be mistaken for coordinate OCR or authoritative bookmarks.
- Provider choice can change recognition provenance; bookmark behavior stays
  deterministic over the same canonical evidence.
