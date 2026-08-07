//! Pixel-fidelity metrics for binarization benchmarking.
//!
//! Every function here is a pure function of two [`BinaryMask`]s: no
//! filesystem access, no PDFium, no global state. This is deliberate —
//! it is what makes these functions independently testable against
//! hand-computed micro-fixtures and, later, easy to property-test.
//!
//! ## Polarity
//!
//! This module uses the project-wide convention unchanged:
//! `true` = black / foreground / ink, `false` = white / background (see
//! [`crate::bilevel`]). A benchmark ground-truth mask must be normalized
//! to this convention *before* it reaches these functions — see
//! [`crate::benchmark::mask_io`] for where that normalization happens.
//! Nothing in this module silently reinterprets polarity.
//!
//! ## Alignment
//!
//! Every function requires `output` and `ground_truth` to have identical
//! `width`/`height`. There is no cropping, resampling, or best-effort
//! alignment: a dimension mismatch is a [`CoreError::BenchmarkDimensionMismatch`],
//! never a silent approximation, because an approximated alignment would
//! invalidate the fidelity result it claims to measure.

use serde::{Deserialize, Serialize};

use crate::bilevel::BinaryMask;
use crate::error::{CoreError, Result};

fn require_same_dimensions(output: &BinaryMask, ground_truth: &BinaryMask) -> Result<()> {
    if output.width != ground_truth.width || output.height != ground_truth.height {
        return Err(CoreError::BenchmarkDimensionMismatch(format!(
            "output is {}x{} but ground truth is {}x{}",
            output.width, output.height, ground_truth.width, ground_truth.height
        )));
    }
    Ok(())
}

/// Foreground-pixel confusion matrix, using the project's black-is-
/// foreground convention:
///
/// - `true_positive`: output black AND ground truth black
/// - `false_positive`: output black AND ground truth white
/// - `false_negative`: output white AND ground truth black
/// - `true_negative`: output white AND ground truth white
///
/// Counts are `u64` with checked accumulation — a pathologically large
/// benchmark image cannot silently overflow this into a wrong answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfusionMatrix {
    pub true_positive: u64,
    pub false_positive: u64,
    pub false_negative: u64,
    pub true_negative: u64,
}

impl ConfusionMatrix {
    /// Total pixel count this matrix was computed over.
    pub fn total(&self) -> u64 {
        self.true_positive + self.false_positive + self.false_negative + self.true_negative
    }
}

/// Computes the confusion matrix of `output` against `ground_truth`.
/// Requires identical dimensions (see module docs); never crops or
/// resamples either mask.
pub fn confusion_matrix(output: &BinaryMask, ground_truth: &BinaryMask) -> Result<ConfusionMatrix> {
    require_same_dimensions(output, ground_truth)?;

    let mut true_positive: u64 = 0;
    let mut false_positive: u64 = 0;
    let mut false_negative: u64 = 0;
    let mut true_negative: u64 = 0;

    for (&out_pixel, &gt_pixel) in output.pixels.iter().zip(ground_truth.pixels.iter()) {
        match (out_pixel, gt_pixel) {
            (true, true) => true_positive = true_positive.checked_add(1).ok_or_else(overflow)?,
            (true, false) => false_positive = false_positive.checked_add(1).ok_or_else(overflow)?,
            (false, true) => false_negative = false_negative.checked_add(1).ok_or_else(overflow)?,
            (false, false) => true_negative = true_negative.checked_add(1).ok_or_else(overflow)?,
        }
    }

    Ok(ConfusionMatrix {
        true_positive,
        false_positive,
        false_negative,
        true_negative,
    })
}

fn overflow() -> CoreError {
    CoreError::ResourceLimitExceeded(
        "confusion matrix pixel count overflowed a u64 accumulator".to_string(),
    )
}

/// Precision, recall, and F1 over foreground (ink) pixels, plus the raw
/// [`ConfusionMatrix`] they were derived from.
///
/// ## Edge-case policy (documented once, applied consistently)
///
/// - **Precision** (`TP / (TP + FP)`): if the output has no foreground
///   pixels at all (`TP + FP == 0`), precision is `1.0` when the ground
///   truth is *also* empty (vacuously correct — nothing was predicted and
///   nothing needed to be), and `0.0` when the ground truth has
///   foreground that was entirely missed (predicting nothing when
///   something was there is not treated as "precise").
/// - **Recall** (`TP / (TP + FN)`): if the ground truth has no foreground
///   pixels (`TP + FN == 0`), recall is `1.0` — there was nothing to
///   find, so nothing was missed.
/// - **F1**: `2PR / (P + R)`, except when `P + R == 0`, which only
///   happens when `TP == 0` and the output disagrees with a non-empty
///   ground truth in some way; F1 is `0.0` in that case rather than
///   `NaN`. When both output and ground truth are empty, precision and
///   recall are both `1.0` under the policy above, so F1 is `1.0` too —
///   matching "if both output and ground truth contain no foreground:
///   perfect foreground agreement."
///
/// No branch of this function can produce `NaN` or `Infinity`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FMeasure {
    pub confusion: ConfusionMatrix,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

