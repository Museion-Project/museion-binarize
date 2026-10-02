//! Printed contents-line parsing: columns, continuations, numbering, and
//! trailing printed page labels.
//!
//! Everything here is textual and layout-local. Deciding whether a parsed
//! entry may become a bookmark happens later, in `align` and `scoring`; this
//! module only turns lines into typed, fully referenced records and records
//! why a line was rejected.

use crate::derived::Bbox;

use super::config::AutoBookmarkConfig;
use super::text_index::{
    index_keys, primary_key, secondary_key, EvidenceLine, EvidencePage, GeometryQuality,
};
use super::toc_detect::CONTENTS_KEYWORDS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumberingFamily {
    Arabic,
    Roman,
}

impl NumberingFamily {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Arabic => "arabic",
            Self::Roman => "roman",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PrintedNumber {
    pub(crate) raw: String,
    pub(crate) value: u32,
    pub(crate) family: NumberingFamily,
}

#[derive(Debug, Clone)]
pub(crate) struct TocEntryDraft {
    pub(crate) raw_title: String,
    pub(crate) primary_key: String,
    pub(crate) secondary_key: String,
    pub(crate) tokens: Vec<String>,
    pub(crate) numbering_prefix: Option<String>,
    pub(crate) numbering_path: Vec<u32>,
    pub(crate) printed: Option<PrintedNumber>,
    pub(crate) page_id: String,
    pub(crate) page_index: u32,
    pub(crate) line_ids: Vec<String>,
    pub(crate) word_ids: Vec<String>,
    pub(crate) bbox: Bbox,
    /// Height of the entry's first printed line. A wrapped entry must not
    /// look like a larger typeface merely because it occupies two lines.
    pub(crate) title_line_height: f64,
    pub(crate) column_index: u32,
    pub(crate) indent_bucket: u32,
    pub(crate) min_confidence: f32,
    pub(crate) has_leader: bool,
    pub(crate) merged_lines: u32,
    pub(crate) reason_codes: Vec<String>,
}

/// Characters used as dot leaders in printed contents lists.
fn is_leader(character: char) -> bool {
    matches!(
        character,
        '.' | '\u{00b7}'
            | '\u{2022}'
            | '\u{2024}'
            | '\u{2026}'
            | '\u{2027}'
            | '\u{ff0e}'
            | '\u{3002}'
            | '_'
            | '-'
            | '\u{2500}'
    )
}

pub(crate) fn has_leader_run(text: &str) -> bool {
    let mut run = 0;
    for character in text.chars() {
        if is_leader(character) || character == ' ' {
            if is_leader(character) {
                run += 1;
                if run >= 3 {
                    return true;
                }
            }
        } else {
            run = 0;
        }
    }
    false
}

pub(crate) fn parse_roman(token: &str) -> Option<u32> {
    let lower = token.to_ascii_lowercase();
    if lower.is_empty() || lower.len() > 15 {
        return None;
    }
    let mut total: u32 = 0;
    let mut previous = 0;
    for character in lower.chars().rev() {
        let value = match character {
            'i' => 1,
            'v' => 5,
            'x' => 10,
            'l' => 50,
            'c' => 100,
            'd' => 500,
            'm' => 1000,
            _ => return None,
        };
        if value < previous {
            total = total.checked_sub(value)?;
        } else {
            total = total.checked_add(value)?;
            previous = value;
        }
    }
    (total > 0 && total <= 5_000).then_some(total)
}

/// The trailing printed page token of a contents line, if the last token
/// resolves to a page number. Ranges keep only the first page; the original
/// token is retained as evidence.
pub(crate) fn trailing_printed_number(text: &str) -> Option<PrintedNumber> {
    let last = text.split_whitespace().next_back()?;
    let trimmed = last.trim_matches(|character: char| is_leader(character) || character == ',');
    if trimmed.is_empty() {
        return None;
    }
    let head = trimmed
        .split(['-', '\u{2013}', '\u{2014}'])
        .next()
        .unwrap_or(trimmed);
    if !head.is_empty() && head.len() <= 5 && head.chars().all(|c| c.is_ascii_digit()) {
        return head.parse::<u32>().ok().map(|value| PrintedNumber {
            raw: last.to_owned(),
            value,
            family: NumberingFamily::Arabic,
        });
    }
    parse_roman(head).map(|value| PrintedNumber {
        raw: last.to_owned(),
        value,
        family: NumberingFamily::Roman,
    })
}

/// Explicit numbering path of a heading (`1`, `1.2`, `1.2.3`, `IV.`, `第三章`).
pub(crate) fn numbering_prefix(text: &str) -> (Option<String>, Vec<u32>) {
    let Some(token) = text.split_whitespace().next() else {
        return (None, Vec::new());
    };
    let core = token.trim_end_matches(['.', '\u{3001}', '\u{ff0e}', ')', '\u{ff09}']);
    if core.is_empty() {
        return (None, Vec::new());
    }
    let decimal: Vec<&str> = core.split('.').collect();
    if decimal.len() <= 6
        && decimal.iter().all(|part| {
            !part.is_empty() && part.len() <= 4 && part.chars().all(|c| c.is_ascii_digit())
        })
    {
        let path: Vec<u32> = decimal
            .iter()
            .filter_map(|part| part.parse::<u32>().ok())
            .collect();
        if path.len() == decimal.len() {
            return (Some(token.to_owned()), path);
        }
    }
    if core.len() <= 8 && core.chars().all(|c| c.is_ascii_alphabetic()) {
        if let Some(value) = parse_roman(core) {
            return (Some(token.to_owned()), vec![value]);
        }
    }
    // CJK chapter markers: 第N章 / 第N节 / 第N部分.
    let characters: Vec<char> = token.chars().collect();
    if characters.first() == Some(&'第') && characters.len() >= 3 {
        let level = match characters.last() {
            Some('章') | Some('部') | Some('編') | Some('编') => 1,
            Some('节') | Some('節') => 2,
            _ => return (None, Vec::new()),
        };
        return (Some(token.to_owned()), vec![level]);
    }
    (None, Vec::new())
}

/// Removes only the trailing printed page token and the leader run in front
/// of it. Numbering, case, punctuation, and Unicode are preserved exactly.
pub(crate) fn strip_printed_tail(text: &str, printed: Option<&PrintedNumber>) -> String {
    let mut remaining = text.trim_end();
    if let Some(printed) = printed {
        if let Some(stripped) = remaining.strip_suffix(&printed.raw) {
            remaining = stripped;
        }
    }
    remaining
        .trim_end_matches(|character: char| is_leader(character) || character.is_whitespace())
        .trim()
        .to_owned()
}

/// Column assignment for one contents page. Two columns are only declared
/// when the geometry is measured and both clusters are well populated;
/// approximate native-text boxes never produce a multi-column decision.
fn column_of(page: &EvidencePage, line: &EvidenceLine, columns: u32) -> u32 {
    if columns < 2 {
        return 0;
    }
    let center = line.bbox.x + line.bbox.width / 2.0;
    u32::from(center > page.width / 2.0)
}

pub(crate) fn column_count(page: &EvidencePage) -> u32 {
    if page.geometry != GeometryQuality::Measured || page.width <= 0.0 {
        return 1;
    }
    let middle = page.width / 2.0;
    let (mut left, mut right) = (0u32, 0u32);
    let mut crossing = 0u32;
    for line in page.lines.iter().filter(|line| !line.is_blank()) {
        let start = line.bbox.x;
        let end = line.bbox.x + line.bbox.width;
        if end <= middle {
            left += 1;
        } else if start >= middle {
            right += 1;
        } else {
            crossing += 1;
        }
    }
    if left >= 3 && right >= 3 && crossing * 4 <= left + right {
        2
    } else {
        1
    }
}

/// Parses one printed contents page into typed entries.
pub(crate) fn parse_toc_page(
    page: &EvidencePage,
    config: &AutoBookmarkConfig,
) -> Vec<TocEntryDraft> {
    let columns = column_count(page);
    let mut ordered: Vec<(u32, &EvidenceLine)> = page
        .lines
        .iter()
        .filter(|line| !line.is_blank())
        .map(|line| (column_of(page, line, columns), line))
        .collect();
    ordered.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.bbox.y.total_cmp(&b.1.bbox.y))
            .then_with(|| a.1.ordinal.cmp(&b.1.ordinal))
    });

    // A trailing token only becomes a printed page label when the line has a
    // dot leader or the page shows the same trailing-number column on other
    // lines. A year or volume number at the end of a title is not a page.
    let printed_column_consensus = ordered
        .iter()
        .filter(|(_, line)| trailing_printed_number(&line.raw_text).is_some())
        .count()
        >= 2;

    let mut entries = Vec::new();
    let mut index = 0usize;
    while index < ordered.len() {
        let (column, line) = ordered[index];
        let mut members: Vec<&EvidenceLine> = vec![line];
        let mut printed = trailing_printed_number(&line.raw_text);
        // A contents entry whose title wrapped: the first line has no
        // trailing page number and the next line in the same column, at a
        // compatible indent and vertical distance, carries it.
        while printed.is_none() && (members.len() as u32) < config.max_continuation_lines {
            let Some((next_column, next)) = ordered.get(index + members.len()).copied() else {
                break;
            };
            if next_column != column {
                break;
            }
            let previous = *members.last().expect("members is never empty");
            let line_height = previous.bbox.height.max(page.median_line_height).max(1.0);
            let gap = next.bbox.y - (previous.bbox.y + previous.bbox.height);
            let indent = (next.bbox.x - line.bbox.x).abs();
            let compatible = if page.geometry == GeometryQuality::Measured {
                indent <= page.width * 0.08 && gap <= line_height * 1.2 && gap >= -line_height
            } else {
                next.ordinal == previous.ordinal + 1
            };
            if !compatible {
                break;
            }
            members.push(next);
            printed = trailing_printed_number(&next.raw_text);
        }
        index += members.len();

        let joined = members
            .iter()
            .map(|line| line.raw_text.trim())
            .collect::<Vec<_>>()
            .join(" ");
        let has_leader = has_leader_run(&joined);
        let mut reason_codes = Vec::new();
        if printed.is_some() && !has_leader && !printed_column_consensus {
            printed = None;
            reason_codes.push("printed_page_without_consensus".to_owned());
        }
        let raw_title = strip_printed_tail(&joined, printed.as_ref());
        if raw_title.trim().is_empty() {
            continue;
        }
        if raw_title.len() > config.max_title_bytes as usize {
            continue;
        }
        if raw_title
            .chars()
            .any(|character| character.is_control() && character != '\t')
        {
            continue;
        }
        let key = primary_key(&raw_title);
        if key.is_empty() {
            continue;
        }
        if key
            .chars()
            .all(|character| character.is_ascii_digit() || character == ' ')
        {
            // A line consisting only of digits is a page number, not a title.
            continue;
        }
        if CONTENTS_KEYWORDS.contains(&key.as_str()) {
            // The heading of the contents page itself is not an entry.
            continue;
        }
        if members.len() > 1 {
            reason_codes.push("toc_continuation_merged".to_owned());
        }
        if printed.is_none() {
            reason_codes.push("toc_no_printed_page".to_owned());
        }
        let (numbering_prefix, numbering_path) = numbering_prefix(&raw_title);
        let bbox = union(members.iter().map(|line| line.bbox));
        let indent_bucket = if page.geometry == GeometryQuality::Measured && page.width > 0.0 {
            ((line.bbox.x.max(0.0) / page.width) * 100.0)
                .round()
                .clamp(0.0, 100.0) as u32
                / 2
        } else {
            0
        };
        let secondary = secondary_key(&key);
        entries.push(TocEntryDraft {
            tokens: index_keys(&key, &secondary),
            secondary_key: secondary,
            primary_key: key,
            numbering_prefix,
            numbering_path,
            has_leader,
            printed,
            page_id: page.page_id.clone(),
            page_index: page.page_index,
            line_ids: members.iter().map(|line| line.line_id.clone()).collect(),
            word_ids: members.iter().flat_map(|line| line.word_ids()).collect(),
            bbox,
            title_line_height: line.bbox.height,
            column_index: column,
            indent_bucket,
            min_confidence: members
                .iter()
                .map(|line| line.min_confidence)
                .fold(1.0_f32, f32::min),
            merged_lines: members.len() as u32,
            raw_title,
            reason_codes,
        });
    }
    entries
}

