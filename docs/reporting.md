# JSON reporting

Every JSON document this project emits — to stdout via `--json`, or to a
file via `--report` — is one `ReportEnvelope` (or, on failure, one
`ErrorEnvelope`):

```json
{
  "schema": "museion-binarize-<kind>",
  "schema_version": "1.0",
  "tool": { "name": "Museion Binarize", "version": "0.1.0" },
  "result": { ... }
}
```

`schema` and `schema_version` are always at this same top-level location
regardless of which command produced the document, so a consumer can
dispatch on them without knowing the command in advance.

## Compatibility policy

- New fields may be added to `result` in a minor-compatible way (existing
  consumers ignoring unknown fields are unaffected).
- Removing or repurposing a field is a breaking change and bumps
  `schema_version`.
- `schema` names are stable identifiers, not free text; do not parse them
  for embedded meaning beyond exact-string matching.

## Determinism

**No report includes a wall-clock timestamp by default.** Two runs over
identical input and settings would otherwise produce different report
bytes even though nothing meaningful changed — easy to mistake for
nondeterminism in the conversion itself. Durations (see below) *are*
included and *do* vary run to run, so **report bytes are not claimed to be
deterministic** — only the converted output PDF is (see
[`pdf-output.md`](pdf-output.md)). This was verified directly: converting
the same input twice with `--report` produces different report files (due
to timing) but byte-identical output PDFs.

## Timing units

Every duration field is named `*_us` and is an integer number of whole
microseconds (`std::time::Instant`-based), never a floating-point number
of seconds — so a report contains exact integers with an explicit,
documented unit rather than a value whose precision looks more meaningful
than it is.

## Report kinds

### `museion-binarize-info` (`info --json`)

Static project/build information plus, only if `--probe-pdfium` was
passed, the result of resolving (and attempting to load) a PDFium
library. `info` never touches PDFium otherwise.

| Field | Meaning |
|---|---|
| `name`, `phase`, `version` | Project identity and current development phase. |
| `build_profile` | `"debug"` or `"release"`. |
| `target_arch`, `target_os` | `std::env::consts::{ARCH,OS}` — not a formal Rust target triple, which is not reliably available at runtime without a build script. |
| `supported_dpi` | The DPI values every command accepts (`[300, 400, 600]`). |
| `report_schemas` | Every schema name/version this binary can produce, including this one. |
| `pdfium.probed` | Whether `--probe-pdfium` was passed. |
| `pdfium.resolved` | Description of the bound library, if the probe succeeded. |
| `pdfium.error` | The probe's error message, if it failed. A failed probe still prints the rest of the report but exits non-zero (exit code 4; see [`cli.md`](cli.md)). |
| `limitations` | Short, human-readable strings — the same claims made in `limitations.md`, kept in sync by convention, not by generation. |

### `museion-binarize-inspect` (`inspect --json`)

| Field | Meaning |
|---|---|
| `source_path` | Rendered per `--path-mode` (`basename` by default); `null` for `omit`. |
| `source_bytes`, `page_count` | Size and page count of the exact snapshot opened. |
| `title`, `author`, `subject`, `keywords` | Sanitized source metadata; `null` if absent. |
| `pdfium_library` | Description of the PDFium library used. |
| `pages[].width_points`/`height_points` | Visible (post-rotation) page size, in PDF points. |
| `pages[].source_rotation_degrees` | The source `/Rotate`; informational only — never applied a second time. See [`pdf-output.md`](pdf-output.md). |
| `pages[].render_sizes[]` | Pixel dimensions at each of `supported_dpi`; `null` width/height for a DPI where the page geometry is out of range. |

### `museion-binarize-analysis` (`analyze --json` / `--report`)

Produced by `analyze` — real rendering and binarization through the same
pipeline `process` uses, without writing a reconstructed output PDF. Not
the Milestone 6 benchmarking framework: a low file size or black-pixel
ratio here is not evidence of preservation quality.

Document level:

