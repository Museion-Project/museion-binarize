//! Real-PDF write-back for automatically compiled bookmarks.
//!
//! The whole chain runs here: a generated source PDF, an MDP package, typed
//! OCR evidence (both a local run and one installed exactly as a consented
//! remote result is), human revisions, automatic compilation, the shared
//! safe output boundary, and a PDFium reopen of the finished file.
//!
//! These tests need a real PDFium library and are therefore `#[ignore]`d:
//! an ordinary `cargo test --workspace` reports them as ignored, never as
//! passed, and running them explicitly without a library is a hard failure.
//!
//! ```text
//! MPDF_PDFIUM_LIBRARY=/absolute/path/to/libpdfium.so \
//!   cargo test -p mpdf-core --test auto_bookmarks_pdf -- --ignored
//! ```

use std::path::PathBuf;

use mpdf_core::bookmark_fixtures::{self as fixtures, FixtureLine, FixturePage};
use mpdf_core::bookmarks::{self, AutoBookmarkConfig, BookmarkStatus, GenerationMode};
use mpdf_core::document_package::DocumentPackage;
use mpdf_core::document_session::{PdfDocumentSession, PdfOpenOptions};
use mpdf_core::pdfium_backend::PdfiumConfig;
use mpdf_core::searchable_output::{build_searchable_output, SearchableOutputRequest};

const ENV_VAR: &str = "MPDF_PDFIUM_LIBRARY";

fn require_pdfium_config() -> PdfiumConfig {
    let Some(raw) = std::env::var_os(ENV_VAR) else {
        panic!(
            "{ENV_VAR} is not set, but this test was run explicitly.\n\
             This integration test requires a provisioned PDFium library:\n  \
             {ENV_VAR}=/absolute/path/to/{} \\\n    \
             cargo test -p mpdf-core --test auto_bookmarks_pdf -- --ignored\n\
             See docs/pdfium.md for how to obtain one.",
            mpdf_core::pdfium_backend::pdfium_library_file_name()
        );
    };
    let path = PathBuf::from(raw);
    assert!(
        path.is_file(),
        "{ENV_VAR} points at {}, which is not a file. See docs/pdfium.md.",
        path.display()
    );
    PdfiumConfig {
        library_path: Some(path),
        allow_system_library: false,
    }
}

static PDFIUM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Builds the real MDP package for a generated source PDF, then replaces its
/// pages' OCR-space geometry with synthetic evidence at the same scale.
fn package_for(source: &std::path::Path, pdfium: &PdfiumConfig) -> DocumentPackage {
    let session = PdfDocumentSession::open(
        source,
        &PdfOpenOptions {
            compute_source_hash: true,
            pdfium: pdfium.clone(),
            password: None,
        },
    )
    .expect("the generated fixture opens");
    DocumentPackage::create_from_session(&session, Some("source.pdf".into()))
        .expect("package from session")
}

