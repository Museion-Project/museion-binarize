# OCR providers: Local, Gemini BYOK, M PDF Credits

Local OCR is the default and needs no part of this document. Read it only if
you are considering sending page images to a cloud model, or you maintain the
code that does.

The architecture and the reasoning behind it are in
[ADR 0010](adr/0010-provider-neutral-ocr-and-cloud-modes.md). This page is the
operational reference: what each mode does, what leaves your machine, what it
costs, and what is not finished.

## The three modes

| | `local` | `gemini-byok` | `mpdf-credits` |
|---|---|---|---|
| Default | **yes** | no | no |
| Network | none | Google Generative Language API | M PDF service |
| Credential | none | your key, in your OS keychain | short-lived job token |
| Who is billed | nobody | you, by Google | you, in credits |
| Executes | this machine | Google | M PDF's service |
| Production ready | yes | code-complete, unvalidated live | **no service exists** |

All three produce the same evidence: a block/line/word tree with measured
rectangles. Everything downstream — the searchable text layer, contents-page
parsing, printed-page mapping, bookmark generation — treats them identically.

## What actually leaves your machine

Only in a cloud mode, and only after explicit consent:

- A rendered PNG of each page that is *routed to OCR*. Pages with usable
  embedded text are never rasterized or uploaded.
- The transcription prompt. It is fixed, versioned, and contains no document
  metadata, no file name, and no path.

Never sent, in any mode: the source PDF itself, file names, paths, the
workspace, bookmark decisions, or any identifier of you beyond whatever your
own API key implies to your provider.

The raw transcription is **not** kept on disk. The canonical block tree is the
evidence; a second free-text copy of every page would be an extra place for
personal data to live. Only its SHA-256 is recorded, as part of the audit
trail.

The full-page request uses the same 32,768-token ceiling as the measured
holdouts. A response ending in `MAX_TOKENS` is rejected as incomplete and
follows the selected fallback policy; a truncated prefix is never installed
as if it were a complete page. If a completed provider response later fails
alignment or geometry validation, its token/credit usage remains visible even
though the canonical page keeps local text.

## How a cloud page is recognized

```text
original page raster
  1. LOCAL detector      blocks, lines, words, rectangles   (always runs first)
  2. cloud transcription whole page, no region hints
  3. line split          deterministic; NFC; whitespace collapsed
  4. alignment           monotone, injective match to the local lines
  5. gates               coverage / order / injectivity
  6. canonical OcrPage   local rectangles + cloud text
```

Step 1 runs even for pages whose cloud call will fail. That is what makes a
failure cheap: the page is already recognized, so all that is lost is the
cloud text.

**Cloud modes still need local models.** The detector is the geometry source,
not a backup. `mpdf run --ocr-provider gemini-byok` fails with the same
"provision the models first" error as a local run.

### The gates

| Gate | Effect when it fails |
|---|---|
| Line similarity below 0.55 | that line keeps its local text |
| Page coverage below 0.60 | the whole page keeps local text; `alignment_coverage_below_gate` |
| Reading-order inversion | impossible by construction; counted in evidence anyway |
| Duplicate use of a transcribed line | impossible by construction |
| Transcribed line matching nothing | discarded — never given invented coordinates |
| Locally detected line matching nothing | keeps its local text — never deleted |

No gate failure can produce a page with an empty text layer. The worst case is
a page whose text is entirely local, recorded as such.

### Confidence

A replaced line's confidence is *corroboration between two independent
readings*, never the model's opinion of itself. Two readings that agree raise
confidence above either alone; weak agreement caps it, so a confident local
engine cannot lend certainty to a line it was just overruled on.

## Using your own Gemini key

```bash
pbpaste | mpdf provider credential set --slot default
```

The key is read from stdin and handed to the OS keychain
(`org.mpdf.model-provider`). There is deliberately no `--api-key` flag: a key
on a command line is in your shell history, in `ps` output, and in every CI
log that echoes the command.

