use crate::derived::Bbox;
use serde::{Deserialize, Serialize};

pub const BOOKMARK_SCHEMA: &str = "mpdf-bookmarks";
pub const BOOKMARK_SCHEMA_VERSION: &str = "0.1";
pub const MAX_CANDIDATES: usize = 100_000;
pub const MAX_REVIEWS: usize = 100_000;

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
}
impl BookmarkSnapshot {
    pub(crate) fn without_digest(&self) -> Self {
        let mut x = self.clone();
        x.generation_digest = String::new();
        x
    }
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.schema != BOOKMARK_SCHEMA
            || self.schema_version != BOOKMARK_SCHEMA_VERSION
            || self.candidates.len() > MAX_CANDIDATES
            || !digest(&self.source_digest)
            || !digest(&self.package_digest)
            || self.derived_digest.as_ref().is_some_and(|x| !digest(x))
            || self.generator.kind.is_empty()
            || self.generator.name.is_empty()
            || self.generator.version.is_empty()
        {
            return Err(crate::error::CoreError::InvalidDocument(
                "invalid bookmark snapshot".into(),
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
}
impl BookmarkCandidate {
    pub fn validate(&self) -> crate::error::Result<()> {
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
        if self.status == BookmarkStatus::Confirmed && self.generator.kind == "ai_suggestion" {
            return Err(crate::error::CoreError::InvalidDocument(
                "AI suggestions cannot be confirmed in M5".into(),
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
            || self.schema_version != BOOKMARK_SCHEMA_VERSION
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
mod schema_tests {
    fn assert_local_refs_resolve(value: &serde_json::Value, root: &serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(reference) = map.get("$ref").and_then(serde_json::Value::as_str) {
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
            serde_json::Value::Array(values) => {
                for child in values {
                    assert_local_refs_resolve(child, root);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn persistent_schemas_are_strict_and_have_no_dangling_local_refs() {
        for source in [
            include_str!("../../../../schemas/mpdf-bookmarks-0.1.schema.json"),
            include_str!("../../../../schemas/mpdf-bookmark-reviews-0.1.schema.json"),
        ] {
            let schema: serde_json::Value = serde_json::from_str(source).unwrap();
            assert_eq!(schema["additionalProperties"], false);
            assert_local_refs_resolve(&schema, &schema);
        }
    }
}
