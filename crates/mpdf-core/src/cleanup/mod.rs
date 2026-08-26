//! Optional post-binarization cleanup.
//!
//! Phase 1 ships exactly one cleanup operation: despeckling (removal of
//! small isolated black connected components). It defaults to **off**.
//! See [`despeckle`] for the rationale and the risk to small punctuation
//! and diacritics.

pub mod despeckle;

pub use despeckle::despeckle;

/// How aggressively to despeckle after binarization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DespeckleLevel {
    /// No despeckling. This is the default: small punctuation, diacritics,
    /// and apostrophes are visually indistinguishable from noise at the
    /// pixel level, so removing anything automatically is a real risk to
    /// scholarly text, not just a cosmetic tradeoff.
    #[default]
    Off,
    /// Removes only very small components (a handful of pixels) — aimed at
    /// sensor dust, not typography.
    Conservative,
    /// Removes a wider range of small components. May remove some
    /// diacritics, dots, or thin serifs; only recommended for known-noisy
    /// scans where legibility of small marks is not the priority.
    Strong,
}

/// Settings controlling the despeckle cleanup step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CleanupSettings {
    pub despeckle: DespeckleLevel,
    /// Overrides the connected-component pixel-count limit implied by
    /// `despeckle`. Exposed in Advanced Settings; `None` uses the level's
    /// built-in default.
    pub max_component_pixels: Option<u32>,
}

impl Default for CleanupSettings {
    fn default() -> Self {
        Self {
            despeckle: DespeckleLevel::Off,
            max_component_pixels: None,
        }
    }
}
