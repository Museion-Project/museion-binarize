# CLI

`mpdf` — commands, global options, the stdout/stderr contract,
and the exit-code table. For JSON report field meanings, see
[`reporting.md`](reporting.md).

## Commands

| Command | Purpose |
|---|---|
| **`run`** | **The main flow.** One original PDF in, one final PDF out: the original pages are OCR'd, bookmarks are compiled from that text, the visible pages are binarized, and a single searchable, outlined, bilevel PDF is assembled and verified. See below. |
| `info` | Print project/build information. Never touches PDFium unless `--probe-pdfium` is passed. |
| `inspect` | Page count, geometry, rotation, and render sizes for a PDF. |
| `analyze` | Render and binarize a PDF through the real pipeline, without writing an output PDF. For choosing settings and scripting — not a benchmark. |
| `estimate` | Sample a handful of pages through the real pipeline and extrapolate an experimental output-size estimate, without writing an output PDF. See [`size-estimation.md`](size-estimation.md). |
| `process` | Convert a PDF into a bilevel CCITT Group 4 PDF. |
| `preview` | Render and process one page, saving a PNG. |
| `benchmark run` / `benchmark validate` | Ground-truth binarization-fidelity benchmarking against a dataset/profile manifest. **`benchmark` requires pixel-accurate ground truth; `analyze` is not a benchmark.** See [`benchmark-running.md`](benchmark-running.md). |
| `package create <PDF> --output <DIR>` / `package validate <DIR>` | Create or validate an MDP 0.1 evidence package. Creation records source digest and real page geometry without copying the PDF. See [`document-package.md`](document-package.md). |
| `ocr <PDF> --output <DIR> --jobs-db <FILE> --job-id <ID>` | **Expert/debugging step.** Runs durable local per-page text routing and writes typed `ocr/` MDP extension records. It always reads the PDF you give it, so **give it the original** — never a converted output. Prefer `run` for ordinary use. |
| `export <MDP> --format <FORMAT> --output <PATH>` | Build deterministic JSON/JSONL/Markdown/TXT/HTML/hOCR/ALTO derived records; use `--format all` with an output directory. |
| `review <MDP>` | Emit the typed local review queue (`--json` for machine-readable output). |
| `revision add/list <MDP>` | Append a human or AI-suggested revision, or inspect the revision overlay; stale base evidence is rejected. |

Run `mpdf <command> --help` for the full flag list.

## `mpdf run` — the main flow

```sh
mpdf run book.pdf --output book-bw.pdf
```

That is the whole ordinary invocation. The command:

1. reads the original PDF and builds its evidence package;
2. **recognizes text on renders of the original pages**;
3. derives the text layer and rebuilds printed logical lines;
4. compiles bookmarks from that evidence;
5. pauses for review if any entry needs a human decision (nothing is written);
6. binarizes the **visible pages only**;
7. assembles one PDF carrying the binarized pixels, the *original* OCR
   coordinates as an invisible text layer, and the confirmed outline;
8. reopens the result and verifies geometry, text and outline independently.

The order is enforced by `mpdf_core::orchestrator`, which the desktop app uses
too. **OCR never reads a binarized page.**

### Configuration

Local OCR needs a sidecar and a provisioned model directory. Set them once:

```sh
export MPDF_OCR_SIDECAR="$PWD/scripts/ocr/mpdf_ocr_sidecar.py"
export MPDF_OCR_MODELS="$HOME/.local/share/mpdf/ocr-models"
python3 scripts/ocr/provision_models.py --target-dir "$MPDF_OCR_MODELS" --download
```

Nothing is ever downloaded by the application itself; `provision_models.py` is
a developer/packaging tool and verifies every file against the pinned
`distribution/ocr-models/manifest.toml`.

### Options

- `--output <PDF>` — the finished file. Required.
- `--overwrite` — replace an existing regular file at that path.
- `--language <PROFILE>` — default `auto` (polytonic Ancient Greek + German +
  English in one pass). See [`ocr-engines.md`](ocr-engines.md) for the list.
- `--on-review <pause|confirmed|reviewed>` — default `pause`: stop and write
  nothing when entries need a decision. `confirmed` writes only what is
  already confirmed; `reviewed` applies the decisions stored in the workspace.
- `--workspace <DIR>` — durable evidence directory. Defaults to a hidden
  directory beside the output. **Reusing it is what makes a run resumable**:
  OCR pages already committed and digest-verified are not recomputed.
