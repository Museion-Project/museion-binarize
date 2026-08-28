# ADR 0008: Consented API providers and portable remote tasks

Status: accepted for M6 on 2026-08-28.

## Context

M1–M5 deliberately keep PDF processing, OCR evidence, derived documents, and
bookmark review useful without a network. M6 adds an optional remote provider
path without creating a second MDP model or weakening the M2 durable-job
contract. The risky boundaries are content upload, credentials, retries,
provider cost, remote retention, and importing a task on another device.

No vendor was selected for M6. A vendor-specific request or response shape
would therefore be an unstable product dependency and would make tests depend
on a live paid service.

## Decision

### Layering

- `mpdf-core` remains network-free. It owns the versioned remote-task IR,
  consent and budget policy, durable SQLite audit records, typed provider
  result mapping, and test transports.
- A reusable `mpdf-api-client` crate owns HTTPS and native credential-store
  integration. CLI and desktop use this crate; neither implements a parallel
  protocol.
- Remote OCR results map into the existing `OcrPage`/MDP extension and retain
  the exact bounded provider response as a digest-addressed artifact. The
  source OCR observation is never overwritten. Bookmark/AI operations may use
  the same task protocol later, but M6 implements remote OCR only.
- Local OCR stays the default and remains fully functional when the API crate,
  credentials, or network are unavailable.

### Protocol

The client speaks `mpdf-api` version `0.1` to a configured service:

1. `POST /v1/tasks` creates or finds a task from metadata and content digest.
2. `PUT /v1/blobs/{sha256}` uploads source bytes only after explicit consent.
3. `POST /v1/tasks/{id}/start` starts processing.
4. `GET /v1/tasks/{id}` returns durable state and integer cost usage.
5. `GET /v1/tasks/{id}/result` returns typed OCR results plus the raw artifact.
6. `DELETE /v1/tasks/{id}/content` requests provider-side content deletion.

Every mutating request carries a stable `Idempotency-Key`. The client stores a
request/response digest, status, byte count, attempt, integer monetary amount,
and timestamps, but never a bearer token or unredacted request body in SQLite.
Unknown protocol majors, unbounded bodies, missing digests, cost regressions,
and invalid state transitions fail closed.

The default client accepts HTTPS only, rejects URL credentials and fragments,
does not follow redirects, and applies connect/read/whole-request timeouts.
Plain HTTP is available only for an explicitly enabled literal loopback
address in tests and local development. Authentication uses an `Authorization`
header populated after the request log is built; tokens never appear in URLs,
CLI arguments, reports, or errors.

### Upload consent and routing

`api plan` creates a canonical, no-clobber JSON plan containing the endpoint
origin, operation, provider/model identifiers, source SHA-256 and byte length,
page count, retention request, maximum cost in integer micros, currency, and a
plan digest. It contains no source path or credential.

`api run` requires the source to match that plan and requires the caller to
repeat its digest as the upload-consent value. Consent authorizes both remote
processing and any upload; deduplication does not bypass it. The desktop shows
the same fields and requires an affirmative action immediately before the
first network request. Changed content, endpoint, model, retention, or budget
invalidates consent.

Routing is explicit: `local`, `api`, or `api_then_local`. A remote failure or
budget exhaustion never silently changes providers. `api_then_local` may fall
back only when the user selected it before the run, and the audit must record
the reason. Budget exhaustion pauses the remote task; it is not a successful
completion.

### Credentials

Profiles use a stable non-secret identifier. The OS credential store holds the
token under service `org.mpdf.api`; configuration and SQLite hold only the
profile identifier. The CLI accepts a secret for storage through stdin, never
an argv value. Status reports only presence/absence. Delete removes the native
credential. Tests use an in-memory `SecretStore` and cannot touch a developer's
credential store.

The production implementation uses the platform credential store (Keychain
Services on macOS, Windows Credential Manager, and Secret Service on Linux).
Unavailable or locked stores produce a diagnostic and do not fall back to a
plaintext file.

### Portable task receipt and retention

After task creation the client writes a canonical no-clobber receipt containing
the service origin, remote task ID, request ID, operation, source digest,
provider/model, budget, retention request, and protocol version. It contains no
credential, local path, or document text. Another device can import this
receipt, select a local credential profile for the same origin, poll the task,
and retrieve verified results.

The safe retention default is `delete_after_result`. A successful result is
not reported as fully finalized until its artifact is durably installed and
the deletion request is either acknowledged or recorded as a visible pending
retention action. Cancellation and deletion are distinct audit events.

## Retry and resource limits

- Retry only transport timeouts, HTTP 408/429, and 5xx responses.
- Use at most three attempts with the same idempotency key; honor bounded
  `Retry-After` and expose retry state to cancellation.
- Do not retry authentication, consent, schema, digest, budget, or other 4xx
  failures.
- Bound request metadata, response headers/bodies, raw provider artifacts,
  task counts, polling duration, and audit messages before allocation.
- Install artifacts with a same-directory temporary file, sync, digest check,
  and atomic no-clobber persistence. Partial or stale results never enter MDP.

## User surfaces

The CLI provides `mpdf api credential set/status/delete`, `api plan`, `api
run`, `api status`, `api cancel`, `api import`, and `api delete-content`, all
with stable JSON and exit categories. The desktop exposes Local / Cloud
enhanced / Cloud then local routing, credential presence, the consent summary,
durable progress/cancel/resume, cost and retention status. It never displays or
round-trips the secret value.

## Non-goals

- No automatic upload, endpoint discovery, telemetry, or analytics.
- No vendor SDK or live paid-service test in required CI.
- No cloud LLM bookmark generation in M6.
- No browser-based OAuth, account system, billing purchase, or hosted M PDF
  service implementation.
- No destructive MDP migration and no change to local output semantics.
- No M7 signing, notarization, packaging, repository rename, or final brand.

## Consequences

M6 can validate real HTTP behavior against a deterministic loopback fixture
while keeping CI free of secrets and paid calls. A conforming hosted service
can be added independently of the document model. Cross-device continuation is
portable but not automatic synchronization: the user must move a non-secret
receipt and configure a credential on the second device.
