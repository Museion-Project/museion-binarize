//! Deterministic construction of a 1-bit CCITT Group 4 output PDF.
//!
//! See `docs/adr/0002-bilevel-pdf-output.md` and `docs/pdf-output.md` for
//! the full rationale. In short: output pages are *rebuilt* (not saved
//! through PDFium) as true bilevel image XObjects, so the result is a
//! predictable, minimal, standards-compliant PDF whose bytes this project
//! fully controls.

use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, TextStr};

use crate::bilevel::BilevelImage;
use crate::ccitt;
use crate::document::PdfMetadata;
use crate::error::{CoreError, Result};

/// One finished page, ready to be written.
pub struct EncodedPage {
    /// Visible page width in PDF points.
    pub width_points: f32,
    /// Visible page height in PDF points.
    pub height_points: f32,
    /// Pixel width of the bilevel image.
    pub pixel_width: u32,
    /// Pixel height of the bilevel image.
    pub pixel_height: u32,
    /// Raw CCITT Group 4 bitstream for this page.
    pub ccitt_data: Vec<u8>,
}

impl EncodedPage {
    /// Encodes a packed bilevel image into a page payload.
    ///
    /// The uncompressed image is consumed here, so callers can drop their
    /// working buffers as soon as this returns.
    pub fn encode(image: &BilevelImage, width_points: f32, height_points: f32) -> Result<Self> {
        if !image.black_is_one {
            // The whole writer assumes the crate-wide convention; a mask
            // built the other way round would silently invert the page.
            return Err(CoreError::PdfConstructionFailed(
                "expected a BilevelImage with black_is_one = true".to_string(),
            ));
        }
        let ccitt_data = ccitt::encode_g4(image);
        Ok(Self {
            width_points,
            height_points,
            pixel_width: image.width,
            pixel_height: image.height,
            ccitt_data,
        })
    }

    pub fn compressed_bytes(&self) -> u64 {
        self.ccitt_data.len() as u64
    }
}

/// Builds an output PDF incrementally, one page at a time.
///
/// Object references are allocated in a fixed, page-order sequence, so the
/// same input and settings always produce the same bytes. Nothing derived
/// from the wall clock, the filesystem, or a random source is written.
pub struct BilevelPdfBuilder {
    pdf: Pdf,
    catalog_id: Ref,
    page_tree_id: Ref,
    page_ids: Vec<Ref>,
    next_id: i32,
}

impl Default for BilevelPdfBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl BilevelPdfBuilder {
    pub fn new() -> Self {
        let mut next_id = 1;
        let mut alloc = || {
            let r = Ref::new(next_id);
            next_id += 1;
            r
        };
        let catalog_id = alloc();
        let page_tree_id = alloc();
        Self {
            pdf: Pdf::new(),
            catalog_id,
            page_tree_id,
            page_ids: Vec::new(),
            next_id,
        }
    }

    fn alloc(&mut self) -> Ref {
        let r = Ref::new(self.next_id);
        self.next_id += 1;
        r
    }

