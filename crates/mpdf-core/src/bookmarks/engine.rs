//! The deterministic automatic bookmark engine.
//!
//! Three modes, in strict priority order:
//!
//! 1. `existing_outline` — a validated native PDF outline is preserved
//!    exactly and never mixed with inferred entries;
//! 2. `toc_aligned` — a printed contents list is detected, parsed, aligned
//!    against body headings and printed page labels, and only fully
//!    supported entries are confirmed automatically;
//! 3. `safe_refusal` — nothing reliable was found. No title is invented and
//!    no bookmark is written.
//!
//! The engine is provider-neutral: it consumes typed OCR records regardless
//! of whether they were produced locally (M3) or installed from a consented
//! API result (M6), and it never reads a raw provider artifact.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::derived::{Bbox, DerivedDocument};
use crate::document_package::{DocumentPackage, Page};
use crate::error::{CoreError, Result};
use crate::ocr::{OcrRoute, OcrRun};

use super::align::{self, EntryAlignment};
use super::config::{AutoBookmarkConfig, SCORE_NUMBERING_MAX, SCORE_TOTAL};
use super::hierarchy;
use super::model::*;
use super::report::*;
use super::scoring::{self, GateContext};
use super::text_index::{GeometryQuality, TextIndex};
use super::toc_detect;
use super::toc_parse::{self, TocEntryDraft};
use super::{digest_bytes, GENERATOR_KIND, GENERATOR_NAME};

/// The generator version advertised by the v2 engine.
pub const GENERATOR_VERSION_V2: &str = "0.2";

/// Provider-neutral engine input. Disk access, CLI arguments, and IPC
/// payloads never reach the algorithm.
pub struct AutoBookmarkInput<'a> {
    pub package: &'a DocumentPackage,
    pub ocr: Option<&'a OcrRun>,
    pub derived: Option<&'a DerivedDocument>,
}

pub struct AutoBookmarkResult {
    pub snapshot: BookmarkSnapshot,
    pub report: BookmarkGenerationReport,
}

impl AutoBookmarkResult {
    pub fn auto_confirmed(&self) -> usize {
        self.snapshot
            .candidates
            .iter()
            .filter(|candidate| candidate.status == BookmarkStatus::AutoConfirmed)
            .count()
    }
}

/// Maximum number of review-only heading proposals emitted when a document
/// has no printed contents list at all.
const MAX_HEADING_REVIEW_CANDIDATES: usize = 100;

pub fn generate_auto(
    input: &AutoBookmarkInput<'_>,
    config: &AutoBookmarkConfig,
) -> Result<AutoBookmarkResult> {
    generate_auto_with_cancel(input, config, &|| false)
}

pub fn generate_auto_with_cancel(
    input: &AutoBookmarkInput<'_>,
    config: &AutoBookmarkConfig,
    cancelled: &dyn Fn() -> bool,
) -> Result<AutoBookmarkResult> {
    let package = input.package;
    package.validate()?;
    if let Some(ocr) = input.ocr {
        ocr.validate()
            .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        super::text_index::bind_ocr_to_package(package, ocr)?;
    }
    if let Some(derived) = input.derived {
        derived.validate()?;
        if derived.manifest.source_digest != package.source.content_sha256 {
            return Err(CoreError::InvalidDocument(
                "derived source does not match package".into(),
            ));
        }
        if let Some(ocr) = input.ocr {
            let ocr_digest = digest_bytes(
                &serde_json::to_vec(ocr).map_err(|e| CoreError::InvalidDocument(e.to_string()))?,
            );
            if derived.manifest.ocr_digest.as_deref() != Some(ocr_digest.as_str()) {
                return Err(CoreError::InvalidDocument(
                    "derived document was not built from this OCR run".into(),
                ));
            }
        }
    }
    if cancelled() {
        return Err(CoreError::Cancelled);
    }

    let package_digest = digest_bytes(
        &serde_json::to_vec(package).map_err(|e| CoreError::InvalidDocument(e.to_string()))?,
    );
    let derived_digest = input
        .derived
        .map(|derived| -> Result<String> {
            Ok(digest_bytes(
                &serde_json::to_vec(derived)
                    .map_err(|e| CoreError::InvalidDocument(e.to_string()))?,
            ))
        })
        .transpose()?;
    let ocr_digest = input
        .ocr
        .map(|ocr| -> Result<String> {
            Ok(digest_bytes(
                &serde_json::to_vec(ocr).map_err(|e| CoreError::InvalidDocument(e.to_string()))?,
            ))
        })
        .transpose()?;
    let revision_digest = input
        .derived
        .map(|derived| derived.manifest.revision_digest.clone());
    let rule_config_digest = config.digest();

    let mut builder = ReportBuilder::new(
        package.source.content_sha256.clone(),
        package_digest.clone(),
        ocr_digest.clone(),
        derived_digest.clone(),
        revision_digest.clone(),
        rule_config_digest.clone(),
        config.rule_version.clone(),
    );
    builder.ocr_provenance = provenance_summary(input.ocr);

    let has_outline = package
        .pages
        .iter()
        .any(|page| !page.existing_outline_evidence.is_empty());
    let (mode, candidates) = if has_outline {
        (
            GenerationMode::ExistingOutline,
            existing_outline_candidates(package, config, &rule_config_digest)?,
        )
    } else {
        match (input.ocr, input.derived) {
            (Some(ocr), Some(derived)) => compile_printed_contents(
                package,
                ocr,
                derived,
                config,
                &rule_config_digest,
                &mut builder,
                cancelled,
            )?,
            _ => {
                builder.safe_refusal_reason = Some(
                    "no validated native outline and no complete OCR evidence for this document"
                        .to_owned(),
                );
                (GenerationMode::SafeRefusal, Vec::new())
            }
        }
    };

    let mut snapshot = BookmarkSnapshot {
        schema: BOOKMARK_SCHEMA.into(),
        schema_version: BOOKMARK_SCHEMA_VERSION_V2.into(),
        source_digest: package.source.content_sha256.clone(),
        package_digest,
        derived_digest,
        generator: GeneratorProvenance {
            kind: GENERATOR_KIND.into(),
            name: GENERATOR_NAME.into(),
            version: GENERATOR_VERSION_V2.into(),
        },
        candidates,
        generation_digest: String::new(),
        ocr_digest: if mode == GenerationMode::TocAligned {
            ocr_digest
        } else {
            None
        },
        revision_digest: if mode == GenerationMode::TocAligned {
            revision_digest
        } else {
            None
        },
        rule_config_digest: Some(rule_config_digest),
        rule_version: Some(config.rule_version.clone()),
        generation_mode: Some(mode),
    };
    sort_candidates(package, &mut snapshot.candidates);
    snapshot.generation_digest = snapshot.recomputed_generation_digest();
    snapshot.validate()?;
    let report = builder.finish(mode, &snapshot);
    Ok(AutoBookmarkResult { snapshot, report })
}

