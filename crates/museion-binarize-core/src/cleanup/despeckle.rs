//! Connected-component despeckling.

use crate::bilevel::BinaryMask;

use super::{CleanupSettings, DespeckleLevel};

/// Built-in component-size limit (inclusive, in pixels) for
/// [`DespeckleLevel::Conservative`].
const CONSERVATIVE_LIMIT: u32 = 4;
/// Built-in component-size limit (inclusive, in pixels) for
/// [`DespeckleLevel::Strong`].
const STRONG_LIMIT: u32 = 12;

/// Removes small black connected components from `mask` according to
/// `settings`. A component is removed only if its pixel count is **at or
/// below** the configured limit — components above the limit are never
/// touched, silently or otherwise.
pub fn despeckle(mask: &BinaryMask, settings: &CleanupSettings) -> BinaryMask {
    let limit = match settings.despeckle {
        DespeckleLevel::Off => return mask.clone(),
        DespeckleLevel::Conservative => settings.max_component_pixels.unwrap_or(CONSERVATIVE_LIMIT),
        DespeckleLevel::Strong => settings.max_component_pixels.unwrap_or(STRONG_LIMIT),
    };
    remove_small_components(mask, limit)
}

/// Removes 4-connected black components with `size <= limit` pixels.
fn remove_small_components(mask: &BinaryMask, limit: u32) -> BinaryMask {
    let width = mask.width as usize;
    let height = mask.height as usize;
    let mut visited = vec![false; width * height];
    let mut out = mask.clone();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut component: Vec<(usize, usize)> = Vec::new();

    for start_y in 0..height {
        for start_x in 0..width {
            let start_idx = start_y * width + start_x;
            if visited[start_idx] || !mask.pixels[start_idx] {
                continue;
            }

            component.clear();
            stack.push((start_x, start_y));
            visited[start_idx] = true;

            while let Some((cx, cy)) = stack.pop() {
                component.push((cx, cy));
                let neighbors = [
                    (cx.wrapping_sub(1), cy),
                    (cx + 1, cy),
                    (cx, cy.wrapping_sub(1)),
                    (cx, cy + 1),
                ];
                for (nx, ny) in neighbors {
                    if nx < width && ny < height {
                        let nidx = ny * width + nx;
                        if !visited[nidx] && mask.pixels[nidx] {
                            visited[nidx] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
            }

            if (component.len() as u32) <= limit {
                for &(cx, cy) in &component {
                    out.set(cx as u32, cy as u32, false);
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask_from_ascii(rows: &[&str]) -> BinaryMask {
        let height = rows.len() as u32;
        let width = rows[0].len() as u32;
        let mut mask = BinaryMask::new(width, height);
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                mask.set(x as u32, y as u32, c == '#');
            }
        }
        mask
    }

    #[test]
    fn off_never_removes_anything() {
        let mask = mask_from_ascii(&["#....", ".....", "....."]);
        let settings = CleanupSettings {
            despeckle: DespeckleLevel::Off,
            max_component_pixels: None,
        };
        let out = despeckle(&mask, &settings);
        assert_eq!(mask, out);
    }

    #[test]
    fn conservative_removes_single_pixel_noise_speck() {
        let mask = mask_from_ascii(&[".....", ".....", "..#..", ".....", "....."]);
        let settings = CleanupSettings {
            despeckle: DespeckleLevel::Conservative,
            max_component_pixels: None,
        };
        let out = despeckle(&mask, &settings);
        assert!(out.pixels.iter().all(|&p| !p));
    }

    #[test]
    fn conservative_never_touches_components_above_the_limit() {
        // A 3x3 solid block (9 pixels) is well above the conservative
        // limit (4) and must survive untouched — this stands in for a
        // small glyph, dot, or accent mark, not noise.
        let mask = mask_from_ascii(&[".....", ".###.", ".###.", ".###.", "....."]);
        let settings = CleanupSettings {
            despeckle: DespeckleLevel::Conservative,
            max_component_pixels: None,
        };
        let out = despeckle(&mask, &settings);
        assert_eq!(mask, out);
    }

    #[test]
    fn strong_removes_components_conservative_would_keep() {
        // A 3x3 block (9 px) is removed at Strong (limit 12) but kept at
        // Conservative (limit 4).
        let mask = mask_from_ascii(&[".....", ".###.", ".###.", ".###.", "....."]);
        let conservative = despeckle(
            &mask,
            &CleanupSettings {
                despeckle: DespeckleLevel::Conservative,
                max_component_pixels: None,
            },
        );
        assert_eq!(mask, conservative);

        let strong = despeckle(
            &mask,
            &CleanupSettings {
                despeckle: DespeckleLevel::Strong,
                max_component_pixels: None,
            },
        );
        assert!(strong.pixels.iter().all(|&p| !p));
    }

    #[test]
    fn explicit_limit_override_is_respected() {
        let mask = mask_from_ascii(&[".....", "..#..", "....."]);
        let settings = CleanupSettings {
            despeckle: DespeckleLevel::Conservative,
            max_component_pixels: Some(0), // never remove anything
        };
        let out = despeckle(&mask, &settings);
        assert_eq!(mask, out);
    }

    #[test]
    fn dot_like_diacritic_marks_survive_conservative_when_large_enough() {
        // Simulates an apostrophe / accent: a small but multi-pixel mark
        // (5 px, a plus-shape) sitting isolated above a "letter" — larger
        // than the conservative limit (4), so it must not be deleted even
        // though a naive "small = noise" heuristic would flag it.
        let mask = mask_from_ascii(&["..#..", ".###.", "..#..", ".....", "#####"]);
        let settings = CleanupSettings {
            despeckle: DespeckleLevel::Conservative,
            max_component_pixels: None,
        };
        let out = despeckle(&mask, &settings);
        assert_eq!(
            mask, out,
            "conservative despeckle must not erase a 5px diacritic-like mark"
        );
    }

    #[test]
    fn isolated_noise_speckles_are_removed_without_touching_real_strokes() {
        let mask = mask_from_ascii(&[
            "#...#", ".....", "#####", // a real stroke: 5px in a row
            ".....", "#...#",
        ]);
        let settings = CleanupSettings {
            despeckle: DespeckleLevel::Conservative,
            max_component_pixels: None,
        };
        let out = despeckle(&mask, &settings);
        // Corner specks (1px each) removed.
        assert!(!out.get(0, 0));
        assert!(!out.get(4, 0));
        assert!(!out.get(0, 4));
        assert!(!out.get(4, 4));
        // The 5px stroke survives.
        for x in 0..5 {
            assert!(out.get(x, 2));
        }
    }
}
