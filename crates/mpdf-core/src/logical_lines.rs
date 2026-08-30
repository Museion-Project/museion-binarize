//! Deterministic logical-line assembly over raw OCR evidence.
//!
//! # Why this exists
//!
//! Region-based recognizers (RapidOCR, PaddleOCR, and anything else built on
//! a detector + a line recognizer) return *rectangles*.  The previous adapter
//! published each rectangle as a block **and** a line **and** a word at once.
//! On a printed contents page that is fatal: "Einleitung", the leader dots,
//! and "17" are three separate detections, so the row never exists as a row,
//! the page number is never associated with its title, and every entry is
//! skipped for `printed_page_unmapped` — which is exactly what the 64-page
//! corpus showed (2 contents pages, 31 entries, 31 skipped).
//!
//! This module rebuilds rows from segments using only geometry that the
//! provider actually measured.  It never invents text, never consults a word
//! list, and never reorders words inside a segment.  Every merge records why
//! it happened so a reviewer can audit the decision.
//!
//! # What it deliberately does not do
//!
//! * It does not merge across columns.
//! * It does not merge a running header/footer into body text — furniture
//!   detection stays in the bookmark layer, but height and band rules here
//!   keep a footnote or a header from being glued onto a body line.
//!   Segments whose heights differ by more than
//!   [`LogicalLineConfig::max_height_ratio`] never join.
//! * It does not touch a page whose route is native text: those boxes are
//!   synthesized from extracted text runs and carry no measured layout.

use serde::{Deserialize, Serialize};

use crate::ocr::{OcrBlock, OcrBox, OcrLine, OcrPage, OcrRoute, OcrWord};

/// Tunables for [`assemble_page`].  All ratios are relative to the page's
/// median segment height, so they hold across DPI settings.
#[derive(Debug, Clone, PartialEq)]
pub struct LogicalLineConfig {
    /// Minimum vertical overlap (as a fraction of the shorter segment) for
    /// two segments to be considered the same printed row.
    pub row_overlap_ratio: f32,
    /// Maximum baseline difference, in median-heights, within one row.
    pub baseline_tolerance: f32,
    /// Ordinary word/segment gap tolerated inside one logical line.
    pub max_gap_ratio: f32,
    /// Wider gap tolerated when the right-hand segment looks like a printed
    /// page number sitting at the right edge of its column (a contents row).
    pub leader_gap_ratio: f32,
    /// Minimum gap, in median-heights, before a right-flush numeric token may
    /// be claimed as a *contents page number* rather than an ordinary word.
    /// Without this, a superscript footnote marker one space after body text
    /// would be absorbed as if it were a page number.
    pub min_page_number_gap_ratio: f32,
    /// Two segments whose heights differ by more than this factor are treated
    /// as different typographic roles (body vs. footnote) and never merged.
    pub max_height_ratio: f32,
    /// A horizontal gap wider than this fraction of the page width, present
    /// on enough rows, is treated as a column boundary.
    pub column_gap_ratio: f32,
    /// Minimum number of rows that must show the same gap before it is
    /// accepted as a column boundary.
    pub column_min_rows: usize,
    /// Hard ceiling on segments considered on one page.
    pub max_segments: usize,
}

impl Default for LogicalLineConfig {
    fn default() -> Self {
        Self {
            row_overlap_ratio: 0.5,
            baseline_tolerance: 0.6,
            max_gap_ratio: 2.5,
            leader_gap_ratio: 60.0,
            min_page_number_gap_ratio: 4.0,
            max_height_ratio: 1.8,
            column_gap_ratio: 0.06,
            column_min_rows: 3,
            max_segments: 8_192,
        }
    }
}

/// Why two source segments ended up on one logical line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeReason {
    /// The provider already published this line; nothing was merged.
    ProviderLine,
    /// Same row, ordinary inter-word gap.
    AdjacentInRow,
    /// Same row, wide gap bridged by a leader run ending in a page number.
    LeaderToPageNumber,
    /// A run of leader characters ("....", "· · ·") joining a contents title
    /// to whatever follows it. Leader glyphs are much shorter than the text
    /// they connect, so they are exempt from the height-compatibility rule
    /// that keeps footnotes out of body lines.
    LeaderRun,
}

