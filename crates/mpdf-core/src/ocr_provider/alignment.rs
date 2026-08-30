//! Local geometry + full-page cloud transcription.
//!
//! # Why this shape and not the obvious one
//!
//! The obvious design is to ask the model for boxes. The measured evidence
//! says two things that rule it out as a default. First, whole-page
//! transcription is markedly more accurate than region-cropped prompting on
//! this corpus (63 polytonic Greek pages: CER 0.0318 with no region hints),
//! so the prompt must show the model the *whole* page. Second, a model that
//! is transcribing a whole page is not measuring anything: its coordinates
//! are a plausible-looking secondary generation, and writing them into a PDF
//! text layer would place invisible text where no glyph is.
//!
//! So the two claims are separated by who is competent to make them. The
//! local detector measured the page and owns every rectangle. The cloud model
//! read the page and owns the characters. This module is the join: it decides
//! which locally measured line each transcribed line belongs to, and refuses
//! to decide when it cannot.
//!
//! # The properties the join must have
//!
//! * **Monotone.** Printed text has a reading order; an alignment that lets
//!   line 9 of the transcription land above line 4 of the page has silently
//!   reordered a column. The dynamic program only ever moves forward in both
//!   sequences, so order violations are impossible by construction rather
//!   than checked afterwards.
//! * **Injective.** One transcribed line is consumed at most once. A model
//!   that repeats a header cannot stamp it over three body lines.
//! * **Lossless on the local side.** A locally detected line that nothing
//!   aligned to keeps its local text. Cloud output is never allowed to delete
//!   a body line, a folio, or a contents page number by omitting it.
//! * **Refusing.** Below a coverage gate the whole page keeps local text and
//!   says so. A half-aligned page is worse than an unaligned one, because the
//!   half that aligned looks authoritative.
//!
//! # The limitation this cannot fix
//!
//! Monotonicity detects a reordering only when the reordered lines are
//! distinguishable. A page of near-identical lines — a table of repeated
//! stems, "Kapitel I / Kapitel II / Kapitel III" — that a model returns in
//! the wrong order can still align, because every pairing scores highly and
//! the matcher has no evidence to prefer one over another. Nothing in the
//! text can settle it; only a rectangle could, and the model did not measure
//! one. This is recorded in docs/limitations.md rather than papered over
//! with a threshold that would reject good pages instead.

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::ocr::{OcrBox, OcrLine, OcrPage, OcrWord};

/// Identity of this algorithm, bound into the checkpoint fingerprint. Any
/// change to the matching rules must bump it, because the same inputs would
/// otherwise resume against evidence built under different rules.
pub const ALIGNMENT_VERSION: &str = "local-layout-plus-full-page/1";

/// Above this many DP cells the alignment refuses rather than allocating.
/// A real book page is tens of lines; hitting this means the inputs are not
/// what this function is for.
const MAX_DP_CELLS: usize = 4_000_000;

#[derive(Debug, Clone, PartialEq)]
pub struct AlignmentConfig {
    /// Minimum similarity before a transcribed line may replace a local
    /// line's text.
    pub min_line_similarity: f32,
    /// Minimum fraction of locally detected lines that must be replaced
    /// before the page's cloud text is used at all.
    pub min_coverage: f32,
    /// Beyond this length ratio two lines are never the same line, whatever
    /// their characters look like. Stops a one-word folio from absorbing a
    /// full body line.
    pub max_length_ratio: f32,
    /// Hard ceiling on transcribed lines considered for one page.
    pub max_provider_lines: usize,
    /// Hard ceiling on locally detected lines considered for one page.
    pub max_local_lines: usize,
}

