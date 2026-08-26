//! CLI error presentation: the stable exit-code contract and the
//! JSON/human error envelope.
//!
//! # Exit codes
//!
//! | Code | Meaning                                    |
//! |------|---------------------------------------------|
//! | 0    | success                                      |
//! | 2    | command-line usage / invalid parameters      |
//! | 3    | input or filesystem error                    |
//! | 4    | PDFium loading or PDF open error             |
//! | 5    | rendering or image-processing error          |
//! | 6    | output/write/validation error                |
//! | 7    | cancellation                                 |
//!
//! Every [`mpdf_core::error::CoreError`] variant is mapped to
//! exactly one of these in [`classify`], so no ordinary user-facing error
//! can escape through Rust's uncontrolled panic exit code (101) — a panic
//! would mean a bug, not a documented failure mode.

use std::process::ExitCode;

use mpdf_core::error::CoreError;
use mpdf_core::report::ErrorEnvelope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    Success,
    UsageError,
    InputError,
    PdfiumError,
    ProcessingError,
    OutputError,
    Cancelled,
}

impl ExitReason {
    pub fn code(self) -> u8 {
        match self {
            ExitReason::Success => 0,
            ExitReason::UsageError => 2,
            ExitReason::InputError => 3,
            ExitReason::PdfiumError => 4,
            ExitReason::ProcessingError => 5,
            ExitReason::OutputError => 6,
            ExitReason::Cancelled => 7,
        }
    }

    pub fn exit_code(self) -> ExitCode {
        ExitCode::from(self.code())
    }
}

/// Maps a [`CoreError`] to a stable, machine-readable code string (used
/// in JSON error reports) and its exit-code category. The string never
/// exposes an internal Rust type name or FFI detail.
pub fn classify(error: &CoreError) -> (&'static str, ExitReason) {
    match error {
        CoreError::InvalidParameter(_) => ("invalid_parameter", ExitReason::UsageError),
        CoreError::Image(_) => ("image_error", ExitReason::ProcessingError),
        CoreError::Ccitt(_) => ("ccitt_error", ExitReason::ProcessingError),
        CoreError::Io { .. } => ("io_error", ExitReason::InputError),
        CoreError::PdfiumNotFound { .. } => ("pdfium_library_not_found", ExitReason::PdfiumError),
        CoreError::PdfiumLoadFailed { .. } => ("pdfium_load_failed", ExitReason::PdfiumError),
        CoreError::PdfOpenFailed { .. } => ("pdf_open_failed", ExitReason::InputError),
        CoreError::PasswordRequired { .. } => ("password_required", ExitReason::InputError),
        CoreError::InvalidDocument(_) => ("invalid_document", ExitReason::InputError),
        CoreError::InvalidPageGeometry(_) => ("invalid_page_geometry", ExitReason::InputError),
        CoreError::RenderFailed { .. } => ("render_failed", ExitReason::ProcessingError),
        CoreError::ResourceLimitExceeded(_) => {
            ("resource_limit_exceeded", ExitReason::ProcessingError)
        }
        CoreError::PdfConstructionFailed(_) => ("pdf_construction_failed", ExitReason::OutputError),
        CoreError::TemporaryFile(_) => ("temporary_file_error", ExitReason::OutputError),
        CoreError::DestinationConflict(_) => ("destination_conflict", ExitReason::OutputError),
        CoreError::OutputValidationFailed(_) => {
            ("output_validation_failed", ExitReason::OutputError)
        }
        CoreError::Cancelled => ("cancelled", ExitReason::Cancelled),
        CoreError::InvalidBenchmarkManifest(_) => {
            ("invalid_benchmark_manifest", ExitReason::UsageError)
        }
        CoreError::UnsupportedBenchmarkSchema(_) => {
            ("unsupported_benchmark_schema", ExitReason::UsageError)
        }
        CoreError::DatasetPathTraversal(_) => ("dataset_path_traversal", ExitReason::UsageError),
        CoreError::MissingGroundTruth(_) => ("missing_ground_truth", ExitReason::InputError),
        CoreError::InvalidGroundTruthPolarity(_) => {
            ("invalid_ground_truth_polarity", ExitReason::InputError)
        }
        CoreError::BenchmarkDimensionMismatch(_) => {
            ("benchmark_dimension_mismatch", ExitReason::InputError)
        }
        CoreError::InvalidRoi(_) => ("invalid_roi", ExitReason::UsageError),
        CoreError::InvalidProfile(_) => ("invalid_benchmark_profile", ExitReason::UsageError),
        CoreError::MetricComputationFailed(_) => {
            ("metric_computation_failed", ExitReason::ProcessingError)
        }
    }
}

