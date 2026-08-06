//! Tauri backend for the Museion Binarize desktop application.
//!
//! This backend depends on `museion-binarize-core` (see the workspace root
//! `docs/architecture.md`) and exposes it to the frontend through Tauri
//! commands. At this stage (Milestone 0) it exposes only project metadata,
//! to verify the frontend/backend bridge — no PDF processing is wired up.

use museion_binarize_core::ProjectInfo;
use serde::Serialize;

#[derive(Serialize)]
struct ProjectInfoPayload {
    name: String,
    phase: String,
}

/// Returns basic project information from `museion-binarize-core`, proving
/// that the desktop frontend can call into the shared processing core
/// through the Tauri IPC bridge.
#[tauri::command]
fn project_info() -> ProjectInfoPayload {
    let info = ProjectInfo::current();
    ProjectInfoPayload {
        name: info.name.to_string(),
        phase: info.phase.to_string(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![project_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