| Field | Meaning |
|---|---|
| `source_path` | Per `--path-mode`. |
| `source_bytes`, `page_count` | Of the whole document. |
| `analyzed_page_count`, `failed_page_count` | How many selected pages were actually measured vs. failed (a page failure does not abort the run; see below). |
| `total_visible_area_points2` | Sum of analyzed pages' visible area, in square points. |
| `dpi`, `method` | The settings used. `method` is `"otsu"`, `"sauvola"`, or `"manual"` — the per-page `threshold` field carries the full configuration. |
| `total_duration_us` | Wall-clock duration of the whole analysis. |
| `page_duration` | `{min_us, max_us, mean_us, median_us}` over analyzed pages' `total_us`; `null` if no page was analyzed. Median of an even count is the arithmetic mean of the two middle values. |
| `pdfium_library` | As in `inspect`. |
| `pages[]` | See below. |

Per page:

| Field | Meaning |
|---|---|
| `page_index`, `page_number` | Zero-based / one-based. |
| `width_points`, `height_points`, `source_rotation_degrees` | As in `inspect`. |
| `pixel_width`, `pixel_height`, `pixel_count` | Of the rendered raster at the analysis DPI. |
| `grayscale.{min,max,mean,std_dev,pixel_count}` | Computed once, over the exact buffer binarization reads. `std_dev` is the **population** standard deviation (divide by N, the appropriate convention for a full pixel population, not a sample). |
| `threshold` | Tagged by `method`: `{"method":"otsu","threshold":128}`, `{"method":"manual","threshold":180}`, or `{"method":"sauvola","window_size":33,"k":0.2,"dynamic_range":128.0}`. Sauvola is local/adaptive — there is deliberately no single scalar threshold reported for it. |
| `ink.{black_pixels,white_pixels,black_pixel_ratio}` | Counted on the final (post-cleanup) bilevel page. `black_pixel_ratio` is never `NaN`: a zero-pixel page is rejected as a structured error before this could divide by zero. |
| `raw_raster_bytes_estimate` | `width * height * 3` — an estimate of the raw RGB raster's size, not a measurement of a buffer that was actually retained. |
| `packed_bilevel_bytes` | Size of the packed (8 px/byte) bilevel raster. |
| `ccitt_bytes`, `ccitt_bytes_per_pixel` | Present only when `--encode` was passed; CCITT Group 4 encoding is otherwise skipped as unnecessary extra work for `analyze`'s core purpose. |
| `stage_durations` | `{render_us, grayscale_prep_us, binarization_us, cleanup_us, ccitt_encode_us?, total_us}`. `grayscale_prep_us` combines grayscale conversion, contrast, and preprocessing (they run back-to-back over one buffer with nothing else between them, so separating them would not change any decision this project makes). |
| `warnings` | Reserved for future per-page notes; currently always empty. |

A page that fails to render or process is **not** included in `pages[]` —
it is counted in `failed_page_count` instead, and the rest of a long
document is still analyzed. `process` has the opposite policy (any page
failure aborts the whole conversion), because a partial output PDF is not
a valid PDF.

### `museion-binarize-process` (`process --json` / `--report`)

