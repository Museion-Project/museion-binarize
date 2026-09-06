# Resumption audit and milestone 1 blocker

Authority: current user milestone 0–4 instruction, recorded in intent.md §8.8.
Inspected starting commit: `0567d20d00b77c7b88413964f545b04c3f687f3e` plus the
existing uncommitted local-broker implementation and live-call evidence.

## Independently verified

- Upgrade evidence: 112/112 recorded artifact hashes match. Apparatus repair:
  57/57 match. Frozen adapter hash remains
  `be148e58c3599c8687b723f8ec68437d2ae4f82c986b9ef1b2860bc8c2d7fdf0`.
  No detector inference, version comparison or adapter tuning was repeated.
- ADR0015 and provider-freeze.json select Surya 0.17.0,
  text_detection/2025_05_07, r3.0-apparatus-repair. Existing results record
  25/25 equality and reproduction of the repaired baseline.
- All 25 development image/reference hashes match the existing reference
  manifest; reference_gaps returns no gaps for all 25, covering 1,378 lines.
  The older milestone-status.json's 24/25 reference finding is stale.
- Metadata-only isolation passes for page ID, image digest and source digest
  plus PDF page, using the existing selection manifests. The 16 holdout image
  and annotation files were not opened, hashed, viewed, run or scored.
  Their content integrity rehash is deferred to milestone 4. Existing repository
  records show no consumption; this is not exhaustive proof of external history.
- Measurement contract SHA256 remains
  `4a3af40af67615e759085d5ed54def31aa434c745c692fe1ed1a13755f26c749`.
- Existing live result records one successful real Gemini 3.7 Flash request,
  40 composed lines and unchanged geometry. It used the Rust example and
  run_page.py, not CLI/Desktop document conversion; no final searchable PDF
  proves the requested product chain. No new model request was made here.
- Gemini key file exists, is nonempty and mode 0600; its contents were not read.
  A pattern scan of the 23 existing changed/untracked files found no Google
  key or private-key markers. This is a bounded scan, not an exhaustive secret audit.

## Milestone 1: incompatible production service contract

The available broker is a real **local transcription service**, not an
implementation of the current production Credits service contract:

1. `scripts/ocr/broker/server.py::Handler.do_POST` only accepts
   `/v1/transcription/pages`. It limits cumulative attempted requests; it has
   no Credits reservation, charge, signed settlement, release or refund API.
   `transcribe` currently reports `credits_charged: 0`. That does not establish
   that Google execution is free or that a monetary budget was enforced.
2. `schemas/mpdf-ocr-provider-0.2.schema.json` admits only local, gemini-byok
   and mpdf-credits. Local forbids network; BYOK cannot be complete OCR;
   mpdf-credits requires brokered_credits billing. The same restrictions are
   enforced by OcrProviderCapabilities::validate_complete_ocr in core.
3. ADR0012 explicitly retains production brokered payment and disabled BYOK.
   intent §8.7 authorizes a local development broker without enabling Credits.
   The current instruction authorizes wiring and real dev calls, but explicitly
   prohibits schema changes. Treating that as permission to invent a fourth
   product mode or to mislabel this local service would be an unsupported decision.
4. CLI build_cloud_factory and Desktop final_pdf_cloud still refuse Credits;
   MpdfCreditsFactory::build constructs legacy CloudOcrProvider. Merely deleting
   these refusals or setting PRODUCTION_BACKEND to loopback does not supply the
   missing lifecycle or build the frozen geometry stack.

This is a concrete service/capability mismatch, not a claim that geometry-bound
transcription is technically infeasible, and not a demand to complete every
commercial release gate before testing a product entry. A new local-broker
product mode is technically possible, but changing its capability schema needs
an exception to the user's explicit freeze. No such change was made.

## Concrete integration alternatives, not implemented

Preferred for the configured local broker, **subject to a narrow schema decision**:
add an explicitly named local-broker product mode with user-managed billing,
explicit upload consent, a bounded request/token budget, real usage reporting
and no fabricated Credits settlement. Keep offline local and commercial Credits
semantics intact. Add a shared API-client factory used by CLI and Desktop; it
constructs a runtime Surya GeometryProvider from the frozen entrypoint, the real
GeometryBroker transcriber, and GeometryTranscriptionPageProvider. Preserve the
full raw evidence alongside core geometry and transcription, pass the resulting
OcrPage through the existing document orchestrator and searchable-PDF writer.
Use fresh document workspaces and explicitly isolated dev pages for E2E.

If all schemas must remain unchanged: supply a real production Credits backend
and its nonsecret contract/configuration, including reservation and settlement
authority. Adapt that transport to the new geometry-bound protocol and use the
same new shared factory. A simulated balance/signature or a local request counter
is not an acceptable substitute. The present repository/configuration supplies
no such service; no external deployment or payment setup was attempted.

Both alternatives preserve the exact geometry provider, model, adapter,
geometry schema, measurement definition and references. Neither permits sending
raw split parents as additional logical transcription units. Existing adapter
code replaces split parents with child fragments before grouping. A read-only
scan of the 25 stored outputs found no logical-line pair with >90% smaller-box
coverage; this diagnostic is not a certified overlap threshold or E2E proof.

## Later gates remain unresolved

- Existing geometry-only numbers: 1,339/1,378 matched reference lines,
  97.1698% recall, versus required 99%; matched-gold area coverage is 89.4463%.
  These are historical geometry diagnostics, not new production A–C scores.
  Stable output does not imply dev exit. Geometry repairs remain prohibited.
- The frozen measurement contract explicitly states that a full executable A–C
  scorer is not certified. Existing metrics.py does not provide the required
  Greek/Latin errors from the same character alignment. The user now forbids
  scorer changes, so milestone 3 also needs an identified approved scorer or
  an explicit decision allowing its contract-conforming implementation.
- No full production dev runs, no new substantive product iteration, no blind
  ledger, no blind output directory and no blind authorization were created.

## Validation and disposition

`python3 -m pytest -q scripts/ocr/broker/test_broker.py`: 14 passed.
`cargo test -p mpdf-api-client --lib`: 15 passed with a one-time sandbox exemption
for loopback test listeners. Initial sandbox run had 12 pass/3 listener permission
failures; these were environment failures, not suppressed test failures.
These tests use test transports and prove no real production E2E behavior.

Task execution: **aligned** with intent §8.8 and the protected-stop rule.
Requested actual product chain: **unverified**, known factory/entry wiring absent.
No evidenced conforming production state exists in the inspected history;
the earliest historical origin of the wiring gap is not established.

Milestone 0: freeze checkpoint requested after this audit; identify it with
`git log` (this document is included in that commit).
Milestone 1: **MILESTONE_BLOCKED — PRODUCTION_SERVICE_CONTRACT_MISMATCH**.
Milestones 2–4: **NOT_RUN / NOT_STARTED**. Do not report FROZEN_BLIND_READY.
