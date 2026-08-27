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

#[test]
fn m5_cli_bookmark_review_and_searchable_pdf_binary_e2e() {
    let Some(pdfium) = std::env::var_os("MPDF_PDFIUM_LIBRARY") else {
        return;
    };
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
