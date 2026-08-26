# Benchmarks

**Status: planned, not yet implemented.** This directory will hold the
reproducible benchmarking framework and its results, described in
[`docs/benchmarking.md`](../docs/benchmarking.md).

## What will live here

- Scripts/tooling to run `mpdf-cli` (or
  `mpdf-core` directly) against a fixed, documented dataset and
  parameter set.
- Structured, versioned output recording the metrics described in
  [`docs/benchmarking.md`](../docs/benchmarking.md): F-measure,
  pseudo-F-measure, PSNR, DRD, processing time, peak memory, and compressed
  bytes per megapixel.
- Reference hardware/OS details for any recorded timing results.

## What will not live here

- **No benchmark datasets without license review.** Scanned book pages,
  including pages from public-domain editions, must not be committed here
  without documented provenance and a license/permission check. See
  [`test-data/README.md`](../test-data/README.md) and
  [`CONTRIBUTING.md`](../CONTRIBUTING.md).
- **No performance or preservation claims without reproducible evidence.**
  Numbers in this directory (once they exist) must be reproducible by
  re-running the recorded command against the recorded dataset version.

This directory is currently a placeholder for Milestone 6 of
[`docs/roadmap.md`](../docs/roadmap.md).
