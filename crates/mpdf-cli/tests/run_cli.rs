//! End-to-end contract for `mpdf run`.
//!
//! These tests open and write real PDF bytes, so they require the same
//! provisioned PDFium library as the core integration suites. They are
//! ignored during an ordinary workspace run and fail clearly when invoked
//! explicitly without the required environment variable.

use std::path::PathBuf;
use std::process::Command;

const PDFIUM_ENV: &str = "MPDF_PDFIUM_LIBRARY";

fn pdfium_path() -> PathBuf {
    let Some(raw) = std::env::var_os(PDFIUM_ENV) else {
        panic!("{PDFIUM_ENV} is not set, but this test was run explicitly; see docs/pdfium.md");
    };
    let path = PathBuf::from(raw);
    assert!(path.is_file(), "{} is not a file", path.display());
    path
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/pdfium.md"]
fn bookmark_safe_refusal_still_exits_successfully_and_writes_the_final_pdf() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.pdf");
    let output = directory.path().join("final.pdf");
    let workspace = directory.path().join("workspace");
    std::fs::write(&source, mpdf_core::test_fixtures::heterogeneous_document(3)).unwrap();
    let source_before = std::fs::read(&source).unwrap();

    let result = Command::new(env!("CARGO_BIN_EXE_mpdf"))
        .args([
            "run",
            source.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--workspace",
            workspace.to_str().unwrap(),
            "--provider",
            "reference",
            "--on-review",
            "confirmed",
            "--method",
            "otsu",
            "--dpi",
            "300",
            "--pdfium-library",
            pdfium_path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "mpdf run failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["schema"], "mpdf-run");
    assert_eq!(report["schema_version"], "1.1");
    assert_eq!(report["status"], "completed");
    assert_eq!(report["bookmark_status"], "safe_refusal");
    assert_eq!(report["bookmarks_written"], 0);
    assert!(report["refusal_reason"].is_string());
    assert_eq!(report["output_path"], output.display().to_string());
    assert!(report["output_verification"]["geometry_matches_source"] == true);
    assert!(report["output_verification"]["outline_matches_confirmed"] == true);
    assert!(report["output_verification"]["source_unchanged"] == true);

    let stages = report["stages"].as_array().unwrap();
    let ocr = stages
        .iter()
        .position(|stage| stage == "ocr_original")
        .unwrap();
    let binarize = stages
        .iter()
        .position(|stage| stage == "binarizing_visuals")
        .unwrap();
    assert!(ocr < binarize, "OCR must precede binarization: {stages:?}");
    assert!(
        output.is_file(),
        "a safe refusal must still produce the PDF"
    );
    assert_eq!(std::fs::read(&source).unwrap(), source_before);
}
