//! Front-matter printed-contents detection.
//!
//! Detection is multi-signal and integer-scored. A single "contents" word on
//! a copyright page can never make a page a table of contents on its own:
//! without a keyword a page needs several parsable entries *and* agreeing
//! leader or right-edge evidence.

use std::collections::BTreeMap;

use super::config::AutoBookmarkConfig;
use super::text_index::{secondary_key, EvidencePage, GeometryQuality, TextIndex};
use super::toc_parse::{has_leader_run, trailing_printed_number};

/// Normalized contents keywords. Compared against the primary and the
/// accent-folded secondary key, so `Índice`/`Indice` and
/// `matières`/`matieres` both hit.
pub(crate) const CONTENTS_KEYWORDS: [&str; 12] = [
    "contents",
    "table of contents",
    "目录",
    "目次",
    "sommaire",
    "indice",
    "índice",
    "inhalt",
    "inhaltsverzeichnis",
    "table des matières",
    "sumário",
    "содержание",
];

#[derive(Debug, Clone)]
pub(crate) struct TocPageDetection {
    pub(crate) page_id: String,
    pub(crate) page_index: u32,
    pub(crate) score: u32,
    pub(crate) signals: BTreeMap<String, u32>,
    pub(crate) keyword_hit: bool,
    pub(crate) entry_count: u32,
    pub(crate) keyword_line_ids: Vec<String>,
    pub(crate) entry_line_ids: Vec<String>,
}

fn keyword_hit(page: &EvidencePage) -> Option<String> {
    for line in page.lines.iter().filter(|line| !line.is_blank()) {
        if line.primary_key.chars().count() > 40 {
            continue;
        }
        for keyword in CONTENTS_KEYWORDS {
            if line.primary_key == keyword
                || line.secondary_key == secondary_key(keyword)
                || (line.primary_key.starts_with(keyword)
                    && line.primary_key.split_whitespace().count()
                        <= keyword.split_whitespace().count() + 1)
            {
                return Some(line.line_id.clone());
            }
        }
    }
    None
}

fn score_page(page: &EvidencePage, config: &AutoBookmarkConfig) -> TocPageDetection {
    let mut signals = BTreeMap::new();
    let mut score = 0u32;
    let keyword_line = keyword_hit(page);
    if keyword_line.is_some() {
        score += config.toc_keyword_score;
        signals.insert("contents_keyword".to_owned(), config.toc_keyword_score);
    }
    let mut entry_line_ids = Vec::new();
    let mut leader_lines = 0u32;
    let mut right_edges: Vec<f64> = Vec::new();
    let mut short_lines = 0u32;
    let mut content_lines = 0u32;
    for line in page.lines.iter().filter(|line| !line.is_blank()) {
        content_lines += 1;
        if line.primary_key.chars().count() <= 60 {
            short_lines += 1;
        }
        if has_leader_run(&line.raw_text) {
            leader_lines += 1;
        }
        if trailing_printed_number(&line.raw_text).is_some() {
            entry_line_ids.push(line.line_id.clone());
            right_edges.push(line.bbox.x + line.bbox.width);
        }
    }
    let entry_count = entry_line_ids.len() as u32;
    let entry_score = (entry_count * config.toc_entry_score).min(config.toc_entry_score_cap);
    if entry_score > 0 {
        score += entry_score;
        signals.insert("parsable_entries".to_owned(), entry_score);
    }
    if leader_lines >= 2 {
        score += config.toc_leader_score;
        signals.insert("dot_leaders".to_owned(), config.toc_leader_score);
    }
    let clustered = page.geometry == GeometryQuality::Measured
        && right_edges.len() >= 3
        && right_edge_spread(&mut right_edges) <= page.width * 0.05;
    if clustered {
        score += config.toc_right_cluster_score;
        signals.insert(
            "right_edge_cluster".to_owned(),
            config.toc_right_cluster_score,
        );
    }
    if content_lines >= 4 && short_lines * 10 >= content_lines * 6 {
        score += config.toc_short_line_score;
        signals.insert("short_line_ratio".to_owned(), config.toc_short_line_score);
    }
    TocPageDetection {
        page_id: page.page_id.clone(),
        page_index: page.page_index,
        score,
        signals,
        keyword_hit: keyword_line.is_some(),
        entry_count,
        keyword_line_ids: keyword_line.into_iter().collect(),
        entry_line_ids,
    }
}

fn right_edge_spread(edges: &mut [f64]) -> f64 {
    edges.sort_by(|a, b| a.total_cmp(b));
    // Interquartile spread: one stray full-width line must not defeat an
    // otherwise well-aligned page-number column.
    let low = edges[edges.len() / 4];
    let high = edges[edges.len() * 3 / 4];
    high - low
}

/// Detects the printed contents pages inside the front-matter window.
pub(crate) fn detect(
    index: &TextIndex,
    config: &AutoBookmarkConfig,
    cancelled: &dyn Fn() -> bool,
) -> Result<(u32, Vec<TocPageDetection>), crate::error::CoreError> {
    let page_count = index.pages.len() as u32;
    let limit = config.front_page_limit(page_count);
    let mut scored = Vec::new();
    for page in index.pages.iter().filter(|page| page.page_index < limit) {
        if cancelled() {
            return Err(crate::error::CoreError::Cancelled);
        }
        scored.push(score_page(page, config));
    }
    // Adjacent contents pages reinforce each other; an isolated page with a
    // single stray number column does not.
    let base: Vec<u32> = scored.iter().map(|page| page.score).collect();
    for (position, page) in scored.iter_mut().enumerate() {
        let neighbours = base
            .iter()
            .enumerate()
            .filter(|(other, value)| {
                *other != position
                    && other.abs_diff(position) <= 4
                    && **value >= config.toc_page_min_score / 2
            })
            .count() as u32;
        if neighbours > 0 {
            let bonus = (neighbours * config.toc_adjacent_page_score)
                .min(config.toc_adjacent_page_score * 4);
            page.score += bonus;
            page.signals.insert("adjacent_toc_layout".to_owned(), bonus);
        }
    }
    let detected: Vec<TocPageDetection> = scored
        .into_iter()
        .filter(|page| {
            page.score >= config.toc_page_min_score
                && (page.keyword_hit
                    || (page.entry_count >= config.toc_min_entries_without_keyword
                        && (page.signals.contains_key("dot_leaders")
                            || page.signals.contains_key("right_edge_cluster"))))
        })
        .collect();
    Ok((limit, detected))
}
