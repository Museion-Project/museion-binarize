use super::*;
use crate::error::{CoreError, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::Mutex;

#[cfg(test)]
static FAIL_REPORT_INSTALL_ONCE: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static INSTALL_HOOK_LOCK: Mutex<()> = Mutex::new(());

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

fn stage_json<T: serde::Serialize>(directory: &Path, value: &T) -> Result<PathBuf> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
    bytes.push(b'\n');
    stage_bytes(directory, &bytes)
}

fn stage_bytes(directory: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let mut temporary = tempfile::Builder::new()
        .prefix(".bookmark-generation-")
        .tempfile_in(directory)
        .map_err(|e| CoreError::io(directory, e))?;
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| CoreError::io(temporary.path(), e))?;
    let (_, path) = temporary
        .keep()
        .map_err(|e| CoreError::io(directory, e.error))?;
    Ok(path)
}

fn ensure_target_safe(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            CoreError::InvalidDocument(format!("unsafe bookmark target: {}", path.display())),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CoreError::io(path, error)),
    }
}

fn install_staged(staged: &Path, target: &Path) -> Result<()> {
    // Re-check immediately before touching the target: the earlier preflight
    // is not sufficient against a target replaced by a symlink in between.
    ensure_target_safe(target)?;
    #[cfg(test)]
    if target.file_name().and_then(|name| name.to_str()) == Some("generation-report.json")
        && FAIL_REPORT_INSTALL_ONCE.swap(false, Ordering::SeqCst)
    {
        return Err(CoreError::io(
            target,
            std::io::Error::other("injected report install failure"),
        ));
    }
    #[cfg(windows)]
    if fs::symlink_metadata(target).is_ok() {
        fs::remove_file(target).map_err(|error| CoreError::io(target, error))?;
    }
    fs::rename(staged, target).map_err(|error| CoreError::io(target, error))
}

/// A generation replacement that can be rolled back if a later output stage
/// fails.  The transaction owns the previous pair's bytes; this is necessary
/// because candidates and report are two independently named files and cannot
/// be replaced by one filesystem rename.
pub struct GenerationTransaction {
    root: PathBuf,
    old_snapshot: Option<Vec<u8>>,
    old_report: Option<Vec<u8>>,
    generation_digest: String,
    source_digest: String,
    package_digest: String,
    committed: bool,
}

impl GenerationTransaction {
    pub fn commit(mut self) -> Result<()> {
        let verification = verify_generation_pair(
            &self.root,
            &self.generation_digest,
            &self.source_digest,
            &self.package_digest,
        );
        if let Err(error) = verification {
            return match self.rollback() {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(CoreError::InvalidDocument(format!(
                    "committed bookmark pair verification failed: {error}; rollback failed: {rollback_error}"
                ))),
            };
        }
        self.committed = true;
        Ok(())
    }

    pub fn rollback(mut self) -> Result<()> {
        let result = restore_generation(
            &self.root,
            self.old_snapshot.as_deref(),
            self.old_report.as_deref(),
        );
        // Normal callers explicitly invoke rollback. Mark the transaction
        // handled even on failure so Drop is only a last-resort safety net,
        // never a second unreportable rollback attempt.
        self.committed = true;
        result
    }
}

impl Drop for GenerationTransaction {
    fn drop(&mut self) {
        if !self.committed {
            let _ = restore_generation(
                &self.root,
                self.old_snapshot.as_deref(),
                self.old_report.as_deref(),
            );
        }
    }
}

