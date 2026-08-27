use std::path::{Path, PathBuf};

use mpdf_core::derived::{export, DerivedDocument, ExportFormat};
use mpdf_core::document_package::DocumentPackage;
use mpdf_core::document_session::PdfOpenOptions;
use mpdf_core::jobs::JobStore;
use mpdf_core::ocr::{self, ReferenceOcrProvider};
use mpdf_core::pdfium_backend::PdfiumConfig;
use mpdf_core::test_fixtures;

fn pdfium_path() -> Option<PathBuf> {
    std::env::var_os("MPDF_PDFIUM_LIBRARY")
        .map(PathBuf::from)
        .or_else(|| {
            let candidate = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/pdfium/aarch64-apple-darwin/libpdfium.dylib");
            candidate.is_file().then_some(candidate)
        })
}

/// Real PDFium/session integration is kept ignored for ordinary portable CI;
/// run it locally with `cargo test -p mpdf-core --test document_package
/// -- --ignored` on a host with the bundled dynamic library.
#[test]
#[ignore = "requires the host PDFium dynamic library"]
fn creates_and_validates_package_from_real_pdfium_geometry() {
    let library = pdfium_path().expect("set MPDF_PDFIUM_LIBRARY or build target/pdfium");
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("mixed.pdf");
    std::fs::write(&input, test_fixtures::mixed_page_sizes()).unwrap();
    let session = mpdf_core::document_session::PdfDocumentSession::open(
        &input,
        &PdfOpenOptions {
            pdfium: PdfiumConfig {
                library_path: Some(library.clone()),
                allow_system_library: false,
            },
            compute_source_hash: true,
            ..Default::default()
        },
    )
    .unwrap();
    let package = DocumentPackage::create_from_session(&session, Some("mixed.pdf".into())).unwrap();
    assert_eq!(package.pages.len(), 3);
    assert!(package.pages[0].master_space.width > 0.0);
    assert!(package.pages[0].transforms[0].d < 0.0);
    let output = directory.path().join("mixed.mdp");
    package.write_to(&output).unwrap();
    let store = JobStore::open(&directory.path().join("jobs.sqlite3")).unwrap();
    let mut provider = ReferenceOcrProvider;
    let ocr_run = ocr::run_session_durable(
        &session,
        &mut provider,
        &store,
        "real-pdfium-smoke",
        "reference-real-pdfium-smoke-v1",
        &output,
        "integration-test",
        ocr::CANONICAL_OCR_DPI,
    )
    .unwrap();
    assert!(ocr_run.is_complete(session.info().page_count));
    DocumentPackage::read_from(&output).unwrap();
    let derived = DerivedDocument::from_package(&package, Some(&ocr_run)).unwrap();
    derived.validate().unwrap();
    for format in [
        ExportFormat::Json,
        ExportFormat::Jsonl,
        ExportFormat::Markdown,
        ExportFormat::Text,
        ExportFormat::Html,
        ExportFormat::Hocr,
        ExportFormat::Alto,
    ] {
        assert!(!export(&derived, format).unwrap().is_empty());
    }
}
