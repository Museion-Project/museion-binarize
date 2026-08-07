//! Tauri backend for the Museion Binarize desktop application.
//!
//! This crate depends on `museion-binarize-core` (see the workspace root
//! `docs/architecture.md`) and exposes it to the frontend through Tauri
//! commands. It never duplicates a core algorithm, never shells out to
//! the CLI, and never sends the source PDF's bytes to the frontend — see
//! `docs/desktop.md` for the full architecture.

mod commands;
mod dto;
mod errors;
mod settings;
mod state;
mod worker;

use museion_binarize_core::ProjectInfo;
use serde::Serialize;

use state::AppState;

#[derive(Serialize)]
struct ProjectInfoPayload {
    name: String,
    phase: String,
}

/// Returns basic project information from `museion-binarize-core`. Kept
/// from Milestone 0 as a minimal, dependency-free bridge check.
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
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            project_info,
            commands::document::open_document,
            commands::document::close_document,
            commands::document::pdfium_status,
            commands::preview::render_preview,
            commands::estimate::start_estimate,
            commands::processing::start_processing,
            commands::processing::cancel_processing,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
