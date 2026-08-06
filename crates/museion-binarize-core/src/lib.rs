//! Processing core for Museion Binarize.
//!
//! This crate must never depend on Tauri or any GUI toolkit; see
//! `docs/architecture.md` in the workspace root for the rationale. At this
//! stage (Milestone 0) it contains only honest project scaffolding — no PDF
//! processing is implemented yet.

/// The human-readable project name.
pub const PROJECT_NAME: &str = "Museion Binarize";

/// Static information about the project, exposed to front ends (CLI,
/// desktop app) so they can display consistent identity without hardcoding
/// it themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInfo {
    pub name: &'static str,
    pub phase: &'static str,
}

impl ProjectInfo {
    /// Returns the current [`ProjectInfo`] for this build of the core.
    pub fn current() -> Self {
        Self {
            name: PROJECT_NAME,
            phase: "Phase 1 — under development",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_project_info_reports_expected_name_and_phase() {
        let info = ProjectInfo::current();
        assert_eq!(info.name, "Museion Binarize");
        assert_eq!(info.phase, "Phase 1 — under development");
    }
}
