//! Experimental output-size estimation.
//!
//! Renders, binarizes, and CCITT-encodes a small, deterministic sample of
//! pages through the exact same code `process` uses (see
//! [`crate::pipeline::estimate_output_size`]), measures each sampled
//! page's compressed bytes per pixel, and extrapolates to the whole
//! document using the sample's page geometry — never a separate
//! approximate algorithm, and never a purely file-size-based heuristic.
//!
//! This module owns only the parts of that process that do not need
//! PDFium: deterministic sample-page selection, the quartile/central-
//! tendency math, the report types, and the estimate-vs-actual
//! comparison used for calibration. See `docs/size-estimation.md` for the
//! full methodology and its documented limitations.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::settings::ProcessingSettings;

pub const SIZE_ESTIMATE_REPORT_SCHEMA: &str = "museion-binarize-size-estimate";
pub const SIZE_ESTIMATE_REPORT_SCHEMA_VERSION: &str = "1.0";

pub const DEFAULT_SAMPLE_COUNT: u32 = 8;
pub const MIN_SAMPLE_COUNT: u32 = 1;
pub const MAX_SAMPLE_COUNT: u32 = 32;

/// Validates a requested sample count and clamps it to the document's
/// actual page count. A request below [`MIN_SAMPLE_COUNT`] or above
/// [`MAX_SAMPLE_COUNT`] is a structured error — `--samples 1000000` must
/// not be silently accepted — but a request larger than the document
/// itself is not an error: it just means "sample every page," clamped
/// quietly, since that is a reasonable thing to ask for on a short
/// document.
pub fn resolve_sample_count(requested: u32, page_count: u32) -> Result<u32> {
    if requested < MIN_SAMPLE_COUNT {
        return Err(CoreError::InvalidParameter(format!(
            "sample count must be at least {MIN_SAMPLE_COUNT}, got {requested}"
        )));
    }
    if requested > MAX_SAMPLE_COUNT {
        return Err(CoreError::InvalidParameter(format!(
            "sample count must be at most {MAX_SAMPLE_COUNT}, got {requested}"
        )));
    }
    Ok(requested.min(page_count.max(1)))
}

/// Deterministically selects up to `sample_count` zero-based page indices,
/// evenly spaced across `[0, page_count - 1]`, rounded to the nearest
/// index. The first and last page are always included whenever
/// `sample_count >= 2` and `page_count >= 2` — sampling only from the
/// front of a book would systematically miss the fact that front matter
/// (title pages, tables of contents) often looks nothing like the body.
///
/// Deterministic for a given `(page_count, sample_count)` pair: no
/// randomness, so the same document and sample count always select the
/// same pages, and a test can assert on exact indices.
///
/// The returned vector is sorted, deduplicated, and therefore may contain
/// **fewer** than `sample_count` entries when evenly-spaced positions
/// happen to coincide (only possible when `sample_count` is close to
/// `page_count`) — this is expected, not an error.
pub fn select_sample_pages(page_count: u32, sample_count: u32) -> Vec<u32> {
    if page_count == 0 {
        return Vec::new();
    }
    let sample_count = sample_count.clamp(1, page_count);
    if sample_count >= page_count {
        return (0..page_count).collect();
    }
    if sample_count == 1 {
        return vec![0];
    }

    let last = u64::from(page_count - 1);
    let denom = u64::from(sample_count - 1);
    let mut indices: Vec<u32> = (0..sample_count)
        .map(|i| {
            let numerator = u64::from(i) * last;
            // Rounds to the nearest index rather than truncating, so the
            // spacing is as even as integer arithmetic allows.
            ((numerator + denom / 2) / denom) as u32
        })
        .collect();
    indices.sort_unstable();
    indices.dedup();
    indices
}

/// The 25th/50th/75th percentile of a set of values, via linear
/// interpolation between the two closest ranks (the same convention as
/// NumPy's default `'linear'` method and R's type-7 quantile). Documented
/// here once and used everywhere this crate needs a percentile, rather
/// than re-deriving the interpolation rule at each call site.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quartiles {
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
}

/// Computes [`Quartiles`] over `values`. Returns `None` for an empty
/// slice. Does not require `values` to be pre-sorted.
pub fn quartiles(values: &[f64]) -> Option<Quartiles> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(Quartiles {
        p25: percentile_of_sorted(&sorted, 0.25),
        p50: percentile_of_sorted(&sorted, 0.50),
        p75: percentile_of_sorted(&sorted, 0.75),
    })
}

fn percentile_of_sorted(sorted: &[f64], q: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = q * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let fraction = rank - lower as f64;
        sorted[lower] * (1.0 - fraction) + sorted[upper] * fraction
    }
}

