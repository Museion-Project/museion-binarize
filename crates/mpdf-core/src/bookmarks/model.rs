use crate::derived::Bbox;
use serde::{Deserialize, Serialize};

pub const BOOKMARK_SCHEMA: &str = "mpdf-bookmarks";
pub const BOOKMARK_SCHEMA_VERSION: &str = "0.1";
/// The schema every new generation writes. 0.1 files stay readable and
/// reviewable exactly as they are; nothing migrates them in place, and no
/// 0.1 record can acquire an automatic status by being re-read.
pub const BOOKMARK_SCHEMA_VERSION_V2: &str = "0.2";
pub const BOOKMARK_SCHEMA_VERSIONS: [&str; 2] =
    [BOOKMARK_SCHEMA_VERSION, BOOKMARK_SCHEMA_VERSION_V2];
pub const MAX_CANDIDATES: usize = 100_000;
pub const MAX_REVIEWS: usize = 100_000;
pub const MAX_TOC_ENTRIES: usize = 10_000;
/// Longest accepted bookmark title, in bytes.
pub const MAX_TITLE_BYTES: usize = 4 * 1024;
pub const REPORT_SCHEMA: &str = "mpdf-bookmark-generation-report";
pub const REPORT_SCHEMA_VERSION: &str = "0.1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BookmarkSnapshot {
    pub schema: String,
    pub schema_version: String,
    pub source_digest: String,
    pub package_digest: String,
    pub derived_digest: Option<String>,
    pub generator: GeneratorProvenance,
    pub candidates: Vec<BookmarkCandidate>,
    pub generation_digest: String,
    // --- 0.2 additions -------------------------------------------------
    // Every field below is omitted entirely when absent, so a 0.1 snapshot
    // read from disk re-serializes byte-identically and keeps its original
    // generation digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_config_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_mode: Option<GenerationMode>,
}
impl BookmarkSnapshot {
    pub(crate) fn without_digest(&self) -> Self {
        let mut x = self.clone();
        x.generation_digest = String::new();
        x
    }
    /// True for a snapshot written by the automatic table-of-contents engine.
    pub fn is_v2(&self) -> bool {
        self.schema_version == BOOKMARK_SCHEMA_VERSION_V2
    }

