//! Bilevel (1-bit) image representation and packing.
//!
//! [`BinaryMask`] is the unpacked, one-`bool`-per-pixel working
//! representation produced by the binarization algorithms.
//! [`BilevelImage`] is the packed, eight-pixels-per-byte representation
//! that is handed to the CCITT Group 4 encoder and, eventually, embedded
//! in the output PDF.

/// An unpacked bilevel mask: one `bool` per pixel.
///
/// `true` marks a black (foreground, ink) pixel. The Phase 1 pipeline
/// assumes dark text on a light background; inversion is handled
/// internally by individual algorithms, never exposed as a public toggle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryMask {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<bool>,
}

impl BinaryMask {
    /// Creates an all-white (no foreground) mask of the given dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![false; width as usize * height as usize],
        }
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> bool {
        self.pixels[y as usize * self.width as usize + x as usize]
    }

    #[inline]
    pub fn set(&mut self, x: u32, y: u32, value: bool) {
        self.pixels[y as usize * self.width as usize + x as usize] = value;
    }
}

/// A packed 1-bit-per-pixel bitmap: eight pixels per byte, most-significant
/// bit first within each byte.
///
/// Rows are padded to a whole number of bytes; `stride` is the number of
/// bytes per row, which may exceed `width.div_ceil(8)` only in the sense
/// that it is exactly `width.div_ceil(8)` — there is no additional
/// alignment padding beyond the final partial byte of each row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BilevelImage {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub data: Vec<u8>,
    /// If `true`, a set bit (`1`) represents a black pixel. This is always
    /// `true` for images produced by this crate; the field exists so the
    /// convention is explicit wherever a `BilevelImage` is consumed (e.g.
    /// by the PDF writer's `/BlackIs1` decode parameter).
    pub black_is_one: bool,
}

impl BilevelImage {
    /// Packs an unpacked [`BinaryMask`] into a [`BilevelImage`].
    pub fn from_mask(mask: &BinaryMask) -> Self {
        let width = mask.width;
        let height = mask.height;
        let stride = (width as usize).div_ceil(8);
        let mut data = vec![0u8; stride * height as usize];

        for y in 0..height {
            for x in 0..width {
                if mask.get(x, y) {
                    let byte_index = y as usize * stride + (x as usize / 8);
                    let bit_index = 7 - (x as usize % 8);
                    data[byte_index] |= 1 << bit_index;
                }
            }
        }

        Self {
            width,
            height,
            stride,
            data,
            black_is_one: true,
        }
    }

    /// Unpacks this image back into a [`BinaryMask`].
    ///
    /// Round-tripping `BinaryMask -> BilevelImage -> BinaryMask` must be
    /// pixel-identical; this is covered by unit tests in this module.
    pub fn to_mask(&self) -> BinaryMask {
        let mut mask = BinaryMask::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                mask.set(x, y, self.get_pixel(x, y));
            }
        }
        mask
    }

    /// Returns whether the pixel at `(x, y)` is black, accounting for
    /// `black_is_one`.
    #[inline]
    pub fn get_pixel(&self, x: u32, y: u32) -> bool {
        let byte_index = y as usize * self.stride + (x as usize / 8);
        let bit_index = 7 - (x as usize % 8);
        let bit_set = (self.data[byte_index] >> bit_index) & 1 == 1;
        bit_set == self.black_is_one
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkerboard_mask(width: u32, height: u32) -> BinaryMask {
        let mut mask = BinaryMask::new(width, height);
        for y in 0..height {
            for x in 0..width {
                mask.set(x, y, (x + y) % 2 == 0);
            }
        }
        mask
    }

    #[test]
    fn round_trip_is_pixel_identical_for_even_width() {
        let mask = checkerboard_mask(16, 10);
        let packed = BilevelImage::from_mask(&mask);
        assert_eq!(packed.stride, 2);
        let unpacked = packed.to_mask();
        assert_eq!(mask, unpacked);
    }

    #[test]
    fn round_trip_is_pixel_identical_for_width_not_divisible_by_eight() {
        for width in [1u32, 3, 7, 9, 13, 17, 31, 33] {
            let mask = checkerboard_mask(width, 5);
            let packed = BilevelImage::from_mask(&mask);
            assert_eq!(packed.stride, (width as usize).div_ceil(8));
            let unpacked = packed.to_mask();
            assert_eq!(mask, unpacked, "round trip failed for width={width}");
        }
    }

    #[test]
    fn all_white_and_all_black_round_trip() {
        let white = BinaryMask::new(23, 11);
        assert_eq!(white, BilevelImage::from_mask(&white).to_mask());

        let mut black = BinaryMask::new(23, 11);
        for p in black.pixels.iter_mut() {
            *p = true;
        }
        assert_eq!(black, BilevelImage::from_mask(&black).to_mask());
    }

    #[test]
    fn padding_bits_are_zero() {
        // width=9 -> stride=2 bytes -> 7 padding bits in the second byte.
        let mask = {
            let mut m = BinaryMask::new(9, 1);
            for x in 0..9 {
                m.set(x, 0, true);
            }
            m
        };
        let packed = BilevelImage::from_mask(&mask);
        // First byte: bits 0..7 (x=0..7) all set -> 0xFF.
        assert_eq!(packed.data[0], 0xFF);
        // Second byte: only bit 7 (x=8) set -> 0b1000_0000.
        assert_eq!(packed.data[1], 0b1000_0000);
    }

    #[test]
    fn msb_first_bit_order() {
        // x=0 -> MSB (0x80), x=7 -> LSB (0x01).
        let mut mask = BinaryMask::new(8, 1);
        mask.set(0, 0, true);
        let packed = BilevelImage::from_mask(&mask);
        assert_eq!(packed.data[0], 0b1000_0000);

        let mut mask = BinaryMask::new(8, 1);
        mask.set(7, 0, true);
        let packed = BilevelImage::from_mask(&mask);
        assert_eq!(packed.data[0], 0b0000_0001);
    }
}
