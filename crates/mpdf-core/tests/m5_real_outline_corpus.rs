use std::fs;
use std::path::{Component, Path, PathBuf};

use mpdf_core::bookmarks::{self, BookmarkStatus, EvidenceRef};
use mpdf_core::document_package::DocumentPackage;
use mpdf_core::document_session::{PdfDocumentSession, PdfOpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct CorpusManifest {
    schema: String,
    schema_version: String,
    title: String,
    documents: Vec<CorpusDocument>,
}

#[derive(Deserialize)]
struct CorpusDocument {
    id: String,
    path: String,
    sha256: String,
    page_count: u32,
    outline_count: usize,
    outline_semantic_sha256: String,
    #[serde(default)]
    expected_error: Option<String>,
}

#[derive(Serialize)]
struct CorpusResult {
    schema: &'static str,
    schema_version: &'static str,
    corpus_title: String,
    documents: usize,
    accepted_documents: usize,
    rejected_documents: usize,
    pages: u64,
    outline_entries: usize,
    exact_titles: usize,
    exact_levels: usize,
    exact_target_pages: usize,
    unresolved_evidence: usize,
    automatically_confirmed: usize,
    deterministic_regenerations: usize,
}

fn manifest() -> CorpusManifest {
    serde_json::from_str(include_str!(
        "../../../test-data/benchmark/m5-real-outline-v1/manifest.json"
    ))
    .expect("real corpus manifest must parse")
}

fn checked_source(root: &Path, relative: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("corpus path is not a safe relative path".into());
    }
    let path = root.join(relative);
    if fs::symlink_metadata(&path)?.file_type().is_symlink() {
        return Err("corpus source must not be a symlink".into());
    }
    let canonical_root = root.canonicalize()?;
    let canonical_path = path.canonicalize()?;
    if !canonical_path.starts_with(canonical_root) {
        return Err("corpus source escapes the configured root".into());
    }
    Ok(canonical_path)
}

#[test]
fn translation_agent_2_twenty_pdf_native_outline_corpus() -> Result<(), Box<dyn std::error::Error>>
{
    let corpus = manifest();
    assert_eq!(corpus.schema, "mpdf-m5-real-outline-corpus");
    assert_eq!(corpus.schema_version, "1.0");
    assert_eq!(corpus.documents.len(), 20);

    let Some(root) = std::env::var_os("MPDF_M5_REAL_CORPUS_ROOT") else {
        return Ok(());
    };
    let Some(library) = std::env::var_os("MPDF_PDFIUM_LIBRARY") else {
        return Err("MPDF_PDFIUM_LIBRARY is required for the real corpus".into());
    };
    let root = PathBuf::from(root);
    let options = PdfOpenOptions {
        compute_source_hash: true,
        pdfium: mpdf_core::pdfium_backend::PdfiumConfig {
            library_path: Some(library.into()),
            allow_system_library: false,
        },
        password: None,
    };

    let mut result = CorpusResult {
        schema: "mpdf-m5-real-outline-result",
        schema_version: "1.0",
        corpus_title: corpus.title,
        documents: 0,
        accepted_documents: 0,
        rejected_documents: 0,
        pages: 0,
        outline_entries: 0,
        exact_titles: 0,
        exact_levels: 0,
        exact_target_pages: 0,
        unresolved_evidence: 0,
        automatically_confirmed: 0,
        deterministic_regenerations: 0,
    };

    for document in corpus.documents {
        let path = checked_source(&root, &document.path)?;
        let bytes = fs::read(&path)?;
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), document.sha256);

        let session = PdfDocumentSession::open(&path, &options)?;
        assert_eq!(
            session.info().page_count,
            document.page_count,
            "{}",
            document.id
        );
        result.documents += 1;
        result.pages += u64::from(document.page_count);
        let source_outline = match session.native_outline() {
            Ok(outline) if document.expected_error.is_none() => outline,
            Ok(_) => {
                return Err(format!("{}: expected native outline rejection", document.id).into())
            }
            Err(error) => {
                let rendered = error.to_string();
                let Some(expected) = &document.expected_error else {
                    return Err(std::io::Error::other(format!(
                        "{}: native outline: {error}",
                        document.id
                    ))
                    .into());
                };
                assert!(
                    rendered.contains(expected),
                    "{}: unexpected rejection: {rendered}",
                    document.id
                );
                result.rejected_documents += 1;
                continue;
            }
        };
        assert_eq!(
            source_outline.len(),
            document.outline_count,
            "{}",
            document.id
        );
        let canonical_outline = source_outline
            .iter()
            .map(|item| {
                let semantic_title = item.title.trim_end_matches(|character: char| {
                    character == '\0' || character.is_whitespace()
                });
                serde_json::to_string(&(item.level, item.page_index, semantic_title))
            })
            .collect::<Result<Vec<_>, _>>()?
            .join("\n")
            + "\n";
        let outline_sha256 = format!("{:x}", Sha256::digest(canonical_outline.as_bytes()));
        assert_eq!(
            outline_sha256, document.outline_semantic_sha256,
            "{}: PDFium outline differs from the independent qpdf semantic digest",
            document.id
        );

        let package = DocumentPackage::create_from_session(&session, Some(document.id.clone()))
            .map_err(|error| {
                std::io::Error::other(format!("{}: package creation: {error}", document.id))
            })?;
        let evidence = &package.pages[0].existing_outline_evidence;
        assert_eq!(evidence.len(), source_outline.len(), "{}", document.id);
        let snapshot = bookmarks::generate(&package, None)?;
        assert_eq!(
            snapshot.candidates.len(),
            source_outline.len(),
            "{}",
            document.id
        );
        assert_eq!(
            snapshot,
            bookmarks::generate(&package, None)?,
            "{}",
            document.id
        );

        for ((truth, stored), candidate) in source_outline
            .iter()
            .zip(evidence)
            .zip(&snapshot.candidates)
        {
            let target_page_id = package.pages[truth.page_index as usize].page_id.as_str();
            assert_eq!(stored.title, truth.title, "{}", document.id);
            assert_eq!(stored.level, truth.level, "{}", document.id);
            assert_eq!(
                stored.target_page_id.as_deref(),
                Some(target_page_id),
                "{}",
                document.id
            );
            assert_eq!(candidate.source_title, truth.title, "{}", document.id);
            assert_eq!(candidate.source_level, truth.level, "{}", document.id);
            assert_eq!(
                candidate.physical_page_index, truth.page_index,
                "{}",
                document.id
            );
            assert_eq!(
                candidate.status,
                BookmarkStatus::Proposed,
                "{}",
                document.id
            );
            assert!(candidate.evidence.iter().any(|item| matches!(item, EvidenceRef::MdpOutline { source, .. } if source == "source-pdf")));
            result.exact_titles += 1;
            result.exact_levels += 1;
            result.exact_target_pages += 1;
        }

        result.accepted_documents += 1;
        result.outline_entries += source_outline.len();
        result.automatically_confirmed += snapshot
            .candidates
            .iter()
            .filter(|candidate| candidate.status == BookmarkStatus::Confirmed)
            .count();
        result.deterministic_regenerations += 1;
    }

    let json = serde_json::to_string_pretty(&result)?;
    println!("{json}");
    if let Some(report) = std::env::var_os("MPDF_M5_REAL_REPORT") {
        fs::write(report, format!("{json}\n"))?;
    }
    Ok(())
}
