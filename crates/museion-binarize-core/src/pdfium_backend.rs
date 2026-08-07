//! The PDFium boundary.
//!
//! Everything that knows PDFium exists lives behind this module and
//! [`crate::renderer`]. No `pdfium-render` type appears in any public
//! signature outside these two modules.
//!
//! PDFium is *not* bundled with `pdfium-render` and is never downloaded at
//! runtime by this application (see `docs/adr/0001-pdfium-runtime-binding.md`).
//! The library must already exist on disk; this module only decides which
//! copy to load.

use std::path::PathBuf;

use crate::error::{CoreError, Result};

/// Environment variable naming an explicit PDFium library path.
pub const PDFIUM_LIBRARY_ENV: &str = "MUSEION_PDFIUM_LIBRARY";

/// Platform-specific PDFium dynamic library file name.
pub const fn pdfium_library_file_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "libpdfium.dylib"
    }
    #[cfg(target_os = "windows")]
    {
        "pdfium.dll"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "libpdfium.so"
    }
}

/// How the PDFium library should be located.
#[derive(Debug, Clone, Default)]
pub struct PdfiumConfig {
    /// An explicit path supplied by the caller (CLI flag, GUI setting).
    /// When set, no other candidate is tried — an explicit path that
    /// fails is an error, never a silent fallback to a different binary.
    pub library_path: Option<PathBuf>,
    /// Whether the operating system's default search path may be used as
    /// a last resort. Off by default: loading an arbitrary system PDFium
    /// of unknown provenance should be a deliberate choice.
    pub allow_system_library: bool,
}

/// Which category of location a library was ultimately loaded from.
/// Reported to the user so it is always clear *which* binary ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibrarySource {
    ExplicitPath,
    Environment,
    BundledResource,
    ExecutableAdjacent,
    SystemLibrary,
}

impl LibrarySource {
    pub fn describe(self) -> &'static str {
        match self {
            LibrarySource::ExplicitPath => "explicit path",
            LibrarySource::Environment => "MUSEION_PDFIUM_LIBRARY environment variable",
            LibrarySource::BundledResource => "bundled application resource",
            LibrarySource::ExecutableAdjacent => "directory containing the executable",
            LibrarySource::SystemLibrary => "system library search path",
        }
    }
}

/// A resolved PDFium library location.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLibrary {
    pub source: LibrarySource,
    /// `None` for [`LibrarySource::SystemLibrary`], where the operating
    /// system performs the lookup and no concrete path is known up front.
    pub path: Option<PathBuf>,
}

/// Resolves which PDFium library to load, in the documented precedence
/// order:
///
/// 1. explicit path from [`PdfiumConfig::library_path`];
/// 2. the `MUSEION_PDFIUM_LIBRARY` environment variable;
/// 3. an application-relative bundled resource directory;
/// 4. a library sitting next to the running executable;
/// 5. the system library path, only when explicitly allowed.
///
/// Only steps 3-5 are "search" steps. If an explicit path (step 1 or 2) is
/// given but does not exist, resolution fails rather than quietly loading
/// some other copy.
pub fn resolve_library(config: &PdfiumConfig) -> Result<ResolvedLibrary> {
    let file_name = pdfium_library_file_name();
    let mut attempted: Vec<String> = Vec::new();

    if let Some(explicit) = &config.library_path {
        return if explicit.is_file() {
            Ok(ResolvedLibrary {
                source: LibrarySource::ExplicitPath,
                path: Some(explicit.clone()),
            })
        } else {
            Err(CoreError::PdfiumNotFound {
                attempted: vec![format!("{} (explicit path)", explicit.display())],
            })
        };
    }

    if let Some(from_env) = std::env::var_os(PDFIUM_LIBRARY_ENV) {
        let candidate = PathBuf::from(from_env);
        return if candidate.is_file() {
            Ok(ResolvedLibrary {
                source: LibrarySource::Environment,
                path: Some(candidate),
            })
        } else {
            Err(CoreError::PdfiumNotFound {
                attempted: vec![format!(
                    "{} (from {PDFIUM_LIBRARY_ENV})",
                    candidate.display()
                )],
            })
        };
    }

    for (source, candidate) in search_candidates(file_name) {
        if candidate.is_file() {
            return Ok(ResolvedLibrary {
                source,
                path: Some(candidate),
            });
        }
        attempted.push(candidate.display().to_string());
    }

    if config.allow_system_library {
        return Ok(ResolvedLibrary {
            source: LibrarySource::SystemLibrary,
            path: None,
        });
    }
    attempted.push(format!(
        "system library path (not searched; enable it explicitly to allow {file_name} of \
         unknown provenance)"
    ));

    Err(CoreError::PdfiumNotFound { attempted })
}

