# ADR 0012: Deterministic geometry and Gemini 3.7 Flash transcription

Status: accepted on 2026-08-30. Supersedes the composition and cloud-provider
selection in ADR 0011; its optional-distribution, consent, payment, credential,
bookmark, and compatibility decisions remain in force.

## Context

The Complete OCR bakeoff exposed two independent Gemini failure modes. Dense
pages sometimes overgenerated until the structured response was truncated,
while repeated successful calls could preserve line text/count yet move model-
generated rectangles enough to fail the geometry contract. The CGPG PAGE XML
also proved to be open-world annotated-target data: visible Latin lines and
footnote markers were absent, so it could not fairly score an instruction to
return every visible line.

Gemini 3.7 Flash remains promising for polytonic transcription, but its native
rectangles are not stable observational geometry. Requiring one remote model to
own both jobs couples a strong transcription component to its weaker output.

## Decision

Scanned-page OCR is a strict composition of two separately replaceable roles:

1. `GeometryProvider` deterministically observes line boxes and reading order
   on the source raster. It returns no authoritative text. Its pinned engine,
   model/config version, image digest, line ids, boxes, and order form a
   canonical geometry digest.
2. `TranscriptionProvider` receives the image and immutable geometry. The paid
   cloud implementation is Gemini 3.7 Flash. It returns text, optional language
   and confidence, keyed by the supplied line ids. It cannot return boxes,
   change order, merge/split lines, add a line, or omit a line.
3. The deterministic compositor requires an exact ordered bijection and the
   exact geometry digest. A missing, duplicate, unknown, or reordered line id,
   a digest mismatch, malformed text, or non-NFC output rejects the whole page.
   There is no fuzzy/monotone post-hoc alignment on the production path.

The versioned core boundary is `mpdf-geometry-transcription/1`. The existing
monolithic `CompleteOcrProvider` contract remains readable and usable for a
future engine that genuinely supplies stable direct text/geometry, but it is no
longer the selected cloud architecture. Gemini structured-box experiments stay
historical evidence and are not production geometry.

Gemini transcription remains a brokered paid feature (`mpdf-credits`). The
platform credential stays server-side; BYOK remains disabled. Page images may
leave the machine only after the existing explicit-consent and cost-limit
gates. The geometry provider remains separately distributable/pinnable and is
not silently bundled into the base application by this decision.

## Gold and evaluation

Cloud transcription is evaluated only against closed-world gold. CGPG's
recognition annotation is Greek-only and therefore remains a specialist stress
test rather than the target gold for German/French/English/polytonic-Greek
pages. First-party target pages start from blank annotation. A page is eligible
only when a human has verified every visible line's box, order, literal text,
language/class label, inspected the full page for missing text, and cleared all
unresolved notes. The validator freezes image and record digests and refuses a
manifest whose page set differs from the declared holdout. Dataset reuse and
the deferred GeometryProvider selection gate are specified in ADR 0013.

The existing open-world CGPG evidence remains valid for forensic analysis, but
cannot select a production transcription provider. A fair bakeoff begins only
after all frozen pages pass the new closed-world coverage contract.

## Consequences

- Box drift and model-driven reading-order drift are removed from Gemini's
  authority rather than tuned with another prompt.
- Truncation becomes an explicit missing-line contract failure, not a partially
  accepted page.
- Geometry quality and transcription quality can be benchmarked, upgraded, and
  rolled back independently.
- Cloud transcription now requires usable deterministic geometry first. If
  geometry is unavailable or incomplete, the cloud call is not made.
- Line-level geometry can produce a valid text layer. Word-level geometry is a
  later capability and must not be synthesized by proportionally splitting a
  line box.
