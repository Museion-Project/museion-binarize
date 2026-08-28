//! The single place that turns an MDP directory into engine input.
//!
//! The CLI and the desktop backend both call [`load_auto_bookmark_inputs`];
//! neither re-implements staleness or digest binding. Everything read here
//! is validated before the algorithm sees it, and nothing outside the
//! package directory is touched.

use std::fs;
use std::path::Path;

use crate::derived::{self, DerivedDocument};
use crate::document_package::DocumentPackage;
use crate::error::{CoreError, Result};
use crate::ocr::{self, OcrRun};

use super::config::AutoBookmarkConfig;
use super::engine::{generate_auto_with_cancel, AutoBookmarkInput, AutoBookmarkResult};

/// Validated, self-consistent inputs for one automatic bookmark run.
pub struct AutoBookmarkInputs {
    pub package: DocumentPackage,
    pub ocr: Option<OcrRun>,
    pub derived: Option<DerivedDocument>,
}

impl AutoBookmarkInputs {
    pub fn as_input(&self) -> AutoBookmarkInput<'_> {
        AutoBookmarkInput {
            package: &self.package,
            ocr: self.ocr.as_ref(),
            derived: self.derived.as_ref(),
        }
    }

    /// True when a native outline alone can carry the run (no OCR needed).
    pub fn has_existing_outline(&self) -> bool {
        self.package
            .pages
            .iter()
            .any(|page| !page.existing_outline_evidence.is_empty())
    }
}

/// Reads and validates the package, the complete OCR run when one exists,
/// and the derived document with human revisions applied.
///
/// A partial OCR run is rejected rather than padded with guesses; a package
/// with a native outline and no OCR at all is accepted, because the
/// existing-outline mode needs no text evidence.
pub fn load_auto_bookmark_inputs(root: &Path) -> Result<AutoBookmarkInputs> {
    let package = DocumentPackage::read_from(root)?;
    let ocr_directory = root.join("ocr");
    let ocr = match fs::symlink_metadata(&ocr_directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(CoreError::io(&ocr_directory, error)),
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err(CoreError::InvalidDocument("OCR directory is unsafe".into()))
        }
        Ok(_) => Some(
            ocr::read_ocr_records(root)
                .map_err(|error| CoreError::InvalidDocument(error.to_string()))?,
        ),
    };
    if let Some(run) = &ocr {
        if !run.is_complete(package.manifest.page_count) {
            return Err(CoreError::InvalidDocument(
                "OCR evidence is incomplete; automatic bookmarks need every page".into(),
            ));
        }
    }
    let derived = match &ocr {
        None => None,
        Some(run) => {
            let mut document = DerivedDocument::from_package(&package, Some(run))?;
            let revisions = derived::load_revisions(root)?;
            document.apply_revisions(&revisions)?;
            Some(document)
        }
    };
    Ok(AutoBookmarkInputs {
        package,
        ocr,
        derived,
    })
}

/// Convenience wrapper used by both front ends: load, then generate.
pub fn generate_auto_from_package(
    root: &Path,
    config: &AutoBookmarkConfig,
    cancelled: &dyn Fn() -> bool,
) -> Result<AutoBookmarkResult> {
    let inputs = load_auto_bookmark_inputs(root)?;
    generate_auto_with_cancel(&inputs.as_input(), config, cancelled)
}
