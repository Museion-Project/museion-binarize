//! Provider-independent, bookmark-only input. Raw provider observations are kept
//! separately by the caller; these are measured visual lines in visible PDF points.
#[path = "local_grammar.rs"]
mod grammar;
use super::{config::AutoBookmarkConfig, text_index::*, toc_detect::CONTENTS_KEYWORDS, toc_parse};
use crate::derived::Bbox;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    NativeText,
    AppleVisionFast,
    SuryaGeometry,
    Manual,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceState {
    Inspected,
    LocallyRecognized,
    Uninspected,
    Failed,
    Ambiguous,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkLine {
    pub id: String,
    pub group_id: String,
    pub raw_ids: Vec<String>,
    pub text: String,
    pub bbox: Bbox,
    pub size_proxy: Option<f64>,
    pub confidence: Option<f32>,
    pub source_kind: SourceKind,
    pub state: EvidenceState,
    pub printed_candidate: Option<String>,
    #[serde(default)]
    pub geometry_support_ids: Vec<String>,
    #[serde(default)]
    pub title_start_x: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeometrySupport {
    pub id: String,
    pub bbox: Bbox,
    pub source_kind: SourceKind,
    pub state: EvidenceState,
    pub reading_order: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkEvidence {
    pub schema: String,
    pub source: String,
    pub source_sha256: String,
    pub page_index: u32,
    pub page_count: u32,
    pub width: f64,
    pub height: f64,
    /// ROI in visible PDF points. Affine maps input coordinates into that space.
    pub source_roi: Bbox,
    pub coordinate_transform: [f64; 6],
    pub raw_path: String,
    pub lines: Vec<BookmarkLine>,
    #[serde(default)]
    pub geometry_supports: Vec<GeometrySupport>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalEntry {
    pub id: String,
    pub title: String,
    pub section_label: Option<String>,
    pub printed_page: Option<String>,
    pub printed_value: Option<u32>,
    pub printed_family: Option<String>,
    pub source_page: u32,
    pub source_bbox: Bbox,
    pub evidence_ids: Vec<String>,
    pub parent: Option<String>,
    pub level: u16,
    pub hierarchy_reason: String,
    #[serde(default)]
    pub label_hypotheses: Vec<Option<String>>,
    #[serde(default)]
    pub parent_hypotheses: Vec<Option<String>>,
    pub target_pdf_page: Option<u32>,
    pub review_reasons: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalCompilation {
    pub schema: String,
    pub source: String,
    pub source_sha256: String,
    pub page_count: u32,
    pub entries: Vec<LocalEntry>,
    /// Immutable source-tree interpretation; entries is its navigation projection.
    #[serde(default)]
    pub source_entries: Vec<LocalEntry>,
    #[serde(default)]
    pub navigation_projection: Vec<ProjectionDecision>,
    pub excluded_headers: Vec<String>,
    pub fallback_reasons: Vec<String>,
    pub parse_seconds: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionDecision {
    pub source_id: String,
    pub action: String,
    pub reason: String,
    pub requires_review: bool,
}

/// Drop only unpaginated, numbered/explicit containers with actual children.
/// Missing-number leaves survive. This is a reviewable navigation convention,
/// never a claim that the source container has no printed target in the image.
fn project_navigation(source: &[LocalEntry]) -> (Vec<LocalEntry>, Vec<ProjectionDecision>) {
    let dropped: std::collections::HashSet<_> = source
        .iter()
        .filter(|e| {
            e.printed_page.is_none()
                && e.section_label.is_some()
                && source
                    .iter()
                    .any(|child| child.parent.as_ref() == Some(&e.id))
        })
        .map(|e| e.id.clone())
        .collect();
    let by_id: std::collections::HashMap<_, _> = source.iter().map(|e| (e.id.clone(), e)).collect();
    let mut entries: Vec<LocalEntry> = vec![];
    for e in source.iter().filter(|e| !dropped.contains(&e.id)) {
        let mut projected = e.clone();
        while projected
            .parent
            .as_ref()
            .is_some_and(|p| dropped.contains(p))
        {
            projected.parent = by_id[projected.parent.as_ref().unwrap()].parent.clone();
        }
        projected.level = projected
            .parent
            .as_ref()
            .map(|p| {
                entries
                    .iter()
                    .find(|e| &e.id == p)
                    .expect("source parent precedes child")
                    .level
                    + 1
            })
            .unwrap_or(0);
        // Parent alternatives belong to the source interpretation, explicitly
        // retained in source_entries; project each alternative into navigation.
        for parent in &mut projected.parent_hypotheses {
            while parent.as_ref().is_some_and(|p| dropped.contains(p)) {
                *parent = by_id[parent.as_ref().unwrap()].parent.clone();
            }
        }
        projected.parent_hypotheses.sort();
        projected.parent_hypotheses.dedup();
        if projected.parent != e.parent {
            projected
                .review_reasons
                .push("container_projection_requires_review".into());
        }
        entries.push(projected);
    }
    let decisions = source
        .iter()
        .filter(|e| dropped.contains(&e.id))
        .map(|e| ProjectionDecision {
            source_id: e.id.clone(),
            action: "omit_container_promote_children".into(),
            reason: "unpaginated_numbered_container_navigation_convention".into(),
            requires_review: true,
        })
        .collect();
    (entries, decisions)
}

/// Uses the existing Museion TOC and hierarchy compiler, without manufacturing
/// an OCR run or marking approximate native geometry as measured.
pub fn compile_local(pages: &[BookmarkEvidence]) -> Result<LocalCompilation, String> {
    let start = std::time::Instant::now();
    let first = pages.first().ok_or("no evidence pages")?;
    let mut result = LocalCompilation {
        schema: "mpdf-bookmark-table/1".into(),
        source: first.source.clone(),
        source_sha256: first.source_sha256.clone(),
        page_count: first.page_count,
        entries: vec![],
        source_entries: vec![],
        navigation_projection: vec![],
        excluded_headers: vec![],
        fallback_reasons: vec![],
        parse_seconds: 0.,
    };
    let config = AutoBookmarkConfig::default();
    let mut drafts = vec![];
    for p in pages {
        if p.schema != "mpdf-bookmark-evidence/1"
            || p.source_sha256 != first.source_sha256
            || p.source != first.source
            || p.page_count != first.page_count
            || p.page_index >= p.page_count
            || !p.width.is_finite()
            || !p.height.is_finite()
            || p.width <= 0.
            || p.height <= 0.
        {
            return Err("inconsistent evidence source or dimensions".into());
        }
        let mut hs: Vec<f64> = p
            .lines
            .iter()
            .map(|l| l.bbox.height)
            .filter(|v| v.is_finite() && *v > 0.)
            .collect();
        hs.sort_by(f64::total_cmp);
        let median = hs.get(hs.len() / 2).copied().unwrap_or(1.);
        let mut lines = vec![];
        let mut ids = std::collections::HashSet::new();
        for (i, l) in p.lines.iter().enumerate() {
            let b = l.bbox;
            if l.title_start_x
                .is_some_and(|x| !x.is_finite() || x < b.x || x > b.x + b.width)
            {
                return Err("invalid title start geometry".into());
            }
            if !ids.insert(&l.id)
                || [b.x, b.y, b.width, b.height].iter().any(|v| !v.is_finite())
                || b.width <= 0.
                || b.height <= 0.
                || b.x < 0.
                || b.y < 0.
                || b.x + b.width > p.width + 0.1
                || b.y + b.height > p.height + 0.1
            {
                return Err("invalid measured line bbox/id".into());
            }
            if matches!(l.state, EvidenceState::Failed | EvidenceState::Uninspected) {
                result
                    .fallback_reasons
                    .push(format!("page {}: unusable evidence {}", p.page_index, l.id));
                continue;
            }
            let supports: Vec<_> = l
                .geometry_support_ids
                .iter()
                .map(|id| {
                    p.geometry_supports
                        .iter()
                        .find(|s| s.id == *id)
                        .ok_or("unresolved geometry support")
                })
                .collect::<Result<_, _>>()?;
            let measured = if supports.len() == 1 && supports[0].state == EvidenceState::Inspected {
                supports[0].bbox
            } else {
                b
            };
            if [measured.x, measured.y, measured.width, measured.height]
                .iter()
                .any(|v| !v.is_finite())
                || measured.width <= 0.
                || measured.height <= 0.
                || measured.x < 0.
                || measured.y < 0.
                || measured.x + measured.width > p.width + 0.1
                || measured.y + measured.height > p.height + 0.1
            {
                return Err("invalid geometry support bbox".into());
            }
            let key = primary_key(&l.text);
            // Header filtering precedes grouping. Dictionary is gated by visual
            // top/size/centering evidence and absence of a trailing page label.
            let centered = ((b.x + b.width / 2.) / p.width - 0.5).abs() < 0.16;
            if CONTENTS_KEYWORDS.contains(&key.as_str())
                && toc_parse::trailing_printed_number(&l.text).is_none()
                && b.y < p.height * 0.30
                && (centered || b.height >= median * 1.05 || b.y < p.height * 0.20)
            {
                result.excluded_headers.push(l.id.clone());
                continue;
            }
            if b.y < p.height * 0.13
                && (l.text.split_whitespace().count() == 1
                    && toc_parse::trailing_printed_number(&l.text).is_some()
                    || key.starts_with("page "))
            {
                result.excluded_headers.push(l.id.clone());
                continue;
            }
            let secondary = secondary_key(&key);
            lines.push(EvidenceLine {
                page_id: format!("p{}", p.page_index),
                page_index: p.page_index,
                line_id: l.id.clone(),
                ordinal: i as u32,
                raw_text: l.text.clone(),
                source_text: l.text.clone(),
                tokens: index_keys(&key, &secondary),
                primary_key: key,
                secondary_key: secondary,
                bbox: measured,
                min_confidence: l.confidence.unwrap_or(1.),
                words: vec![],
                geometry: GeometryQuality::Measured,
                top_permille: (b.y / p.height * 1000.) as u32,
                repeated_furniture: false,
            });
        }
        let page = EvidencePage {
            page_id: format!("p{}", p.page_index),
            page_index: p.page_index,
            width: p.width,
            height: p.height,
            lines,
            median_line_height: median,
            geometry: GeometryQuality::Measured,
            degraded_order: false,
        };
        // Explicit group boundaries in the projection prevent a missing page
        // label from swallowing the next entry. Each group uses the SAME parser.
        let mut groups: Vec<String> = vec![];
        for l in &p.lines {
            if !groups.contains(&l.group_id) {
                groups.push(l.group_id.clone())
            }
        }
        for group in groups {
            let selected: Vec<_> = page
                .lines
                .iter()
                .filter(|l| {
                    p.lines
                        .iter()
                        .any(|x| x.id == l.line_id && x.group_id == group)
                })
                .cloned()
                .collect();
            if selected.is_empty() {
                continue;
            }
            // Consensus comes from measured page-number lanes in the projection;
            // no fabricated leaders. Parse whole page then retain this group would
            // cross boundaries; use a single grouped line with explicit lane tail.
            let mut sub = page.clone();
            sub.lines = selected;
            let mut ds = toc_parse::parse_toc_page(&sub, &config);
            for d in &mut ds {
                let originals: Vec<_> = p
                    .lines
                    .iter()
                    .filter(|l| d.line_ids.contains(&l.id))
                    .collect();
                let candidate = originals.iter().find_map(|l| l.printed_candidate.as_ref());
                d.printed = candidate.and_then(|s| toc_parse::trailing_printed_number(s));
                let text = originals
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                d.raw_title = toc_parse::strip_printed_tail(&text, d.printed.as_ref());
                let (prefix, path) = toc_parse::numbering_prefix(&d.raw_title);
                d.numbering_prefix = prefix;
                d.numbering_path = path;
                if let Some(size) = originals.first().and_then(|l| l.size_proxy) {
                    d.title_line_height = size;
                }
                d.reason_codes.retain(|r| {
                    r != "toc_no_printed_page" && r != "printed_page_without_consensus"
                });
            }
            drafts.extend(ds);
        }
    }
    let input: Vec<_> = drafts
        .iter()
        .map(|d| {
            let width = pages
                .iter()
                .find(|p| p.page_index == d.page_index)
                .unwrap()
                .width;
            let title_x = pages
                .iter()
                .find(|p| p.page_index == d.page_index)
                .and_then(|p| p.lines.iter().find(|l| d.line_ids.contains(&l.id)))
                .and_then(|l| l.title_start_x)
                .unwrap_or(d.bbox.x);
            (d.raw_title.clone(), title_x / width, d.printed.is_some())
        })
        .collect();
    let levels = grammar::resolve(&input);
    let ids: Vec<_> = (0..drafts.len()).map(|i| format!("entry-{i}")).collect();
    let parents: Vec<_> = levels
        .iter()
        .map(|l| l.parent.map(|i| ids[i].clone()))
        .collect();
    let mut previous: Option<(String, u32)> = None;
    for (i, d) in drafts.into_iter().enumerate() {
        let mut reasons = d.reason_codes.clone();
        if levels[i].suspect {
            reasons.push("hierarchy_ambiguous".into())
        }
        let source = pages.iter().find(|p| p.page_index == d.page_index).unwrap();
        let members: Vec<_> = source
            .lines
            .iter()
            .filter(|l| d.line_ids.contains(&l.id))
            .collect();
        if members.iter().any(|l| l.state == EvidenceState::Ambiguous) {
            reasons.push("projection_ambiguous".into())
        }
        if let Some(n) = &d.printed {
            if n.value > first.page_count {
                reasons.push("printed_out_of_range".into())
            }
            if previous
                .as_ref()
                .is_some_and(|(f, v)| f == n.family.as_str() && *v > n.value)
            {
                reasons.push("printed_nonmonotonic".into())
            }
            if previous.as_ref().is_some_and(|(f, v)| {
                f == n.family.as_str() && n.value.saturating_sub(*v) > first.page_count / 3
            }) {
                reasons.push("printed_large_neighbor_gap".into())
            }
            previous = Some((n.family.as_str().into(), n.value));
        } else {
            reasons.push("printed_missing".into())
        }
        reasons.push("pagination_uninspected".into());
        reasons.sort();
        reasons.dedup();
        result.entries.push(LocalEntry {
            id: ids[i].clone(),
            title: d.raw_title,
            section_label: levels[i].label.clone(),
            printed_page: d.printed.as_ref().map(|n| n.raw.clone()),
            printed_value: d.printed.as_ref().map(|n| n.value),
            printed_family: d.printed.as_ref().map(|n| n.family.as_str().into()),
            source_page: d.page_index,
            source_bbox: d.bbox,
            evidence_ids: d.line_ids,
            parent: parents[i].clone(),
            level: levels[i].level,
            hierarchy_reason: levels[i].reason.clone(),
            label_hypotheses: levels[i].label_hypotheses.clone(),
            parent_hypotheses: levels[i]
                .parent_hypotheses
                .iter()
                .map(|p| p.map(|j| ids[j].clone()))
                .collect(),
            target_pdf_page: None,
            review_reasons: reasons,
        });
    }
    result.source_entries = result.entries.clone();
    (result.entries, result.navigation_projection) = project_navigation(&result.source_entries);
    result.parse_seconds = start.elapsed().as_secs_f64();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evidence() -> BookmarkEvidence {
        serde_json::from_value(serde_json::json!({
            "schema":"mpdf-bookmark-evidence/1", "source":"source.pdf", "source_sha256":"abc",
            "page_index":0,"page_count":100,"width":600.,"height":800.,
            "source_roi":{"x":0.,"y":0.,"width":600.,"height":800.},
            "coordinate_transform":[1.,0.,0.,1.,0.,0.],"raw_path":"raw.json",
            "lines":[
              {"id":"header","group_id":"header","raw_ids":["r0"],"text":"Inhaltsverzeichnis","bbox":{"x":200.,"y":70.,"width":200.,"height":24.},"size_proxy":24.,"confidence":1.,"source_kind":"apple_vision_fast","state":"locally_recognized","printed_candidate":null},
              {"id":"first","group_id":"first","raw_ids":["r1"],"text":"1. First subject 10","bbox":{"x":70.,"y":110.,"width":460.,"height":12.},"size_proxy":12.,"confidence":1.,"source_kind":"native_text","state":"inspected","printed_candidate":"10"},
              {"id":"second","group_id":"second","raw_ids":["r2"],"text":"1.1. Child subject 15","bbox":{"x":85.,"y":135.,"width":445.,"height":12.},"size_proxy":12.,"confidence":1.,"source_kind":"native_text","state":"inspected","printed_candidate":"15"}
            ]
        })).unwrap()
    }
    #[test]
    fn header_precedes_grouping_and_numbering_drives_tree() {
        let c = compile_local(&[evidence()]).unwrap();
        assert_eq!(c.excluded_headers, vec!["header"]);
        assert_eq!(c.entries.len(), 2);
        assert_eq!(c.entries[0].printed_value, Some(10));
        assert_eq!(c.entries[1].parent.as_deref(), Some("entry-0"));
        assert!(c.entries.iter().all(|e| e.target_pdf_page.is_none()));
    }
    #[test]
    fn raw_title_numeral_without_lane_is_not_a_page() {
        let mut p = evidence();
        p.lines[1].printed_candidate = None;
        let c = compile_local(&[p]).unwrap();
        assert!(c.source_entries[0].printed_page.is_none());
        assert!(c.source_entries[0].title.ends_with("10"));
        assert!(c.navigation_projection[0].requires_review);
    }
    #[test]
    fn failed_evidence_and_cross_document_input_cannot_be_promoted() {
        let mut p = evidence();
        p.lines[1].state = EvidenceState::Failed;
        let c = compile_local(&[p.clone()]).unwrap();
        assert_eq!(c.entries.len(), 1);
        assert!(!c.fallback_reasons.is_empty());
        p.source_sha256 = "other".into();
        assert!(compile_local(&[evidence(), p]).is_err());
    }
    #[test]
    fn geometry_fallback_must_resolve_to_measured_support() {
        let mut p = evidence();
        p.lines[1].geometry_support_ids = vec!["missing".into()];
        assert!(compile_local(&[p]).is_err());
    }
    #[test]
    fn navigation_projection_preserves_container_and_missing_leaf_evidence() {
        let mut p = evidence();
        p.lines[1].text = "1. Container".into();
        p.lines[1].printed_candidate = None;
        p.lines[2].printed_candidate = None;
        let c = compile_local(&[p]).unwrap();
        assert_eq!(c.source_entries.len(), 2);
        assert_eq!(c.source_entries[1].parent.as_deref(), Some("entry-0"));
        assert_eq!(c.source_entries[1].level, 1);
        assert_eq!(c.entries.len(), 1);
        assert_eq!(c.entries[0].id, "entry-1");
        assert_eq!(c.entries[0].parent, None);
        assert!(c.entries[0].printed_page.is_none());
        assert!(c.navigation_projection[0].requires_review);
        assert!(c.entries[0]
            .review_reasons
            .iter()
            .any(|r| r == "container_projection_requires_review"));
    }
    #[test]
    fn invalid_title_geometry_is_rejected() {
        let mut p = evidence();
        p.lines[1].title_start_x = Some(599.);
        assert!(compile_local(&[p]).is_err());
    }
}
