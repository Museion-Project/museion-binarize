# Benchmark results: `synthetic-document-v1` (baseline profile)

**These are results on a synthetic, procedurally generated fixture
suite, not on a representative corpus of real scanned or printed
documents.** They demonstrate that the benchmark framework works end
to end and are useful for regression detection and understanding how
Otsu/Sauvola/manual thresholding differ on the specific stress cases
this suite covers. They are **not** evidence for a broad claim about
preservation quality on real scholarly material, and in particular are
**not** evidence about polytonic Ancient Greek preservation — see
`docs/limitations.md`.

## How to reproduce

```bash
cargo build -p mpdf-cli --release
./target/release/mpdf benchmark run \
  --dataset test-data/benchmark/synthetic-v1/dataset.toml \
  --profile test-data/benchmark/profiles/baseline.toml \
  --report /tmp/mpdf-benchmark.json
```

Raster-level benchmarking never touches PDFium, so this reproduces on
any machine with the workspace built — no PDFium provisioning needed.
Timing figures are release-build, single-machine numbers (see
"Environment" below); content-fidelity fields (F1, PSNR, DRD,
confusion counts, compressed bytes) are deterministic and should match
exactly on any machine, since they depend only on the pipeline's
arithmetic, not on hardware.

## Environment

| | |
|---|---|
| Tool version | 0.1.0 |
| Commit | (see the commit this file is part of) |
| OS / arch | macOS 27.0 (Darwin) / aarch64 |
| Rust | 1.97.1 |
| Build | `--release` |
| Benchmark level | raster (no PDFium) |
| Dataset | `synthetic-document-v1`, digest `e59cf7da5c030d678936d4c9d3ace905da46eb08d8212275c9c27b18e102c904` |
| Profile | `baseline-method-comparison`, digest `dbd019a92736247e99c558cb55b58a07d31aad52ce085a6e488c88f855cc1598` |

Timing numbers below are from this one machine and are not a
cross-platform performance claim — see `docs/benchmark-metrics.md`,
"Performance measurement policy."

## Method comparison

| Run | Macro F1 | Micro F1 | Mean precision | Mean recall | Mean DRD | Median DRD | Max DRD | Mean bytes/MP | Total time (µs, release) |
|---|---|---|---|---|---|---|---|---|---|
| `otsu-300` | 0.9949 | 0.9946 | 0.9967 | 0.9933 | 0.1385 | 0.0 | 1.1038 | 30125.0 | 2220 |
| `sauvola-300-default` | 0.9934 | 0.9927 | 0.9884 | 0.9988 | 0.2260 | 0.0 | 1.6079 | 30080.7 | 2627 |
| `manual-300-128` | 0.9115 | 0.9479 | 0.9133 | 0.9099 | 1.5559 | 0.0 | 17.0096 | 28617.2 | 1261 |

PSNR: 10 of 12 pages were an exact perfect match under Otsu and
Sauvola (`mean_psnr_db` computed over the 2 non-perfect pages only —
see `docs/benchmark-metrics.md` for why perfect-match pages are
excluded from the mean rather than contributing `Infinity`); 9 of 12
under manual thresholding.

## Worst page per run (metric-specific, not a scholarly judgement)

| Run | Lowest F1 page | Highest DRD page |
|---|---|---|
| `otsu-300` | `blur` (0.9655) | `salt_pepper_noise` (1.1038) |
| `sauvola-300-default` | `blur` (0.9478) | `blur` (1.6079) |
| `manual-300-128` | `faint_text` (0.0000) | `faint_text` (17.0096) |

## `diacritic` and `apparatus` ROI aggregates

Every run scored **macro F1 = 1.0, mean DRD = 0.0** on both the
`diacritic` ROI tag (63 regions, from `synthetic_diacritic_detail`) and
the `apparatus` ROI tag (9 regions, from `dense_apparatus_density`).

**This is a real observation about this specific fixture, not a
preservation claim.** Those two categories currently apply no
grayscale degradation (no blur, noise, faintness, or contrast
reduction) — they stress *shape complexity* (small marks, dense
packing) at full contrast, not degraded-content recovery. All three
methods trivially separate high-contrast ink from paper regardless of
mark size, so the ROI aggregate does not yet distinguish them. The
methods *do* diverge sharply elsewhere — most dramatically on
`faint_text`, where `manual-300-128`'s fixed threshold classifies every
foreground pixel as background (F1 = 0.0) because the faded ink (~166)
falls above the fixed threshold (128), while Otsu and Sauvola adapt and
recover it correctly. A natural follow-up (not done in this milestone,
to avoid retuning fixtures mid-benchmark and presenting only the
post-tuning picture) is a *combined* diacritic-plus-degradation
category, so the ROI aggregate can measure what it is ultimately meant
to measure: whether small marks survive specifically when the rest of
the page is already under stress.

## Interpretation

- **No algorithm was changed to chase these scores.** This document
  observes the existing Otsu/Sauvola/manual implementations as they
  already are.
- **`manual-300-128`'s catastrophic failure on `faint_text` is a real,
  physically-explained result** (the fixed threshold sits above the
  faded ink's grayscale value), not a benchmark or pipeline defect — it
  is exactly the kind of quality difference this framework exists to
  surface, distinct from a correctness bug (see
  `docs/benchmark-metrics.md`, "Quality limitation vs. correctness
  bug").
- Bytes/MP is lowest for `manual-300-128`, which is expected: a
  fixed, high threshold with no adaptive lightening produces less ink
  overall on faded content, and less ink compresses smaller — this is
  a compression-size effect, not a quality signal, and should not be
  read as "more efficient."