impl Default for AlignmentConfig {
    fn default() -> Self {
        Self {
            // Chosen so that a line with one or two OCR character errors on
            // either side still matches, while two different lines of the
            // same page do not. Bigram Dice on the mixed-script holdout puts
            // true pairs above 0.8 and unrelated same-page pairs below 0.3.
            min_line_similarity: 0.55,
            // A page where fewer than three in five detected lines aligned is
            // a page where something structural went wrong — a rotated scan,
            // a plate, a model that summarized instead of transcribing.
            min_coverage: 0.6,
            max_length_ratio: 4.0,
            max_provider_lines: 4_096,
            max_local_lines: 4_096,
        }
    }
}

/// What happened to one locally detected line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineDecision {
    /// The transcribed text replaced the local text; geometry unchanged.
    ReplacedFromTranscription,
    /// Nothing aligned well enough; the local text was kept.
    KeptLocalUnaligned,
    /// Something aligned but scored below the gate; the local text was kept.
    KeptLocalLowSimilarity,
}

impl LineDecision {
    pub fn code(self) -> &'static str {
        match self {
            Self::ReplacedFromTranscription => "replaced_from_transcription",
            Self::KeptLocalUnaligned => "kept_local_unaligned",
            Self::KeptLocalLowSimilarity => "kept_local_low_similarity",
        }
    }
}

/// Per-page counters, carried into the evidence parameters and the report.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct DecisionSummary {
    pub local_lines: usize,
    pub provider_lines: usize,
    pub replaced: usize,
    pub kept: usize,
    /// Transcribed lines that matched nothing and were discarded rather than
    /// given invented coordinates.
    pub dropped: usize,
    pub coverage: f32,
    /// Always zero: the dynamic program cannot emit a non-monotone pairing.
    /// Recorded anyway so a regression in the matcher is visible in evidence
    /// rather than only in a test.
    pub order_violations: usize,
}

#[derive(Debug, Clone)]
pub struct AlignedPage {
    pub page: OcrPage,
    pub summary: DecisionSummary,
    pub decisions: Vec<LineDecision>,
    /// `true` when the coverage gate passed and the page's text is the
    /// transcription's. `false` means the returned page is the local one,
    /// untouched.
    pub used_transcription: bool,
}

/// Splits a full-page transcription into lines, deterministically.
///
/// No language model output is trusted to be well-formed: this normalizes to
/// NFC, drops empty lines, collapses internal whitespace, and caps the count.
/// The same input always produces the same split, which is what lets the
/// digest of the transcription be a meaningful part of the fingerprint.
pub fn split_transcription(text: &str, limit: usize) -> Vec<String> {
    text.nfc()
        .collect::<String>()
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .take(limit)
        .collect()
}

/// Flattened view of one locally detected line, in reading order.
#[derive(Debug, Clone)]
struct LocalLine {
    block: usize,
    line: usize,
    text: String,
}

fn flatten_local(page: &OcrPage) -> Vec<LocalLine> {
    let mut blocks: Vec<(u32, usize)> = page
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.reading_order, index))
        .collect();
    blocks.sort_by_key(|(order, index)| (*order, *index));
    let mut lines = Vec::new();
    for (_, block_index) in blocks {
        let block = &page.blocks[block_index];
        let mut ordered: Vec<(u32, usize)> = block
            .lines
            .iter()
            .enumerate()
            .map(|(index, line)| (line.reading_order, index))
            .collect();
        ordered.sort_by_key(|(order, index)| (*order, *index));
        for (_, line_index) in ordered {
            let line = &block.lines[line_index];
            lines.push(LocalLine {
                block: block_index,
                line: line_index,
                text: line
                    .words
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            });
        }
    }
    lines
}