| Field | Meaning |
|---|---|
| `pages_processed` | All of them — `process` does not currently support a partial `--pages` selection (see `docs/cli.md`'s narrower-scope note). |
| `original_bytes`, `output_bytes` | Source and output PDF sizes. |
| `elapsed_us` | Whole conversion. |
| `page_reports[].{page_number,pixel_width,pixel_height,width_points,height_points,compressed_bytes}` | Per page. |
| `page_reports[].pixel_count`, `.black_pixel_ratio`, `.bytes_per_pixel` | Added Milestone 5; `bytes_per_pixel = compressed_bytes / pixel_count`, the same normalized quantity the size estimator uses. |
| `page_reports[].render_duration_us`, `.processing_duration_us`, `.encoding_duration_us`, `.total_page_duration_us` | Per-page timing breakdown; `processing_duration_us` combines grayscale/contrast/preprocessing/binarization/cleanup for the same reason `analyze`'s `stage_durations` does. |
| `page_reports[].warnings` | Simple, document-relative outlier flags (see "Outlier flags" below); omitted from JSON when empty. Never a quality judgement. |
| `pdfium_library` | As above. |

Aggregate metrics, added Milestone 5 (all additive to the `1.0` schema):

| Field | Meaning |
|---|---|
| `absolute_bytes_saved` | `original_bytes.saturating_sub(output_bytes)` — `0`, not negative, when the output is larger than the input. |
| `size_reduction_fraction` | `1.0 - output_bytes / original_bytes`; `0.871` means 87.1% smaller. `null` when `original_bytes` is `0`. |
| `input_to_output_ratio` | `original_bytes / output_bytes`. `null` when `output_bytes` is `0`. Distinct from `size_reduction_fraction` — this crate always uses these two specific names for these two specific quantities, never ambiguous terms like "compression" or "efficiency." |
| `total_pixel_count`, `total_black_pixels`, `overall_black_pixel_ratio` | Summed/derived across every processed page. |
| `total_ccitt_bytes` | Sum of every page's `compressed_bytes`. Normally somewhat less than `output_bytes`, since the completed PDF also carries container/structural overhead (see `docs/size-estimation.md`, "Container overhead"). |
| `mean_ccitt_bytes_per_page`, `median_ccitt_bytes_per_page`, `min_ccitt_bytes_per_page`, `max_ccitt_bytes_per_page` | Distribution of `compressed_bytes` across pages. |
| `mean_processing_duration_us`, `median_processing_duration_us` | Distribution of per-page `processing_duration_us`. |
| `slowest_page`, `largest_encoded_page`, `smallest_encoded_page` | `{page_number, value}` or `null`; the extremes by `total_page_duration_us` / `compressed_bytes` / `compressed_bytes` respectively. |
| `estimate_comparison` | `{estimated_output_bytes, actual_output_bytes, absolute_error_bytes, relative_error_fraction}`, or `null`. Present only when the caller supplied a prior estimate for the same document and settings (the desktop app does this automatically when a matching estimate was requested before conversion; the CLI does not cache estimates across separate invocations). |

#### Outlier flags

`page_reports[].warnings` entries are simple, document-relative thresholds
compared against the document's own median, not any absolute or
cross-document standard:

| Flag | Condition |
|---|---|
| `large_output` | `compressed_bytes > 2.0 x median_ccitt_bytes_per_page` |
| `slow_processing` | `total_page_duration_us > 2.0 x median_processing_duration_us` |
| `high_ink_ratio` | `black_pixel_ratio > median_black_pixel_ratio + 0.15` |
| `low_ink_ratio` | `black_pixel_ratio < median_black_pixel_ratio - 0.15` |

These describe *difference from the rest of this document*, nothing more.
A flagged page may simply contain a full-page illustration or a bold
diagram; it is not evidence of a scanning problem, and no downstream
consumer should treat it as one.

### `museion-binarize-size-estimate` (`estimate --json` / `--report`)

Produced by `estimate` (CLI) and the desktop app's "Estimate" action.
Always `experimental: true` — see `docs/size-estimation.md` for the full
methodology, including why the central estimate uses the mean rather than
the median, and why the range is called a "likely range" rather than a
confidence interval.

| Field | Meaning |
|---|---|
| `document_page_count` | Total pages in the document (sampled or not). |
| `sampled_pages[]` | See below; only the deterministically selected sample was actually rendered. |
| `sampled_total_encoded_bytes` | Sum of `sampled_pages[].ccitt_bytes` — a real, measured total for the sample itself, not an extrapolation. |
| `min_bytes_per_pixel`, `mean_bytes_per_pixel`, `median_bytes_per_pixel`, `max_bytes_per_pixel` | Distribution of sampled `bytes_per_pixel`. |
| `range_method` | `"quartiles"` (4+ samples) or `"min_max"` (fewer). |
| `estimated_output_bytes` | Central estimate: `mean_bytes_per_pixel` extrapolated over every document page's true pixel count, plus the writer's measured PDF container overhead. |
| `estimated_lower_bytes`, `estimated_upper_bytes` | Same extrapolation using the range method's lower/upper bound. |
| `dpi`, `method` | Settings used, matching `analyze`'s convention. |
| `settings_fingerprint` | Opaque stable string encoding every setting that affects output bytes; not meant to be parsed. |
| `estimate_total_duration_us`, `mean_sample_duration_us`, `median_sample_duration_us` | Timing of the estimate itself. |
| `pdfium_library` | As above. |
| `experimental` | Always `true`. |

Per sampled page:

| Field | Meaning |
|---|---|
| `page_number`, `page_index` | One-based / zero-based. |
| `width_points`, `height_points`, `raster_width`, `raster_height`, `pixel_count` | Geometry at the estimate DPI. |
| `black_pixel_ratio` | As in `analyze`. |
| `packed_bytes`, `ccitt_bytes` | Packed bilevel size and CCITT-encoded size, measured (not extrapolated) for this page. |
| `bytes_per_pixel` | `ccitt_bytes / pixel_count` — the normalized quantity the whole estimate is built from. |
| `processing_duration_us` | Time to render, binarize, and encode this one sampled page. |

### `museion-binarize-preview` (`preview --json`)

| Field | Meaning |
|---|---|
| `output_path` | The PNG that was written. |
| `page_number` | One-based. |
| `pixel_width`, `pixel_height` | Of the written PNG. |

### `museion-binarize-benchmark` (`benchmark run --json` / `--report`)

Ground-truth binarization-fidelity benchmark results — not the same
thing as `analyze`, which has no ground truth. See
[`benchmark-metrics.md`](benchmark-metrics.md) for what every field
means and [`benchmark-datasets.md`](benchmark-datasets.md) for the
dataset/profile manifests referenced below.

| Field | Meaning |
|---|---|
| `metrics_schema_version` | Independent of the report's own `schema_version` — versions the metric *definitions* (e.g. DRD's formula), so a future change to how a metric is computed is visible even if the report's field shape did not change. |
| `dataset.{id,title,manifest_digest,page_count}` | `manifest_digest` is the SHA-256 of the exact dataset manifest bytes used. |
| `profile.{id,digest}` | SHA-256 of the exact profile manifest bytes used. |
| `environment.{os,arch,tool_version,benchmark_level}` | `pdfium_library` is present only at the (currently unimplemented) PDF benchmark level. No hostname, username, or absolute private path is ever recorded. |
| `runs[].{run_id,dpi,method}` | `dpi` is metadata only at the raster level (no active rendering factor — see `benchmark-metrics.md`). |
| `runs[].pages[]` | See below. |
| `runs[].aggregate` | `AggregateMetrics` — see below. |
| `runs[].category_aggregates` | The same `AggregateMetrics` shape, keyed by each page's `category`. |
| `runs[].roi_tag_aggregates` | The same shape again, keyed by ROI `tag` across every page (omitted from JSON when there are no ROIs). |
| `runs[].worst_pages` | `{lowest_f1, highest_drd, largest_compressed_page, slowest_page}`, each `{page_id, value}` or absent. Metric-specific, not a scholarly judgement — see `benchmark-metrics.md`. |