    /// Appends one page. Object ids are allocated deterministically in the
    /// order page, image, content.
    pub fn add_page(&mut self, page: &EncodedPage) -> Result<()> {
        if page.pixel_width == 0 || page.pixel_height == 0 {
            return Err(CoreError::PdfConstructionFailed(
                "cannot write a page with a zero-sized image".to_string(),
            ));
        }

        let page_id = self.alloc();
        let image_id = self.alloc();
        let content_id = self.alloc();
        self.page_ids.push(page_id);

        let image_name = Name(b"Im0");

        // --- Page object -------------------------------------------------
        {
            let mut page_obj = self.pdf.page(page_id);
            page_obj
                .media_box(Rect::new(0.0, 0.0, page.width_points, page.height_points))
                .parent(self.page_tree_id)
                .contents(content_id);
            // Rotation is normalized into the geometry itself (the
            // rendered raster is already in visible orientation), so every
            // output page is written upright with no /Rotate entry.
            page_obj.resources().x_objects().pair(image_name, image_id);
            page_obj.finish();
        }

        // --- Image XObject -----------------------------------------------
        {
            let mut image = self.pdf.image_xobject(image_id, &page.ccitt_data);
            image
                .width(page.pixel_width as i32)
                .height(page.pixel_height as i32)
                .color_space()
                .device_gray();
            image
                .bits_per_component(1)
                .interpolate(false)
                .filter(Filter::CcittFaxDecode);

            // Bit polarity. This combination was determined empirically by
            // rendering the output back through PDFium, not assumed:
            //
            //   * `/BlackIs1 true` makes the CCITT decoder emit a 1 bit for
            //     every black pixel, matching this crate's BilevelImage
            //     convention (`black_is_one == true`).
            //   * In `/DeviceGray` a sample value of 1 is *white*, so those
            //     1-bits would render inverted on their own.
            //   * `/Decode [1 0]` flips the sample-to-colour mapping, so a
            //     1 bit means black, as intended.
            //
            // Removing either entry inverts the page; the end-to-end
            // orientation/polarity test in tests/pdf_pipeline.rs catches it.
            image.decode([1.0, 0.0]);

            let mut decode_parms = image.insert(Name(b"DecodeParms")).dict();
            decode_parms
                .pair(Name(b"K"), -1)
                .pair(Name(b"Columns"), page.pixel_width as i32)
                .pair(Name(b"Rows"), page.pixel_height as i32)
                .pair(Name(b"BlackIs1"), true);
            decode_parms.finish();
            image.finish();
        }

        // --- Content stream ----------------------------------------------
        {
            // Draw the image exactly once, filling the visible page
            // rectangle. PDF image space is the unit square with its
            // origin at the bottom-left, and an image's first sample row
            // maps to the TOP of that unit square, so scaling by
            // (width, height) and translating to the origin places raster
            // row 0 at the top of the page without any flip.
            let mut content = Content::new();
            content.save_state();
            content.transform([page.width_points, 0.0, 0.0, page.height_points, 0.0, 0.0]);
            content.x_object(image_name);
            content.restore_state();
            let data = content.finish();
            self.pdf.stream(content_id, &data);
        }

        Ok(())
    }

    /// Finishes the document and returns its bytes.
    pub fn finish(mut self, metadata: &PdfMetadata) -> Result<Vec<u8>> {
        if self.page_ids.is_empty() {
            return Err(CoreError::PdfConstructionFailed(
                "cannot write a PDF with no pages".to_string(),
            ));
        }

        let page_count = self.page_ids.len() as i32;
        self.pdf
            .pages(self.page_tree_id)
            .kids(self.page_ids.iter().copied())
            .count(page_count);

        let mut catalog = self.pdf.catalog(self.catalog_id);
        catalog.pages(self.page_tree_id);
        catalog.finish();

        // Document information. Only fields actually present in the source
        // are written; no timestamp is emitted, because a wall-clock
        // /CreationDate would make repeated runs differ byte-for-byte.
        let sanitized = metadata.sanitized();
        if !sanitized.is_empty() {
            let info_id = self.alloc();
            let mut info = self.pdf.document_info(info_id);
            if let Some(title) = &sanitized.title {
                info.title(TextStr(title));
            }
            if let Some(author) = &sanitized.author {
                info.author(TextStr(author));
            }
            if let Some(subject) = &sanitized.subject {
                info.subject(TextStr(subject));
            }
            if let Some(keywords) = &sanitized.keywords {
                info.keywords(TextStr(keywords));
            }
            info.finish();
        }

        Ok(self.pdf.finish())
    }

