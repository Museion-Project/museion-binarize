# M6 API provider contract

M6 adds an optional, explicitly consented remote OCR route. The authoritative
architecture is [ADR 0008](adr/0008-consented-api-provider.md). This document
is the implementation and acceptance checklist.

## Frozen artifacts

- `mpdf-api-plan` `0.1`: immutable upload/processing consent summary.
- `mpdf-api-task-receipt` `0.1`: portable, non-secret cross-device handle.
- `mpdf-api-audit` `0.1`: append-only local request, cost, retry, retention,
  cancellation, fallback, and result-install events.
- `mpdf-api` HTTP `0.1`: task/blob/start/status/result/delete-content protocol.

All canonical JSON uses sorted deterministic collections and bounded strings.
Every file has a schema identity/version and a SHA-256 binding to its source
or parent record. Persistent schemas have strict JSON Schema files under
`schemas/` and reject unknown fields.

## State model

`planned -> consented -> creating -> upload_pending -> ready -> running ->
completed -> result_installed`

`paused_budget`, `paused_service`, `cancelling`, `cancelled`, and `failed` are
explicit states. A retention sub-state is one of `not_requested`, `pending`,
`acknowledged`, or `failed`. A task cannot be reported as fully finalized while
result installation or a requested deletion is ambiguous.

## Acceptance coverage

- Identical content and plan fields produce identical plan, request, and
  idempotency IDs; different endpoint/model/budget/retention fields do not.
- Duplicate content can skip blob transfer but cannot skip consent.
- Source mutation, stale receipt, response digest mismatch, unknown protocol,
  partial result, cost regression, and budget overrun fail closed.
- Redirects, URL credentials, non-HTTPS remote endpoints, oversized bodies,
  token leakage, and unapproved upload are rejected.
- 408, 429, 5xx, timeouts, bounded `Retry-After`, cancellation during backoff,
  resume after restart, and three-attempt exhaustion have deterministic tests.
- Credentials round-trip through an in-memory test store; reports, SQLite,
  artifacts, errors, process arguments, and frontend state never contain the
  secret. The client uses the platform-native credential store and reports
  unavailable or locked stores as a visible failure.

The reusable client is built with reqwest's rustls TLS backend. It rejects
redirects, URL credentials, query/fragment origin smuggling, and all HTTP
except an explicitly enabled literal `127.0.0.0/8` or `::1` fixture endpoint.
Source uploads are bounded to 512 MiB independently of the 8 MiB response and
raw-provider-artifact bound. Larger sources fail before any network request.
- Raw response artifacts are installed before MDP references them. Rebuilding
  the same MDP/DerivedDocument from a verified remote result is deterministic.
- `local` performs no network call. `api` never silently falls back.
  `api_then_local` records the authorized fallback and its cause.
- A receipt exported on one client can be imported by a fresh client and used
  to poll/download with a separately configured credential profile.
- CLI binary tests cover every command, JSON mode, exit category, no-clobber,
  alias/symlink safety, cancellation, and failure cleanup.
- Desktop tests cover route selection, credential-presence UI, consent,
  progress, cost/budget, cancellation, retention, resume, and fallback.

Required CI uses a deterministic loopback server and fake secret store. An
optional manual gate may target a real conforming service, but CI never needs a
real token and never spends money.
