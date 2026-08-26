//! M PDF-owned document and page metadata types.
//!
//! These types deliberately contain no PDFium types, so front ends (CLI,
//! desktop app) can display document information without linking against
//! or knowing about the rendering backend.

use crate::page_geometry::{PageGeometry, PageRotation};

/// Basic document metadata copied from the source PDF where safely
/// available. Fields absent from the source stay `None` — nothing here is
/// invented.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PdfMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
}

impl PdfMetadata {
    /// Returns a copy with control characters stripped and each field
    /// truncated to a sane length, so that hostile document metadata
    /// cannot corrupt terminal output or bloat the written PDF.
    pub fn sanitized(&self) -> Self {
        Self {
            title: self.title.as_deref().map(sanitize_metadata_value),
            author: self.author.as_deref().map(sanitize_metadata_value),
            subject: self.subject.as_deref().map(sanitize_metadata_value),
            keywords: self.keywords.as_deref().map(sanitize_metadata_value),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.author.is_none()
            && self.subject.is_none()
            && self.keywords.is_none()
    }
}

/// Maximum length of any single copied metadata string.
const MAX_METADATA_LEN: usize = 512;

fn sanitize_metadata_value(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.chars().count() <= MAX_METADATA_LEN {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX_METADATA_LEN).collect()
}

/// Information about one page of a source document.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfPageInfo {
    /// Zero-based page index. User-facing output adds one.
    pub index: u32,
    /// The page's **visible** geometry, with any `/Rotate` already
    /// applied. See [`PageGeometry`] for the convention.
    pub geometry: PageGeometry,
    /// The `/Rotate` value declared by the source page.
    ///
    /// This is **informational only** — it is reported to the user and
    /// never used to transform `geometry`, which is already visible.
    /// Rebuilt output pages are always written upright, so this is
    /// [`PageRotation::None`] for any document this crate produced.
    pub source_rotation: PageRotation,
}

impl PdfPageInfo {
    /// One-based page number, for user-facing messages.
    pub fn page_number(&self) -> u32 {
        self.index + 1
    }
}

/// The result of inspecting a source PDF.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfDocumentInfo {
    pub page_count: u32,
    pub pages: Vec<PdfPageInfo>,
    pub metadata: PdfMetadata,
    /// Size of the source file on disk, in bytes.
    pub source_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_number_is_one_based() {
        let page = PdfPageInfo {
            index: 0,
            geometry: PageGeometry::new(595.0, 842.0).unwrap(),
            source_rotation: PageRotation::None,
        };
        assert_eq!(page.index, 0);
        assert_eq!(page.page_number(), 1);
    }

    #[test]
    fn metadata_sanitization_strips_control_characters() {
        let metadata = PdfMetadata {
            title: Some("Hello\u{0}\u{1}\nWorld".to_string()),
            ..Default::default()
        };
        assert_eq!(metadata.sanitized().title.unwrap(), "HelloWorld");
    }

    #[test]
    fn metadata_sanitization_trims_and_truncates() {
        let long = "x".repeat(MAX_METADATA_LEN + 100);
        let metadata = PdfMetadata {
            author: Some(format!("  {long}  ")),
            ..Default::default()
        };
        let sanitized = metadata.sanitized().author.unwrap();
        assert_eq!(sanitized.chars().count(), MAX_METADATA_LEN);
    }

    #[test]
    fn metadata_sanitization_preserves_ordinary_text() {
        let metadata = PdfMetadata {
            title: Some("Plato — Respublica".to_string()),
            author: Some("Pei Haoran".to_string()),
            ..Default::default()
        };
        let sanitized = metadata.sanitized();
        assert_eq!(sanitized.title.unwrap(), "Plato — Respublica");
        assert_eq!(sanitized.author.unwrap(), "Pei Haoran");
    }

    #[test]
    fn empty_metadata_is_reported_as_empty() {
        assert!(PdfMetadata::default().is_empty());
        assert!(!PdfMetadata {
            title: Some("t".into()),
            ..Default::default()
        }
        .is_empty());
    }
}