// ---------------------------------------------------------------------------
// Existing outline
// ---------------------------------------------------------------------------

fn existing_outline_candidates(
    package: &DocumentPackage,
    config: &AutoBookmarkConfig,
    rule_config_digest: &str,
) -> Result<Vec<BookmarkCandidate>> {
    let mut candidates = Vec::new();
    let mut stack: Vec<(u16, String)> = Vec::new();
    for page in &package.pages {
        for (ordinal, outline) in page.existing_outline_evidence.iter().enumerate() {
            let target_id = outline
                .target_page_id
                .clone()
                .unwrap_or_else(|| page.page_id.clone());
            let target = package
                .pages
                .iter()
                .find(|candidate| candidate.page_id == target_id)
                .ok_or_else(|| {
                    CoreError::InvalidDocument("outline target page is unresolved".into())
                })?;
            let title = outline.title.trim();
            if title.is_empty() {
                return Err(CoreError::InvalidDocument("outline title is empty".into()));
            }
            if outline.title.len() > MAX_TITLE_BYTES {
                return Err(CoreError::InvalidDocument(
                    "outline title exceeds its byte limit".into(),
                ));
            }
            if outline.level > 64 {
                return Err(CoreError::InvalidDocument(
                    "outline level exceeds the schema limit".into(),
                ));
            }
            while stack
                .last()
                .is_some_and(|(level, _)| *level >= outline.level)
            {
                stack.pop();
            }
            let parent = stack.last().map(|(_, id)| id.clone());
            let breakdown = scoring::breakdown(4_000, 2_000, 1_000, 1_000, 1_000, 1_000);
            let mut candidate = base_candidate(
                stable_outline_id(
                    &package.source.content_sha256,
                    &page.page_id,
                    ordinal as u32,
                    &target.page_id,
                    &outline.title,
                ),
                &outline.title,
                outline.level,
                parent,
                target,
                None,
                vec![EvidenceRef::MdpOutline {
                    page_id: page.page_id.clone(),
                    ordinal: ordinal as u32,
                    source: outline.source.clone(),
                }],
                breakdown,
                vec!["validated_existing_outline".into()],
                vec!["existing_outline".into()],
            );
            candidate.status = BookmarkStatus::AutoConfirmed;
            candidate.outline_evidence = Some(OutlineEvidence {
                title: outline.title.clone(),
                level: outline.level,
                target_page_id: outline.target_page_id.clone(),
                source: outline.source.clone(),
            });
            candidate.automatic_decision = Some(AutomaticDecision {
                decided_status: "auto_confirmed".into(),
                reason: "validated_existing_outline".into(),
                rule_version: config.rule_version.clone(),
                rule_config_digest: rule_config_digest.to_owned(),
            });
            stack.push((outline.level, candidate.candidate_id.clone()));
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

// ---------------------------------------------------------------------------
// Printed contents compilation
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn compile_printed_contents(
    package: &DocumentPackage,
    ocr: &OcrRun,
    derived: &DerivedDocument,
    config: &AutoBookmarkConfig,
    rule_config_digest: &str,
    builder: &mut ReportBuilder,
    cancelled: &dyn Fn() -> bool,
) -> Result<(GenerationMode, Vec<BookmarkCandidate>)> {
    let mut index = TextIndex::build(package, ocr, derived, config, cancelled)?;
    let (front_limit, detections) = toc_detect::detect(&index, config, cancelled)?;
    builder.front_page_limit = front_limit;
    builder.scanned_front_pages = index
        .pages
        .iter()
        .filter(|page| page.page_index < front_limit)
        .count() as u32;

    if detections.is_empty() {
        builder.safe_refusal_reason = Some(
            "no printed table of contents was detected in the document's front matter".to_owned(),
        );
        let review = heading_review_candidates(package, &index, config, rule_config_digest);
        for candidate in &review {
            builder.count(candidate);
        }
        return Ok((GenerationMode::SafeRefusal, review));
    }

    let toc_pages: Vec<u32> = detections.iter().map(|page| page.page_index).collect();
    let mut drafts: Vec<TocEntryDraft> = Vec::new();
    for detection in &detections {
        if cancelled() {
            return Err(CoreError::Cancelled);
        }
        let page = index
            .pages
            .iter()
            .find(|page| page.page_index == detection.page_index)
            .ok_or_else(|| CoreError::InvalidDocument("detected TOC page is missing".into()))?;
        let parsed = toc_parse::parse_toc_page(page, config);
        builder.toc_pages.push(TocPageReport {
            page_id: detection.page_id.clone(),
            page_index: detection.page_index,
            score: detection.score,
            signals: detection.signals.clone(),
            parsed_entries: parsed.len() as u32,
            keyword_line_ids: detection.keyword_line_ids.clone(),
            entry_line_ids: detection.entry_line_ids.iter().take(64).cloned().collect(),
        });
        drafts.extend(parsed);
    }
    if drafts.len() > config.max_toc_entries as usize {
        drafts.truncate(config.max_toc_entries as usize);
        builder.truncated = true;
        builder
            .truncation_reasons
            .push("max_toc_entries".to_owned());
    }
    builder.parsed_entries = drafts.len() as u32;
    if drafts.is_empty() {
        builder.safe_refusal_reason = Some(
            "a contents page was detected but no contents entry could be parsed from it".to_owned(),
        );
        return Ok((GenerationMode::SafeRefusal, Vec::new()));
    }

    index.index_body(&toc_pages);
    let mut aligned = align::shortlist(&mut index, drafts, config, cancelled)?;
    if aligned.iter().any(|entry| entry.capped_shortlist) {
        builder.truncated = true;
        builder
            .truncation_reasons
            .push("shortlist_ranked_tail_capped".to_owned());
    }
    if index.truncated_postings {
        builder.truncated = true;
        builder
            .truncation_reasons
            .push("shortlist_recall_truncated".to_owned());
    }
    builder.shortlist_postings_visited = index.visited_postings;
    builder.body_lines_indexed = index.body_line_count() as u64;
    let mapping = align::solve_mapping(&aligned, config);
    for segment in &mapping.segments {
        builder.mapping_segments.push(MappingSegmentReport {
            numbering_family: segment.family.as_str().to_owned(),
            segment_index: segment.index,
            offset: segment.offset,
            anchor_count: segment.anchor_count,
            first_printed_number: segment.first_printed,
            last_printed_number: segment.last_printed,
            residual_min: segment.residual_min,
            residual_max: segment.residual_max,
        });
    }

    // Preliminary per-target scores, used only to choose a globally monotone
    // assignment. The final score adds the sequence/uniqueness component.
    let mut preliminary: Vec<Vec<u32>> = Vec::with_capacity(aligned.len());
    for (ordinal, entry) in aligned.iter().enumerate() {
        let mut row = Vec::with_capacity(entry.targets.len());
        for target in &entry.targets {
            let residual = residual_of(&mapping, ordinal, entry, target.page_index);
            let (page_score, _) = scoring::page_score(residual, config);
            row.push(
                target.title_score
                    + page_score
                    + numbering_score(entry, index.line(target.line_ref).primary_key.as_str())
                    + target.layout_score
                    + target.ocr_score,
            );
        }
        preliminary.push(row);
    }
    if cancelled() {
        return Err(CoreError::Cancelled);
    }
    let selection = align::monotonic_selection(&index, &aligned, &preliminary, cancelled)?;

    // Levels are decided over the contents order, before any entry is dropped,
    // so that a skipped parent cannot renumber its siblings.
    let level_inputs: Vec<(Option<Vec<u32>>, u32, f64)> = aligned
        .iter()
        .map(|entry| {
            (
                Some(entry.draft.numbering_path.clone()),
                entry.draft.indent_bucket,
                entry.draft.title_line_height,
            )
        })
        .collect();
    let measured = detections.iter().all(|detection| {
        index
            .pages
            .iter()
            .find(|page| page.page_index == detection.page_index)
            .map(|page| page.geometry == GeometryQuality::Measured)
            .unwrap_or(false)
    });
    let level_decisions = hierarchy::levels(&level_inputs, measured, config);

    let mut ids = Vec::with_capacity(aligned.len());
    let mut levels = Vec::with_capacity(aligned.len());
    let mut retained = Vec::with_capacity(aligned.len());
    let mut prepared = Vec::with_capacity(aligned.len());
    for (ordinal, entry) in aligned.iter_mut().enumerate() {
        let chosen = selection[ordinal];
        let target = chosen.and_then(|position| entry.targets.get(position));
        let runner_up_margin = match (chosen, entry.targets.len()) {
            (Some(position), _) => {
                let best = preliminary[ordinal][position];
                let other = preliminary[ordinal]
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != position)
                    .map(|(_, value)| *value)
                    .max()
                    .unwrap_or(0);
                best.saturating_sub(other)
            }
            (None, _) => 0,
        };
        let id = stable_toc_id(
            &package.source.content_sha256,
            rule_config_digest,
            &entry.draft,
            target.map(|target| index.line(target.line_ref).line_id.as_str()),
        );
        ids.push(id);
        levels.push(level_decisions[ordinal].level);
        retained.push(true);
        prepared.push((ordinal, target.cloned(), runner_up_margin));
    }
    let parents = hierarchy::parents(&levels, &retained, &ids);

    let mut candidates = Vec::with_capacity(prepared.len());
    for (ordinal, target, runner_up_margin) in prepared {
        if cancelled() {
            return Err(CoreError::Cancelled);
        }
        let entry = &aligned[ordinal];
        let draft = &entry.draft;
        let residual = target
            .as_ref()
            .and_then(|target| residual_of(&mapping, ordinal, entry, target.page_index));
        let (page_component, page_reason) = scoring::page_score(residual, config);
        let segment = mapping.segment_of(ordinal);
        let sequence = scoring::sequence_score(runner_up_margin, target.is_some(), config);
        let numbering = target
            .as_ref()
            .map(|target| numbering_score(entry, index.line(target.line_ref).primary_key.as_str()))
            .unwrap_or(0);
        let breakdown = scoring::breakdown(
            target
                .as_ref()
                .map(|target| target.title_score)
                .unwrap_or(0),
            page_component,
            numbering,
            target
                .as_ref()
                .map(|target| target.layout_score)
                .unwrap_or(0),
            target.as_ref().map(|target| target.ocr_score).unwrap_or(0),
            sequence,
        );
        let body_line = target.as_ref().map(|target| index.line(target.line_ref));
        let context = GateContext {
            min_toc_confidence: draft.min_confidence,
            min_body_confidence: target
                .as_ref()
                .map(|target| target.min_confidence)
                .unwrap_or(0.0),
            runner_up_margin,
            has_body_evidence: body_line.is_some_and(|line| line.bbox.finite()),
            printed_page_residual: residual,
            residual_supported: segment.is_some_and(|segment| segment.anchor_count >= 2),
            level_ambiguous: level_decisions[ordinal].ambiguous,
            secondary_only: target
                .as_ref()
                .map(|target| target.secondary_only)
                .unwrap_or(false),
            monotone: target.is_some(),
            approximate_multi_column: draft.column_index > 0
                && target
                    .as_ref()
                    .is_some_and(|target| target.geometry == GeometryQuality::Approximate),
            repeated_furniture: body_line.is_some_and(|line| line.repeated_furniture),
            truncated: entry.truncated_recall,
        };
        let decision = scoring::decide(&breakdown, &context, config);
        let (target_page, master_bbox) = match body_line {
            Some(line) => (
                package
                    .pages
                    .iter()
                    .find(|page| page.page_id == line.page_id)
                    .ok_or_else(|| {
                        CoreError::InvalidDocument("body heading page is unresolved".into())
                    })?,
                Some(line.bbox),
            ),
            None => (
                package
                    .pages
                    .iter()
                    .find(|page| page.page_id == draft.page_id)
                    .ok_or_else(|| {
                        CoreError::InvalidDocument("contents page is unresolved".into())
                    })?,
                None,
            ),
        };
        let mut evidence = vec![EvidenceRef::DerivedLine {
            page_id: draft.page_id.clone(),
            line_id: draft.line_ids[0].clone(),
            bbox: line_bbox(&index, &draft.page_id, &draft.line_ids[0]).unwrap_or(draft.bbox),
        }];
        if let Some(line) = body_line {
            evidence.push(EvidenceRef::DerivedLine {
                page_id: line.page_id.clone(),
                line_id: line.line_id.clone(),
                bbox: line.bbox,
            });
            // The word evidence is the line's most confident word, and its
            // effective text must actually occur in the title being written.
            if let Some(word) = line
                .words
                .iter()
                .filter(|word| !word.text.trim().is_empty())
                .max_by(|left, right| {
                    left.confidence
                        .total_cmp(&right.confidence)
                        .then_with(|| right.id.cmp(&left.id))
                })
            {
                if !line.raw_text.contains(word.text.trim()) {
                    return Err(CoreError::InvalidDocument(
                        "body heading word evidence does not occur in its line".into(),
                    ));
                }
                evidence.push(EvidenceRef::DerivedWord {
                    page_id: line.page_id.clone(),
                    word_id: word.id.clone(),
                    bbox: word.bbox,
                });
            }
        }
        let mut reason_codes = decision.reason_codes.clone();
        reason_codes.push(page_reason.to_owned());
        if body_line.is_some_and(|line| line.human_revised()) {
            reason_codes.push("human_revision_applied".to_owned());
        }
        reason_codes.extend(draft.reason_codes.iter().cloned());
        reason_codes.push(format!("level_{}", level_decisions[ordinal].reason));
        reason_codes.truncate(32);
        let mut candidate = base_candidate(
            ids[ordinal].clone(),
            &draft.raw_title,
            levels[ordinal],
            parents[ordinal].clone(),
            target_page,
            master_bbox,
            evidence,
            breakdown,
            reason_codes,
            vec![
                "toc_detect".into(),
                "toc_parse".into(),
                "body_alignment".into(),
                "printed_page_mapping".into(),
                "monotonic_sequence".into(),
            ],
        );
        candidate.status = decision.status;
        candidate.alignment_evidence = Some(AlignmentEvidence {
            toc_page_id: draft.page_id.clone(),
            toc_page_index: draft.page_index,
            toc_line_ids: draft.line_ids.clone(),
            toc_word_ids: draft.word_ids.iter().take(64).cloned().collect(),
            body_page_id: body_line.map(|line| line.page_id.clone()),
            body_page_index: body_line.map(|line| line.page_index),
            body_line_id: body_line.map(|line| line.line_id.clone()),
            printed_label_raw: draft.printed.as_ref().map(|printed| printed.raw.clone()),
            printed_number: draft.printed.as_ref().map(|printed| printed.value),
            numbering_family: draft
                .printed
                .as_ref()
                .map(|printed| printed.family.as_str().to_owned()),
            mapping_segment_index: segment.map(|segment| segment.index),
            mapping_offset: segment.map(|segment| segment.offset),
            page_residual: residual,
            runner_up_margin,
            column_index: draft.column_index,
            merged_toc_lines: draft.merged_lines,
            toc_has_leader: draft.has_leader,
            body_human_revised: body_line.is_some_and(|line| line.human_revised()),
            secondary_key_only: context.secondary_only,
            geometry_quality: body_line
                .map(|line| line.geometry.as_str().to_owned())
                .unwrap_or_else(|| "unknown".to_owned()),
            min_toc_word_confidence: draft.min_confidence,
            min_body_word_confidence: context.min_body_confidence,
        });
        candidate.automatic_decision = Some(AutomaticDecision {
            decided_status: match decision.status {
                BookmarkStatus::AutoConfirmed => "auto_confirmed",
                BookmarkStatus::Skipped => "skipped",
                _ => "needs_review",
            }
            .to_owned(),
            reason: decision.reason.clone(),
            rule_version: config.rule_version.clone(),
            rule_config_digest: rule_config_digest.to_owned(),
        });
        builder.count(&candidate);
        candidates.push(candidate);
    }
    // A parent that did not survive the gate must not leave a child pointing
    // at a bookmark that will never be written.
    let writable: std::collections::HashSet<String> = candidates
        .iter()
        .filter(|candidate| candidate.status.writes_to_pdf())
        .map(|candidate| candidate.candidate_id.clone())
        .collect();
    let by_id: BTreeMap<String, (Option<String>, bool)> = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.candidate_id.clone(),
                (
                    candidate.source_parent_id.clone(),
                    candidate.status.writes_to_pdf(),
                ),
            )
        })
        .collect();
    for candidate in &mut candidates {
        if !candidate.status.writes_to_pdf() {
            continue;
        }
        let mut parent = candidate.source_parent_id.clone();
        while let Some(id) = parent.clone() {
            if writable.contains(&id) {
                break;
            }
            parent = by_id.get(&id).and_then(|(grand, _)| grand.clone());
            if candidate
                .reason_codes
                .iter()
                .all(|code| code != "parent_promoted_to_nearest_ancestor")
                && candidate.reason_codes.len() < 32
            {
                candidate
                    .reason_codes
                    .push("parent_promoted_to_nearest_ancestor".to_owned());
            }
        }
        candidate.effective_parent_id = parent.clone();
        candidate.source_parent_id = parent;
    }
    normalize_levels(&mut candidates);
    Ok((GenerationMode::TocAligned, candidates))
}