impl MergeReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProviderLine => "provider_line",
            Self::AdjacentInRow => "adjacent_in_row",
            Self::LeaderToPageNumber => "leader_to_page_number",
            Self::LeaderRun => "leader_run",
        }
    }
}

/// Audit record kept on every assembled line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineAssembly {
    /// How many provider segments were combined (1 = untouched).
    pub source_segments: u32,
    /// Zero-based column this line belongs to, in left-to-right order.
    pub column: u32,
    pub reasons: Vec<MergeReason>,
    /// True when the assembled row ends in a page-number-shaped token that
    /// arrived from a *different* segment than the title.
    pub page_number_recovered: bool,
}

#[derive(Debug, Clone)]
struct Segment {
    words: Vec<OcrWord>,
    bbox: OcrBox,
    confidence: f32,
    block_order: u32,
    line_order: u32,
}

impl Segment {
    fn baseline(&self) -> f32 {
        self.bbox.y + self.bbox.height
    }
    fn right(&self) -> f32 {
        self.bbox.x + self.bbox.width
    }
}

/// Rebuilds one page's blocks into logical lines.
///
/// Native-text pages and pages with no measured geometry are returned
/// unchanged: there is nothing here that could be honestly assembled.
pub fn assemble_page(page: &OcrPage, config: &LogicalLineConfig) -> (OcrPage, Vec<LineAssembly>) {
    if matches!(page.route, OcrRoute::NativeText) {
        return (page.clone(), untouched_assemblies(page));
    }
    let mut segments = Vec::new();
    for block in &page.blocks {
        for line in &block.lines {
            if line.words.is_empty() {
                continue;
            }
            if segments.len() >= config.max_segments {
                // Refuse to assemble a page that exceeds the bound rather
                // than silently assembling only its beginning.
                return (page.clone(), untouched_assemblies(page));
            }
            segments.push(Segment {
                words: line.words.clone(),
                bbox: line.bbox.clone(),
                confidence: line.confidence,
                block_order: block.reading_order,
                line_order: line.reading_order,
            });
        }
    }
    if segments.is_empty() {
        return (page.clone(), Vec::new());
    }

    let median_height = median_height(&segments);
    let columns = detect_columns(&segments, page.width as f32, median_height, config);
    let rows = group_rows(&segments, &columns, median_height, config);
    let assembled = merge_rows(rows, median_height, page.width as f32, config);

    let mut blocks: Vec<OcrBlock> = Vec::with_capacity(assembled.len());
    let mut assemblies = Vec::with_capacity(assembled.len());
    for (index, line) in assembled.into_iter().enumerate() {
        let (segment, assembly) = line;
        let mut words = segment.words;
        for (order, word) in words.iter_mut().enumerate() {
            word.reading_order = order as u32;
        }
        let ocr_line = OcrLine {
            bbox: segment.bbox.clone(),
            confidence: segment.confidence,
            reading_order: 0,
            words,
        };
        blocks.push(OcrBlock {
            bbox: segment.bbox,
            confidence: segment.confidence,
            reading_order: index as u32,
            lines: vec![ocr_line],
        });
        assemblies.push(assembly);
    }

    let mut result = page.clone();
    result.blocks = blocks;
    (result, assemblies)
}

fn untouched_assemblies(page: &OcrPage) -> Vec<LineAssembly> {
    page.blocks
        .iter()
        .flat_map(|block| block.lines.iter())
        .map(|_| LineAssembly {
            source_segments: 1,
            column: 0,
            reasons: vec![MergeReason::ProviderLine],
            page_number_recovered: false,
        })
        .collect()
}

fn median_height(segments: &[Segment]) -> f32 {
    let mut heights: Vec<f32> = segments
        .iter()
        .map(|segment| segment.bbox.height)
        .filter(|height| *height > 0.0)
        .collect();
    if heights.is_empty() {
        return 1.0;
    }
    heights.sort_by(f32::total_cmp);
    heights[heights.len() / 2].max(1.0)
}