fn union(boxes: impl Iterator<Item = Bbox>) -> Bbox {
    let mut result: Option<Bbox> = None;
    for bbox in boxes {
        result = Some(match result {
            None => bbox,
            Some(current) => {
                let x = current.x.min(bbox.x);
                let y = current.y.min(bbox.y);
                let right = (current.x + current.width).max(bbox.x + bbox.width);
                let bottom = (current.y + current.height).max(bbox.y + bbox.height);
                Bbox {
                    x,
                    y,
                    width: right - x,
                    height: bottom - y,
                }
            }
        });
    }
    result.unwrap_or(Bbox {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_numbers_cover_arabic_roman_and_ranges() {
        assert_eq!(
            trailing_printed_number("Introduction .... 45")
                .unwrap()
                .value,
            45
        );
        assert_eq!(
            trailing_printed_number("Preface ... xii").unwrap().family,
            NumberingFamily::Roman
        );
        assert_eq!(
            trailing_printed_number("Sources 120-135").unwrap().value,
            120
        );
        assert!(trailing_printed_number("Chapter One").is_none());
    }

    #[test]
    fn strip_keeps_numbering_and_unicode_but_removes_leaders() {
        let printed = trailing_printed_number("1.2 Ἀρχή .... 45");
        assert_eq!(
            strip_printed_tail("1.2 Ἀρχή .... 45", printed.as_ref()),
            "1.2 Ἀρχή"
        );
    }

    #[test]
    fn numbering_paths_are_explicit_before_anything_else() {
        assert_eq!(numbering_prefix("1.2.3 Title").1, vec![1, 2, 3]);
        assert_eq!(numbering_prefix("IV. Title").1, vec![4]);
        assert_eq!(numbering_prefix("第三章 绪论").1, vec![1]);
        assert!(numbering_prefix("Introduction").0.is_none());
    }
}
