//! `open_document` / `close_document`: the one-document-per-window
//! lifecycle described in `docs/desktop.md`.

use std::path::PathBuf;

use tauri::State;

use crate::dto::{DocumentSummaryDto, PdfiumStatusDto, UiErrorDto};
use crate::errors::{classify_core_error, request_error};
use crate::state::{AppState, OpenDocumentState};
use crate::worker::WorkerCommand;

/// Opens `path` as the window's one active document, replacing any
/// previously open document. Rejected while a processing job is running —
/// replacing the session out from under an in-flight conversion would
/// silently corrupt it (see `docs/desktop.md`, "Open-document
/// lifecycle").
#[tauri::command]
pub async fn open_document(
    path: String,
    password: Option<String>,
    state: State<'_, AppState>,
) -> Result<DocumentSummaryDto, UiErrorDto> {
    if state.job.lock().unwrap().is_some() {
        return Err(request_error(
            "job_active",
            "a processing job is running; cancel it before opening another document",
        ));
    }

    let path_buf = PathBuf::from(&path);
    let file_name = path_buf
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());

    let opened = state
        .worker
        .call(|reply| WorkerCommand::Open {
            path: path_buf.clone(),
            password,
            reply,
        })
        .await
        .map_err(|e| classify_core_error(&e))?;

    let document_id = state.new_id("doc");
    let canonical_path = std::fs::canonicalize(&path_buf).unwrap_or(path_buf);

    *state.document.lock().unwrap() = Some(OpenDocumentState {
        document_id: document_id.clone(),
        file_name: file_name.clone(),
        input_path: canonical_path,
        page_count: opened.info.page_count,
    });

    Ok(DocumentSummaryDto::build(
        document_id,
        file_name,
        &opened.info,
        opened.pdfium_library,
    ))
}

/// Closes the currently open document, if any. Rejected while a
/// processing job is running, for the same reason as `open_document`.
#[tauri::command]
pub fn close_document(state: State<'_, AppState>) -> Result<(), UiErrorDto> {
    if state.job.lock().unwrap().is_some() {
        return Err(request_error(
            "job_active",
            "a processing job is running; cancel it before closing the document",
        ));
    }
    state.worker.send(WorkerCommand::Close);
    *state.document.lock().unwrap() = None;
    Ok(())
}

/// Attempts to resolve a PDFium library without opening any document —
/// used by the GUI to show a clear status/error panel proactively rather
/// than only failing at the first `open_document` call. Never downloads
/// anything (see `docs/adr/0001-pdfium-runtime-binding.md`).
#[tauri::command]
pub fn pdfium_status() -> PdfiumStatusDto {
    match museion_binarize_core::pipeline::describe_pdfium_library(
        &museion_binarize_core::pdfium_backend::PdfiumConfig::default(),
    ) {
        Ok(description) => PdfiumStatusDto {
            resolved: true,
            description: Some(description),
            error: None,
        },
        Err(e) => PdfiumStatusDto {
            resolved: false,
            description: None,
            error: Some(classify_core_error(&e)),
        },
    }
}
