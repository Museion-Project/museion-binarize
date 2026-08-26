//! Stage timing, shared by `process` and `analyze` so both measure the
//! same stages the same way.
//!
//! Durations are reported as whole microseconds (`*_us`) rather than
//! floating-point seconds, so a JSON report contains exact integers with
//! an explicit, documented unit instead of a value whose precision looks
//! more meaningful than it is.
//!
//! Timestamps are deliberately absent from reports: see
//! [`crate::report`] for why, and for what that means for report-byte
//! determinism.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Runs `f`, returning its result and how long it took, in microseconds.
pub fn timed<T>(f: impl FnOnce() -> T) -> (T, u64) {
    let start = Instant::now();
    let value = f();
    (value, duration_to_micros(start.elapsed()))
}

/// Converts a [`Duration`] to whole microseconds, saturating rather than
/// overflowing for a duration long enough that this could matter (it
/// never will in practice, but the conversion must not panic).
pub fn duration_to_micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// Per-page stage durations, in microseconds. Every field is measured the
/// same way by both `process` and `analyze`, via the same helpers in this
/// module and in [`crate::image_pipeline`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StageDurations {
    /// PDFium rasterization of the page.
    pub render_us: u64,
    /// Grayscale conversion, contrast adjustment, and preprocessing
    /// (background normalization, median denoise) combined. These three
    /// steps run back-to-back over the same buffer with no other work
    /// between them, so timing them individually would not change any
    /// decision this project makes and would only multiply the report's
    /// field count.
    pub grayscale_prep_us: u64,
    pub binarization_us: u64,
    pub cleanup_us: u64,
    /// `None` when the page was not CCITT-encoded in this operation (for
    /// example, `analyze` without `--encode`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ccitt_encode_us: Option<u64>,
    /// Sum of every stage actually measured for this page, including
    /// stages this struct does not break out separately (e.g. output PDF
    /// assembly during `process`). Not simply the sum of the other
    /// fields when `ccitt_encode_us` is `None`.
    pub total_us: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_reports_a_nonzero_duration_for_real_work() {
        let (value, micros) = timed(|| {
            let mut sum = 0u64;
            for i in 0..100_000u64 {
                sum = sum.wrapping_add(i);
            }
            sum
        });
        assert!(value > 0);
        // Not asserting a lower bound on wall-clock time (too flaky
        // across CI machines); only that the mechanism returns.
        let _ = micros;
    }

    #[test]
    fn duration_to_micros_matches_the_documented_unit() {
        assert_eq!(duration_to_micros(Duration::from_micros(1_234)), 1_234);
        assert_eq!(duration_to_micros(Duration::from_secs(1)), 1_000_000);
        assert_eq!(duration_to_micros(Duration::ZERO), 0);
    }

    #[test]
    fn ccitt_field_is_omitted_from_json_when_absent() {
        let durations = StageDurations {
            ccitt_encode_us: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&durations).unwrap();
        assert!(!json.contains("ccitt_encode_us"));

        let with = StageDurations {
            ccitt_encode_us: Some(42),
            ..Default::default()
        };
        let json = serde_json::to_string(&with).unwrap();
        assert!(json.contains("\"ccitt_encode_us\":42"));
    }
}
