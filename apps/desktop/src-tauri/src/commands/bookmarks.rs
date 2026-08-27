use mpdf_core::bookmarks::{self, BookmarkCandidate, BookmarkSnapshot, ReviewAction};
use std::path::{Path, PathBuf};

fn path(value: String) -> Result<PathBuf, String> {
    if value.is_empty() || value.len() > 4096 {
        return Err("package path is out of range".into());
    }
    Ok(PathBuf::from(value))
}
fn snapshot(p: &Path) -> Result<BookmarkSnapshot, String> {
    bookmarks::load_snapshot(p).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn load_bookmarks(package_path: String) -> Result<Vec<BookmarkCandidate>, String> {
    let p = path(package_path)?;
    let s = snapshot(&p)?;
    let r = bookmarks::load_reviews(&p, &s).map_err(|e| e.to_string())?;
    bookmarks::effective(&s, &r).map_err(|e| e.to_string())
}
fn mutate(package_path: String, candidate_id: String, a: ReviewAction) -> Result<(), String> {
    let p = path(package_path)?;
    let s = snapshot(&p)?;
    let mut r = bookmarks::load_reviews(&p, &s).map_err(|e| e.to_string())?;
    bookmarks::append(&s, &mut r, candidate_id, a).map_err(|e| e.to_string())?;
    bookmarks::save_reviews(&p, &r).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn confirm_bookmark(package_path: String, candidate_id: String) -> Result<(), String> {
    mutate(package_path, candidate_id, ReviewAction::Confirm)
}
#[tauri::command]
pub fn reject_bookmark(package_path: String, candidate_id: String) -> Result<(), String> {
    mutate(package_path, candidate_id, ReviewAction::Reject)
}
#[tauri::command]
pub fn edit_bookmark(
    package_path: String,
    candidate_id: String,
    title: String,
) -> Result<(), String> {
    mutate(package_path, candidate_id, ReviewAction::Edit { title })
}
#[tauri::command]
pub fn reparent_bookmark(
    package_path: String,
    candidate_id: String,
    parent_id: Option<String>,
    level: u16,
) -> Result<(), String> {
    mutate(
        package_path,
        candidate_id,
        ReviewAction::Reparent { parent_id, level },
    )
}
