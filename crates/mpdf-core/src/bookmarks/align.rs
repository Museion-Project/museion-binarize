//! Body-heading shortlists, printed-page mapping, and the globally monotone
//! target assignment.
//!
//! The three stages are deliberately separate: a title can match without a
//! page mapping, a page mapping can hold without a unique title, and only the
//! combination — plus a monotone position in the document — is allowed to
//! pass the automatic gate in `scoring`.

use std::collections::BTreeMap;

use super::config::{AutoBookmarkConfig, SCORE_LAYOUT_MAX, SCORE_OCR_MAX, SCORE_TITLE_MAX};
use super::text_index::{EvidenceLine, GeometryQuality, LineRef, TextIndex};
use super::toc_parse::{NumberingFamily, TocEntryDraft};

/// Similarity in permille (0..=1000): bounded character edit similarity
/// combined with token overlap. No NLP model, no unbounded comparison.
pub(crate) fn similarity(left: &str, right: &str) -> u32 {
    if left.is_empty() || right.is_empty() {
        return 0;
    }
    if left == right {
        return 1_000;
    }
    let character = character_similarity(left, right);
    let token = token_similarity(left, right);
    (character * 6 + token * 4) / 10
}

const MAX_COMPARED_CHARS: usize = 256;

fn character_similarity(left: &str, right: &str) -> u32 {
    let a: Vec<char> = left.chars().take(MAX_COMPARED_CHARS).collect();
    let b: Vec<char> = right.chars().take(MAX_COMPARED_CHARS).collect();
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 0;
    }
    // A large length difference cannot recover to a useful similarity, and
    // skipping the matrix keeps the shortlist pass linear in practice.
    if a.len().abs_diff(b.len()) * 2 > longest {
        return 0;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, left_char) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, right_char) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(left_char != right_char);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[b.len()];
    if distance >= longest {
        0
    } else {
        ((longest - distance) * 1_000 / longest) as u32
    }
}

fn token_similarity(left: &str, right: &str) -> u32 {
    let a: std::collections::BTreeSet<&str> = left.split_whitespace().collect();
    let b: std::collections::BTreeSet<&str> = right.split_whitespace().collect();
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let intersection = a.intersection(&b).count();
    let union = a.union(&b).count();
    (intersection * 1_000 / union) as u32
}

#[derive(Debug, Clone)]
pub(crate) struct TargetCandidate {
    pub(crate) line_ref: LineRef,
    pub(crate) page_index: u32,
    pub(crate) title_score: u32,
    pub(crate) layout_score: u32,
    pub(crate) ocr_score: u32,
    pub(crate) secondary_only: bool,
    pub(crate) min_confidence: f32,
    pub(crate) geometry: GeometryQuality,
}

#[derive(Debug, Clone)]
pub(crate) struct EntryAlignment {
    pub(crate) draft: TocEntryDraft,
    pub(crate) targets: Vec<TargetCandidate>,
    /// The entry's recall was cut short, so its best target may be missing.
    pub(crate) truncated_recall: bool,
    /// The ranked tail was capped. Reported, but not a reason to distrust the
    /// entries that did rank.
    pub(crate) capped_shortlist: bool,
}

/// Layout plausibility of a body line as a heading, 0..=1000.
fn layout_score(index: &TextIndex, reference: LineRef, config: &AutoBookmarkConfig) -> u32 {
    let page = index.page(reference);
    let line = index.line(reference);
    let mut score = 0u32;
    if line.top_permille <= config.body_top_percent * 10 {
        score += 400;
    }
    if page.median_line_height > 0.0 && line.bbox.height >= page.median_line_height * 1.15 {
        score += 250;
    }
    if line.primary_key.chars().count() <= 80 {
        score += 150;
    }
    let previous = page
        .lines
        .iter()
        .take(reference.line)
        .rev()
        .find(|candidate| !candidate.is_blank());
    let whitespace_above = match previous {
        None => true,
        Some(previous) => {
            page.geometry == GeometryQuality::Measured
                && line.bbox.y - (previous.bbox.y + previous.bbox.height)
                    >= page.median_line_height.max(1.0) * 1.5
        }
    };
    if whitespace_above {
        score += 200;
    }
    score.min(SCORE_LAYOUT_MAX)
}

