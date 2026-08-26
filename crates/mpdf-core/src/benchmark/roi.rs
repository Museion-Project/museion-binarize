//! Region-of-interest (ROI) definitions.
//!
//! A page-level fidelity score can hide a real failure: a handful of
//! missing diacritic pixels are statistically invisible against a whole
//! page of body text. ROIs let a dataset tag a small region (e.g. "the
//! breathing mark on this page") and get the exact same metrics computed
//! over just that crop, reported separately from the whole-page score —
//! see `docs/benchmark-metrics.md`, "Why ROIs."

use serde::{Deserialize, Serialize};

use crate::bilevel::BinaryMask;
use crate::error::{CoreError, Result};

/// One rectangular region of interest, in pixel coordinates of the page
/// it is declared on (same coordinate system as the input/ground-truth
/// raster, top-left origin — matching [`BinaryMask`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roi {
    pub id: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Free-text tag (e.g. `"diacritic"`, `"small_text"`) used for
    /// category-style aggregation across ROIs, separate from the page's
    /// own `category`.
    #[serde(default)]
    pub tag: Option<String>,
}

impl Roi {
    /// Validates that this ROI is non-degenerate and fits entirely
    /// within an image of the given dimensions. Never clamps or crops
    /// silently — an out-of-bounds ROI is a structured error, since a
    /// silently-adjusted region would no longer be the region the
    /// dataset author actually meant to measure.
    pub fn validate(&self, image_width: u32, image_height: u32) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(CoreError::InvalidRoi(format!(
                "ROI '{}' has zero width or height",
                self.id
            )));
        }
        let right = self.x.checked_add(self.width).ok_or_else(|| {
            CoreError::InvalidRoi(format!("ROI '{}' x+width overflowed", self.id))
        })?;
        let bottom = self.y.checked_add(self.height).ok_or_else(|| {
            CoreError::InvalidRoi(format!("ROI '{}' y+height overflowed", self.id))
        })?;
        if right > image_width || bottom > image_height {
            return Err(CoreError::InvalidRoi(format!(
                "ROI '{}' ({},{} {}x{}) exceeds the page bounds ({}x{})",
                self.id, self.x, self.y, self.width, self.height, image_width, image_height
            )));
        }
        Ok(())
    }

    /// Crops `mask` to exactly this ROI. The caller must have already
    /// validated the ROI against `mask`'s dimensions via [`Self::validate`];
    /// this function trusts that and will panic on out-of-bounds access
    /// otherwise, since by the time cropping runs the ROI is assumed
    /// already-checked internal state, not fresh untrusted input.
    pub fn crop(&self, mask: &BinaryMask) -> BinaryMask {
        let mut cropped = BinaryMask::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                cropped.set(x, y, mask.get(self.x + x, self.y + y));
            }
        }
        cropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roi(x: u32, y: u32, width: u32, height: u32) -> Roi {
        Roi {
            id: "test".to_string(),
            x,
            y,
            width,
            height,
            tag: None,
        }
    }

    #[test]
    fn validate_accepts_a_roi_exactly_at_the_edge() {
        assert!(roi(90, 90, 10, 10).validate(100, 100).is_ok());
    }

    #[test]
    fn validate_rejects_a_roi_that_overflows_the_page() {
        assert!(roi(95, 95, 10, 10).validate(100, 100).is_err());
    }

    #[test]
    fn validate_rejects_zero_sized_roi() {
        assert!(roi(0, 0, 0, 5).validate(100, 100).is_err());
        assert!(roi(0, 0, 5, 0).validate(100, 100).is_err());
    }

    #[test]
    fn validate_rejects_coordinate_overflow() {
        assert!(roi(u32::MAX, 0, 10, 10).validate(100, 100).is_err());
    }

    #[test]
    fn crop_extracts_exactly_the_requested_pixels() {
        let mut mask = BinaryMask::new(4, 4);
        mask.set(2, 1, true);
        let region = roi(1, 1, 2, 2);
        let cropped = region.crop(&mask);
        assert_eq!((cropped.width, cropped.height), (2, 2));
        assert!(cropped.get(1, 0)); // (2,1) in original -> (1,0) in crop
        assert!(!cropped.get(0, 0));
    }
}