Per page (`runs[].pages[]`):

| Field | Meaning |
|---|---|
| `page_id`, `category`, `tags` | From the dataset manifest. |
| `width`, `height`, `pixel_count` | Of the compared rasters (output and ground truth, which must match — see `benchmark-metrics.md`, "Alignment"). |
| `f_measure` | `{confusion: {true_positive,false_positive,false_negative,true_negative}, precision, recall, f1}`. |
| `psnr` | `{"psnr_db": <number>}` or `{"perfect_match": true}` — never `Infinity`. |
| `drd` | Always a finite, non-negative number. |
| `black_pixel_ratio_output`, `black_pixel_ratio_ground_truth` | For comparing how much ink the output produced versus how much ground truth actually has. |
| `ccitt_bytes`, `bytes_per_megapixel` | Raw CCITT payload for this page and its per-megapixel normalization — deliberately excludes PDF container overhead (see [`size-estimation.md`](size-estimation.md)). |
| `render_duration_us` | `null` at the raster level (no PDFium render stage — distinct from a genuine zero-cost measurement). |
| `processing_duration_us` | Sum of the per-stage timings `process`/`analyze` already measure. |
| `roi_results` | `[{roi_id, tag, f_measure, psnr, drd}]`, omitted from JSON when the page has no ROIs. |

`AggregateMetrics` (used identically for a run, a category, or an ROI
tag):