/// Non-explicit candidate locations, in precedence order.
fn search_candidates(file_name: &str) -> Vec<(LibrarySource, PathBuf)> {
    let mut candidates = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Tauri-style bundled resources sit in a `resources`
            // subdirectory next to the executable.
            candidates.push((
                LibrarySource::BundledResource,
                dir.join("resources").join(file_name),
            ));
            candidates.push((LibrarySource::ExecutableAdjacent, dir.join(file_name)));
        }
    }

    // The gitignored development location populated per docs/pdfium.md.
    // Only consulted when running out of a build tree.
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push((
            LibrarySource::BundledResource,
            cwd.join("target")
                .join("pdfium")
                .join(current_target_triple())
                .join(file_name),
        ));
    }

    candidates
}

/// Best-effort target triple for the development library directory.
const fn current_target_triple() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    {
        "unknown"
    }
}

/// A process-wide PDFium session.
///
/// PDFium's own `FPDF_InitLibrary`/`FPDF_DestroyLibrary` are **global to
/// the process**: binding a second time returns
/// `PdfiumLibraryBindingsAlreadyInitialized`, and dropping one binding
/// while another is in use tears the library out from under it. The
/// library therefore has to be initialized exactly once per process and
/// shared.
///
/// This is a process-wide *immutable* singleton, not global mutable
/// configuration: it is written once, never mutated afterwards, and all
/// caller-facing configuration still travels explicitly through
/// [`PdfiumConfig`]. If a caller later asks for a *different* explicit
/// library than the one already bound, that is reported as an error
/// rather than silently satisfied with the wrong binary.
pub(crate) struct PdfiumSession {
    pub(crate) pdfium: pdfium_render::prelude::Pdfium,
    pub(crate) resolved: ResolvedLibrary,
}

static SESSION: std::sync::OnceLock<PdfiumSession> = std::sync::OnceLock::new();
static INIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Returns the process-wide PDFium session, initializing it on first use.
pub(crate) fn session(config: &PdfiumConfig) -> Result<&'static PdfiumSession> {
    if let Some(existing) = SESSION.get() {
        return check_compatible(existing, config);
    }

    // Serialize initialization so two threads cannot both construct a
    // `Pdfium` (the loser's drop would destroy the winner's library).
    let _guard = INIT_LOCK.lock().map_err(|_| CoreError::PdfiumLoadFailed {
        path: PathBuf::from(pdfium_library_file_name()),
        reason: "the PDFium initialization lock was poisoned".to_string(),
    })?;
    if let Some(existing) = SESSION.get() {
        return check_compatible(existing, config);
    }

    let resolved = resolve_library(config)?;
    let bindings = match &resolved.path {
        Some(path) => pdfium_render::prelude::Pdfium::bind_to_library(path).map_err(|e| {
            CoreError::PdfiumLoadFailed {
                path: path.clone(),
                reason: e.to_string(),
            }
        })?,
        None => pdfium_render::prelude::Pdfium::bind_to_system_library().map_err(|e| {
            CoreError::PdfiumLoadFailed {
                path: PathBuf::from(pdfium_library_file_name()),
                reason: e.to_string(),
            }
        })?,
    };

    let session = PdfiumSession {
        pdfium: pdfium_render::prelude::Pdfium::new(bindings),
        resolved,
    };
    // `set` cannot fail here: we hold the init lock and checked `get`.
    let _ = SESSION.set(session);
    SESSION.get().ok_or_else(|| CoreError::PdfiumLoadFailed {
        path: PathBuf::from(pdfium_library_file_name()),
        reason: "the PDFium session disappeared immediately after initialization".to_string(),
    })
}