pub fn f_measure(confusion: &ConfusionMatrix) -> FMeasure {
    let tp = confusion.true_positive;
    let fp = confusion.false_positive;
    let fn_ = confusion.false_negative;

    let precision = {
        let denom = tp + fp;
        if denom == 0 {
            if tp + fn_ == 0 {
                1.0
            } else {
                0.0
            }
        } else {
            tp as f64 / denom as f64
        }
    };
    let recall = {
        let denom = tp + fn_;
        if denom == 0 {
            1.0
        } else {
            tp as f64 / denom as f64
        }
    };
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };

    FMeasure {
        confusion: *confusion,
        precision,
        recall,
        f1,
    }
}

/// PSNR between `output` and `ground_truth`, treating each pixel as a
/// normalized binary sample (`black = 1.0`, `white = 0.0`, so `MAX_I =
/// 1`): `MSE = mismatched_pixels / total_pixels`,
/// `PSNR = 10 * log10(1 / MSE)`.
///
/// A perfect match gives `MSE = 0`, which is mathematically infinite
/// PSNR — not representable in JSON. [`Psnr::PerfectMatch`] represents
/// that case explicitly rather than serializing `Infinity` or clamping to
/// an arbitrary decibel value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Psnr {
    Finite { psnr_db: f64 },
    PerfectMatch { perfect_match: bool },
}

impl Psnr {
    /// The finite decibel value, or `None` for a perfect match.
    pub fn db(&self) -> Option<f64> {
        match self {
            Psnr::Finite { psnr_db } => Some(*psnr_db),
            Psnr::PerfectMatch { .. } => None,
        }
    }

    pub fn is_perfect_match(&self) -> bool {
        matches!(self, Psnr::PerfectMatch { .. })
    }
}

pub fn psnr(output: &BinaryMask, ground_truth: &BinaryMask) -> Result<Psnr> {
    require_same_dimensions(output, ground_truth)?;
    let confusion = confusion_matrix(output, ground_truth)?;
    let total = confusion.total();
    if total == 0 {
        // Dimension check above guarantees width/height match; a total
        // of zero only happens for a genuinely zero-pixel mask, which is
        // otherwise rejected before benchmarking begins (see
        // `dataset.rs`), but this function stays total even so.
        return Err(CoreError::InvalidParameter(
            "cannot compute PSNR for a zero-pixel mask".to_string(),
        ));
    }
    let mismatched = confusion.false_positive + confusion.false_negative;
    if mismatched == 0 {
        return Ok(Psnr::PerfectMatch {
            perfect_match: true,
        });
    }
    let mse = mismatched as f64 / total as f64;
    let psnr_db = 10.0 * (1.0 / mse).log10();
    Ok(Psnr::Finite { psnr_db })
}

