//! `mpdf bookmark generate` / `bookmark auto` behavior that needs no PDFium.
//!
//! The PDF write-back half of `bookmark auto` is covered by the PDFium
//! integration test in `bookmarks_cli.rs`; everything here exercises the
//! parts that must hold on any machine: schema 0.2 output, the structured
//! safe refusal, the separation of `--overwrite` from `--regenerate`, and
//! fail-closed handling of stale, partial, or aliased inputs.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use mpdf_core::bookmark_fixtures as fixtures;
use mpdf_core::document_package::{DocumentPackage, ExistingOutlineEvidence};
use mpdf_core::ocr::OcrRun;

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mpdf"))
        .args(arguments)
        .output()
        .unwrap()
}

fn json(output: &Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn write_mdp(root: &Path, package: &DocumentPackage, ocr: Option<&OcrRun>) {
    package.write_to(root).unwrap();
    if let Some(ocr) = ocr {
        mpdf_core::ocr::write_ocr_records(root, ocr).unwrap();
    }
}

fn aligned_mdp(root: &Path) {
    let (package, pages) = fixtures::aligned_book();
    write_mdp(root, &package, Some(&fixtures::ocr_run(&pages, None)));
}

#[test]
fn generate_writes_a_zero_two_snapshot_and_a_generation_report() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("book.mdp");
    aligned_mdp(&root);
    let path = root.to_str().unwrap();

    let snapshot = json(&run(&["bookmark", "generate", path, "--json"]));
    assert_eq!(snapshot["schema_version"], "0.2");
    assert_eq!(snapshot["generation_mode"], "toc_aligned");
    assert!(snapshot["rule_config_digest"].is_string());
    assert!(snapshot["ocr_digest"].is_string());
    let automatic = snapshot["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|candidate| candidate["status"] == "auto_confirmed")
        .count();
    assert_eq!(automatic, 3);

    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("bookmarks/generation-report.json")).unwrap())
            .unwrap();
    assert_eq!(report["schema"], "mpdf-bookmark-generation-report");
    assert_eq!(report["status"], "auto_confirmed");
    assert_eq!(report["generation_digest"], snapshot["generation_digest"]);

    // A second generation without authorization must not replace the first.
    let blocked = run(&["bookmark", "generate", path]);
    assert!(!blocked.status.success());
    assert!(run(&["bookmark", "generate", path, "--regenerate"])
        .status
        .success());
}

#[test]
fn a_regeneration_never_silently_discards_human_reviews() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("book.mdp");
    aligned_mdp(&root);
    let path = root.to_str().unwrap();
    let snapshot = json(&run(&["bookmark", "generate", path, "--json"]));
    let candidate = snapshot["candidates"][0]["candidate_id"].as_str().unwrap();
    assert!(run(&["bookmark", "reject", path, "--candidate", candidate])
        .status
        .success());

    let refused = run(&["bookmark", "generate", path, "--regenerate"]);
    assert!(!refused.status.success());
    let message = String::from_utf8_lossy(&refused.stderr);
    assert!(
        message.contains("review operation") && message.contains("reviews.json"),
        "the refusal must say what exists and where: {message}"
    );
    assert!(root.join("bookmarks/reviews.json").is_file());
}

#[test]
fn auto_returns_a_structured_safe_refusal_and_writes_no_output() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("book.mdp");
    let mut pages = vec![fixtures::FixturePage::new(vec![
        fixtures::FixtureLine::new("A Book Without Contents", 100.0, 100.0),
    ])];
    for index in 0..5 {
        pages.push(fixtures::FixturePage::new(vec![
            fixtures::FixtureLine::new(&format!("Page {index} body text"), 60.0, 300.0),
        ]));
    }
    let package = fixtures::package("cli-no-contents", pages.len() as u32);
    write_mdp(&root, &package, Some(&fixtures::ocr_run(&pages, None)));
    let source = directory.path().join("source.pdf");
    fs::write(&source, b"not the real source").unwrap();
    let output = directory.path().join("out.pdf");

    let result = json(&run(&[
        "bookmark",
        "auto",
        root.to_str().unwrap(),
        "--source",
        source.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--json",
    ]));
    assert_eq!(result["status"], "safe_refusal");
    assert_eq!(result["mode"], "safe_refusal");
    assert_eq!(result["written_bookmarks"], 0);
    assert!(result["output_path"].is_null());
    assert!(result["safe_refusal_reason"].is_string());
    assert!(
        !output.exists(),
        "a refusal must not create an output file, even an empty one"
    );
    // The refusal is a business result, not an internal failure.
    assert!(root.join("bookmarks/generation-report.json").is_file());
}