/// Level repair after promotion: a written child must always be deeper than
/// its written parent, without ever inventing a new hierarchy.
fn normalize_levels(candidates: &mut [BookmarkCandidate]) {
    let levels: BTreeMap<String, u16> = candidates
        .iter()
        .map(|candidate| (candidate.candidate_id.clone(), candidate.source_level))
        .collect();
    for candidate in candidates.iter_mut() {
        let Some(parent) = candidate.source_parent_id.as_ref() else {
            continue;
        };
        if let Some(parent_level) = levels.get(parent) {
            if *parent_level >= candidate.source_level {
                candidate.source_level = parent_level + 1;
                candidate.effective_level = candidate.source_level;
            }
        }
    }
}

fn line_bbox(index: &TextIndex, page_id: &str, line_id: &str) -> Option<Bbox> {
    index
        .pages
        .iter()
        .find(|page| page.page_id == page_id)?
        .lines
        .iter()
        .find(|line| line.line_id == line_id)
        .map(|line| line.bbox)
}

fn residual_of(
    mapping: &align::PageMapping,
    ordinal: usize,
    entry: &EntryAlignment,
    physical_page: u32,
) -> Option<i64> {
    let printed = entry.draft.printed.as_ref()?;
    let expected = mapping.expected_page(ordinal, printed.value)?;
    Some(i64::from(physical_page) - expected)
}