- `--provider <tesseract|paddleocr|reference>` — advanced. `paddleocr` is an
  evaluation comparator and warns; `reference` recognizes nothing and exists
  for development and tests.
- `--ocr-sidecar`, `--models` — advanced overrides for the two environment
  variables above.
- `--ocr-dpi <N>` — advanced; the OCR raster resolution, unrelated to
  `--dpi`, which controls the binarized output.
- Every `process` binarization flag (`--dpi`, `--method`, `--sauvola-k`, …)
  applies to step 6.

### Choosing where recognition runs

Default: `--ocr-provider local`. Nothing leaves the machine, no credential is
read, and no network call is made. Everything below is opt-in.

- `--ocr-provider <local|gemini-byok|mpdf-credits>` — execution mode.
- `--cloud-consent` — **required** by every non-local mode. It acknowledges
  that a rendered image of every OCR'd page is uploaded to that provider.
  There is no default and no configuration file that can pre-supply it.
- `--cloud-fallback <local|fail>` — default `local`: a page the provider could
  not do is recognized here instead, and every such page is listed in the
  evidence and in the report. `fail` stops the run and writes nothing. Local
  is the default because one transient 503 on page 300 should not discard a
  400-page run, and because a fallback is never silent.
- `--credential-slot <NAME>` — default `default`. A slot **label**; see
  `mpdf provider credential` below. **There is no `--api-key` flag**, and
  there will not be one: a key on a command line is in your shell history, in
  `ps` output, and in every CI log that echoes the command.
- `--cloud-model`, `--cloud-model-version`, `--cloud-endpoint` — advanced
  pins. The version is never `latest`: a floating alias would let a resumed
  job re-run against different weights.
- `--structured-bbox <disabled|evaluate-with-fallback>` — default `disabled`.
  Model-returned rectangles are never a coordinate source in this build.
- `--credits-per-page`, `--max-credits` — M PDF Credits only. `--max-credits`
  is a hard ceiling; the run refuses to start rather than exceed it.
- `--dry-run` — print exactly what would be uploaded and what it would cost,
  then exit. Opens nothing, calls nothing, reserves nothing, charges nothing.

Cloud modes still need the local sidecar and models: the local detector
supplies the geometry that the cloud transcription is aligned onto. See
[`ocr-providers.md`](ocr-providers.md).

```sh
mpdf run book.pdf --output book-bw.pdf --ocr-provider gemini-byok \
  --cloud-consent --language greek-ancient-german-english --dry-run
```

### Managing model-provider credentials

```sh
pbpaste | mpdf provider credential set --slot default   # stdin only
mpdf provider credential status --slot default          # present / absent, masked
mpdf provider credential delete --slot default          # back to fully local
mpdf provider list --json                               # modes, capabilities, blockers
mpdf provider test --mode gemini-byok --slot default    # metadata only; bills nothing
```

The key goes to this machine's OS credential store and is never shown again,
written to a settings file, included in a log, or placed in a checkpoint.

### Reviewing and continuing

```sh
mpdf run book.pdf --output book-bw.pdf          # pauses, writes nothing
mpdf bookmark list .book-bw.mpdf-workspace      # inspect what needs a decision
mpdf bookmark confirm .book-bw.mpdf-workspace --candidate bookmark-…
mpdf run book.pdf --output book-bw.pdf --on-review reviewed
```

The second `run` reuses the OCR evidence already in the workspace; it does not
re-recognize the book.

### Results that are not errors

- **`awaiting_review`** — entries need a decision. Exit code 0; no file
  written.
- **`safe_refusal`** — no reliable structure was found, so no bookmark was
  invented. Exit code 0; **no file written**. If a plain conversion is what
  you want, use `mpdf process`.
- **cloud fallback pages** — in a cloud run, `cloud_fallback_pages` in the
  JSON report lists every page whose text came from the local engine instead.
  The run completed and the PDF is valid; the field exists so "I paid for a
  cloud model" and "these twelve pages are Tesseract" can never be confused.

## Global options

Available on every command that opens a document:

- `--pdfium-library <PATH>` — explicit PDFium dynamic library path.
- `--allow-system-pdfium` — allow the OS library search path as a last
  resort. Off by default.

Available on every command with a report:

- `--json` — emit one machine-readable JSON report to stdout instead of
  human-readable text.
- `--pretty` — two-space-indented JSON. No effect without `--json` or
  `--report`.
- `--quiet` — suppress human progress/success text. The final result
  (human or `--json`) is still printed.