/// Column boundaries as x coordinates.  A boundary is only accepted when the
/// same vertical whitespace corridor is empty across at least
/// `column_min_rows` rows, so a single short line cannot invent a column.
fn detect_columns(
    segments: &[Segment],
    page_width: f32,
    median_height: f32,
    config: &LogicalLineConfig,
) -> Vec<f32> {
    if page_width <= 0.0 || segments.len() < config.column_min_rows {
        return Vec::new();
    }
    // Occupancy histogram at median-height resolution.
    let bucket = median_height.max(1.0);
    let buckets = ((page_width / bucket).ceil() as usize).clamp(1, 4_096);
    let mut occupied = vec![0usize; buckets];
    for segment in segments {
        let start = ((segment.bbox.x / bucket).floor().max(0.0) as usize).min(buckets - 1);
        let end = ((segment.right() / bucket).ceil().max(0.0) as usize).min(buckets);
        for slot in occupied.iter_mut().take(end).skip(start) {
            *slot += 1;
        }
    }
    let min_gap_buckets = ((page_width * config.column_gap_ratio) / bucket).ceil() as usize;
    let min_gap_buckets = min_gap_buckets.max(1);
    let mut boundaries = Vec::new();
    let mut run_start: Option<usize> = None;
    for (index, count) in occupied.iter().enumerate() {
        if *count == 0 {
            run_start.get_or_insert(index);
            continue;
        }
        if let Some(start) = run_start.take() {
            let length = index - start;
            // A gap at the very left or right of the page is a margin, not a
            // column boundary.
            let touches_margin = start == 0 || index == buckets;
            if length >= min_gap_buckets && !touches_margin {
                let left_rows = rows_touching(segments, 0.0, start as f32 * bucket);
                let right_rows = rows_touching(segments, index as f32 * bucket, page_width);
                if left_rows >= config.column_min_rows && right_rows >= config.column_min_rows {
                    boundaries.push((start + length / 2) as f32 * bucket);
                }
            }
        }
    }
    boundaries.truncate(8);
    boundaries
}

fn rows_touching(segments: &[Segment], from: f32, to: f32) -> usize {
    segments
        .iter()
        .filter(|segment| segment.bbox.x < to && segment.right() > from)
        .count()
}

fn column_of(segment: &Segment, boundaries: &[f32]) -> u32 {
    let center = segment.bbox.x + segment.bbox.width / 2.0;
    boundaries.iter().filter(|edge| center > **edge).count() as u32
}

/// Groups segments into printed rows within each column.
fn group_rows(
    segments: &[Segment],
    boundaries: &[f32],
    median_height: f32,
    config: &LogicalLineConfig,
) -> Vec<(u32, Vec<Segment>)> {
    let mut by_column: Vec<(u32, Vec<Segment>)> = Vec::new();
    for segment in segments {
        let column = column_of(segment, boundaries);
        match by_column.iter_mut().find(|(id, _)| *id == column) {
            Some((_, bucket)) => bucket.push(segment.clone()),
            None => by_column.push((column, vec![segment.clone()])),
        }
    }
    by_column.sort_by_key(|(column, _)| *column);

    let mut rows: Vec<(u32, Vec<Segment>)> = Vec::new();
    for (column, mut bucket) in by_column {
        bucket.sort_by(|a, b| {
            a.baseline()
                .total_cmp(&b.baseline())
                .then_with(|| a.bbox.x.total_cmp(&b.bbox.x))
                .then_with(|| a.block_order.cmp(&b.block_order))
                .then_with(|| a.line_order.cmp(&b.line_order))
        });
        let mut current: Vec<Segment> = Vec::new();
        for segment in bucket {
            let joins = current
                .last()
                .is_some_and(|previous| same_row(previous, &segment, median_height, config));
            if joins {
                current.push(segment);
            } else {
                if !current.is_empty() {
                    rows.push((column, std::mem::take(&mut current)));
                }
                current.push(segment);
            }
        }
        if !current.is_empty() {
            rows.push((column, current));
        }
    }
    rows
}