/// Numbering/hierarchy agreement between a contents entry and its body line.
fn numbering_score(entry: &EntryAlignment, body_primary_key: &str) -> u32 {
    match entry.draft.numbering_prefix.as_deref() {
        Some(prefix) => {
            let normalized = super::text_index::primary_key(prefix);
            if !normalized.is_empty() && body_primary_key.starts_with(&normalized) {
                SCORE_NUMBERING_MAX
            } else {
                SCORE_NUMBERING_MAX * 3 / 5
            }
        }
        None => SCORE_NUMBERING_MAX / 2,
    }
}

/// Review-only heading proposals for a document with no printed contents.
/// These are never automatically confirmed: without a contents list there is
/// no second, independent signal that a large line is a chapter title.
fn heading_review_candidates(
    package: &DocumentPackage,
    index: &TextIndex,
    config: &AutoBookmarkConfig,
    rule_config_digest: &str,
) -> Vec<BookmarkCandidate> {
    let mut candidates = Vec::new();
    for page in &index.pages {
        if page.geometry != GeometryQuality::Measured || page.median_line_height <= 0.0 {
            continue;
        }
        let Some(source_page) = package
            .pages
            .iter()
            .find(|source| source.page_id == page.page_id)
        else {
            continue;
        };
        for line in &page.lines {
            if candidates.len() >= MAX_HEADING_REVIEW_CANDIDATES {
                return candidates;
            }
            if line.is_blank()
                || line.repeated_furniture
                || line.raw_text.len() > config.max_title_bytes as usize
                || line.bbox.height < page.median_line_height * 1.3
                || line.bbox.y + line.bbox.height > page.height * 0.9
                || line.top_permille > config.body_top_percent * 10
                || line.primary_key.chars().count() > 80
            {
                continue;
            }
            let breakdown = scoring::breakdown(0, 0, 0, 600, 600, 0);
            let mut candidate = base_candidate(
                stable_id(
                    &package.source.content_sha256,
                    source_page.physical_index,
                    &line.raw_text,
                    Some(line.bbox),
                ),
                &line.raw_text,
                0,
                None,
                source_page,
                Some(line.bbox),
                vec![EvidenceRef::DerivedLine {
                    page_id: line.page_id.clone(),
                    line_id: line.line_id.clone(),
                    bbox: line.bbox,
                }],
                breakdown,
                vec![
                    "heading_without_printed_contents".into(),
                    "requires_human_review".into(),
                ],
                vec!["heading_review_only".into()],
            );
            candidate.status = BookmarkStatus::NeedsReview;
            candidate.automatic_decision = Some(AutomaticDecision {
                decided_status: "needs_review".into(),
                reason: "no_printed_contents_for_corroboration".into(),
                rule_version: config.rule_version.clone(),
                rule_config_digest: rule_config_digest.to_owned(),
            });
            candidates.push(candidate);
        }
    }
    candidates
}

