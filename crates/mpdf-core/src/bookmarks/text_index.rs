//! Bounded, searchable line view over OCR evidence and the derived overlay.
//!
//! The automatic engine never walks `OcrRun` or `DerivedDocument` directly:
//! it consumes [`EvidenceLine`] records built once here, in master
//! coordinates, carrying both the human-effective text used for matching and
//! the stable source references used for auditability. Provider identity
//! never reaches this layer — only the route, which decides whether a
//! bounding box may be treated as measured layout evidence or merely as an
//! approximate native-text box.

use std::collections::{BTreeMap, HashMap};

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use crate::derived::{Bbox, DerivedDocument};
use crate::error::{CoreError, Result};
use crate::ocr::{OcrPage, OcrRoute, OcrRun};

use super::config::AutoBookmarkConfig;

/// How much a bounding box may be trusted as layout evidence.
///
/// `NativeText` pages synthesize line/word boxes from extracted text runs
/// (see `docs/adr/0005-local-ocr-routing.md`), so their x coordinates must
/// not drive column or font-size decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GeometryQuality {
    Measured,
    Approximate,
}

impl GeometryQuality {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::Approximate => "approximate",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EvidenceWord {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) bbox: Bbox,
    pub(crate) confidence: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct EvidenceLine {
    pub(crate) page_id: String,
    pub(crate) page_index: u32,
    pub(crate) line_id: String,
    pub(crate) ordinal: u32,
    /// Human-effective text (M4 revision overlay applied), unchanged Unicode.
    pub(crate) raw_text: String,
    /// The untouched OCR/native text the revision overlay was applied to.
    pub(crate) source_text: String,
    pub(crate) primary_key: String,
    pub(crate) secondary_key: String,
    pub(crate) tokens: Vec<String>,
    pub(crate) bbox: Bbox,
    pub(crate) min_confidence: f32,
    pub(crate) words: Vec<EvidenceWord>,
    pub(crate) geometry: GeometryQuality,
    /// Vertical position of the line top as permille of page height.
    pub(crate) top_permille: u32,
    pub(crate) repeated_furniture: bool,
}

impl EvidenceLine {
    pub(crate) fn is_blank(&self) -> bool {
        self.primary_key.is_empty()
    }

    pub(crate) fn word_ids(&self) -> Vec<String> {
        self.words.iter().map(|word| word.id.clone()).collect()
    }