/// Aligns a full-page transcription onto a locally measured page.
///
/// Returns the local page unchanged, with `used_transcription = false`, when
/// the coverage gate fails. It never returns a page with fewer lines, fewer
/// blocks, or different rectangles than the local page it was given.
pub fn align_transcription(
    local: &OcrPage,
    transcription: &str,
    config: &AlignmentConfig,
) -> AlignedPage {
    let local_lines = flatten_local(local);
    let provider_lines = split_transcription(transcription, config.max_provider_lines);
    let mut summary = DecisionSummary {
        local_lines: local_lines.len(),
        provider_lines: provider_lines.len(),
        ..Default::default()
    };

    if local_lines.is_empty()
        || provider_lines.is_empty()
        || local_lines.len() > config.max_local_lines
        || local_lines.len().saturating_mul(provider_lines.len()) > MAX_DP_CELLS
    {
        summary.kept = local_lines.len();
        summary.dropped = provider_lines.len();
        return AlignedPage {
            page: local.clone(),
            summary,
            decisions: vec![LineDecision::KeptLocalUnaligned; local_lines.len()],
            used_transcription: false,
        };
    }

    let local_keys: Vec<MatchKey> = local_lines
        .iter()
        .map(|line| MatchKey::new(&line.text))
        .collect();
    let provider_keys: Vec<MatchKey> = provider_lines.iter().map(|t| MatchKey::new(t)).collect();
    let pairs = monotone_match(&local_keys, &provider_keys, config);

    // `monotone_match` reports one entry per *paired* local line, so the
    // decision vector is built per local line here: everything the matcher
    // never paired keeps its local text under `KeptLocalUnaligned`.
    let mut per_line = vec![LineDecision::KeptLocalUnaligned; local_lines.len()];
    let mut used_provider = vec![false; provider_lines.len()];
    let mut replacements: Vec<Option<(usize, f32)>> = vec![None; local_lines.len()];
    for (local_index, provider_index, score) in pairs {
        if score >= config.min_line_similarity {
            replacements[local_index] = Some((provider_index, score));
            used_provider[provider_index] = true;
            per_line[local_index] = LineDecision::ReplacedFromTranscription;
        } else {
            per_line[local_index] = LineDecision::KeptLocalLowSimilarity;
        }
    }

    summary.replaced = replacements.iter().filter(|item| item.is_some()).count();
    summary.kept = local_lines.len() - summary.replaced;
    summary.dropped = used_provider.iter().filter(|used| !**used).count();
    summary.coverage = if local_lines.is_empty() {
        0.0
    } else {
        summary.replaced as f32 / local_lines.len() as f32
    };
    summary.order_violations = 0;

    if summary.coverage < config.min_coverage {
        return AlignedPage {
            page: local.clone(),
            summary,
            decisions: vec![LineDecision::KeptLocalUnaligned; local_lines.len()],
            used_transcription: false,
        };
    }

    let mut page = local.clone();
    for (index, replacement) in replacements.iter().enumerate() {
        let Some((provider_index, score)) = replacement else {
            continue;
        };
        let position = &local_lines[index];
        let line = &mut page.blocks[position.block].lines[position.line];
        rewrite_line(line, &provider_lines[*provider_index], *score);
    }
    AlignedPage {
        page,
        summary,
        decisions: per_line,
        used_transcription: true,
    }
}

/// Replaces a line's words with the transcribed text, keeping the line's
/// measured rectangle.
///
/// Word rectangles are inherited one-to-one when the word counts agree —
/// which is the common case, because the local detector and the model are
/// reading the same printed words. When they disagree (a ligature the model
/// split, a hyphen it joined) the line's own measured box is divided in
/// proportion to word length. That is an honest approximation *inside a
/// measured line*, and it is recorded as such; it never invents a rectangle
/// outside geometry the detector actually produced.
fn rewrite_line(line: &mut OcrLine, text: &str, score: f32) {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return;
    }
    let confidence = corroborated_confidence(line.confidence, score);
    let inherit = tokens.len() == line.words.len();
    let total: usize = tokens
        .iter()
        .map(|token| token.chars().count().max(1))
        .sum();
    let mut cursor = line.bbox.x;
    let mut words = Vec::with_capacity(tokens.len());
    for (index, token) in tokens.iter().enumerate() {
        let bbox = if inherit {
            line.words[index].bbox.clone()
        } else {
            let share = token.chars().count().max(1) as f32 / total as f32;
            let width = line.bbox.width * share;
            let bbox = OcrBox {
                x: cursor,
                y: line.bbox.y,
                width,
                height: line.bbox.height,
            };
            cursor += width;
            bbox
        };
        words.push(OcrWord {
            normalized_text: normalize(token),
            text: (*token).to_owned(),
            bbox,
            confidence,
            reading_order: index as u32,
        });
    }
    line.words = words;
    line.confidence = confidence;
}

