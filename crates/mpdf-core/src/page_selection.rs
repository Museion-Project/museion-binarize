//! A reusable page-selection model, shared by every command that can act
//! on a subset of a document's pages (`analyze`, `preview`).
//!
//! Syntax (one-based, matching what a user sees):
//!
//! ```text
//! all
//! 1
//! 1-5
//! 1,3,8-12
//! ```
//!
//! Parsing normalizes to a sorted, deduplicated set of zero-based page
//! indices, validated against the document's actual page count.

use crate::error::{CoreError, Result};

/// A parsed, validated page selection: zero-based indices, sorted and
/// deduplicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSelection {
    indices: Vec<u32>,
}

impl PageSelection {
    /// Every page in `0..page_count`.
    pub fn all(page_count: u32) -> Self {
        Self {
            indices: (0..page_count).collect(),
        }
    }

    /// Parses the syntax documented at the top of this module against a
    /// document of `page_count` pages.
    ///
    /// Rejects: the literal digit `0` (page numbers are one-based), a
    /// non-numeric token, a reversed range (`5-1`), and any page number
    /// beyond `page_count`. Duplicate pages named more than once (e.g.
    /// `1,1,2`) are accepted and deduplicated — this is a convenience for
    /// scripts that build the argument programmatically, not a malformed
    /// input; the resulting selection is documented to contain each page
    /// at most once regardless.
    pub fn parse(spec: &str, page_count: u32) -> Result<Self> {
        let trimmed = spec.trim();
        if trimmed.eq_ignore_ascii_case("all") {
            return Ok(Self::all(page_count));
        }
        if trimmed.is_empty() {
            return Err(CoreError::InvalidParameter(
                "page selection must not be empty; use \"all\" to select every page".to_string(),
            ));
        }

        // A pathological input like "1-999999999999" must not be allowed
        // to build a multi-gigabyte Vec before the page-count check can
        // reject it. Bound the number of comma-separated terms and the
        // width of any single range up front, independent of page_count,
        // since page_count itself could be small while the spec is huge.
        const MAX_TERMS: usize = 10_000;
        const MAX_RANGE_WIDTH: u64 = 1_000_000;

        let mut indices: Vec<u32> = Vec::new();
        let terms: Vec<&str> = trimmed.split(',').map(str::trim).collect();
        if terms.len() > MAX_TERMS {
            return Err(CoreError::InvalidParameter(format!(
                "page selection has {} comma-separated terms, exceeding the limit of {MAX_TERMS}",
                terms.len()
            )));
        }

        for term in terms {
            if term.is_empty() {
                return Err(CoreError::InvalidParameter(
                    "page selection contains an empty term (check for a stray comma)".to_string(),
                ));
            }
            if let Some((start_raw, end_raw)) = term.split_once('-') {
                let start = parse_one_based(start_raw)?;
                let end = parse_one_based(end_raw)?;
                if end < start {
                    return Err(CoreError::InvalidParameter(format!(
                        "page range \"{term}\" is reversed; the end must not be before the start"
                    )));
                }
                if end - start + 1 > MAX_RANGE_WIDTH {
                    return Err(CoreError::InvalidParameter(format!(
                        "page range \"{term}\" spans more than {MAX_RANGE_WIDTH} pages"
                    )));
                }
                for page_number in start..=end {
                    indices.push(check_in_range(page_number, page_count)?);
                }
            } else {
                let page_number = parse_one_based(term)?;
                indices.push(check_in_range(page_number, page_count)?);
            }
        }

        indices.sort_unstable();
        indices.dedup();
        Ok(Self { indices })
    }

    /// The selected pages as sorted, deduplicated, zero-based indices.
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn len(&self) -> usize {
        self.indices.len()
    }
}

/// Parses a one-based page number, rejecting `0`, negatives, and anything
/// that is not a plain non-negative integer.
fn parse_one_based(raw: &str) -> Result<u64> {
    let raw = raw.trim();
    let value: u64 = raw.parse().map_err(|_| {
        CoreError::InvalidParameter(format!("\"{raw}\" is not a valid page number"))
    })?;
    if value == 0 {
        return Err(CoreError::InvalidParameter(
            "page numbers are one-based; page 0 does not exist".to_string(),
        ));
    }
    Ok(value)
}