fn same_row(a: &Segment, b: &Segment, median_height: f32, config: &LogicalLineConfig) -> bool {
    if (a.baseline() - b.baseline()).abs() > config.baseline_tolerance * median_height {
        return false;
    }
    let top = a.bbox.y.max(b.bbox.y);
    let bottom = (a.bbox.y + a.bbox.height).min(b.bbox.y + b.bbox.height);
    let overlap = (bottom - top).max(0.0);
    let shorter = a.bbox.height.min(b.bbox.height).max(f32::EPSILON);
    overlap / shorter >= config.row_overlap_ratio
}

fn compatible_height(a: &Segment, b: &Segment, config: &LogicalLineConfig) -> bool {
    let (small, large) = if a.bbox.height <= b.bbox.height {
        (a.bbox.height, b.bbox.height)
    } else {
        (b.bbox.height, a.bbox.height)
    };
    if small <= 0.0 {
        return false;
    }
    large / small <= config.max_height_ratio
}

/// Merges the segments of each row into one or more logical lines.
fn merge_rows(
    rows: Vec<(u32, Vec<Segment>)>,
    median_height: f32,
    page_width: f32,
    config: &LogicalLineConfig,
) -> Vec<(Segment, LineAssembly)> {
    let mut output: Vec<(Segment, LineAssembly)> = Vec::new();
    for (column, mut row) in rows {
        row.sort_by(|a, b| {
            a.bbox
                .x
                .total_cmp(&b.bbox.x)
                .then_with(|| a.block_order.cmp(&b.block_order))
                .then_with(|| a.line_order.cmp(&b.line_order))
        });
        let column_right = row
            .iter()
            .map(Segment::right)
            .fold(f32::MIN, f32::max)
            .max(0.0);
        let mut current: Option<(Segment, LineAssembly)> = None;
        for segment in row {
            match current.take() {
                None => {
                    current = Some((
                        segment,
                        LineAssembly {
                            source_segments: 1,
                            column,
                            reasons: vec![MergeReason::ProviderLine],
                            page_number_recovered: false,
                        },
                    ));
                }
                Some((accumulated, mut assembly)) => {
                    let gap = segment.bbox.x - accumulated.right();
                    let bridging_leader = is_leader_run(&segment) || is_leader_run(&accumulated);
                    let tail =
                        is_page_number_tail(&segment, column_right, page_width, median_height);
                    let reason =
                        if gap <= config.leader_gap_ratio * median_height && bridging_leader {
                            // A leader run belongs to the row it decorates,
                            // whatever its glyph height.
                            Some(MergeReason::LeaderRun)
                        } else if gap <= config.max_gap_ratio * median_height
                            && compatible_height(&accumulated, &segment, config)
                        {
                            Some(MergeReason::AdjacentInRow)
                        } else if tail
                            && gap >= config.min_page_number_gap_ratio * median_height
                            && gap <= config.leader_gap_ratio * median_height
                        {
                            Some(MergeReason::LeaderToPageNumber)
                        } else {
                            None
                        };
                    match reason {
                        Some(reason) => {
                            assembly.source_segments += 1;
                            assembly.reasons.push(reason);
                            // "Recovered" means a page-number-shaped token
                            // arrived from a different segment across a
                            // contents-style bridge -- either a wide gap or a
                            // leader run this line already absorbed. A number
                            // that is simply the next word (a year, a figure
                            // reference) never sets this.
                            let bridged = reason == MergeReason::LeaderToPageNumber
                                || assembly.reasons.contains(&MergeReason::LeaderRun);
                            if tail && bridged {
                                assembly.page_number_recovered = true;
                            }
                            current = Some((join(accumulated, segment), assembly));
                        }
                        None => {
                            output.push((accumulated, assembly));
                            current = Some((
                                segment,
                                LineAssembly {
                                    source_segments: 1,
                                    column,
                                    reasons: vec![MergeReason::ProviderLine],
                                    page_number_recovered: false,
                                },
                            ));
                        }
                    }
                }
            }
        }
        if let Some(entry) = current {
            output.push(entry);
        }
    }
    // Reading order: column-major (all of column 0 top to bottom, then
    // column 1), which is how a printed two-column page is read.
    output.sort_by(|a, b| {
        a.1.column
            .cmp(&b.1.column)
            .then_with(|| a.0.bbox.y.total_cmp(&b.0.bbox.y))
            .then_with(|| a.0.bbox.x.total_cmp(&b.0.bbox.x))
    });
    output
}

