use crate::document_package::{
    ExistingOutlineEvidence, PrintedPageLabel, Rect, RegionEvidence, TypographyEvidence,
};
use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};

pub const DERIVED_SCHEMA: &str = "mpdf-derived-document";
pub const DERIVED_SCHEMA_VERSION: &str = "0.1";
pub const DERIVED_EXPORTER_VERSION: &str = "0.1";
pub const MAX_DERIVED_CHUNKS: usize = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DerivedManifest {
    pub schema: String,
    pub schema_version: String,
    pub source_digest: String,
    pub document_id: String,
    pub package_digest: String,
    pub ocr_digest: Option<String>,
    pub revision_digest: String,
    pub exporter_version: String,
    pub artifacts: Vec<DerivedArtifact>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DerivedArtifact {
    pub format: String,
    pub path: String,
    pub sha256: String,
    pub byte_len: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleStatus {
    Current,
    Stale,
    Corrupt,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedDocument {
    pub manifest: DerivedManifest,
    pub pages: Vec<DerivedPage>,
    pub chunks: Vec<DerivedChunk>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedPage {
    pub page_id: String,
    pub page_index: u32,
    pub bbox: Bbox,
    pub coordinate_space: String,
    pub evidence_digest: String,
    pub blocks: Vec<DerivedBlock>,
    pub regions: Vec<DerivedRegion>,
    pub outline_evidence: Vec<OutlineEvidenceRef>,
    pub printed_page_label: Option<PrintedPageLabel>,
    pub existing_outline_evidence: Vec<ExistingOutlineEvidence>,
    pub typography_evidence: Vec<TypographyEvidence>,
    pub region_evidence: Vec<RegionEvidence>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedRegion {
    pub id: String,
    pub page_id: String,
    pub bbox: Bbox,
    pub kind: String,
    pub bounds: Rect,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutlineEvidenceRef {
    pub title: String,
    pub level: u16,
    pub target_page_id: Option<String>,
    pub source: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedBlock {
    pub id: String,
    pub page_id: String,
    pub bbox: Bbox,
    pub coordinate_space: String,
    pub structural_path: String,
    pub reading_order: u32,
    pub lines: Vec<DerivedLine>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedLine {
    pub id: String,
    pub page_id: String,
    pub bbox: Bbox,
    pub coordinate_space: String,
    pub structural_path: String,
    pub reading_order: u32,
    pub words: Vec<DerivedWord>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedWord {
    pub id: String,
    pub page_id: String,
    pub bbox: Bbox,
    pub coordinate_space: String,
    pub structural_path: String,
    pub source_text: String,
    pub source_normalized_text: String,
    pub effective_text: String,
    pub effective_normalized_text: String,
    pub text: String,
    pub normalized_text: String,
    pub confidence: f32,
    pub reading_order: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedChunk {
    pub id: String,
    pub document_id: String,
    pub page_id: String,
    pub page_index: u32,
    pub bbox: Bbox,
    pub coordinate_space: String,
    pub structural_path: String,
    pub constituent_word_refs: Vec<String>,
    pub text: String,
    pub reading_order: u32,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Bbox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bbox {
    pub fn finite(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width >= 0.0
            && self.height >= 0.0
    }
}
fn within(a: Bbox, b: Bbox) -> bool {
    a.x >= b.x && a.y >= b.y && a.x + a.width <= b.x + b.width && a.y + a.height <= b.y + b.height
}

impl DerivedDocument {
    pub fn validate(&self) -> Result<()> {
        if self.manifest.schema != DERIVED_SCHEMA
            || self.manifest.schema_version != DERIVED_SCHEMA_VERSION
            || self.pages.is_empty()
            || self.chunks.len() > MAX_DERIVED_CHUNKS
        {
            return Err(CoreError::InvalidDocument(
                "invalid derived document manifest".into(),
            ));
        }
        let pages: std::collections::HashSet<_> =
            self.pages.iter().map(|p| p.page_id.as_str()).collect();
        if pages.len() != self.pages.len() || self.manifest.document_id.is_empty() {
            return Err(CoreError::InvalidDocument(
                "duplicate or empty derived page ID".into(),
            ));
        }
        let mut ids = std::collections::HashSet::new();
        let mut words = std::collections::HashSet::new();
        for page in &self.pages {
            if !page.bbox.finite()
                || page.bbox.x != 0.0
                || page.bbox.y != 0.0
                || page.coordinate_space.is_empty()
            {
                return Err(CoreError::InvalidDocument(
                    "invalid derived page geometry".into(),
                ));
            }
            for region in &page.regions {
                if region.page_id != page.page_id
                    || region.bounds.space_id.is_empty()
                    || !region.bbox.finite()
                    || !within(region.bbox, page.bbox)
                    || !ids.insert(region.id.as_str())
                {
                    return Err(CoreError::InvalidDocument("invalid derived region".into()));
                }
            }
            for block in &page.blocks {
                if block.page_id != page.page_id
                    || block.coordinate_space != page.coordinate_space
                    || !block.bbox.finite()
                    || !within(block.bbox, page.bbox)
                    || !ids.insert(block.id.as_str())
                {
                    return Err(CoreError::InvalidDocument("invalid derived block".into()));
                }
                for line in &block.lines {
                    if line.page_id != page.page_id
                        || line.coordinate_space != page.coordinate_space
                        || !line.bbox.finite()
                        || !within(line.bbox, page.bbox)
                        || !ids.insert(line.id.as_str())
                    {
                        return Err(CoreError::InvalidDocument("invalid derived line".into()));
                    }
                    for word in &line.words {
                        if word.page_id != page.page_id
                            || word.coordinate_space != page.coordinate_space
                            || !word.bbox.finite()
                            || !within(word.bbox, page.bbox)
                            || !word.confidence.is_finite()
                            || !(0.0..=1.0).contains(&word.confidence)
                            || !words.insert(word.id.as_str())
                        {
                            return Err(CoreError::InvalidDocument("invalid derived word".into()));
                        }
                    }
                }
            }
        }
        let mut chunks = std::collections::HashSet::new();
        for chunk in &self.chunks {
            if !chunks.insert(chunk.id.as_str())
                || chunk.document_id != self.manifest.document_id
                || !pages.contains(chunk.page_id.as_str())
                || !chunk.bbox.finite()
                || chunk.constituent_word_refs.is_empty()
                || chunk
                    .constituent_word_refs
                    .iter()
                    .any(|id| !words.contains(id.as_str()))
            {
                return Err(CoreError::InvalidDocument("invalid derived chunk".into()));
            }
        }
        Ok(())
    }
}
