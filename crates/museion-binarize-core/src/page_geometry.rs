//! Page geometry: physical page rectangles, rotation, and the conversion
//! from PDF points to rendered pixels.
//!
//! All of this module is pure arithmetic with no PDFium dependency, so it
//! is fully unit-testable without a PDFium runtime library.

use crate::error::{CoreError, Result};

/// Maximum number of pixels a single rendered page may occupy.
///
/// At 600 DPI a 200x200mm page is roughly 4724x4724 = 22.3 megapixels, so
/// this limit (400 megapixels, about 400 MB as 8-bit grayscale) leaves
/// generous headroom for oversized pages while still refusing to attempt
/// an allocation that would exhaust memory on a typical machine. Exceeding
/// it produces [`CoreError::ResourceLimitExceeded`] rather than an
/// out-of-memory abort.
pub const MAX_RENDERED_PIXELS: u64 = 400_000_000;

/// Largest page edge accepted, in PDF points. The PDF specification's own
/// limit for a user-space coordinate is 14400 points (200 inches) per
/// side; anything beyond that is treated as a malformed page rather than
/// silently rendered.
pub const MAX_PAGE_EDGE_POINTS: f32 = 14_400.0;

/// Page rotation, as recorded in a PDF page's `/Rotate` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageRotation {
    #[default]
    None,
    Degrees90,
    Degrees180,
    Degrees270,
}

impl PageRotation {
    /// Converts a raw `/Rotate` value into a [`PageRotation`].
    ///
    /// The PDF specification requires `/Rotate` to be a multiple of 90;
    /// values may be negative or exceed 360, and are normalized modulo
    /// 360 per those documented semantics. A value that is *not* a
    /// multiple of 90 is not silently reinterpreted — it is rejected.
    pub fn from_degrees(degrees: i32) -> Result<Self> {
        if degrees % 90 != 0 {
            return Err(CoreError::InvalidPageGeometry(format!(
                "page /Rotate must be a multiple of 90 degrees, got {degrees}"
            )));
        }
        let normalized = degrees.rem_euclid(360);
        Ok(match normalized {
            0 => PageRotation::None,
            90 => PageRotation::Degrees90,
            180 => PageRotation::Degrees180,
            270 => PageRotation::Degrees270,
            // Unreachable given the checks above, but handled explicitly
            // rather than with a panicking branch.
            other => {
                return Err(CoreError::InvalidPageGeometry(format!(
                    "unsupported normalized page rotation {other}"
                )))
            }
        })
    }

    pub fn degrees(self) -> u32 {
        match self {
            PageRotation::None => 0,
            PageRotation::Degrees90 => 90,
            PageRotation::Degrees180 => 180,
            PageRotation::Degrees270 => 270,
        }
    }

    /// Whether this rotation swaps the visible width and height.
    pub fn swaps_axes(self) -> bool {
        matches!(self, PageRotation::Degrees90 | PageRotation::Degrees270)
    }
}

/// The visible physical geometry of one page.
///
/// `width_points`/`height_points` are the dimensions of the selected
/// visible page box (CropBox when valid, else MediaBox) *before* rotation
/// is applied. [`PageGeometry::display_width_points`] and
/// [`PageGeometry::display_height_points`] apply the rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGeometry {
    pub width_points: f32,
    pub height_points: f32,
    pub rotation: PageRotation,
}

impl PageGeometry {
    pub fn new(width_points: f32, height_points: f32, rotation: PageRotation) -> Result<Self> {
        let geometry = Self {
            width_points,
            height_points,
            rotation,
        };
        geometry.validate()?;
        Ok(geometry)
    }

    /// Rejects zero, negative, non-finite, or absurdly large dimensions.
    pub fn validate(&self) -> Result<()> {
        for (label, value) in [("width", self.width_points), ("height", self.height_points)] {
            if !value.is_finite() {
                return Err(CoreError::InvalidPageGeometry(format!(
                    "page {label} is not a finite number"
                )));
            }
            if value <= 0.0 {
                return Err(CoreError::InvalidPageGeometry(format!(
                    "page {label} must be positive, got {value}"
                )));
            }
            if value > MAX_PAGE_EDGE_POINTS {
                return Err(CoreError::InvalidPageGeometry(format!(
                    "page {label} of {value} points exceeds the {MAX_PAGE_EDGE_POINTS} point limit"
                )));
            }
        }
        Ok(())
    }