Production BYOK transport is pinned to
`https://generativelanguage.googleapis.com`. A renderer or CLI request cannot
redirect a stored Google key to another HTTPS origin. Custom endpoints are
accepted only as loopback fixtures under an explicit development transport
policy, and the endpoint participates in the checkpoint fingerprint.

```bash
mpdf provider test --mode gemini-byok --slot default
```

A metadata request. It transcribes nothing and bills nothing.

```bash
mpdf run book.pdf --output book.out.pdf \
  --ocr-provider gemini-byok --cloud-consent \
  --language greek-ancient-german-english
```

`--cloud-consent` is required and has no default. Add `--dry-run` first to see
exactly what would be uploaded without contacting anything.

To go back to a fully offline configuration:

```bash
mpdf provider credential delete --slot default
```

### When a page fails

`--cloud-fallback local` (the default) recognizes that page locally, records
the fallback in the evidence, and lists the affected pages in the run report.
`--cloud-fallback fail` stops the run and writes nothing.

Local is the default because a 400-page book should not be discarded over one
transient 503, and because a fallback is never silent: it appears in the
per-page evidence, in the JSON report's `cloud_fallback_pages`, in the CLI
summary, and in the desktop completion panel. Choose `fail` when mixed
provenance is unacceptable to you regardless.

BYOK does not claim exactly-once execution or billing. Google does not
document an idempotency contract for `generateContent`, so the client retries
pre-request connection failures and explicit retryable HTTP responses, but it
does not silently retry an outcome-ambiguous timeout. Such a page can still
fall back locally; its evidence and final report carry
`transport_outcome_unknown`, and token/cost totals are labelled as lower
bounds rather than zero.

## M PDF Credits

The client protocol, the reservation/settlement/refund state machine, the
idempotency rules and the signature verification are implemented and tested
against an in-process fake service.

**There is no production service, and no credits can be bought.** Production
CLI and desktop builds compile out the endpoint path and development receipt
secret; the protocol fixture is reachable only from a build made explicitly
with the `dev-credits` Cargo feature. Outstanding
blockers, printed verbatim by `mpdf provider list` and by the desktop provider
picker:

- no production Credits service is deployed; only the in-process fake backend exists
- no payment provider is integrated and no real credits can be purchased
- server-side custody of the platform model credential is unimplemented
- receipt signing keys are not provisioned, so signatures are verified against a test secret only
- the privacy policy, data-retention window and deletion endpoint for uploaded page images are not published

The design constraint that shapes the protocol: the client never receives the
platform's provider key. It holds a short-lived, job-scoped token whose only
authority is over its own reservations. A client that could read the
platform's key would be a client that could spend the platform's money, and
every desktop binary would be a copy of it.

Money moves like this: the estimate is **reserved** before the first page is
encoded, so insufficient balance fails pre-flight; each page is recorded
against a **derived** idempotency key, so a retry bills once; on completion
the reservation **settles** what was used and **releases** the rest;
cancellation and failure charge the pages that completed and release the
remainder; a settled reservation can be **refunded** exactly once. Every
settlement is HMAC-signed and verified before it is believed.

The reservation deliberately **over-estimates**. It covers every page in the
document, including pages that already have usable embedded text and will
never be sent, and — on a resumed run — pages whose evidence is already
committed and will not be re-recognized. Reserving the ceiling and releasing
the difference is the conservative direction: a run can never discover
mid-book that it cannot afford to finish, and nothing that was not executed is
charged.

## Threat model

What an attacker in each position can and cannot do, and what stops them.

