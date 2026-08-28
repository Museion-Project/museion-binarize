//! Binary-level bookmark review and searchable-PDF output.
//!
//! These tests drive the real `mpdf` binary against a real PDFium library
//! and are therefore `#[ignore]`d: an ordinary `cargo test --workspace`
//! reports them as ignored, never as passed. Running one explicitly without
//! `MPDF_PDFIUM_LIBRARY` set is a hard failure, not a quiet success.
//!
//! ```text
//! MPDF_PDFIUM_LIBRARY=/absolute/path/to/libpdfium.so \
//!   cargo test -p mpdf-cli --test bookmarks_cli -- --ignored
//! ```

use std::ffi::OsString;
use std::fs;
use std::process::{Command, Output};

use mpdf_core::document_package::{DocumentPackage, ExistingOutlineEvidence};
use mpdf_core::document_session::{PdfDocumentSession, PdfOpenOptions};

fn run(arguments: &[&str], pdfium: &std::ffi::OsStr) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mpdf"))
        .args(arguments)
        .env("MPDF_PDFIUM_LIBRARY", pdfium)
        .output()
        .unwrap()
}

fn require_pdfium() -> OsString {
    std::env::var_os("MPDF_PDFIUM_LIBRARY").unwrap_or_else(|| {
        panic!(
            "MPDF_PDFIUM_LIBRARY is not set, but this test was run explicitly. \
             See docs/pdfium.md for how to provision a library."
        )
    })
}

#[test]
#[ignore = "requires a provisioned PDFium library; see docs/pdfium.md"]
fn m5_cli_bookmark_review_and_searchable_pdf_binary_e2e() {
    let pdfium = require_pdfium();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.pdf");
    fs::write(&source, mpdf_core::test_fixtures::page_rotations()).unwrap();
    let session = PdfDocumentSession::open(
        &source,
        &PdfOpenOptions {
            compute_source_hash: true,
            pdfium: mpdf_core::pdfium_backend::PdfiumConfig {
                library_path: Some(pdfium.clone().into()),
                allow_system_library: false,
            },
            password: None,
        },
    )
    .unwrap();
    let mut package =
        DocumentPackage::create_from_session(&session, Some("source.pdf".into())).unwrap();
    let first_page = package.pages[0].page_id.clone();
    let second_page = package.pages[1].page_id.clone();
    package.pages[0].existing_outline_evidence = vec![
        ExistingOutlineEvidence {
            title: "Root Ἀρχή".into(),
            level: 0,
            target_page_id: Some(first_page),
            source: "cli-fixture".into(),
        },
        ExistingOutlineEvidence {
            title: "Child Πολιτεία".into(),
            level: 1,
            target_page_id: Some(second_page),
            source: "cli-fixture".into(),
        },
    ];
    let mdp = directory.path().join("book.mdp");
    package.write_to(&mdp).unwrap();
    let mdp_s = mdp.to_str().unwrap();
    let generated = run(&["bookmark", "generate", mdp_s, "--json"], &pdfium);
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let snapshot: serde_json::Value = serde_json::from_slice(&generated.stdout).unwrap();
    let root = snapshot["candidates"][0]["candidate_id"].as_str().unwrap();
    let child = snapshot["candidates"][1]["candidate_id"].as_str().unwrap();

    for id in [root, child] {
        let confirmed = run(
            &["bookmark", "confirm", mdp_s, "--candidate", id, "--json"],
            &pdfium,
        );
        assert!(confirmed.status.success());
    }
    let edited = run(
        &[
            "bookmark",
            "edit",
            mdp_s,
            "--candidate",
            child,
            "--title",
            "Edited Πολιτεία",
            "--json",
        ],
        &pdfium,
    );
    assert!(edited.status.success());
    let reparented = run(
        &[
            "bookmark",
            "reparent",
            mdp_s,
            "--candidate",
            child,
            "--parent",
            root,
            "--level",
            "1",
            "--json",
        ],
        &pdfium,
    );
    assert!(reparented.status.success());
    let rejected = run(
        &["bookmark", "reject", mdp_s, "--candidate", child],
        &pdfium,
    );
    assert!(rejected.status.success());
    let reconfirmed = run(
        &["bookmark", "confirm", mdp_s, "--candidate", child],
        &pdfium,
    );
    assert!(
        reconfirmed.status.success(),
        "a later human transition must be appendable"
    );
    let listed = run(&["bookmark", "list", mdp_s, "--json"], &pdfium);
    let effective: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(effective[1]["effective_title"], "Edited Πολιτεία");
    assert_eq!(effective[1]["status"], "confirmed");

    let output = directory.path().join("searchable.pdf");
    let source_s = source.to_str().unwrap();
    let output_s = output.to_str().unwrap();
    let built = run(
        &[
            "pdf",
            "build-searchable",
            mdp_s,
            "--source",
            source_s,
            "--output",
            output_s,
            "--json",
        ],
        &pdfium,
    );
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(output.is_file());
    let no_clobber = run(
        &[
            "pdf",
            "build-searchable",
            mdp_s,
            "--source",
            source_s,
            "--output",
            output_s,
        ],
        &pdfium,
    );
    assert!(!no_clobber.status.success());
    let source_before = fs::read(&source).unwrap();
    let overwritten = run(
        &[
            "pdf",
            "build-searchable",
            mdp_s,
            "--source",
            source_s,
            "--output",
            output_s,
            "--overwrite",
        ],
        &pdfium,
    );
    assert!(overwritten.status.success());
    assert_eq!(fs::read(&source).unwrap(), source_before);
    let alias = run(
        &[
            "pdf",
            "build-searchable",
            mdp_s,
            "--source",
            source_s,
            "--output",
            source_s,
            "--overwrite",
        ],
        &pdfium,
    );
    assert!(!alias.status.success());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let source_link = directory.path().join("source-link.pdf");
        symlink(&source, &source_link).unwrap();
        let linked = run(
            &[
                "pdf",
                "build-searchable",
                mdp_s,
                "--source",
                source_link.to_str().unwrap(),
                "--output",
                directory.path().join("linked.pdf").to_str().unwrap(),
            ],
            &pdfium,
        );
        assert!(!linked.status.success());
    }

    fs::write(mdp.join("bookmarks/candidates.json"), b"{corrupt").unwrap();
    let corrupt = run(&["bookmark", "list", mdp_s, "--json"], &pdfium);
    assert!(!corrupt.status.success());
}

