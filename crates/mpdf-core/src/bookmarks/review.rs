use super::*;
use crate::error::{CoreError, Result};
use sha2::{Digest, Sha256};

/// Computes the effective view without mutating the immutable generation.
pub fn effective(
    snapshot: &BookmarkSnapshot,
    reviews: &BookmarkReviews,
) -> Result<Vec<BookmarkCandidate>> {
    snapshot.validate()?;
    reviews.validate()?;
    if reviews.base_generation_digest != snapshot.generation_digest {
        return Err(CoreError::InvalidDocument(
            "stale bookmark review generation".into(),
        ));
    }
    let mut out = snapshot.candidates.clone();
    for op in &reviews.operations {
        let Some(c) = out.iter_mut().find(|c| c.candidate_id == op.candidate_id) else {
            return Err(CoreError::InvalidDocument(
                "review references missing candidate".into(),
            ));
        };
        match &op.operation {
            ReviewAction::Confirm => {
                if c.generator.kind == "ai_suggestion" {
                    return Err(CoreError::InvalidDocument(
                        "AI suggestions cannot be applied".into(),
                    ));
                }
                c.status = BookmarkStatus::Confirmed;
            }
            ReviewAction::Reject => c.status = BookmarkStatus::Rejected,
            ReviewAction::Edit { title } => {
                if title.trim().is_empty() {
                    return Err(CoreError::InvalidParameter(
                        "bookmark title must not be empty".into(),
                    ));
                }
                c.effective_title = title.clone();
            }
            ReviewAction::Reparent { parent_id, level } => {
                if *level > 64 {
                    return Err(CoreError::InvalidParameter(
                        "bookmark level exceeds 64".into(),
                    ));
                }
                c.effective_parent_id = parent_id.clone();
                c.effective_level = *level;
            }
        }
    }
    let ids: std::collections::HashSet<_> = out.iter().map(|c| c.candidate_id.as_str()).collect();
    for c in &out {
        if let Some(p) = &c.effective_parent_id {
            if p == &c.candidate_id || !ids.contains(p.as_str()) {
                return Err(CoreError::InvalidDocument(
                    "bookmark parent is missing or cyclic".into(),
                ));
            }
            let parent = out
                .iter()
                .find(|candidate| candidate.candidate_id == *p)
                .ok_or_else(|| CoreError::InvalidDocument("bookmark parent is missing".into()))?;
            if parent.effective_level >= c.effective_level {
                return Err(CoreError::InvalidDocument(
                    "bookmark parent level must be shallower than its child".into(),
                ));
            }
            let mut seen = std::collections::HashSet::new();
            let mut cur = Some(p.as_str());
            while let Some(id) = cur {
                if !seen.insert(id) {
                    return Err(CoreError::InvalidDocument(
                        "bookmark hierarchy contains a cycle".into(),
                    ));
                }
                cur = out
                    .iter()
                    .find(|x| x.candidate_id == id)
                    .and_then(|x| x.effective_parent_id.as_deref());
            }
        }
    }
    out.sort_by(|a, b| {
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
                a.effective_title
                    .to_lowercase()
                    .cmp(&b.effective_title.to_lowercase())
            })
            .then_with(|| a.candidate_id.cmp(&b.candidate_id))
    });
    Ok(out)
}
pub fn append(
    snapshot: &BookmarkSnapshot,
    reviews: &mut BookmarkReviews,
    candidate_id: String,
    action: ReviewAction,
) -> Result<()> {
    snapshot.validate()?;
    reviews.validate()?;
    if reviews.base_generation_digest != snapshot.generation_digest {
        return Err(CoreError::InvalidDocument(
            "stale bookmark review generation".into(),
        ));
    }
    if !snapshot
        .candidates
        .iter()
        .any(|c| c.candidate_id == candidate_id)
    {
        return Err(CoreError::InvalidDocument(
            "review candidate does not exist".into(),
        ));
    }
    let operation_id = operation_id(&candidate_id, &action, reviews.operations.len());
    let mut next = reviews.clone();
    next.operations.push(ReviewOperation {
        operation_id,
        candidate_id,
        operation: action,
    });
    effective(snapshot, &next)?;
    *reviews = next;
    Ok(())
}
fn operation_id(id: &str, a: &ReviewAction, ordinal: usize) -> String {
    let mut h = Sha256::new();
    h.update(id);
    h.update(serde_json::to_vec(a).unwrap());
    h.update(ordinal.to_le_bytes());
    let digest = h
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!("review-{}", &digest[..32])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_keeps_source_title_and_evidence() {
        let c = BookmarkCandidate {
            candidate_id: "bookmark-a".into(),
            source_title: "Original".into(),
            effective_title: "Original".into(),
            source_level: 1,
            effective_level: 1,
            source_parent_id: None,
            effective_parent_id: None,
            target_page_id: "page".into(),
            physical_page_index: 0,
            master_bbox: None,
            outline_evidence: None,
            evidence: vec![EvidenceRef::DerivedPage {
                page_id: "page".into(),
                bbox: Bbox {
                    x: 0.,
                    y: 0.,
                    width: 1.,
                    height: 1.,
                },
            }],
            confidence: 1.,
            status: BookmarkStatus::Proposed,
            generator: GeneratorProvenance {
                kind: "deterministic_rules".into(),
                name: "x".into(),
                version: "1".into(),
            },
            reason_codes: vec![],
            rule_trace: vec![],
        };
        let mut s = BookmarkSnapshot {
            schema: BOOKMARK_SCHEMA.into(),
            schema_version: BOOKMARK_SCHEMA_VERSION.into(),
            source_digest: "a".repeat(64),
            package_digest: "b".repeat(64),
            derived_digest: None,
            generator: c.generator.clone(),
            candidates: vec![c],
            generation_digest: "c".repeat(64),
        };
        s.generation_digest = s.recomputed_generation_digest();
        let mut r = BookmarkReviews::empty(s.generation_digest.clone());
        append(
            &s,
            &mut r,
            "bookmark-a".into(),
            ReviewAction::Edit {
                title: "Human".into(),
            },
        )
        .unwrap();
        let e = effective(&s, &r).unwrap();
        assert_eq!(e[0].source_title, "Original");
        assert_eq!(e[0].effective_title, "Human");
        s.generation_digest = "d".repeat(64);
        assert!(effective(&s, &r).is_err());
    }
}
