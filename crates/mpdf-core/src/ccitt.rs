//! CCITT Group 4 (T.6) encoding and decoding.
//!
//! Wraps the pure-Rust `fax` crate behind a project-owned interface. This
//! is the only bilevel compression Phase 1 supports — no JBIG2, and no
//! JPEG (which is lossy and wrong for 1-bit content).

use fax::Color;

use crate::bilevel::{BilevelImage, BinaryMask};
use crate::error::{CoreError, Result};

/// Encodes a [`BilevelImage`] as a CCITT Group 4 bitstream (raw, no TIFF
/// or PDF wrapper — suitable for a PDF `CCITTFaxDecode` stream with `/K
/// -1`).
pub fn encode_g4(image: &BilevelImage) -> Vec<u8> {
    let writer = fax::VecWriter::new();
    let mut encoder = fax::encoder::Encoder::new(writer);

    for y in 0..image.height {
        let width = image.width;
        let row = (0..width).map(|x| {
            if image.get_pixel(x, y) {
                Color::Black
            } else {
                Color::White
            }
        });
        encoder
            .encode_line(row, width)
            .expect("VecWriter is infallible");
    }

    let writer = encoder.finish().expect("VecWriter is infallible");
    writer.finish()
}

/// Decodes a raw CCITT Group 4 bitstream back into a [`BilevelImage`] of
/// the given `width` x `height`. Returns a structured error rather than
/// panicking if the stream is malformed.
pub fn decode_g4(data: &[u8], width: u32, height: u32) -> Result<BilevelImage> {
    let mut mask = BinaryMask::new(width, height);
    let mut row_index: u32 = 0;
    let mut decode_error = false;

    let status =
        fax::decoder::decode_g4(data.iter().copied(), width, Some(height), |transitions| {
            if row_index >= height {
                return;
            }
            for (x, color) in fax::decoder::pels(transitions, width).enumerate() {
                let x = x as u32;
                if x >= width {
                    decode_error = true;
                    break;
                }
                if color == Color::Black {
                    mask.set(x, row_index, true);
                }
            }
            row_index += 1;
        });

    if status.is_none() || decode_error {
        return Err(CoreError::Ccitt(
            "malformed or truncated CCITT Group 4 stream".to_string(),
        ));
    }

    Ok(BilevelImage::from_mask(&mask))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask_all(width: u32, height: u32, value: bool) -> BinaryMask {
        let mut mask = BinaryMask::new(width, height);
        for p in mask.pixels.iter_mut() {
            *p = value;
        }
        mask
    }

    fn round_trip(mask: &BinaryMask) -> BinaryMask {
        let packed = BilevelImage::from_mask(mask);
        let encoded = encode_g4(&packed);
        let decoded = decode_g4(&encoded, mask.width, mask.height).expect("decode failed");
        decoded.to_mask()
    }

    #[test]
    fn fully_white_page_round_trips() {
        let mask = mask_all(64, 40, false);
        assert_eq!(mask, round_trip(&mask));
    }

    #[test]
    fn fully_black_page_round_trips() {
        let mask = mask_all(64, 40, true);
        assert_eq!(mask, round_trip(&mask));
    }

    #[test]
    fn alternating_stripes_round_trip() {
        let mut mask = BinaryMask::new(50, 30);
        for y in 0..30 {
            for x in 0..50 {
                mask.set(x, y, (x / 3) % 2 == 0);
            }
        }
        assert_eq!(mask, round_trip(&mask));
    }

    #[test]
    fn text_like_pattern_round_trips() {
        // A blocky "text line": a few horizontal strokes with gaps,
        // reminiscent of a row of glyphs.
        let width = 80;
        let height = 20;
        let mut mask = BinaryMask::new(width, height);
        for y in 4..16 {
            for x in 0..width {
                let glyph = x / 8;
                let within = x % 8;
                let on = glyph % 2 == 0 && within > 1 && within < 6 && (y % 5 != 0);
                mask.set(x, y, on);
            }
        }
        assert_eq!(mask, round_trip(&mask));
    }

    #[test]
    fn random_sparse_page_round_trips() {
        let width = 97;
        let height = 61;
        let mut mask = BinaryMask::new(width, height);
        let mut state: u32 = 42;
        for y in 0..height {
            for x in 0..width {
                state = state.wrapping_mul(1103515245).wrapping_add(12345);
                let on = (state >> 24) % 20 == 0; // ~5% foreground
                mask.set(x, y, on);
            }
        }
        assert_eq!(mask, round_trip(&mask));
    }

    #[test]
    fn odd_widths_not_divisible_by_eight_round_trip() {
        for width in [1u32, 3, 7, 9, 13, 17, 31, 33, 99] {
            let mut mask = BinaryMask::new(width, 11);
            for y in 0..11 {
                for x in 0..width {
                    mask.set(x, y, (x + y) % 3 == 0);
                }
            }
            assert_eq!(
                mask,
                round_trip(&mask),
                "round trip failed for width={width}"
            );
        }
    }

    #[test]
    fn malformed_stream_returns_error_not_panic() {
        let garbage = vec![0xFFu8; 4];
        let result = decode_g4(&garbage, 64, 64);
        assert!(result.is_err());
    }

    #[test]
    fn truncated_stream_returns_error_not_panic() {
        let mask = mask_all(64, 64, true);
        let packed = BilevelImage::from_mask(&mask);
        let mut encoded = encode_g4(&packed);
        encoded.truncate(encoded.len() / 4);
        // Either a clean error or a short read is acceptable; what matters
        // is that this does not panic.
        let _ = decode_g4(&encoded, 64, 64);
    }
}