`analyze` and `process` additionally accept `--report <PATH>` to write the
same report to a file, atomically, subject to `--overwrite`.

## Password

There is no `--password` flag. A password is read only from the
`MPDF_PDF_PASSWORD` environment variable, so it never appears in a
command line, shell history, or process listing (`ps`):

```bash
MPDF_PDF_PASSWORD=secret mpdf inspect protected.pdf
```

It is never logged, serialized, or included in an error's JSON `context`.

## MDP packages

```bash
mpdf package create book.pdf --output book.mdp
mpdf package validate book.mdp
```

`package create` uses the same PDFium/session path as `inspect`, requires an
available PDFium library, and refuses to overwrite the destination. `package
validate` is local and does not open PDFium. Both commands support `--json`,
`--pretty`, and `--quiet`.

## Local OCR (expert step)

**For ordinary use, run [`mpdf run`](#mpdf-run--the-main-flow)
instead.** This command is the individual OCR stage, kept for debugging and
scripting. It recognizes whatever PDF you hand it, so it must be given the
**original** document: running it on a binarized output would recognize
one-bit pixels and produce a worse text layer than the original can.

```bash
mpdf ocr book.pdf --output book.mdp --jobs-db .mpdf/jobs.sqlite --job-id book-1 --provider reference
mpdf ocr scan.pdf --output scan.mdp --jobs-db .mpdf/jobs.sqlite --job-id scan-1 \
  --provider rapidocr --provider-executable /opt/mpdf-ocr-sidecar \
  --model-dir /opt/mpdf-ocr-models
```

The pipeline first asks PDFium for the native text layer. Reliable text is
recorded without rasterization; empty, very short, or garbled pages are
rendered one at a time and sent to the selected local provider. The
`ocr/summary.json` record is incomplete when any page fails, and the command
returns a processing error rather than claiming success. Provider execution
uses argv directly and never invokes a shell or downloads a model.
The SQLite job is source/provider-fingerprint scoped: rerunning the same
command verifies completed page files and skips their provider calls. A
missing file or digest mismatch fails closed; cancellation leaves committed
pages and returns the distinct cancelled exit category.
Transient provider failures are recorded as retryable page failures; a later
run with the same job id retries them and retains both provider-run records.
After cancellation, a new job id may adopt valid page files already present in
the same source-matching MDP directory; it never adopts malformed or
out-of-range files. RapidOCR fingerprints include the source, protocol,
configuration, and SHA-256 of each provisioned ONNX model file.

## Consented API OCR (M6)

The API path is opt-in and remains separate from local OCR. Create a plan
first; it contains source metadata and a digest but no path or secret:

```bash
mpdf api plan scan.pdf --endpoint https://provider.example --model ocr-1 \
  --page-count 12 --budget-micros 500000 --output scan.plan.json
mpdf api run scan.plan.json --source scan.pdf --consent "$(jq -r .plan_digest scan.plan.json)" \
  --profile work --jobs-db .mpdf/api.sqlite --artifact-dir .mpdf/artifacts \
  --receipt scan.receipt.json
```

`api run` performs no request until the consent digest matches the plan. A
deduplicated create still requires consent. Credentials are read from the OS
credential store; `api credential set --profile work` reads the token from
stdin and never accepts a secret argument. The native store is the platform
Keychain/Credential Manager/Secret Service; unavailability is a visible
failure, with no plaintext fallback. CI uses an in-memory store and a
deterministic loopback fixture. Receipts are portable handles and contain no
path, token, or document text. Use `--allow-loopback-http` only for a literal
loopback fixture; production endpoints must be HTTPS without URL credentials,
query strings, fragments, or redirects.

For headless CI and deterministic loopback fixtures only, select the explicit
credential profile `env` and provide `MPDF_API_TOKEN`. Other profile names use
the native OS credential store and are never overridden by that environment
variable.

## Page selection (`analyze --pages`)

One-based, matching what a user sees:

```text
all              every page (default)
3                page 3 only
1-5              pages 1 through 5
1,3,8-12         pages 1, 3, and 8 through 12
```

Rejected: page `0`, a reversed range (`5-1`), a page beyond the document's
actual count, and non-numeric input. Duplicate pages (`1,1,2`) are accepted
and deduplicated. See `PageSelection` in
`crates/mpdf-core/src/page_selection.rs` for the exact rules
and their tests.

## Estimating output size (`estimate`)

```bash
mpdf estimate book.pdf --dpi 400 --method sauvola --samples 8
```

`estimate` parses `--dpi`/`--method`/binarization settings exactly the way
`analyze` and `process` do — the same `SettingsArgs`, not a separate
parser — so an estimate's settings are guaranteed to match what a later
`process` call with the same flags would actually do.

- `--samples <N>` — how many pages to sample, deterministically and evenly
  spaced (default 8, range 1–32; a value above the document's page count
  quietly samples every page instead of erroring). See
  [`size-estimation.md`](size-estimation.md#sampling-policy).
- `--report <PATH>` — write the `mpdf-size-estimate` report to
  a file, atomically, subject to `--overwrite`; the same path-aliasing
  check used elsewhere rejects a report path that resolves to the input
  file.

The result is always labeled experimental — see
[`size-estimation.md`](size-estimation.md) for what the estimate is (and
is not) a guarantee of.

## Benchmarking (`benchmark run` / `benchmark validate`)

```bash
mpdf benchmark run \
  --dataset test-data/benchmark/synthetic-v1/dataset.toml \
  --profile test-data/benchmark/profiles/baseline.toml \
  --report /tmp/mpdf-benchmark.json

mpdf benchmark validate \
  --dataset test-data/benchmark/synthetic-v1/dataset.toml \
  --profile test-data/benchmark/profiles/baseline.toml
```

Unlike every other command, `benchmark run` does not take an `--input`
PDF directly — it takes a **dataset manifest** (pages with pixel-
accurate ground truth) and a **profile manifest** (which settings to
run). There is deliberately no `--method`/`--dpi` shortcut; a profile
file is the reproducibility unit — see
[`benchmark-datasets.md`](benchmark-datasets.md), "Why profiles
enumerate runs." `benchmark validate` checks both manifests (schema,
path containment, ground-truth validity, dimensions, ROI bounds,
settings) without processing any page.

`--report` uses the same atomic-write and path-alias-rejection helpers
as `process`/`analyze`/`estimate --report` — a report path that
resolves to the dataset or profile manifest is rejected before
anything runs. See [`benchmark-running.md`](benchmark-running.md) for
the full workflow and [`benchmark-metrics.md`](benchmark-metrics.md)
for what the resulting numbers mean.

**`benchmark` requires pixel-accurate ground truth. `analyze` is not a
benchmark** — it has no ground truth and computes none of the fidelity
metrics `benchmark` does.

## stdout / stderr contract

**Human mode (default):**
- stdout: the result (the report, formatted for reading).
- stderr: progress and diagnostics.

**`--json` mode:**
- stdout: exactly one JSON document. No prose before or after it, no ANSI
  escape sequences.
- stderr: progress (unless `--quiet`) and diagnostics — never anything
  that would end up mixed into a redirected stdout stream.

**On failure:** a versioned JSON error envelope is printed to stdout in
`--json` mode (never a mix of prose and JSON); in human mode the message
goes to stderr, prefixed `error:`. Either way, the process exits non-zero
per the table below. See [`reporting.md`](reporting.md#error-envelope) for
the error envelope's fields.

**`--quiet`:** suppresses human progress/success text; does not affect
`--json` output, error output, or the exit code.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 2 | command-line usage / invalid parameters |
| 3 | input or filesystem error |
| 4 | PDFium loading or PDF open error |
| 5 | rendering or image-processing error |
| 6 | output/write/validation error |
| 7 | cancellation |

The mapping from every `CoreError` variant to one of these codes is
centralized in `crates/mpdf-cli/src/errors.rs::classify`, so
no ordinary user-facing failure can escape through Rust's uncontrolled
panic exit code (101) — a panic here would mean a bug, not a documented
failure mode. The same function also produces the JSON error `code`
string (e.g. `"password_required"`, `"pdfium_library_not_found"`).

## Path aliasing

Every path a command accepts (`input`, `output`, `--report`) is checked
against every other one before any work begins: two different outputs
pointed at the same file would silently corrupt whichever is written
second. This is in addition to the core's own input/output same-file
protection (see [`pdf-output.md`](pdf-output.md)).

## Derived exports and review

```bash
mpdf export book.mdp --format all --output book-derived
mpdf review book.mdp --json
mpdf revision add book.mdp --revision-id r1 --target-ref word-... \
  --base-evidence-digest <page-evidence-sha256> --text corrected
mpdf revision list book.mdp --json
```

The derived IR retains page/bbox references and the artifact manifest records
input and output digests. `all` installs a complete export directory
atomically. Human revisions affect only the effective derived text; AI
suggestions remain append-only and stale base evidence is rejected. See
[`derived-document.md`](derived-document.md).

## Persistent document session

`inspect`, `analyze`, `process`, and `preview` each open the source PDF
**once** — see [`pdf-pipeline-session.md`](pdf-pipeline-session.md) for the
session architecture, the memory model, and the source-mutation policy
this implies.

## Persistent jobs (development API)

M2 exposes a local, provider-neutral job store for integration testing and
desktop recovery. It does not run OCR:

```bash
mpdf job create --db .mpdf/jobs.sqlite --job-id demo --pages 500
mpdf job status --db .mpdf/jobs.sqlite --job-id demo
mpdf job cancel --db .mpdf/jobs.sqlite --job-id demo
```

The store uses SQLite WAL mode. Workers claim pages with a lease, heartbeat
while processing, and commit each page checkpoint atomically. Expired leases
return to the queue; cancellation marks only unfinished pages cancelled and
never removes completed checkpoints. Provider adapters must speak the
versioned `mpdf-job` NDJSON contract and report engine/model/version,
parameters, input asset SHA-256 and execution location. See
[`document-jobs.md`](document-jobs.md) and
[`adr/0004-persistent-jobs-and-provider-contract.md`](adr/0004-persistent-jobs-and-provider-contract.md).

## Evidence bookmarks and searchable PDFs

### One command: compile a table of contents and write the PDF

```bash
mpdf bookmark auto book.mdp \
  --source source.pdf --output bookmarked.pdf [--overwrite] [--regenerate] --json
```

`bookmark auto` validates the package and its source binding, compiles
bookmarks from the document's own evidence, saves `bookmarks/candidates.json`
(schema 0.2) plus `bookmarks/generation-report.json`, and — when at least one
entry passed the deterministic gate — writes and verifies a searchable,
outlined PDF. It reports one of three outcomes:

| `status` | Meaning | Output written |
|---|---|---|
| `written` | A native outline was preserved, or a printed contents list was compiled | yes |
| `needs_review` | Entries were found but none reached the confidence gate | no |
| `safe_refusal` | No usable contents structure; nothing was guessed | no |

A refusal is a **successful command** with a structured result — exit code 0,
`"output_path": null`, and a `safe_refusal_reason` — not an internal error.
Text mode prints how many bookmarks were added, how many need review, how
many were skipped, and where the output went.

`--overwrite` authorizes replacing an existing regular *output file*;
`--regenerate` authorizes replacing existing *candidates and report*. Neither
implies the other, and `--regenerate` is still refused while human review
decisions exist for the current generation — the reviews are never deleted or
migrated by guesswork. Input/output aliases, symlinks, directories, digest
mismatches, and stale or partial OCR all fail closed.

The user does not choose an OCR provider, a font-size threshold, a page-number
offset, or a contents page: those are the engine's decisions, recorded in the
generation report. See
[`adr/0009-deterministic-automatic-toc-compilation.md`](adr/0009-deterministic-automatic-toc-compilation.md).

### Generation and review

`bookmark generate` runs the same engine and writes the same snapshot and
report, without producing a PDF:

```bash
mpdf bookmark generate book.mdp --json
mpdf bookmark list book.mdp --json
mpdf bookmark confirm book.mdp --candidate bookmark-...
mpdf bookmark edit book.mdp --candidate bookmark-... --title "Human title"
mpdf bookmark reparent book.mdp --candidate bookmark-... --level 2 --parent bookmark-...
mpdf pdf build-searchable book.mdp --source source.pdf --output searchable.pdf --json
```

The PDF command checks the source SHA-256 binding, writes invisible Unicode
text and confirmed outline entries without rasterizing existing pages, and
uses a same-directory temporary file plus PDFium reopen validation. Existing
outputs require `--overwrite`; the source can never be replaced. `bookmark
auto`, `pdf build-searchable`, and the desktop application all go through the
one shared output boundary in `mpdf_core::searchable_output`.

`bookmark list` labels each entry with its effective status. `auto_confirmed`
means the frozen deterministic gate added it; `confirmed` means a person did.
Both are written to the outline; `proposed`, `needs_review`, `skipped`, and
`rejected` are not. Confirming, editing, or reparenting an automatic entry
makes it `confirmed`, and its automatic score and reason remain in the record.
`bookmark list`, `confirm`, `reject`, `edit`, and `reparent` work with both
schema 0.1 and 0.2 snapshots.
