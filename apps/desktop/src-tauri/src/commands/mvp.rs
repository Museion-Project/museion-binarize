//! Explicit opt-in development consumer. Fixed Python module; no shell/provider fallback.
use crate::{dto::UiErrorDto, errors::request_error, state::{AppState, OperationKind}};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{io::{self, BufRead, BufReader, Read, Write}, path::{Path, PathBuf}, process::{Child, Command, ExitStatus, Stdio}, thread::JoinHandle};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Deserialize, Serialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub struct Request {
    pub document_id: Option<String>,
    pub action: String,
    pub mode: String,
    pub session_id: Option<String>,
    pub client_operation_id: Option<String>,
    pub continuation_state_sha256: Option<String>,
    pub pages: Option<Vec<u32>>,
    pub import_path: Option<String>,
    pub expected_revision: Option<u64>,
    pub actions: Option<Vec<Value>>,
    pub output_path: Option<String>,
    pub selected_ids: Option<Vec<String>>,
    pub partial_confirmed: Option<bool>,
    pub query: Option<String>,
    pub saved_pdf: Option<String>,
    pub save_journal_id: Option<String>,
    pub save_journal_sha256: Option<String>,
}
fn enabled() -> bool {
    cfg!(feature="mvp-development") && std::env::var("MUSEION_MVP_DEV").as_deref()==Ok("1")
}
#[tauri::command]
pub fn mvp_capabilities() -> Value {
    json!({"enabled":enabled(),"networkSendEnabled":false,"releaseDefault":"OCR-off"})
}
#[tauri::command]
pub fn local_ocr_capabilities() -> Value {
    json!({"enabled":cfg!(feature="local-ocr-candidate"),"networkSendEnabled":false,"qualityReady":false,"releaseDefault":"OCR-off"})
}
#[tauri::command]
pub async fn local_ocr_call(request: Request, app: AppHandle, state: State<'_,AppState>) -> Result<Value,UiErrorDto> {
    if !cfg!(feature="local-ocr-candidate") {return Err(request_error("candidate_disabled","Local OCR candidate is disabled."));}
    validate_local_request(&request)?;
    bridge_call(request,app,state,true).await
}
fn validate_local_request(request: &Request) -> Result<(),UiErrorDto> {
    if request.mode!="local" || !["readiness","start","cancel","continue","recover-processing","resume","reload","review","save","search","recover-save"].contains(&request.action.as_str()) {
        return Err(request_error("invalid_request","Only the local OCR workflow is available."));
    }
    if ["start","cancel","continue","recover-processing"].contains(&request.action.as_str()) && request.client_operation_id.as_ref().is_none_or(|id|id.len()!=32||!id.bytes().all(|b|b.is_ascii_hexdigit())) {return Err(request_error("invalid_request","A bound local operation identity is required."));}
    if ["continue","recover-processing"].contains(&request.action.as_str()) && (request.session_id.as_ref().is_none_or(|id|id.len()!=32||!id.bytes().all(|b|b.is_ascii_hexdigit())) || request.continuation_state_sha256.as_ref().is_none_or(|id|id.len()!=64||!id.bytes().all(|b|b.is_ascii_hexdigit())) || request.pages.is_some()) {
        return Err(request_error("invalid_request","Continue requires the observed original task and its unchanged page selection."));
    }
    Ok(())
}
#[tauri::command]
pub async fn mvp_call(request: Request, app: AppHandle, state: State<'_,AppState>) -> Result<Value,UiErrorDto> {
    if !enabled() {return Err(request_error("development_disabled","OCR development entry is disabled."));}
    bridge_call(request,app,state,false).await
}
async fn bridge_call(request: Request, app: AppHandle, state: State<'_,AppState>, packaged: bool) -> Result<Value,UiErrorDto> {
    if !["local","critical-edition","paid","paid-contents"].contains(&request.mode.as_str()) || !["readiness","start","cancel","continue","recover-processing","import","resume","reload","review","save","search","preflight","recover-save"].contains(&request.action.as_str()) {
        return Err(request_error("invalid_request","Explicit mode/action required."));
    }
    if request.action=="start" && ["paid","paid-contents"].contains(&request.mode.as_str()) {
        return Err(request_error("cloud_disabled","Cloud sending is disabled. Import an existing result; a future send requires exact payload, endpoint, model and budget approval."));
    }
    let mut lease=None;
    let document=if ["start","continue","recover-processing","import","review","save","recover-save"].contains(&request.action.as_str()) {
        let (bound,doc)=state.claim_document_operation(request.document_id.as_deref(),OperationKind::Processing)
            .map_err(|code|request_error(code,if code=="operation_active" {"Wait for the current document operation."} else {"Open the matching source PDF first."}))?;
        lease=Some(bound);
        Some(doc)
    } else if request.action!="readiness" {
        Some(state.document.lock().unwrap().as_ref().filter(|d|Some(d.document_id.as_str())==request.document_id.as_deref()).cloned().ok_or_else(||request_error("document_stale","Open the matching source PDF first."))?)
    } else {None};
    if let Some(doc)=document.as_ref() {
        if doc.password_protected_session {return Err(request_error("invalid_request","Encrypted sessions are not supported by this development OCR entry."));}
        if request.action=="start" {validate_start_pages(request.pages.as_deref(),doc.page_count)?;}
    }
    let (config_path,python,package_root,session_root)=if packaged {
        let root=app.path().resource_dir().map_err(|e|request_error("runtime_missing",e.to_string()))?.join("local-ocr");
        let manifest=root.join("runtime-manifest.json");
        let config:Value=serde_json::from_slice(&std::fs::read(&manifest).map_err(|_|request_error("runtime_missing","The local OCR resource package is missing."))?).map_err(|e|request_error("runtime_missing",e.to_string()))?;
        let relative=config["python"].as_str().ok_or_else(||request_error("runtime_missing","Packaged Python is missing."))?;
        if std::path::Path::new(relative).is_absolute() || std::path::Path::new(relative).components().any(|c|matches!(c,std::path::Component::ParentDir)) {return Err(request_error("runtime_missing","Invalid packaged Python path."));}
        let session=app.path().app_local_data_dir().map_err(|e|request_error("runtime_missing",e.to_string()))?.join("local-ocr-candidate/sessions");
        (manifest.to_string_lossy().to_string(),root.join(relative),root,Some(session))
    } else {
        let path=std::env::var("MUSEION_MVP_RUNTIME_CONFIG").map_err(|_|request_error("runtime_missing","An explicit external development runtime configuration is required."))?;
        let config:Value=serde_json::from_slice(&std::fs::read(&path).map_err(|e|request_error("runtime_missing",e.to_string()))?).map_err(|e|request_error("runtime_missing",e.to_string()))?;
        (path,PathBuf::from(config["python"].as_str().ok_or_else(||request_error("runtime_missing","Explicit Python missing."))?),PathBuf::from(config["package_root"].as_str().ok_or_else(||request_error("runtime_missing","Package resources missing."))?),None)
    };
    if !python.is_absolute()||!python.is_file()||!package_root.is_absolute()||!package_root.join("scripts/ocr/app_mvp_bridge/__main__.py").is_file() {return Err(request_error("runtime_missing","Local OCR components are incomplete."));}
    let mut payload=json!({"action":request.action,"mode":request.mode});
    let fields=[("session_id",json!(request.session_id)),("client_operation_id",json!(request.client_operation_id)),("continuation_state_sha256",json!(request.continuation_state_sha256)),("pages",json!(request.pages)),("import_path",json!(request.import_path)),("expected_revision",json!(request.expected_revision)),("actions",json!(request.actions)),("output_path",json!(request.output_path)),("selected_ids",json!(request.selected_ids)),("partial_confirmed",json!(request.partial_confirmed)),("query",json!(request.query)),("saved_pdf",json!(request.saved_pdf)),("save_journal_id",json!(request.save_journal_id)),("save_journal_sha256",json!(request.save_journal_sha256))];
    for (key,value) in fields {if !value.is_null(){payload[key]=value;}}
    if let Some(doc)=document {
        payload["input_pdf"]=json!(doc.input_path);payload["input_sha256"]=json!(doc.source_sha256);
        if packaged {payload["document_id"]=json!(doc.document_id);payload["document_page_count"]=json!(doc.page_count);}
    }
    let document_id=request.document_id.clone();
    tauri::async_runtime::spawn_blocking(move ||{
        let _lease=lease;
        let (child,launcher_proof)=if let Some(root)=session_root {
            let launch=crate::local_runtime::PackagedLaunch::prepare(&package_root,&root,Some(&std::env::temp_dir().join("museion-local-ocr-candidate/sessions")))
                .map_err(|e|request_error("runtime_identity",e.to_string()))?;
            prepare_session_root(&root).map_err(|e|request_error("session_storage",e.to_string()))?;
            let (child,proof)=launch.spawn().map_err(|e|request_error("runtime_identity",e.to_string()))?;
            (child,Some(proof))
        } else {
            let mut command=Command::new(python);
            command.args(["-s","-B","-m","scripts.ocr.app_mvp_bridge"]).current_dir(&package_root)
                .env_remove("PYTHONHOME").env_remove("PYTHONPATH").env("PYTHONPATH",&package_root)
                .env_remove("DYLD_LIBRARY_PATH").env_remove("DYLD_FRAMEWORK_PATH").env("PYTHONNOUSERSITE","1")
                .env("PYTHONDONTWRITEBYTECODE","1").env("MUSEION_MVP_RUNTIME_CONFIG",config_path);
            (command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e|request_error("runtime_failed",e.to_string()))?,None)
        };
        let (bytes,status)=exchange_bridge(child,&payload,32*1024*1024,move |event| {
            let _=app.emit("mpdf://mvp-progress",json!({"documentId":document_id,"event":event}));
        }).map_err(|e|request_error("runtime_failed",e.to_string()))?;
        let mut value:Value=serde_json::from_slice(&bytes).map_err(|_|request_error("runtime_protocol","Bridge did not return a bounded JSON result."))?;
        if !status.success(){return Err(request_error("mvp_failed",value["message"].as_str().unwrap_or("Local bridge failed.")));}
        if let Some(proof)=launcher_proof {value["launcher_runtime_proof"]=serde_json::to_value(proof).map_err(|e|request_error("runtime_protocol",e.to_string()))?;}
        Ok(value)
    }).await.map_err(|e|request_error("runtime_failed",e.to_string()))?
}

