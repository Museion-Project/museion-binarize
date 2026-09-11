//! Tauri backend for the M PDF Processor desktop application.
//!
//! This crate depends on `mpdf-core` (see the workspace root
//! `docs/architecture.md`) and exposes it to the frontend through Tauri
//! commands. It never duplicates a core algorithm, never shells out to
//! the CLI, and never sends the source PDF's bytes to the frontend — see
//! `docs/desktop.md` for the full architecture.

#[cfg(all(feature = "desktop-ui-test", debug_assertions))]
mod ui_test;

mod commands;
#[path = "local_dto.rs"]
mod dto;
mod errors;
mod local_tools;
mod settings;
#[path = "local_state.rs"]
mod state;
#[path = "local_worker.rs"]
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
    let builder = tauri::Builder::default();
    #[cfg(all(feature = "desktop-ui-test", debug_assertions))]
    let builder = ui_test::attach(builder);
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let bundled_pdfium_path = resolve_bundled_pdfium_path(&app.handle().clone());
            app.manage(AppState::new(bundled_pdfium_path));
            app.manage(commands::local_tools::LocalToolsState::default());
            app.manage(commands::document_analysis::PaginationState::default());
            Ok(())
        })
        // macOS local-tool release: body OCR, cloud/API and legacy workbench
        // commands are not registered, rather than merely hidden by the UI.
        .invoke_handler(tauri::generate_handler![
            project_info,
            commands::document_analysis::document_analysis_status,
            commands::local_tools::local_bookmark_readiness,
            commands::local_tools::local_hierarchy_models,
            commands::local_tools::generate_local_contents,
            commands::local_tools::save_local_pdf,
            commands::local_tools::prepare_local_binarization,
            commands::local_tools::cancel_local_tools,
            commands::document::open_document,
            commands::document::close_document,
            commands::document::pdfium_status,
            commands::preview::render_preview,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