#[test]
fn auto_refuses_a_source_that_is_not_the_packaged_document() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("book.mdp");
    aligned_mdp(&root);
    let source = directory.path().join("source.pdf");
    fs::write(&source, b"a different document").unwrap();
    let output = directory.path().join("out.pdf");
    let failed = run(&[
        "bookmark",
        "auto",
        root.to_str().unwrap(),
        "--source",
        source.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("digest"));
    assert!(!output.exists());
}

#[test]
fn a_partial_or_corrupt_ocr_run_is_refused_rather_than_guessed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("partial.mdp");
    let (package, pages) = fixtures::aligned_book();
    let mut ocr = fixtures::ocr_run(&pages, None);
    ocr.pages.pop();
    write_mdp(&root, &package, None);
    // Write the truncated run directly: the package still declares every page.
    mpdf_core::ocr::write_ocr_records(&root, &ocr).unwrap();
    let failed = run(&["bookmark", "generate", root.to_str().unwrap()]);
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("complete"),
        "stderr was: {}",
        String::from_utf8_lossy(&failed.stderr)
    );

    let corrupt = directory.path().join("corrupt.mdp");
    let (package, pages) = fixtures::aligned_book();
    write_mdp(&corrupt, &package, Some(&fixtures::ocr_run(&pages, None)));
    fs::write(corrupt.join("ocr/summary.json"), b"{not json").unwrap();
    let failed = run(&["bookmark", "generate", corrupt.to_str().unwrap()]);
    assert!(!failed.status.success());
}

#[test]
fn list_and_review_work_for_an_automatically_generated_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("book.mdp");
    aligned_mdp(&root);
    let path = root.to_str().unwrap();
    let snapshot = json(&run(&["bookmark", "generate", path, "--json"]));
    let candidate = snapshot["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["status"] == "auto_confirmed")
        .unwrap()["candidate_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let text = run(&["bookmark", "list", path]);
    let listing = String::from_utf8_lossy(&text.stdout);
    assert!(
        listing.contains("auto_confirmed"),
        "an automatic entry is labelled distinctly from a human one: {listing}"
    );

    assert!(run(&[
        "bookmark",
        "edit",
        path,
        "--candidate",
        &candidate,
        "--title",
        "Human Title"
    ])
    .status
    .success());
    let effective = json(&run(&["bookmark", "list", path, "--json"]));
    let edited = effective
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["candidate_id"] == candidate.as_str())
        .unwrap();
    assert_eq!(edited["effective_title"], "Human Title");
    assert_eq!(
        edited["status"], "confirmed",
        "a human edit makes the entry a human decision"
    );
    assert!(
        edited["automatic_decision"]["decided_status"] == "auto_confirmed",
        "the automatic decision provenance is retained"
    );

    assert!(run(&["bookmark", "list", path, "--quiet"])
        .stdout
        .is_empty());
}

#[test]
fn an_existing_outline_package_generates_without_any_ocr() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("outline.mdp");
    let mut package = fixtures::package("cli-outline", 3);
    let target = package.pages[1].page_id.clone();
    package.pages[0].existing_outline_evidence = vec![ExistingOutlineEvidence {
        title: "Ἀρχή".into(),
        level: 0,
        target_page_id: Some(target),
        source: "source-pdf".into(),
    }];
    write_mdp(&root, &package, None);
    let snapshot = json(&run(&[
        "bookmark",
        "generate",
        root.to_str().unwrap(),
        "--json",
    ]));
    assert_eq!(snapshot["generation_mode"], "existing_outline");
    assert_eq!(snapshot["candidates"][0]["status"], "auto_confirmed");
    assert_eq!(snapshot["candidates"][0]["effective_title"], "Ἀρχή");
    assert!(snapshot["ocr_digest"].is_null());
}
