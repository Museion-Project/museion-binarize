use super::*;
use crate::error::{CoreError, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn safe_dir(root: &Path) -> Result<PathBuf> {
    let m = fs::symlink_metadata(root).map_err(|e| CoreError::io(root, e))?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err(CoreError::InvalidDocument(
            "MDP root must be a real directory".into(),
        ));
    }
    let d = root.join("bookmarks");
    if let Ok(m) = fs::symlink_metadata(&d) {
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err(CoreError::InvalidDocument(
                "bookmarks directory is unsafe".into(),
            ));
        }
    } else {
        fs::create_dir(&d).map_err(|e| CoreError::io(&d, e))?;
    }
    Ok(d)
}
fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T> {
    let m = fs::symlink_metadata(p).map_err(|e| CoreError::io(p, e))?;
    if !m.is_file() || m.file_type().is_symlink() || m.len() > 64 * 1024 * 1024 {
        return Err(CoreError::InvalidDocument(format!(
            "unsafe bookmark file: {}",
            p.display()
        )));
    }
    serde_json::from_slice(&fs::read(p).map_err(|e| CoreError::io(p, e))?)
        .map_err(|e| CoreError::InvalidDocument(e.to_string()))
}
fn write_atomic<T: serde::Serialize>(p: &Path, v: &T) -> Result<()> {
    let mut bytes =
        serde_json::to_vec_pretty(v).map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
    bytes.push(b'\n');
    let parent = p.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| CoreError::io(parent, e))?;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| CoreError::io(temporary.path(), e))?;
    temporary
        .persist(p)
        .map_err(|e| CoreError::io(p, e.error))?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| CoreError::io(parent, e))?;
    Ok(())
}
pub fn candidates_path(root: &Path) -> PathBuf {
    root.join("bookmarks/candidates.json")
}
pub fn reviews_path(root: &Path) -> PathBuf {
    root.join("bookmarks/reviews.json")
}
pub fn load_snapshot(root: &Path) -> Result<BookmarkSnapshot> {
    let package = crate::document_package::DocumentPackage::read_from(root)?;
    let s: BookmarkSnapshot = read_json(&candidates_path(root))?;
    s.validate()?;
    let derived = if s.derived_digest.is_some() {
        let ocr = crate::ocr::read_ocr_records(root)
            .map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
        let mut d = crate::derived::DerivedDocument::from_package(&package, Some(&ocr))?;
        let revisions = crate::derived::load_revisions(root)?;
        d.apply_revisions(&revisions)?;
        Some(d)
    } else {
        None
    };
    super::validate_against(&s, &package, derived.as_ref())?;
    Ok(s)
}
pub fn save_snapshot(root: &Path, s: &BookmarkSnapshot, overwrite: bool) -> Result<()> {
    s.validate()?;
    let d = safe_dir(root)?;
    let p = d.join("candidates.json");
    if !overwrite && fs::symlink_metadata(&p).is_ok() {
        return Err(CoreError::DestinationConflict(
            "bookmark candidates already exist".into(),
        ));
    }
    write_atomic(&p, s)
}
pub fn load_reviews(root: &Path, snapshot: &BookmarkSnapshot) -> Result<BookmarkReviews> {
    let p = reviews_path(root);
    match fs::symlink_metadata(&p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(BookmarkReviews::empty(snapshot.generation_digest.clone()))
        }
        Err(e) => Err(CoreError::io(&p, e)),
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => Err(CoreError::InvalidDocument(
            "unsafe bookmark review file".into(),
        )),
        Ok(_) => {
            let r: BookmarkReviews = read_json(&p)?;
            r.validate()?;
            if r.base_generation_digest != snapshot.generation_digest {
                return Err(CoreError::InvalidDocument(
                    "stale bookmark review generation".into(),
                ));
            }
            Ok(r)
        }
    }
}
pub fn save_reviews(root: &Path, r: &BookmarkReviews) -> Result<()> {
    r.validate()?;
    let d = safe_dir(root)?;
    write_atomic(&d.join("reviews.json"), r)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_save_is_append_file_replace_only() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("bookmarks")).unwrap();
        let r = BookmarkReviews::empty("a".repeat(64));
        save_reviews(d.path(), &r).unwrap();
        assert!(reviews_path(d.path()).exists());
    }
}
