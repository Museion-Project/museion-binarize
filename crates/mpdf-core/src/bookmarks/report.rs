//! The auditable generation report written next to a bookmark snapshot.
//!
//! The report explains *why* a run produced what it produced: which front
//! pages were scanned, which of them looked like a printed contents list and
//! on what signals, how printed page labels mapped onto physical pages, and
//! which reason codes drove entries away from automatic confirmation. It
//! deliberately does not copy the document's text or any raw provider
//! artifact — the per-candidate evidence already carries the titles, and the
//! provider blob is never an input to a decision.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::model::{GenerationMode, GenerationStatus, REPORT_SCHEMA, REPORT_SCHEMA_VERSION};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OcrProvenanceSummary {
    pub route: String,
    pub engine: Option<String>,
    pub model: Option<String>,
    pub version: Option<String>,
    pub execution_location: Option<String>,
    pub page_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TocPageReport {
    pub page_id: String,
    pub page_index: u32,
    pub score: u32,
    pub signals: BTreeMap<String, u32>,
    pub parsed_entries: u32,
    /// Line references behind the decision, not just a page number.
    pub keyword_line_ids: Vec<String>,
    pub entry_line_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MappingSegmentReport {
    pub numbering_family: String,
    pub segment_index: u32,
    pub offset: i64,
    pub anchor_count: u32,
    pub first_printed_number: u32,
    pub last_printed_number: u32,
    pub residual_min: i64,
    pub residual_max: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BookmarkGenerationReport {
    pub schema: String,
    pub schema_version: String,
    pub source_digest: String,
    pub package_digest: String,
    pub ocr_digest: Option<String>,
    pub derived_digest: Option<String>,
    pub revision_digest: Option<String>,
    pub rule_config_digest: String,
    pub rule_version: String,
    pub mode: GenerationMode,
    pub status: GenerationStatus,
    pub safe_refusal_reason: Option<String>,
    pub ocr_provenance: Vec<OcrProvenanceSummary>,
    pub front_page_limit: u32,
    pub scanned_front_pages: u32,
    pub toc_pages: Vec<TocPageReport>,
    pub parsed_entries: u32,
    pub auto_confirmed: u32,
    pub needs_review: u32,
    pub skipped: u32,
    pub reason_code_counts: BTreeMap<String, u32>,
    pub mapping_segments: Vec<MappingSegmentReport>,
    pub truncated: bool,
    pub truncation_reasons: Vec<String>,
    /// Inverted-index work actually performed, as evidence that the engine
    /// never degrades into a contents x document comparison.
    pub shortlist_postings_visited: u64,
    pub body_lines_indexed: u64,
    pub generation_digest: String,
    pub report_digest: String,
}

impl BookmarkGenerationReport {
    pub fn recomputed_report_digest(&self) -> String {
        let mut copy = self.clone();
        copy.report_digest = String::new();
        let bytes = serde_json::to_vec(&copy).expect("bookmark report is serializable");
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn validate(&self) -> crate::error::Result<()> {
        let hex = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        };
        if self.schema != REPORT_SCHEMA
            || self.schema_version != REPORT_SCHEMA_VERSION
            || !hex(&self.source_digest)
            || !hex(&self.package_digest)
            || !hex(&self.rule_config_digest)
            || !hex(&self.generation_digest)
            || self.ocr_digest.as_deref().is_some_and(|value| !hex(value))
            || self
                .derived_digest
                .as_deref()
                .is_some_and(|value| !hex(value))
            || self
                .revision_digest
                .as_deref()
                .is_some_and(|value| !hex(value))
            || self.rule_version.is_empty()
            || self.toc_pages.len() > 10_000
            || self.mapping_segments.len() > 10_000
            || self.reason_code_counts.len() > 256
            || self.truncation_reasons.len() > 64
        {
            return Err(crate::error::CoreError::InvalidDocument(
                "invalid bookmark generation report".into(),
            ));
        }
        if self.status == GenerationStatus::SafeRefusal && self.safe_refusal_reason.is_none() {
            return Err(crate::error::CoreError::InvalidDocument(
                "a safe refusal must state its reason".into(),
            ));
        }
        if self.mode == GenerationMode::TocAligned && self.ocr_digest.is_none() {
            return Err(crate::error::CoreError::InvalidDocument(
                "an aligned report must bind its OCR digest".into(),
            ));
        }
        if self.report_digest != self.recomputed_report_digest() {
            return Err(crate::error::CoreError::InvalidDocument(
                "bookmark report digest does not match contents".into(),
            ));
        }
        Ok(())
    }
}