fn restore_generation(root: &Path, snapshot: Option<&[u8]>, report: Option<&[u8]>) -> Result<()> {
    let directory = safe_dir(root)?;
    let snapshot_path = candidates_path(root);
    let report_path = generation_report_path(root);
    let restore_one = |path: &Path, bytes: Option<&[u8]>| -> Result<()> {
        ensure_target_safe(path)?;
        match bytes {
            Some(bytes) => {
                let staged = stage_bytes(&directory, bytes)?;
                if let Err(error) = install_staged(&staged, path) {
                    let _ = fs::remove_file(&staged);
                    return Err(error);
                }
                Ok(())
            }
            None => match fs::symlink_metadata(path) {
                Ok(_) => fs::remove_file(path).map_err(|error| CoreError::io(path, error)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(CoreError::io(path, error)),
            },
        }
    };
    let snapshot_error = restore_one(&snapshot_path, snapshot);
    let report_error = restore_one(&report_path, report);
    if let (Err(snapshot_error), Err(report_error)) = (&snapshot_error, &report_error) {
        return Err(CoreError::InvalidDocument(format!(
            "generation rollback failed for candidates ({snapshot_error}) and report ({report_error})"
        )));
    }
    if let Err(error) = snapshot_error {
        return Err(CoreError::InvalidDocument(format!(
            "generation rollback failed for candidates: {error}"
        )));
    }
    if let Err(error) = report_error {
        return Err(CoreError::InvalidDocument(format!(
            "generation rollback failed for report: {error}"
        )));
    }
    #[cfg(unix)]
    fs::File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| CoreError::io(root, e))?;
    // A successful pair restore is part of the transaction contract, not an
    // assumption about rename. Read both names back and compare the exact
    // bytes that were captured before installation. This also makes a race
    // that swaps a target after the preflight fail closed.
    verify_restored_member(&snapshot_path, snapshot)?;
    verify_restored_member(&report_path, report)?;
    Ok(())
}

