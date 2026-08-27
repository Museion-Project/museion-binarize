use crate::derived::Bbox;
use crate::document_package::{AffineTransform, Page};
use crate::error::{CoreError, Result};

/// Converts a master top-left rectangle to the source page's default user
/// space. The final rotation is the same mapping used by explicit PDF XYZ
/// destinations. It returns a rectangle enclosing the four transformed
/// corners and rejects singular, non-finite, and out-of-page input.
pub fn master_bbox_to_visible_pdf(page: &Page, bbox: Bbox) -> Result<Bbox> {
    if !bbox.finite()
        || bbox.x + bbox.width > page.master_space.width + 1e-6
        || bbox.y + bbox.height > page.master_space.height + 1e-6
    {
        return Err(CoreError::InvalidPageGeometry(
            "master bbox is outside page".into(),
        ));
    }
    let t = page
        .transforms
        .iter()
        .find(|x| x.from_space == page.source_space.id && x.to_space == page.master_space.id)
        .ok_or_else(|| {
            CoreError::InvalidPageGeometry("source-to-master transform missing".into())
        })?;
    let det = t.a * t.d - t.b * t.c;
    if !det.is_finite() || det.abs() < f64::EPSILON {
        return Err(CoreError::InvalidPageGeometry(
            "affine transform is singular".into(),
        ));
    }
    let corners = [
        (bbox.x, bbox.y),
        (bbox.x + bbox.width, bbox.y),
        (bbox.x, bbox.y + bbox.height),
        (bbox.x + bbox.width, bbox.y + bbox.height),
    ]
    .map(|(x, y)| inverse(t, x, y));
    if corners
        .iter()
        .any(|(x, y)| !x.is_finite() || !y.is_finite())
    {
        return Err(CoreError::InvalidPageGeometry(
            "non-finite transformed bbox".into(),
        ));
    }
    let minx = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let maxx = corners
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let miny = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let maxy = corners
        .iter()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let x = minx;
    // The inverse declared transform already lands in the source space's
    // bottom-left coordinate system (the MDP source-space contract).
    let y = miny;
    let w = maxx - minx;
    let h = maxy - miny;
    let out = Bbox {
        x,
        y,
        width: w,
        height: h,
    };
    if !out.finite()
        || out.x < -1e-6
        || out.y < -1e-6
        || out.x + out.width > page.source_space.width + 1e-6
        || out.y + out.height > page.source_space.height + 1e-6
    {
        return Err(CoreError::InvalidPageGeometry(
            "transformed bbox is outside visible PDF page".into(),
        ));
    }
    Ok(out)
}

/// Converts a master rectangle into zero-origin default page user space.
/// Searchable-PDF write-back adds the resolved page-box lower-left origin.
pub fn master_bbox_to_pdf(page: &Page, bbox: Bbox) -> Result<Bbox> {
    let visible = master_bbox_to_visible_pdf(page, bbox)?;
    let (x, y, w, h) = visible_to_default(
        page.rotation_degrees,
        page.source_space.width,
        page.source_space.height,
        visible.x,
        visible.y,
        visible.width,
        visible.height,
    );
    let out = Bbox {
        x,
        y,
        width: w,
        height: h,
    };
    if !out.finite()
        || out.x < -1e-6
        || out.y < -1e-6
        || out.x + out.width > rotated_width(page) + 1e-6
        || out.y + out.height > rotated_height(page) + 1e-6
    {
        return Err(CoreError::InvalidPageGeometry(
            "transformed bbox is outside PDF page".into(),
        ));
    }
    Ok(out)
}
fn inverse(t: &AffineTransform, x: f64, y: f64) -> (f64, f64) {
    (
        (t.d * (x - t.e) - t.c * (y - t.f)) / (t.a * t.d - t.b * t.c),
        (-t.b * (x - t.e) + t.a * (y - t.f)) / (t.a * t.d - t.b * t.c),
    )
}
pub fn visible_to_default(
    r: u16,
    visible_width: f64,
    visible_height: f64,
    x: f64,
    y: f64,
    bw: f64,
    bh: f64,
) -> (f64, f64, f64, f64) {
    match r {
        0 => (x, y, bw, bh),
        90 => (visible_height - y - bh, x, bh, bw),
        180 => (visible_width - x - bw, visible_height - y - bh, bw, bh),
        270 => (y, visible_width - x - bw, bh, bw),
        _ => (x, y, bw, bh),
    }
}
pub fn rotated_width(page: &Page) -> f64 {
    if matches!(page.rotation_degrees, 90 | 270) {
        page.source_space.height
    } else {
        page.source_space.width
    }
}
pub fn rotated_height(page: &Page) -> f64 {
    if matches!(page.rotation_degrees, 90 | 270) {
        page.source_space.width
    } else {
        page.source_space.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_package::*;
    fn p(r: u16) -> Page {
        Page {
            page_id: "p".into(),
            physical_index: 0,
            order: 0,
            rotation_degrees: r,
            master_space: CoordinateSpace {
                id: "m".into(),
                unit: CoordinateUnit::Pixels,
                width: 100.,
                height: 200.,
                origin: Origin::TopLeft,
                pixels_per_inch: Some(300),
            },
            source_space: CoordinateSpace {
                id: "s".into(),
                unit: CoordinateUnit::PdfPoints,
                width: 100.,
                height: 200.,
                origin: Origin::BottomLeft,
                pixels_per_inch: None,
            },
            transforms: vec![AffineTransform {
                from_space: "s".into(),
                to_space: "m".into(),
                a: 1.,
                b: 0.,
                c: 0.,
                d: -1.,
                e: 0.,
                f: 200.,
            }],
            printed_page_label: None,
            existing_outline_evidence: vec![],
            typography_evidence: vec![],
            region_evidence: vec![],
            asset_ids: vec![],
        }
    }
    #[test]
    fn rotation_destinations() {
        let b = Bbox {
            x: 10.,
            y: 20.,
            width: 30.,
            height: 40.,
        };
        assert_eq!(
            master_bbox_to_pdf(&p(0), b).unwrap(),
            Bbox {
                x: 10.,
                y: 140.,
                width: 30.,
                height: 40.
            }
        );
        assert_eq!(
            master_bbox_to_pdf(&p(90), b).unwrap(),
            Bbox {
                x: 20.,
                y: 10.,
                width: 40.,
                height: 30.
            }
        );
        assert_eq!(
            master_bbox_to_pdf(&p(180), b).unwrap(),
            Bbox {
                x: 60.,
                y: 20.,
                width: 30.,
                height: 40.
            }
        );
        assert_eq!(
            master_bbox_to_pdf(&p(270), b).unwrap(),
            Bbox {
                x: 140.,
                y: 60.,
                width: 40.,
                height: 30.
            }
        );
    }
}