// ---------------------------------------------------------------------------
// Shared candidate construction
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn base_candidate(
    candidate_id: String,
    title: &str,
    level: u16,
    parent: Option<String>,
    page: &Page,
    bbox: Option<Bbox>,
    evidence: Vec<EvidenceRef>,
    breakdown: ConfidenceBreakdown,
    reason_codes: Vec<String>,
    rule_trace: Vec<String>,
) -> BookmarkCandidate {
    BookmarkCandidate {
        candidate_id,
        source_title: title.to_owned(),
        effective_title: title.trim().to_owned(),
        source_level: level,
        effective_level: level,
        source_parent_id: parent.clone(),
        effective_parent_id: parent,
        target_page_id: page.page_id.clone(),
        physical_page_index: page.physical_index,
        master_bbox: bbox,
        outline_evidence: None,
        evidence,
        confidence: breakdown.confidence(),
        status: BookmarkStatus::Proposed,
        generator: GeneratorProvenance {
            kind: GENERATOR_KIND.into(),
            name: GENERATOR_NAME.into(),
            version: GENERATOR_VERSION_V2.into(),
        },
        reason_codes,
        rule_trace,
        confidence_breakdown: Some(breakdown),
        alignment_evidence: None,
        automatic_decision: None,
    }
}

fn sort_candidates(package: &DocumentPackage, candidates: &mut [BookmarkCandidate]) {
    let orders: BTreeMap<&str, u32> = package
        .pages
        .iter()
        .map(|page| (page.page_id.as_str(), page.order))
        .collect();
    let outline_position = |candidate: &BookmarkCandidate| {
        candidate
            .evidence
            .iter()
            .find_map(|evidence| match evidence {
                EvidenceRef::MdpOutline {
                    page_id, ordinal, ..
                } => Some((
                    orders.get(page_id.as_str()).copied().unwrap_or(u32::MAX),
                    *ordinal,
                )),
                _ => None,
            })
    };
    candidates.sort_by(|a, b| {
        match (outline_position(a), outline_position(b)) {
            (Some(left), Some(right)) => return left.cmp(&right),
            (Some(_), None) => return std::cmp::Ordering::Less,
            (None, Some(_)) => return std::cmp::Ordering::Greater,
            (None, None) => {}
        }
        a.physical_page_index
            .cmp(&b.physical_page_index)
            .then_with(|| {
                a.master_bbox
                    .map(|bbox| bbox.y)
                    .unwrap_or(0.0)
                    .total_cmp(&b.master_bbox.map(|bbox| bbox.y).unwrap_or(0.0))
            })
            .then_with(|| a.effective_level.cmp(&b.effective_level))
            .then_with(|| a.candidate_id.cmp(&b.candidate_id))
    });
}

