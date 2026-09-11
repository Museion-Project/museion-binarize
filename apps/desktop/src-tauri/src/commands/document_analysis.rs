//! Short-lived document-open analysis, independent of the PDFium preview queue.
use crate::{
    dto::UiErrorDto,
    errors::request_error,
    local_tools::{Notify, Runtime},
    state::{AppState, OpenDocumentState},
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State};
use tempfile::TempDir;

pub struct PaginationResult {
    pub path: PathBuf,
    pub model: Value,
    _work: TempDir,
}
struct AnalysisEntry {
    document_id: String,
    cancel: Arc<AtomicBool>,
    status: Value,
    result: Option<Arc<PaginationResult>>,
}
#[derive(Clone, Default)]
pub struct PaginationState {
    entry: Arc<Mutex<Option<AnalysisEntry>>>,
}
impl PaginationState {
    pub fn clear(&self) {
        if let Some(entry) = self.entry.lock().unwrap().take() {
            entry.cancel.store(true, Ordering::SeqCst);
        }
    }
    fn update(&self, id: &str, update: impl FnOnce(&mut AnalysisEntry)) {
        if let Some(entry) = self
            .entry
            .lock()
            .unwrap()
            .as_mut()
            .filter(|e| e.document_id == id)
        {
            update(entry);
        }
    }
    pub fn start(&self, doc: OpenDocumentState, app: AppHandle) {
        self.clear();
        let cancel = Arc::new(AtomicBool::new(false));
        *self.entry.lock().unwrap() = Some(AnalysisEntry {
            document_id: doc.document_id.clone(),
            cancel: cancel.clone(),
            result: None,
            status: json!({"documentId":doc.document_id,"textLayer":"checking","paginationStatus":"running","sequenceCount":0,"sampledPages":0,"message":null}),
        });
        let store = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = (|| -> Result<PaginationResult, String> {
                if doc.password_protected_session {
                    return Err("加密 PDF 暂不支持自动重建页码。".into());
                }
                let cache = app
                    .path()
                    .app_cache_dir()
                    .map_err(|e| e.to_string())?
                    .join("local-contents");
                let rt = Runtime::resolve(app.path().resource_dir().ok().as_deref(), cache)?;
                if cancel.load(Ordering::SeqCst) {
                    return Err("operation_cancelled".into());
                }
                let work = tempfile::Builder::new()
                    .prefix("pagination-")
                    .tempdir_in(&rt.cache)
                    .map_err(|e| e.to_string())?;
                let notify: Notify = Arc::new(|_, _, _| {});
                let model=rt.run_observed("analyze",work.path(),json!({"source":doc.input_path,"source_sha256":doc.source_sha256,"runtime_cache":rt.cache.join("runtime")}),&cancel,&notify,&|value|{
                    store.update(&doc.document_id,|e|{
                        if let Some(layer)=value.get("text_layer"){e.status["textLayer"]=layer.clone();}
                        // Terminal status is published atomically with the cached result below.
                        if value.get("pagination_status")==Some(&json!("running")){e.status["paginationStatus"]=json!("running");}
                        if let Some(count)=value.get("sampled_pages"){e.status["sampledPages"]=count.clone();}
                    });
                })?;
                Ok(PaginationResult {
                    path: work.path().join("pagination.json"),
                    model,
                    _work: work,
                })
            })();
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            store.update(&doc.document_id,|entry|match result {
                Ok(result)=>{
                    entry.status=json!({"documentId":doc.document_id,"textLayer":result.model["text_layer"],"paginationStatus":result.model["status"],"sequenceCount":result.model["sequence_count"],"sampledPages":result.model["sampled_pages"].as_array().map(Vec::len).unwrap_or(0),"elapsedSeconds":result.model["elapsed_seconds"],"message":null});
                    entry.result=Some(Arc::new(result));
                }
                Err(message)=>{entry.status["paginationStatus"]=json!("failed");if entry.status["textLayer"]=="checking"{entry.status["textLayer"]=json!("unavailable");}entry.status["message"]=json!(message);}
            });
        });
    }
    /// A user can click Generate before the background pass completes. Wait
    /// only here, not on the UI thread or the serialized rendering worker.
    pub fn wait_result(&self, id: &str, cancel: &AtomicBool) -> Option<Arc<PaginationResult>> {
        let began = Instant::now();
        loop {
            if cancel.load(Ordering::SeqCst) {
                return None;
            }
            {
                let guard = self.entry.lock().unwrap();
                let entry = guard.as_ref().filter(|e| e.document_id == id)?;
                if let Some(result) = &entry.result {
                    return Some(result.clone());
                }
                if entry.status["paginationStatus"] != "running" {
                    return None;
                }
            }
            if began.elapsed() > Duration::from_secs(30) {
                return None;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }
}
#[tauri::command]
pub fn document_analysis_status(
    document_id: String,
    state: State<'_, AppState>,
    pagination: State<'_, PaginationState>,
) -> Result<Value, UiErrorDto> {
    if state
        .document
        .lock()
        .unwrap()
        .as_ref()
        .is_none_or(|d| d.document_id != document_id)
    {
        return Err(request_error("document_stale", "此 PDF 已关闭。"));
    }
    Ok(pagination.entry.lock().unwrap().as_ref().filter(|e|e.document_id==document_id).map(|e|e.status.clone()).unwrap_or_else(||json!({"documentId":document_id,"textLayer":"unavailable","paginationStatus":"unavailable","sequenceCount":0,"sampledPages":0,"message":null})))
}
