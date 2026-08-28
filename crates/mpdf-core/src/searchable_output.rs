//! The one safe boundary for writing a searchable, outlined derivative.
//!
//! Both front ends call [`build_searchable_output`]; neither re-implements
//! the source-binding checks, the temporary-file discipline, or the reopen
//! verification. The source PDF is never modified and never opened for
//! writing, the destination is only created after every input check has
//! passed, and the finished file is verified before it is installed:
//!
//! * PDFium reopens it and re-reads page count, geometry, and rotation;
//! * lopdf independently walks the written `/Outlines` tree and compares its
//!   titles, nesting, and destination pages with the effective bookmarks —
//!   a documented claim is never accepted in place of a check that can be
//!   made.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use lopdf::{Document, Object, ObjectId};
use sha2::{Digest, Sha256};

use crate::bookmarks::{self, BookmarkCandidate};
use crate::derived::DerivedDocument;
use crate::document_package::DocumentPackage;
use crate::document_session::{PdfDocumentSession, PdfOpenOptions};
use crate::error::{CoreError, Result};
use crate::pdfium_backend::PdfiumConfig;

pub struct SearchableOutputRequest<'a> {
    pub package: &'a DocumentPackage,
    pub source: &'a Path,
    pub output: &'a Path,
    pub overwrite: bool,
    pub candidates: &'a [BookmarkCandidate],
    pub derived: Option<&'a DerivedDocument>,
    pub pdfium: PdfiumConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchableOutputSummary {
    pub output_path: PathBuf,
    pub source_sha256: String,
    pub output_sha256: String,
    pub written_bookmarks: usize,
    pub auto_confirmed_bookmarks: usize,
    pub human_confirmed_bookmarks: usize,
    pub byte_len: u64,
}

/// Coarse stages a front end can show while a derivative is produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStage {
    WritingPdf,
    Validating,
}

impl OutputStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WritingPdf => "writing_pdf",
            Self::Validating => "validating",
        }
    }
}

pub fn build_searchable_output(
    request: &SearchableOutputRequest<'_>,
) -> Result<SearchableOutputSummary> {
    build_searchable_output_with_cancel(request, &|| false)
}

pub fn build_searchable_output_with_cancel(
    request: &SearchableOutputRequest<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<SearchableOutputSummary> {
    build_searchable_output_observed(request, cancelled, &|_| {})
}

/// As [`build_searchable_output_with_cancel`], reporting each stage so a
/// desktop UI can show progress without duplicating any of this logic.
pub fn build_searchable_output_observed(
    request: &SearchableOutputRequest<'_>,
    cancelled: &dyn Fn() -> bool,
    stage: &dyn Fn(OutputStage),
) -> Result<SearchableOutputSummary> {
    let source_metadata =
        fs::symlink_metadata(request.source).map_err(|e| CoreError::io(request.source, e))?;
    if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
        return Err(CoreError::InvalidDocument(
            "source PDF must be a regular file".into(),
        ));
    }
    let bytes = fs::read(request.source).map_err(|e| CoreError::io(request.source, e))?;
    let source_sha256 = hex(&Sha256::digest(&bytes));
    if source_sha256 != request.package.source.content_sha256 {
        return Err(CoreError::InvalidDocument(
            "source PDF digest does not match MDP source binding".into(),
        ));
    }
    if same_path(request.output, request.source) {
        return Err(CoreError::DestinationConflict(
            "source and output must be distinct".into(),
        ));
    }
    if let Ok(metadata) = fs::symlink_metadata(request.output) {
        if metadata.file_type().is_symlink() || metadata.is_dir() || !request.overwrite {
            return Err(CoreError::DestinationConflict(
                "output exists or is unsafe; pass overwrite for a regular file".into(),
            ));
        }
    }
    let writable: Vec<BookmarkCandidate> = request
        .candidates
        .iter()
        .filter(|candidate| candidate.status.writes_to_pdf())
        .cloned()
        .collect();
    let auto_confirmed_bookmarks = writable
        .iter()
        .filter(|candidate| candidate.status == bookmarks::BookmarkStatus::AutoConfirmed)
        .count();
    stage(OutputStage::WritingPdf);
    let built = crate::searchable_pdf::build_with_cancel(
        &bytes,
        request.package,
        request.candidates,
        request.derived,
        cancelled,
    )?;
    if cancelled() {
        return Err(CoreError::Cancelled);
    }
    let parent = request.output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| CoreError::io(parent, e))?;
    temporary
        .write_all(&built)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| CoreError::io(temporary.path(), e))?;
    stage(OutputStage::Validating);
    verify_geometry(temporary.path(), request.package, &request.pdfium)?;
    verify_outline(temporary.path(), request.package, &writable)?;
    if cancelled() {
        return Err(CoreError::Cancelled);
    }
    temporary
        .persist(request.output)
        .map_err(|e| CoreError::io(request.output, e.error))?;
    // The source bytes must be exactly what they were before the write.
    let after = fs::read(request.source).map_err(|e| CoreError::io(request.source, e))?;
    if after != bytes {
        return Err(CoreError::OutputValidationFailed(
            "source PDF changed while its derivative was written".into(),
        ));
    }
    Ok(SearchableOutputSummary {
        output_path: request.output.to_path_buf(),
        source_sha256,
        output_sha256: hex(&Sha256::digest(&built)),
        written_bookmarks: writable.len(),
        auto_confirmed_bookmarks,
        human_confirmed_bookmarks: writable.len() - auto_confirmed_bookmarks,
        byte_len: built.len() as u64,
    })
}