pub(super) fn stable_id(source: &str, page: u32, title: &str, bbox: Option<Bbox>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source);
    hasher.update(page.to_le_bytes());
    hasher.update(title.trim().as_bytes());
    if let Some(bbox) = bbox {
        hasher.update(format!(
            "{:.6},{:.6},{:.6},{:.6}",
            bbox.x, bbox.y, bbox.width, bbox.height
        ));
    }
    format!("bookmark-{}", &hex(&hasher.finalize())[..32])
}

pub(super) fn stable_outline_id(
    source: &str,
    evidence_page_id: &str,
    ordinal: u32,
    target_page_id: &str,
    title: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source);
    hasher.update(evidence_page_id);
    hasher.update(ordinal.to_le_bytes());
    hasher.update(target_page_id);
    hasher.update(title.as_bytes());
    format!("bookmark-{}", &hex(&hasher.finalize())[..32])
}

fn stable_toc_id(
    source: &str,
    rule_config_digest: &str,
    draft: &TocEntryDraft,
    body_line_id: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source);
    hasher.update([0]);
    hasher.update(rule_config_digest);
    hasher.update([0]);
    hasher.update(&draft.page_id);
    for line in &draft.line_ids {
        hasher.update([0]);
        hasher.update(line);
    }
    hasher.update([0]);
    hasher.update(draft.raw_title.as_bytes());
    hasher.update([0]);
    hasher.update(body_line_id.unwrap_or("").as_bytes());
    format!("bookmark-{}", &hex(&hasher.finalize())[..32])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Report assembly