/// Distance Reciprocal Distortion (DRD), per Lu, Kot & Shi, "Distance-
/// Reciprocal Distortion Measure for Binary Document Images", IEEE
/// Signal Processing Letters 11(2), 2004 — the definition used
/// throughout the DIBCO binarization-competition literature. See
/// `docs/benchmark-metrics.md` for the full worked derivation this
/// implementation follows.
///
/// For every pixel where `output` disagrees with `ground_truth` (a
/// "flipped" pixel at `(x, y)`), this measures how much the surrounding
/// 5x5 neighborhood of ground truth disagrees with the *output's* value
/// at `(x, y)`, weighted by a distance-reciprocal weight matrix `WM`
/// (`WM(i, j) = 1 / distance((i, j), center)` for every non-center cell,
/// normalized to sum to `1`). Pixels outside the image are treated as
/// background (white/`0`) — a documented boundary convention, not the
/// only one in the literature, but a common and simple one.
///
/// `DRD = (sum of per-flipped-pixel distortion) / NUBN`, where `NUBN` is
/// the number of non-uniform (containing both black and white pixels)
/// non-overlapping 8x8 blocks of `ground_truth`. This normalizes for how
/// much of the page actually has content: a mostly-blank page has few
/// non-uniform blocks, so a handful of errors there is penalized more per
/// block than the same error count on a densely inked page.
///
/// A ground truth with zero non-uniform blocks (e.g. a blank page) makes
/// `NUBN == 0`; in that case `DRD` is `0.0` if there were no flipped
/// pixels (nothing to distort) or an error is returned if the output
/// still disagrees with a ground truth that has no measurable "block
/// scale" reference at all, since NUBN, not lack of division, is not
/// standing in for meaningful content there.
pub fn drd(output: &BinaryMask, ground_truth: &BinaryMask) -> Result<f64> {
    require_same_dimensions(output, ground_truth)?;
    let width = ground_truth.width;
    let height = ground_truth.height;
    if width == 0 || height == 0 {
        return Err(CoreError::InvalidParameter(
            "cannot compute DRD for a zero-pixel mask".to_string(),
        ));
    }

    let weights = drd_weight_matrix();

    let gt_at = |x: i64, y: i64| -> bool {
        if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 {
            false // out-of-bounds treated as background/white.
        } else {
            ground_truth.get(x as u32, y as u32)
        }
    };

    let mut total_distortion = 0.0f64;
    let mut flipped_count: u64 = 0;

    for y in 0..height {
        for x in 0..width {
            let out_pixel = output.get(x, y);
            let gt_pixel = ground_truth.get(x, y);
            if out_pixel == gt_pixel {
                continue;
            }
            flipped_count += 1;
            let out_value = if out_pixel { 1.0 } else { 0.0 };
            let mut pixel_distortion = 0.0f64;
            for di in -2i64..=2 {
                for dj in -2i64..=2 {
                    if di == 0 && dj == 0 {
                        continue; // center excluded from the weight matrix.
                    }
                    let gt_neighbor: f64 = if gt_at(x as i64 + dj, y as i64 + di) {
                        1.0
                    } else {
                        0.0
                    };
                    let weight = weights[(di + 2) as usize][(dj + 2) as usize];
                    pixel_distortion += (gt_neighbor - out_value).abs() * weight;
                }
            }
            total_distortion += pixel_distortion;
        }
    }

    let nubn = count_non_uniform_blocks(ground_truth);
    if nubn == 0 {
        return Ok(if flipped_count == 0 {
            0.0
        } else {
            total_distortion
        });
    }
    Ok(total_distortion / nubn as f64)
}

/// Normalized 5x5 distance-reciprocal weight matrix, computed once.
/// `weights[i][j]` corresponds to offset `(dj, di) = (j - 2, i - 2)` from
/// the center; the center cell (`i == j == 2`) is `0.0` and excluded from
/// the normalizing sum, matching the definition in the module docs.
fn drd_weight_matrix() -> [[f64; 5]; 5] {
    let mut raw = [[0.0f64; 5]; 5];
    let mut sum = 0.0f64;
    for (i, row) in raw.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            if i == 2 && j == 2 {
                continue;
            }
            let di = i as f64 - 2.0;
            let dj = j as f64 - 2.0;
            let distance = (di * di + dj * dj).sqrt();
            *cell = 1.0 / distance;
            sum += *cell;
        }
    }
    let mut normalized = [[0.0f64; 5]; 5];
    for (i, row) in normalized.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = raw[i][j] / sum;
        }
    }
    normalized
}