| Field | Meaning |
|---|---|
| `page_count` | Pages/ROIs this aggregate was computed over. |
| `macro_f1`, `micro_f1` | Mean of per-page F1 vs. one F1 from summed confusion counts — see `benchmark-metrics.md`, "Macro vs. micro F1." Never a single unqualified `f1`. |
| `mean_precision`, `mean_recall` | Macro-averaged. |
| `mean_psnr_db` | Mean over non-perfect-match pages only; absent if every page was a perfect match. |
| `perfect_psnr_page_count` | How many pages were excluded from `mean_psnr_db` that way. |
| `mean_drd`, `median_drd`, `max_drd` | Always finite. |
| `mean_bytes_per_megapixel` | Absent for ROI-tag aggregates (compression is a whole-page property, not a crop's). |
| `median_processing_duration_us`, `total_processing_duration_us` | Absent for ROI-tag aggregates. |

## Error envelope

Schema `museion-binarize-error`, version `1.0`:

```json
{
  "schema": "museion-binarize-error",
  "schema_version": "1.0",
  "error": {
    "code": "password_required",
    "message": "the PDF at book.pdf is password-protected ...",
    "context": { "input": "book.pdf" }
  }
}
```

`code` is a stable, machine-readable identifier — never an internal Rust
type name or FFI detail. The full mapping from every `CoreError` variant
to a `code` and an exit-code category lives in
`crates/museion-binarize-cli/src/errors.rs::classify` and is documented in
[`cli.md`](cli.md#exit-codes). `context` carries non-secret, caller-supplied
detail such as the input path; nothing in this envelope can carry a
password — see the test
`error_envelope_never_contains_a_password_even_with_context`.

## Privacy: path rendering

Every report that names a source path accepts `--path-mode`:

| Mode | Behaviour |
|---|---|
| `basename` (default) | Just the file name — a report intended to be reproducible or shared should not silently embed the local filesystem layout. |
| `relative` | Relative to the current working directory where possible. |
| `absolute` | Full canonicalized path. |
| `omit` | Field is `null`. |

## Sample output

```bash
$ museion-binarize analyze book.pdf --dpi 300 --method otsu --json --pretty
```

```json
{
  "schema": "museion-binarize-analysis",
  "schema_version": "1.0",
  "tool": { "name": "Museion Binarize", "version": "0.1.0" },
  "result": {
    "source_path": "book.pdf",
    "source_bytes": 1321,
    "page_count": 3,
    "analyzed_page_count": 3,
    "failed_page_count": 0,
    "total_visible_area_points2": 1002305.0,
    "dpi": 300,
    "method": "otsu",
    "total_duration_us": 842103,
    "page_duration": { "min_us": 210044, "max_us": 320511, "mean_us": 280701.0, "median_us": 311548.0 },
    "pdfium_library": "/usr/local/lib/libpdfium.dylib (explicit path)",
    "pages": [
      {
        "page_index": 0,
        "page_number": 1,
        "width_points": 595.0,
        "height_points": 842.0,
        "source_rotation_degrees": 0,
        "pixel_width": 2479,
        "pixel_height": 3508,
        "pixel_count": 8695532,
        "grayscale": { "min": 12, "max": 250, "mean": 231.4, "std_dev": 48.2, "pixel_count": 8695532 },
        "threshold": { "method": "otsu", "threshold": 128 },
        "ink": { "black_pixels": 1885123, "white_pixels": 6810409, "black_pixel_ratio": 0.2168 },
        "raw_raster_bytes_estimate": 26086596,
        "packed_bilevel_bytes": 1086850,
        "stage_durations": { "render_us": 120033, "grayscale_prep_us": 8811, "binarization_us": 4102, "cleanup_us": 0, "total_us": 132946 },
        "warnings": []
      }
    ]
  }
}
```

(Timing values are illustrative, not golden — see Determinism above.)

## No committed sample reports

`test-data/synthetic/reports/` is deliberately not populated with
committed sample JSON in this milestone: every field above that involves
timing varies run to run, and a "golden" report file would either need
those fields stripped (adding a second, undocumented report shape) or
would be silently stale the first time it was regenerated. The schemas
and worked example above serve the same documentation purpose without
that risk.