    /// Visible width in points, after rotation.
    pub fn display_width_points(&self) -> f32 {
        if self.rotation.swaps_axes() {
            self.height_points
        } else {
            self.width_points
        }
    }

    /// Visible height in points, after rotation.
    pub fn display_height_points(&self) -> f32 {
        if self.rotation.swaps_axes() {
            self.width_points
        } else {
            self.height_points
        }
    }

    /// Pixel dimensions of this page's *visible* orientation when rendered
    /// at `dpi`.
    pub fn pixel_size(&self, dpi: u16) -> Result<(u32, u32)> {
        self.validate()?;
        let width = points_to_pixels(self.display_width_points(), dpi)?;
        let height = points_to_pixels(self.display_height_points(), dpi)?;

        let total = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or_else(|| {
                CoreError::ResourceLimitExceeded(format!(
                    "rendered page size {width}x{height} overflows a 64-bit pixel count"
                ))
            })?;
        if total > MAX_RENDERED_PIXELS {
            return Err(CoreError::ResourceLimitExceeded(format!(
                "rendered page would be {width}x{height} = {total} pixels at {dpi} DPI, \
                 exceeding the {MAX_RENDERED_PIXELS} pixel safety limit"
            )));
        }
        Ok((width, height))
    }
}

/// Converts a length in PDF points to pixels at `dpi`:
/// `pixels = round(points * dpi / 72)`.
///
/// Returns at least 1 pixel for any valid positive length, so a very small
/// but legitimate page never renders to a zero-sized bitmap.
pub fn points_to_pixels(points: f32, dpi: u16) -> Result<u32> {
    if !points.is_finite() || points <= 0.0 {
        return Err(CoreError::InvalidPageGeometry(format!(
            "cannot convert non-positive or non-finite length {points} to pixels"
        )));
    }
    if dpi == 0 {
        return Err(CoreError::InvalidParameter(
            "DPI must be greater than zero".to_string(),
        ));
    }

    // f64 for the intermediate so that large pages at 600 DPI cannot lose
    // precision before the range check below.
    let pixels = (f64::from(points) * f64::from(dpi) / 72.0).round();
    if !pixels.is_finite() || pixels < 0.0 || pixels > f64::from(u32::MAX) {
        return Err(CoreError::ResourceLimitExceeded(format!(
            "{points} points at {dpi} DPI does not fit in a 32-bit pixel count"
        )));
    }
    Ok((pixels as u32).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A4_WIDTH: f32 = 595.276;
    const A4_HEIGHT: f32 = 841.89;

    #[test]
    fn points_to_pixels_matches_the_documented_formula() {
        // 72 points = 1 inch, so 1 inch at N DPI is exactly N pixels.
        assert_eq!(points_to_pixels(72.0, 300).unwrap(), 300);
        assert_eq!(points_to_pixels(72.0, 400).unwrap(), 400);
        assert_eq!(points_to_pixels(72.0, 600).unwrap(), 600);
        // 612 x 792 (US Letter) at 300 DPI.
        assert_eq!(points_to_pixels(612.0, 300).unwrap(), 2550);
        assert_eq!(points_to_pixels(792.0, 300).unwrap(), 3300);
    }

    #[test]
    fn points_to_pixels_rounds_rather_than_truncates() {
        // 10.5 pt at 72 DPI = 10.5 px -> rounds to 11 (round-half-away).
        assert_eq!(points_to_pixels(10.5, 72).unwrap(), 11);
        // 10.4 pt at 72 DPI = 10.4 px -> 10.
        assert_eq!(points_to_pixels(10.4, 72).unwrap(), 10);
    }

    #[test]
    fn points_to_pixels_never_returns_zero_for_a_valid_length() {
        assert_eq!(points_to_pixels(0.01, 72).unwrap(), 1);
    }

    #[test]
    fn points_to_pixels_rejects_invalid_input() {
        assert!(points_to_pixels(0.0, 300).is_err());
        assert!(points_to_pixels(-1.0, 300).is_err());
        assert!(points_to_pixels(f32::NAN, 300).is_err());
        assert!(points_to_pixels(f32::INFINITY, 300).is_err());
        assert!(points_to_pixels(100.0, 0).is_err());
    }

    #[test]
    fn geometry_rejects_degenerate_and_absurd_pages() {
        assert!(PageGeometry::new(0.0, 100.0, PageRotation::None).is_err());
        assert!(PageGeometry::new(100.0, -5.0, PageRotation::None).is_err());
        assert!(PageGeometry::new(f32::NAN, 100.0, PageRotation::None).is_err());
        assert!(PageGeometry::new(20_000.0, 100.0, PageRotation::None).is_err());
        assert!(PageGeometry::new(A4_WIDTH, A4_HEIGHT, PageRotation::None).is_ok());
    }

    #[test]
    fn rotation_normalizes_documented_pdf_values() {
        assert_eq!(PageRotation::from_degrees(0).unwrap(), PageRotation::None);
        assert_eq!(
            PageRotation::from_degrees(90).unwrap(),
            PageRotation::Degrees90
        );
        assert_eq!(PageRotation::from_degrees(360).unwrap(), PageRotation::None);
        assert_eq!(
            PageRotation::from_degrees(450).unwrap(),
            PageRotation::Degrees90
        );
        assert_eq!(
            PageRotation::from_degrees(-90).unwrap(),
            PageRotation::Degrees270
        );
        assert_eq!(
            PageRotation::from_degrees(-270).unwrap(),
            PageRotation::Degrees90
        );
    }

    #[test]
    fn rotation_rejects_values_that_are_not_multiples_of_ninety() {
        assert!(PageRotation::from_degrees(45).is_err());
        assert!(PageRotation::from_degrees(1).is_err());
        assert!(PageRotation::from_degrees(-30).is_err());
    }

    #[test]
    fn display_dimensions_swap_for_ninety_and_two_seventy() {
        let portrait = PageGeometry::new(A4_WIDTH, A4_HEIGHT, PageRotation::None).unwrap();
        assert_eq!(portrait.display_width_points(), A4_WIDTH);
        assert_eq!(portrait.display_height_points(), A4_HEIGHT);

        let rot90 = PageGeometry::new(A4_WIDTH, A4_HEIGHT, PageRotation::Degrees90).unwrap();
        assert_eq!(rot90.display_width_points(), A4_HEIGHT);
        assert_eq!(rot90.display_height_points(), A4_WIDTH);

        let rot180 = PageGeometry::new(A4_WIDTH, A4_HEIGHT, PageRotation::Degrees180).unwrap();
        assert_eq!(rot180.display_width_points(), A4_WIDTH);
        assert_eq!(rot180.display_height_points(), A4_HEIGHT);

        let rot270 = PageGeometry::new(A4_WIDTH, A4_HEIGHT, PageRotation::Degrees270).unwrap();
        assert_eq!(rot270.display_width_points(), A4_HEIGHT);
        assert_eq!(rot270.display_height_points(), A4_WIDTH);
    }

    #[test]
    fn pixel_size_at_all_supported_dpis_for_portrait_and_landscape() {
        let portrait = PageGeometry::new(595.0, 842.0, PageRotation::None).unwrap();
        assert_eq!(portrait.pixel_size(300).unwrap(), (2479, 3508));
        assert_eq!(portrait.pixel_size(400).unwrap(), (3306, 4678));
        assert_eq!(portrait.pixel_size(600).unwrap(), (4958, 7017));

        let landscape = PageGeometry::new(842.0, 595.0, PageRotation::None).unwrap();
        assert_eq!(landscape.pixel_size(300).unwrap(), (3508, 2479));
    }

    #[test]
    fn pixel_size_accounts_for_rotation() {
        let base = PageGeometry::new(595.0, 842.0, PageRotation::None).unwrap();
        let rotated = PageGeometry::new(595.0, 842.0, PageRotation::Degrees90).unwrap();
        let (bw, bh) = base.pixel_size(300).unwrap();
        let (rw, rh) = rotated.pixel_size(300).unwrap();
        assert_eq!((bw, bh), (rh, rw));

        for rotation in [
            PageRotation::None,
            PageRotation::Degrees90,
            PageRotation::Degrees180,
            PageRotation::Degrees270,
        ] {
            let g = PageGeometry::new(595.0, 842.0, rotation).unwrap();
            assert!(g.pixel_size(600).is_ok(), "failed for {rotation:?}");
        }
    }

    #[test]
    fn pixel_size_rejects_pages_over_the_safety_limit() {
        // 14000 x 14000 points at 600 DPI is ~1.36e10 pixels, far over the
        // limit; must be refused rather than attempted.
        let huge = PageGeometry::new(14_000.0, 14_000.0, PageRotation::None).unwrap();
        let err = huge.pixel_size(600).unwrap_err();
        assert!(matches!(err, CoreError::ResourceLimitExceeded(_)));
        // The same page at a low DPI is fine.
        assert!(huge.pixel_size(1).is_ok());
    }
}
