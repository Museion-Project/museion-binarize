//! Local review-workbench commands. They read only a caller-selected MDP
//! directory and persist append-only revision records; no network or model
//! process is involved.

use std::fs;
use std::path::{Path, PathBuf};

use mpdf_core::derived::{self, DerivedDocument, RevisionKind, RevisionRecord};
use mpdf_core::document_package::DocumentPackage;

fn checked_package_path(value: String) -> Result<PathBuf, String> {
    if value.is_empty() || value.len() > 4096 {
        return Err("package path is out of range".into());
    }
    Ok(PathBuf::from(value))
}

fn load(path: &Path) -> Result<DerivedDocument, String> {
    let package = DocumentPackage::read_from(path).map_err(|error| error.to_string())?;
    let ocr_dir = path.join("ocr");
    let ocr = match fs::symlink_metadata(&ocr_dir) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            Some(mpdf_core::ocr::read_ocr_records(path).map_err(|error| error.to_string())?)
        }
        Ok(_) => return Err("OCR directory is unsafe".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    let mut document =
        DerivedDocument::from_package(&package, ocr.as_ref()).map_err(|error| error.to_string())?;
    let revisions = derived::load_revisions(path).map_err(|error| error.to_string())?;
    document
        .apply_revisions(&revisions)
        .map_err(|error| error.to_string())?;
    Ok(document)
}

#[tauri::command]
pub fn load_review_queue(package_path: String) -> Result<Vec<derived::ReviewIssue>, String> {
    let path = checked_package_path(package_path)?;
    let document = load(&path)?;
    derived::review_queue(&document).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn add_review_revision(
    package_path: String,
    revision_id: Option<String>,
    target_ref: String,
    base_evidence_digest: String,
    text: String,
    ai_suggested: bool,
) -> Result<(), String> {
    if revision_id.as_deref().unwrap_or_default().len() > 256
        || target_ref.is_empty()
        || text.len() > derived::MAX_REVISION_TEXT_BYTES
    {
        return Err("revision fields are out of range".into());
    }
    let path = checked_package_path(package_path)?;
    let mut document = load(&path)?;
    let mut store = derived::load_revisions(&path).map_err(|error| error.to_string())?;
    let kind = if ai_suggested {
        RevisionKind::AiSuggested
    } else {
        RevisionKind::Human
    };
    let revision_id = if revision_id.as_deref().unwrap_or_default().is_empty() {
        derived::deterministic_revision_id(&target_ref, &base_evidence_digest, kind, &text)
    } else {
        revision_id.unwrap()
    };
    store.revisions.push(RevisionRecord {
        revision_id,
        target_ref,
        base_evidence_digest,
        text,
        kind,
    });
    document
        .apply_revisions(&store)
        .map_err(|error| error.to_string())?;
    derived::save_revisions(&path, &store).map_err(|error| error.to_string())
}