/// Confidence for a line whose text came from the transcription.
///
/// Three things are true at once and have to be reconciled:
///
/// * The model's own confidence is not evidence. A language model's certainty
///   about its own output is not a measurement of accuracy, and this project
///   never records it as one.
/// * The local engine's confidence describes the reading that was just
///   *replaced*, so it cannot simply be carried over.
/// * The only thing actually measured here is the agreement between two
///   independent recognitions of the same rectangle.
///
/// So two sources that agree are treated as corroborating — the noisy-OR of
/// their independent chances of being right — and the result is then capped
/// by the agreement itself, so a confident local engine cannot lend
/// near-certainty to a line the two sources barely agree on.
///
/// The cap is released at [`SAME_LINE_AGREEMENT`]. Above that, the two
/// readings are treated as the same printed line differing only by ordinary
/// OCR noise, which is what they almost always are: 0.8 agreement is a 20%
/// character disagreement, well above the 14% CER the local combined pass
/// shows on the mixed Greek/German/Latin holdout and several times the 3–6%
/// the cloud model shows on the same material. Below it the ceiling falls
/// away linearly, reaching 0.69 at the replacement threshold.
fn corroborated_confidence(local: f32, agreement: f32) -> f32 {
    let local = local.clamp(0.0, 1.0);
    let agreement = agreement.clamp(0.0, 1.0);
    let corroborated = 1.0 - (1.0 - local) * (1.0 - agreement);
    let ceiling = (agreement / SAME_LINE_AGREEMENT).min(1.0);
    corroborated.min(ceiling).clamp(0.0, 1.0)
}

/// Agreement at or above which two readings are treated as one printed line
/// read twice rather than two different claims.
const SAME_LINE_AGREEMENT: f32 = 0.8;

