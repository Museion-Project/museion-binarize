//! Safe source-boundary for the searchable-PDF writer.  The low-level writer
//! is intentionally kept behind this seam so a malformed MDP can never cause
//! a destination to be opened before all source checks have completed.
use crate::cli::PdfBuildSearchableArgs;
use crate::errors::{self, ExitReason};
use crate::output;
use mpdf_core::bookmarks;
use mpdf_core::document_package::DocumentPackage;
use mpdf_core::document_session::{PdfDocumentSession, PdfOpenOptions};
use mpdf_core::error::CoreError;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::process::ExitCode;
pub fn build_searchable(a: PdfBuildSearchableArgs) -> ExitCode {
    let package = match DocumentPackage::read_from(&a.input) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    match fs::symlink_metadata(&a.source) {
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => {
            return fail(&CoreError::InvalidDocument(
                "source PDF must be a regular file".into(),
            ))
        }
        Err(e) => return fail(&CoreError::io(&a.source, e)),
        _ => {}
    }
    let bytes = match fs::read(&a.source) {
        Ok(x) => x,
        Err(e) => return fail(&CoreError::io(&a.source, e)),
    };
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if digest != package.source.content_sha256 {
        return fail(&CoreError::InvalidDocument(
            "source PDF digest does not match MDP source binding".into(),
        ));
    }
    if a.output == a.source || same_path(&a.output, &a.source) {
        return fail(&CoreError::DestinationConflict(
            "source and output must be distinct".into(),
        ));
    }
    if let Ok(m) = fs::symlink_metadata(&a.output) {
        if m.file_type().is_symlink() || m.is_dir() || (!a.overwrite) {
            return fail(&CoreError::DestinationConflict(
                "output exists or is unsafe; pass --overwrite for a regular file".into(),
            ));
        }
    }
    let snapshot = match bookmarks::load_snapshot(&a.input) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let reviews = match bookmarks::load_reviews(&a.input, &snapshot) {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let effective = match bookmarks::effective(&snapshot, &reviews) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    let derived = match load_derived(&a.input, &package) {
        Ok(d) => d,
        Err(e) => return fail(&e),
    };
    let built =
        match mpdf_core::searchable_pdf::build(&bytes, &package, &effective, derived.as_ref()) {
            Ok(x) => x,
            Err(e) => return fail(&e),
        };
    let parent = a.output.parent().unwrap_or_else(|| Path::new("."));
    if let Err(e) = fs::create_dir_all(parent) {
        return fail(&CoreError::io(parent, e));
    }
    let mut temp = match tempfile::NamedTempFile::new_in(parent) {
        Ok(x) => x,
        Err(e) => return fail(&CoreError::io(parent, e)),
    };
    use std::io::Write;
    if let Err(e) = temp
        .write_all(&built)
        .and_then(|_| temp.as_file().sync_all())
    {
        return fail(&CoreError::io(parent, e));
    }
    match PdfDocumentSession::open(temp.path(), &PdfOpenOptions::default()) {
        Ok(session)
            if session.info().page_count == package.manifest.page_count
                && session
                    .info()
                    .pages
                    .iter()
                    .zip(&package.pages)
                    .all(|(actual, expected)| {
                        actual.source_rotation.degrees() as u16 == expected.rotation_degrees
                            && (f64::from(actual.geometry.width_points)
                                - expected.source_space.width)
                                .abs()
                                < 0.05
                            && (f64::from(actual.geometry.height_points)
                                - expected.source_space.height)
                                .abs()
                                < 0.05
                    }) => {}
        Ok(_) => {
            return fail(&CoreError::OutputValidationFailed(
                "output page count, geometry, or rotation changed".into(),
            ))
        }
        Err(e) => return fail(&CoreError::OutputValidationFailed(e.to_string())),
    }
    if let Err(e) = temp.persist(&a.output) {
        return fail(&CoreError::io(&a.output, e.error));
    }
    if a.output_mode.json {
        output::print_json(
            &serde_json::json!({"schema":"mpdf-searchable-pdf","schema_version":"0.1","source_sha256":digest,"confirmed_bookmarks":effective.iter().filter(|c|matches!(c.status,bookmarks::BookmarkStatus::Confirmed)).count()}),
            a.output_mode.pretty,
        )
    } else if !a.output_mode.quiet {
        println!("built searchable PDF: {}", a.output.display());
    }
    ExitReason::Success.exit_code()
}

fn load_derived(
    root: &Path,
    p: &DocumentPackage,
) -> Result<Option<mpdf_core::derived::DerivedDocument>, CoreError> {
    let d = root.join("ocr");
    match fs::symlink_metadata(&d) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::io(&d, e)),
        Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
            Err(CoreError::InvalidDocument("OCR directory is unsafe".into()))
        }
        Ok(_) => {
            let o = mpdf_core::ocr::read_ocr_records(root)
                .map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
            let mut x = mpdf_core::derived::DerivedDocument::from_package(p, Some(&o))?;
            let r = mpdf_core::derived::load_revisions(root)?;
            x.apply_revisions(&r)?;
            Ok(Some(x))
        }
    }
}
fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => {
            a.file_name() == b.file_name()
                && a.parent().and_then(|x| std::fs::canonicalize(x).ok())
                    == b.parent().and_then(|x| std::fs::canonicalize(x).ok())
        }
    }
}
fn fail(e: &CoreError) -> ExitCode {
    eprintln!("error: {}", e);
    errors::classify(e).1.exit_code()
}