fn ocr_score(index: &TextIndex, reference: LineRef, entry: &TocEntryDraft) -> u32 {
    let line = index.line(reference);
    let page = index.page(reference);
    let confidence = line
        .min_confidence
        .min(entry.min_confidence)
        .clamp(0.0, 1.0);
    let mut score = (f64::from(confidence) * f64::from(SCORE_OCR_MAX)).round() as u32;
    if page.degraded_order {
        score = score.saturating_sub(200);
    }
    score.min(SCORE_OCR_MAX)
}

/// True for lines that can never be a heading target.
fn excluded(line: &EvidenceLine) -> bool {
    line.repeated_furniture
        || line.is_blank()
        || line
            .primary_key
            .chars()
            .all(|character| character.is_ascii_digit() || character == ' ')
}

/// Builds the bounded shortlist and per-target evidence scores.
pub(crate) fn shortlist(
    index: &mut TextIndex,
    drafts: Vec<TocEntryDraft>,
    config: &AutoBookmarkConfig,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<EntryAlignment>, crate::error::CoreError> {
    let mut aligned = Vec::with_capacity(drafts.len());
    for draft in drafts {
        if cancelled() {
            return Err(crate::error::CoreError::Cancelled);
        }
        let (raw, truncated_recall) =
            index.shortlist(&draft.tokens, config.max_shortlist as usize * 2);
        let capped_shortlist = raw.len() > config.max_shortlist as usize;
        let mut targets = Vec::new();
        for (reference, _) in raw {
            let line = index.line(reference);
            if excluded(line) {
                continue;
            }
            let primary = similarity(&draft.primary_key, &line.primary_key);
            let secondary = similarity(&draft.secondary_key, &line.secondary_key);
            let secondary_only = secondary > primary;
            // A secondary (accent-folded) match is deliberately discounted:
            // polytonic Greek differing only in breathing marks must not
            // pass on folded evidence alone.
            let permille = if secondary_only {
                secondary * 9 / 10
            } else {
                primary
            };
            if permille == 0 {
                continue;
            }
            let title_score = permille * SCORE_TITLE_MAX / 1_000;
            let candidate = TargetCandidate {
                line_ref: reference,
                page_index: line.page_index,
                title_score,
                layout_score: layout_score(index, reference, config),
                ocr_score: ocr_score(index, reference, &draft),
                secondary_only,
                min_confidence: line.min_confidence,
                geometry: line.geometry,
            };
            targets.push(candidate);
        }
        targets.sort_by(|a, b| {
            b.title_score
                .cmp(&a.title_score)
                .then_with(|| a.page_index.cmp(&b.page_index))
                .then_with(|| {
                    index
                        .line(a.line_ref)
                        .bbox
                        .y
                        .total_cmp(&index.line(b.line_ref).bbox.y)
                })
                .then_with(|| {
                    index
                        .line(a.line_ref)
                        .line_id
                        .cmp(&index.line(b.line_ref).line_id)
                })
        });
        targets.truncate(config.max_shortlist as usize);
        aligned.push(EntryAlignment {
            draft,
            targets,
            truncated_recall,
            capped_shortlist,
        });
    }
    Ok(aligned)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MappingSegment {
    pub(crate) family: NumberingFamily,
    pub(crate) index: u32,
    pub(crate) offset: i64,
    pub(crate) anchor_count: u32,
    pub(crate) first_printed: u32,
    pub(crate) last_printed: u32,
    pub(crate) residual_min: i64,
    pub(crate) residual_max: i64,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct PageMapping {
    pub(crate) segments: Vec<MappingSegment>,
    /// Entry ordinal -> index into `segments`.
    pub(crate) assignment: BTreeMap<usize, usize>,
}

impl PageMapping {
    pub(crate) fn expected_page(&self, entry: usize, printed: u32) -> Option<i64> {
        let segment = self.segments.get(*self.assignment.get(&entry)?)?;
        i64::from(printed).checked_add(segment.offset)
    }

    pub(crate) fn segment_of(&self, entry: usize) -> Option<&MappingSegment> {
        self.segments.get(*self.assignment.get(&entry)?)
    }
}

/// Solves printed-label to physical-page mapping as piecewise-constant
/// offsets per numbering family. Roman front matter, arabic body text, and
/// inserted plates therefore produce their own segments instead of one
/// document-wide guess — and a new segment costs a fixed penalty so a single
/// stray anchor cannot fragment the mapping.
pub(crate) fn solve_mapping(
    entries: &[EntryAlignment],
    config: &AutoBookmarkConfig,
) -> PageMapping {
    let mut mapping = PageMapping::default();
    for family in [NumberingFamily::Arabic, NumberingFamily::Roman] {
        let anchors: Vec<(usize, u32, i64)> = entries
            .iter()
            .enumerate()
            .filter_map(|(ordinal, entry)| {
                let printed = entry.draft.printed.as_ref()?;
                if printed.family != family {
                    return None;
                }
                let best = entry.targets.first()?;
                if best.title_score < config.anchor_min_title_score {
                    return None;
                }
                Some((
                    ordinal,
                    printed.value,
                    i64::from(best.page_index) - i64::from(printed.value),
                ))
            })
            .collect();
        if anchors.is_empty() {
            continue;
        }
        let mut offsets: Vec<i64> = anchors.iter().map(|anchor| anchor.2).collect();
        offsets.sort_unstable();
        offsets.dedup();
        // Deterministic dynamic program over the observed offset hypotheses:
        // no randomized consensus, no unseeded sampling.
        let states = offsets.len();
        let mut cost = vec![vec![u64::MAX; states]; anchors.len()];
        let mut back = vec![vec![0usize; states]; anchors.len()];
        for (state, offset) in offsets.iter().enumerate() {
            cost[0][state] =
                u64::from(anchors[0].2 != *offset) * u64::from(config.anchor_mismatch_penalty);
        }
        for step in 1..anchors.len() {
            for (state, offset) in offsets.iter().enumerate() {
                let local = u64::from(anchors[step].2 != *offset)
                    * u64::from(config.anchor_mismatch_penalty);
                let mut best = u64::MAX;
                let mut best_previous = state;
                for (previous, previous_cost) in cost[step - 1].iter().enumerate() {
                    if *previous_cost == u64::MAX {
                        continue;
                    }
                    let transition = if previous == state {
                        0
                    } else {
                        u64::from(config.segment_change_penalty)
                    };
                    let total = previous_cost.saturating_add(transition);
                    if total < best || (total == best && previous < best_previous) {
                        best = total;
                        best_previous = previous;
                    }
                }
                cost[step][state] = best.saturating_add(local);
                back[step][state] = best_previous;
            }
        }
        let mut state = (0..states)
            .min_by_key(|state| (cost[anchors.len() - 1][*state], *state))
            .unwrap_or(0);
        let mut path = vec![0usize; anchors.len()];
        for step in (0..anchors.len()).rev() {
            path[step] = state;
            if step > 0 {
                state = back[step][state];
            }
        }
        // Materialize maximal equal-offset runs as segments.
        let mut start = 0usize;
        while start < path.len() {
            let mut end = start;
            while end + 1 < path.len() && path[end + 1] == path[start] {
                end += 1;
            }
            let offset = offsets[path[start]];
            let members = &anchors[start..=end];
            let residuals: Vec<i64> = members.iter().map(|anchor| anchor.2 - offset).collect();
            let segment_index = mapping.segments.len();
            mapping.segments.push(MappingSegment {
                family,
                index: segment_index as u32,
                offset,
                anchor_count: members.len() as u32,
                first_printed: members.first().map(|anchor| anchor.1).unwrap_or(0),
                last_printed: members.last().map(|anchor| anchor.1).unwrap_or(0),
                residual_min: residuals.iter().copied().min().unwrap_or(0),
                residual_max: residuals.iter().copied().max().unwrap_or(0),
            });
            let first_entry = members.first().map(|anchor| anchor.0).unwrap_or(0);
            let last_entry = members.last().map(|anchor| anchor.0).unwrap_or(0);
            for (ordinal, entry) in entries.iter().enumerate() {
                let Some(printed) = entry.draft.printed.as_ref() else {
                    continue;
                };
                if printed.family != family {
                    continue;
                }
                let inside = ordinal >= first_entry && ordinal <= last_entry;
                let before_first = start == 0 && ordinal < first_entry;
                let after_last = end + 1 == path.len() && ordinal > last_entry;
                if inside || before_first || after_last {
                    mapping.assignment.insert(ordinal, segment_index);
                }
            }
            start = end + 1;
        }
    }
    mapping
}

/// Fenwick tree over prefix maxima, used by the monotone assignment DP.
struct PrefixMax {
    tree: Vec<(i64, usize)>,
}

impl PrefixMax {
    fn new(size: usize) -> Self {
        Self {
            tree: vec![(i64::MIN, usize::MAX); size + 1],
        }
    }
    fn update(&mut self, mut position: usize, value: (i64, usize)) {
        position += 1;
        while position < self.tree.len() {
            if value.0 > self.tree[position].0 {
                self.tree[position] = value;
            }
            position += position & position.wrapping_neg();
        }
    }
    fn query(&self, mut position: usize) -> (i64, usize) {
        let mut best = (0i64, usize::MAX);
        while position > 0 {
            if self.tree[position].0 > best.0 {
                best = self.tree[position];
            }
            position -= position & position.wrapping_neg();
        }
        best
    }
}

/// Chooses at most one target per entry so that targets never move backwards
/// through the document. Entries whose best target would break monotonicity
/// are left unassigned rather than silently reordered.
pub(crate) fn monotonic_selection(
    index: &TextIndex,
    entries: &[EntryAlignment],
    preliminary: &[Vec<u32>],
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Option<usize>>, crate::error::CoreError> {
    // One global, totally ordered position key per candidate target.
    let mut positions: Vec<(u32, i64, String)> = Vec::new();
    for entry in entries {
        for target in &entry.targets {
            let line = index.line(target.line_ref);
            positions.push((
                line.page_index,
                (line.bbox.y * 1_000.0).round() as i64,
                line.line_id.clone(),
            ));
        }
    }
    positions.sort();
    positions.dedup();
    let position_of = |line: &EvidenceLine| -> usize {
        positions
            .binary_search(&(
                line.page_index,
                (line.bbox.y * 1_000.0).round() as i64,
                line.line_id.clone(),
            ))
            .unwrap_or(0)
    };

    let mut tree = PrefixMax::new(positions.len() + 1);
    // (entry ordinal, target ordinal) -> predecessor encoded as the flat
    // index into `flat`, or usize::MAX for "first assigned entry".
    let mut flat: Vec<(usize, usize, usize, i64)> = Vec::new();
    for (ordinal, entry) in entries.iter().enumerate() {
        if cancelled() {
            return Err(crate::error::CoreError::Cancelled);
        }
        let mut updates = Vec::new();
        for (target_ordinal, target) in entry.targets.iter().enumerate() {
            let line = index.line(target.line_ref);
            let position = position_of(line);
            let (best_previous, previous_flat) = tree.query(position);
            let value = best_previous + i64::from(preliminary[ordinal][target_ordinal]);
            let flat_index = flat.len();
            flat.push((ordinal, target_ordinal, previous_flat, value));
            updates.push((position, (value, flat_index)));
        }
        for (position, value) in updates {
            tree.update(position, value);
        }
    }
    let mut selection = vec![None; entries.len()];
    let mut cursor = flat
        .iter()
        .enumerate()
        .max_by_key(|(index, item)| (item.3, std::cmp::Reverse(*index)))
        .map(|(index, _)| index);
    while let Some(current) = cursor {
        let (ordinal, target_ordinal, previous, _) = flat[current];
        selection[ordinal] = Some(target_ordinal);
        cursor = (previous != usize::MAX).then_some(previous);
    }
    Ok(selection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_is_bounded_and_symmetric() {
        assert_eq!(similarity("introduction", "introduction"), 1_000);
        assert_eq!(similarity("introduction", ""), 0);
        assert!(similarity("the wine dark sea", "the wine dark sea") == 1_000);
        assert!(similarity("introduction", "conclusion") < 700);
        assert_eq!(
            similarity("chapter one", "chapter two"),
            similarity("chapter two", "chapter one")
        );
    }

    #[test]
    fn a_large_length_difference_scores_zero_characters() {
        assert_eq!(character_similarity("a", "abcdefghij"), 0);
    }
}