    pub fn validate(&self) -> crate::error::Result<()> {
        if self.schema != BOOKMARK_SCHEMA
            || !BOOKMARK_SCHEMA_VERSIONS.contains(&self.schema_version.as_str())
            || self.candidates.len() > MAX_CANDIDATES
            || !digest(&self.source_digest)
            || !digest(&self.package_digest)
            || self.derived_digest.as_ref().is_some_and(|x| !digest(x))
            || self.ocr_digest.as_ref().is_some_and(|x| !digest(x))
            || self.revision_digest.as_ref().is_some_and(|x| !digest(x))
            || self.rule_config_digest.as_ref().is_some_and(|x| !digest(x))
            || self.generator.kind.is_empty()
            || self.generator.name.is_empty()
            || self.generator.version.is_empty()
        {
            return Err(crate::error::CoreError::InvalidDocument(
                "invalid bookmark snapshot".into(),
            ));
        }
        if self.is_v2() {
            // 0.2 binds every input that can change an automatic decision.
            if self.rule_config_digest.is_none()
                || self.rule_version.as_ref().is_none_or(String::is_empty)
                || self.generation_mode.is_none()
            {
                return Err(crate::error::CoreError::InvalidDocument(
                    "bookmark snapshot 0.2 is missing its rule or mode binding".into(),
                ));
            }
            if self.generation_mode == Some(GenerationMode::TocAligned) && self.ocr_digest.is_none()
            {
                return Err(crate::error::CoreError::InvalidDocument(
                    "an aligned bookmark snapshot must bind its OCR digest".into(),
                ));
            }
        } else if self.ocr_digest.is_some()
            || self.revision_digest.is_some()
            || self.rule_config_digest.is_some()
            || self.rule_version.is_some()
            || self.generation_mode.is_some()
            || self
                .candidates
                .iter()
                .any(|candidate| candidate.is_v2_shaped())
        {
            // A 0.1 file must never carry 0.2 semantics: an old snapshot
            // cannot acquire an automatic status by relabeling.
            return Err(crate::error::CoreError::InvalidDocument(
                "bookmark snapshot 0.1 must not contain 0.2 fields".into(),
            ));
        }
        let mut ids = std::collections::HashSet::new();
        for c in &self.candidates {
            c.validate()?;
            if !ids.insert(&c.candidate_id) {
                return Err(crate::error::CoreError::InvalidDocument(
                    "duplicate bookmark candidate ID".into(),
                ));
            }
        }
        if self.generation_digest.is_empty() {
            return Err(crate::error::CoreError::InvalidDocument(
                "missing bookmark generation digest".into(),
            ));
        }
        if self.generation_digest != self.recomputed_generation_digest() {
            return Err(crate::error::CoreError::InvalidDocument(
                "bookmark generation digest does not match contents".into(),
            ));
        }
        Ok(())
    }
    pub fn recomputed_generation_digest(&self) -> String {
        use sha2::{Digest, Sha256};
        let bytes =
            serde_json::to_vec(&self.without_digest()).expect("bookmark model is serializable");
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BookmarkCandidate {
    pub candidate_id: String,
    pub source_title: String,
    pub effective_title: String,
    pub source_level: u16,
    pub effective_level: u16,
    pub source_parent_id: Option<String>,
    pub effective_parent_id: Option<String>,
    pub target_page_id: String,
    pub physical_page_index: u32,
    pub master_bbox: Option<Bbox>,
    pub outline_evidence: Option<OutlineEvidence>,
    pub evidence: Vec<EvidenceRef>,
    pub confidence: f32,
    pub status: BookmarkStatus,
    pub generator: GeneratorProvenance,
    pub reason_codes: Vec<String>,
    pub rule_trace: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_breakdown: Option<ConfidenceBreakdown>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alignment_evidence: Option<AlignmentEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_decision: Option<AutomaticDecision>,
}
impl BookmarkCandidate {
    /// True when this record uses any 0.2-only field or status.
    pub fn is_v2_shaped(&self) -> bool {
        self.status.requires_v2()
            || self.confidence_breakdown.is_some()
            || self.alignment_evidence.is_some()
            || self.automatic_decision.is_some()
    }

    pub fn validate(&self) -> crate::error::Result<()> {
        if self
            .confidence_breakdown
            .is_some_and(|breakdown| !breakdown.is_consistent())
        {
            return Err(crate::error::CoreError::InvalidDocument(
                "bookmark confidence breakdown is inconsistent".into(),
            ));
        }
        if self.status == BookmarkStatus::AutoConfirmed
            && (self.confidence_breakdown.is_none() || self.automatic_decision.is_none())
        {
            return Err(crate::error::CoreError::InvalidDocument(
                "an automatically confirmed bookmark must carry its decision provenance".into(),
            ));
        }
        if self.source_title.len() > MAX_TITLE_BYTES || self.effective_title.len() > MAX_TITLE_BYTES
        {
            return Err(crate::error::CoreError::InvalidDocument(
                "bookmark title exceeds its byte limit".into(),
            ));
        }
        if self.candidate_id.is_empty()
            || self.source_title.trim().is_empty()
            || self.effective_title.trim().is_empty()
            || self.target_page_id.is_empty()
            || self.effective_level > 64
            || !self.confidence.is_finite()
            || !(0.0..=1.0).contains(&self.confidence)
            || self.evidence.is_empty()
            || self.evidence.iter().any(|e| !e.is_resolvable())
        {
            return Err(crate::error::CoreError::InvalidDocument(
                "invalid bookmark candidate or unresolved evidence".into(),
            ));
        }
        if self.master_bbox.is_some_and(|b| !b.finite()) {
            return Err(crate::error::CoreError::InvalidDocument(
                "invalid bookmark bbox".into(),
            ));
        }
        if matches!(
            self.status,
            BookmarkStatus::Confirmed | BookmarkStatus::AutoConfirmed
        ) && self.generator.kind == "ai_suggestion"
        {
            return Err(crate::error::CoreError::InvalidDocument(
                "AI suggestions cannot be confirmed".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeneratorProvenance {
    pub kind: String,
    pub name: String,
    pub version: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutlineEvidence {
    pub title: String,
    pub level: u16,
    pub target_page_id: Option<String>,
    pub source: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceRef {
    MdpOutline {
        page_id: String,
        ordinal: u32,
        source: String,
    },
    MdpPageLabel {
        page_id: String,
        label: String,
    },
    MdpTypography {
        page_id: String,
        ordinal: u32,
        bbox: crate::document_package::Rect,
    },
    MdpRegion {
        page_id: String,
        ordinal: u32,
        bbox: crate::document_package::Rect,
    },
    DerivedPage {
        page_id: String,
        bbox: Bbox,
    },
    DerivedLine {
        page_id: String,
        line_id: String,
        bbox: Bbox,
    },
    DerivedWord {
        page_id: String,
        word_id: String,
        bbox: Bbox,
    },
}
impl EvidenceRef {
    pub fn is_resolvable(&self) -> bool {
        match self {
            Self::MdpOutline { page_id, .. }
            | Self::MdpPageLabel { page_id, .. }
            | Self::MdpTypography { page_id, .. }
            | Self::MdpRegion { page_id, .. }
            | Self::DerivedPage { page_id, .. }
            | Self::DerivedLine { page_id, .. }
            | Self::DerivedWord { page_id, .. } => !page_id.is_empty(),
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkStatus {
    Proposed,
    NeedsReview,
    Confirmed,
    Rejected,
    /// Produced only by the frozen deterministic gate in `scoring`. It is
    /// written into the PDF outline like `Confirmed`, but it is never the
    /// result of a human review and never claims to be one.
    AutoConfirmed,
    /// Retained for audit: evidence exists but is too weak to propose.
    Skipped,
}

impl BookmarkStatus {
    /// Statuses the searchable-PDF writer materializes into the outline.
    pub fn writes_to_pdf(self) -> bool {
        matches!(self, Self::Confirmed | Self::AutoConfirmed)
    }

    /// Statuses introduced by bookmark snapshot 0.2.
    pub fn requires_v2(self) -> bool {
        matches!(self, Self::AutoConfirmed | Self::Skipped)
    }
}

/// How a snapshot was produced. Reported to users as the three business
/// outcomes: a preserved native outline, a compiled printed contents list,
/// or an explained refusal.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GenerationMode {
    ExistingOutline,
    TocAligned,
    SafeRefusal,
}

impl GenerationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExistingOutline => "existing_outline",
            Self::TocAligned => "toc_aligned",
            Self::SafeRefusal => "safe_refusal",
        }
    }
}

/// The overall business result of one generation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GenerationStatus {
    AutoConfirmed,
    NeedsReview,
    SafeRefusal,
}

impl GenerationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AutoConfirmed => "auto_confirmed",
            Self::NeedsReview => "needs_review",
            Self::SafeRefusal => "safe_refusal",
        }
    }
}

/// Integer score components, 0..=10,000 in total. Floating point never
/// participates in a bookmark decision.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfidenceBreakdown {
    pub title_match: u32,
    pub page_mapping: u32,
    pub numbering_hierarchy: u32,
    pub body_layout: u32,
    pub ocr_quality: u32,
    pub sequence_uniqueness: u32,
    pub total: u32,
}

impl ConfidenceBreakdown {
    pub fn confidence(&self) -> f32 {
        (self.total.min(10_000) as f32) / 10_000.0
    }
    pub fn is_consistent(&self) -> bool {
        self.title_match <= 4_000
            && self.page_mapping <= 2_000
            && self.numbering_hierarchy <= 1_000
            && self.body_layout <= 1_000
            && self.ocr_quality <= 1_000
            && self.sequence_uniqueness <= 1_000
            && self.total
                == self.title_match
                    + self.page_mapping
                    + self.numbering_hierarchy
                    + self.body_layout
                    + self.ocr_quality
                    + self.sequence_uniqueness
    }
}

/// Typed alignment evidence for one compiled contents entry: which printed
/// contents lines it came from, which body heading it resolved to, and how
/// the printed page label was mapped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlignmentEvidence {
    pub toc_page_id: String,
    pub toc_page_index: u32,
    pub toc_line_ids: Vec<String>,
    pub toc_word_ids: Vec<String>,
    pub body_page_id: Option<String>,
    pub body_page_index: Option<u32>,
    pub body_line_id: Option<String>,
    pub printed_label_raw: Option<String>,
    pub printed_number: Option<u32>,
    pub numbering_family: Option<String>,
    pub mapping_segment_index: Option<u32>,
    pub mapping_offset: Option<i64>,
    pub page_residual: Option<i64>,
    pub runner_up_margin: u32,
    pub column_index: u32,
    pub merged_toc_lines: u32,
    pub toc_has_leader: bool,
    pub body_human_revised: bool,
    pub secondary_key_only: bool,
    pub geometry_quality: String,
    pub min_toc_word_confidence: f32,
    pub min_body_word_confidence: f32,
}

/// Provenance of an automatic decision, kept even after a human overrides it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomaticDecision {
    pub decided_status: String,
    pub reason: String,
    pub rule_version: String,
    pub rule_config_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BookmarkReviews {
    pub schema: String,
    pub schema_version: String,
    pub base_generation_digest: String,
    pub operations: Vec<ReviewOperation>,
}
impl BookmarkReviews {
    pub fn empty(base: String) -> Self {
        Self {
            schema: "mpdf-bookmark-reviews".into(),
            schema_version: BOOKMARK_SCHEMA_VERSION.into(),
            base_generation_digest: base,
            operations: vec![],
        }
    }
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.schema != "mpdf-bookmark-reviews"
            || !BOOKMARK_SCHEMA_VERSIONS.contains(&self.schema_version.as_str())
            || !digest(&self.base_generation_digest)
            || self.operations.len() > MAX_REVIEWS
        {
            return Err(crate::error::CoreError::InvalidDocument(
                "invalid bookmark review store".into(),
            ));
        }
        let mut ids = std::collections::HashSet::new();
        for op in &self.operations {
            if op.operation_id.is_empty()
                || !ids.insert(&op.operation_id)
                || op.candidate_id.is_empty()
            {
                return Err(crate::error::CoreError::InvalidDocument(
                    "invalid bookmark review operation".into(),
                ));
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewOperation {
    pub operation_id: String,
    pub candidate_id: String,
    pub operation: ReviewAction,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewAction {
    Confirm,
    Reject,
    Edit {
        title: String,
    },
    Reparent {
        parent_id: Option<String>,
        level: u16,
    },
}

#[cfg(test)]
pub(crate) mod schema_tests {
    use serde_json::Value;

    fn assert_local_refs_resolve(value: &Value, root: &Value) {
        match value {
            Value::Object(map) => {
                if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                    if let Some(pointer) = reference.strip_prefix('#') {
                        assert!(
                            root.pointer(pointer).is_some(),
                            "unresolved schema ref {reference}"
                        );
                    }
                }
                for child in map.values() {
                    assert_local_refs_resolve(child, root);
                }
            }
            Value::Array(values) => {
                for child in values {
                    assert_local_refs_resolve(child, root);
                }
            }
            _ => {}
        }
    }

    const BOOKMARKS_0_1: &str = include_str!("../../../../schemas/mpdf-bookmarks-0.1.schema.json");
    const BOOKMARKS_0_2: &str = include_str!("../../../../schemas/mpdf-bookmarks-0.2.schema.json");
    const REVIEWS_0_1: &str =
        include_str!("../../../../schemas/mpdf-bookmark-reviews-0.1.schema.json");
    const REPORT_0_1: &str =
        include_str!("../../../../schemas/mpdf-bookmark-generation-report-0.1.schema.json");

    #[test]
    fn persistent_schemas_are_strict_and_have_no_dangling_local_refs() {
        for source in [BOOKMARKS_0_1, BOOKMARKS_0_2, REVIEWS_0_1, REPORT_0_1] {
            let schema: Value = serde_json::from_str(source).unwrap();
            assert_eq!(schema["additionalProperties"], false);
            assert_local_refs_resolve(&schema, &schema);
        }
    }

    /// Every key a serialized record can carry must be declared by the
    /// schema, and every schema-required key must actually be written. This
    /// catches a Rust field added without its schema counterpart, which a
    /// strict external validator would otherwise reject at read time.
    pub(crate) fn assert_conforms(value: &Value, schema: &Value, root: &Value, path: &str) {
        let schema = match schema.get("$ref").and_then(Value::as_str) {
            Some(reference) => root
                .pointer(reference.trim_start_matches('#'))
                .unwrap_or_else(|| panic!("unresolved ref {reference}")),
            None => schema,
        };
        let Some(object) = value.as_object() else {
            return;
        };
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for key in object.keys() {
                assert!(
                    properties.contains_key(key),
                    "{path}: schema does not declare {key}"
                );
            }
            for required in schema
                .get("required")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let key = required.as_str().unwrap_or_default();
                assert!(
                    object.contains_key(key),
                    "{path}: required key {key} is missing"
                );
            }
            for (key, child) in object {
                if let Some(child_schema) = properties.get(key) {
                    match child {
                        Value::Array(items) => {
                            if let Some(item_schema) = child_schema.get("items") {
                                for (index, item) in items.iter().enumerate() {
                                    assert_conforms(
                                        item,
                                        item_schema,
                                        root,
                                        &format!("{path}/{key}[{index}]"),
                                    );
                                }
                            }
                        }
                        Value::Object(_) => {
                            assert_conforms(child, child_schema, root, &format!("{path}/{key}"))
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    pub(crate) fn bookmarks_0_2_schema() -> Value {
        serde_json::from_str(BOOKMARKS_0_2).unwrap()
    }

    pub(crate) fn report_0_1_schema() -> Value {
        serde_json::from_str(REPORT_0_1).unwrap()
    }

    #[test]
    fn zero_two_schema_allows_the_automatic_statuses_and_zero_one_does_not() {
        let old: Value = serde_json::from_str(BOOKMARKS_0_1).unwrap();
        let new = bookmarks_0_2_schema();
        let statuses = |schema: &Value| -> Vec<String> {
            schema["$defs"]["candidate"]["properties"]["status"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect()
        };
        assert!(!statuses(&old).contains(&"auto_confirmed".to_owned()));
        assert!(statuses(&new).contains(&"auto_confirmed".to_owned()));
        assert!(statuses(&new).contains(&"skipped".to_owned()));
        assert_eq!(old["properties"]["schema_version"]["const"], "0.1");
        assert_eq!(new["properties"]["schema_version"]["const"], "0.2");
    }
}