// ---------------------------------------------------------------------------

/// Route plus the four provider identity fields, used only to group the
/// report's provenance summary. It never reaches an algorithmic decision.
type ProvenanceKey = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn provenance_summary(ocr: Option<&OcrRun>) -> Vec<OcrProvenanceSummary> {
    let Some(ocr) = ocr else {
        return Vec::new();
    };
    let mut grouped: BTreeMap<ProvenanceKey, u32> = BTreeMap::new();
    for page in &ocr.pages {
        let route = match page.route {
            OcrRoute::NativeText => "native_text".to_owned(),
            OcrRoute::Ocr { .. } => "ocr".to_owned(),
        };
        let key = (
            route,
            page.provider_provenance
                .as_ref()
                .map(|provenance| provenance.engine.clone()),
            page.provider_provenance
                .as_ref()
                .map(|provenance| provenance.model.clone()),
            page.provider_provenance
                .as_ref()
                .map(|provenance| provenance.version.clone()),
            page.provider_provenance
                .as_ref()
                .map(|provenance| provenance.execution_location.clone()),
        );
        *grouped.entry(key).or_default() += 1;
    }
    grouped
        .into_iter()
        .map(
            |((route, engine, model, version, execution_location), page_count)| {
                OcrProvenanceSummary {
                    route,
                    engine,
                    model,
                    version,
                    execution_location,
                    page_count,
                }
            },
        )
        .collect()
}

struct ReportBuilder {
    source_digest: String,
    package_digest: String,
    ocr_digest: Option<String>,
    derived_digest: Option<String>,
    revision_digest: Option<String>,
    rule_config_digest: String,
    rule_version: String,
    safe_refusal_reason: Option<String>,
    ocr_provenance: Vec<OcrProvenanceSummary>,
    front_page_limit: u32,
    scanned_front_pages: u32,
    toc_pages: Vec<TocPageReport>,
    parsed_entries: u32,
    auto_confirmed: u32,
    needs_review: u32,
    skipped: u32,
    reason_code_counts: BTreeMap<String, u32>,
    mapping_segments: Vec<MappingSegmentReport>,
    truncated: bool,
    truncation_reasons: Vec<String>,
    shortlist_postings_visited: u64,
    body_lines_indexed: u64,
}

impl ReportBuilder {
    fn new(
        source_digest: String,
        package_digest: String,
        ocr_digest: Option<String>,
        derived_digest: Option<String>,
        revision_digest: Option<String>,
        rule_config_digest: String,
        rule_version: String,
    ) -> Self {
        Self {
            source_digest,
            package_digest,
            ocr_digest,
            derived_digest,
            revision_digest,
            rule_config_digest,
            rule_version,
            safe_refusal_reason: None,
            ocr_provenance: Vec::new(),
            front_page_limit: 0,
            scanned_front_pages: 0,
            toc_pages: Vec::new(),
            parsed_entries: 0,
            auto_confirmed: 0,
            needs_review: 0,
            skipped: 0,
            reason_code_counts: BTreeMap::new(),
            mapping_segments: Vec::new(),
            truncated: false,
            truncation_reasons: Vec::new(),
            shortlist_postings_visited: 0,
            body_lines_indexed: 0,
        }
    }

    fn count(&mut self, candidate: &BookmarkCandidate) {
        match candidate.status {
            BookmarkStatus::AutoConfirmed => self.auto_confirmed += 1,
            BookmarkStatus::Skipped => self.skipped += 1,
            _ => self.needs_review += 1,
        }
        for code in &candidate.reason_codes {
            if self.reason_code_counts.len() < 256 {
                *self.reason_code_counts.entry(code.clone()).or_default() += 1;
            }
        }
    }