    /// True when a human revision changed this line's text. The source text
    /// is kept either way, so the original OCR evidence stays auditable.
    pub(crate) fn human_revised(&self) -> bool {
        self.raw_text != self.source_text
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EvidencePage {
    pub(crate) page_id: String,
    pub(crate) page_index: u32,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) lines: Vec<EvidenceLine>,
    pub(crate) median_line_height: f64,
    pub(crate) geometry: GeometryQuality,
    /// True when the page's reading order repeats or skips values. The
    /// sequence is still used in its recorded order — it is only scored
    /// lower, never silently re-sorted into an asserted fact.
    pub(crate) degraded_order: bool,
}

/// A reference into [`TextIndex::pages`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LineRef {
    pub(crate) page: usize,
    pub(crate) line: usize,
}

pub(crate) struct TextIndex {
    pub(crate) pages: Vec<EvidencePage>,
    /// Token -> body lines containing it, in ascending [`LineRef`] order.
    postings: HashMap<String, Vec<LineRef>>,
    body_line_count: usize,
    pub(crate) truncated_postings: bool,
    /// How many posting entries the shortlist pass has visited. Reported so
    /// a test can prove the engine uses the inverted index instead of
    /// comparing every contents entry against every line in the book.
    pub(crate) visited_postings: u64,
}

/// A token present in more than this permille of body lines is treated as
/// non-distinctive and skipped when building a shortlist.
const NON_DISTINCTIVE_PERMILLE: usize = 200;
/// Postings longer than this are not scanned for a shortlist; the entry
/// falls back to its rarer tokens and the truncation is reported.
const MAX_SCANNED_POSTINGS: usize = 4_096;

impl TextIndex {
    /// Builds the line view. `ocr` supplies route and page identity; `derived`
    /// supplies master-space geometry and the effective (human-revised) text.
    pub(crate) fn build(
        package: &crate::document_package::DocumentPackage,
        ocr: &OcrRun,
        derived: &DerivedDocument,
        config: &AutoBookmarkConfig,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self> {
        bind_ocr_to_package(package, ocr)?;
        let routes: HashMap<u32, &OcrPage> = ocr
            .pages
            .iter()
            .map(|page| (page.page_index, page))
            .collect();
        let mut pages = Vec::with_capacity(derived.pages.len());
        for derived_page in &derived.pages {
            if cancelled() {
                return Err(CoreError::Cancelled);
            }
            let ocr_page = routes.get(&derived_page.page_index).ok_or_else(|| {
                CoreError::InvalidDocument("OCR page evidence is missing for a derived page".into())
            })?;
            let geometry = match ocr_page.route {
                OcrRoute::NativeText => GeometryQuality::Approximate,
                OcrRoute::Ocr { .. } => GeometryQuality::Measured,
            };
            let mut ordered: Vec<(u32, &str, u32, &str, &crate::derived::DerivedLine)> = Vec::new();
            for block in &derived_page.blocks {
                for line in &block.lines {
                    ordered.push((
                        block.reading_order,
                        block.structural_path.as_str(),
                        line.reading_order,
                        line.structural_path.as_str(),
                        line,
                    ));
                }
            }
            ordered.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| a.1.cmp(b.1))
                    .then_with(|| a.2.cmp(&b.2))
                    .then_with(|| a.3.cmp(b.3))
            });
            let mut seen_order = std::collections::HashSet::new();
            let mut degraded_order = false;
            let mut lines = Vec::with_capacity(ordered.len());
            for (ordinal, (_, _, line_order, _, line)) in ordered.iter().enumerate() {
                if !seen_order.insert(*line_order) {
                    degraded_order = true;
                }
                let raw_text = join_words(line.words.iter().map(|word| word.effective_text.trim()));
                let source_text = join_words(line.words.iter().map(|word| word.source_text.trim()));
                let primary = primary_key(&raw_text);
                let secondary = secondary_key(&primary);
                let confidences: Vec<f32> = line.words.iter().map(|word| word.confidence).collect();
                let min_confidence = confidences.iter().copied().fold(1.0_f32, f32::min);
                let height = derived_page.bbox.height.max(1.0);
                lines.push(EvidenceLine {
                    page_id: derived_page.page_id.clone(),
                    page_index: derived_page.page_index,
                    line_id: line.id.clone(),
                    ordinal: ordinal as u32,
                    tokens: index_keys(&primary, &secondary),
                    raw_text,
                    source_text,
                    primary_key: primary,
                    secondary_key: secondary,
                    bbox: line.bbox,
                    min_confidence,
                    words: line
                        .words
                        .iter()
                        .map(|word| EvidenceWord {
                            id: word.id.clone(),
                            text: word.effective_text.clone(),
                            bbox: word.bbox,
                            confidence: word.confidence,
                        })
                        .collect(),
                    geometry,
                    top_permille: ((line.bbox.y.max(0.0) / height) * 1000.0)
                        .round()
                        .min(1000.0) as u32,
                    repeated_furniture: false,
                });
            }
            let median_line_height = median(
                &mut lines
                    .iter()
                    .filter(|line| !line.is_blank())
                    .map(|line| line.bbox.height)
                    .collect::<Vec<_>>(),
            );
            pages.push(EvidencePage {
                page_id: derived_page.page_id.clone(),
                page_index: derived_page.page_index,
                width: derived_page.bbox.width,
                height: derived_page.bbox.height,
                lines,
                median_line_height,
                geometry,
                degraded_order,
            });
        }
        let mut index = Self {
            pages,
            postings: HashMap::new(),
            body_line_count: 0,
            truncated_postings: false,
            visited_postings: 0,
        };
        index.mark_repeated_furniture(config);
        Ok(index)
    }

    /// Cross-page running header/footer detection. A repeated short line in
    /// the top/bottom band of many pages is negative evidence for a heading;
    /// the original text is still kept on the line.
    fn mark_repeated_furniture(&mut self, config: &AutoBookmarkConfig) {
        let page_count = self.pages.len().max(1);
        let mut occurrences: BTreeMap<String, std::collections::BTreeSet<u32>> = BTreeMap::new();
        for page in &self.pages {
            let band = u64::from(config.furniture_band_percent);
            for line in &page.lines {
                if line.is_blank() || line.primary_key.chars().count() > 80 {
                    continue;
                }
                let top = u64::from(line.top_permille);
                if top <= band * 10 || top >= 1000 - band * 10 {
                    occurrences
                        .entry(line.primary_key.clone())
                        .or_default()
                        .insert(page.page_index);
                }
            }
        }
        let repeated: std::collections::BTreeSet<String> = occurrences
            .into_iter()
            .filter(|(_, pages)| {
                pages.len() as u32 >= config.furniture_min_pages
                    && pages.len() * 100 >= page_count * config.furniture_percent as usize
            })
            .map(|(key, _)| key)
            .collect();
        for page in &mut self.pages {
            for line in &mut page.lines {
                if repeated.contains(&line.primary_key) {
                    line.repeated_furniture = true;
                }
            }
        }
    }

    /// Builds the inverted index over the pages that are *not* printed
    /// contents pages, so a TOC line can never match itself.
    pub(crate) fn index_body(&mut self, toc_pages: &[u32]) {
        self.postings.clear();
        self.body_line_count = 0;
        for (page_ordinal, page) in self.pages.iter().enumerate() {
            if toc_pages.contains(&page.page_index) {
                continue;
            }
            for (line_ordinal, line) in page.lines.iter().enumerate() {
                if line.is_blank() {
                    continue;
                }
                self.body_line_count += 1;
                let reference = LineRef {
                    page: page_ordinal,
                    line: line_ordinal,
                };
                let mut seen = std::collections::HashSet::new();
                for token in &line.tokens {
                    if seen.insert(token.as_str()) {
                        self.postings
                            .entry(token.clone())
                            .or_default()
                            .push(reference);
                    }
                }
            }
        }
    }

    pub(crate) fn body_line_count(&self) -> usize {
        self.body_line_count
    }

    pub(crate) fn line(&self, reference: LineRef) -> &EvidenceLine {
        &self.pages[reference.page].lines[reference.line]
    }

    pub(crate) fn page(&self, reference: LineRef) -> &EvidencePage {
        &self.pages[reference.page]
    }

    /// Bounded shortlist for one contents entry. Never a full scan: only the
    /// postings of the entry's distinctive tokens are visited, and the result
    /// is capped by `limit`.
    ///
    /// Returns the ranked hits and whether *recall* was cut short — that is,
    /// whether a token's postings list was too long to scan, so the true best
    /// target may not be in the list at all. Merely capping the ranked tail is
    /// not recall truncation and does not weaken a candidate's decision.
    pub(crate) fn shortlist(
        &mut self,
        tokens: &[String],
        limit: usize,
    ) -> (Vec<(LineRef, u32)>, bool) {
        let non_distinctive = self.body_line_count * NON_DISTINCTIVE_PERMILLE / 1_000;
        let mut recall_truncated = false;
        let mut selected: Vec<&Vec<LineRef>> = Vec::new();
        let mut fallback: Vec<&Vec<LineRef>> = Vec::new();
        let mut seen_token = std::collections::HashSet::new();
        for token in tokens {
            if !seen_token.insert(token.as_str()) {
                continue;
            }
            let Some(postings) = self.postings.get(token) else {
                continue;
            };
            if postings.len() > MAX_SCANNED_POSTINGS {
                self.truncated_postings = true;
                recall_truncated = true;
                continue;
            }
            if postings.len() > non_distinctive.max(1) {
                fallback.push(postings);
            } else {
                selected.push(postings);
            }
        }
        if selected.is_empty() {
            // Every token is common in this document; use the rarest of them
            // rather than degrading into a full-document scan.
            fallback.sort_by_key(|postings| postings.len());
            selected = fallback.into_iter().take(4).collect();
        }
        let mut hits: BTreeMap<LineRef, u32> = BTreeMap::new();
        let mut visited = 0u64;
        for postings in selected {
            for reference in postings {
                visited += 1;
                *hits.entry(*reference).or_default() += 1;
            }
        }
        self.visited_postings = self.visited_postings.saturating_add(visited);
        let mut ranked: Vec<(LineRef, u32)> = hits.into_iter().collect();
        ranked.sort_by(|a, b| {
            b.1.cmp(&a.1).then_with(|| {
                let left = self.line(a.0);
                let right = self.line(b.0);
                left.page_index
                    .cmp(&right.page_index)
                    .then_with(|| left.bbox.y.total_cmp(&right.bbox.y))
                    .then_with(|| left.line_id.cmp(&right.line_id))
            })
        });
        ranked.truncate(limit);
        (ranked, recall_truncated)
    }
}

