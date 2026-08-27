//! Evidence-backed, deterministic bookmark candidates and human review state.
//!
//! This module deliberately has no network/model dependency.  A candidate is
//! useful only when its evidence reference resolves to the package or derived
//! document that produced it; this makes generated navigation auditable and
//! prevents an attractive but unsupported title from entering a PDF outline.
mod model;
mod persistence;
mod review;
mod transform;
pub use model::*;
pub use persistence::*;
pub use review::*;
pub use transform::*;

use crate::derived::{Bbox, DerivedDocument, DerivedLine};
use crate::document_package::DocumentPackage;
use crate::error::{CoreError, Result};
use sha2::{Digest, Sha256};

pub const GENERATOR_KIND: &str = "deterministic_rules";
pub const GENERATOR_NAME: &str = "mpdf-bookmarks";
pub const GENERATOR_VERSION: &str = "0.1";

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
            || c.generator.version != GENERATOR_VERSION
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

/// Generate the immutable candidate snapshot. Inputs are validated before any
/// candidate is emitted; in particular, every candidate has a page and typed
/// evidence reference.
pub fn generate(
    package: &DocumentPackage,
    derived: Option<&DerivedDocument>,
) -> Result<BookmarkSnapshot> {
    package.validate()?;
    if let Some(d) = derived {
        d.validate()?;
        if d.manifest.source_digest != package.source.content_sha256 {
            return Err(CoreError::InvalidDocument(
                "derived source does not match package".into(),
            ));
        }
    }
    let mut candidates = Vec::new();
    // Existing outline records are the strongest evidence and preserve their
    // explicit hierarchy exactly.
    let mut outline_stack: Vec<(u16, String)> = Vec::new();
    for page in &package.pages {
        for (i, outline) in page.existing_outline_evidence.iter().enumerate() {
            let target = outline
                .target_page_id
                .clone()
                .unwrap_or_else(|| page.page_id.clone());
            let target_page = package
                .pages
                .iter()
                .find(|p| p.page_id == target)
                .ok_or_else(|| {
                    CoreError::InvalidDocument("outline target page is unresolved".into())
                })?;
            let title = outline.title.trim();
            if title.is_empty() {
                return Err(CoreError::InvalidDocument("outline title is empty".into()));
            }
            let ev = EvidenceRef::MdpOutline {
                page_id: page.page_id.clone(),
                ordinal: i as u32,
                source: outline.source.clone(),
            };
            while outline_stack
                .last()
                .is_some_and(|(l, _)| *l >= outline.level)
            {
                outline_stack.pop();
            }
            let parent = outline_stack.last().map(|(_, id)| id.clone());
            let mut c = candidate(
                package,
                title,
                outline.level,
                parent,
                target_page,
                None,
                vec![ev],
                1.0,
                vec!["existing_outline".into()],
                vec!["existing_outline".into()],
            );
            c.outline_evidence = Some(OutlineEvidence {
                title: title.into(),
                level: outline.level,
                target_page_id: outline.target_page_id.clone(),
                source: outline.source.clone(),
            });
            outline_stack.push((outline.level, c.candidate_id.clone()));
            candidates.push(c);
        }
    }
    // With no outline, use heading-like derived lines. Typography evidence is
    // required: plain OCR text is not silently promoted to a title.
    if candidates.is_empty() {
        if let Some(d) = derived {
            let mut repeated =
                std::collections::HashMap::<String, std::collections::HashSet<String>>::new();
            for page in &d.pages {
                for line in page.blocks.iter().flat_map(|b| b.lines.iter()) {
                    let text = line_text(line);
                    let page_height = page.bbox.height;
                    let header_footer = line.bbox.y <= page_height * 0.12
                        || line.bbox.y + line.bbox.height >= page_height * 0.88;
                    if !text.is_empty() && header_footer {
                        repeated
                            .entry(text.to_lowercase())
                            .or_default()
                            .insert(page.page_id.clone());
                    }
                }
            }
            // Table-of-contents lines are candidates only when the trailing
            // token maps to one unique printed label. The TOC line, TOC
            // region, and target label are all retained as evidence.
            for page in &d.pages {
                let source_page = package
                    .pages
                    .iter()
                    .find(|candidate| candidate.page_id == page.page_id)
                    .ok_or_else(|| {
                        CoreError::InvalidDocument("derived page is unresolved".into())
                    })?;
                for (region_ordinal, region) in source_page.region_evidence.iter().enumerate() {
                    if !is_toc_region(&region.kind) {
                        continue;
                    }
                    for line in page
                        .blocks
                        .iter()
                        .flat_map(|block| block.lines.iter())
                        .filter(|line| {
                            intersects(&region.bounds, line.bbox, &source_page.master_space.id)
                        })
                    {
                        let text = line_text(line);
                        let Some((title, label)) = split_toc_entry(&text) else {
                            continue;
                        };
                        let targets: Vec<_> = package
                            .pages
                            .iter()
                            .filter(|target| {
                                target
                                    .printed_page_label
                                    .as_ref()
                                    .is_some_and(|record| record.label == label)
                            })
                            .collect();
                        if targets.len() != 1 {
                            continue;
                        }
                        let target = targets[0];
                        let confidence = line
                            .words
                            .iter()
                            .map(|word| word.confidence)
                            .fold(1.0_f32, f32::min);
                        let (level, numbering_reason) = level_for_text(&title, source_page, line);
                        let mut candidate = candidate(
                            package,
                            &title,
                            level,
                            None,
                            target,
                            None,
                            vec![
                                EvidenceRef::DerivedLine {
                                    page_id: page.page_id.clone(),
                                    line_id: line.id.clone(),
                                    bbox: line.bbox,
                                },
                                EvidenceRef::MdpRegion {
                                    page_id: page.page_id.clone(),
                                    ordinal: region_ordinal as u32,
                                    bbox: region.bounds.clone(),
                                },
                                EvidenceRef::MdpPageLabel {
                                    page_id: target.page_id.clone(),
                                    label,
                                },
                            ],
                            confidence,
                            vec!["toc_exact_page_label".into(), numbering_reason],
                            vec!["toc_region".into(), "printed_page_label_exact".into()],
                        );
                        if confidence < 0.85 {
                            candidate.status = BookmarkStatus::NeedsReview;
                        }
                        candidates.push(candidate);
                    }
                }
            }
            for page in &d.pages {
                let Some(source_page) = package.pages.iter().find(|p| p.page_id == page.page_id)
                else {
                    return Err(CoreError::InvalidDocument(
                        "derived page is unresolved".into(),
                    ));
                };
                for line in page.blocks.iter().flat_map(|b| b.lines.iter()) {
                    let text = line_text(line);
                    if text.is_empty() {
                        continue;
                    }
                    let typography =
                        source_page
                            .typography_evidence
                            .iter()
                            .enumerate()
                            .find(|(_, evidence)| {
                                intersects(
                                    &evidence.bounds,
                                    line.bbox,
                                    &source_page.master_space.id,
                                )
                            });
                    let title_region =
                        source_page
                            .region_evidence
                            .iter()
                            .enumerate()
                            .find(|(_, evidence)| {
                                is_title_region(&evidence.kind)
                                    && intersects(
                                        &evidence.bounds,
                                        line.bbox,
                                        &source_page.master_space.id,
                                    )
                            });
                    if typography.is_none() && title_region.is_none() {
                        continue;
                    }
                    let mut confidence = line
                        .words
                        .iter()
                        .map(|w| w.confidence)
                        .fold(1.0_f32, f32::min);
                    let (level, reason) = level_for_text(&text, source_page, line);
                    let repeated_pages = repeated
                        .get(&text.to_lowercase())
                        .map_or(0, std::collections::HashSet::len);
                    let repeated_header =
                        repeated_pages >= 3 && repeated_pages * 5 >= package.pages.len().max(1) * 3;
                    if repeated_header {
                        confidence = confidence.min(0.40);
                    }
                    let status = if confidence < 0.70 || repeated_header {
                        BookmarkStatus::NeedsReview
                    } else {
                        BookmarkStatus::Proposed
                    };
                    let mut evidence = vec![EvidenceRef::DerivedLine {
                        page_id: page.page_id.clone(),
                        line_id: line.id.clone(),
                        bbox: line.bbox,
                    }];
                    if let Some((ordinal, record)) = typography {
                        evidence.push(EvidenceRef::MdpTypography {
                            page_id: page.page_id.clone(),
                            ordinal: ordinal as u32,
                            bbox: record.bounds.clone(),
                        });
                    }
                    if let Some((ordinal, record)) = title_region {
                        evidence.push(EvidenceRef::MdpRegion {
                            page_id: page.page_id.clone(),
                            ordinal: ordinal as u32,
                            bbox: record.bounds.clone(),
                        });
                    }
                    if let Some(label) = &source_page.printed_page_label {
                        evidence.push(EvidenceRef::MdpPageLabel {
                            page_id: page.page_id.clone(),
                            label: label.label.clone(),
                        });
                    }
                    let mut reasons = vec![reason];
                    if repeated_header {
                        reasons.push("repeated_header_footer_suppressed".into());
                    }
                    let mut c = candidate(
                        package,
                        &text,
                        level,
                        None,
                        source_page,
                        Some(line.bbox),
                        evidence,
                        confidence,
                        reasons,
                        vec!["typography".into(), "ocr_confidence".into()],
                    );
                    c.status = status;
                    candidates.push(c);
                }
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.physical_page_index
            .cmp(&b.physical_page_index)
            .then_with(|| {
                a.master_bbox
                    .map(|x| x.y)
                    .unwrap_or(0.0)
                    .total_cmp(&b.master_bbox.map(|x| x.y).unwrap_or(0.0))
            })
            .then_with(|| a.effective_level.cmp(&b.effective_level))
            .then_with(|| {
                a.source_title
                    .to_lowercase()
                    .cmp(&b.source_title.to_lowercase())
            })
            .then_with(|| a.candidate_id.cmp(&b.candidate_id))
    });
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|c| seen.insert(c.candidate_id.clone()));
    let mut hierarchy: Vec<(u16, String)> = Vec::new();
    for candidate in &mut candidates {
        if candidate.outline_evidence.is_some() || candidate.source_parent_id.is_some() {
            continue;
        }
        while hierarchy
            .last()
            .is_some_and(|(level, _)| *level >= candidate.source_level)
        {
            hierarchy.pop();
        }
        let parent = hierarchy.last().map(|(_, id)| id.clone());
        candidate.source_parent_id = parent.clone();
        candidate.effective_parent_id = parent;
        hierarchy.push((candidate.source_level, candidate.candidate_id.clone()));
    }
    let mut snapshot = BookmarkSnapshot {
        schema: BOOKMARK_SCHEMA.into(),
        schema_version: BOOKMARK_SCHEMA_VERSION.into(),
        source_digest: package.source.content_sha256.clone(),
        package_digest: digest(
            &serde_json::to_vec(package).map_err(|e| CoreError::InvalidDocument(e.to_string()))?,
        ),
        derived_digest: derived.map(|d| digest(&serde_json::to_vec(d).unwrap())),
        generator: GeneratorProvenance {
            kind: GENERATOR_KIND.into(),
            name: GENERATOR_NAME.into(),
            version: GENERATOR_VERSION.into(),
        },
        candidates,
        generation_digest: String::new(),
    };
    snapshot.generation_digest = snapshot.recomputed_generation_digest();
    Ok(snapshot)
}