    fn finish(
        mut self,
        mode: GenerationMode,
        snapshot: &BookmarkSnapshot,
    ) -> BookmarkGenerationReport {
        if mode == GenerationMode::ExistingOutline {
            self.auto_confirmed = snapshot
                .candidates
                .iter()
                .filter(|candidate| candidate.status == BookmarkStatus::AutoConfirmed)
                .count() as u32;
            self.parsed_entries = snapshot.candidates.len() as u32;
            self.reason_code_counts
                .insert("validated_existing_outline".to_owned(), self.auto_confirmed);
        }
        let status = if self.auto_confirmed > 0 {
            GenerationStatus::AutoConfirmed
        } else if mode == GenerationMode::SafeRefusal {
            GenerationStatus::SafeRefusal
        } else if self.needs_review > 0 {
            GenerationStatus::NeedsReview
        } else {
            GenerationStatus::SafeRefusal
        };
        if status == GenerationStatus::SafeRefusal && self.safe_refusal_reason.is_none() {
            self.safe_refusal_reason = Some(
                "a printed contents list was found but no entry reached the confidence gate"
                    .to_owned(),
            );
        }
        self.truncation_reasons.sort();
        self.truncation_reasons.dedup();
        let mut report = BookmarkGenerationReport {
            schema: REPORT_SCHEMA.into(),
            schema_version: REPORT_SCHEMA_VERSION.into(),
            source_digest: self.source_digest,
            package_digest: self.package_digest,
            ocr_digest: if mode == GenerationMode::TocAligned {
                self.ocr_digest
            } else {
                None
            },
            derived_digest: self.derived_digest,
            revision_digest: self.revision_digest,
            rule_config_digest: self.rule_config_digest,
            rule_version: self.rule_version,
            mode,
            status,
            safe_refusal_reason: self.safe_refusal_reason,
            ocr_provenance: self.ocr_provenance,
            front_page_limit: self.front_page_limit,
            scanned_front_pages: self.scanned_front_pages,
            toc_pages: self.toc_pages,
            parsed_entries: self.parsed_entries,
            auto_confirmed: self.auto_confirmed,
            needs_review: self.needs_review,
            skipped: self.skipped,
            reason_code_counts: self.reason_code_counts,
            mapping_segments: self.mapping_segments,
            truncated: self.truncated,
            truncation_reasons: self.truncation_reasons,
            shortlist_postings_visited: self.shortlist_postings_visited,
            body_lines_indexed: self.body_lines_indexed,
            generation_digest: snapshot.generation_digest.clone(),
            report_digest: String::new(),
        };
        report.report_digest = report.recomputed_report_digest();
        report
    }
}

/// The maximum representable score, exported for UI normalization.
pub const MAX_SCORE: u32 = SCORE_TOTAL;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bookmark_fixtures as fixtures;
    use crate::bookmarks::model::schema_tests::{
        assert_conforms, bookmarks_0_2_schema, report_0_1_schema,
    };

    fn aligned_result() -> AutoBookmarkResult {
        let (package, pages) = fixtures::aligned_book();
        let ocr = fixtures::ocr_run(&pages, None);
        let derived = DerivedDocument::from_package(&package, Some(&ocr)).unwrap();
        generate_auto(
            &AutoBookmarkInput {
                package: &package,
                ocr: Some(&ocr),
                derived: Some(&derived),
            },
            &AutoBookmarkConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn a_generated_snapshot_and_report_conform_to_their_published_schemas() {
        let result = aligned_result();
        let snapshot_schema = bookmarks_0_2_schema();
        let snapshot = serde_json::to_value(&result.snapshot).unwrap();
        assert_conforms(&snapshot, &snapshot_schema, &snapshot_schema, "snapshot");
        let report_schema = report_0_1_schema();
        let report = serde_json::to_value(&result.report).unwrap();
        assert_conforms(&report, &report_schema, &report_schema, "report");
        assert!(result.snapshot.candidates.iter().any(|candidate| candidate
            .confidence_breakdown
            .is_some_and(|breakdown| breakdown.is_consistent())));
    }

    #[test]
    fn a_zero_one_snapshot_never_serializes_a_zero_two_field() {
        let mut package = fixtures::package("compat", 2);
        let first = package.pages[0].page_id.clone();
        package.pages[0].existing_outline_evidence =
            vec![crate::document_package::ExistingOutlineEvidence {
                title: "Chapter".into(),
                level: 0,
                target_page_id: Some(first),
                source: "source-pdf".into(),
            }];
        let mut snapshot = generate_auto(
            &AutoBookmarkInput {
                package: &package,
                ocr: None,
                derived: None,
            },
            &AutoBookmarkConfig::default(),
        )
        .unwrap()
        .snapshot;
        // Downgrading the label alone must not produce a readable 0.1 file.
        snapshot.schema_version = BOOKMARK_SCHEMA_VERSION.into();
        snapshot.generation_digest = snapshot.recomputed_generation_digest();
        assert!(snapshot.validate().is_err());
    }
}
