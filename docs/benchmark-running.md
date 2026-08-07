# Running a benchmark

See [`benchmark-metrics.md`](benchmark-metrics.md) for what the numbers
mean and [`benchmark-datasets.md`](benchmark-datasets.md) for the
dataset/profile manifest formats.

## Quick start (no PDFium required)

```bash
cargo build -p museion-binarize-cli --release
./target/release/museion-binarize benchmark run \
  --dataset test-data/benchmark/synthetic-v1/dataset.toml \
  --profile test-data/benchmark/profiles/baseline.toml \
  --report /tmp/museion-benchmark.json
```

The raster benchmark level never touches PDFium — this reproduces on
any machine with the workspace built, with no library provisioning
step. See `docs/benchmark-results/synthetic-v1.md` for the actual
recorded output of this exact command.

**Use `--release` for any timing you intend to report.** Content-
fidelity numbers (F1, PSNR, DRD, confusion counts, compressed bytes)
are identical between debug and release builds; only timing differs,
and debug timings must never be published as performance evidence.

## Validating a dataset/profile without running anything

```bash
museion-binarize benchmark validate \
  --dataset test-data/benchmark/synthetic-v1/dataset.toml \
  --profile test-data/benchmark/profiles/baseline.toml
```

Checks manifest schema, dataset-root path containment, ground-truth
file validity (strict binary PNG, correct polarity), image dimensions,
ROI bounds, and profile run settings — without processing a single
page. Useful before committing or sharing a new dataset.

## `--json` and `--report`

```bash
museion-binarize benchmark run --dataset ... --profile ... --json --pretty
```

follows the same stdout/stderr contract as every other command (see
`docs/cli.md`): exactly one JSON document on stdout, progress on
stderr. `--report <path>` writes the same report to a file atomically,
subject to `--overwrite`, using the same path-alias-rejection helper
`process`/`analyze`/`estimate --report` already use — a `--report` path
that resolves to the dataset or profile manifest is rejected before
anything runs, the same "reject before touching anything" discipline
`docs/cli.md`'s path-aliasing section documents for every other
command.

## Reading the report

Top-level fields (see `docs/reporting.md` for the full schema):

- `dataset.manifest_digest` / `profile.digest` — SHA-256 of the exact
  manifest bytes that produced this report. Two reports with different
  digests were not run against the same dataset/profile, even if the
  file paths look the same.
- `environment.benchmark_level` — always `"raster"` in this milestone
  (see `benchmark-metrics.md`, "Benchmark levels").
- `runs[].aggregate` — `macro_f1`/`micro_f1` (see
  `benchmark-metrics.md` for why both are reported), DRD statistics,
  mean bytes/megapixel, processing-time statistics.
- `runs[].category_aggregates` — the same aggregate shape, grouped by
  each page's `category`.
- `runs[].roi_tag_aggregates` — the same aggregate shape again, grouped
  by ROI `tag` across every page (see `benchmark-datasets.md`, "Why
  ROIs matter").
- `runs[].worst_pages` — lowest F1, highest DRD, largest compressed
  page, slowest page. Metric-specific, not a scholarly judgement about
  the page's content.

## Reproducibility

Running the same dataset/profile twice produces identical content
fields (confusion counts, F1, PSNR, DRD, compressed bytes, category/ROI
aggregates) — verified by
`running_the_committed_suite_twice_is_deterministic_in_content_fields`
in `crates/museion-binarize-core/tests/benchmark_suite.rs`. Timing
fields are runtime-dependent and are not part of that guarantee.

Regenerating the committed `synthetic-document-v1` suite from its
generator reproduces every PNG byte-for-byte:

```bash
cargo run -p museion-binarize-core --example gen_benchmark_fixtures -- /tmp/regen
diff -r /tmp/regen test-data/benchmark/synthetic-v1
```

(also verified automatically by
`benchmark_fixture_suite_regenerates_byte_identical_pngs`).

## Ad hoc single-method runs

There is deliberately no `--method`/`--dpi` flag on `benchmark run` —
only `--dataset`/`--profile`. The milestone specification's preferred
reproducibility model is a committed profile file, which fully
documents what was compared by itself; see
`docs/benchmark-datasets.md`, "Why profiles enumerate runs." To try one
method, write a one-run profile file rather than passing settings on
the command line.