fn verify_geometry(path: &Path, package: &DocumentPackage, pdfium: &PdfiumConfig) -> Result<()> {
    let session = PdfDocumentSession::open(
        path,
        &PdfOpenOptions {
            password: None,
            pdfium: pdfium.clone(),
            compute_source_hash: false,
        },
    )
    .map_err(|error| CoreError::OutputValidationFailed(error.to_string()))?;
    let info = session.info();
    let matches = info.page_count == package.manifest.page_count
        && info
            .pages
            .iter()
            .zip(&package.pages)
            .all(|(actual, expected)| {
                actual.source_rotation.degrees() as u16 == expected.rotation_degrees
                    && (f64::from(actual.geometry.width_points) - expected.source_space.width).abs()
                        < 0.05
                    && (f64::from(actual.geometry.height_points) - expected.source_space.height)
                        .abs()
                        < 0.05
            });
    if !matches {
        return Err(CoreError::OutputValidationFailed(
            "output page count, geometry, or rotation changed".into(),
        ));
    }
    Ok(())
}

/// Independently re-reads the written outline and compares it with the tree
/// that was supposed to be written: count, titles, nesting depth, and the
/// physical destination page of every node.
fn verify_outline(
    path: &Path,
    package: &DocumentPackage,
    expected: &[BookmarkCandidate],
) -> Result<()> {
    let document =
        Document::load(path).map_err(|e| CoreError::OutputValidationFailed(e.to_string()))?;
    let catalog = document
        .catalog()
        .map_err(|e| CoreError::OutputValidationFailed(e.to_string()))?;
    let outlines = catalog.get(b"Outlines").ok();
    if expected.is_empty() {
        if outlines.is_some() {
            return Err(CoreError::OutputValidationFailed(
                "an outline was written for a document with no confirmed bookmarks".into(),
            ));
        }
        return Ok(());
    }
    let root = outlines
        .and_then(|value| value.as_reference().ok())
        .ok_or_else(|| CoreError::OutputValidationFailed("output has no outline root".into()))?;
    let pages: BTreeMap<ObjectId, u32> = document
        .get_pages()
        .into_iter()
        .map(|(number, id)| (id, number.saturating_sub(1)))
        .collect();
    let mut actual = Vec::new();
    walk_outline(&document, root, 0, &pages, &mut actual)?;
    if actual.len() != expected.len() {
        return Err(CoreError::OutputValidationFailed(format!(
            "output outline has {} entries but {} were confirmed",
            actual.len(),
            expected.len()
        )));
    }
    // Compare as multisets keyed by title, depth, and target page: the PDF
    // sibling order is the tree order, which the writer derives from the same
    // effective list.
    let mut expected_keys: Vec<(String, u16, u32)> = expected
        .iter()
        .map(|candidate| {
            (
                candidate.effective_title.clone(),
                depth_of(expected, candidate),
                candidate.physical_page_index,
            )
        })
        .collect();
    expected_keys.sort();
    actual.sort();
    if actual != expected_keys {
        return Err(CoreError::OutputValidationFailed(
            "output outline titles, nesting, or destination pages do not match the confirmed tree"
                .into(),
        ));
    }
    if package.manifest.page_count == 0 {
        return Err(CoreError::OutputValidationFailed(
            "package declares no pages".into(),
        ));
    }
    Ok(())
}

