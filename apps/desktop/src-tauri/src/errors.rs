//! Maps [`CoreError`] and desktop-local failures to the stable
//! [`UiErrorDto`] the frontend switches on.
//!
//! Mirrors `mpdf-cli`'s `errors::classify` (same stable code
//! strings, independently maintained per crate since the CLI exposes no
//! library target to share one with) — see `docs/cli.md`'s exit-code
//! table for the sibling mapping.

use mpdf_core::error::CoreError;

use crate::dto::UiErrorDto;

/// Maps a [`CoreError`] to a stable, machine-readable code the frontend
/// can switch on, plus a short actionable hint where one exists. Never
/// includes a password or other secret — `CoreError`'s own `Display`
/// impl already guarantees that (see `mpdf_core::error`).
pub fn classify_core_error(error: &CoreError) -> UiErrorDto {
    let code = match error {
        CoreError::InvalidParameter(_) => "invalid_parameter",
        CoreError::Image(_) => "image_error",
        CoreError::Ccitt(_) => "ccitt_error",
        CoreError::Io { .. } => "io_error",
        CoreError::PdfiumNotFound { .. } => "pdfium_missing",
        CoreError::PdfiumLoadFailed { .. } => "pdfium_load_failed",
        CoreError::PdfOpenFailed { .. } => "pdf_open_failed",
        CoreError::PasswordRequired { .. } => "password_required",
        CoreError::InvalidDocument(_) => "invalid_document",
        CoreError::InvalidPageGeometry(_) => "invalid_page_geometry",
        CoreError::RenderFailed { .. } => "render_failed",
        CoreError::ResourceLimitExceeded(_) => "resource_limit_exceeded",
        CoreError::PdfConstructionFailed(_) => "pdf_construction_failed",
        CoreError::TemporaryFile(_) => "temporary_file_error",
        CoreError::DestinationConflict(_) => "destination_conflict",
        CoreError::OutputValidationFailed(_) => "output_validation_failed",
        CoreError::Cancelled => "cancelled",
        // The desktop app never constructs or surfaces these — the
        // benchmark framework (Milestone 6) is core/CLI-only — but the
        // match must stay exhaustive so a new CoreError variant can
        // never silently fall through unclassified.
        CoreError::InvalidBenchmarkManifest(_) => "invalid_benchmark_manifest",
        CoreError::UnsupportedBenchmarkSchema(_) => "unsupported_benchmark_schema",
        CoreError::DatasetPathTraversal(_) => "dataset_path_traversal",
        CoreError::MissingGroundTruth(_) => "missing_ground_truth",
        CoreError::InvalidGroundTruthPolarity(_) => "invalid_ground_truth_polarity",
        CoreError::BenchmarkDimensionMismatch(_) => "benchmark_dimension_mismatch",
        CoreError::InvalidRoi(_) => "invalid_roi",
        CoreError::InvalidProfile(_) => "invalid_benchmark_profile",
        CoreError::MetricComputationFailed(_) => "metric_computation_failed",
    };
    let hint = match error {
        CoreError::PdfiumNotFound { .. } | CoreError::PdfiumLoadFailed { .. } => Some(
            "For a development build, set MPDF_PDFIUM_LIBRARY to a local PDFium library \
             before launching the app. See docs/pdfium.md."
                .to_string(),
        ),
        CoreError::PasswordRequired { .. } => {
            Some("Enter the document password and try again.".to_string())
        }
        CoreError::DestinationConflict(_) => {
            Some("Choose a different output location.".to_string())
        }
        CoreError::PdfOpenFailed { .. } | CoreError::InvalidDocument(_) => {
            Some("Confirm the file is a valid, uncorrupted PDF.".to_string())
        }
        _ => None,
    };
    let detail = match error {
        CoreError::PdfiumNotFound { attempted } => Some(attempted.join("\n")),
        _ => None,
    };
    UiErrorDto {
        code: code.to_string(),
        message: error.to_string(),
        hint,
        detail,
    }
}

/// A structured error for a request this crate rejects before it ever
/// reaches core (a malformed DTO, a stale/unknown document id, a second
/// concurrent job, ...). Always code `"invalid_request"` with `message`
/// as the specific reason — narrower codes are added here as the
/// frontend needs to distinguish them.
pub fn request_error(code: &str, message: impl Into<String>) -> UiErrorDto {
    UiErrorDto {
        code: code.to_string(),
        message: message.into(),
        hint: None,
        detail: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn password_required_never_leaks_into_the_dto() {
        let error = CoreError::PasswordRequired {
            path: PathBuf::from("/secret.pdf"),
        };
        let dto = classify_core_error(&error);
        assert_eq!(dto.code, "password_required");
        assert!(!dto.message.to_lowercase().contains("hunter2"));
        let json = serde_json::to_string(&dto).unwrap();
        assert!(!json.to_lowercase().contains("\"password\""));
    }

    #[test]
    fn pdfium_missing_carries_attempted_paths_in_detail_not_message() {
        let error = CoreError::PdfiumNotFound {
            attempted: vec!["/a/libpdfium.dylib".to_string()],
        };
        let dto = classify_core_error(&error);
        assert_eq!(dto.code, "pdfium_missing");
        assert!(dto.detail.unwrap().contains("/a/libpdfium.dylib"));
        assert!(dto.hint.is_some());
    }

    #[test]
    fn cancelled_maps_to_a_stable_code() {
        assert_eq!(classify_core_error(&CoreError::Cancelled).code, "cancelled");
    }

    #[test]
    fn request_error_has_no_hint_or_detail() {
        let dto = request_error("document_not_open", "no document is currently open");
        assert_eq!(dto.code, "document_not_open");
        assert!(dto.hint.is_none());
        assert!(dto.detail.is_none());
    }
}