/// Which values were used for the estimate's lower/upper bound. Named so
/// a report reader never mistakes this for a statistically-justified
/// confidence interval — it is neither of those things, just the
/// documented observed-sample spread. See `docs/size-estimation.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimationRangeMethod {
    /// Lower/upper bound are the sample's 25th/75th percentile
    /// bytes-per-pixel. Used when there are enough samples (4 or more)
    /// for a quartile to mean anything.
    Quartiles,
    /// Lower/upper bound are the sample's minimum/maximum bytes-per-pixel.
    /// Used for fewer than 4 samples, where a quartile would be
    /// determined by only one or two data points and so would not add
    /// information beyond the extremes themselves.
    MinMax,
}

/// The sample-count threshold below which [`EstimationRangeMethod::MinMax`]
/// is used instead of quartiles.
pub const QUARTILE_MIN_SAMPLES: usize = 4;

/// Measurements for one page rendered, processed, and CCITT-encoded
/// purely to build a size estimate — never written to any output PDF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageSizeEstimateSample {
    /// One-based.
    pub page_number: u32,
    /// Zero-based.
    pub page_index: u32,
    pub width_points: f32,
    pub height_points: f32,
    pub raster_width: u32,
    pub raster_height: u32,
    pub pixel_count: u64,
    pub black_pixel_ratio: f64,
    pub packed_bytes: u64,
    pub ccitt_bytes: u64,
    /// `ccitt_bytes as f64 / pixel_count as f64` — the normalized quantity
    /// this whole estimate is built from, since it generalizes across
    /// pages of different physical size far better than raw bytes/page.
    pub bytes_per_pixel: f64,
    pub processing_duration_us: u64,
}

/// The complete result of an experimental size estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SizeEstimateReport {
    pub document_page_count: u32,
    pub sampled_pages: Vec<PageSizeEstimateSample>,
    /// Sum of `sampled_pages[].ccitt_bytes` — the real, measured total
    /// for the sample itself (not an extrapolation).
    pub sampled_total_encoded_bytes: u64,
    pub min_bytes_per_pixel: f64,
    pub mean_bytes_per_pixel: f64,
    pub median_bytes_per_pixel: f64,
    pub max_bytes_per_pixel: f64,
    pub range_method: EstimationRangeMethod,
    /// Central estimate: sum over **every** document page (not just the
    /// sampled ones) of that page's pixel count at the selected DPI,
    /// multiplied by `median_bytes_per_pixel`.
    pub estimated_output_bytes: u64,
    /// Same extrapolation using the range method's lower bound
    /// (25th percentile or minimum bytes-per-pixel).
    pub estimated_lower_bytes: u64,
    /// Same extrapolation using the range method's upper bound
    /// (75th percentile or maximum bytes-per-pixel).
    pub estimated_upper_bytes: u64,
    pub dpi: u16,
    /// `"otsu"`, `"sauvola"`, or `"manual"` — matches
    /// [`crate::pipeline::DocumentAnalysisReport::method`]'s convention.
    pub method: String,
    /// A stable fingerprint of every setting that affects output bytes,
    /// so a later caller can tell whether a stored estimate still matches
    /// the settings someone is about to convert with. See
    /// [`settings_fingerprint`]. Opaque — do not parse its contents.
    pub settings_fingerprint: String,
    pub estimate_total_duration_us: u64,
    pub mean_sample_duration_us: f64,
    pub median_sample_duration_us: f64,
    pub pdfium_library: String,
    /// Always `true`. Present so a consumer that only ever sees this
    /// struct's serialized form (not its Rust type name) still carries
    /// the "this is not authoritative" signal in the data itself.
    pub experimental: bool,
}

/// A stable, opaque fingerprint of every [`ProcessingSettings`] field that
/// affects output bytes. Two settings values that would produce the same
/// converted output produce the same fingerprint; this is used only to
/// decide whether a stored estimate or comparison still applies, never
/// displayed to a user and never parsed for meaning.
pub fn settings_fingerprint(settings: &ProcessingSettings) -> String {
    use crate::settings::BinarizationMethod;
    let method_part = match settings.method {
        BinarizationMethod::Otsu => "otsu".to_string(),
        BinarizationMethod::Manual { threshold } => format!("manual:{threshold}"),
        BinarizationMethod::Sauvola(p) => {
            format!(
                "sauvola:{}:{}:{}",
                p.window_size,
                p.k.to_bits(),
                p.dynamic_range.to_bits()
            )
        }
    };
    format!(
        "dpi={};method={};contrast={};denoise={};bgnorm={}:{};despeckle={:?}:{:?}",
        settings.dpi,
        method_part,
        settings.contrast.to_bits(),
        settings.preprocessing.median_denoise,
        settings.preprocessing.background_normalization,
        settings.preprocessing.background_radius,
        settings.cleanup.despeckle,
        settings.cleanup.max_component_pixels,
    )
}