fn join(mut left: Segment, right: Segment) -> Segment {
    let x = left.bbox.x.min(right.bbox.x);
    let y = left.bbox.y.min(right.bbox.y);
    let far_x = left.right().max(right.right());
    let far_y = (left.bbox.y + left.bbox.height).max(right.bbox.y + right.bbox.height);
    left.confidence = left.confidence.min(right.confidence);
    left.words.extend(right.words);
    left.bbox = OcrBox {
        x,
        y,
        width: far_x - x,
        height: far_y - y,
    };
    left
}

/// True when a segment is nothing but leader glyphs.
///
/// Printed contents pages connect a title to its page number with a run of
/// periods, middle dots, or dashes. Recognizers frequently return that run as
/// its own detection, and its glyphs are far shorter than the surrounding
/// text, so it has to be recognized as a connector rather than filtered out by
/// the footnote-height rule.
fn is_leader_run(segment: &Segment) -> bool {
    let mut saw_leader = false;
    for word in &segment.words {
        for character in word.text.chars() {
            if character.is_whitespace() {
                continue;
            }
            if matches!(
                character,
                '.' | '\u{00b7}' | '\u{2022}' | '\u{2026}' | '-' | '\u{2013}' | '\u{2014}' | '_'
            ) {
                saw_leader = true;
            } else {
                return false;
            }
        }
    }
    saw_leader
}

