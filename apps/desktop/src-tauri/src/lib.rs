//! Tauri backend for the M PDF Processor desktop application.
//!
//! This crate depends on `mpdf-core` (see the workspace root
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

use mpdf_core::pdfium_backend::pdfium_library_file_name;
use mpdf_core::ProjectInfo;
use serde::Serialize;
use tauri::Manager;

use state::AppState;

/// Resolves this build's trusted bundled PDFium library, if one was
/// actually staged into the app's resource directory at packaging time
/// (see `docs/pdfium-bundling.md` and `tauri.conf.json`'s `bundle.resources`).
///
/// Uses Tauri's own `BaseDirectory::Resource` resolution rather than
/// guessing a platform-specific bundle layout ourselves — on macOS this
/// correctly means `Contents/Resources/`, distinct from
/// `Contents/MacOS/`, where the generic executable-adjacent search in
/// `mpdf_core::pdfium_backend::resolve_library` looks. A
/// development run (no such resource staged) returns `None`, and every
/// caller falls back to the core resolver's existing search/override
/// behavior unchanged — see `worker::pdfium_config`.
fn resolve_bundled_pdfium_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    let resolved = app
        .path()
        .resolve(
            pdfium_library_file_name(),
            tauri::path::BaseDirectory::Resource,
        )
        .ok()?;
    resolved.is_file().then_some(resolved)
}

#[derive(Serialize)]
struct ProjectInfoPayload {
    name: String,
    phase: String,
}

/// Returns basic project information from `mpdf-core`. Kept
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
        .setup(|app| {
            let bundled_pdfium_path = resolve_bundled_pdfium_path(&app.handle().clone());
            app.manage(AppState::new(bundled_pdfium_path));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            project_info,
            commands::document::open_document,
            commands::document::close_document,
            commands::document::pdfium_status,
            commands::api::api_route_options,
            commands::api::api_credential_presence,
            commands::api::api_prepare_plan,
            commands::api::api_run_task,
            commands::api::api_cancel_current,
            commands::preview::render_preview,
            commands::estimate::start_estimate,
            commands::processing::start_processing,
            commands::processing::cancel_processing,
            commands::ocr::local_ocr_provider_status,
            commands::ocr::local_ocr_status,
            commands::ocr::local_ocr_cancel,
            commands::derived::load_review_queue,
            commands::derived::add_review_revision,
            commands::auto_bookmarks::start_auto_bookmark,
            commands::auto_bookmarks::cancel_auto_bookmark,
            commands::auto_bookmarks::load_bookmark_tree,
            commands::bookmarks::confirm_bookmark,
            commands::bookmarks::reject_bookmark,
            commands::bookmarks::edit_bookmark,
            commands::bookmarks::reparent_bookmark,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