/// Converts a validated one-based page number to a zero-based index,
/// rejecting one that is beyond `page_count`.
fn check_in_range(page_number: u64, page_count: u32) -> Result<u32> {
    if page_number > u64::from(page_count) {
        return Err(CoreError::InvalidParameter(format!(
            "page {page_number} is out of range; the document has {page_count} pages"
        )));
    }
    Ok((page_number - 1) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_selects_every_page() {
        let selection = PageSelection::parse("all", 5).unwrap();
        assert_eq!(selection.indices(), &[0, 1, 2, 3, 4]);
    }

    #[test]
    fn all_is_case_insensitive() {
        assert!(PageSelection::parse("ALL", 3).is_ok());
        assert!(PageSelection::parse("All", 3).is_ok());
    }

    #[test]
    fn a_single_page_number() {
        let selection = PageSelection::parse("3", 5).unwrap();
        assert_eq!(selection.indices(), &[2]);
    }

    #[test]
    fn a_simple_range() {
        let selection = PageSelection::parse("1-5", 5).unwrap();
        assert_eq!(selection.indices(), &[0, 1, 2, 3, 4]);
    }

    #[test]
    fn mixed_singles_and_ranges() {
        let selection = PageSelection::parse("1,3,8-12", 20).unwrap();
        assert_eq!(selection.indices(), &[0, 2, 7, 8, 9, 10, 11]);
    }

    #[test]
    fn duplicates_are_deduplicated() {
        let selection = PageSelection::parse("1,1,2,1-2", 5).unwrap();
        assert_eq!(selection.indices(), &[0, 1]);
    }

    #[test]
    fn out_of_order_input_is_sorted() {
        let selection = PageSelection::parse("5,1,3", 5).unwrap();
        assert_eq!(selection.indices(), &[0, 2, 4]);
    }

    #[test]
    fn page_zero_is_rejected() {
        assert!(PageSelection::parse("0", 5).is_err());
        assert!(PageSelection::parse("0-3", 5).is_err());
    }

    #[test]
    fn negative_like_syntax_is_rejected() {
        assert!(PageSelection::parse("-1", 5).is_err());
    }

    #[test]
    fn a_reversed_range_is_rejected() {
        let err = PageSelection::parse("5-1", 10).unwrap_err();
        assert!(err.to_string().contains("reversed"));
    }

    #[test]
    fn an_out_of_range_page_is_rejected() {
        let err = PageSelection::parse("6", 5).unwrap_err();
        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn an_out_of_range_range_end_is_rejected() {
        assert!(PageSelection::parse("1-6", 5).is_err());
    }

    #[test]
    fn non_numeric_input_is_rejected() {
        assert!(PageSelection::parse("abc", 5).is_err());
        assert!(PageSelection::parse("1-abc", 5).is_err());
    }

    #[test]
    fn empty_input_is_rejected() {
        assert!(PageSelection::parse("", 5).is_err());
        assert!(PageSelection::parse("   ", 5).is_err());
    }

    #[test]
    fn an_empty_comma_term_is_rejected() {
        assert!(PageSelection::parse("1,,2", 5).is_err());
        assert!(PageSelection::parse("1,", 5).is_err());
    }

    #[test]
    fn a_pathological_range_is_rejected_before_allocating() {
        let err = PageSelection::parse("1-99999999999999", 5).unwrap_err();
        assert!(err.to_string().contains("spans more than"));
    }

    #[test]
    fn too_many_terms_is_rejected() {
        let spec = (1..=10_001)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let err = PageSelection::parse(&spec, 20_000).unwrap_err();
        assert!(err.to_string().contains("comma-separated terms"));
    }

    #[test]
    fn all_of_a_single_page_document() {
        let selection = PageSelection::all(1);
        assert_eq!(selection.indices(), &[0]);
        assert_eq!(selection.len(), 1);
        assert!(!selection.is_empty());
    }

    #[test]
    fn whitespace_around_terms_is_tolerated() {
        let selection = PageSelection::parse(" 1 , 3 , 8 - 12 ", 20).unwrap();
        assert_eq!(selection.indices(), &[0, 2, 7, 8, 9, 10, 11]);
    }
}
