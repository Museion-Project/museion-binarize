//! Document-scoped commands for the unified black-and-white / contents UI.
use crate::{
    dto::{ProcessingSettingsDto, UiErrorDto},
    errors::request_error,
    local_tools::{self, BookmarkSession, EntryEdit, Notify, Runtime, SavePlan},
    settings::to_processing_settings,
    state::{AppState, OpenDocumentState, OperationKind},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone)]
struct Active {
    id: String,
    cancel: Arc<AtomicBool>,
}
#[derive(Clone, Default)]
pub struct LocalToolsState {
    session: Arc<Mutex<Option<Arc<BookmarkSession>>>>,
    prepared: Arc<Mutex<Option<Arc<local_tools::PreparedBinarization>>>>,
    active: Arc<Mutex<Option<Active>>>,
}
struct JobGuard {
    id: String,
    active: Arc<Mutex<Option<Active>>>,
}
impl Drop for JobGuard {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.active.lock() {
            if slot.as_ref().is_some_and(|a| a.id == self.id) {
                slot.take();
            }
        }
    }
}
impl LocalToolsState {
    pub fn clear_document(&self) {
        self.session.lock().unwrap().take();
        self.prepared.lock().unwrap().take();
    }

    fn begin(&self, id: &str) -> Result<(Arc<AtomicBool>, JobGuard), UiErrorDto> {
        if id.is_empty()
            || id.len() > 100
            || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(request_error("invalid_request", "无效的操作标识。"));
        }
        let mut slot = self.active.lock().unwrap();
        if slot.is_some() {
            return Err(request_error("operation_active", "请等待当前操作完成。"));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *slot = Some(Active {
            id: id.into(),
            cancel: cancel.clone(),
        });
        Ok((
            cancel,
            JobGuard {
                id: id.into(),
                active: self.active.clone(),
            },
        ))
    }
}
fn current(state: &AppState, id: &str) -> Result<OpenDocumentState, UiErrorDto> {
    state
        .document
        .lock()
        .unwrap()
        .as_ref()
        .filter(|d| d.document_id == id)
        .cloned()
        .ok_or_else(|| request_error("document_stale", "此 PDF 已关闭，请重新打开。"))
}
fn map_error(error: String) -> UiErrorDto {
    let code = if error.contains("operation_cancelled") || error == "processing cancelled" {
        "cancelled"
    } else {
        "local_tools_failed"
    };
    request_error(code, error)
}
fn notify(app: AppHandle, document_id: String, operation_id: String) -> Notify {
    Arc::new(move |stage, page, count| {
        let _=app.emit("mpdf://local-tools-progress",json!({"documentId":document_id,"operationId":operation_id,"stage":stage,"pageNumber":page,"pageCount":count}));
    })
}
fn locations(
    app: &AppHandle,
) -> Result<(Option<std::path::PathBuf>, std::path::PathBuf), UiErrorDto> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|e| request_error("runtime_unavailable", e.to_string()))?
        .join("local-contents");
    Ok((app.path().resource_dir().ok(), cache))
}
#[tauri::command]
pub async fn local_bookmark_readiness(app: AppHandle) -> Result<Value, UiErrorDto> {
    let (resources, cache) = locations(&app)?;
    tauri::async_runtime::spawn_blocking(move ||match Runtime::resolve(resources.as_deref(),cache){
        Ok(_)=>json!({"available":true,"imageRecognitionAvailable":cfg!(target_os="macos"),"message":null}),
        Err(e)=>json!({"available":false,"imageRecognitionAvailable":false,"message":e})
    }).await.map_err(|e|request_error("runtime_unavailable",e.to_string()))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerateRequest {
    document_id: String,
    operation_id: String,
    pages: Vec<u32>,
    mode: String,
    #[serde(default)]
    hierarchy_provider: Option<String>,
}
#[tauri::command]
pub async fn generate_local_contents(
    request: GenerateRequest,
    app: AppHandle,
    state: State<'_, AppState>,
    tools: State<'_, LocalToolsState>,
    pagination: State<'_, super::document_analysis::PaginationState>,
) -> Result<Value, UiErrorDto> {
    if request
        .hierarchy_provider
        .as_deref()
        .is_some_and(|p| p != "apple")
    {
        return Err(request_error("invalid_request", "目录层级仅支持 Apple。"));
    }
    let operation = state
        .try_claim_operation(OperationKind::AutoBookmark)
        .ok_or_else(|| request_error("operation_active", "请等待当前文档操作完成。"))?;
    let doc = current(&state, &request.document_id)?;
    let (cancel, guard) = tools.begin(&request.operation_id)?;
    let session_id = state.new_id("local-contents");
    let (resources, cache) = locations(&app)?;
    let progress = notify(app, doc.document_id.clone(), request.operation_id);
    let store = tools.inner().clone();
    let worker = state.worker.clone();
    let pagination = pagination.inner().clone();
    tauri::async_runtime::spawn_blocking(move ||{
        let _operation=operation;let _guard=guard;
        let runtime=Runtime::resolve(resources.as_deref(),cache)?;
        progress("mapping_pages",None,None);
        let map=pagination.wait_result(&doc.document_id,&cancel);
        let mut session=local_tools::generate_with_pagination(&runtime,&worker,&doc,&request.pages,&request.mode,local_tools::LocalJob { id: session_id, cancel: cancel.clone(), notify: progress.clone() },map.as_ref().map(|m|m.path.as_path()))?;
        if let Some(provider) = request.hierarchy_provider {
            let result = runtime.hierarchy_runtime().run("hierarchy",session.work.path(),json!({"provider":provider,"model_root":runtime.model_root()}),&cancel,&progress)?;
            session.table=result["table"].clone();session.intake=result["intake"].clone();
        }
        let session=Arc::new(session);
        let response=json!({"documentId":doc.document_id,"sessionId":session.id,"table":session.table,"intake":session.intake});
        *store.session.lock().unwrap()=Some(session);Ok(response)
    }).await.map_err(|e|request_error("internal_error",e.to_string()))?.map_err(map_error)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareRequest {
    document_id: String,
    operation_id: String,
    settings: ProcessingSettingsDto,
    binarize_pages: Option<Vec<u32>>,
}
#[tauri::command]
pub async fn prepare_local_binarization(
    request: PrepareRequest,
    app: AppHandle,
    state: State<'_, AppState>,
    tools: State<'_, LocalToolsState>,
) -> Result<Value, UiErrorDto> {
    let operation = state
        .try_claim_operation(OperationKind::Processing)
        .ok_or_else(|| request_error("operation_active", "请等待当前操作完成。"))?;
    let doc = current(&state, &request.document_id)?;
    let settings = to_processing_settings(&request.settings)
        .map_err(|e| request_error("invalid_settings", e))?;
    let (cancel, guard) = tools.begin(&request.operation_id)?;
    let progress = notify(app.clone(), doc.document_id.clone(), request.operation_id);
    let (_, cache) = locations(&app)?;
    let worker = state.worker.clone();
    let store = tools.inner().clone();
    let id = state.new_id("binarized");
    tauri::async_runtime::spawn_blocking(move ||{
        let _operation=operation;let _guard=guard;
        let prepared=Arc::new(local_tools::prepare_binarization(&worker,&doc,&cache,settings,request.binarize_pages.as_deref(),local_tools::LocalJob { id, cancel, notify: progress })?);
        let value=json!({"documentId":doc.document_id,"preparedId":prepared.id,"pagesProcessed":prepared.report.pages_processed,"elapsedSeconds":prepared.report.elapsed_us as f64/1_000_000.0});
        *store.prepared.lock().unwrap()=Some(prepared);Ok(value)
    }).await.map_err(|e|request_error("internal_error",e.to_string()))?.map_err(map_error)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveRequest {
    document_id: String,
    operation_id: String,
    output_path: String,
    binarize: bool,
    binarize_pages: Option<Vec<u32>>,
    prepared_id: Option<String>,
    settings: ProcessingSettingsDto,
    bookmark_session_id: Option<String>,
    entries: Vec<EntryEdit>,
    projection: String,
    review_accepted: bool,
}
#[tauri::command]
pub async fn save_local_pdf(
    request: SaveRequest,
    app: AppHandle,
    state: State<'_, AppState>,
    tools: State<'_, LocalToolsState>,
) -> Result<Value, UiErrorDto> {
    let operation = state
        .try_claim_operation(OperationKind::Processing)
        .ok_or_else(|| request_error("operation_active", "请等待当前文档操作完成。"))?;
    let doc = current(&state, &request.document_id)?;
    let session = if let Some(id) = request.bookmark_session_id {
        Some(
            tools
                .session
                .lock()
                .unwrap()
                .as_ref()
                .filter(|s| s.id == id && s.document_id == doc.document_id)
                .cloned()
                .ok_or_else(|| request_error("document_stale", "目录结果已失效，请重新生成。"))?,
        )
    } else {
        None
    };
    let settings = if request.binarize {
        Some(
            to_processing_settings(&request.settings)
                .map_err(|e| request_error("invalid_settings", e))?,
        )
    } else {
        None
    };
    let prepared = if request.binarize {
        Some(
            tools
                .prepared
                .lock()
                .unwrap()
                .as_ref()
                .filter(|p| {
                    Some(p.id.as_str()) == request.prepared_id.as_deref()
                        && p.document_id == doc.document_id
                })
                .cloned()
                .ok_or_else(|| {
                    request_error("binarization_required", "请先点击黑白处理的开始。")
                })?,
        )
    } else {
        None
    };
    let (cancel, guard) = tools.begin(&request.operation_id)?;
    let progress = notify(app.clone(), doc.document_id.clone(), request.operation_id);
    let (resources, cache) = locations(&app)?;
    let worker = state.worker.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        let _guard = guard;
        let runtime = if session.is_some() {
            Some(Runtime::resolve(resources.as_deref(), cache)?)
        } else {
            None
        };
        local_tools::save_pdf(
            runtime.as_ref(),
            &worker,
            &doc,
            SavePlan {
                output: request.output_path.into(),
                settings,
                binarize_pages: request.binarize_pages,
                prepared,
                session,
                entries: request.entries,
                review_accepted: request.review_accepted,
                projection: request.projection,
            },
            cancel,
            progress,
        )
    })
    .await
    .map_err(|e| request_error("internal_error", e.to_string()))?
    .map_err(map_error)
}
#[tauri::command]
pub fn cancel_local_tools(
    operation_id: String,
    tools: State<'_, LocalToolsState>,
) -> Result<(), UiErrorDto> {
    let active = tools.active.lock().unwrap();
    if let Some(a) = active.as_ref().filter(|a| a.id == operation_id) {
        a.cancel.store(true, Ordering::SeqCst);
        Ok(())
    } else {
        Err(request_error("job_not_found", "操作已结束。"))
    }
}

#[tauri::command]
pub async fn local_hierarchy_models(app: AppHandle) -> Result<Value, UiErrorDto> {
    let (resources, cache) = locations(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let rt = Runtime::resolve(resources.as_deref(), cache)?;
        let work = tempfile::Builder::new()
            .prefix("models-status-")
            .tempdir_in(&rt.cache)
            .map_err(|e| e.to_string())?;
        rt.hierarchy_runtime().run(
            "models_status",
            work.path(),
            json!({"model_root":rt.model_root()}),
            &AtomicBool::new(false),
            &(Arc::new(|_: &str, _: Option<u32>, _: Option<u32>| {}) as Notify),
        )
    })
    .await
    .map_err(|e| request_error("internal_error", e.to_string()))?
    .map_err(map_error)
}
