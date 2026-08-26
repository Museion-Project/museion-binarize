# ADR 0004: Durable jobs and provider-neutral execution

Status: accepted for M2; real OCR is deferred to M3.

## Decision

Use a small SQLite database in WAL mode as the source of truth for job/page
state. A worker owns a page through a lease and commits one checkpoint per
page. State changes and job counters are single transactions. Expired leases
are recoverable, retries are bounded, and cancellation preserves completed
artifacts. Provider communication is a versioned `mpdf-job` NDJSON contract;
the core ships a deterministic fake provider for contract tests only.

## Compatibility and safety

The protocol identity and version are checked on every message and unknown
major versions fail closed. A transcript has strict start/page/finish framing
and bounded records. The database is local and bounded (including page count,
retry count, payloads, and errors). Sidecars are installed atomically and
incomplete records are invalid, so a process crash cannot be interpreted as
success. Provider requests carry identifiers and SHA-256 digests rather than
paths or shell fragments. Provider attempts are durable and a successful
attempt is committed with its page checkpoint. Future OCR adapters may add
fields, but must preserve provenance and must not bypass the MDP validator.

## Consequences

The desktop can reconstruct a compact progress snapshot after restart from
the same store. A provider timeout/crash is a failed attempt, not a committed
page. Out-of-order and protocol-mismatch responses remain explicit errors.
The design intentionally does not prescribe OCR models, network access, or
the eventual MDP text schema; those belong to later milestones.