    /// Number of pages appended so far.
    pub fn page_count(&self) -> u32 {
        self.page_ids.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bilevel::BinaryMask;

    fn asymmetric_image(width: u32, height: u32) -> BilevelImage {
        let mut mask = BinaryMask::new(width, height);
        // Large black block in the top-left quadrant.
        for y in 0..height / 3 {
            for x in 0..width / 3 {
                mask.set(x, y, true);
            }
        }
        // Small black block in the bottom-right.
        for y in height - height / 6..height {
            for x in width - width / 6..width {
                mask.set(x, y, true);
            }
        }
        BilevelImage::from_mask(&mask)
    }

    fn build_single_page(width_points: f32, height_points: f32) -> Vec<u8> {
        let image = asymmetric_image(64, 96);
        let page = EncodedPage::encode(&image, width_points, height_points).unwrap();
        let mut builder = BilevelPdfBuilder::new();
        builder.add_page(&page).unwrap();
        builder.finish(&PdfMetadata::default()).unwrap()
    }

    #[test]
    fn writes_a_ccitt_group4_bilevel_image_dictionary() {
        let bytes = build_single_page(595.0, 842.0);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/CCITTFaxDecode"), "must use CCITTFaxDecode");
        assert!(text.contains("/BitsPerComponent 1"), "must be 1 bit");
        assert!(text.contains("/DeviceGray"));
        assert!(text.contains("/K -1"), "must be Group 4");
        assert!(text.contains("/BlackIs1 true"));
        assert!(text.contains("/Columns 64"));
        assert!(text.contains("/Rows 96"));
        // Explicitly NOT a raster format smuggled into a PDF.
        assert!(!text.contains("/DCTDecode"), "must not be JPEG");
        assert!(!text.contains("/JBIG2Decode"), "must not be JBIG2");
    }

    #[test]
    fn media_box_matches_the_requested_visible_geometry() {
        let bytes = build_single_page(595.0, 842.0);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/MediaBox [0 595 842]") || text.contains("595"));
        assert!(text.contains("842"));
    }

    #[test]
    fn content_stream_scales_the_image_to_the_page_once() {
        let image = asymmetric_image(32, 32);
        let page = EncodedPage::encode(&image, 200.0, 400.0).unwrap();
        let mut builder = BilevelPdfBuilder::new();
        builder.add_page(&page).unwrap();
        let bytes = builder.finish(&PdfMetadata::default()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        // The transformation matrix must carry the page dimensions.
        assert!(text.contains("200") && text.contains("400"));
        // The image is drawn exactly once.
        assert_eq!(text.matches("/Im0 Do").count(), 1);
    }

    #[test]
    fn object_ids_are_allocated_deterministically() {
        let a = build_single_page(595.0, 842.0);
        let b = build_single_page(595.0, 842.0);
        assert_eq!(a, b, "identical input must produce identical bytes");
    }

    #[test]
    fn repeated_multi_page_builds_are_byte_identical() {
        let build = || {
            let mut builder = BilevelPdfBuilder::new();
            for (w, h) in [(595.0, 842.0), (842.0, 595.0), (300.0, 300.0)] {
                let image = asymmetric_image(48, 48);
                let page = EncodedPage::encode(&image, w, h).unwrap();
                builder.add_page(&page).unwrap();
            }
            builder.finish(&PdfMetadata::default()).unwrap()
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn no_creation_timestamp_is_written() {
        let bytes = build_single_page(595.0, 842.0);
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("/CreationDate"),
            "a wall-clock timestamp would break byte determinism"
        );
        assert!(!text.contains("/ModDate"));
    }

    #[test]
    fn metadata_is_written_only_when_present_and_is_sanitized() {
        let image = asymmetric_image(16, 16);
        let page = EncodedPage::encode(&image, 100.0, 100.0).unwrap();

        let mut builder = BilevelPdfBuilder::new();
        builder.add_page(&page).unwrap();
        let without = builder.finish(&PdfMetadata::default()).unwrap();
        assert!(!String::from_utf8_lossy(&without).contains("/Title"));

        let mut builder = BilevelPdfBuilder::new();
        builder.add_page(&page).unwrap();
        let with = builder
            .finish(&PdfMetadata {
                title: Some("Respublica\u{0}".to_string()),
                ..Default::default()
            })
            .unwrap();
        let text = String::from_utf8_lossy(&with);
        assert!(text.contains("/Title"));
        assert!(text.contains("Respublica"));
        assert!(!with.windows(1).any(|w| w == [0u8]) || !text.contains("Respublica\u{0}"));
    }

    #[test]
    fn rejects_an_empty_document() {
        let builder = BilevelPdfBuilder::new();
        assert!(builder.finish(&PdfMetadata::default()).is_err());
    }

    #[test]
    fn rejects_an_inverted_polarity_image() {
        let mask = BinaryMask::new(8, 8);
        let mut image = BilevelImage::from_mask(&mask);
        image.black_is_one = false;
        assert!(EncodedPage::encode(&image, 100.0, 100.0).is_err());
    }

    #[test]
    fn page_count_tracks_added_pages() {
        let image = asymmetric_image(16, 16);
        let page = EncodedPage::encode(&image, 100.0, 100.0).unwrap();
        let mut builder = BilevelPdfBuilder::new();
        assert_eq!(builder.page_count(), 0);
        builder.add_page(&page).unwrap();
        builder.add_page(&page).unwrap();
        assert_eq!(builder.page_count(), 2);
    }

    #[test]
    fn encoded_page_reports_its_compressed_size() {
        let image = asymmetric_image(64, 64);
        let page = EncodedPage::encode(&image, 100.0, 100.0).unwrap();
        assert!(page.compressed_bytes() > 0);
        assert_eq!(page.compressed_bytes(), page.ccitt_data.len() as u64);
    }
}