/// Rejects a request for a different explicit library than the one this
/// process already bound, instead of silently using the wrong binary.
fn check_compatible(
    session: &'static PdfiumSession,
    config: &PdfiumConfig,
) -> Result<&'static PdfiumSession> {
    if let Some(requested) = &config.library_path {
        let already = session.resolved.path.as_deref();
        let same = match already {
            Some(bound) => paths_equal(bound, requested),
            None => false,
        };
        if !same {
            return Err(CoreError::PdfiumLoadFailed {
                path: requested.clone(),
                reason: format!(
                    "this process already bound a different PDFium library ({}); \
                     PDFium can only be initialized once per process",
                    already
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "system library".to_string())
                ),
            });
        }
    }
    Ok(session)
}

fn paths_equal(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Describes a resolved library for user-facing output, without leaking
/// more of the user's filesystem layout than they supplied themselves.
pub fn describe_resolved(resolved: &ResolvedLibrary) -> String {
    match &resolved.path {
        Some(path) => format!("{} ({})", path.display(), resolved.source.describe()),
        None => resolved.source.describe().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_library_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(pdfium_library_file_name());
        std::fs::write(&path, b"not a real library").expect("write");
        (dir, path)
    }

    #[test]
    fn explicit_path_takes_precedence_and_is_reported_as_such() {
        let (_dir, path) = temp_library_file();
        let config = PdfiumConfig {
            library_path: Some(path.clone()),
            allow_system_library: true,
        };
        let resolved = resolve_library(&config).expect("should resolve");
        assert_eq!(resolved.source, LibrarySource::ExplicitPath);
        assert_eq!(resolved.path.unwrap(), path);
    }

    #[test]
    fn a_missing_explicit_path_fails_instead_of_falling_back() {
        // Even with the system library allowed, an explicit path that does
        // not exist must be an error — never a silent switch to a
        // different binary.
        let config = PdfiumConfig {
            library_path: Some(PathBuf::from("/nonexistent/libpdfium.dylib")),
            allow_system_library: true,
        };
        let err = resolve_library(&config).unwrap_err();
        match err {
            CoreError::PdfiumNotFound { attempted } => {
                assert_eq!(attempted.len(), 1);
                assert!(attempted[0].contains("/nonexistent/"));
                assert!(attempted[0].contains("explicit path"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn error_lists_every_attempted_location() {
        let config = PdfiumConfig {
            library_path: None,
            allow_system_library: false,
        };
        // With no env var and (almost certainly) no library in the search
        // locations, this reports what it tried. If a developer library
        // does happen to exist locally, resolution legitimately succeeds.
        if std::env::var_os(PDFIUM_LIBRARY_ENV).is_some() {
            return;
        }
        match resolve_library(&config) {
            Err(CoreError::PdfiumNotFound { attempted }) => {
                assert!(!attempted.is_empty());
                assert!(attempted.iter().any(|a| a.contains("system library path")));
            }
            Ok(resolved) => {
                // A real development library is present; that is a valid
                // outcome and still must name a concrete path.
                assert!(resolved.path.is_some());
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn system_library_is_only_used_when_explicitly_allowed() {
        if std::env::var_os(PDFIUM_LIBRARY_ENV).is_some() {
            return;
        }
        let denied = PdfiumConfig {
            library_path: None,
            allow_system_library: false,
        };
        // Either it found a real development library, or it refused
        // without ever reporting SystemLibrary.
        if let Ok(resolved) = resolve_library(&denied) {
            assert_ne!(resolved.source, LibrarySource::SystemLibrary);
        }
    }

    #[test]
    fn library_file_name_matches_the_host_platform() {
        let name = pdfium_library_file_name();
        #[cfg(target_os = "macos")]
        assert_eq!(name, "libpdfium.dylib");
        #[cfg(target_os = "windows")]
        assert_eq!(name, "pdfium.dll");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(name, "libpdfium.so");
    }

    #[test]
    fn describe_resolved_names_the_source_category() {
        let resolved = ResolvedLibrary {
            source: LibrarySource::ExplicitPath,
            path: Some(PathBuf::from("/x/libpdfium.dylib")),
        };
        let described = describe_resolved(&resolved);
        assert!(described.contains("/x/libpdfium.dylib"));
        assert!(described.contains("explicit path"));

        let system = ResolvedLibrary {
            source: LibrarySource::SystemLibrary,
            path: None,
        };
        assert_eq!(describe_resolved(&system), "system library search path");
    }
}
