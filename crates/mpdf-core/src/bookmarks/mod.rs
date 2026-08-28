//! Evidence-backed, deterministic bookmark candidates and human review state.
//!
//! This module deliberately has no network/model dependency.  A candidate is
//! useful only when its evidence reference resolves to the package or derived
//! document that produced it; this makes generated navigation auditable and
//! prevents an attractive but unsupported title from entering a PDF outline.
//!
//! Composition only lives here: detection, parsing, alignment, hierarchy, and
//! scoring each own one stage of the automatic table-of-contents pipeline
//! (see `docs/adr/0009-deterministic-automatic-toc-compilation.md`).
mod align;
mod assembly;
mod config;
mod engine;
mod hierarchy;
mod model;
mod persistence;
mod report;
mod review;
mod scoring;
mod text_index;
mod toc_detect;
mod toc_parse;
mod transform;
pub use assembly::*;
pub use config::*;
pub use engine::{
    generate_auto, generate_auto_with_cancel, AutoBookmarkInput, AutoBookmarkResult, MAX_SCORE,
};
pub use model::*;
pub use persistence::*;
pub use report::*;
pub use review::*;
pub use transform::*;

use crate::derived::DerivedDocument;
use crate::document_package::DocumentPackage;
use crate::error::{CoreError, Result};
use sha2::{Digest, Sha256};

pub const GENERATOR_KIND: &str = "deterministic_rules";
pub const GENERATOR_NAME: &str = "mpdf-bookmarks";
pub const GENERATOR_VERSION: &str = "0.1";
/// Generator versions a snapshot may legitimately carry: 0.1 records written
/// by M5 stay valid, and 0.2 records come from the automatic engine.
pub const GENERATOR_VERSIONS: [&str; 2] = [GENERATOR_VERSION, engine::GENERATOR_VERSION_V2];

/// Compatibility names used by front ends that distinguish a generation
/// snapshot from its persisted filename.
pub type BookmarkGeneration = BookmarkSnapshot;

pub fn generate_candidates(
    package: &DocumentPackage,
    derived: Option<&DerivedDocument>,
) -> Result<BookmarkSnapshot> {
    generate(package, derived)
}

pub fn effective_candidates(
    snapshot: &BookmarkSnapshot,
    reviews: &BookmarkReviews,
) -> Result<Vec<BookmarkCandidate>> {
    effective(snapshot, reviews)
}

