# ADR 0010: Provider-neutral OCR, and what a cloud model is allowed to decide

Status: accepted on 2026-08-29.

## Context

Local Tesseract is this project's OCR. On the corpus that matters here —
polytonic Ancient Greek printed alongside German and Latin — it is the weakest
link. The measurements are unambiguous:

| Holdout | Engine | CER | WER | Diacritic errors | Script confusions |
|---|---|---:|---:|---:|---:|
| 63 pages, Greek (CGPG) | Gemini 3.7 Flash, full page, no region hints | 0.0318 | 0.0885 | 0.0114 | 2 |
| 8 pages, Greek/German/Latin | Gemini 3.7 Flash, full page | 0.0597 | 0.1884 | 0.0175 | 4 |
| 8 pages, Greek/German/Latin | Tesseract combined pass | 0.1398 | 0.4011 | 0.0740 | 16 |

Two secondary findings shape the design as much as the headline numbers.
Whole-page prompting beat region-cropped prompting, so the model must be shown
the entire page rather than pre-segmented boxes. And the model returns
*characters*, not measurements: there is no coordinate in a plain
transcription, and a coordinate the model volunteers is a second generation,
not an observation.

Three things then have to be true at once. Local must stay the default and
stay fully offline. A user's own API key must be usable without that key ever
touching a config file, a log, a checkpoint or a command line. And a brokered
"we run it for you" mode must be expressible without shipping the platform's
own provider key inside every desktop binary.

## Decision

### One contract, three implementations

`mpdf_core::ocr_provider::OcrProvider` is the whole surface: `capabilities`,
`validate_configuration`, `prepare_job`, `recognize_page`, `cancel_job`,
`finalize_job`, `fingerprint_contribution`. Every mode implements it, and
every mode returns the same thing — `OcrPage`, the existing block/line/word
tree with measured boxes in the page's own pixel coordinate system.

This is the load-bearing decision. Everything downstream — logical-line
assembly, the derived bundle, printed-page mapping, body-heading verification,
bookmark scoring, and the invisible text layer in the final PDF — consumes
`OcrPage` and nothing else. A cloud provider that handed back "the text of the
page" would be an attachment: it could not be aligned to a contents row, could
not carry a printed page number to a target page, and could not be drawn where
the glyphs are. So cloud OCR is not a special case in the pipeline; it is
another implementation of this trait, and it is bridged onto the *existing*
durable job loop rather than given a second one.

### The cloud path: local geometry, cloud characters

```text
original page raster
  -> LOCAL detector      : blocks, lines, words, rectangles  (always, first)
  -> cloud transcription : characters for the whole page, no region hints
  -> deterministic line split
  -> monotone alignment  : which characters belong to which rectangle
  -> gates               : coverage, order, injectivity
  -> canonical OcrPage   : local rectangles, cloud text
```

The local detector runs first and always, including on pages whose cloud call
will fail. That is what makes fallback free rather than catastrophic: the page
is already recognized, so a failure costs the cloud text and nothing else.

The join is a monotone, injective dynamic program, which gives three
properties by construction rather than by assertion: reading order can never
be inverted, one transcribed line can never be stamped onto several
rectangles, and a locally detected line that nothing aligned to keeps its own
text. Cloud output can therefore never delete a body line, a folio, or a
contents page number by omitting it. Below a coverage gate the whole page
keeps local text and records why — a half-aligned page is worse than an
unaligned one, because the half that aligned looks authoritative.

Word rectangles are inherited one-to-one when the word counts agree, and
otherwise derived by proportional split *inside the measured line box*. No
rectangle is ever invented outside geometry the detector actually produced.

### Model-returned rectangles are experimental and disabled

A strict schema, a parser that rejects unknown fields, and a full set of
geometry gates exist so the structured path can be *evaluated* honestly. It is
not enabled: `structured_bbox::GATES_VALIDATED_FOR_DEFAULT` is `false`, and no
gate-failing rectangle can become a coordinate in a written PDF. Adopting it
requires a hand-labelled bbox fixture set, a recorded adversarial-response
suite, and the documented thresholds met on a holdout.

### Confidence

A model's confidence in its own transcription is not a measurement of accuracy
and is never recorded as one. A replaced line carries *corroboration*: the
noisy-OR of two independent readings, capped by their agreement, so weak
agreement cannot borrow certainty from a confident local engine.

### Credentials

BYOK keys live in the OS credential store under their own service
(`org.mpdf.model-provider`), separate from the M PDF task token, so "delete my
key" is unambiguous. Settings hold a slot *label*; the key is read immediately
before a request and dropped after. There is no `--api-key` flag, no
environment echo, no serializable field, and no path by which a key reaches a
fingerprint, a checkpoint, an evidence record, or an IPC payload. Provider
error bodies — which sometimes quote the key back — are never returned to a
caller unredacted.

Rotating a key inside a slot deliberately does *not* invalidate evidence: the
same model under the same prompt produced it. Switching slots does, because
that is a different account.

### Checkpoints

The durable fingerprint binds provider mode, provider implementation version,
model and pinned model version, prompt digest, document and page-image
digests, DPI, language profile, local detector identity, alignment version,
structured-bbox policy and gates, and fallback policy. It binds nothing
derived from a secret.

The cloud segment is **appended** to the existing identity string rather than
interleaved, so a local run hashes byte-for-byte what it did before this work
existed. Adding the provider architecture invalidates no existing local
checkpoint — a frozen-digest test enforces exactly that.

### Credits

A versioned client protocol and a complete reservation → execution →
settlement → refund state machine, with derived (never random) idempotency
keys and HMAC-signed settlements the client verifies before believing. The
estimate is reserved up front, so insufficient balance is a pre-flight
failure before a single page image is encoded; whatever is not spent is
released; a retried page bills once.

There is no production service. `credits::PRODUCTION_BACKEND` is `None`,
`release_blockers()` enumerates what is missing, and both front ends print it
verbatim. Shipping a convincing but unbacked billing flow would be worse than
shipping none.

## Consequences

- Local remains the default, offline, and unchanged. Selecting a cloud mode
  requires an explicit flag plus explicit consent, in both front ends and
  again in the backend.
- A cloud run reports, per page, which engine actually produced it. "I paid
  for Gemini" and "these twelve pages are Tesseract" can never be confused.
- Cloud OCR still requires provisioned local models: the detector is the
  geometry source, not a fallback bolted on afterwards.
- The alignment cannot detect a reordering among near-identical lines. Nothing
  in the text can settle that; only a rectangle could, and the model did not
  measure one. Recorded in `docs/limitations.md`.
- Credits is code-complete and unreleasable. That combination is deliberate.
