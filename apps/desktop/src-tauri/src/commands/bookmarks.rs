//! Human review of bookmark candidates.
//!
//! Loading the tree lives in `auto_bookmarks::load_bookmark_tree` (one
//! project-owned DTO for both the automatic and the manual path); this
//! module holds the append-only review mutations. No core type crosses the
//! IPC boundary, and every failure is a structured `UiErrorDto`.

use std::path::{Path, PathBuf};

use mpdf_core::bookmarks::{self, BookmarkSnapshot, ReviewAction};

use crate::dto::UiErrorDto;
use crate::errors::{classify_core_error, request_error};

fn path(value: String) -> Result<PathBuf, UiErrorDto> {
    if value.trim().is_empty() || value.len() > 4096 || value.bytes().any(|byte| byte == 0) {
        return Err(request_error(
            "invalid_parameter",
            "the package path is empty or out of range",
        ));
    }
    Ok(PathBuf::from(value))
}

fn snapshot(root: &Path) -> Result<BookmarkSnapshot, UiErrorDto> {
    bookmarks::load_snapshot(root).map_err(|error| classify_core_error(&error))
}

fn mutate(
    package_path: String,
    candidate_id: String,
    action: ReviewAction,
) -> Result<(), UiErrorDto> {
    if candidate_id.trim().is_empty() || candidate_id.len() > 256 {
        return Err(request_error(
            "invalid_parameter",
            "the candidate id is empty or out of range",
        ));
    }
    let root = path(package_path)?;
    let snapshot = snapshot(&root)?;
    let mut reviews =
        bookmarks::load_reviews(&root, &snapshot).map_err(|error| classify_core_error(&error))?;
    bookmarks::append(&snapshot, &mut reviews, candidate_id, action)
        .map_err(|error| classify_core_error(&error))?;
    bookmarks::save_reviews(&root, &reviews).map_err(|error| classify_core_error(&error))
}

#[tauri::command]
pub fn confirm_bookmark(package_path: String, candidate_id: String) -> Result<(), UiErrorDto> {
    mutate(package_path, candidate_id, ReviewAction::Confirm)
}

#[tauri::command]
pub fn reject_bookmark(package_path: String, candidate_id: String) -> Result<(), UiErrorDto> {
    mutate(package_path, candidate_id, ReviewAction::Reject)
}

#[tauri::command]
pub fn edit_bookmark(
    package_path: String,
    candidate_id: String,
    title: String,
) -> Result<(), UiErrorDto> {
    mutate(package_path, candidate_id, ReviewAction::Edit { title })
}

#[tauri::command]
pub fn reparent_bookmark(
    package_path: String,
    candidate_id: String,
    parent_id: Option<String>,
    level: u16,
) -> Result<(), UiErrorDto> {
    mutate(
        package_path,
        candidate_id,
        ReviewAction::Reparent { parent_id, level },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_review_target_must_be_a_bounded_path_and_id() {
        assert_eq!(
            mutate(String::new(), "bookmark-a".into(), ReviewAction::Confirm)
                .unwrap_err()
                .code,
            "invalid_parameter"
        );
        assert_eq!(
            mutate("/tmp/book.mdp".into(), String::new(), ReviewAction::Confirm)
                .unwrap_err()
                .code,
            "invalid_parameter"
        );
    }
}