fn depth_of(candidates: &[BookmarkCandidate], candidate: &BookmarkCandidate) -> u16 {
    let mut depth = 0u16;
    let mut current = candidate.effective_parent_id.clone();
    let mut guard = 0;
    while let Some(id) = current {
        guard += 1;
        if guard > 64 {
            break;
        }
        let Some(parent) = candidates
            .iter()
            .find(|other| other.candidate_id == id)
            .filter(|other| other.status.writes_to_pdf())
        else {
            break;
        };
        depth += 1;
        current = parent.effective_parent_id.clone();
    }
    depth
}

fn walk_outline(
    document: &Document,
    parent: ObjectId,
    depth: u16,
    pages: &BTreeMap<ObjectId, u32>,
    into: &mut Vec<(String, u16, u32)>,
) -> Result<()> {
    if depth > 64 || into.len() > 100_000 {
        return Err(CoreError::OutputValidationFailed(
            "output outline is too deep or too large".into(),
        ));
    }
    let dictionary = document
        .get_dictionary(parent)
        .map_err(|e| CoreError::OutputValidationFailed(e.to_string()))?;
    let mut current = dictionary.get(b"First").and_then(Object::as_reference).ok();
    let mut seen = std::collections::HashSet::new();
    while let Some(node) = current {
        if !seen.insert(node) {
            return Err(CoreError::OutputValidationFailed(
                "output outline contains a sibling cycle".into(),
            ));
        }
        let node_dictionary = document
            .get_dictionary(node)
            .map_err(|e| CoreError::OutputValidationFailed(e.to_string()))?;
        let title = node_dictionary
            .get(b"Title")
            .and_then(Object::as_str)
            .map(decode_pdf_text)
            .map_err(|e| CoreError::OutputValidationFailed(e.to_string()))?;
        let destination = node_dictionary
            .get(b"Dest")
            .and_then(Object::as_array)
            .map_err(|_| {
                CoreError::OutputValidationFailed(
                    "outline entry has no explicit destination".into(),
                )
            })?;
        let page_reference = destination
            .first()
            .and_then(|value| value.as_reference().ok())
            .ok_or_else(|| {
                CoreError::OutputValidationFailed(
                    "outline destination does not name a page object".into(),
                )
            })?;
        let page_index = *pages.get(&page_reference).ok_or_else(|| {
            CoreError::OutputValidationFailed(
                "outline destination points outside the page tree".into(),
            )
        })?;
        into.push((title, depth, page_index));
        walk_outline(document, node, depth + 1, pages, into)?;
        current = node_dictionary
            .get(b"Next")
            .and_then(Object::as_reference)
            .ok();
    }
    Ok(())
}

fn decode_pdf_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

