//! Binary-level M4 smoke test. The fixture is assembled from public core
//! records, so this exercises the real CLI without PDFium or model downloads.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use mpdf_core::derived::{
    load_revisions, verify_bundle, BundleStatus, DerivedDocument, RevisionKind, RevisionRecord,
};
use mpdf_core::document_package::{
    AffineTransform, CoordinateSpace, CoordinateUnit, DocumentPackage, ExecutionKind, Manifest,
    Page, Source, SourceKind, ToolInfo, ValidationSummary, CANONICAL_MASTER_DPI, MDP_SCHEMA,
    MDP_SCHEMA_VERSION,
};
use mpdf_core::ocr::{
    OcrBlock, OcrBox, OcrLine, OcrPage, OcrProviderProvenance, OcrRoute, OcrRouteReason, OcrRun,
    OcrWord, OCR_PROTOCOL, OCR_PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};

fn mpdf() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mpdf"))
}
fn run(args: &[&str]) -> Output {
    mpdf().args(args).output().expect("mpdf binary")
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn page_id(source: &str, index: u32) -> String {
    format!("page-{}", digest(format!("{source}:{index}").as_bytes()))
}

fn fixture(root: &Path) -> (PathBuf, OcrRun) {
    let source_digest = digest(b"cli-derived-fixture");
    let pid = page_id(&source_digest, 0);
    let source_space = CoordinateSpace {
        id: "page-1-pdf".into(),
        unit: CoordinateUnit::PdfPoints,
        width: 100.0,
        height: 200.0,
        origin: mpdf_core::document_package::Origin::BottomLeft,
        pixels_per_inch: None,
    };
    let master_space = CoordinateSpace {
        id: "page-1-master".into(),
        unit: CoordinateUnit::Pixels,
        width: 100.0,
        height: 200.0,
        origin: mpdf_core::document_package::Origin::TopLeft,
        pixels_per_inch: Some(CANONICAL_MASTER_DPI),
    };
    let package = DocumentPackage {
        manifest: Manifest {
            schema: MDP_SCHEMA.into(),
            schema_version: MDP_SCHEMA_VERSION.into(),
            document_id: format!("doc-{source_digest}"),
            source_id: format!("source-{source_digest}"),
            page_count: 1,
            asset_count: 0,
            tool: ToolInfo {
                name: "mpdf".into(),
                version: "0.1".into(),
            },
        },
        source: Source {
            source_id: format!("source-{source_digest}"),
            kind: SourceKind::Pdf,
            content_sha256: source_digest,
            byte_len: 19,
            page_count: 1,
            external_reference: Some("fixture.pdf".into()),
            packaged_path: None,
        },
        pages: vec![Page {
            page_id: pid.clone(),
            physical_index: 0,
            order: 0,
            rotation_degrees: 0,
            master_space: master_space.clone(),
            source_space: source_space.clone(),
            transforms: vec![AffineTransform {
                from_space: source_space.id,
                to_space: master_space.id,
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: -1.0,
                e: 0.0,
                f: 200.0,
            }],
            printed_page_label: None,
            existing_outline_evidence: vec![],
            typography_evidence: vec![],
            region_evidence: vec![],
            asset_ids: vec![],
        }],
        assets: vec![],
        provenance: vec![mpdf_core::document_package::ProvenanceStep {
            step_id: "step-fixture".into(),
            operation: "fixture".into(),
            inputs: vec![format!("source-{}", digest(b"cli-derived-fixture"))],
            outputs: vec![pid],
            parameters: BTreeMap::new(),
            software: "mpdf-test".into(),
            software_version: "0.1".into(),
            execution: ExecutionKind::Local,
        }],
        validation: ValidationSummary {
            schema: MDP_SCHEMA.into(),
            schema_version: MDP_SCHEMA_VERSION.into(),
            valid: true,
            checked_pages: 1,
            checked_assets: 0,
            errors: vec![],
        },
    };
    package.validate().unwrap();
    let package_root = root.join("package.mdp");
    package.write_to(&package_root).unwrap();
    let word = OcrWord {
        text: "A <&>".into(),
        normalized_text: "A <&>".into(),
        bbox: OcrBox {
            x: 2.0,
            y: 3.0,
            width: 30.0,
            height: 12.0,
        },
        confidence: 0.99,
        reading_order: 0,
    };
    let line = OcrLine {
        bbox: word.bbox.clone(),
        confidence: 0.99,
        reading_order: 0,
        words: vec![word],
    };
    let page = OcrPage {
        page_index: 0,
        route: OcrRoute::Ocr {
            reason: OcrRouteReason::MissingText,
        },
        width: 100,
        height: 200,
        blocks: vec![OcrBlock {
            bbox: line.bbox.clone(),
            confidence: 0.99,
            reading_order: 0,
            lines: vec![line],
        }],
        revisions: vec![],
        provider_provenance: Some(OcrProviderProvenance {
            engine: "reference".into(),
            model: "fixture".into(),
            version: "0.1".into(),
            parameters: BTreeMap::new(),
            input_asset_sha256: "a".repeat(64),
            execution_location: "local".into(),
        }),
        provider_raw_artifact: Some("fixture-provider".into()),
    };
    let ocr = OcrRun {
        protocol: OCR_PROTOCOL.into(),
        protocol_version: OCR_PROTOCOL_VERSION.into(),
        pages: vec![page],
        errors: vec![],
    };
    mpdf_core::ocr::write_ocr_records(&package_root, &ocr).unwrap();
    (package_root, ocr)
}

#[test]
fn derived_cli_exports_revisions_and_verifies_bundle() {
    let temp = tempfile::tempdir().unwrap();
    let (package, ocr) = fixture(temp.path());
    let package_str = package.to_str().unwrap();
    let output_dir = temp.path().join("derived");
    let output_str = output_dir.to_str().unwrap();
    let first = run(&[
        "export",
        package_str,
        "--output",
        output_str,
        "--format",
        "all",
    ]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    for name in [
        "derived.json",
        "derived.jsonl",
        "derived.md",
        "derived.txt",
        "derived.html",
        "derived.hocr.html",
        "derived.alto.xml",
        "derived-manifest.json",
    ] {
        assert!(output_dir.join(name).is_file(), "missing {name}");
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(output_dir.join("derived-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["artifacts"].as_array().unwrap().len(), 7);
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(output_dir.join("derived.json")).unwrap()).unwrap();
    let word = &json["pages"][0]["blocks"][0]["lines"][0]["words"][0];
    let target = word["id"].as_str().unwrap();
    let page_digest = json["pages"][0]["evidence_digest"].as_str().unwrap();
    assert_eq!(word["source_text"], "A <&>");
    let add = run(&[
        "revision",
        "add",
        package_str,
        "--target-ref",
        target,
        "--base-evidence-digest",
        page_digest,
        "--text",
        "corrected",
    ]);
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let list = run(&["revision", "list", package_str]);
    assert!(list.status.success() && String::from_utf8_lossy(&list.stdout).contains(target));
    let human = run(&[
        "export",
        package_str,
        "--output",
        output_str,
        "--format",
        "all",
        "--overwrite",
    ]);
    assert!(human.status.success());
    assert!(
        String::from_utf8_lossy(&fs::read(output_dir.join("derived.md")).unwrap())
            .contains("corrected")
    );
    let ai = run(&[
        "revision",
        "add",
        package_str,
        "--target-ref",
        target,
        "--base-evidence-digest",
        page_digest,
        "--text",
        "ai-only",
        "--kind",
        "ai-suggested",
    ]);
    assert!(ai.status.success());
    let after_ai = run(&[
        "export",
        package_str,
        "--output",
        output_str,
        "--format",
        "all",
        "--overwrite",
    ]);
    assert!(after_ai.status.success());
    let markdown_bytes = fs::read(output_dir.join("derived.md")).unwrap();
    let markdown = String::from_utf8_lossy(&markdown_bytes);
    assert!(markdown.contains("corrected") && !markdown.contains("ai-only"));
    assert!(!run(&[
        "export",
        package_str,
        "--output",
        output_str,
        "--format",
        "all"
    ])
    .status
    .success());
    assert!(run(&[
        "export",
        package_str,
        "--output",
        output_str,
        "--format",
        "all",
        "--overwrite"
    ])
    .status
    .success());
    let revisions = load_revisions(&package).unwrap();
    let package_record = DocumentPackage::read_from(&package).unwrap();
    assert_eq!(
        verify_bundle(&output_dir, &package_record, Some(&ocr), &revisions).unwrap(),
        BundleStatus::Current
    );
    let mut stale_revisions = revisions.clone();
    stale_revisions.revisions.push(RevisionRecord {
        revision_id: "future-revision".into(),
        target_ref: target.into(),
        kind: RevisionKind::AiSuggested,
        text: "future".into(),
        base_evidence_digest: page_digest.into(),
    });
    assert_eq!(
        verify_bundle(&output_dir, &package_record, Some(&ocr), &stale_revisions).unwrap(),
        BundleStatus::Stale
    );
    let manifest_path = output_dir.join("derived-manifest.json");
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let mut incomplete_manifest: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).unwrap();
    incomplete_manifest["artifacts"]
        .as_array_mut()
        .unwrap()
        .pop();
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&incomplete_manifest).unwrap(),
    )
    .unwrap();
    assert_eq!(
        verify_bundle(&output_dir, &package_record, Some(&ocr), &revisions).unwrap(),
        BundleStatus::Corrupt
    );
    fs::write(&manifest_path, manifest_bytes).unwrap();
    fs::write(output_dir.join("derived.txt"), b"tampered").unwrap();
    assert_eq!(
        verify_bundle(&output_dir, &package_record, Some(&ocr), &revisions).unwrap(),
        BundleStatus::Corrupt
    );
    let summary = package.join("ocr/summary.json");
    let summary_bytes = fs::read(&summary).unwrap();
    fs::remove_file(&summary).unwrap();
    let partial = run(&[
        "export",
        package_str,
        "--output",
        temp.path().join("partial").to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(!partial.status.success());
    fs::write(summary, summary_bytes).unwrap();
    #[cfg(unix)]
    {
        let linked = temp.path().join("output-link");
        std::os::unix::fs::symlink(&output_dir, &linked).unwrap();
        assert!(!run(&[
            "export",
            package_str,
            "--output",
            linked.to_str().unwrap(),
            "--format",
            "all",
            "--overwrite"
        ])
        .status
        .success());
    }
    DerivedDocument::from_package(&package_record, Some(&ocr)).unwrap();
}

#[test]
fn derived_commands_have_stable_parser_surface() {
    for args in [
        &["export", "--help"][..],
        &["review", "--help"][..],
        &["revision", "add", "--help"][..],
        &["revision", "list", "--help"][..],
    ] {
        let output = run(args);
        assert!(output.status.success(), "{args:?}: {:?}", output.stderr);
        assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    }
}
