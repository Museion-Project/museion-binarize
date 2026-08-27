use super::*;
use crate::document_package::{PrintedLabelSource, PrintedPageLabel};

fn doc() -> DerivedDocument {
    DerivedDocument {
        manifest: DerivedManifest {
            schema: DERIVED_SCHEMA.into(),
            schema_version: DERIVED_SCHEMA_VERSION.into(),
            source_digest: "a".repeat(64),
            document_id: "doc-1".into(),
            package_digest: "b".repeat(64),
            ocr_digest: None,
            revision_digest: "c".repeat(64),
            exporter_version: DERIVED_EXPORTER_VERSION.into(),
            artifacts: vec![],
        },
        pages: vec![DerivedPage {
            page_id: "page-1".into(),
            page_index: 0,
            bbox: Bbox {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 100.,
            },
            coordinate_space: "master".into(),
            evidence_digest: "d".repeat(64),
            blocks: vec![],
            regions: vec![],
            outline_evidence: vec![],
            printed_page_label: Some(PrintedPageLabel {
                label: "i".into(),
                source: PrintedLabelSource::Observed,
            }),
            existing_outline_evidence: vec![],
            typography_evidence: vec![],
            region_evidence: vec![],
        }],
        chunks: vec![],
    }
}
fn word_doc() -> DerivedDocument {
    let mut d = doc();
    let w = DerivedWord {
        id: "word-1".into(),
        page_id: "page-1".into(),
        bbox: Bbox {
            x: 1.,
            y: 1.,
            width: 20.,
            height: 10.,
        },
        coordinate_space: "master".into(),
        structural_path: "p/page-1/b000000/l000000/w000000".into(),
        source_text: "A\u{301} <&>".into(),
        source_normalized_text: "Á <&>".into(),
        effective_text: "A\u{301} <&>".into(),
        effective_normalized_text: "Á <&>".into(),
        text: "A\u{301} <&>".into(),
        normalized_text: "Á <&>".into(),
        confidence: 0.5,
        reading_order: 0,
    };
    d.pages[0].blocks = vec![DerivedBlock {
        id: "block-1".into(),
        page_id: "page-1".into(),
        bbox: Bbox {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 20.,
        },
        coordinate_space: "master".into(),
        structural_path: "p/page-1/b000000".into(),
        reading_order: 0,
        lines: vec![DerivedLine {
            id: "line-1".into(),
            page_id: "page-1".into(),
            bbox: Bbox {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 20.,
            },
            coordinate_space: "master".into(),
            structural_path: "p/page-1/b000000/l000000".into(),
            reading_order: 0,
            words: vec![w],
        }],
    }];
    d
}
#[test]
fn all_export_formats_are_deterministic_and_escaped() {
    let d = word_doc();
    for f in [
        ExportFormat::Json,
        ExportFormat::Jsonl,
        ExportFormat::Markdown,
        ExportFormat::Text,
        ExportFormat::Html,
        ExportFormat::Hocr,
        ExportFormat::Alto,
    ] {
        let a = export(&d, f).unwrap();
        assert_eq!(a, export(&d, f).unwrap());
        if matches!(
            f,
            ExportFormat::Html | ExportFormat::Hocr | ExportFormat::Alto
        ) {
            assert!(a.contains("&lt;") && a.contains("&amp;"));
        }
    }
}
#[test]
fn hocr_and_alto_have_typed_nodes() {
    let d = word_doc();
    let h = export(&d, ExportFormat::Hocr).unwrap();
    assert!(
        h.contains("ocr_page")
            && h.contains("ocr_carea")
            && h.contains("ocr_line")
            && h.contains("ocrx_word")
            && h.contains("x_wconf")
    );
    let a = export(&d, ExportFormat::Alto).unwrap();
    assert!(
        a.contains("<Page")
            && a.contains("WIDTH=")
            && a.contains("<TextBlock")
            && a.contains("<TextLine")
            && a.contains("<String")
    );
}
#[test]
fn human_revision_changes_effective_exports_only() {
    let mut d = word_doc();
    let r = RevisionStore {
        schema: "mpdf-revisions".into(),
        schema_version: "0.1".into(),
        revisions: vec![RevisionRecord {
            revision_id: "r".into(),
            target_ref: "word-1".into(),
            kind: RevisionKind::Human,
            text: "corrected".into(),
            base_evidence_digest: "d".repeat(64),
        }],
    };
    d.apply_revisions(&r).unwrap();
    assert_eq!(
        d.pages[0].blocks[0].lines[0].words[0].source_text,
        "A\u{301} <&>"
    );
    assert!(export(&d, ExportFormat::Markdown)
        .unwrap()
        .contains("corrected"));
    assert_eq!(d.chunks[0].text, "corrected");
    assert_eq!(d.chunks[0].document_id, "doc-1");
}
#[test]
fn review_queue_is_typed_and_references_word() {
    let q = review_queue(&word_doc()).unwrap();
    assert!(q.iter().any(|x| x.kind == ReviewIssueKind::LowConfidence
        && x.target_ref == "word-1"
        && x.confidence == Some(0.5)));
}
#[test]
fn deterministic_revision_ids_are_stable_and_distinct() {
    let b = "a".repeat(64);
    assert_ne!(
        deterministic_revision_id("word", &b, RevisionKind::Human, "a"),
        deterministic_revision_id("word", &b, RevisionKind::Human, "b")
    );
}

#[test]
fn serialized_top_level_keys_match_schema_contract() {
    let value = serde_json::to_value(doc()).unwrap();
    let object = value.as_object().unwrap();
    let mut keys: Vec<_> = object.keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["chunks", "manifest", "pages"]);
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../schemas/mpdf-derived-0.1.schema.json"
    ))
    .unwrap();
    let required = schema["required"].as_array().unwrap();
    for key in required {
        assert!(object.contains_key(key.as_str().unwrap()));
    }
}
