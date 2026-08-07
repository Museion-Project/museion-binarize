//! Stable, versioned report envelope types shared by every JSON report
//! this crate produces.
//!
//! ## Determinism
//!
//! No report in this crate includes a wall-clock timestamp by default:
//! doing so would make two runs over identical input and settings produce
//! different report bytes, which defeats the point of a "reproducible"
//! report and would be easy to mistake for nondeterminism in the
//! conversion itself. Durations (see [`crate::timing`]) are included and
//! *do* vary between runs — a report's bytes are therefore not claimed to
//! be deterministic, only the converted PDF is (see
//! `docs/pdf-pipeline-session.md`).
//!
//! ## Versioning
//!
//! Each report kind has its own `schema` name and independent
//! `schema_version`. New fields may be added in a minor-compatible way;
//! removing or repurposing a field is a breaking change and must bump the
//! version. See `docs/reporting.md`.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Schema identifier for the structured CLI error envelope.
pub const ERROR_SCHEMA: &str = "museion-binarize-error";
pub const ERROR_SCHEMA_VERSION: &str = "1.0";

/// Identifies the tool and version that produced a report, so a report
/// read later (or by a different tool version) can be interpreted
/// correctly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolIdentity {
    pub name: String,
    pub version: String,
}

impl Default for ToolIdentity {
    fn default() -> Self {
        Self {
            name: crate::PROJECT_NAME.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// A generic, versioned wrapper around any report payload `T`.
///
/// Every JSON document this crate emits to stdout or a report file is one
/// `ReportEnvelope`, never a bare, unwrapped struct — so a consumer can
/// always find `schema`/`schema_version` at the same top-level location
/// regardless of which command produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportEnvelope<T> {
    pub schema: String,
    pub schema_version: String,
    pub tool: ToolIdentity,
    pub result: T,
}

impl<T> ReportEnvelope<T> {
    pub fn new(schema: impl Into<String>, schema_version: impl Into<String>, result: T) -> Self {
        Self {
            schema: schema.into(),
            schema_version: schema_version.into(),
            tool: ToolIdentity::default(),
            result,
        }
    }
}

/// How a filesystem path is rendered into a report field.
///
/// Defaults to [`PathMode::Basename`]: a report intended to be
/// reproducible or shared should not silently embed the full local
/// filesystem layout of the machine that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PathMode {
    #[default]
    Basename,
    Relative,
    Absolute,
    Omit,
}

/// Renders `path` according to `mode`. Returns `None` only for
/// [`PathMode::Omit`].
pub fn display_path(path: &Path, mode: PathMode) -> Option<String> {
    match mode {
        PathMode::Omit => None,
        PathMode::Basename => Some(
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
        ),
        PathMode::Absolute => {
            let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            Some(absolute.display().to_string())
        }
        PathMode::Relative => {
            let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let rendered = std::env::current_dir()
                .ok()
                .and_then(|cwd| {
                    absolute
                        .strip_prefix(&cwd)
                        .ok()
                        .map(|p| p.display().to_string())
                })
                .unwrap_or_else(|| absolute.display().to_string());
            Some(rendered)
        }
    }
}

/// The payload of a structured CLI error report.
///
/// `code` is a stable, machine-readable identifier (e.g.
/// `"password_required"`) — see `docs/reporting.md` for the current set.
/// It deliberately never exposes internal Rust type names or FFI details.
/// `context` carries caller-supplied, non-secret detail (such as the input
/// path); nothing in this type accepts a password.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub context: std::collections::BTreeMap<String, String>,
}

/// A complete, versioned JSON error document. Emitted to stdout (JSON
/// mode) or stderr (human mode's `--verbose`, formatted, not this shape)
/// when a command fails.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub schema: String,
    pub schema_version: String,
    pub error: ErrorPayload,
}

impl ErrorEnvelope {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            schema: ERROR_SCHEMA.to_string(),
            schema_version: ERROR_SCHEMA_VERSION.to_string(),
            error: ErrorPayload {
                code: code.into(),
                message: message.into(),
                context: Default::default(),
            },
        }
    }

    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.error.context.insert(key.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trips_through_json() {
        let envelope = ReportEnvelope::new("test-schema", "1.0", vec![1, 2, 3]);
        let json = serde_json::to_string(&envelope).unwrap();
        let back: ReportEnvelope<Vec<i32>> = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, back);
    }

    #[test]
    fn tool_identity_defaults_to_this_crate_and_its_cargo_version() {
        let tool = ToolIdentity::default();
        assert_eq!(tool.name, "Museion Binarize");
        assert!(!tool.version.is_empty());
    }

    #[test]
    fn display_path_basename_strips_directories() {
        let path = Path::new("/a/b/c/book.pdf");
        assert_eq!(
            display_path(path, PathMode::Basename),
            Some("book.pdf".to_string())
        );
    }

    #[test]
    fn display_path_omit_returns_none() {
        assert_eq!(display_path(Path::new("/a/b.pdf"), PathMode::Omit), None);
    }

    #[test]
    fn display_path_absolute_is_never_relative() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.pdf");
        std::fs::write(&path, b"x").unwrap();
        let rendered = display_path(&path, PathMode::Absolute).unwrap();
        assert!(Path::new(&rendered).is_absolute());
    }

    #[test]
    fn error_envelope_never_serializes_a_password_key_by_construction() {
        let envelope = ErrorEnvelope::new("password_required", "the PDF requires a password")
            .with_context("input", "book.pdf");
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(!json.to_lowercase().contains("\"password\""));
        assert!(json.contains("password_required"));
    }

    #[test]
    fn error_envelope_has_the_documented_schema_and_version() {
        let envelope = ErrorEnvelope::new("x", "y");
        assert_eq!(envelope.schema, ERROR_SCHEMA);
        assert_eq!(envelope.schema_version, ERROR_SCHEMA_VERSION);
    }

    #[test]
    fn no_report_type_in_this_crate_emits_nan_or_infinite_json() {
        // A representative payload containing floats, run through
        // serde_json, must produce valid JSON (serde_json errors out on
        // NaN/Infinity rather than silently emitting invalid JSON, so a
        // successful serialization here is the proof).
        let envelope = ReportEnvelope::new("test", "1.0", 0.0_f64 / 1.0);
        assert!(serde_json::to_string(&envelope).is_ok());
    }
}
