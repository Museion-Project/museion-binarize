//! Versioned benchmark report types: `museion-binarize-benchmark`.
//!
//! Wrapped in the shared [`crate::report::ReportEnvelope`] like every
//! other report this crate produces. See `docs/benchmark-metrics.md` for
//! what each metric field means and `docs/reporting.md` for the general
//! report-schema compatibility policy this follows.

use serde::{Deserialize, Serialize};

use crate::benchmark::metrics::{ConfusionMatrix, Psnr};

pub const BENCHMARK_REPORT_SCHEMA: &str = "museion-binarize-benchmark";
pub const BENCHMARK_REPORT_SCHEMA_VERSION: &str = "1.0";

/// Which pipeline stage the benchmark exercised. Kept as an explicit,
/// serialized field so a report can never be misread as measuring the
/// other level — see `docs/benchmark-metrics.md`, "Benchmark levels."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkLevel {
    /// `degraded raster -> Museion image pipeline -> binary output`.
    /// The primary fidelity benchmark; does not involve PDFium.
    Raster,
    /// `PDF -> PDFium render -> Museion image pipeline -> binary output`.
    /// Measures the full application path; PDFium/platform-sensitive.
    Pdf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetInfo {
    pub id: String,
    pub title: String,
    pub manifest_digest: String,
    pub page_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub id: String,
    pub digest: String,
}

/// Reproducibility-relevant environment metadata. Deliberately excludes
/// hostname, username, home directory, and any other machine-identifying
/// field — see `docs/benchmark-running.md`, "What environment metadata
/// is and isn't recorded."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    pub os: String,
    pub arch: String,
    pub tool_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdfium_library: Option<String>,
    pub benchmark_level: BenchmarkLevel,
}

/// Precision/recall/F1 plus the confusion matrix, mirroring
/// [`crate::benchmark::metrics::FMeasure`] but as a report-facing type
/// (kept separate so the internal metric type can evolve without an
/// automatic, unreviewed report-schema change).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FMeasureReport {
    pub confusion: ConfusionMatrix,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

impl From<crate::benchmark::metrics::FMeasure> for FMeasureReport {
    fn from(f: crate::benchmark::metrics::FMeasure) -> Self {
        Self {
            confusion: f.confusion,
            precision: f.precision,
            recall: f.recall,
            f1: f.f1,
        }
    }
}

/// One region-of-interest's metrics on one page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoiResult {
    pub roi_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub f_measure: FMeasureReport,
    pub psnr: Psnr,
    pub drd: f64,
}

/// One page's full benchmark result within one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageBenchmarkResult {
    pub page_id: String,
    pub category: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub pixel_count: u64,
    pub f_measure: FMeasureReport,
    pub psnr: Psnr,
    pub drd: f64,
    pub black_pixel_ratio_output: f64,
    pub black_pixel_ratio_ground_truth: f64,
    pub ccitt_bytes: u64,
    pub bytes_per_megapixel: f64,
    /// `null` for the raster benchmark level, which has no PDFium render
    /// stage — distinct from a genuine zero-cost measurement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_duration_us: Option<u64>,
    pub processing_duration_us: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roi_results: Vec<RoiResult>,
}

/// Aggregate statistics over a set of pages (a whole run, one category,
/// or one ROI tag — the same shape every time, so callers do not need
/// three different aggregate types).
///
/// **Macro vs. micro F1** (see `docs/benchmark-metrics.md`): `macro_f1`
/// is the mean of each page's own F1; `micro_f1` sums confusion counts
/// across every page first, then computes one F1 from the total. They
/// can differ substantially when page sizes vary. Never report a bare
/// `f1` without specifying which.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateMetrics {
    pub page_count: usize,
    pub macro_f1: f64,
    pub micro_f1: f64,
    pub mean_precision: f64,
    pub mean_recall: f64,
    /// Mean PSNR in dB over pages that were *not* a perfect match (a
    /// perfect match has no finite dB value to average in — see
    /// [`Psnr`]). `None` if every page was a perfect match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_psnr_db: Option<f64>,
    pub perfect_psnr_page_count: usize,
    pub mean_drd: f64,
    pub median_drd: f64,
    pub max_drd: f64,
    /// `None` for ROI-tag aggregates, which have no independent
    /// compressed-byte figure of their own (compression happens over the
    /// whole page, not a crop of it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_bytes_per_megapixel: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_processing_duration_us: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_processing_duration_us: Option<u64>,
}

/// A named extreme page within a run (worst F1, highest DRD, ...) — see
/// `docs/benchmark-metrics.md`, "Worst-page reporting is metric-specific,
/// not a scholarly judgement."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageExtreme {
    pub page_id: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorstPages {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lowest_f1: Option<PageExtreme>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highest_drd: Option<PageExtreme>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub largest_compressed_page: Option<PageExtreme>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slowest_page: Option<PageExtreme>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunReport {
    pub run_id: String,
    pub dpi: u16,
    pub method: String,
    pub pages: Vec<PageBenchmarkResult>,
    pub aggregate: AggregateMetrics,
    /// Aggregate metrics per page `category`, keyed by category name.
    pub category_aggregates: std::collections::BTreeMap<String, AggregateMetrics>,
    /// Aggregate metrics per ROI `tag` (across every page that declared
    /// an ROI with that tag), keyed by tag name.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub roi_tag_aggregates: std::collections::BTreeMap<String, AggregateMetrics>,
    pub worst_pages: WorstPages,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub metrics_schema_version: String,
    pub dataset: DatasetInfo,
    pub profile: ProfileInfo,
    pub environment: Environment,
    pub runs: Vec<RunReport>,
}

pub const METRICS_SCHEMA_VERSION: &str = "1.0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_report_schema_constants_are_1_0() {
        assert_eq!(BENCHMARK_REPORT_SCHEMA, "museion-binarize-benchmark");
        assert_eq!(BENCHMARK_REPORT_SCHEMA_VERSION, "1.0");
        assert_eq!(METRICS_SCHEMA_VERSION, "1.0");
    }

    #[test]
    fn benchmark_level_serializes_as_a_distinct_tag() {
        let raster = serde_json::to_string(&BenchmarkLevel::Raster).unwrap();
        let pdf = serde_json::to_string(&BenchmarkLevel::Pdf).unwrap();
        assert_eq!(raster, "\"raster\"");
        assert_eq!(pdf, "\"pdf\"");
        assert_ne!(raster, pdf);
    }
}