pub fn same_path(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(a), Ok(b)) => a == b,
        _ => {
            left.file_name() == right.file_name()
                && left.parent().and_then(|p| fs::canonicalize(p).ok())
                    == right.parent().and_then(|p| fs::canonicalize(p).ok())
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bookmark_fixtures as fixtures;
    use crate::bookmarks::{BookmarkCandidate, BookmarkStatus, EvidenceRef, GeneratorProvenance};

    fn candidate(id: &str, title: &str, page: usize, parent: Option<&str>) -> BookmarkCandidate {
        let package = fixtures::package_for_source(&fixtures::source_pdf(4), 4);
        BookmarkCandidate {
            candidate_id: id.to_owned(),
            source_title: title.to_owned(),
            effective_title: title.to_owned(),
            source_level: u16::from(parent.is_some()),
            effective_level: u16::from(parent.is_some()),
            source_parent_id: parent.map(str::to_owned),
            effective_parent_id: parent.map(str::to_owned),
            target_page_id: package.pages[page].page_id.clone(),
            physical_page_index: page as u32,
            master_bbox: None,
            outline_evidence: None,
            evidence: vec![EvidenceRef::DerivedPage {
                page_id: package.pages[page].page_id.clone(),
                bbox: crate::derived::Bbox {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
            }],
            confidence: 1.0,
            status: BookmarkStatus::AutoConfirmed,
            generator: GeneratorProvenance {
                kind: crate::bookmarks::GENERATOR_KIND.into(),
                name: crate::bookmarks::GENERATOR_NAME.into(),
                version: "0.2".into(),
            },
            reason_codes: vec!["test".into()],
            rule_trace: vec!["test".into()],
            confidence_breakdown: Some(crate::bookmarks::ConfidenceBreakdown {
                title_match: 4_000,
                page_mapping: 2_000,
                numbering_hierarchy: 1_000,
                body_layout: 1_000,
                ocr_quality: 1_000,
                sequence_uniqueness: 1_000,
                total: 10_000,
            }),
            alignment_evidence: None,
            automatic_decision: Some(crate::bookmarks::AutomaticDecision {
                decided_status: "auto_confirmed".into(),
                reason: "test".into(),
                rule_version: "0.2".into(),
                rule_config_digest: "0".repeat(64),
            }),
        }
    }

    /// Writes a real outlined PDF and returns its path plus the package.
    fn written(
        directory: &std::path::Path,
        candidates: &[BookmarkCandidate],
    ) -> (std::path::PathBuf, DocumentPackage) {
        let source = fixtures::source_pdf(4);
        let package = fixtures::package_for_source(&source, 4);
        let built = crate::searchable_pdf::build(&source, &package, candidates, None).unwrap();
        let path = directory.join("out.pdf");
        std::fs::write(&path, built).unwrap();
        (path, package)
    }

    #[test]
    fn a_correctly_written_outline_verifies() {
        let directory = tempfile::tempdir().unwrap();
        let candidates = vec![
            candidate("a", "Ἀρχή", 1, None),
            candidate("b", "Child", 2, Some("a")),
        ];
        let (path, package) = written(directory.path(), &candidates);
        verify_outline(&path, &package, &candidates).expect("the written tree matches");
    }

    #[test]
    fn a_changed_title_nesting_or_destination_fails_verification() {
        let directory = tempfile::tempdir().unwrap();
        let candidates = vec![
            candidate("a", "Ἀρχή", 1, None),
            candidate("b", "Child", 2, Some("a")),
        ];
        let (path, package) = written(directory.path(), &candidates);

        let mut renamed = candidates.clone();
        renamed[0].effective_title = "A Different Title".into();
        assert!(verify_outline(&path, &package, &renamed).is_err());

        let mut moved = candidates.clone();
        moved[1].physical_page_index = 3;
        moved[1].target_page_id = package.pages[3].page_id.clone();
        assert!(verify_outline(&path, &package, &moved).is_err());

        let mut flattened = candidates.clone();
        flattened[1].effective_parent_id = None;
        assert!(
            verify_outline(&path, &package, &flattened).is_err(),
            "a child written under a parent must not verify as a sibling"
        );

        let mut extra = candidates.clone();
        extra.push(candidate("c", "Never Written", 3, None));
        assert!(verify_outline(&path, &package, &extra).is_err());
    }

    #[test]
    fn an_outline_written_for_nothing_fails_verification() {
        let directory = tempfile::tempdir().unwrap();
        let candidates = vec![candidate("a", "Ἀρχή", 1, None)];
        let (path, package) = written(directory.path(), &candidates);
        assert!(
            verify_outline(&path, &package, &[]).is_err(),
            "an outline must not exist when nothing was confirmed"
        );
        let (empty_path, package) = written(directory.path(), &[]);
        verify_outline(&empty_path, &package, &[]).expect("no outline, nothing expected");
    }

    #[test]
    fn a_source_and_output_alias_is_detected_even_before_the_file_exists() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("book.pdf");
        std::fs::write(&source, b"pdf").unwrap();
        assert!(same_path(&source, &directory.path().join("book.pdf")));
        assert!(!same_path(&source, &directory.path().join("other.pdf")));
        assert!(!same_path(
            &directory.path().join("missing-a.pdf"),
            &directory.path().join("missing-b.pdf")
        ));
    }
}
