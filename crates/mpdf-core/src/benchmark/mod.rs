//! Reproducible, ground-truth binarization-fidelity benchmarking.
//!
//! **Not to be confused with `analysis.rs`.** `analyze`/`analysis.rs`
//! reports real measurements from the production pipeline (grayscale
//! statistics, ink ratio, timing, ...) but has no ground truth and
//! computes none of the fidelity metrics in this module. This module
//! exists specifically to answer: "given a pixel-accurate ground-truth
//! binary image, how close is M PDF Processor's output to it?" — a
//! different question that needs a different kind of input (a dataset
//! with ground truth, not just a document to convert).
//!
//! See `docs/benchmark-metrics.md` for metric definitions and their
//! sources, `docs/benchmark-datasets.md` for the dataset/profile manifest
//! formats, and `docs/benchmark-running.md` for how to run a benchmark.
//!
//! ## Module layout
//!
//! - [`metrics`] — pure, filesystem-free fidelity metrics (confusion
//!   matrix, precision/recall/F1, PSNR, DRD).
//! - [`mask_io`] — loading a ground-truth/input raster from disk and
//!   normalizing it to the project's binary polarity convention.
//! - [`roi`] — region-of-interest definition and validation.
//! - [`dataset`] — the versioned dataset manifest format, with dataset-
//!   root path containment.
//! - [`profile`] — the versioned benchmark-profile (run configuration)
//!   format.
//! - [`runner`] — orchestrates a benchmark run: process every page of
//!   every profile run through the real production pipeline, compare
//!   against ground truth, aggregate.
//! - [`report`] — the versioned benchmark report types.

pub mod dataset;
pub mod limits;
pub mod mask_io;
pub mod metrics;
pub mod profile;
pub mod report;
pub mod roi;
pub mod runner;
