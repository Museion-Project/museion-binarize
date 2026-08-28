//! Frozen rule constants for automatic table-of-contents compilation.
//!
//! Every threshold, cap, and weight the deterministic engine consults lives
//! here so a generated snapshot can bind itself to an auditable
//! `rule_config_digest`. Values are integers: bookmark scoring must not
//! depend on floating-point comparison order or platform rounding.
//!
//! These defaults are a conservative first baseline, not a corpus-tuned
//! optimum. Changing one changes [`AutoBookmarkConfig::rule_version`] and
//! therefore every generation digest, which is exactly the intent: a
//! recalibration must invalidate old automatic decisions instead of
//! silently reinterpreting them.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The rule set identifier written into every 0.2 snapshot and report.
pub const RULE_VERSION: &str = "0.2";

/// Score component ceilings. The six components sum to exactly 10,000.
pub const SCORE_TOTAL: u32 = 10_000;
pub const SCORE_TITLE_MAX: u32 = 4_000;
pub const SCORE_PAGE_MAX: u32 = 2_000;
pub const SCORE_NUMBERING_MAX: u32 = 1_000;
pub const SCORE_LAYOUT_MAX: u32 = 1_000;
pub const SCORE_OCR_MAX: u32 = 1_000;
pub const SCORE_SEQUENCE_MAX: u32 = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoBookmarkConfig {
    pub rule_version: String,
    /// Front-matter scan window: `min(pages, min(40, max(8, ceil(pages * 15%))))`.
    pub front_page_absolute_max: u32,
    pub front_page_minimum: u32,
    pub front_page_percent: u32,
    /// A page needs at least this multi-signal score to be a TOC page.
    pub toc_page_min_score: u32,
    /// Without a contents keyword a page needs at least this many parsable
    /// entries plus leader/right-edge agreement.
    pub toc_min_entries_without_keyword: u32,
    /// A contents keyword alone scores this much and cannot pass the gate.
    pub toc_keyword_score: u32,
    pub toc_entry_score: u32,
    pub toc_entry_score_cap: u32,
    pub toc_leader_score: u32,
    pub toc_right_cluster_score: u32,
    pub toc_short_line_score: u32,
    pub toc_adjacent_page_score: u32,
    pub max_toc_entries: u32,
    pub max_shortlist: u32,
    pub max_continuation_lines: u32,
    pub max_auto_level: u16,
    pub max_title_bytes: u32,
    /// Repeated header/footer detection: at least N pages and P% coverage
    /// inside the top/bottom band.
    pub furniture_min_pages: u32,
    pub furniture_percent: u32,
    pub furniture_band_percent: u32,
    /// A body heading in the top fraction of the page scores the layout bonus.
    pub body_top_percent: u32,
    /// Automatic gate.
    pub auto_confirm_total: u32,
    pub auto_confirm_title: u32,
    pub auto_confirm_margin: u32,
    /// Minimum per-word confidence in permille (800 == 0.80).
    pub auto_confirm_min_word_confidence_permille: u32,
    pub review_total: u32,
    /// Maximum tolerated |printed page - resolved physical page| difference.
    pub max_page_residual: u32,
    /// Mapping DP: cost of opening a new printed-page offset segment and of
    /// an anchor disagreeing with its segment offset. The change penalty sits
    /// between one and two mismatches, so a single stray anchor never forks
    /// the mapping while a genuine plate or gap does.
    pub segment_change_penalty: u32,
    pub anchor_mismatch_penalty: u32,
    /// Minimum title score for a shortlist entry to act as a mapping anchor.
    pub anchor_min_title_score: u32,
}

impl Default for AutoBookmarkConfig {
    fn default() -> Self {
        Self {
            rule_version: RULE_VERSION.to_owned(),
            front_page_absolute_max: 40,
            front_page_minimum: 8,
            front_page_percent: 15,
            toc_page_min_score: 400,
            toc_min_entries_without_keyword: 4,
            toc_keyword_score: 200,
            toc_entry_score: 40,
            toc_entry_score_cap: 400,
            toc_leader_score: 150,
            toc_right_cluster_score: 150,
            toc_short_line_score: 100,
            toc_adjacent_page_score: 100,
            max_toc_entries: 10_000,
            max_shortlist: 32,
            max_continuation_lines: 3,
            max_auto_level: 8,
            max_title_bytes: 4_096,
            furniture_min_pages: 3,
            furniture_percent: 60,
            furniture_band_percent: 12,
            body_top_percent: 45,
            auto_confirm_total: 9_200,
            auto_confirm_title: 3_600,
            auto_confirm_margin: 600,
            auto_confirm_min_word_confidence_permille: 800,
            review_total: 7_500,
            max_page_residual: 1,
            segment_change_penalty: 600,
            anchor_mismatch_penalty: 400,
            anchor_min_title_score: 3_000,
        }
    }
}

impl AutoBookmarkConfig {
    /// The digest bound into snapshots and reports. Serialization is over
    /// the whole frozen struct, so adding a field or changing a default
    /// changes the digest and therefore invalidates stale automatic
    /// decisions rather than silently reinterpreting them.
    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("bookmark rule config is serializable");
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Front-matter page window actually scanned for a printed contents list.
    pub fn front_page_limit(&self, page_count: u32) -> u32 {
        let fraction = (u64::from(page_count) * u64::from(self.front_page_percent)).div_ceil(100);
        let fraction = u32::try_from(fraction).unwrap_or(u32::MAX);
        page_count.min(
            self.front_page_absolute_max
                .min(self.front_page_minimum.max(fraction)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_ceilings_sum_to_the_total_scale() {
        assert_eq!(
            SCORE_TITLE_MAX
                + SCORE_PAGE_MAX
                + SCORE_NUMBERING_MAX
                + SCORE_LAYOUT_MAX
                + SCORE_OCR_MAX
                + SCORE_SEQUENCE_MAX,
            SCORE_TOTAL
        );
    }

    #[test]
    fn frozen_defaults_are_locked_by_digest() {
        let config = AutoBookmarkConfig::default();
        // Locking the concrete thresholds prevents a future change from
        // quietly loosening the automatic gate: a deliberate recalibration
        // must update this test and the rule version together.
        assert_eq!(config.rule_version, "0.2");
        assert_eq!(config.auto_confirm_total, 9_200);
        assert_eq!(config.auto_confirm_title, 3_600);
        assert_eq!(config.auto_confirm_margin, 600);
        assert_eq!(config.auto_confirm_min_word_confidence_permille, 800);
        assert_eq!(config.review_total, 7_500);
        assert_eq!(config.max_page_residual, 1);
        assert_eq!(config.max_auto_level, 8);
        assert_eq!(config.max_toc_entries, 10_000);
        assert_eq!(config.max_shortlist, 32);
        assert_eq!(config.segment_change_penalty, 600);
        assert_eq!(config.anchor_mismatch_penalty, 400);
        assert!(
            config.segment_change_penalty > config.anchor_mismatch_penalty
                && config.segment_change_penalty < config.anchor_mismatch_penalty * 2,
            "one stray anchor must not fork the mapping; two must be able to"
        );
        assert_eq!(config.digest(), AutoBookmarkConfig::default().digest());
        assert_eq!(config.digest().len(), 64);
    }

    #[test]
    fn front_page_window_is_bounded_at_both_ends() {
        let config = AutoBookmarkConfig::default();
        assert_eq!(config.front_page_limit(3), 3);
        assert_eq!(config.front_page_limit(20), 8);
        assert_eq!(config.front_page_limit(100), 15);
        assert_eq!(config.front_page_limit(10_000), 40);
    }
}
