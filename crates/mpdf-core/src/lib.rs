//! Processing core for M PDF Processor.
//!
//! This crate must never depend on Tauri or any GUI toolkit; see
//! `docs/architecture.md` in the workspace root for the rationale.
//!
//! As of Milestone 3, this crate implements the full local PDF pipeline
//! (PDFium rasterization via a persistent, single-open-per-operation
//! document session; grayscale, binarization, preprocessing, cleanup,
//! bilevel packing; CCITT Group 4 encoding; deterministic 1-bit PDF
//! reconstruction and validation) plus document/page analysis and
//! versioned JSON reporting. See `docs/pdf-pipeline-session.md` for the
//! document session and source-mutation policy, and `docs/reporting.md`
//! for the report schemas.

pub mod analysis;
pub mod benchmark;
pub mod benchmark_fixtures;
pub mod bilevel;
pub mod binarization;
pub mod bookmark_fixtures;
pub mod bookmarks;
pub mod ccitt;
pub mod cleanup;
pub mod derived;
pub mod document;
pub mod document_package;
pub mod document_session;
pub mod error;
pub mod estimation;
pub mod grayscale;
pub mod image_pipeline;
pub mod jobs;
pub mod ocr;
pub mod page_geometry;
pub mod page_selection;
pub mod pdf_writer;
pub mod pdfium_backend;
pub mod pipeline;
pub mod preprocessing;
pub mod progress;
pub mod remote_api;
pub mod report;
pub mod searchable_output;
pub mod searchable_pdf;
pub mod settings;
pub mod source_identity;
pub mod test_fixtures;
pub mod timing;
pub mod validation;
/// Short technical alias used by MDP consumers.
pub use document_package as mdp;

/// The human-readable project name.
pub const PROJECT_NAME: &str = "M PDF Processor";

/// Static information about the project, exposed to front ends (CLI,
/// desktop app) so they can display consistent identity without hardcoding
/// it themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInfo {
    pub name: &'static str,
    pub phase: &'static str,
}

impl ProjectInfo {
    /// Returns the current [`ProjectInfo`] for this build of the core.
    pub fn current() -> Self {
        Self {
            name: PROJECT_NAME,
            phase: "Phase 1 — under development",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_project_info_reports_expected_name_and_phase() {
        let info = ProjectInfo::current();
        assert_eq!(info.name, "M PDF Processor");
        assert_eq!(info.phase, "Phase 1 — under development");
    }
}
