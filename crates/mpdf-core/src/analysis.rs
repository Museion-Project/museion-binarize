//! Document- and page-level analysis reports produced by `analyze`.
//!
//! `analyze` runs the real rendering and image-processing pipeline but
//! never reconstructs an output PDF; see `docs/reporting.md`. It exists
//! for choosing settings, scripting, and identifying difficult pages — it
//! is **not** the Milestone 6 reproducible benchmarking framework, and a
//! low file size or black-pixel ratio here is not evidence of
//! preservation quality.

use serde::{Deserialize, Serialize};

use crate::image_pipeline::{GrayscaleStats, InkStats, ThresholdInfo};
use crate::timing::StageDurations;

pub const ANALYSIS_REPORT_SCHEMA: &str = "mpdf-analysis";
pub const ANALYSIS_REPORT_SCHEMA_VERSION: &str = "1.0";

/// Measurements for one analyzed page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageAnalysisReport {
    /// Zero-based page index.
    pub page_index: u32,
    /// One-based page number, matching what the user sees.
    pub page_number: u32,
    pub width_points: f32,
    pub height_points: f32,
    /// The source PDF's declared `/Rotate`, in degrees. Informational
    /// only — see `crate::document::PdfPageInfo::source_rotation`.
    pub source_rotation_degrees: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub pixel_count: u64,
    pub grayscale: GrayscaleStats,
    pub threshold: ThresholdInfo,
    pub ink: InkStats,
    /// Estimated size of the raw rendered RGB raster, in bytes
    /// (`width * height * 3`). An estimate, not a measurement of any
    /// buffer actually retained after this page finished processing.
    pub raw_raster_bytes_estimate: u64,
    /// Size of the packed (8 pixels/byte) bilevel raster, in bytes.
    pub packed_bilevel_bytes: u64,
    /// CCITT Group 4-encoded size, in bytes. `None` unless CCITT encoding
    /// was requested for this analysis (see `--encode`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ccitt_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ccitt_bytes_per_pixel: Option<f64>,
    pub stage_durations: StageDurations,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Aggregate statistics over one field (page duration, in this crate)
/// across every analyzed page.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DurationStats {
    pub min_us: u64,
    pub max_us: u64,
    pub mean_us: f64,
    /// Median of the sampled values. For an even sample count, the
    /// arithmetic mean of the two middle values (the conventional
    /// definition), not either middle value alone.
    pub median_us: f64,
}

/// The complete result of analyzing a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentAnalysisReport {
    /// The input path as rendered by the caller's chosen `PathMode`.
    /// `None` when the caller asked to omit it.
    pub source_path: Option<String>,
    pub source_bytes: u64,
    pub page_count: u32,
    pub analyzed_page_count: u32,
    pub failed_page_count: u32,
    /// Sum of every analyzed page's visible area, in square PDF points
    /// (72 points = 1 inch).
    pub total_visible_area_points2: f64,
    pub dpi: u16,
    /// The binarization method's name (`"otsu"`, `"sauvola"`, or
    /// `"manual"`) — the per-page `threshold` field carries the full
    /// configuration.
    pub method: String,
    pub total_duration_us: u64,
    /// `None` only when zero pages were analyzed.
    pub page_duration: Option<DurationStats>,
    pub pdfium_library: String,
    pub pages: Vec<PageAnalysisReport>,
}

/// Computes [`DurationStats`] over each analyzed page's `total_us`.
/// Returns `None` for an empty slice rather than dividing by zero.
pub fn aggregate_page_durations(pages: &[PageAnalysisReport]) -> Option<DurationStats> {
    if pages.is_empty() {
        return None;
    }
    let mut values: Vec<u64> = pages.iter().map(|p| p.stage_durations.total_us).collect();
    values.sort_unstable();

    let min_us = *values.first().expect("checked non-empty above");
    let max_us = *values.last().expect("checked non-empty above");
    let sum: u128 = values.iter().map(|&v| u128::from(v)).sum();
    let mean_us = sum as f64 / values.len() as f64;
    let median_us = median_of_sorted(&values);

    Some(DurationStats {
        min_us,
        max_us,
        mean_us,
        median_us,
    })
}