/// Own the direct bridge until it is reaped, including transport-error exits.
/// This does not certify the independent OCR worker/cancellation lifecycle.
struct BridgeProcess {
    child: Child,
    reaped: bool,
    progress: Option<JoinHandle<()>>,
}

impl Drop for BridgeProcess {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(progress) = self.progress.take() {
            let _ = progress.join();
        }
    }
}

fn exchange_bridge(
    child: Child,
    payload: &Value,
    result_limit: u64,
    on_progress: impl Fn(Value) + Send + 'static,
) -> io::Result<(Vec<u8>, ExitStatus)> {
    let mut process = BridgeProcess { child, reaped: false, progress: None };
    let missing_pipe = || io::Error::new(io::ErrorKind::InvalidInput, "Bridge requires piped standard streams.");
    let mut stdin = process.child.stdin.take().ok_or_else(missing_pipe)?;
    let stdout = process.child.stdout.take().ok_or_else(missing_pipe)?;
    let stderr = process.child.stderr.take().ok_or_else(missing_pipe)?;
    let encoded = serde_json::to_vec(payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    process.progress = Some(std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Ok(event) = serde_json::from_str::<Value>(&line) { on_progress(event); }
        }
    }));
    stdin.write_all(&encoded)?;
    drop(stdin); // EOF completes the bridge's structured stdin request.
    let mut bytes = Vec::new();
    stdout.take(result_limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > result_limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Bridge result exceeds the bounded protocol size."));
    }
    let status = process.child.wait()?;
    process.reaped = true;
    Ok((bytes, status))
}

