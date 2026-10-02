//! Traceable text-only enhancement over canonical OCR geometry.
//!
//! An enhancer never receives an image and never returns an [`OcrPage`]. It
//! proposes typed word patches; the core applies them after checking the stable
//! reading-order path and a digest of the original text. Consequently an
//! enhancer has no API with which to move a box, reorder a line, or resize a
//! page.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::ocr::{validate_ocr_page, OcrPage};

/// Stable identity of one word inside a page's reading-order tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrWordPath {
    pub page_index: u32,
    pub block_reading_order: u32,
    pub line_reading_order: u32,
    pub word_reading_order: u32,
}

/// A proposed replacement, bound to both position and original bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrTextPatch {
    pub target: OcrWordPath,
    pub original_text_sha256: String,
    /// Must already be Unicode NFC. The core refuses to normalize an opaque
    /// enhancer result silently because that would make the recorded patch and
    /// the applied text differ.
    pub replacement_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextEnhancement {
    pub enhancer_id: String,
    pub enhancer_version: String,
    pub patches: Vec<OcrTextPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedTextPatch {
    pub target: OcrWordPath,
    pub original_text_sha256: String,
    pub replacement_text_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextEnhancementResult {
    pub page: OcrPage,
    pub enhancer_id: String,
    pub enhancer_version: String,
    pub lineage: Vec<AppliedTextPatch>,
}

/// A text enhancer can propose patches only. Geometry is deliberately absent
/// from the return type.
pub trait TextEnhancer {
    fn enhancer_id(&self) -> &'static str;
    fn enhancer_version(&self) -> &'static str;
    fn propose(&self, page: &OcrPage) -> Result<Vec<OcrTextPatch>, TextEnhancementError>;

    fn enhancement(&self, page: &OcrPage) -> Result<TextEnhancement, TextEnhancementError> {
        Ok(TextEnhancement {
            enhancer_id: self.enhancer_id().to_owned(),
            enhancer_version: self.enhancer_version().to_owned(),
            patches: self.propose(page)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextEnhancementError {
    #[error("invalid input OCR page: {0}")]
    InvalidInput(String),
    #[error("enhancer identity is empty")]
    MissingEnhancerIdentity,
    #[error("duplicate text patch target")]
    DuplicateTarget,
    #[error("text patch target does not resolve to an original OCR word")]
    TargetNotFound,
    #[error("text patch original digest does not match the targeted OCR word")]
    OriginalDigestMismatch,
    #[error("text patch replacement must be non-empty Unicode NFC")]
    ReplacementNotNfc,
    #[error("text patch replacement is identical to its source")]
    NoOp,
    #[error("enhanced OCR page is invalid: {0}")]
    InvalidOutput(String),
    #[error("every enhanced OCR word and normalized word must be Unicode NFC")]
    OutputNotNfc,
}

/// Applies an enhancement plan through the only mutation path exposed to text
/// enhancers. The source page is cloned; boxes, confidence, reading order,
/// route, dimensions, revisions and provenance are never assigned here.
pub fn apply_text_enhancement(
    page: &OcrPage,
    enhancement: &TextEnhancement,
) -> Result<TextEnhancementResult, TextEnhancementError> {
    validate_ocr_page(page)
        .map_err(|error| TextEnhancementError::InvalidInput(error.to_string()))?;
    if enhancement.enhancer_id.trim().is_empty() || enhancement.enhancer_version.trim().is_empty() {
        return Err(TextEnhancementError::MissingEnhancerIdentity);
    }

    let mut enhanced = page.clone();
    let mut seen = HashSet::new();
    let mut lineage = Vec::with_capacity(enhancement.patches.len());
    for patch in &enhancement.patches {
        if !seen.insert(patch.target.clone()) {
            return Err(TextEnhancementError::DuplicateTarget);
        }
        let normalized: String = patch.replacement_text.nfc().collect();
        if patch.replacement_text.is_empty() || normalized != patch.replacement_text {
            return Err(TextEnhancementError::ReplacementNotNfc);
        }
        if patch.target.page_index != enhanced.page_index {
            return Err(TextEnhancementError::TargetNotFound);
        }
        let word = enhanced
            .blocks
            .iter_mut()
            .find(|block| block.reading_order == patch.target.block_reading_order)
            .and_then(|block| {
                block
                    .lines
                    .iter_mut()
                    .find(|line| line.reading_order == patch.target.line_reading_order)
            })
            .and_then(|line| {
                line.words
                    .iter_mut()
                    .find(|word| word.reading_order == patch.target.word_reading_order)
            })
            .ok_or(TextEnhancementError::TargetNotFound)?;
        if text_sha256(&word.text) != patch.original_text_sha256 {
            return Err(TextEnhancementError::OriginalDigestMismatch);
        }
        if word.text == patch.replacement_text {
            return Err(TextEnhancementError::NoOp);
        }
        let replacement_digest = text_sha256(&patch.replacement_text);
        word.text.clone_from(&patch.replacement_text);
        word.normalized_text.clone_from(&patch.replacement_text);
        lineage.push(AppliedTextPatch {
            target: patch.target.clone(),
            original_text_sha256: patch.original_text_sha256.clone(),
            replacement_text_sha256: replacement_digest,
        });
    }
    validate_ocr_page(&enhanced)
        .map_err(|error| TextEnhancementError::InvalidOutput(error.to_string()))?;
    if enhanced.blocks.iter().any(|block| {
        block.lines.iter().any(|line| {
            line.words.iter().any(|word| {
                word.text.nfc().ne(word.text.chars())
                    || word.normalized_text.nfc().ne(word.normalized_text.chars())
            })
        })
    }) {
        return Err(TextEnhancementError::OutputNotNfc);
    }
    Ok(TextEnhancementResult {
        page: enhanced,
        enhancer_id: enhancement.enhancer_id.clone(),
        enhancer_version: enhancement.enhancer_version.clone(),
        lineage,
    })
}

pub fn text_sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Minimal deterministic Modern-Greek-to-Ancient-Greek post-processing
/// hypothesis.
///
/// This does not invent breathings, accents, or dictionary forms. It performs
/// NFC and, only inside an otherwise Greek token with high OCR confidence,
/// replaces a small set of Latin glyph lookalikes with their Greek codepoints.
/// It is an evaluation candidate, not a product default.
#[derive(Debug, Clone, Copy)]
pub struct ConservativeGreekEnhancer {
    pub minimum_confidence: f32,
}

impl Default for ConservativeGreekEnhancer {
    fn default() -> Self {
        Self {
            minimum_confidence: 0.95,
        }
    }
}

impl TextEnhancer for ConservativeGreekEnhancer {
    fn enhancer_id(&self) -> &'static str {
        "conservative-greek-text"
    }

    fn enhancer_version(&self) -> &'static str {
        "1"
    }

    fn propose(&self, page: &OcrPage) -> Result<Vec<OcrTextPatch>, TextEnhancementError> {
        validate_ocr_page(page)
            .map_err(|error| TextEnhancementError::InvalidInput(error.to_string()))?;
        let mut patches = Vec::new();
        for block in &page.blocks {
            for line in &block.lines {
                for word in &line.words {
                    let nfc: String = word.text.nfc().collect();
                    let replacement = if word.confidence >= self.minimum_confidence
                        && is_conservative_greek_token(&nfc)
                    {
                        nfc.chars().map(repair_lookalike).collect()
                    } else {
                        nfc
                    };
                    if replacement != word.text {
                        patches.push(OcrTextPatch {
                            target: OcrWordPath {
                                page_index: page.page_index,
                                block_reading_order: block.reading_order,
                                line_reading_order: line.reading_order,
                                word_reading_order: word.reading_order,
                            },
                            original_text_sha256: text_sha256(&word.text),
                            replacement_text: replacement,
                        });
                    }
                }
            }
        }
        Ok(patches)
    }
}

fn is_greek(character: char) -> bool {
    matches!(character as u32, 0x0370..=0x03ff | 0x1f00..=0x1fff)
}

fn repair_lookalike(character: char) -> char {
    match character {
        'A' => 'Α',
        'B' => 'Β',
        'E' => 'Ε',
        'H' => 'Η',
        'I' => 'Ι',
        'K' => 'Κ',
        'M' => 'Μ',
        'N' => 'Ν',
        'O' => 'Ο',
        'P' => 'Ρ',
        'T' => 'Τ',
        'X' => 'Χ',
        'Y' => 'Υ',
        'Z' => 'Ζ',
        'o' => 'ο',
        'p' => 'ρ',
        'x' => 'χ',
        other => other,
    }
}

fn is_lookalike(character: char) -> bool {
    repair_lookalike(character) != character
}

fn is_conservative_greek_token(text: &str) -> bool {
    text.chars().any(is_greek)
        && text
            .chars()
            .filter(|character| character.is_alphabetic())
            .all(|character| is_greek(character) || is_lookalike(character))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::{OcrBlock, OcrBox, OcrLine, OcrRoute, OcrRouteReason, OcrWord};

    fn page(text: &str, confidence: f32) -> OcrPage {
        let bbox = OcrBox {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 12.0,
        };
        OcrPage {
            page_index: 7,
            route: OcrRoute::Ocr {
                reason: OcrRouteReason::MissingText,
            },
            width: 100,
            height: 200,
            blocks: vec![OcrBlock {
                bbox: bbox.clone(),
                confidence,
                reading_order: 4,
                lines: vec![OcrLine {
                    bbox: bbox.clone(),
                    confidence,
                    reading_order: 9,
                    words: vec![OcrWord {
                        text: text.to_owned(),
                        normalized_text: text.to_owned(),
                        bbox,
                        confidence,
                        reading_order: 3,
                    }],
                }],
            }],
            revisions: Vec::new(),
            provider_provenance: None,
            provider_raw_artifact: None,
        }
    }

    #[test]
    fn a_valid_patch_preserves_geometry_order_dimensions_and_lineage() {
        let source = page("ἀνθpωπος", 0.99);
        let enhancement = ConservativeGreekEnhancer::default()
            .enhancement(&source)
            .unwrap();
        let result = apply_text_enhancement(&source, &enhancement).unwrap();
        assert_eq!(result.page.blocks[0].lines[0].words[0].text, "ἀνθρωπος");
        let mut expected = source.clone();
        expected.blocks[0].lines[0].words[0].text = "ἀνθρωπος".into();
        expected.blocks[0].lines[0].words[0].normalized_text = "ἀνθρωπος".into();
        assert_eq!(result.page, expected);
        assert_eq!(source.blocks[0].lines[0].words[0].text, "ἀνθpωπος");
        assert_eq!(result.page.width, source.width);
        assert_eq!(result.page.height, source.height);
        assert_eq!(result.page.blocks[0].bbox, source.blocks[0].bbox);
        assert_eq!(
            result.page.blocks[0].reading_order,
            source.blocks[0].reading_order
        );
        assert_eq!(
            result.page.blocks[0].lines[0].bbox,
            source.blocks[0].lines[0].bbox
        );
        assert_eq!(
            result.page.blocks[0].lines[0].reading_order,
            source.blocks[0].lines[0].reading_order
        );
        assert_eq!(
            result.page.blocks[0].lines[0].words[0].bbox,
            source.blocks[0].lines[0].words[0].bbox
        );
        assert_eq!(
            result.page.blocks[0].lines[0].words[0].reading_order,
            source.blocks[0].lines[0].words[0].reading_order
        );
        assert_eq!(result.lineage.len(), 1);
        assert_eq!(
            result.lineage[0].original_text_sha256,
            text_sha256("ἀνθpωπος")
        );
    }

    #[test]
    fn an_untraceable_patch_is_rejected() {
        let source = page("λογος", 1.0);
        let patch = OcrTextPatch {
            target: OcrWordPath {
                page_index: 7,
                block_reading_order: 4,
                line_reading_order: 9,
                word_reading_order: 3,
            },
            original_text_sha256: text_sha256("different source"),
            replacement_text: "λόγος".into(),
        };
        let error = apply_text_enhancement(
            &source,
            &TextEnhancement {
                enhancer_id: "fixture".into(),
                enhancer_version: "1".into(),
                patches: vec![patch],
            },
        )
        .unwrap_err();
        assert_eq!(error, TextEnhancementError::OriginalDigestMismatch);
    }

    #[test]
    fn output_must_be_nfc_and_low_confidence_lookalikes_are_left_alone() {
        let source = page("λoγος", 0.5);
        assert!(ConservativeGreekEnhancer::default()
            .enhancement(&source)
            .unwrap()
            .patches
            .is_empty());
        let patch = OcrTextPatch {
            target: OcrWordPath {
                page_index: 7,
                block_reading_order: 4,
                line_reading_order: 9,
                word_reading_order: 3,
            },
            original_text_sha256: text_sha256("λoγος"),
            replacement_text: "α\u{301}".into(),
        };
        assert_eq!(
            apply_text_enhancement(
                &source,
                &TextEnhancement {
                    enhancer_id: "fixture".into(),
                    enhancer_version: "1".into(),
                    patches: vec![patch],
                },
            )
            .unwrap_err(),
            TextEnhancementError::ReplacementNotNfc
        );
    }

    #[test]
    fn nfc_is_allowed_without_inventing_polytonic_marks() {
        let source = page("α\u{313}\u{301}", 0.2);
        let plan = ConservativeGreekEnhancer::default()
            .enhancement(&source)
            .unwrap();
        let result = apply_text_enhancement(&source, &plan).unwrap();
        assert_eq!(result.page.blocks[0].lines[0].words[0].text, "ἄ");
        assert_eq!(result.lineage.len(), 1);
    }

    #[test]
    fn an_enhancer_cannot_leave_unpatched_non_nfc_text_in_its_output() {
        let source = page("α\u{313}\u{301}", 1.0);
        let error = apply_text_enhancement(
            &source,
            &TextEnhancement {
                enhancer_id: "incomplete-fixture".into(),
                enhancer_version: "1".into(),
                patches: Vec::new(),
            },
        )
        .unwrap_err();
        assert_eq!(error, TextEnhancementError::OutputNotNfc);
    }
}