| Position | What they want | What stops them |
|---|---|---|
| Anyone reading logs, crash reports or CI output | the API key | Provider error bodies are redacted and bounded before reaching any caller; 401/403 bodies are dropped unread; no `Debug` impl prints a `Secret`; no `--api-key` flag exists, so a key is never in `ps` or shell history |
| Anyone reading the workspace or an evidence bundle | the API key, or the document's text | Evidence holds a slot label and digests — never a key, never a reversible function of one; the raw transcription is not written at all, only its SHA-256 |
| A compromised webview renderer | to start an upload the user never authorized | Consent, credential presence, credit ceiling and production-readiness are all re-checked in the Rust command handler; the disabled button is a courtesy, not the gate |
| A hostile or confused model response | to place invisible text where no glyph is, or blank a page | Coordinates come from the local detector; structured rectangles are disabled and, when evaluated, cross-validated against measured geometry; alignment is monotone and injective; below the coverage gate the page keeps local text |
| A hostile provider response | to exhaust memory or the disk | Bounded response reads, a line cap, a page-image upload cap, per-page deadlines, and the existing typed-evidence validator on the way in |
| A network position between client and provider | to redirect a credential-bearing request | HTTPS required (loopback HTTP only in an explicit development mode), redirects disabled, no credential in a URL or query string |
| Someone replaying a settlement from another tenant | to make a run look paid | Settlements are HMAC-signed over their own contents and verified before they are believed; a wrong key or an edited number fails |
| A user retrying a failed 400-page run | to be billed twice | Idempotency keys are derived from the work (document, page index, page image digest, provider config), never from the attempt; the reservation is held once and a repeated page overwrites its own record |
| A client trying to spend the platform's money | the platform's provider key | The brokered protocol has no field capable of carrying it; the client holds a short-lived, job-scoped token whose only authority is over its own reservation |

The residual risks are honest ones: a user who chooses a cloud mode has
decided to send page images to a third party, and this project cannot make
that private. It can only make it explicit, per run, with the page count
stated, and easy to reverse.

## Credentials and secrets

- Keys live in the OS credential store, never in settings, never in the
  workspace, never in a log.
- Settings and evidence hold a slot *label*, not a key and not a digest of
  one: a hash of a 39-character key is a fine oracle for anyone who can guess
  candidates.
- Provider error bodies are redacted and length-bounded before they reach any
  caller. 401 and 403 bodies are dropped unread, because this provider's
  bodies sometimes quote the key back.
- Rotating a key in a slot does not invalidate evidence. Switching slots does.
- The test suite runs a canary key through the failure paths and asserts it
  appears in no evidence record, no report, no serialized state, and no file
  the run wrote. A separate scan asserts no tracked file in the repository
  contains a credential-shaped string.

## Checkpoints and resumption

The durable fingerprint binds: provider mode, provider version, model and
pinned model version, prompt digest, document and page-image digests, DPI,
language profile, local detector identity and weights, alignment version,
structured-bbox policy and gates, and fallback policy. Changing any of them
starts a new job rather than resuming across a change in what the evidence
*is*.

Durable page files live under a fingerprint-derived run namespace. The small
`ocr/active-run.json` pointer selects one complete or resumable run for package
readers. A new provider, model, prompt, language or DPI therefore cannot adopt
`pNNNNNN.json` records left by another job merely because the workspace is
reused.

It binds nothing derived from a secret.

A **local** run's fingerprint remains byte-identical to what it was before
cloud modes existed. Its digest is now also the filesystem namespace, rather
than only a SQLite field.

## Model-returned rectangles

Disabled, in every build. `--structured-bbox evaluate-with-fallback` exists
for evaluation only: it parses under a strict schema, cross-validates every
rectangle against the locally measured page, and falls back deterministically
when any gate fails. It has not met the thresholds that would make it a
default, and until it does no model coordinate is ever written into a PDF.
The evaluation request uses a separate, fingerprinted JSON instruction; it
does not reuse the ordinary transcription prompt that explicitly requests
plain text. A valid JSON response that fails a geometry gate is recorded as
`geometry_gate_failed` and is never reinterpreted as transcription text.

## Licensing and terms

Using `gemini-byok` makes you the API customer: Google's terms, pricing and
data-handling policy apply to your key and your documents, and this project
neither accepts them on your behalf nor proxies your traffic. Review them
before uploading material you do not own. The project ships no Google SDK; the
transport is a direct HTTPS call over the same `reqwest`/`rustls` stack
already in the dependency set.

See also [`limitations.md`](limitations.md) and
[`distribution.md`](distribution.md).