/// True when a segment looks like a printed contents page number: a short
/// Arabic or Roman token, flush against the right edge of its column.
fn is_page_number_tail(
    segment: &Segment,
    column_right: f32,
    page_width: f32,
    median_height: f32,
) -> bool {
    if segment.words.len() > 2 {
        return false;
    }
    let text: String = segment
        .words
        .iter()
        .map(|word| word.text.trim())
        .collect::<Vec<_>>()
        .join("");
    let trimmed = text.trim_matches(|c: char| c == '.' || c == ',' || c.is_whitespace());
    if trimmed.is_empty() || trimmed.chars().count() > 8 {
        return false;
    }
    let arabic = trimmed.chars().all(|c| c.is_ascii_digit());
    let roman = trimmed.chars().all(|c| {
        matches!(
            c,
            'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm' | 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M'
        )
    });
    if !arabic && !roman {
        return false;
    }
    // "Flush right" is judged against the column's own right edge, so the
    // rule works on a two-column contents page as well as a full-width one.
    let slack = (median_height * 4.0).min(page_width * 0.12);
    segment.right() >= column_right - slack
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, x: f32, y: f32, width: f32, height: f32) -> OcrWord {
        OcrWord {
            text: text.to_owned(),
            normalized_text: text.to_owned(),
            bbox: OcrBox {
                x,
                y,
                width,
                height,
            },
            confidence: 0.9,
            reading_order: 0,
        }
    }

    /// Builds the shape a detector-box provider produces: every rectangle is
    /// republished as its own block, line, and word.
    fn detector_page(entries: &[(&str, f32, f32, f32, f32)]) -> OcrPage {
        let blocks = entries
            .iter()
            .enumerate()
            .map(|(index, (text, x, y, width, height))| {
                let w = word(text, *x, *y, *width, *height);
                let line = OcrLine {
                    bbox: w.bbox.clone(),
                    confidence: 0.9,
                    reading_order: index as u32,
                    words: vec![w],
                };
                OcrBlock {
                    bbox: line.bbox.clone(),
                    confidence: 0.9,
                    reading_order: index as u32,
                    lines: vec![line],
                }
            })
            .collect();
        OcrPage {
            page_index: 0,
            route: OcrRoute::Ocr {
                reason: crate::ocr::OcrRouteReason::MissingText,
            },
            width: 1200,
            height: 1600,
            blocks,
            revisions: Vec::new(),
            provider_provenance: None,
            provider_raw_artifact: None,
        }
    }

    fn line_texts(page: &OcrPage) -> Vec<String> {
        page.blocks
            .iter()
            .flat_map(|block| block.lines.iter())
            .map(|line| {
                line.words
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn contents_row_regains_its_page_number() {
        // Title on the left, page number flush right: the exact shape that
        // used to strand every printed-contents entry.
        let page = detector_page(&[
            ("Einleitung", 100.0, 200.0, 260.0, 30.0),
            ("1", 1050.0, 202.0, 20.0, 28.0),
            ("Literaturverzeichnis", 100.0, 260.0, 420.0, 30.0),
            ("201", 1020.0, 262.0, 50.0, 28.0),
        ]);
        let (assembled, audit) = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(
            line_texts(&assembled),
            vec!["Einleitung 1", "Literaturverzeichnis 201"]
        );
        assert!(audit.iter().all(|entry| entry.page_number_recovered));
        assert!(audit
            .iter()
            .all(|entry| entry.reasons.contains(&MergeReason::LeaderToPageNumber)));
    }

    #[test]
    fn ordinary_words_in_a_row_join_without_a_page_number_claim() {
        let page = detector_page(&[
            ("Der", 100.0, 200.0, 70.0, 30.0),
            ("Begriff", 180.0, 200.0, 120.0, 30.0),
        ]);
        let (assembled, audit) = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(line_texts(&assembled), vec!["Der Begriff"]);
        assert_eq!(audit[0].source_segments, 2);
        assert!(!audit[0].page_number_recovered);
    }

    #[test]
    fn different_rows_never_merge() {
        let page = detector_page(&[
            ("Erste", 100.0, 200.0, 120.0, 30.0),
            ("Zweite", 100.0, 400.0, 140.0, 30.0),
        ]);
        let (assembled, _) = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(line_texts(&assembled), vec!["Erste", "Zweite"]);
    }

    #[test]
    fn a_footnote_sized_segment_never_joins_body_text() {
        // Same baseline band, but half the height: a footnote marker must not
        // be glued onto the body line.
        let page = detector_page(&[
            ("Haupttext", 100.0, 200.0, 240.0, 32.0),
            ("12", 360.0, 214.0, 20.0, 14.0),
        ]);
        let (assembled, _) = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(line_texts(&assembled), vec!["Haupttext", "12"]);
    }

    #[test]
    fn two_columns_are_read_column_by_column() {
        let page = detector_page(&[
            ("links-eins", 60.0, 200.0, 300.0, 30.0),
            ("rechts-eins", 700.0, 200.0, 300.0, 30.0),
            ("links-zwei", 60.0, 260.0, 300.0, 30.0),
            ("rechts-zwei", 700.0, 260.0, 300.0, 30.0),
            ("links-drei", 60.0, 320.0, 300.0, 30.0),
            ("rechts-drei", 700.0, 320.0, 300.0, 30.0),
        ]);
        let (assembled, audit) = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(
            line_texts(&assembled),
            vec![
                "links-eins",
                "links-zwei",
                "links-drei",
                "rechts-eins",
                "rechts-zwei",
                "rechts-drei",
            ]
        );
        assert_eq!(audit[0].column, 0);
        assert_eq!(audit[3].column, 1);
    }

    #[test]
    fn a_far_right_word_that_is_not_a_number_stays_separate() {
        let page = detector_page(&[
            ("Einleitung", 100.0, 200.0, 260.0, 30.0),
            ("Anhang", 1000.0, 202.0, 130.0, 28.0),
        ]);
        let (assembled, _) = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(line_texts(&assembled), vec!["Einleitung", "Anhang"]);
    }

    #[test]
    fn roman_front_matter_numbers_are_recovered() {
        let page = detector_page(&[
            ("Preface", 100.0, 200.0, 200.0, 30.0),
            ("vii", 1040.0, 202.0, 40.0, 28.0),
        ]);
        let (assembled, audit) = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(line_texts(&assembled), vec!["Preface vii"]);
        assert!(audit[0].page_number_recovered);
    }

    #[test]
    fn a_provider_that_already_emits_real_lines_is_left_alone() {
        // One line, three words: nothing to assemble, and the words keep
        // their order and their boxes.
        let words = vec![
            word("Über", 100.0, 200.0, 90.0, 30.0),
            word("die", 200.0, 200.0, 60.0, 30.0),
            word("Größe", 270.0, 200.0, 110.0, 30.0),
        ];
        let line = OcrLine {
            bbox: OcrBox {
                x: 100.0,
                y: 200.0,
                width: 280.0,
                height: 30.0,
            },
            confidence: 0.9,
            reading_order: 0,
            words: words.clone(),
        };
        let page = OcrPage {
            page_index: 0,
            route: OcrRoute::Ocr {
                reason: crate::ocr::OcrRouteReason::MissingText,
            },
            width: 1200,
            height: 1600,
            blocks: vec![OcrBlock {
                bbox: line.bbox.clone(),
                confidence: 0.9,
                reading_order: 0,
                lines: vec![line],
            }],
            revisions: Vec::new(),
            provider_provenance: None,
            provider_raw_artifact: None,
        };
        let (assembled, audit) = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(line_texts(&assembled), vec!["Über die Größe"]);
        assert_eq!(audit[0].source_segments, 1);
        assert_eq!(
            assembled.blocks[0].lines[0]
                .words
                .iter()
                .map(|word| word.bbox.x)
                .collect::<Vec<_>>(),
            words.iter().map(|word| word.bbox.x).collect::<Vec<_>>()
        );
    }

    #[test]
    fn native_text_pages_are_returned_untouched() {
        let mut page = detector_page(&[("a", 0.0, 0.0, 10.0, 10.0)]);
        page.route = OcrRoute::NativeText;
        let (assembled, audit) = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(assembled, page);
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].reasons, vec![MergeReason::ProviderLine]);
    }

    #[test]
    fn a_leader_run_bridges_a_title_to_its_page_number() {
        // The exact three-detection shape a recognizer returns for a printed
        // contents row: title, a short-glyph run of dots, then the number.
        // The dots are half the height of the text, which used to push the
        // number out of the row entirely.
        let page = detector_page(&[
            ("Einleitung", 100.0, 200.0, 260.0, 30.0),
            (".................", 380.0, 218.0, 620.0, 8.0),
            ("2", 1040.0, 202.0, 24.0, 28.0),
        ]);
        let (assembled, audit) = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(
            line_texts(&assembled),
            vec!["Einleitung ................. 2"]
        );
        assert!(audit[0].page_number_recovered);
        assert_eq!(audit[0].source_segments, 3);
    }

    #[test]
    fn a_leader_run_with_no_number_after_it_still_stays_on_its_row() {
        let page = detector_page(&[
            ("Anhang", 100.0, 200.0, 200.0, 30.0),
            ("..........", 320.0, 218.0, 400.0, 8.0),
        ]);
        let (assembled, audit) = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(line_texts(&assembled), vec!["Anhang .........."]);
        assert!(!audit[0].page_number_recovered);
    }

    #[test]
    fn assembly_is_deterministic() {
        let page = detector_page(&[
            ("Kapitel", 100.0, 200.0, 200.0, 30.0),
            ("17", 1050.0, 202.0, 40.0, 28.0),
            ("Anhang", 100.0, 300.0, 180.0, 30.0),
            ("88", 1050.0, 302.0, 40.0, 28.0),
        ]);
        let first = assemble_page(&page, &LogicalLineConfig::default());
        let second = assemble_page(&page, &LogicalLineConfig::default());
        assert_eq!(first.0, second.0);
        assert_eq!(first.1, second.1);
    }
}
