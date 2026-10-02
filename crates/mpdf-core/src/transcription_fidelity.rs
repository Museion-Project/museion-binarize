//! Lossless transcription eligibility. This never edits or normalizes text.
//!
//! Eligibility is only a precondition: the installed PDF's actual encoded
//! spans/font maps must match literally and PDFium must confirm its objects.
//! PDFium layout synthesis is reported separately, never normalized to pass.

use serde::Serialize;
use unicode_normalization::{char::is_combining_mark, is_nfc};

use crate::error::{CoreError, Result};

mod pdf_literal;
pub use pdf_literal::decode_pdf_literal_spans;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FidelityClass {
    LosslessCandidate,
    NeedsReview,
    CanonicalRequired,
    Invalid,
}

impl FidelityClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LosslessCandidate => "LOSSLESS_CANDIDATE",
            Self::NeedsReview => "NEEDS_REVIEW",
            Self::CanonicalRequired => "CANONICAL_REQUIRED",
            Self::Invalid => "INVALID",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FidelityAssessment {
    pub classification: FidelityClass,
    pub reason_codes: Vec<&'static str>,
}

/// Classify literal Unicode; do not silently turn markup into rendered text,
/// trim whitespace, replace line breaks, or repair a decomposed sequence.
pub fn assess(text: &str) -> FidelityAssessment {
    let mut invalid = Vec::new();
    let mut canonical = Vec::new();
    let mut review = Vec::new();
    if text.trim().is_empty() {
        invalid.push("empty_or_whitespace");
    }
    if text.contains('\0') {
        invalid.push("nul_character");
    }
    if text
        .chars()
        .any(|c| matches!(c as u32, 1..=9 | 11..=12 | 14..=31 | 127..=159))
    {
        invalid.push("control_character");
    }
    if text.chars().any(|c| {
        let n = c as u32;
        (0xfdd0..=0xfdef).contains(&n) || n & 0xffff >= 0xfffe
    }) {
        invalid.push("unicode_noncharacter");
    }
    if text
        .chars()
        .any(|c| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
    {
        canonical.push("line_break_requires_representation");
    }
    if !is_nfc(text) {
        canonical.push("non_nfc");
    }
    // A backslash followed by a known TeX command is notation requiring an
    // explicit representation decision. Ordinary Unicode math stays literal.
    const COMMANDS: &[&str] = &[
        "sqrt",
        "frac",
        "dfrac",
        "tfrac",
        "text",
        "textrm",
        "textbf",
        "textit",
        "mathrm",
        "mathbf",
        "mathit",
        "mathbb",
        "mathcal",
        "operatorname",
        "begin",
        "end",
        "left",
        "right",
        "overline",
        "underline",
        "overset",
        "underset",
        "superscript",
        "subscript",
        "alpha",
        "beta",
        "gamma",
        "delta",
        "theta",
        "lambda",
        "mu",
        "pi",
        "sigma",
        "phi",
        "omega",
        "sum",
        "prod",
        "int",
        "infty",
        "cdot",
        "times",
        "leq",
        "geq",
    ];
    if text.split('\\').skip(1).any(|tail| {
        let end = tail
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(tail.len());
        COMMANDS.contains(&&tail[..end])
    }) {
        canonical.push("latex_markup");
    }
    if text.contains('\u{fffd}') {
        review.push("replacement_character");
    }
    if text
        .chars()
        .find(|c| !c.is_whitespace())
        .is_some_and(is_combining_mark)
    {
        review.push("leading_isolated_combining_mark");
    }
    let classification = if !invalid.is_empty() {
        FidelityClass::Invalid
    } else if !canonical.is_empty() {
        FidelityClass::CanonicalRequired
    } else if !review.is_empty() {
        FidelityClass::NeedsReview
    } else {
        FidelityClass::LosslessCandidate
    };
    invalid.extend(canonical);
    invalid.extend(review);
    FidelityAssessment {
        classification,
        reason_codes: invalid,
    }
}

/// Fail closed with stable classification and reason codes in the existing
/// error path. Include location, never the potentially sensitive text itself.
pub fn require_lossless(text: &str, location: &str) -> Result<()> {
    let assessment = assess(text);
    if assessment.classification != FidelityClass::LosslessCandidate {
        return Err(CoreError::OutputValidationFailed(format!(
            "transcription_fidelity:{}:{} at {}",
            assessment.classification.as_str(),
            assessment.reason_codes.join(","),
            location,
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_semantics_are_classified_without_repair() {
        for (text, class, reason) in [
            ("", FidelityClass::Invalid, "empty_or_whitespace"),
            (" \u{2003}", FidelityClass::Invalid, "empty_or_whitespace"),
            ("a\0b", FidelityClass::Invalid, "nul_character"),
            ("a\t b", FidelityClass::Invalid, "control_character"),
            ("a\u{85}b", FidelityClass::Invalid, "control_character"),
            ("a\u{fdd0}", FidelityClass::Invalid, "unicode_noncharacter"),
            (
                "a\u{10ffff}",
                FidelityClass::Invalid,
                "unicode_noncharacter",
            ),
            (
                "a\nb",
                FidelityClass::CanonicalRequired,
                "line_break_requires_representation",
            ),
            (
                "a\rb",
                FidelityClass::CanonicalRequired,
                "line_break_requires_representation",
            ),
            (
                "a\u{2028}b",
                FidelityClass::CanonicalRequired,
                "line_break_requires_representation",
            ),
            (
                "a\u{2029}b",
                FidelityClass::CanonicalRequired,
                "line_break_requires_representation",
            ),
            ("e\u{301}", FidelityClass::CanonicalRequired, "non_nfc"),
            (
                r"\frac{1}{2}",
                FidelityClass::CanonicalRequired,
                "latex_markup",
            ),
            (
                r"\sqrt{x}",
                FidelityClass::CanonicalRequired,
                "latex_markup",
            ),
            (
                r"\text{λόγος}",
                FidelityClass::CanonicalRequired,
                "latex_markup",
            ),
            (
                "a\u{fffd}",
                FidelityClass::NeedsReview,
                "replacement_character",
            ),
            (
                "\u{301}x",
                FidelityClass::NeedsReview,
                "leading_isolated_combining_mark",
            ),
        ] {
            let result = assess(text);
            assert_eq!(result.classification, class, "{text:?}");
            assert!(result.reason_codes.contains(&reason), "{text:?}");
            assert!(require_lossless(text, "fixture").is_err());
        }
        for text in [
            "Ἀρχὴ λόγος",
            "x² + y³ = ½",
            "α + β ≤ ∞",
            "x\u{301}",
            "text",
            " x ",
        ] {
            assert_eq!(
                assess(text).classification,
                FidelityClass::LosslessCandidate,
                "{text:?}"
            );
            require_lossless(text, "fixture").unwrap();
        }
    }
}