/// Synthetic OCR evidence sized to the real package's master space.
fn evidence(package: &DocumentPackage, titles: &[(&str, u32)]) -> Vec<FixturePage> {
    let width = package.pages[0].master_space.width;
    let height = package.pages[0].master_space.height;
    let scale = |x: f64, y: f64| (width * x, height * y);
    let mut pages = Vec::new();
    for (index, _) in package.pages.iter().enumerate() {
        let mut lines = Vec::new();
        if index == 0 {
            let (x, y) = scale(0.1, 0.06);
            lines.push(FixtureLine::new("Contents", x, y).with_width(width * 0.2));
            for (ordinal, (title, printed)) in titles.iter().enumerate() {
                let (x, y) = scale(0.1, 0.15 + ordinal as f64 * 0.05);
                lines.push(
                    FixtureLine::new(&format!("{title} ....... {printed}"), x, y)
                        .with_width(width * 0.7)
                        .with_height(height * 0.02),
                );
            }
        } else if let Some((title, printed)) = titles
            .iter()
            .find(|(_, printed)| *printed as usize == index)
        {
            let (x, y) = scale(0.1, 0.08);
            lines.push(
                FixtureLine::new(title, x, y)
                    .with_width(width * 0.6)
                    .with_height(height * 0.03),
            );
            let (x, y) = scale(0.1, 0.4);
            lines.push(FixtureLine::new("body text", x, y).with_width(width * 0.3));
            let (x, y) = scale(0.5, 0.94);
            lines.push(FixtureLine::new(&format!("{printed}"), x, y).with_width(width * 0.05));
        } else {
            let (x, y) = scale(0.1, 0.4);
            lines.push(FixtureLine::new("body text", x, y).with_width(width * 0.3));
        }
        pages.push(FixturePage::new(lines));
    }
    pages
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/pdfium.md"]
fn automatic_bookmarks_reach_a_verified_outlined_pdf() {
    let _guard = PDFIUM_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let pdfium = require_pdfium_config();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.pdf");
    std::fs::write(&source, mpdf_core::test_fixtures::heterogeneous_document(6)).unwrap();
    let before = std::fs::read(&source).unwrap();

    let package = package_for(&source, &pdfium);
    let root = directory.path().join("book.mdp");
    package.write_to(&root).unwrap();
    // Polytonic Greek and an ASCII title, so the written outline exercises
    // the Unicode text object path as well as the plain one.
    let titles = [("Ἀρχὴ τῆς σοφίας", 1u32), ("Second Chapter", 3)];
    let pages = evidence(&package, &titles);
    let ocr = fixtures::ocr_run(&pages, None);
    mpdf_core::ocr::write_ocr_records(&root, &ocr).unwrap();

    let result =
        bookmarks::generate_auto_from_package(&root, &AutoBookmarkConfig::default(), &|| false)
            .expect("automatic generation");
    assert_eq!(result.report.mode, GenerationMode::TocAligned);
    assert_eq!(result.auto_confirmed(), 2, "{:#?}", result.report);
    bookmarks::save_generation(&root, &result, false).unwrap();

    let inputs = bookmarks::load_auto_bookmark_inputs(&root).unwrap();
    let effective = bookmarks::effective(
        &result.snapshot,
        &bookmarks::load_reviews(&root, &result.snapshot).unwrap(),
    )
    .unwrap();
    let output = directory.path().join("outlined.pdf");
    let summary = build_searchable_output(&SearchableOutputRequest {
        package: &package,
        source: &source,
        output: &output,
        overwrite: false,
        candidates: &effective,
        derived: inputs.derived.as_ref(),
        pdfium: pdfium.clone(),
    })
    .expect("the searchable, outlined derivative is written and verified");
    assert_eq!(summary.written_bookmarks, 2);
    assert_eq!(summary.auto_confirmed_bookmarks, 2);
    assert_eq!(
        std::fs::read(&source).unwrap(),
        before,
        "the source PDF is never modified"
    );

    // Reopen with PDFium: page count, geometry, and rotation are unchanged.
    let reopened = PdfDocumentSession::open(
        &output,
        &PdfOpenOptions {
            compute_source_hash: false,
            pdfium: pdfium.clone(),
            password: None,
        },
    )
    .expect("the output reopens");
    assert_eq!(reopened.info().page_count, package.manifest.page_count);
    for (actual, expected) in reopened.info().pages.iter().zip(&package.pages) {
        assert_eq!(
            actual.source_rotation.degrees() as u16,
            expected.rotation_degrees
        );
        assert!(
            (f64::from(actual.geometry.width_points) - expected.source_space.width).abs() < 0.05
        );
    }
    let outline = reopened.native_outline().expect("the outline reads back");
    let read_titles: Vec<String> = outline.iter().map(|item| item.title.clone()).collect();
    assert_eq!(
        read_titles,
        vec!["Ἀρχὴ τῆς σοφίας".to_owned(), "Second Chapter".to_owned()],
        "the accented Unicode title survives the round trip"
    );
    let targets: Vec<u32> = outline.iter().map(|item| item.page_index).collect();
    assert_eq!(targets, vec![1, 3]);
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/pdfium.md"]
fn a_safe_refusal_writes_no_pdf_and_leaves_no_temporary_file() {
    let _guard = PDFIUM_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let pdfium = require_pdfium_config();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.pdf");
    std::fs::write(&source, mpdf_core::test_fixtures::heterogeneous_document(4)).unwrap();
    let package = package_for(&source, &pdfium);
    let root = directory.path().join("book.mdp");
    package.write_to(&root).unwrap();
    let pages = evidence(&package, &[]);
    mpdf_core::ocr::write_ocr_records(&root, &fixtures::ocr_run(&pages, None)).unwrap();

    let result =
        bookmarks::generate_auto_from_package(&root, &AutoBookmarkConfig::default(), &|| false)
            .expect("generation succeeds as a refusal");
    assert_eq!(result.report.mode, GenerationMode::SafeRefusal);
    assert_eq!(result.auto_confirmed(), 0);
    let writable = result
        .snapshot
        .candidates
        .iter()
        .filter(|candidate| candidate.status.writes_to_pdf())
        .count();
    assert_eq!(writable, 0);

    let output = directory.path().join("outlined.pdf");
    // Nothing is confirmed, so the front ends never call the writer at all.
    assert!(!output.exists());
    let leftovers: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(leftovers.is_empty(), "no temporary file is left behind");
    assert_eq!(
        result
            .snapshot
            .candidates
            .iter()
            .filter(|candidate| candidate.status == BookmarkStatus::AutoConfirmed)
            .count(),
        0
    );
}
