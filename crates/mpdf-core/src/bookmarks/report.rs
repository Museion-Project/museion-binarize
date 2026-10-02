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

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use super::model::{
    GenerationMode, GenerationStatus, REPORT_SCHEMA, REPORT_SCHEMA_VERSION, REPORT_SCHEMA_VERSIONS,
    REPORT_SCHEMA_VERSION_V2,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingSegmentReport {
    pub numbering_family: String,
    pub segment_index: u32,
    pub offset: i64,
    /// The 0.1 segment count. It meant every anchor assigned to the run,
    /// including anchors that disagreed with the selected offset. It remains
    /// the in-memory compatibility alias for `member_count` in 0.2 reports,
    /// but is serialized only by the 0.1 wire format.
    pub anchor_count: u32,
    /// Every anchor in the run, including ones the solver overruled.
    /// Diagnostic only — see `MappingSegment::member_count`.
    pub member_count: Option<u32>,
    /// Anchors whose own observed offset equals the segment offset.
    pub exact_anchor_count: Option<u32>,
    /// Anchors kept in the run despite disagreeing with its offset.
    pub disagreeing_anchor_count: Option<u32>,
    pub first_printed_number: u32,
    pub last_printed_number: u32,
    pub residual_min: i64,
    pub residual_max: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingSegmentWire {
    numbering_family: String,
    segment_index: u32,
    offset: i64,
    #[serde(default)]
    anchor_count: Option<u32>,
    #[serde(default)]
    member_count: Option<u32>,
    #[serde(default)]
    exact_anchor_count: Option<u32>,
    #[serde(default)]
    disagreeing_anchor_count: Option<u32>,
    first_printed_number: u32,
    last_printed_number: u32,
    residual_min: i64,
    residual_max: i64,
}

#[derive(Serialize)]
struct MappingSegmentV1<'a> {
    numbering_family: &'a str,
    segment_index: u32,
    offset: i64,
    anchor_count: u32,
    first_printed_number: u32,
    last_printed_number: u32,
    residual_min: i64,
    residual_max: i64,
}

#[derive(Serialize)]
struct MappingSegmentV2<'a> {
    numbering_family: &'a str,
    segment_index: u32,
    offset: i64,
    member_count: u32,
    exact_anchor_count: u32,
    disagreeing_anchor_count: u32,
    first_printed_number: u32,
    last_printed_number: u32,
    residual_min: i64,
    residual_max: i64,
}

impl Serialize for MappingSegmentReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match (
            self.member_count,
            self.exact_anchor_count,
            self.disagreeing_anchor_count,
        ) {
            (None, None, None) => MappingSegmentV1 {
                numbering_family: &self.numbering_family,
                segment_index: self.segment_index,
                offset: self.offset,
                anchor_count: self.anchor_count,
                first_printed_number: self.first_printed_number,
                last_printed_number: self.last_printed_number,
                residual_min: self.residual_min,
                residual_max: self.residual_max,
            }
            .serialize(serializer),
            (Some(member_count), Some(exact_anchor_count), Some(disagreeing_anchor_count)) => {
                MappingSegmentV2 {
                    numbering_family: &self.numbering_family,
                    segment_index: self.segment_index,
                    offset: self.offset,
                    member_count,
                    exact_anchor_count,
                    disagreeing_anchor_count,
                    first_printed_number: self.first_printed_number,
                    last_printed_number: self.last_printed_number,
                    residual_min: self.residual_min,
                    residual_max: self.residual_max,
                }
                .serialize(serializer)
            }
            _ => Err(serde::ser::Error::custom(
                "bookmark mapping segment mixes report schema shapes",
            )),
        }
    }
}

impl<'de> Deserialize<'de> for MappingSegmentReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MappingSegmentWire::deserialize(deserializer)?;
        let (anchor_count, member_count, exact_anchor_count, disagreeing_anchor_count) = match (
            wire.anchor_count,
            wire.member_count,
            wire.exact_anchor_count,
            wire.disagreeing_anchor_count,
        ) {
            (Some(anchor_count), None, None, None) => (anchor_count, None, None, None),
            (
                None,
                Some(member_count),
                Some(exact_anchor_count),
                Some(disagreeing_anchor_count),
            ) => (
                member_count,
                Some(member_count),
                Some(exact_anchor_count),
                Some(disagreeing_anchor_count),
            ),
            _ => {
                return Err(serde::de::Error::custom(
                    "bookmark mapping segment mixes report schema shapes",
                ))
            }
        };
        Ok(Self {
            numbering_family: wire.numbering_family,
            segment_index: wire.segment_index,
            offset: wire.offset,
            anchor_count,
            member_count,
            exact_anchor_count,
            disagreeing_anchor_count,
            first_printed_number: wire.first_printed_number,
            last_printed_number: wire.last_printed_number,
            residual_min: wire.residual_min,
            residual_max: wire.residual_max,
        })
    }
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
        let legacy_shape = self.schema_version == REPORT_SCHEMA_VERSION
            && self.mapping_segments.iter().all(|segment| {
                segment.member_count.is_none()
                    && segment.exact_anchor_count.is_none()
                    && segment.disagreeing_anchor_count.is_none()
            });
        let detailed_shape = self.schema_version == REPORT_SCHEMA_VERSION_V2
            && self.mapping_segments.iter().all(|segment| {
                segment.member_count.is_some()
                    && segment.exact_anchor_count.is_some()
                    && segment.disagreeing_anchor_count.is_some()
                    && segment.member_count == Some(segment.anchor_count)
                    && segment.member_count
                        == segment
                            .exact_anchor_count
                            .zip(segment.disagreeing_anchor_count)
                            .and_then(|(exact, disagreeing)| exact.checked_add(disagreeing))
            });
        if self.schema != REPORT_SCHEMA
            || !REPORT_SCHEMA_VERSIONS.contains(&self.schema_version.as_str())
            || !(legacy_shape || detailed_shape)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bookmarks::model::schema_tests::{assert_conforms, report_0_1_schema};

    const LEGACY_REPORT: &str =
        include_str!("../../../../test-data/fixtures/bookmark-generation-report-0.1.json");

    #[test]
    fn a_published_zero_one_report_artifact_stays_readable_and_digest_valid() {
        let directory = tempfile::tempdir().unwrap();
        let bookmarks = directory.path().join("bookmarks");
        std::fs::create_dir(&bookmarks).unwrap();
        std::fs::write(bookmarks.join("generation-report.json"), LEGACY_REPORT).unwrap();

        let report = crate::bookmarks::load_generation_report(directory.path())
            .expect("the published 0.1 artifact remains readable");
        assert_eq!(report.schema_version, REPORT_SCHEMA_VERSION);
        assert_eq!(report.mapping_segments[0].anchor_count, 3);
        assert_eq!(report.mapping_segments[0].member_count, None);

        let value = serde_json::to_value(&report).unwrap();
        assert_conforms(&value, &report_0_1_schema(), &report_0_1_schema(), "report");
        assert_eq!(
            value,
            serde_json::from_str::<serde_json::Value>(LEGACY_REPORT).unwrap()
        );
    }

    #[test]
    fn a_report_cannot_mix_a_version_label_with_the_other_segment_shape() {
        let mut report: BookmarkGenerationReport = serde_json::from_str(LEGACY_REPORT).unwrap();
        report.schema_version = REPORT_SCHEMA_VERSION_V2.into();
        report.report_digest = report.recomputed_report_digest();
        assert!(report.validate().is_err());
    }
}
