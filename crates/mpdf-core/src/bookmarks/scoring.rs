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
    /// Other entries whose own observed printed-to-physical offset equals
    /// this entry's mapping segment offset. Excludes this entry: it may not
    /// corroborate itself.
    pub(crate) mapping_exact_independent_anchors: u32,
    /// Anchors the solver kept in the segment despite their offset
    /// disagreeing. Reported for audit; never counts as support.
    pub(crate) mapping_disagreeing_anchors: u32,
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

    // Structural consensus: a second route for entries whose *placement* is
    // demonstrably right but whose numbers fall short on one axis.
    //
    // Every blocker that decides whether the bookmark lands on the correct
    // page must still be clear. Only `total_score_below_gate`,
    // `title_score_below_gate` and `low_word_confidence` may be outweighed,
    // and only by a mapping many other entries independently agree with.
    if let Some(codes) = structural_consensus(breakdown, context, config, &blockers) {
        return Decision {
            status: BookmarkStatus::AutoConfirmed,
            reason: "printed_page_mapping_consensus".to_owned(),
            reason_codes: codes,
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

/// Blockers the consensus path is allowed to outweigh. Everything else is a
/// statement about whether the target page is right, and is never traded.
const COMPENSABLE: [&str; 3] = [
    "total_score_below_gate",
    "title_score_below_gate",
    "low_word_confidence",
];

fn structural_consensus(
    breakdown: &ConfidenceBreakdown,
    context: &GateContext,
    config: &AutoBookmarkConfig,
    blockers: &[String],
) -> Option<Vec<String>> {
    if blockers
        .iter()
        .any(|code| !COMPENSABLE.contains(&code.as_str()))
    {
        return None;
    }
    // The mapping must be exact and independently corroborated. `Some(0)`
    // only: a residual inside tolerance is not the same claim as a residual
    // that agrees exactly.
    if context.printed_page_residual != Some(0) {
        return None;
    }
    // Only anchors that actually agree, and never this entry itself. A run of
    // fourteen members with one exact anchor is not consensus; it is a
    // solver that paid thirteen mismatch penalties to hold a segment together.
    if context.mapping_exact_independent_anchors < config.consensus_min_anchors {
        return None;
    }
    // Safety properties at full strength, not relaxed.
    if !context.has_body_evidence
        || context.repeated_furniture
        || context.level_ambiguous
        || context.secondary_only
        || !context.monotone
        || context.approximate_multi_column
        || context.truncated
    {
        return None;
    }
    if context.runner_up_margin < config.consensus_min_margin {
        return None;
    }
    if breakdown.total < config.consensus_min_total
        || breakdown.title_match < config.consensus_min_title
    {
        return None;
    }
    // OCR uncertainty may be outweighed, never ignored.
    let floor = config.consensus_min_word_confidence_permille;
    if permille(context.min_toc_confidence) < floor || permille(context.min_body_confidence) < floor
    {
        return None;
    }
    let mut codes = vec![
        "printed_page_mapping_consensus".to_owned(),
        "printed_page_exact".to_owned(),
        format!(
            "mapping_exact_independent_anchors_{}",
            context.mapping_exact_independent_anchors
        ),
        format!(
            "mapping_disagreeing_anchors_{}",
            context.mapping_disagreeing_anchors
        ),
        "unique_target_margin_clear".to_owned(),
        "monotonic_target".to_owned(),
        "body_heading_present".to_owned(),
    ];
    // Keep what was compensated visible in the audit trail.
    codes.extend(blockers.iter().map(|code| format!("compensated_{code}")));
    codes.truncate(32);
    Some(codes)
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
pub(crate) mod tests_support {
    use super::*;

    pub(crate) fn passing() -> GateContext {
        super::tests::passing_context()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn passing_context() -> GateContext {
        GateContext {
            min_toc_confidence: 0.95,
            min_body_confidence: 0.95,
            runner_up_margin: 900,
            has_body_evidence: true,
            printed_page_residual: Some(0),
            residual_supported: true,
            mapping_exact_independent_anchors: 8,
            mapping_disagreeing_anchors: 0,
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
        // Rule 0.3 note: word confidence moved off this list. It is now a
        // *compensable* blocker -- an exactly agreeing, independently
        // corroborated printed-page mapping can outweigh it, down to a hard
        // floor. `consensus_tests` pins both halves of that contract. Every
        // condition that decides whether the bookmark lands on the right page
        // is still absolute and stays here.
        let mutations: Vec<Mutation> = vec![
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

#[cfg(test)]
mod consensus_tests {
    use super::tests_support::*;
    use super::*;

    /// The case the consensus path exists for: placement is demonstrably
    /// right, the numbers fall a little short on one axis.
    fn near_miss() -> (ConfidenceBreakdown, GateContext) {
        let mut context = passing();
        context.min_toc_confidence = 0.45;
        context.min_body_confidence = 0.45;
        (breakdown(4_000, 2_000, 1_000, 550, 200, 1_000), context)
    }

    #[test]
    fn low_word_confidence_is_outweighed_by_an_agreed_mapping() {
        let (score, context) = near_miss();
        let config = AutoBookmarkConfig::default();
        // It fails the ordinary gate ...
        assert!(score.total < config.auto_confirm_total);
        let decision = decide(&score, &context, &config);
        assert_eq!(decision.status, BookmarkStatus::AutoConfirmed);
        assert_eq!(decision.reason, "printed_page_mapping_consensus");
        // ... and the audit trail says exactly what was compensated.
        assert!(decision
            .reason_codes
            .iter()
            .any(|code| code == "compensated_low_word_confidence"));
        assert!(decision
            .reason_codes
            .iter()
            .any(|code| code.starts_with("mapping_exact_independent_anchors_")));
    }

    #[test]
    fn consensus_cannot_buy_past_the_word_confidence_floor() {
        let (score, mut context) = near_miss();
        let config = AutoBookmarkConfig::default();
        context.min_toc_confidence = 0.04;
        context.mapping_exact_independent_anchors = 64;
        assert_eq!(
            decide(&score, &context, &config).status,
            BookmarkStatus::NeedsReview,
            "no amount of agreement may confirm text the engine could not read"
        );
    }

    #[test]
    fn consensus_requires_an_exactly_agreeing_printed_page() {
        let (score, mut context) = near_miss();
        let config = AutoBookmarkConfig::default();
        context.printed_page_residual = Some(1);
        context.residual_supported = true;
        assert_ne!(
            decide(&score, &context, &config).status,
            BookmarkStatus::AutoConfirmed
        );
    }

    #[test]
    fn consensus_requires_corroborating_anchors() {
        let (score, mut context) = near_miss();
        let config = AutoBookmarkConfig::default();
        context.mapping_exact_independent_anchors = config.consensus_min_anchors - 1;
        assert_ne!(
            decide(&score, &context, &config).status,
            BookmarkStatus::AutoConfirmed
        );
    }

    /// A large segment carrying almost no agreement is not consensus.
    ///
    /// This is the shape rule 0.3 got wrong: the dynamic program is allowed to
    /// keep disagreeing anchors in a run, paying a mismatch penalty, so that
    /// one stray heading cannot fork the mapping. Counting members therefore
    /// counted the anchors the solver had overruled.
    #[test]
    fn a_large_segment_with_one_exact_anchor_is_not_consensus() {
        let (score, mut context) = near_miss();
        let config = AutoBookmarkConfig::default();
        context.mapping_exact_independent_anchors = 0;
        context.mapping_disagreeing_anchors = 13;
        assert_ne!(
            decide(&score, &context, &config).status,
            BookmarkStatus::AutoConfirmed,
            "13 overruled anchors plus this entry is not corroboration"
        );
    }

    #[test]
    fn an_entry_may_not_corroborate_itself() {
        // The engine subtracts the entry from its own segment before this
        // point; the gate must refuse what is left when that leaves nothing.
        let (score, mut context) = near_miss();
        let config = AutoBookmarkConfig::default();
        context.mapping_exact_independent_anchors = 0;
        context.mapping_disagreeing_anchors = 0;
        assert_ne!(
            decide(&score, &context, &config).status,
            BookmarkStatus::AutoConfirmed
        );
    }

    #[test]
    fn no_number_of_disagreeing_anchors_substitutes_for_exact_ones() {
        let (score, mut context) = near_miss();
        let config = AutoBookmarkConfig::default();
        context.mapping_exact_independent_anchors = config.consensus_min_anchors - 1;
        for disagreeing in [0, 10, 100, 10_000] {
            context.mapping_disagreeing_anchors = disagreeing;
            assert_ne!(
                decide(&score, &context, &config).status,
                BookmarkStatus::AutoConfirmed,
                "{disagreeing} disagreeing anchors must not fill an exact-anchor shortfall"
            );
        }
    }

    #[test]
    fn enough_independent_exact_anchors_do_compensate() {
        let (score, mut context) = near_miss();
        let config = AutoBookmarkConfig::default();
        context.mapping_exact_independent_anchors = config.consensus_min_anchors;
        context.mapping_disagreeing_anchors = 3;
        let decision = decide(&score, &context, &config);
        assert_eq!(decision.status, BookmarkStatus::AutoConfirmed);
        assert!(decision.reason_codes.iter().any(|code| code
            == &format!(
                "mapping_exact_independent_anchors_{}",
                config.consensus_min_anchors
            )));
        assert!(decision
            .reason_codes
            .iter()
            .any(|code| code == "mapping_disagreeing_anchors_3"));
    }

    #[test]
    fn safety_conditions_are_never_traded_away() {
        let config = AutoBookmarkConfig::default();
        // Each of these decides whether the bookmark lands on the right page.
        type SafetyMutation = (&'static str, fn(&mut GateContext));
        let mutations: Vec<SafetyMutation> = vec![
            ("no body heading", |c| c.has_body_evidence = false),
            ("repeated furniture", |c| c.repeated_furniture = true),
            ("ambiguous level", |c| c.level_ambiguous = true),
            ("secondary key only", |c| c.secondary_only = true),
            ("non-monotonic", |c| c.monotone = false),
            ("approximate geometry", |c| {
                c.approximate_multi_column = true
            }),
            ("truncated", |c| c.truncated = true),
            ("tiny margin", |c| c.runner_up_margin = 0),
            ("unmapped page", |c| c.printed_page_residual = None),
        ];
        for (name, mutate) in mutations {
            let (score, mut context) = near_miss();
            mutate(&mut context);
            assert_ne!(
                decide(&score, &context, &config).status,
                BookmarkStatus::AutoConfirmed,
                "{name} must block automatic confirmation"
            );
        }
    }

    #[test]
    fn a_weak_title_is_not_rescued_by_consensus() {
        let (_, context) = near_miss();
        let config = AutoBookmarkConfig::default();
        let weak = breakdown(1_000, 2_000, 1_000, 550, 200, 1_000);
        assert_ne!(
            decide(&weak, &context, &config).status,
            BookmarkStatus::AutoConfirmed
        );
    }
}
