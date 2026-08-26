# CLI

`mpdf` — commands, global options, the stdout/stderr contract,
and the exit-code table. For JSON report field meanings, see
[`reporting.md`](reporting.md).

## Commands

| Command | Purpose |
|---|---|
| `info` | Print project/build information. Never touches PDFium unless `--probe-pdfium` is passed. |
| `inspect` | Page count, geometry, rotation, and render sizes for a PDF. |
| `analyze` | Render and binarize a PDF through the real pipeline, without writing an output PDF. For choosing settings and scripting — not a benchmark. |
| `estimate` | Sample a handful of pages through the real pipeline and extrapolate an experimental output-size estimate, without writing an output PDF. See [`size-estimation.md`](size-estimation.md). |
| `process` | Convert a PDF into a bilevel CCITT Group 4 PDF. |
| `preview` | Render and process one page, saving a PNG. |
| `benchmark run` / `benchmark validate` | Ground-truth binarization-fidelity benchmarking against a dataset/profile manifest. **`benchmark` requires pixel-accurate ground truth; `analyze` is not a benchmark.** See [`benchmark-running.md`](benchmark-running.md). |
| `package create <PDF> --output <DIR>` / `package validate <DIR>` | Create or validate an MDP 0.1 evidence package. Creation records source digest and real page geometry without copying the PDF. See [`document-package.md`](document-package.md). |

Run `mpdf <command> --help` for the full flag list.

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

## Persistent document session

`inspect`, `analyze`, `process`, and `preview` each open the source PDF
**once** — see [`pdf-pipeline-session.md`](pdf-pipeline-session.md) for the
session architecture, the memory model, and the source-mutation policy
this implies.
