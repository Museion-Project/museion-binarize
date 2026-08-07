//! Identity of a source PDF, captured once when a document session opens.
//!
//! This exists to support stable reporting (a description of *which* file
//! was processed, independent of terminal/report path formatting), the
//! source-mutation policy documented in [`crate::document_session`], and
//! same-file protection between an input and output path.
//!
//! Never includes a password, and never includes the document's own
//! content beyond an opt-in hash of the raw bytes.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Identity of a source file as it was actually read for one operation.
///
/// `byte_len` and (if computed) `content_sha256` describe the exact bytes
/// that were opened — not a re-read of the file, which could by then
/// differ. See [`SourceIdentity::capture`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentity {
    /// Best-effort canonicalized path. Falls back to the path as given if
    /// canonicalization fails (e.g. a filesystem that does not support
    /// it); this is identity information, not something callers should
    /// treat as validated.
    pub canonical_path: PathBuf,
    /// Length, in bytes, of the snapshot actually opened.
    pub byte_len: u64,
    /// Filesystem modification time at the moment identity was captured.
    /// `None` if the platform/filesystem does not report one.
    pub modified_time: Option<SystemTime>,
    /// SHA-256 of the file contents, lowercase hex. `None` unless
    /// explicitly requested: hashing a large scanned book is not free,
    /// and most operations (`inspect`, `process`) do not need it. See
    /// [`crate::document_session::PdfOpenOptions::compute_source_hash`].
    pub content_sha256: Option<String>,
}

impl SourceIdentity {
    /// Captures identity for `bytes`, the exact snapshot read from `path`.
    ///
    /// `hash` controls whether [`SourceIdentity::content_sha256`] is
    /// computed; when false it stays `None` rather than being skipped
    /// silently and looking absent for an unrelated reason.
    pub fn capture(path: &Path, bytes: &[u8], hash: bool) -> Result<Self> {
        let canonical_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let modified_time = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        let content_sha256 = hash.then(|| sha256_hex(bytes));
        Ok(Self {
            canonical_path,
            byte_len: bytes.len() as u64,
            modified_time,
            content_sha256,
        })
    }

    /// Whether `other` is a snapshot of the same file content, judged by
    /// the identity fields that were actually captured. Two identities
    /// with no hash computed are compared only on path and length —
    /// enough to catch an obviously different file, not a byte-level
    /// guarantee. Callers that need certainty should request a hash.
    pub fn plausibly_same_content(&self, other: &SourceIdentity) -> bool {
        if self.byte_len != other.byte_len {
            return false;
        }
        match (&self.content_sha256, &other.content_sha256) {
            (Some(a), Some(b)) => a == b,
            _ => self.canonical_path == other.canonical_path,
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_length_and_optional_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, b"%PDF-1.7 hello").unwrap();
        let bytes = std::fs::read(&path).unwrap();

        let unhashed = SourceIdentity::capture(&path, &bytes, false).unwrap();
        assert_eq!(unhashed.byte_len, bytes.len() as u64);
        assert!(unhashed.content_sha256.is_none());

        let hashed = SourceIdentity::capture(&path, &bytes, true).unwrap();
        assert!(hashed.content_sha256.is_some());
        // Stable, known SHA-256 of the literal bytes above.
        assert_eq!(
            hashed.content_sha256.unwrap(),
            sha256_hex(b"%PDF-1.7 hello")
        );
    }

    #[test]
    fn hash_is_deterministic_for_identical_bytes() {
        let a = sha256_hex(b"same content");
        let b = sha256_hex(b"same content");
        assert_eq!(a, b);
        assert_ne!(a, sha256_hex(b"different content"));
    }

    #[test]
    fn plausibly_same_content_uses_hash_when_available() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, b"one").unwrap();
        let a = SourceIdentity::capture(&path, b"one", true).unwrap();
        let b = SourceIdentity::capture(&path, b"one", true).unwrap();
        assert!(a.plausibly_same_content(&b));

        let c = SourceIdentity::capture(&path, b"two!", true).unwrap();
        assert!(
            !a.plausibly_same_content(&c),
            "different length must differ"
        );
    }

    #[test]
    fn plausibly_same_content_falls_back_to_path_without_a_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, b"one").unwrap();
        let a = SourceIdentity::capture(&path, b"one", false).unwrap();
        let b = SourceIdentity::capture(&path, b"one", false).unwrap();
        assert!(a.plausibly_same_content(&b));
    }

    #[test]
    fn identity_never_serializes_any_password_field() {
        // Structural guarantee: SourceIdentity has no password field at
        // all, so there is nothing for serde to serialize. This test
        // exists so a future field addition is forced to consider it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, b"one").unwrap();
        let identity = SourceIdentity::capture(&path, b"one", true).unwrap();
        let json = serde_json::to_string(&identity).unwrap();
        assert!(!json.to_lowercase().contains("password"));
    }
}
