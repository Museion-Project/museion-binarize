# Persistent jobs and provider protocol

M2 provides the durable orchestration boundary for future OCR adapters. The
current implementation is intentionally provider-neutral: `mpdf job` creates
and inspects state, while `FakeProvider` is used by tests. No model, network
service, or shell command is invoked.

## Durable state

The SQLite database is opened in WAL mode with foreign keys enabled. A job has
one row per page. Workers claim the lowest queued page and receive a bounded
lease. Heartbeats extend the lease. A checkpoint transition is atomic: the
page artifact/checkpoint is stored before it is marked completed and the job
counter is updated in the same transaction. A worker crash leaves an expired
running page; recovery returns it to the queue (or marks it cancelled if the
job was cancelled). Completed pages are never deleted. Retryable failures are
bounded to three attempts; terminal failures make the job failed.

The store limits a job to 100,000 pages and rejects empty identifiers. Callers
must keep the database in a trusted local directory; provider input is a
digest, never an interpolated command line.

## Sidecar

Provider progress is versioned NDJSON: each line is a typed `SidecarMessage`
with `protocol: "mpdf-job"`, `protocol_version: "0.1"`, a job id, optional
page index, and bounded string payload map. A transcript must contain exactly
one `JobStarted` first and one `JobFinished` last, with at least one unique
page result/failure and one non-empty job id throughout. Readers and writers
reject malformed, truncated, reordered, duplicated, or post-finish records,
unknown protocol identities/major versions, and oversized input. Writers use
a create-new temporary file, flush/sync, then install it with a no-clobber
same-directory link, so a crash cannot turn a partial sidecar into a
successful result. A destination is never silently
overwritten.

## Provider provenance

Every response carries `engine`, `model`, `version`, typed string parameters,
the input asset SHA-256, and `execution_location`. This is the complete
provider boundary needed by later MDP provenance records; it does not commit
the package schema to a display name or a particular OCR vendor.

Provider attempts are also durable `provider_runs` rows. A successful run and
its page checkpoint are committed in one transaction; failed attempts retain
their bounded error and outcome across database reopen, so a transient
response cannot be mistaken for durable work.

See [ADR 0004](adr/0004-persistent-jobs-and-provider-contract.md) for the
compatibility and failure decisions.