fn validate_start_pages(pages: Option<&[u32]>, page_count: u32) -> Result<(),UiErrorDto> {
    use std::collections::BTreeSet;
    let valid=pages.is_some_and(|p|!p.is_empty() && p.len()<=40
        && p.iter().all(|n|*n>=1 && *n<=page_count)
        && p.iter().copied().collect::<BTreeSet<_>>().len()==p.len());
    if !valid {return Err(request_error("invalid_request","Select at most 40 distinct actual PDF pages."));}
    Ok(())
}

/// Stable private session state; preserve existing trees and reject symlinks.
fn prepare_session_root(path: &Path) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind};
    if !path.is_absolute() || path.components().any(|c|matches!(c,std::path::Component::ParentDir)) {
        return Err(Error::new(ErrorKind::InvalidInput,"Session root must be absolute."));
    }
    for parent in path.ancestors() {
        match std::fs::symlink_metadata(parent) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(Error::new(ErrorKind::InvalidInput,"Session root cannot follow a symlink.")),
            Ok(_) => {},
            Err(e) if e.kind()==ErrorKind::NotFound => {},
            Err(e) => return Err(e),
        }
    }
    let mut builder=std::fs::DirBuilder::new();builder.recursive(true);
    #[cfg(unix)] {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path)?.permissions().mode() & 0o077 != 0 {
            return Err(Error::new(ErrorKind::PermissionDenied,"Existing session root must be private."));
        }
    }
    path.canonicalize()
}

