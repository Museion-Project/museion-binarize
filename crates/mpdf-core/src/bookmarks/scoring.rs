//! The frozen integer score and the automatic-confirmation gate.
//!
//! Scores are integers on a 0..=10,000 scale so that a decision never
//! depends on floating-point comparison order. The public `confidence` is
//! the total divided by 10,000 and exists only for display and for the 0.1
//! contract; the breakdown is what an auditor reads.

use super::config::{
    AutoBookmarkConfig, SCORE_LAYOUT_MAX, SCORE_NUMBERING_MAX, SCORE_OCR_MAX, SCORE_PAGE_MAX,
    SCORE_SEQUENCE_MAX, SCORE_TITLE_MAX, SCORE_TOTAL,
};
use super::model::{BookmarkStatus, ConfidenceBreakdown};

/// Everything the gate needs beyond the numeric breakdown.
#[derive(Debug, Clone)]
pub(crate) struct GateContext {
    pub(crate) min_toc_confidence: f32,
    pub(crate) min_body_confidence: f32,
    pub(crate) runner_up_margin: u32,
    pub(crate) has_body_evidence: bool,
    pub(crate) printed_page_residual: Option<i64>,
    pub(crate) residual_supported: bool,
    pub(crate) level_ambiguous: bool,
    pub(crate) secondary_only: bool,
    pub(crate) monotone: bool,
    pub(crate) approximate_multi_column: bool,
    pub(crate) repeated_furniture: bool,
    pub(crate) truncated: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Decision {
    pub(crate) status: BookmarkStatus,
    pub(crate) reason: String,
    pub(crate) reason_codes: Vec<String>,
}

pub(crate) fn breakdown(
    title: u32,
    page: u32,
    numbering: u32,
    layout: u32,
    ocr: u32,
    sequence: u32,
) -> ConfidenceBreakdown {
    let title = title.min(SCORE_TITLE_MAX);
    let page = page.min(SCORE_PAGE_MAX);
    let numbering = numbering.min(SCORE_NUMBERING_MAX);
    let layout = layout.min(SCORE_LAYOUT_MAX);
    let ocr = ocr.min(SCORE_OCR_MAX);
    let sequence = sequence.min(SCORE_SEQUENCE_MAX);
    ConfidenceBreakdown {
        title_match: title,
        page_mapping: page,
        numbering_hierarchy: numbering,
        body_layout: layout,
        ocr_quality: ocr,
        sequence_uniqueness: sequence,
        total: (title + page + numbering + layout + ocr + sequence).min(SCORE_TOTAL),
    }
}

/// Page-consensus component from the printed-label residual.
pub(crate) fn page_score(
    residual: Option<i64>,
    config: &AutoBookmarkConfig,
) -> (u32, &'static str) {
    match residual {
        Some(0) => (SCORE_PAGE_MAX, "printed_page_exact"),
        Some(value) if value.unsigned_abs() <= u64::from(config.max_page_residual) => {
            (SCORE_PAGE_MAX * 3 / 5, "printed_page_near")
        }
        Some(_) => (0, "printed_page_disagrees"),
        None => (0, "printed_page_unmapped"),
    }
}

/// Uniqueness component from the margin over the runner-up target.
pub(crate) fn sequence_score(margin: u32, monotone: bool, config: &AutoBookmarkConfig) -> u32 {
    let monotone_part = if monotone { 600 } else { 0 };
    let unique_part = if margin >= config.auto_confirm_margin {
        400
    } else {
        margin * 400 / config.auto_confirm_margin.max(1)
    };
    (monotone_part + unique_part).min(SCORE_SEQUENCE_MAX)
}

fn permille(value: f32) -> u32 {
    (f64::from(value.clamp(0.0, 1.0)) * 1_000.0).round() as u32
}

/// The frozen gate. Anything that is not unambiguously supported becomes
/// `needs_review` or `skipped`; nothing here can promote a weak match.
pub(crate) fn decide(
    breakdown: &ConfidenceBreakdown,
    context: &GateContext,
    config: &AutoBookmarkConfig,
) -> Decision {
    let mut blockers: Vec<String> = Vec::new();
    if !context.has_body_evidence {
        blockers.push("no_body_heading_evidence".to_owned());
    }
    if context.repeated_furniture {
        blockers.push("repeated_header_footer".to_owned());
    }
    if breakdown.total < config.auto_confirm_total {
        blockers.push("total_score_below_gate".to_owned());
    }
    if breakdown.title_match < config.auto_confirm_title {
        blockers.push("title_score_below_gate".to_owned());
    }
    if permille(context.min_toc_confidence) < config.auto_confirm_min_word_confidence_permille
        || permille(context.min_body_confidence) < config.auto_confirm_min_word_confidence_permille
    {
        blockers.push("low_word_confidence".to_owned());
    }
    if context.runner_up_margin < config.auto_confirm_margin {
        blockers.push("runner_up_margin_too_small".to_owned());
    }
    match context.printed_page_residual {
        None => blockers.push("printed_page_unmapped".to_owned()),
        Some(0) => {}
        Some(value) if value.unsigned_abs() <= u64::from(config.max_page_residual) => {
            if !context.residual_supported {
                blockers.push("unsupported_page_residual".to_owned());
            }
        }
        Some(_) => blockers.push("printed_page_disagrees".to_owned()),
    }
    if context.level_ambiguous {
        blockers.push("ambiguous_level".to_owned());
    }
    if context.secondary_only {
        blockers.push("secondary_key_match_only".to_owned());
    }
    if !context.monotone {
        blockers.push("non_monotonic_target".to_owned());
    }
    if context.approximate_multi_column {
        blockers.push("approximate_geometry_multi_column".to_owned());
    }
    if context.truncated {
        blockers.push("resource_limit_truncated".to_owned());
    }

    if blockers.is_empty() {
        return Decision {
            status: BookmarkStatus::AutoConfirmed,
            reason: "toc_body_alignment_consensus".to_owned(),
            reason_codes: vec![
                "toc_body_alignment_consensus".to_owned(),
                "printed_page_consensus".to_owned(),
            ],
        };
    }
    let unrecoverable = blockers.iter().any(|code| {
        matches!(
            code.as_str(),
            "no_body_heading_evidence" | "repeated_header_footer"
        )
    });
    if unrecoverable || breakdown.total < config.review_total {
        blockers.truncate(32);
        return Decision {
            status: BookmarkStatus::NeedsReview,
            reason: blockers.first().cloned().unwrap_or_default(),
            reason_codes: blockers,
        }
        .into_skipped();
    }
    blockers.truncate(32);
    Decision {
        reason: blockers.first().cloned().unwrap_or_default(),
        status: BookmarkStatus::NeedsReview,
        reason_codes: blockers,
    }
}

impl Decision {
    /// Below the review floor an entry is not carried as a proposal: it is
    /// reported as skipped with its blocking reasons intact.
    fn into_skipped(mut self) -> Self {
        self.status = BookmarkStatus::Skipped;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_context() -> GateContext {
        GateContext {
            min_toc_confidence: 0.95,
            min_body_confidence: 0.95,
            runner_up_margin: 900,
            has_body_evidence: true,
            printed_page_residual: Some(0),
            residual_supported: true,
            level_ambiguous: false,
            secondary_only: false,
            monotone: true,
            approximate_multi_column: false,
            repeated_furniture: false,
            truncated: false,
        }
    }

    fn passing_breakdown() -> ConfidenceBreakdown {
        breakdown(3_900, 2_000, 1_000, 900, 950, 1_000)
    }

    #[test]
    fn a_fully_supported_entry_passes_the_gate() {
        let config = AutoBookmarkConfig::default();
        let decision = decide(&passing_breakdown(), &passing_context(), &config);
        assert_eq!(decision.status, BookmarkStatus::AutoConfirmed);
    }

    #[test]
    fn each_single_blocker_prevents_automatic_confirmation() {
        let config = AutoBookmarkConfig::default();
        type Mutation = (&'static str, Box<dyn Fn(&mut GateContext)>);
        let mutations: Vec<Mutation> = vec![
            (
                "confidence",
                Box::new(|c: &mut GateContext| c.min_body_confidence = 0.6),
            ),
            (
                "margin",
                Box::new(|c: &mut GateContext| c.runner_up_margin = 100),
            ),
            (
                "residual",
                Box::new(|c: &mut GateContext| c.printed_page_residual = Some(4)),
            ),
            (
                "level",
                Box::new(|c: &mut GateContext| c.level_ambiguous = true),
            ),
            (
                "secondary",
                Box::new(|c: &mut GateContext| c.secondary_only = true),
            ),
            (
                "monotone",
                Box::new(|c: &mut GateContext| c.monotone = false),
            ),
            (
                "columns",
                Box::new(|c: &mut GateContext| c.approximate_multi_column = true),
            ),
            (
                "truncated",
                Box::new(|c: &mut GateContext| c.truncated = true),
            ),
        ];
        for (name, mutate) in mutations {
            let mut context = passing_context();
            mutate(&mut context);
            let decision = decide(&passing_breakdown(), &context, &config);
            assert_ne!(
                decision.status,
                BookmarkStatus::AutoConfirmed,
                "{name} must block automatic confirmation"
            );
        }
    }

    #[test]
    fn missing_body_evidence_is_skipped_not_reviewed() {
        let config = AutoBookmarkConfig::default();
        let mut context = passing_context();
        context.has_body_evidence = false;
        assert_eq!(
            decide(&passing_breakdown(), &context, &config).status,
            BookmarkStatus::Skipped
        );
    }
}