/// Compares a prior estimate's central value against a real conversion's
/// actual output size. Callers are responsible for only calling this when
/// the estimate and the conversion share the same document identity and
/// [`settings_fingerprint`] — this function does not check that itself,
/// since it has no access to either identity, only the two byte counts.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EstimateComparison {
    pub estimated_output_bytes: u64,
    pub actual_output_bytes: u64,
    pub absolute_error_bytes: u64,
    /// `absolute_error_bytes / actual_output_bytes`. `0.0` when
    /// `actual_output_bytes` is `0` (nothing to divide by, and an empty
    /// output is not expected in practice since a document always has at
    /// least one page).
    pub relative_error_fraction: f64,
}

pub fn compare_estimate(estimated_bytes: u64, actual_bytes: u64) -> EstimateComparison {
    let absolute_error_bytes = estimated_bytes.abs_diff(actual_bytes);
    let relative_error_fraction = if actual_bytes == 0 {
        0.0
    } else {
        absolute_error_bytes as f64 / actual_bytes as f64
    };
    EstimateComparison {
        estimated_output_bytes: estimated_bytes,
        actual_output_bytes: actual_bytes,
        absolute_error_bytes,
        relative_error_fraction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- resolve_sample_count -----------------------------------------

    #[test]
    fn rejects_a_sample_count_below_the_minimum() {
        assert!(resolve_sample_count(0, 100).is_err());
    }

    #[test]
    fn rejects_a_sample_count_above_the_maximum() {
        assert!(resolve_sample_count(1_000_000, 100).is_err());
        assert!(resolve_sample_count(MAX_SAMPLE_COUNT + 1, 1_000).is_err());
    }

    #[test]
    fn accepts_the_maximum_sample_count() {
        assert_eq!(
            resolve_sample_count(MAX_SAMPLE_COUNT, 1_000).unwrap(),
            MAX_SAMPLE_COUNT
        );
    }

    #[test]
    fn clamps_a_sample_count_larger_than_the_document_to_the_page_count() {
        assert_eq!(resolve_sample_count(8, 3).unwrap(), 3);
    }

    #[test]
    fn keeps_a_sample_count_within_the_document() {
        assert_eq!(resolve_sample_count(8, 100).unwrap(), 8);
    }

    // --- select_sample_pages -------------------------------------------

    #[test]
    fn one_page_document_samples_only_page_zero() {
        assert_eq!(select_sample_pages(1, 8), vec![0]);
    }

    #[test]
    fn two_page_document_samples_both_pages() {
        assert_eq!(select_sample_pages(2, 8), vec![0, 1]);
    }

    #[test]
    fn five_page_document_with_default_samples_samples_every_page() {
        assert_eq!(select_sample_pages(5, 8), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn eight_page_document_with_eight_samples_samples_every_page() {
        assert_eq!(select_sample_pages(8, 8), (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn twenty_page_document_includes_first_and_last_and_is_sorted_unique() {
        let indices = select_sample_pages(20, 8);
        assert_eq!(indices.first(), Some(&0));
        assert_eq!(indices.last(), Some(&19));
        assert!(
            indices.windows(2).all(|w| w[0] < w[1]),
            "must be sorted and unique"
        );
        assert!(indices.len() <= 8);
    }

    #[test]
    fn hundred_page_document_selects_eight_evenly_spaced_pages() {
        let indices = select_sample_pages(100, 8);
        assert_eq!(indices, vec![0, 14, 28, 42, 57, 71, 85, 99]);
    }

    #[test]
    fn thousand_page_document_includes_first_and_last() {
        let indices = select_sample_pages(1_000, 8);
        assert_eq!(indices.first(), Some(&0));
        assert_eq!(indices.last(), Some(&999));
        assert_eq!(indices.len(), 8);
        assert!(indices.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn selection_is_deterministic_across_repeated_calls() {
        let a = select_sample_pages(347, 8);
        let b = select_sample_pages(347, 8);
        assert_eq!(a, b);
    }

    #[test]
    fn zero_page_document_selects_nothing() {
        assert_eq!(select_sample_pages(0, 8), Vec::<u32>::new());
    }

    #[test]
    fn requesting_more_samples_than_pages_still_covers_every_page_without_duplicates() {
        let indices = select_sample_pages(6, 20);
        assert_eq!(indices, (0..6).collect::<Vec<_>>());
    }

    // --- quartiles -------------------------------------------------------

    #[test]
    fn quartiles_of_empty_is_none() {
        assert!(quartiles(&[]).is_none());
    }

    #[test]
    fn quartiles_of_one_value_are_all_that_value() {
        let q = quartiles(&[5.0]).unwrap();
        assert_eq!(q.p25, 5.0);
        assert_eq!(q.p50, 5.0);
        assert_eq!(q.p75, 5.0);
    }

    #[test]
    fn quartiles_ignore_input_order() {
        let a = quartiles(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        let b = quartiles(&[5.0, 1.0, 4.0, 2.0, 3.0]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn quartiles_of_a_known_sequence_match_linear_interpolation() {
        // 1..=9: p50 (median) is the middle value 5; p25/p75 are the
        // linear-interpolation quartiles for this well-known sequence.
        let values: Vec<f64> = (1..=9).map(f64::from).collect();
        let q = quartiles(&values).unwrap();
        assert_eq!(q.p50, 5.0);
        assert_eq!(q.p25, 3.0);
        assert_eq!(q.p75, 7.0);
    }

    #[test]
    fn quartiles_are_finite_for_repeated_values() {
        let q = quartiles(&[2.0, 2.0, 2.0, 2.0]).unwrap();
        assert_eq!(q.p25, 2.0);
        assert_eq!(q.p50, 2.0);
        assert_eq!(q.p75, 2.0);
    }

    // --- settings_fingerprint --------------------------------------------

    #[test]
    fn settings_fingerprint_is_stable_for_identical_settings() {
        use crate::settings::{BinarizationMethod, PreprocessingSettings, ProcessingSettings};
        let settings = ProcessingSettings {
            dpi: 400,
            method: BinarizationMethod::Otsu,
            contrast: 0.0,
            preprocessing: PreprocessingSettings::default(),
            cleanup: Default::default(),
        };
        assert_eq!(
            settings_fingerprint(&settings),
            settings_fingerprint(&settings)
        );
    }

    #[test]
    fn settings_fingerprint_differs_for_different_dpi() {
        use crate::settings::{BinarizationMethod, PreprocessingSettings, ProcessingSettings};
        let mut a = ProcessingSettings {
            dpi: 300,
            method: BinarizationMethod::Otsu,
            contrast: 0.0,
            preprocessing: PreprocessingSettings::default(),
            cleanup: Default::default(),
        };
        let mut b = a;
        b.dpi = 400;
        assert_ne!(settings_fingerprint(&a), settings_fingerprint(&b));
        a.dpi = 300;
        assert_eq!(settings_fingerprint(&a), settings_fingerprint(&a));
    }

    #[test]
    fn settings_fingerprint_differs_for_different_sauvola_parameters() {
        use crate::binarization::SauvolaParams;
        use crate::settings::{BinarizationMethod, PreprocessingSettings, ProcessingSettings};
        let base = |k: f32| ProcessingSettings {
            dpi: 400,
            method: BinarizationMethod::Sauvola(SauvolaParams {
                window_size: 33,
                k,
                dynamic_range: 128.0,
            }),
            contrast: 0.0,
            preprocessing: PreprocessingSettings::default(),
            cleanup: Default::default(),
        };
        assert_ne!(
            settings_fingerprint(&base(0.2)),
            settings_fingerprint(&base(0.3))
        );
    }

    // --- compare_estimate --------------------------------------------------

    #[test]
    fn compare_estimate_computes_absolute_and_relative_error() {
        let comparison = compare_estimate(6_500_000, 7_000_000);
        assert_eq!(comparison.absolute_error_bytes, 500_000);
        assert!((comparison.relative_error_fraction - 500_000.0 / 7_000_000.0).abs() < 1e-9);
    }

    #[test]
    fn compare_estimate_handles_an_overestimate() {
        let comparison = compare_estimate(8_000_000, 7_000_000);
        assert_eq!(comparison.absolute_error_bytes, 1_000_000);
    }

    #[test]
    fn compare_estimate_never_divides_by_zero() {
        let comparison = compare_estimate(100, 0);
        assert_eq!(comparison.relative_error_fraction, 0.0);
        assert!(comparison.relative_error_fraction.is_finite());
    }

    #[test]
    fn compare_estimate_error_is_always_finite_and_non_negative() {
        let comparison = compare_estimate(1, u64::MAX);
        assert!(comparison.relative_error_fraction.is_finite());
        assert!(comparison.relative_error_fraction >= 0.0);
    }
}
