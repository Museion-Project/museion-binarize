//! `open_document` / `close_document`: the one-document-per-window
//! lifecycle described in `docs/desktop.md`.

use std::path::PathBuf;

use super::document_analysis::PaginationState;
use super::local_tools::LocalToolsState;
use tauri::{AppHandle, State};

use crate::dto::{DocumentSummaryDto, PdfiumStatusDto, UiErrorDto};
use crate::errors::{classify_core_error, request_error};
use crate::state::{AppState, OpenDocumentState, OperationKind};
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
    app: AppHandle,
    pagination: State<'_, PaginationState>,
    tools: State<'_, LocalToolsState>,
) -> Result<DocumentSummaryDto, UiErrorDto> {
    let _operation = state
        .try_claim_operation(OperationKind::Processing)
        .ok_or_else(|| {
            request_error(
                "operation_active",
                "a document operation is running; finish or cancel it before opening another document",
            )
        })?;

    let path_buf = PathBuf::from(&path);
    let password_protected_session = password.is_some();
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

    let doc = OpenDocumentState {
        document_id: document_id.clone(),
        input_path: canonical_path,
        page_count: opened.info.page_count,
        password_protected_session,
        source_sha256: opened.source_sha256,
    };
    *state.document.lock().unwrap() = Some(doc.clone());
    tools.clear_document();
    pagination.start(doc, app);

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
pub fn close_document(
    state: State<'_, AppState>,
    pagination: State<'_, PaginationState>,
    tools: State<'_, LocalToolsState>,
) -> Result<(), UiErrorDto> {
    let _operation = state
        .try_claim_operation(OperationKind::Processing)
        .ok_or_else(|| {
            request_error(
                "operation_active",
                "a document operation is running; finish or cancel it before closing the document",
            )
        })?;
    pagination.clear();
    tools.clear_document();
    state.worker.send(WorkerCommand::Close);
    *state.document.lock().unwrap() = None;
    Ok(())
}

/// Attempts to resolve a PDFium library without opening any document —
/// used by the GUI to show a clear status/error panel proactively rather
/// than only failing at the first `open_document` call. Never downloads
/// anything (see `docs/adr/0001-pdfium-runtime-binding.md`).
#[tauri::command]
pub fn pdfium_status(state: State<'_, AppState>) -> PdfiumStatusDto {
    let config = crate::worker::pdfium_config(state.bundled_pdfium_path.as_deref());
    match mpdf_core::pipeline::describe_pdfium_library(&config) {
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