#[allow(clippy::too_many_arguments)]
fn candidate(
    package: &DocumentPackage,
    title: &str,
    level: u16,
    parent: Option<String>,
    page: &crate::document_package::Page,
    bbox: Option<Bbox>,
    evidence: Vec<EvidenceRef>,
    confidence: f32,
    reasons: Vec<String>,
    trace: Vec<String>,
) -> BookmarkCandidate {
    let source_title = title.to_owned();
    let id = stable_id(
        &package.source.content_sha256,
        page.physical_index,
        title,
        bbox,
    );
    BookmarkCandidate {
        candidate_id: id,
        source_title,
        effective_title: title.to_owned(),
        source_level: level,
        effective_level: level,
        source_parent_id: parent.clone(),
        effective_parent_id: parent,
        target_page_id: page.page_id.clone(),
        physical_page_index: page.physical_index,
        master_bbox: bbox,
        outline_evidence: None,
        evidence,
        confidence,
        status: BookmarkStatus::Proposed,
        generator: GeneratorProvenance {
            kind: GENERATOR_KIND.into(),
            name: GENERATOR_NAME.into(),
            version: GENERATOR_VERSION.into(),
        },
        reason_codes: reasons,
        rule_trace: trace,
    }
}
fn stable_id(source: &str, page: u32, title: &str, bbox: Option<Bbox>) -> String {
    let mut h = Sha256::new();
    h.update(source);
    h.update(page.to_le_bytes());
    h.update(title.trim().as_bytes());
    if let Some(b) = bbox {
        h.update(format!(
            "{:.6},{:.6},{:.6},{:.6}",
            b.x, b.y, b.width, b.height
        ));
    }
    format!("bookmark-{}", hex(&h.finalize())[..32].to_owned())
}
fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn intersects(rect: &crate::document_package::Rect, bbox: Bbox, space: &str) -> bool {
    rect.space_id == space
        && rect.x < bbox.x + bbox.width
        && rect.x + rect.width > bbox.x
        && rect.y < bbox.y + bbox.height
        && rect.y + rect.height > bbox.y
}
fn line_text(line: &DerivedLine) -> String {
    line.words
        .iter()
        .map(|word| word.effective_text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
fn is_toc_region(kind: &str) -> bool {
    let normalized = kind.to_ascii_lowercase();
    normalized.contains("table_of_contents")
        || normalized == "toc"
        || normalized.contains("contents")
}
fn is_title_region(kind: &str) -> bool {
    let normalized = kind.to_ascii_lowercase();
    normalized.contains("title") || normalized.contains("heading")
}
fn split_toc_entry(text: &str) -> Option<(String, String)> {
    let mut tokens = text.split_whitespace().collect::<Vec<_>>();
    let label = tokens.pop()?.trim_matches(['.', '·']);
    if label.is_empty() || tokens.is_empty() {
        return None;
    }
    let title = tokens
        .join(" ")
        .trim_end_matches(['.', '·'])
        .trim()
        .to_owned();
    (!title.is_empty()).then(|| (title, label.to_owned()))
}
fn level_for_text(
    text: &str,
    page: &crate::document_package::Page,
    line: &DerivedLine,
) -> (u16, String) {
    let number_token = text.split_whitespace().next().unwrap_or_default();
    let normalized_number = number_token.trim_matches('.');
    let numbered = !normalized_number.is_empty()
        && normalized_number.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        });
    if numbered {
        return (
            normalized_number.split('.').count().clamp(1, 8) as u16,
            "numbering_pattern".into(),
        );
    }
    let size = page
        .typography_evidence
        .iter()
        .filter_map(|x| x.font_size_points)
        .fold(0.0, f64::max);
    if size >= 18.0 {
        (1, "large_typography".into())
    } else if size >= 14.0 {
        (2, "heading_typography".into())
    } else {
        let _ = line;
        (3, "heading_region".into())
    }
}