/// Median of an already-sorted, non-empty slice. For an even length, the
/// arithmetic mean of the two middle elements.
fn median_of_sorted(sorted: &[u64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2] as f64
    } else {
        (sorted[n / 2 - 1] as f64 + sorted[n / 2] as f64) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(total_us: u64) -> PageAnalysisReport {
        PageAnalysisReport {
            page_index: 0,
            page_number: 1,
            width_points: 595.0,
            height_points: 842.0,
            source_rotation_degrees: 0,
            pixel_width: 100,
            pixel_height: 100,
            pixel_count: 10_000,
            grayscale: GrayscaleStats {
                min: 0,
                max: 255,
                mean: 128.0,
                std_dev: 10.0,
                pixel_count: 10_000,
            },
            threshold: ThresholdInfo::Otsu { threshold: 128 },
            ink: InkStats {
                black_pixels: 100,
                white_pixels: 9_900,
                black_pixel_ratio: 0.01,
            },
            raw_raster_bytes_estimate: 30_000,
            packed_bilevel_bytes: 1_300,
            ccitt_bytes: None,
            ccitt_bytes_per_pixel: None,
            stage_durations: StageDurations {
                total_us,
                ..Default::default()
            },
            warnings: Vec::new(),
        }
    }

    #[test]
    fn aggregate_of_empty_pages_is_none() {
        assert!(aggregate_page_durations(&[]).is_none());
    }

    #[test]
    fn aggregate_computes_min_max_mean() {
        let pages = vec![page(10), page(30), page(20)];
        let stats = aggregate_page_durations(&pages).unwrap();
        assert_eq!(stats.min_us, 10);
        assert_eq!(stats.max_us, 30);
        assert_eq!(stats.mean_us, 20.0);
    }

    #[test]
    fn median_of_odd_count_is_the_middle_value() {
        let pages = vec![page(10), page(30), page(20)];
        let stats = aggregate_page_durations(&pages).unwrap();
        assert_eq!(stats.median_us, 20.0);
    }

    #[test]
    fn median_of_even_count_is_the_mean_of_the_two_middle_values() {
        let pages = vec![page(10), page(20), page(30), page(40)];
        let stats = aggregate_page_durations(&pages).unwrap();
        // Sorted: 10, 20, 30, 40 -> (20 + 30) / 2 = 25.
        assert_eq!(stats.median_us, 25.0);
    }

    #[test]
    fn median_ignores_input_order() {
        let ordered = aggregate_page_durations(&[page(5), page(1), page(9), page(3)]).unwrap();
        let reordered = aggregate_page_durations(&[page(9), page(3), page(1), page(5)]).unwrap();
        assert_eq!(ordered.median_us, reordered.median_us);
    }

    #[test]
    fn single_page_median_equals_its_own_value() {
        let stats = aggregate_page_durations(&[page(42)]).unwrap();
        assert_eq!(stats.median_us, 42.0);
        assert_eq!(stats.min_us, 42);
        assert_eq!(stats.max_us, 42);
        assert_eq!(stats.mean_us, 42.0);
    }

    #[test]
    fn report_serializes_to_stable_field_names() {
        let report = DocumentAnalysisReport {
            source_path: Some("book.pdf".to_string()),
            source_bytes: 1_000,
            page_count: 1,
            analyzed_page_count: 1,
            failed_page_count: 0,
            total_visible_area_points2: 595.0 * 842.0,
            dpi: 300,
            method: "otsu".to_string(),
            total_duration_us: 1_000,
            page_duration: aggregate_page_durations(&[page(1_000)]),
            pdfium_library: "test".to_string(),
            pages: vec![page(1_000)],
        };
        let json = serde_json::to_string(&report).unwrap();
        for key in [
            "source_path",
            "page_count",
            "analyzed_page_count",
            "total_duration_us",
            "pdfium_library",
        ] {
            assert!(json.contains(key), "missing field {key} in {json}");
        }
        // Round-trips.
        let back: DocumentAnalysisReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, back);
    }
}