/// One command from an MDP package to a verified, outlined PDF.
#[test]
#[ignore = "requires a provisioned PDFium library; see docs/pdfium.md"]
fn bookmark_auto_writes_and_verifies_an_outlined_pdf_in_one_command() {
    use mpdf_core::bookmark_fixtures::{self as fixtures, FixtureLine, FixturePage};
    let pdfium = require_pdfium();
    let directory = tempfile::tempdir().unwrap();
    let source_bytes = fixtures::source_pdf(6);
    let source = directory.path().join("source.pdf");
    fs::write(&source, &source_bytes).unwrap();
    let package = fixtures::package_for_source(&source_bytes, 6);
    let root = directory.path().join("book.mdp");
    package.write_to(&root).unwrap();

    let titles = [("Ἀρχὴ τῆς σοφίας", 1u32), ("Second Chapter", 3)];
    let mut pages = vec![FixturePage::new({
        let mut lines = vec![FixtureLine::new("Contents", 40.0, 40.0)];
        for (ordinal, (title, printed)) in titles.iter().enumerate() {
            lines.push(
                FixtureLine::new(
                    &format!("{title} ....... {printed}"),
                    40.0,
                    100.0 + ordinal as f64 * 30.0,
                )
                .with_width(300.0),
            );
        }
        lines
    })];
    for index in 1..6u32 {
        let mut lines = Vec::new();
        if let Some((title, _)) = titles.iter().find(|(_, printed)| *printed == index) {
            lines.push(
                FixtureLine::new(title, 60.0, 70.0)
                    .with_height(20.0)
                    .with_width(320.0),
            );
        }
        lines.push(FixtureLine::new("body text", 60.0, 300.0));
        lines.push(FixtureLine::new(&format!("{index}"), 300.0, 760.0));
        pages.push(FixturePage::new(lines));
    }
    mpdf_core::ocr::write_ocr_records(&root, &fixtures::ocr_run(&pages, None)).unwrap();

    let output = directory.path().join("outlined.pdf");
    let (root_s, source_s, output_s) = (
        root.to_str().unwrap(),
        source.to_str().unwrap(),
        output.to_str().unwrap(),
    );
    let built = run(
        &[
            "bookmark", "auto", root_s, "--source", source_s, "--output", output_s, "--json",
        ],
        &pdfium,
    );
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&built.stdout).unwrap();
    assert_eq!(report["status"], "written");
    assert_eq!(report["mode"], "toc_aligned");
    assert_eq!(report["written_bookmarks"], 2);
    assert!(output.is_file());
    assert_eq!(fs::read(&source).unwrap(), source_bytes);

    // --overwrite and --regenerate are separate authorizations.
    let clobber = run(
        &[
            "bookmark",
            "auto",
            root_s,
            "--source",
            source_s,
            "--output",
            output_s,
            "--regenerate",
        ],
        &pdfium,
    );
    assert!(!clobber.status.success(), "an existing output is protected");
    let stale = run(
        &[
            "bookmark",
            "auto",
            root_s,
            "--source",
            source_s,
            "--output",
            output_s,
            "--overwrite",
        ],
        &pdfium,
    );
    assert!(
        !stale.status.success(),
        "existing candidates need --regenerate as well"
    );
    let both = run(
        &[
            "bookmark",
            "auto",
            root_s,
            "--source",
            source_s,
            "--output",
            output_s,
            "--overwrite",
            "--regenerate",
        ],
        &pdfium,
    );
    assert!(
        both.status.success(),
        "{}",
        String::from_utf8_lossy(&both.stderr)
    );
    let aliased = run(
        &[
            "bookmark",
            "auto",
            root_s,
            "--source",
            source_s,
            "--output",
            source_s,
            "--overwrite",
            "--regenerate",
        ],
        &pdfium,
    );
    assert!(!aliased.status.success(), "the source is never the output");
}