#[cfg(test)]
mod session_storage_tests {
    use super::*;
    #[test]
    fn processing_recovery_requires_original_state_and_holds_the_same_local_contract() {
        let value=json!({"documentId":"source","action":"recover-processing","mode":"local",
            "sessionId":"a".repeat(32),"clientOperationId":"b".repeat(32),"continuationStateSha256":"c".repeat(64)});
        assert!(validate_local_request(&serde_json::from_value(value.clone()).unwrap()).is_ok());
        for (key,bad) in [("sessionId",Value::Null),("continuationStateSha256",Value::Null),
                          ("clientOperationId",Value::Null),("pages",json!([1])),("mode",json!("paid"))] {
            let mut input=value.clone();input[key]=bad;
            assert!(validate_local_request(&serde_json::from_value(input).unwrap()).is_err());
        }
    }
    #[test]
    fn continuation_requires_original_session_state_and_new_operation() {
        let value=json!({"documentId":"open-source","action":"continue","mode":"local",
            "sessionId":"a".repeat(32),"clientOperationId":"b".repeat(32),"continuationStateSha256":"c".repeat(64)});
        let valid:Request=serde_json::from_value(value.clone()).unwrap();
        assert!(validate_local_request(&valid).is_ok());
        assert_eq!(serde_json::to_value(valid).unwrap()["continuationStateSha256"],value["continuationStateSha256"]);
        for (field,invalid) in [("mode",json!("paid")),("action",json!("import")),
            ("sessionId",Value::Null),("clientOperationId",Value::Null),("continuationStateSha256",Value::Null),
            ("sessionId",json!("../outside")),("continuationStateSha256",json!("stale")),("pages",json!([2,3]))] {
            let mut request=value.clone();request[field]=invalid;
            let request:Request=serde_json::from_value(request).unwrap();
            assert!(validate_local_request(&request).is_err(),"{field}");
        }
    }
    #[cfg(unix)]
    fn fixture(script: &str) -> Child {
        Command::new("/bin/sh").args(["-c",script]).stdin(Stdio::piped())
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap()
    }
    #[cfg(unix)]
    fn assert_reaped(pid: u32) {
        assert!(!Command::new("/bin/kill").args(["-0", &pid.to_string()])
            .stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success(),
            "owned bridge {pid} remains after exchange");
    }
    #[cfg(unix)]
    #[test]
    fn bridge_exchange_preserves_request_result_and_progress() {
        use std::sync::{Arc,Mutex};
        let events=Arc::new(Mutex::new(Vec::new()));let captured=events.clone();
        let child=fixture("IFS= read -r request || :; printf '%s' \"$request\"; printf '{\"stage\":\"fixture\"}\\n' >&2");
        let pid=child.id();let payload=json!({"source":"synthetic-only","pages":[2,1]});
        let (bytes,status)=exchange_bridge(child,&payload,1024,move |e|captured.lock().unwrap().push(e)).unwrap();
        assert!(status.success());assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(),payload);
        assert_eq!(*events.lock().unwrap(),vec![json!({"stage":"fixture"})]);assert_reaped(pid);
    }
    #[cfg(unix)]
    #[test]
    fn bridge_exchange_keeps_nonzero_result_for_error_reporting() {
        let child=fixture("IFS= read -r request || :; printf '{\"message\":\"fixture failure\"}'; exit 7");
        let pid=child.id();let (bytes,status)=exchange_bridge(child,&json!({}),1024,|_|{}).unwrap();
        assert_eq!(status.code(),Some(7));assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["message"],"fixture failure");
        assert_reaped(pid);
    }
    #[cfg(unix)]
    #[test]
    fn bridge_exchange_reaps_child_after_closed_stdin() {
        let temp=tempfile::tempdir().unwrap();let ready=temp.path().join("ready");
        let child=Command::new("/bin/sh").args(["-c","exec 0<&-; printf ready > \"$1\"; exec /bin/sleep 30","fixture"])
            .arg(&ready).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let pid=child.id();let deadline=std::time::Instant::now()+std::time::Duration::from_secs(3);
        while !ready.exists() && std::time::Instant::now()<deadline {std::thread::sleep(std::time::Duration::from_millis(5));}
        if !ready.exists() {let mut owned=child;let _=owned.kill();let _=owned.wait();panic!("fixture startup timed out");}
        let error=exchange_bridge(child,&json!({"synthetic":true}),1024,|_|{}).unwrap_err();
        assert_eq!(error.kind(),io::ErrorKind::BrokenPipe);assert_reaped(pid);
    }
    #[cfg(unix)]
    #[test]
    fn bridge_exchange_rejects_oversize_without_waiting_for_child() {
        let child=fixture("IFS= read -r request || :; printf '01234567890123456789'; exec /bin/sleep 30");
        let pid=child.id();let started=std::time::Instant::now();
        let error=exchange_bridge(child,&json!({}),16,|_|{}).unwrap_err();
        assert_eq!(error.kind(),io::ErrorKind::InvalidData);assert!(started.elapsed()<std::time::Duration::from_secs(3));
        assert_reaped(pid);
    }
    #[cfg(unix)]
    #[test]
    fn bridge_exchange_reaps_child_if_required_pipe_missing() {
        let child=Command::new("/bin/sleep").arg("30").stdin(Stdio::piped())
            .stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();let pid=child.id();
        let error=exchange_bridge(child,&json!({}),16,|_|{}).unwrap_err();
        assert_eq!(error.kind(),io::ErrorKind::InvalidInput);assert_reaped(pid);
    }
    #[test]
    fn private_root_survives_and_never_replaces_history() {
        let temp=tempfile::tempdir().unwrap();
        let root=temp.path().canonicalize().unwrap().join("new/sessions");
        let prepared=prepare_session_root(&root).unwrap();
        std::fs::write(prepared.join("history"),b"preserve").unwrap();
        assert_eq!(prepare_session_root(&root).unwrap(),prepared);
        assert_eq!(std::fs::read(prepared.join("history")).unwrap(),b"preserve");
    }
    #[test]
    fn relative_root_is_rejected() {
        assert!(prepare_session_root(Path::new("relative/sessions")).is_err());
    }
    #[test]
    fn invalid_physical_page_lists_are_rejected_before_launch() {
        for pages in [None,Some(vec![]),Some(vec![0]),Some(vec![3]),Some(vec![1,1]),Some(vec![1;41])] {
            assert!(validate_start_pages(pages.as_deref(),2).is_err());
        }
        assert!(validate_start_pages(Some(&[2,1]),2).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_and_public_existing_root_are_rejected() {
        use std::os::unix::fs::{symlink,PermissionsExt};
        let temp=tempfile::tempdir().unwrap();let base=temp.path().canonicalize().unwrap();
        let link=base.join("link");symlink(&base,&link).unwrap();
        assert!(prepare_session_root(&link.join("sessions")).is_err());
        let public=base.join("public");std::fs::create_dir(&public).unwrap();
        std::fs::set_permissions(&public,std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(prepare_session_root(&public).is_err());
    }
}