fn verify_restored_member(path: &Path, expected: Option<&[u8]>) -> Result<()> {
    ensure_target_safe(path)?;
    match (expected, fs::symlink_metadata(path)) {
        (Some(expected), Ok(metadata)) => {
            let actual = fs::read(path).map_err(|error| CoreError::io(path, error))?;
            if actual != expected {
                return Err(CoreError::InvalidDocument(format!(
                    "generation rollback verification failed for {}",
                    path.display()
                )));
            }
            if metadata.len() != expected.len() as u64 {
                return Err(CoreError::InvalidDocument(format!(
                    "generation rollback size verification failed for {}",
                    path.display()
                )));
            }
            Ok(())
        }
        (None, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        (None, Ok(_)) => Err(CoreError::InvalidDocument(format!(
            "generation rollback left unexpected file {}",
            path.display()
        ))),
        (_, Err(error)) => Err(CoreError::io(path, error)),
    }
}

fn verify_generation_pair(
    root: &Path,
    generation_digest: &str,
    source_digest: &str,
    package_digest: &str,
) -> Result<()> {
    let snapshot: BookmarkSnapshot = read_json(&candidates_path(root))?;
    let report: super::BookmarkGenerationReport = read_json(&generation_report_path(root))?;
    snapshot.validate()?;
    report.validate()?;
    if snapshot.generation_digest != generation_digest
        || report.generation_digest != generation_digest
        || snapshot.source_digest != source_digest
        || report.source_digest != source_digest
        || snapshot.package_digest != package_digest
        || report.package_digest != package_digest
    {
        return Err(CoreError::InvalidDocument(
            "committed bookmark candidates/report are not a matching generation".into(),
        ));
    }
    Ok(())
}
pub fn candidates_path(root: &Path) -> PathBuf {
    root.join("bookmarks/candidates.json")
}
pub fn reviews_path(root: &Path) -> PathBuf {
    root.join("bookmarks/reviews.json")
}
pub fn generation_report_path(root: &Path) -> PathBuf {
    root.join("bookmarks/generation-report.json")
}
pub fn load_generation_report(root: &Path) -> Result<super::BookmarkGenerationReport> {
    let report: super::BookmarkGenerationReport = read_json(&generation_report_path(root))?;
    report.validate()?;
    Ok(report)
}
/// Writes the report with the same atomic, same-directory, no-symlink rules
/// as the snapshot, and only when both describe the same generation.
pub fn save_generation_report(
    root: &Path,
    report: &super::BookmarkGenerationReport,
    snapshot: &BookmarkSnapshot,
    overwrite: bool,
) -> Result<()> {
    report.validate()?;
    snapshot.validate()?;
    if report.generation_digest != snapshot.generation_digest
        || report.source_digest != snapshot.source_digest
        || report.package_digest != snapshot.package_digest
    {
        return Err(CoreError::InvalidDocument(
            "bookmark report and snapshot describe different generations".into(),
        ));
    }
    let directory = safe_dir(root)?;
    let path = directory.join("generation-report.json");
    if !overwrite && fs::symlink_metadata(&path).is_ok() {
        return Err(CoreError::DestinationConflict(
            "bookmark generation report already exists".into(),
        ));
    }
    write_atomic(&path, report)
}
/// Saves a snapshot and its report together. Neither is written unless both
/// validate and agree, so a report can never describe a snapshot that is not
/// on disk.
pub fn save_generation(
    root: &Path,
    result: &super::AutoBookmarkResult,
    overwrite: bool,
) -> Result<()> {
    begin_generation(root, result, overwrite)?.commit()
}

/// Installs a matching candidates/report pair and returns a rollback handle.
/// Both files are staged and validated before either target is touched. An
/// existing pair must itself match; a half-pair is rejected rather than
/// silently repaired or clobbered.
pub fn begin_generation(
    root: &Path,
    result: &super::AutoBookmarkResult,
    overwrite: bool,
) -> Result<GenerationTransaction> {
    result.snapshot.validate()?;
    result.report.validate()?;
    if result.report.generation_digest != result.snapshot.generation_digest
        || result.report.source_digest != result.snapshot.source_digest
        || result.report.package_digest != result.snapshot.package_digest
    {
        return Err(CoreError::InvalidDocument(
            "bookmark report and snapshot describe different generations".into(),
        ));
    }
    let directory = safe_dir(root)?;
    let snapshot_path = candidates_path(root);
    let report_path = generation_report_path(root);
    let snapshot_bytes = match fs::symlink_metadata(&snapshot_path) {
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => {
            return Err(CoreError::InvalidDocument(
                "unsafe bookmark candidates file".into(),
            ))
        }
        Ok(_) => Some(fs::read(&snapshot_path).map_err(|e| CoreError::io(&snapshot_path, e))?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(CoreError::io(&snapshot_path, e)),
    };
    let report_bytes = match fs::symlink_metadata(&report_path) {
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => {
            return Err(CoreError::InvalidDocument(
                "unsafe bookmark report file".into(),
            ))
        }
        Ok(_) => Some(fs::read(&report_path).map_err(|e| CoreError::io(&report_path, e))?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(CoreError::io(&report_path, e)),
    };
    if snapshot_bytes.is_some() != report_bytes.is_some() {
        return Err(CoreError::InvalidDocument(
            "bookmark generation is incomplete; refusing to replace a half-pair".into(),
        ));
    }
    if let (Some(snapshot_bytes_ref), Some(report_bytes_ref)) =
        (snapshot_bytes.as_ref(), report_bytes.as_ref())
    {
        if !overwrite {
            return Err(CoreError::DestinationConflict(
                "bookmark candidates already exist; pass regeneration explicitly".into(),
            ));
        }
        let old: BookmarkSnapshot = serde_json::from_slice(snapshot_bytes_ref)
            .map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
        let old_report: super::BookmarkGenerationReport = serde_json::from_slice(report_bytes_ref)
            .map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
        old.validate()?;
        old_report.validate()?;
        if old.generation_digest != old_report.generation_digest
            || old.source_digest != old_report.source_digest
            || old.package_digest != old_report.package_digest
        {
            return Err(CoreError::InvalidDocument(
                "existing bookmark candidates/report do not match".into(),
            ));
        }
    }
    let staged_snapshot = stage_json(&directory, &result.snapshot)?;
    let staged_report = match stage_json(&directory, &result.report) {
        Ok(path) => path,
        Err(error) => {
            let _ = fs::remove_file(&staged_snapshot);
            return Err(error);
        }
    };
    let install = (|| {
        install_staged(&staged_snapshot, &snapshot_path)?;
        install_staged(&staged_report, &report_path)?;
        #[cfg(unix)]
        fs::File::open(&directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|e| CoreError::io(&directory, e))?;
        Ok::<(), CoreError>(())
    })();
    if let Err(error) = install {
        let _ = fs::remove_file(&staged_snapshot);
        let _ = fs::remove_file(&staged_report);
        // Restore the exact old pair (or no pair) before returning. A failed
        // restore is part of the returned error, never silently discarded.
        return match restore_generation(root, snapshot_bytes.as_deref(), report_bytes.as_deref()) {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(CoreError::InvalidDocument(format!(
                "generation install failed: {error}; rollback failed: {rollback_error}"
            ))),
        };
    }
    Ok(GenerationTransaction {
        root: root.to_path_buf(),
        old_snapshot: snapshot_bytes,
        old_report: report_bytes,
        generation_digest: result.snapshot.generation_digest.clone(),
        source_digest: result.snapshot.source_digest.clone(),
        package_digest: result.snapshot.package_digest.clone(),
        committed: false,
    })
}
pub fn load_snapshot(root: &Path) -> Result<BookmarkSnapshot> {
    let package = crate::document_package::DocumentPackage::read_from(root)?;
    let s: BookmarkSnapshot = read_json(&candidates_path(root))?;
    s.validate()?;
    // 0.1 and 0.2 snapshots load through exactly the same path; the
    // derived document is rebuilt only when the snapshot bound one.
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
                // An explicitly empty review file carries no decision to
                // preserve. It is safe to treat it as the current empty
                // queue during regeneration; a non-empty queue remains
                // strictly bound to its original generation.
                if r.operations.is_empty() {
                    return Ok(BookmarkReviews::empty(snapshot.generation_digest.clone()));
                }
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

    fn generated_result() -> AutoBookmarkResult {
        let (package, pages) = crate::bookmark_fixtures::aligned_book();
        let ocr = crate::bookmark_fixtures::ocr_run(&pages, None);
        let derived = crate::derived::DerivedDocument::from_package(&package, Some(&ocr)).unwrap();
        generate_auto(
            &AutoBookmarkInput {
                package: &package,
                ocr: Some(&ocr),
                derived: Some(&derived),
            },
            &AutoBookmarkConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn review_save_is_append_file_replace_only() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("bookmarks")).unwrap();
        let r = BookmarkReviews::empty("a".repeat(64));
        save_reviews(d.path(), &r).unwrap();
        assert!(reviews_path(d.path()).exists());
    }

    #[test]
    fn unsafe_report_target_cannot_leave_a_new_candidates_file_behind() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("bookmarks")).unwrap();
        // A directory at the report target is an unsafe destination. The
        // pair preflight must reject it before staging or installing either
        // generation file.
        std::fs::create_dir(generation_report_path(d.path())).unwrap();
        let result = generated_result();

        assert!(begin_generation(d.path(), &result, false).is_err());
        assert!(!candidates_path(d.path()).exists());
        assert!(generation_report_path(d.path()).is_dir());
    }

    #[test]
    fn stale_empty_reviews_are_reset_for_explicit_regeneration() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("bookmarks")).unwrap();
        let result = generated_result();
        save_reviews(d.path(), &BookmarkReviews::empty("0".repeat(64))).unwrap();
        let reviews = load_reviews(d.path(), &result.snapshot).unwrap();
        assert!(reviews.operations.is_empty());
        assert_eq!(
            reviews.base_generation_digest,
            result.snapshot.generation_digest
        );
    }

    #[test]
    fn second_member_install_failure_restores_an_existing_pair_byte_for_byte() {
        let _hook_guard = INSTALL_HOOK_LOCK.lock().unwrap();
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("bookmarks")).unwrap();
        let result = generated_result();
        save_generation(d.path(), &result, false).unwrap();
        let old_snapshot = fs::read(candidates_path(d.path())).unwrap();
        let old_report = fs::read(generation_report_path(d.path())).unwrap();

        FAIL_REPORT_INSTALL_ONCE.store(true, Ordering::SeqCst);
        assert!(begin_generation(d.path(), &result, true).is_err());
        assert_eq!(fs::read(candidates_path(d.path())).unwrap(), old_snapshot);
        assert_eq!(
            fs::read(generation_report_path(d.path())).unwrap(),
            old_report
        );
        assert!(!fs::read_dir(d.path().join("bookmarks"))
            .unwrap()
            .flatten()
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".bookmark-generation-")
            }));
    }

    #[test]
    fn second_member_install_failure_on_first_generation_leaves_no_pair() {
        let _hook_guard = INSTALL_HOOK_LOCK.lock().unwrap();
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("bookmarks")).unwrap();
        FAIL_REPORT_INSTALL_ONCE.store(true, Ordering::SeqCst);
        assert!(begin_generation(d.path(), &generated_result(), false).is_err());
        assert!(!candidates_path(d.path()).exists());
        assert!(!generation_report_path(d.path()).exists());
    }
}