fn normalize(text: &str) -> String {
    text.nfc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Comparable form of one line: folded characters plus a script profile.
#[derive(Debug, Clone)]
struct MatchKey {
    folded: Vec<char>,
    bigrams: Vec<[char; 2]>,
    /// Fractions of Greek, Latin, digit and other characters. Two lines whose
    /// scripts disagree are penalized even when their shapes are similar,
    /// which is what stops a Greek body line from matching the German running
    /// head above it.
    profile: [f32; 4],
    length: usize,
}

impl MatchKey {
    fn new(text: &str) -> Self {
        let folded: Vec<char> = text
            .nfc()
            .flat_map(|character| character.to_lowercase())
            .filter(|character| character.is_alphanumeric())
            .collect();
        let mut bigrams: Vec<[char; 2]> = folded
            .windows(2)
            .map(|window| [window[0], window[1]])
            .collect();
        bigrams.sort_unstable();
        let mut counts = [0.0_f32; 4];
        for character in &folded {
            let slot = if is_greek(*character) {
                0
            } else if character.is_ascii_alphabetic() || is_latin_extended(*character) {
                1
            } else if character.is_numeric() {
                2
            } else {
                3
            };
            counts[slot] += 1.0;
        }
        let total = folded.len().max(1) as f32;
        for value in &mut counts {
            *value /= total;
        }
        Self {
            length: folded.len(),
            folded,
            bigrams,
            profile: counts,
        }
    }
}

fn is_greek(character: char) -> bool {
    matches!(character as u32, 0x0370..=0x03FF | 0x1F00..=0x1FFF)
}

fn is_latin_extended(character: char) -> bool {
    matches!(character as u32, 0x00C0..=0x024F)
}

/// Similarity in `[0, 1]`: character-bigram Dice, damped by script
/// disagreement, with a hard length-ratio veto.
fn similarity(left: &MatchKey, right: &MatchKey, config: &AlignmentConfig) -> f32 {
    if left.length == 0 || right.length == 0 {
        return 0.0;
    }
    let (short, long) = if left.length <= right.length {
        (left.length, right.length)
    } else {
        (right.length, left.length)
    };
    if long as f32 > short as f32 * config.max_length_ratio {
        return 0.0;
    }
    // Very short lines (a folio, a single numeral) have no usable bigrams, so
    // fall back to exact folded equality rather than scoring them 0 and
    // dropping every page number on the contents page.
    if left.bigrams.is_empty() || right.bigrams.is_empty() {
        return if left.folded == right.folded {
            1.0
        } else {
            0.0
        };
    }
    let shared = shared_multiset(&left.bigrams, &right.bigrams) as f32;
    let dice = 2.0 * shared / (left.bigrams.len() + right.bigrams.len()) as f32;
    // Bigram overlap alone is unfairly harsh on a short line with scattered
    // substitutions: "Thc Winc Dark Sca" against "The Wine Dark Sea" is three
    // character errors, which a reader would call the same line, but it
    // destroys six of thirteen bigrams and scores 0.54. Edit distance sees it
    // correctly at 0.79. Dice is kept as the cheap prefilter — it is the only
    // thing computed for the overwhelming majority of pairs, which are not
    // remotely similar — and the more expensive measure runs only for
    // plausible, bounded-length candidates.
    let similarity = if dice >= EDIT_DISTANCE_PREFILTER
        && left.length.saturating_mul(right.length) <= MAX_EDIT_DISTANCE_CELLS
    {
        dice.max(edit_similarity(&left.folded, &right.folded))
    } else {
        dice
    };
    let script_distance: f32 = left
        .profile
        .iter()
        .zip(right.profile.iter())
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / 2.0;
    (similarity * (1.0 - 0.5 * script_distance)).clamp(0.0, 1.0)
}

/// Bigram score below which two lines are not worth an edit distance.
const EDIT_DISTANCE_PREFILTER: f32 = 0.25;
/// Ceiling on one edit-distance matrix, so a pathological page cannot turn a
/// page-level alignment into a quadratic blowup.
const MAX_EDIT_DISTANCE_CELLS: usize = 64 * 1024;

/// Normalized Levenshtein similarity in `[0, 1]`, two rows at a time.
fn edit_similarity(left: &[char], right: &[char]) -> f32 {
    let columns = right.len();
    let mut previous: Vec<u32> = (0..=columns as u32).collect();
    let mut current = vec![0_u32; columns + 1];
    for (row, left_character) in left.iter().enumerate() {
        current[0] = row as u32 + 1;
        for column in 1..=columns {
            let substitution =
                previous[column - 1] + u32::from(*left_character != right[column - 1]);
            current[column] = substitution
                .min(previous[column] + 1)
                .min(current[column - 1] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[columns] as f32;
    let longest = left.len().max(right.len()).max(1) as f32;
    (1.0 - distance / longest).clamp(0.0, 1.0)
}

fn shared_multiset(left: &[[char; 2]], right: &[[char; 2]]) -> usize {
    let (mut i, mut j, mut shared) = (0, 0, 0);
    while i < left.len() && j < right.len() {
        match left[i].cmp(&right[j]) {
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    shared
}

/// Monotone, injective matching by dynamic programming.
///
/// Gaps on either side are free-ish (a small constant), so an inserted or
/// deleted line costs one skip rather than dragging the whole rest of the
/// page out of alignment. Returns `(local_index, provider_index, score)` in
/// increasing order of both indices — that ordering is the guarantee the
/// caller relies on for reading order and for injectivity.
fn monotone_match(
    local: &[MatchKey],
    provider: &[MatchKey],
    config: &AlignmentConfig,
) -> Vec<(usize, usize, f32)> {
    let rows = local.len();
    let columns = provider.len();
    let mut score = vec![0.0_f32; (rows + 1) * (columns + 1)];
    let at = |i: usize, j: usize| i * (columns + 1) + j;
    for i in 1..=rows {
        for j in 1..=columns {
            let diagonal =
                score[at(i - 1, j - 1)] + similarity(&local[i - 1], &provider[j - 1], config);
            let up = score[at(i - 1, j)];
            let left = score[at(i, j - 1)];
            score[at(i, j)] = diagonal.max(up).max(left);
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (rows, columns);
    while i > 0 && j > 0 {
        let value = similarity(&local[i - 1], &provider[j - 1], config);
        if (score[at(i, j)] - (score[at(i - 1, j - 1)] + value)).abs() < f32::EPSILON * 8.0 {
            pairs.push((i - 1, j - 1, value));
            i -= 1;
            j -= 1;
        } else if (score[at(i, j)] - score[at(i - 1, j)]).abs() < f32::EPSILON * 8.0 {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    pairs.reverse();
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::OcrBlock;

    fn page(lines: &[&str]) -> OcrPage {
        let blocks = lines
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let words: Vec<OcrWord> = text
                    .split_whitespace()
                    .enumerate()
                    .map(|(word_index, word)| OcrWord {
                        text: word.to_owned(),
                        normalized_text: word.to_owned(),
                        bbox: OcrBox {
                            x: 10.0 + word_index as f32 * 40.0,
                            y: 20.0 + index as f32 * 30.0,
                            width: 38.0,
                            height: 20.0,
                        },
                        confidence: 0.8,
                        reading_order: word_index as u32,
                    })
                    .collect();
                let bbox = OcrBox {
                    x: 10.0,
                    y: 20.0 + index as f32 * 30.0,
                    width: 500.0,
                    height: 20.0,
                };
                OcrBlock {
                    bbox: bbox.clone(),
                    confidence: 0.8,
                    reading_order: index as u32,
                    lines: vec![OcrLine {
                        bbox,
                        confidence: 0.8,
                        reading_order: 0,
                        words,
                    }],
                }
            })
            .collect();
        OcrPage {
            page_index: 0,
            route: crate::ocr::OcrRoute::Ocr {
                reason: crate::ocr::OcrRouteReason::MissingText,
            },
            width: 600,
            height: 800,
            blocks,
            revisions: Vec::new(),
            provider_provenance: None,
            provider_raw_artifact: None,
        }
    }

    fn text_of(page: &OcrPage) -> Vec<String> {
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
    fn a_clean_transcription_replaces_text_and_keeps_every_measured_box() {
        let local = page(&["Einleitng", "Die Ubersetzung", "Kapitel I"]);
        let aligned = align_transcription(
            &local,
            "Einleitung\nDie Übersetzung\nKapitel I\n",
            &AlignmentConfig::default(),
        );
        assert!(aligned.used_transcription);
        assert_eq!(
            text_of(&aligned.page),
            vec!["Einleitung", "Die Übersetzung", "Kapitel I"]
        );
        // Geometry is the local detector's, unchanged.
        assert_eq!(
            aligned.page.blocks[1].lines[0].bbox,
            local.blocks[1].lines[0].bbox
        );
        assert_eq!(aligned.summary.replaced, 3);
        assert_eq!(aligned.summary.order_violations, 0);
    }

    #[test]
    fn a_line_the_model_omitted_keeps_its_local_text_rather_than_disappearing() {
        // The failure this prevents: a contents page whose printed page
        // numbers the model skipped, leaving the row with a title and no
        // target. Local text must survive.
        let local = page(&["Einleitung 17", "Kapitel I 23", "Kapitel II 41"]);
        let aligned = align_transcription(
            &local,
            "Einleitung 17\nKapitel II 41\n",
            &AlignmentConfig {
                min_coverage: 0.5,
                ..Default::default()
            },
        );
        assert!(aligned.used_transcription);
        assert_eq!(
            text_of(&aligned.page),
            vec!["Einleitung 17", "Kapitel I 23", "Kapitel II 41"]
        );
        assert_eq!(aligned.summary.replaced, 2);
        assert_eq!(aligned.summary.kept, 1);
        assert_eq!(aligned.decisions[1], LineDecision::KeptLocalUnaligned);
    }

    #[test]
    fn an_extra_transcribed_line_is_dropped_and_never_given_invented_geometry() {
        let local = page(&["alpha beta gamma", "delta epsilon zeta"]);
        let aligned = align_transcription(
            &local,
            "alpha beta gamma\nthis line is not on the page at all\ndelta epsilon zeta\n",
            &AlignmentConfig::default(),
        );
        assert!(aligned.used_transcription);
        assert_eq!(aligned.page.blocks.len(), 2);
        assert_eq!(aligned.summary.dropped, 1);
        assert_eq!(text_of(&aligned.page).len(), 2);
    }

    #[test]
    fn a_repeated_header_cannot_be_stamped_onto_several_body_lines() {
        let local = page(&["RUNNING HEAD", "body one here", "body two here"]);
        let aligned = align_transcription(
            &local,
            "RUNNING HEAD\nRUNNING HEAD\nRUNNING HEAD\n",
            &AlignmentConfig::default(),
        );
        // One local line can align; the rest keep their own text, so the
        // coverage gate refuses the page instead of duplicating the header.
        assert!(!aligned.used_transcription);
        assert_eq!(
            text_of(&aligned.page),
            vec!["RUNNING HEAD", "body one here", "body two here"]
        );
    }

    #[test]
    fn a_transcription_that_reads_the_columns_backwards_is_refused() {
        // A model that read the right column first returns the page's lines
        // in the wrong order. A monotone matcher can pair at most one of
        // them, so the coverage gate refuses the page rather than writing
        // each line's text under a different line's rectangle.
        let local = page(&["alpha beta gamma", "delta epsilon zeta", "eta theta iota"]);
        let aligned = align_transcription(
            &local,
            "eta theta iota\ndelta epsilon zeta\nalpha beta gamma\n",
            &AlignmentConfig::default(),
        );
        assert!(!aligned.used_transcription);
        assert_eq!(
            text_of(&aligned.page),
            vec!["alpha beta gamma", "delta epsilon zeta", "eta theta iota"]
        );
    }

    #[test]
    fn a_greek_line_does_not_match_the_german_running_head_above_it() {
        let greek = MatchKey::new("λόγος ἐστὶν ἀρχή");
        let german = MatchKey::new("logos estin arche");
        let config = AlignmentConfig::default();
        assert!(
            similarity(&greek, &german, &config) < config.min_line_similarity,
            "transliterated Latin must not be treated as the Greek line"
        );
    }

    #[test]
    fn a_summarizing_model_is_refused_rather_than_half_applied() {
        let local = page(&["one", "two", "three", "four", "five"]);
        let aligned = align_transcription(
            &local,
            "This page discusses several numbered items.\n",
            &AlignmentConfig::default(),
        );
        assert!(!aligned.used_transcription);
        assert!(aligned.summary.coverage < AlignmentConfig::default().min_coverage);
        assert_eq!(
            text_of(&aligned.page),
            vec!["one", "two", "three", "four", "five"]
        );
    }

    #[test]
    fn word_boxes_are_inherited_when_the_word_counts_agree() {
        let local = page(&["Die Ubersetzung ist"]);
        let before = local.blocks[0].lines[0].words[1].bbox.clone();
        let aligned =
            align_transcription(&local, "Die Übersetzung ist\n", &AlignmentConfig::default());
        assert_eq!(aligned.page.blocks[0].lines[0].words[1].bbox, before);
        assert_eq!(aligned.page.blocks[0].lines[0].words[1].text, "Übersetzung");
    }

    #[test]
    fn a_split_word_stays_inside_the_measured_line_box() {
        let local = page(&["Ubersetzungsprobleme heute"]);
        let line_box = local.blocks[0].lines[0].bbox.clone();
        let aligned = align_transcription(
            &local,
            "Übersetzungs probleme heute\n",
            &AlignmentConfig {
                min_coverage: 0.5,
                ..Default::default()
            },
        );
        let line = &aligned.page.blocks[0].lines[0];
        assert_eq!(line.words.len(), 3);
        for word in &line.words {
            assert!(word.bbox.x >= line_box.x - 0.01);
            assert!(word.bbox.x + word.bbox.width <= line_box.x + line_box.width + 0.01);
            assert!(word.bbox.y >= line_box.y - 0.01);
        }
    }

    #[test]
    fn confidence_is_corroboration_and_never_a_model_self_report() {
        let local = page(&["Einleitng"]);
        let aligned = align_transcription(&local, "Einleitung\n", &AlignmentConfig::default());
        let line = &aligned.page.blocks[0].lines[0];
        assert!(line.confidence > 0.0 && line.confidence < 1.0);
        assert!(line
            .words
            .iter()
            .all(|word| word.confidence == line.confidence));
    }

    #[test]
    fn weak_agreement_cannot_borrow_certainty_from_a_confident_local_engine() {
        // A line the two engines barely agree on must not come out at 0.99
        // just because Tesseract was sure about the reading it was overruled
        // on. The ceiling at the replacement threshold is 0.69.
        assert!(corroborated_confidence(0.99, 0.55) <= 0.69);
        // Ordinary OCR-level disagreement is not weak agreement: two engines
        // differing on one character in eight are reading the same line.
        assert!(corroborated_confidence(0.97, 0.85) > 0.99);
        // Full agreement between two independent readings is the only way to
        // reach the top of the scale.
        assert_eq!(corroborated_confidence(0.97, 1.0), 1.0);
        // Agreement always helps, never hurts: corroboration cannot lower a
        // line below what the agreement itself supports.
        assert!(corroborated_confidence(0.5, 0.9) >= 0.9);
        assert!(corroborated_confidence(0.9, 0.4) < 0.6);
        assert!(corroborated_confidence(0.0, 0.0) >= 0.0);
        assert!(corroborated_confidence(2.0, 2.0) <= 1.0);
    }

    #[test]
    fn a_detached_accent_line_still_aligns_after_nfc_normalization() {
        // The composed and decomposed forms of the same polytonic line must
        // be the same line.
        let local = page(&["ἀρχή τοῦ λόγου"]);
        let decomposed: String = "ἀρχή τοῦ λόγου"
            .chars()
            .flat_map(|c| c.to_string().nfd().collect::<Vec<_>>())
            .collect();
        let aligned = align_transcription(
            &local,
            &format!("{decomposed}\n"),
            &AlignmentConfig::default(),
        );
        assert!(aligned.used_transcription);
        assert_eq!(aligned.summary.replaced, 1);
    }

    #[test]
    fn an_empty_transcription_leaves_the_local_page_exactly_as_it_was() {
        let local = page(&["alpha", "beta"]);
        let aligned = align_transcription(&local, "   \n\n", &AlignmentConfig::default());
        assert!(!aligned.used_transcription);
        assert_eq!(aligned.page, local);
        assert_eq!(aligned.summary.provider_lines, 0);
    }

    #[test]
    fn splitting_a_transcription_is_deterministic_and_bounded() {
        let text = "a\n\n  b  c \nd\n";
        assert_eq!(split_transcription(text, 16), vec!["a", "b c", "d"]);
        assert_eq!(split_transcription(text, 2), vec!["a", "b c"]);
    }
}