/// Strict one-to-one binding between OCR page identity and MDP pages.
pub(crate) fn bind_ocr_to_package(
    package: &crate::document_package::DocumentPackage,
    ocr: &OcrRun,
) -> Result<()> {
    if !ocr.is_complete(package.manifest.page_count) {
        return Err(CoreError::InvalidDocument(
            "OCR evidence does not cover every page".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for page in &ocr.pages {
        if !seen.insert(page.page_index) {
            return Err(CoreError::InvalidDocument(
                "OCR evidence has a duplicate page index".into(),
            ));
        }
        if !package
            .pages
            .iter()
            .any(|source| source.physical_index == page.page_index)
        {
            return Err(CoreError::InvalidDocument(
                "OCR page index is out of range for the package".into(),
            ));
        }
    }
    Ok(())
}

fn join_words<'a>(words: impl Iterator<Item = &'a str>) -> String {
    words
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    values[values.len() / 2]
}

/// The primary match key: NFKC, lowercase, punctuation and leader folding,
/// whitespace collapse. Never written back into a bookmark title.
pub(crate) fn primary_key(text: &str) -> String {
    let folded: String = text
        .nfkc()
        .filter(|character| *character != '\u{00ad}' && *character != '\u{200b}')
        .flat_map(|character| character.to_lowercase())
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The secondary key folds combining marks. Polytonic Greek accents differ
/// only here, which is why a secondary-only match cannot pass the automatic
/// gate on its own.
pub(crate) fn secondary_key(primary: &str) -> String {
    primary
        .nfd()
        .filter(|character| !is_combining_mark(*character))
        .collect::<String>()
        .nfc()
        .collect()
}

fn is_ideographic(character: char) -> bool {
    matches!(character as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
}

/// Index keys for a line: the primary tokens plus the accent-folded ones, so
/// a contents entry whose accents differ from the body heading is still
/// *recalled*. Whether such a match may be trusted is decided later, by the
/// secondary-only rule in the scoring gate.
pub(crate) fn index_keys(primary: &str, secondary: &str) -> Vec<String> {
    let mut tokens = tokens_of(primary);
    for token in tokens_of(secondary) {
        if !tokens.contains(&token) {
            tokens.push(token);
        }
    }
    tokens.truncate(512);
    tokens
}

/// Whitespace tokens, plus character bigrams for scripts that do not use
/// spaces, so a Chinese contents line still has distinctive index keys.
pub(crate) fn tokens_of(primary: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for token in primary.split_whitespace() {
        let characters: Vec<char> = token.chars().collect();
        if characters
            .iter()
            .any(|character| is_ideographic(*character))
        {
            if characters.len() == 1 {
                tokens.push(token.to_owned());
            }
            for window in characters.windows(2) {
                tokens.push(window.iter().collect());
            }
        } else {
            tokens.push(token.to_owned());
        }
    }
    tokens.truncate(256);
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_key_folds_case_punctuation_and_leaders() {
        assert_eq!(
            primary_key("1.2  Introduction .... 45"),
            "1 2 introduction 45"
        );
        assert_eq!(primary_key("Sof\u{00ad}t hyphen"), "soft hyphen");
    }

    #[test]
    fn secondary_key_folds_polytonic_accents_only() {
        let primary = primary_key("Ἀρχή");
        assert_ne!(primary, secondary_key(&primary));
        assert_eq!(secondary_key(&primary), secondary_key(&primary_key("αρχη")));
    }

    #[test]
    fn ideographic_tokens_become_bigrams() {
        assert_eq!(tokens_of("第一章 绪论"), vec!["第一", "一章", "绪论"]);
    }
}