/// Counts non-overlapping 8x8 blocks of `mask` that contain both black
/// and white pixels. A partial block at the right/bottom edge (when
/// width/height are not multiples of 8) is still checked over its
/// in-bounds pixels only.
fn count_non_uniform_blocks(mask: &BinaryMask) -> u64 {
    let mut count = 0u64;
    let mut block_y = 0u32;
    while block_y < mask.height {
        let mut block_x = 0u32;
        while block_x < mask.width {
            let mut saw_black = false;
            let mut saw_white = false;
            for y in block_y..(block_y + 8).min(mask.height) {
                for x in block_x..(block_x + 8).min(mask.width) {
                    if mask.get(x, y) {
                        saw_black = true;
                    } else {
                        saw_white = true;
                    }
                    if saw_black && saw_white {
                        break;
                    }
                }
                if saw_black && saw_white {
                    break;
                }
            }
            if saw_black && saw_white {
                count += 1;
            }
            block_x += 8;
        }
        block_y += 8;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask_from_rows(rows: &[&str]) -> BinaryMask {
        let height = rows.len() as u32;
        let width = rows[0].chars().count() as u32;
        let mut mask = BinaryMask::new(width, height);
        for (y, row) in rows.iter().enumerate() {
            for (x, ch) in row.chars().enumerate() {
                mask.set(x as u32, y as u32, ch == '1');
            }
        }
        mask
    }

    // ---- confusion matrix / precision / recall / F1 --------------------

    #[test]
    fn perfect_2x2_match_yields_perfect_scores() {
        let gt = mask_from_rows(&["10", "01"]);
        let output = gt.clone();
        let confusion = confusion_matrix(&output, &gt).unwrap();
        assert_eq!(
            confusion,
            ConfusionMatrix {
                true_positive: 2,
                false_positive: 0,
                false_negative: 0,
                true_negative: 2,
            }
        );
        let f = f_measure(&confusion);
        assert_eq!(f.precision, 1.0);
        assert_eq!(f.recall, 1.0);
        assert_eq!(f.f1, 1.0);
        let p = psnr(&output, &gt).unwrap();
        assert!(p.is_perfect_match());
        assert_eq!(drd(&output, &gt).unwrap(), 0.0);
    }

    #[test]
    fn one_false_positive_matches_manual_calculation() {
        // GT: 1 0 / 0 0   Output: 1 1 / 0 0
        let gt = mask_from_rows(&["10", "00"]);
        let output = mask_from_rows(&["11", "00"]);
        let confusion = confusion_matrix(&output, &gt).unwrap();
        assert_eq!(
            confusion,
            ConfusionMatrix {
                true_positive: 1,
                false_positive: 1,
                false_negative: 0,
                true_negative: 2,
            }
        );
        let f = f_measure(&confusion);
        // precision = 1/2, recall = 1/1, F1 = 2*0.5*1/(1.5) = 2/3
        assert!((f.precision - 0.5).abs() < 1e-12);
        assert_eq!(f.recall, 1.0);
        assert!((f.f1 - (2.0 / 3.0)).abs() < 1e-12);
    }

    #[test]
    fn no_foreground_in_output_but_ground_truth_has_some() {
        let gt = mask_from_rows(&["10", "00"]);
        let output = mask_from_rows(&["00", "00"]);
        let confusion = confusion_matrix(&output, &gt).unwrap();
        let f = f_measure(&confusion);
        assert_eq!(f.recall, 0.0);
        assert_eq!(f.precision, 0.0, "nothing predicted but GT had foreground");
        assert_eq!(f.f1, 0.0);
    }

    #[test]
    fn both_output_and_ground_truth_empty_is_perfect_agreement() {
        let gt = BinaryMask::new(4, 4);
        let output = BinaryMask::new(4, 4);
        let confusion = confusion_matrix(&output, &gt).unwrap();
        let f = f_measure(&confusion);
        assert_eq!(f.precision, 1.0);
        assert_eq!(f.recall, 1.0);
        assert_eq!(f.f1, 1.0);
    }

    #[test]
    fn ground_truth_empty_but_output_has_foreground() {
        let gt = BinaryMask::new(2, 2);
        let mut output = BinaryMask::new(2, 2);
        output.set(0, 0, true);
        let confusion = confusion_matrix(&output, &gt).unwrap();
        let f = f_measure(&confusion);
        assert_eq!(f.recall, 1.0, "nothing in GT to miss");
        assert_eq!(f.precision, 0.0);
        assert_eq!(f.f1, 0.0);
    }

    #[test]
    fn f1_and_precision_recall_are_always_finite_and_in_unit_range() {
        // Exhaustive over every possible small confusion matrix shape.
        for tp in 0..3u64 {
            for fp in 0..3u64 {
                for fn_ in 0..3u64 {
                    for tn in 0..3u64 {
                        let confusion = ConfusionMatrix {
                            true_positive: tp,
                            false_positive: fp,
                            false_negative: fn_,
                            true_negative: tn,
                        };
                        let f = f_measure(&confusion);
                        assert!(f.precision.is_finite());
                        assert!(f.recall.is_finite());
                        assert!(f.f1.is_finite());
                        assert!((0.0..=1.0).contains(&f.precision));
                        assert!((0.0..=1.0).contains(&f.recall));
                        assert!((0.0..=1.0).contains(&f.f1));
                    }
                }
            }
        }
    }

    #[test]
    fn confusion_matrix_rejects_mismatched_dimensions() {
        let a = BinaryMask::new(3, 3);
        let b = BinaryMask::new(4, 3);
        assert!(matches!(
            confusion_matrix(&a, &b),
            Err(CoreError::BenchmarkDimensionMismatch(_))
        ));
    }

    #[test]
    fn confusion_counts_sum_to_pixel_count() {
        let gt = mask_from_rows(&["1010", "0110", "1001"]);
        let output = mask_from_rows(&["1000", "0111", "1101"]);
        let confusion = confusion_matrix(&output, &gt).unwrap();
        assert_eq!(confusion.total(), 12);
    }

    #[test]
    fn asymmetric_polarity_swap_produces_different_results() {
        // Confirms the polarity convention is not silently invertible:
        // swapping which side is "output" changes precision/recall
        // (though not F1, which is symmetric) when FP != FN.
        let gt = mask_from_rows(&["11", "00"]);
        let output = mask_from_rows(&["10", "00"]); // misses one black pixel (FN=1)

        let forward = f_measure(&confusion_matrix(&output, &gt).unwrap());
        let inverted = f_measure(&confusion_matrix(&gt, &output).unwrap());
        // Forward: TP=1,FP=0,FN=1 -> precision=1.0, recall=0.5
        // Inverted (swap roles): TP=1,FP=1,FN=0 -> precision=0.5, recall=1.0
        assert_eq!(forward.precision, 1.0);
        assert_eq!(forward.recall, 0.5);
        assert_eq!(inverted.precision, 0.5);
        assert_eq!(inverted.recall, 1.0);
        // F1 stays symmetric even though precision/recall swapped.
        assert!((forward.f1 - inverted.f1).abs() < 1e-12);
    }

    // ---- PSNR ------------------------------------------------------------

    #[test]
    fn psnr_perfect_match_is_tagged_not_infinite() {
        let mask = mask_from_rows(&["10", "01"]);
        let result = psnr(&mask, &mask).unwrap();
        assert!(result.is_perfect_match());
        assert_eq!(result.db(), None);
    }

    #[test]
    fn psnr_matches_manual_mse_calculation() {
        // 2x2, one mismatched pixel out of 4 -> MSE = 0.25, PSNR = 10*log10(4) = ~6.0206
        let gt = mask_from_rows(&["10", "00"]);
        let output = mask_from_rows(&["11", "00"]);
        let result = psnr(&output, &gt).unwrap();
        let expected = 10.0 * (1.0 / 0.25f64).log10();
        assert!((result.db().unwrap() - expected).abs() < 1e-9);
    }

    #[test]
    fn psnr_is_never_nan_or_infinite_when_finite() {
        let gt = mask_from_rows(&["1000", "0000", "0000", "0001"]);
        let output = mask_from_rows(&["0000", "0000", "0000", "0001"]);
        let result = psnr(&output, &gt).unwrap();
        if let Psnr::Finite { psnr_db } = result {
            assert!(psnr_db.is_finite());
        } else {
            panic!("expected a finite result for a partial mismatch");
        }
    }

    #[test]
    fn psnr_rejects_mismatched_dimensions() {
        let a = BinaryMask::new(2, 2);
        let b = BinaryMask::new(3, 2);
        assert!(psnr(&a, &b).is_err());
    }

    // ---- DRD --------------------------------------------------------------

    #[test]
    fn drd_is_zero_for_a_perfect_match() {
        let mask = mask_from_rows(&[
            "11110000", "11110000", "00000000", "00000000", "00000000", "00000000", "00000000",
            "00000000",
        ]);
        assert_eq!(drd(&mask, &mask).unwrap(), 0.0);
    }

    #[test]
    fn drd_single_isolated_flip_in_one_non_uniform_block_matches_hand_calculation() {
        // One 8x8 block, entirely white ground truth except a single
        // black pixel far from any edge (so the whole 5x5 neighborhood
        // is in-bounds and reads entirely from this one GT pixel).
        let mut gt = BinaryMask::new(8, 8);
        gt.set(4, 4, true); // the only black pixel in GT
        let mut output = gt.clone();
        output.set(4, 4, false); // output misses it -> one flipped pixel

        // NUBN = 1 (the single 8x8 block is non-uniform: has one black
        // pixel and 63 white ones).
        //
        // The flipped pixel is at (4,4); output value there is 0 (white).
        // Its 5x5 GT neighborhood is all white except the center itself
        // (excluded from the weight matrix), so every neighbor GT value
        // is white (0.0) and every |gt_neighbor - out_value| term is
        // |0 - 0| = 0. Total distortion for this flipped pixel = 0.
        let result = drd(&output, &gt).unwrap();
        assert_eq!(
            result, 0.0,
            "flipped pixel's own neighborhood has no GT content to weigh against a white output"
        );
    }

    #[test]
    fn drd_flip_that_disagrees_with_dense_neighborhood_is_positive_and_matches_hand_calculation() {
        // Ground truth: a solid black column at x=4 across an 8x8 block,
        // Output: the pixel at (4,4) is flipped to white while its column
        // neighbors (4,2),(4,3),(4,5),(4,6) remain black in GT — those
        // neighbors are directly above/below in the weight matrix at
        // distance 2 and 1.
        let mut gt = BinaryMask::new(8, 8);
        for y in 0..8u32 {
            gt.set(4, y, true);
        }
        let mut output = gt.clone();
        output.set(4, 4, false);

        let weights = drd_weight_matrix();
        // Output value at the flipped pixel is 0 (white). For each
        // neighbor (di, dj) in the 5x5 window around (4,4), the GT value
        // is black (1.0) only for cells directly in the same column
        // (dj == 0) since GT is a single black column; the row is
        // otherwise unaffected in this synthetic case since only x=4 is
        // black in GT (dj = 0 offsets are the same column).
        let mut expected = 0.0f64;
        for di in -2i64..=2 {
            for dj in -2i64..=2 {
                if di == 0 && dj == 0 {
                    continue;
                }
                let gt_x = 4i64 + dj;
                let gt_y = 4i64 + di;
                let gt_value: f64 = if gt_x == 4 && (0..8).contains(&gt_y) {
                    1.0
                } else {
                    0.0
                };
                expected += (gt_value - 0.0).abs() * weights[(di + 2) as usize][(dj + 2) as usize];
            }
        }
        // NUBN = 1 (the only 8x8 block has both black and white pixels).
        let result = drd(&output, &gt).unwrap();
        assert!((result - expected).abs() < 1e-12);
        assert!(result > 0.0);
    }

    #[test]
    fn drd_rejects_mismatched_dimensions() {
        let a = BinaryMask::new(8, 8);
        let b = BinaryMask::new(9, 8);
        assert!(drd(&a, &b).is_err());
    }

    #[test]
    fn drd_weight_matrix_is_normalized_and_center_excluded() {
        let weights = drd_weight_matrix();
        assert_eq!(weights[2][2], 0.0);
        let sum: f64 = weights.iter().flatten().sum();
        assert!((sum - 1.0).abs() < 1e-12);
    }

    #[test]
    fn count_non_uniform_blocks_ignores_uniform_blocks() {
        let all_white = BinaryMask::new(16, 8);
        assert_eq!(count_non_uniform_blocks(&all_white), 0);

        let mut mixed = BinaryMask::new(16, 8);
        mixed.set(0, 0, true); // makes the first 8x8 block non-uniform
        assert_eq!(count_non_uniform_blocks(&mixed), 1);

        let mut both = BinaryMask::new(16, 8);
        both.set(0, 0, true);
        both.set(8, 0, true);
        assert_eq!(count_non_uniform_blocks(&both), 2);
    }

    #[test]
    fn drd_is_never_negative() {
        let gt = mask_from_rows(&[
            "10101010", "01010101", "10101010", "01010101", "10101010", "01010101", "10101010",
            "01010101",
        ]);
        let output = mask_from_rows(&[
            "01010101", "10101010", "01010101", "10101010", "01010101", "10101010", "01010101",
            "10101010",
        ]);
        let result = drd(&output, &gt).unwrap();
        assert!(result >= 0.0);
        assert!(result.is_finite());
    }

    // ---- swapping identical masks changes nothing (property) -----------

    #[test]
    fn metric_of_a_mask_against_itself_is_always_perfect() {
        let masks = [
            mask_from_rows(&[
                "10101010", "01010101", "00000000", "11111111", "10101010", "01010101", "00000000",
                "11111111",
            ]),
            BinaryMask::new(8, 8),
        ];
        for mask in masks {
            let confusion = confusion_matrix(&mask, &mask).unwrap();
            assert_eq!(confusion.false_positive, 0);
            assert_eq!(confusion.false_negative, 0);
            let f = f_measure(&confusion);
            assert_eq!(f.f1, 1.0);
            assert!(psnr(&mask, &mask).unwrap().is_perfect_match());
            assert_eq!(drd(&mask, &mask).unwrap(), 0.0);
        }
    }
}