/// Resolves every persisted reference against the package and (when present)
/// the derived document. This is intentionally separate from structural JSON
/// validation so callers that read an untrusted MDP can fail closed.
pub fn validate_against(
    snapshot: &BookmarkSnapshot,
    package: &DocumentPackage,
    derived: Option<&DerivedDocument>,
) -> Result<()> {
    snapshot.validate()?;
    package.validate()?;
    let pd = digest(
        &serde_json::to_vec(package).map_err(|e| CoreError::InvalidDocument(e.to_string()))?,
    );
    if snapshot.source_digest != package.source.content_sha256 || snapshot.package_digest != pd {
        return Err(CoreError::InvalidDocument(
            "bookmark snapshot is stale for package".into(),
        ));
    }
    if snapshot.derived_digest != derived.map(|d| digest(&serde_json::to_vec(d).unwrap())) {
        return Err(CoreError::InvalidDocument(
            "bookmark snapshot is stale for derived document".into(),
        ));
    }
    let derived_pages: std::collections::HashMap<_, _> = derived
        .map(|d| d.pages.iter().map(|p| (p.page_id.as_str(), p)).collect())
        .unwrap_or_default();
    for c in &snapshot.candidates {
        let Some(target_page) = package.pages.iter().find(|p| p.page_id == c.target_page_id) else {
            return Err(CoreError::InvalidDocument(
                "bookmark target page is unresolved".into(),
            ));
        };
        if target_page.physical_index != c.physical_page_index
            || c.reason_codes.len() > 32
            || c.rule_trace.len() > 128
            || c.reason_codes.iter().any(|x| x.is_empty() || x.len() > 128)
            || c.rule_trace.iter().any(|x| x.is_empty() || x.len() > 256)
            || c.generator.kind != GENERATOR_KIND
            || c.generator.name != GENERATOR_NAME
            || !GENERATOR_VERSIONS.contains(&c.generator.version.as_str())
        {
            return Err(CoreError::InvalidDocument(
                "bookmark candidate provenance or target is inconsistent".into(),
            ));
        }
        for e in &c.evidence {
            match e {
                EvidenceRef::MdpOutline {
                    page_id,
                    ordinal,
                    source,
                } => {
                    if package
                        .pages
                        .iter()
                        .find(|p| p.page_id == *page_id)
                        .and_then(|p| p.existing_outline_evidence.get(*ordinal as usize))
                        .is_none_or(|record| record.source != *source)
                    {
                        return Err(CoreError::InvalidDocument(
                            "MDP outline evidence is unresolved".into(),
                        ));
                    }
                }
                EvidenceRef::MdpPageLabel { page_id, label } => {
                    if package
                        .pages
                        .iter()
                        .find(|p| p.page_id == *page_id)
                        .and_then(|p| p.printed_page_label.as_ref())
                        .is_none_or(|record| record.label != *label)
                    {
                        return Err(CoreError::InvalidDocument(
                            "MDP page-label evidence is unresolved".into(),
                        ));
                    }
                }
                EvidenceRef::MdpTypography {
                    page_id,
                    ordinal,
                    bbox,
                } => {
                    if package
                        .pages
                        .iter()
                        .find(|p| p.page_id == *page_id)
                        .and_then(|p| p.typography_evidence.get(*ordinal as usize))
                        .is_none_or(|record| record.bounds != *bbox)
                    {
                        return Err(CoreError::InvalidDocument(
                            "MDP typography evidence is unresolved".into(),
                        ));
                    }
                }
                EvidenceRef::MdpRegion {
                    page_id,
                    ordinal,
                    bbox,
                } => {
                    if package
                        .pages
                        .iter()
                        .find(|p| p.page_id == *page_id)
                        .and_then(|p| p.region_evidence.get(*ordinal as usize))
                        .is_none_or(|record| record.bounds != *bbox)
                    {
                        return Err(CoreError::InvalidDocument(
                            "MDP region evidence is unresolved".into(),
                        ));
                    }
                }
                EvidenceRef::DerivedPage { page_id, bbox } => {
                    if derived_pages
                        .get(page_id.as_str())
                        .is_none_or(|page| page.bbox != *bbox)
                    {
                        return Err(CoreError::InvalidDocument(
                            "derived page evidence is unresolved".into(),
                        ));
                    }
                }
                EvidenceRef::DerivedLine {
                    page_id,
                    line_id,
                    bbox,
                } => {
                    if derived_pages.get(page_id.as_str()).is_none_or(|p| {
                        !p.blocks
                            .iter()
                            .flat_map(|b| b.lines.iter())
                            .any(|l| l.id == *line_id && l.bbox == *bbox)
                    }) {
                        return Err(CoreError::InvalidDocument(
                            "derived line evidence is unresolved".into(),
                        ));
                    }
                }
                EvidenceRef::DerivedWord {
                    page_id,
                    word_id,
                    bbox,
                } => {
                    if derived_pages.get(page_id.as_str()).is_none_or(|p| {
                        !p.blocks
                            .iter()
                            .flat_map(|b| b.lines.iter())
                            .flat_map(|l| l.words.iter())
                            .any(|w| w.id == *word_id && w.bbox == *bbox)
                    }) {
                        return Err(CoreError::InvalidDocument(
                            "derived word evidence is unresolved".into(),
                        ));
                    }
                }
            }
        }
    }
    for c in &snapshot.candidates {
        if let Some(parent) = &c.effective_parent_id {
            let p = snapshot
                .candidates
                .iter()
                .find(|x| x.candidate_id == *parent)
                .ok_or_else(|| {
                    CoreError::InvalidDocument("bookmark parent is unresolved".into())
                })?;
            if p.effective_level >= c.effective_level || p.candidate_id == c.candidate_id {
                return Err(CoreError::InvalidDocument(
                    "bookmark parent level is inconsistent".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Generate a snapshot from a package and optional derived document.
///
/// This is the compatibility entry point used by callers that hold no OCR
/// run. It runs the same v2 engine as [`generate_auto`]; without OCR
/// evidence only the `existing_outline` mode can succeed, and any other
/// document produces an explained safe refusal rather than a guessed
/// outline. There is no second, simplified generator.
pub fn generate(
    package: &DocumentPackage,
    derived: Option<&DerivedDocument>,
) -> Result<BookmarkSnapshot> {
    Ok(generate_auto(
        &AutoBookmarkInput {
            package,
            ocr: None,
            derived,
        },
        &AutoBookmarkConfig::default(),
    )?
    .snapshot)
}

pub(crate) fn digest_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn digest(bytes: &[u8]) -> String {
    digest_bytes(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
