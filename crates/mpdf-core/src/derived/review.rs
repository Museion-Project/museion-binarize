use super::{Bbox, DerivedDocument};
use crate::error::Result;
use serde::{Deserialize, Serialize};
use sha2::Digest;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewIssue {
    pub issue_id: String,
    pub target_ref: String,
    pub page_id: String,
    pub page_index: u32,
    pub bbox: Bbox,
    pub base_evidence_digest: String,
    pub kind: ReviewIssueKind,
    pub severity: ReviewSeverity,
    pub reason: String,
    pub status: ReviewStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewIssueKind {
    LowConfidence,
    ReadingOrderGap,
    UnicodeNormalization,
    EmptyRegion,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Info,
    Warning,
    Error,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Open,
}
pub fn review_queue(document: &DerivedDocument) -> Result<Vec<ReviewIssue>> {
    document.validate()?;
    let mut out = vec![];
    for p in &document.pages {
        if p.blocks.is_empty() {
            out.push(issue(
                "empty-region",
                &p.page_id,
                p.page_index,
                p.bbox,
                &p.evidence_digest,
                ReviewIssueKind::EmptyRegion,
                ReviewSeverity::Warning,
                "page has no structured text evidence",
            ));
        }
        for b in &p.blocks {
            for l in &b.lines {
                for (i, w) in l.words.iter().enumerate() {
                    if w.reading_order != i as u32 {
                        out.push(issue(
                            "reading-order",
                            &w.id,
                            p.page_index,
                            w.bbox,
                            &p.evidence_digest,
                            ReviewIssueKind::ReadingOrderGap,
                            ReviewSeverity::Warning,
                            "reading order is not contiguous",
                        ));
                    }
                    if w.confidence < 0.75 {
                        out.push(issue(
                            "low-confidence",
                            &w.id,
                            p.page_index,
                            w.bbox,
                            &p.evidence_digest,
                            ReviewIssueKind::LowConfidence,
                            ReviewSeverity::Warning,
                            "word confidence is below 0.75",
                        ));
                    }
                    if w.source_text != w.source_normalized_text {
                        out.push(issue(
                            "unicode-normalization",
                            &w.id,
                            p.page_index,
                            w.bbox,
                            &p.evidence_digest,
                            ReviewIssueKind::UnicodeNormalization,
                            ReviewSeverity::Info,
                            "original and normalized Unicode text differ",
                        ));
                    }
                }
            }
        }
    }
    for i in &mut out {
        if let Some(w) = document
            .pages
            .iter()
            .flat_map(|p| p.blocks.iter())
            .flat_map(|b| b.lines.iter())
            .flat_map(|l| l.words.iter())
            .find(|w| w.id == i.target_ref)
        {
            i.page_id = w.page_id.clone();
            i.coordinate_space = Some(w.coordinate_space.clone());
            i.source_text = Some(w.source_text.clone());
            i.effective_text = Some(w.effective_text.clone());
            i.confidence = Some(w.confidence);
        }
    }
    out.sort_by(|a, b| a.issue_id.cmp(&b.issue_id));
    Ok(out)
}
#[allow(clippy::too_many_arguments)]
fn issue(
    k: &str,
    target: &str,
    page: u32,
    bbox: Bbox,
    digest: &str,
    kind: ReviewIssueKind,
    severity: ReviewSeverity,
    reason: &str,
) -> ReviewIssue {
    let hash = sha2::Sha256::digest(format!("{k}\0{target}\0{page}").as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    ReviewIssue {
        issue_id: format!("issue-{}", &hash[..24]),
        target_ref: target.into(),
        page_id: target.into(),
        page_index: page,
        bbox,
        base_evidence_digest: digest.into(),
        kind,
        severity,
        reason: reason.into(),
        status: ReviewStatus::Open,
        coordinate_space: None,
        source_text: None,
        effective_text: None,
        confidence: None,
    }
}