/// A usage error that never touched [`CoreError`] (clap-independent
/// argument validation done in this crate, e.g. a bad `--dpi`). Always
/// [`ExitReason::UsageError`], with the fixed code `"invalid_argument"`.
pub const USAGE_ERROR_CODE: &str = "invalid_argument";

/// Builds the structured JSON error envelope for a [`CoreError`] failure,
/// with optional non-secret caller context (e.g. `("input", "book.pdf")`).
pub fn core_error_envelope(error: &CoreError, context: &[(&str, String)]) -> ErrorEnvelope {
    let (code, _) = classify(error);
    let mut envelope = ErrorEnvelope::new(code, error.to_string());
    for (key, value) in context {
        envelope = envelope.with_context(*key, value.clone());
    }
    envelope
}

/// Builds the structured JSON error envelope for a plain usage error.
pub fn usage_error_envelope(message: &str, context: &[(&str, String)]) -> ErrorEnvelope {
    let mut envelope = ErrorEnvelope::new(USAGE_ERROR_CODE, message.to_string());
    for (key, value) in context {
        envelope = envelope.with_context(*key, value.clone());
    }
    envelope
}

/// Human-readable rendering of a [`CoreError`], distinguishing
/// cancellation from a genuine failure so scripts parsing stderr text can
/// tell them apart even without `--json`.
pub fn describe_core_error(error: &CoreError) -> String {
    match error {
        CoreError::Cancelled => "processing was cancelled; no output was written".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_exit_reason_has_the_documented_code() {
        assert_eq!(ExitReason::Success.code(), 0);
        assert_eq!(ExitReason::UsageError.code(), 2);
        assert_eq!(ExitReason::InputError.code(), 3);
        assert_eq!(ExitReason::PdfiumError.code(), 4);
        assert_eq!(ExitReason::ProcessingError.code(), 5);
        assert_eq!(ExitReason::OutputError.code(), 6);
        assert_eq!(ExitReason::Cancelled.code(), 7);
    }

    #[test]
    fn cancellation_maps_to_the_cancelled_category() {
        let (code, reason) = classify(&CoreError::Cancelled);
        assert_eq!(code, "cancelled");
        assert_eq!(reason, ExitReason::Cancelled);
    }

    #[test]
    fn password_required_maps_to_input_error() {
        let (code, reason) = classify(&CoreError::PasswordRequired {
            path: "/x.pdf".into(),
        });
        assert_eq!(code, "password_required");
        assert_eq!(reason, ExitReason::InputError);
    }

    #[test]
    fn pdfium_not_found_maps_to_pdfium_error() {
        let (code, reason) = classify(&CoreError::PdfiumNotFound {
            attempted: vec!["a".into()],
        });
        assert_eq!(code, "pdfium_library_not_found");
        assert_eq!(reason, ExitReason::PdfiumError);
    }

    #[test]
    fn destination_conflict_maps_to_output_error() {
        let (code, reason) = classify(&CoreError::DestinationConflict("x".into()));
        assert_eq!(code, "destination_conflict");
        assert_eq!(reason, ExitReason::OutputError);
    }

    #[test]
    fn invalid_parameter_maps_to_usage_error() {
        let (code, reason) = classify(&CoreError::InvalidParameter("x".into()));
        assert_eq!(code, "invalid_parameter");
        assert_eq!(reason, ExitReason::UsageError);
    }

    #[test]
    fn error_envelope_never_contains_a_password_even_with_context() {
        let error = CoreError::PasswordRequired {
            path: "/secret.pdf".into(),
        };
        let envelope = core_error_envelope(&error, &[("input", "secret.pdf".to_string())]);
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(!json.to_lowercase().contains("hunter2"));
        assert!(!json.to_lowercase().contains("\"password\":"));
    }

    #[test]
    fn cancellation_is_described_distinctly_in_human_output() {
        assert!(describe_core_error(&CoreError::Cancelled).contains("cancelled"));
    }

    #[test]
    fn usage_error_envelope_uses_the_fixed_code() {
        let envelope = usage_error_envelope("bad --dpi", &[]);
        assert_eq!(envelope.error.code, USAGE_ERROR_CODE);
    }
}
