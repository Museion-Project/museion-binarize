use std::path::{Path, PathBuf};

use mpdf_core::document_package::DocumentPackage;
use mpdf_core::document_session::PdfOpenOptions;
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
    let package = DocumentPackage::create_from_pdf(
        &input,
        &PdfOpenOptions {
            pdfium: PdfiumConfig {
                library_path: Some(library),
                allow_system_library: false,
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(package.pages.len(), 3);
    assert!(package.pages[0].master_space.width > 0.0);
    assert!(package.pages[0].transforms[0].d < 0.0);
    let output = directory.path().join("mixed.mdp");
    package.write_to(&output).unwrap();
    DocumentPackage::read_from(&output).unwrap();
}
