//! The editable local workflow. PDF algorithms stay in the shared core and
//! existing Python evidence/writer helpers; this service is also used by tests.
use crate::{
    state::OpenDocumentState,
    worker::{WorkerCommand, WorkerHandle},
};
use mpdf_core::{
    bookmarks::local::{compile_local, BookmarkEvidence},
    progress::{ProgressEvent, ProgressReporter},
    settings::ProcessingSettings,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tempfile::TempDir;

pub type Notify = Arc<dyn Fn(&str, Option<u32>, Option<u32>) + Send + Sync>;

#[derive(Clone)]
pub struct Runtime {
    pub python: PathBuf,
    pub script: PathBuf,
    pub cache: PathBuf,
}

impl Runtime {
    pub fn resolve(resources: Option<&Path>, cache: PathBuf) -> Result<Self, String> {
        let bundled = resources.map(|p| p.join("bookmarks/desktop_bridge.py"));
        let script = bundled.filter(|p| p.is_file()).unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../scripts/bookmarks/local/desktop_bridge.py")
        });
        if !script.is_file() {
            return Err("目录组件未安装完整。请使用包含本地目录组件的版本。".into());
        }
        let python = python_candidates().into_iter().find_map(|p| probe_python(&p))
            .ok_or("本地目录组件暂不可用：未找到带 PyMuPDF 和 Pillow 的 Python 运行环境。黑白处理仍可使用。")?;
        fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        Ok(Self {
            python,
            script,
            cache,
        })
    }
    pub fn hierarchy_runtime(&self) -> Self {
        Self { python: self.python.clone(), script: self.script.parent().unwrap().join("../hierarchy_models/desktop_bridge.py"), cache: self.cache.clone() }
    }
    pub fn model_root(&self) -> PathBuf {
        let local = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.runtime/toc-models");
        if local.is_dir() { local } else { self.cache.join("toc-models") }
    }
    pub fn run(
        &self,
        verb: &str,
        root: &Path,
        request: Value,
        cancel: &AtomicBool,
        notify: &Notify,
    ) -> Result<Value, String> {
        self.run_observed(verb, root, request, cancel, notify, &|_| {})
    }
    pub fn run_observed(
        &self, verb: &str, root: &Path, mut request: Value, cancel: &AtomicBool,
        notify: &Notify, observer: &dyn Fn(&Value),
    ) -> Result<Value, String> {
        check_cancel(cancel)?;
        request["work_dir"] = json!(root);
        let path = root.join(format!("{verb}-request.json"));
        write_new(&path, &request)?;
        let stdout =
            fs::File::create(root.join(format!("{verb}.stdout"))).map_err(|e| e.to_string())?;
        let stderr_path = root.join(format!("{verb}.stderr"));
        let stderr = fs::File::create(&stderr_path).map_err(|e| e.to_string())?;
        let mut child = Command::new(&self.python)
            .arg(&self.script)
            .arg(verb)
            .arg(&path)
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .map_err(|e| e.to_string())?;
        let began = Instant::now();
        let mut last = String::new();
        loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = fs::write(root.join("cancel"), b"cancel");
            }
            if let Ok(v) = read_json(&root.join("progress.json")) {
                observer(&v);
                if let Some(s) = v["stage"].as_str() {
                    if s != last {
                        notify(s, None, None);
                        last = s.to_owned();
                    }
                }
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                check_cancel(cancel)?;
                if !status.success() {
                    let detail = fs::read_to_string(&stderr_path).unwrap_or_default();
                    return Err(detail
                        .lines()
                        .last()
                        .unwrap_or("本地目录处理失败。")
                        .chars()
                        .take(1500)
                        .collect());
                }
                return read_json(&root.join(format!("{verb}-result.json")));
            }
            if began.elapsed() > Duration::from_secs(300) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("本地处理超时，未写出结果。".into());
            }
            std::thread::sleep(Duration::from_millis(80));
        }
    }
}

/// Find versioned installations independently of Finder's minimal PATH.
/// Probe only Python executables; never install packages or run a shell.
fn python_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![];
    if let Some(p) = std::env::var_os("MPDF_LOCAL_BOOKMARK_PYTHON") { candidates.push(PathBuf::from(p)); }
    candidates.extend([PathBuf::from("/opt/homebrew/bin/python3"), PathBuf::from("/usr/local/bin/python3")]);
    for prefix in ["/opt/homebrew/opt", "/usr/local/opt"] {
        if let Ok(entries) = fs::read_dir(prefix) {
            let mut versions: Vec<_> = entries.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("python@3.")).map(|e| e.path()).collect();
            versions.sort(); versions.reverse();
            for version in versions { candidates.push(version.join("libexec/bin/python3")); }
        }
    }
    if let Ok(entries) = fs::read_dir("/Library/Frameworks/Python.framework/Versions") {
        let mut versions: Vec<_> = entries.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("3.")).map(|e|e.path().join("bin/python3")).collect();
        versions.sort();versions.reverse();candidates.extend(versions);
    }
    candidates.push(PathBuf::from("python3"));
    candidates
}
fn probe_python(path: &Path) -> Option<PathBuf> {
    let result = Command::new(path).args(["-c", "import sys, fitz, PIL; print(sys.executable)"]).stderr(Stdio::null()).output().ok()?;
    if !result.status.success() { return None; }
    let executable = PathBuf::from(String::from_utf8(result.stdout).ok()?.trim());
    executable.is_absolute().then_some(executable)
}

pub struct BookmarkSession {
    pub id: String,
    pub document_id: String,
    pub work: TempDir,
    pub table: Value,
    pub intake: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EntryEdit {
    pub id: String,
    pub title: String,
    pub parent: Option<String>,
    pub target_pdf_page: Option<u32>,
}

#[cfg(test)]
pub fn generate(
    runtime: &Runtime,
    worker: &WorkerHandle,
    doc: &OpenDocumentState,
    pages: &[u32],
    mode: &str,
    session_id: String,
    cancel: Arc<AtomicBool>,
    notify: Notify,
) -> Result<BookmarkSession, String> {
    generate_with_pagination(runtime,worker,doc,pages,mode,session_id,cancel,notify,None)
}

pub fn generate_with_pagination(
    runtime:&Runtime,worker:&WorkerHandle,doc:&OpenDocumentState,pages:&[u32],mode:&str,
    session_id:String,cancel:Arc<AtomicBool>,notify:Notify,pagination_path:Option<&Path>,
)->Result<BookmarkSession,String>{
    if doc.password_protected_session {
        return Err("目录生成暂不支持加密 PDF，请先保存不加密副本。".into());
    }
    if pages.is_empty()
        || pages.len() > 40
        || pages.windows(2).any(|w| w[0] >= w[1])
        || pages.iter().any(|p| *p == 0 || *p > doc.page_count)
    {
        return Err("请选择有效且不重复的 PDF 目录页，最多40页。".into());
    }
    let work = tempfile::Builder::new()
        .prefix("contents-")
        .tempdir_in(&runtime.cache)
        .map_err(|e| e.to_string())?;
    let mut rendered = serde_json::Map::new();
    for page in pages {
        check_cancel(&cancel)?;
        notify("rendering_contents", Some(*page), Some(doc.page_count));
        let start = Instant::now();
        let (tx, rx) = std::sync::mpsc::channel();
        worker.send(WorkerCommand::RenderPage {
            page_index: page - 1,
            dpi: 150,
            processed: None,
            reply: tx,
        });
        let raster = rx
            .recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let image = work.path().join(format!("page-{page}.png"));
        raster.image.save(&image).map_err(|e| e.to_string())?;
        rendered.insert(page.to_string(), json!({"image_path":image,"render_seconds":start.elapsed().as_secs_f64(),"renderer":"pdfium","dpi":150}));
    }
    runtime.run("prepare",work.path(),json!({"source":doc.input_path,"pages":pages,"mode":mode,"rendered_pages":rendered,"runtime_cache":runtime.cache.join("runtime"),"pagination_path":pagination_path}),&cancel,&notify)?;
    check_cancel(&cancel)?;
    notify("building_tree", None, None);
    let pages: Vec<BookmarkEvidence> =
        serde_json::from_value(read_json(&work.path().join("evidence.json"))?)
            .map_err(|e| e.to_string())?;
    let compiled = compile_local(&pages)?;
    write_new(
        &work.path().join("compiled.json"),
        &serde_json::to_value(compiled).map_err(|e| e.to_string())?,
    )?;
    let finished = runtime.run("finish", work.path(), json!({}), &cancel, &notify)?;
    Ok(BookmarkSession {
        id: session_id,
        document_id: doc.document_id.clone(),
        work,
        table: finished["table"].clone(),
        intake: finished["intake"].clone(),
    })
}

pub struct PreparedBinarization {
    pub id:String,
    pub document_id:String,
    pub source_sha256:String,
    pub settings:ProcessingSettings,
    pub selection:mpdf_core::page_selection::PageSelection,
    pub path:PathBuf,
    pub output_sha256:String,
    pub report:mpdf_core::pipeline::ProcessingReport,
    _work:TempDir,
}
fn selected_pages(pages:Option<&[u32]>,count:u32)->Result<mpdf_core::page_selection::PageSelection,String>{
    match pages {
        None=>Ok(mpdf_core::page_selection::PageSelection::all(count)),
        Some(pages)=>{
            if pages.is_empty()||pages.len()>count as usize {return Err("请选择至少一个有效的黑白处理页。".into());}
            mpdf_core::page_selection::PageSelection::parse(&pages.iter().map(u32::to_string).collect::<Vec<_>>().join(","),count).map_err(|e|e.to_string())
        }
    }
}
pub fn prepare_binarization(
    worker:&WorkerHandle,doc:&OpenDocumentState,cache:&Path,settings:ProcessingSettings,
    pages:Option<&[u32]>,id:String,cancel:Arc<AtomicBool>,notify:Notify,
)->Result<PreparedBinarization,String>{
    check_cancel(&cancel)?;
    if file_sha(&doc.input_path)?!=doc.source_sha256{return Err("原 PDF 已改变，请重新打开。".into());}
    let selection=selected_pages(pages,doc.page_count)?;
    fs::create_dir_all(cache).map_err(|e|e.to_string())?;
    let work=tempfile::Builder::new().prefix("binarized-").tempdir_in(cache).map_err(|e|e.to_string())?;
    let path=work.path().join("prepared.pdf");
    let (tx,rx)=std::sync::mpsc::channel();
    worker.send(WorkerCommand::ProcessSelected{output:path.clone(),settings,selection:selection.clone(),progress:Box::new(LocalProgress{cancel:cancel.clone(),notify,page_count:doc.page_count}),reply:tx});
    let report=rx.recv().map_err(|_|"处理线程已停止。")?.map_err(|e|if matches!(e,mpdf_core::error::CoreError::Cancelled){"operation_cancelled".into()}else{e.to_string()})?;
    check_cancel(&cancel)?;
    if file_sha(&doc.input_path)?!=doc.source_sha256{return Err("处理期间原 PDF 已改变。".into());}
    let output_sha256=file_sha(&path)?;
    Ok(PreparedBinarization{id,document_id:doc.document_id.clone(),source_sha256:doc.source_sha256.clone(),settings,selection,path,output_sha256,report,_work:work})
}

pub struct SavePlan {
    pub output: PathBuf,
    pub settings: Option<ProcessingSettings>,
    pub binarize_pages: Option<Vec<u32>>,
    pub prepared: Option<Arc<PreparedBinarization>>,
    pub session: Option<Arc<BookmarkSession>>,
    pub entries: Vec<EntryEdit>,
    pub review_accepted: bool,
    pub projection: String,
}

pub fn save_pdf(
    runtime: Option<&Runtime>,
    worker: &WorkerHandle,
    doc: &OpenDocumentState,
    plan: SavePlan,
    cancel: Arc<AtomicBool>,
    notify: Notify,
) -> Result<Value, String> {
    let began = Instant::now();
    check_cancel(&cancel)?;
    if plan.settings.is_none() && plan.session.is_none() {
        return Err("请至少选择一项处理内容。".into());
    }
    if !plan.output.is_absolute()
        || plan
            .output
            .extension()
            .is_none_or(|x| !x.to_string_lossy().eq_ignore_ascii_case("pdf"))
    {
        return Err("请选择新 PDF 的保存位置。".into());
    }
    if plan.output.exists() {
        return Err("此位置已有文件。请选择新的文件名，原文件不会被覆盖。".into());
    }
    let parent = plan.output.parent().ok_or("无效的保存位置。")?;
    let source_hash = file_sha(&doc.input_path)?;
    if source_hash!=doc.source_sha256 {return Err("原 PDF 已改变，请重新打开。".into());}
    if let Some(session) = &plan.session {
        if session.document_id != doc.document_id
            || session.table["source_sha256"].as_str() != Some(&source_hash)
        {
            return Err("目录与当前文档不一致，请重新生成。".into());
        }
        // Validate edited fields before expensive binarization as well.
        validate_edits(&plan.entries, doc.page_count)?;
        if !plan.review_accepted {
            return Err("请先核对整份目录并确认。".into());
        }
    }
    let work = tempfile::Builder::new()
        .prefix(".museion-save-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    let mut result = json!({"outputPath":plan.output,"pages":doc.page_count,"bookmarksWritten":0,"binarized":plan.settings.is_some()});
    let processed = if let Some(settings) = plan.settings {
        let selection = selected_pages(plan.binarize_pages.as_deref(),doc.page_count)?;
        result["pagesProcessed"] = json!(selection.len());
        result["pagesPreserved"] = json!(doc.page_count as usize-selection.len());
        if let Some(prepared)=&plan.prepared {
            if prepared.document_id!=doc.document_id || prepared.source_sha256!=source_hash || prepared.settings!=settings || prepared.selection!=selection {
                return Err("范围、参数或原 PDF 已改变，请重新点击开始。".into());
            }
            check_cancel(&cancel)?;
            // Cache and chosen destination can live on different volumes.
            // Copy validated bytes into destination-local staging; never re-run
            // rasterization/encoding and never hard-link across filesystems.
            let staged=work.path().join("binarized.pdf");
            fs::copy(&prepared.path,&staged).map_err(|e|e.to_string())?;
            if file_sha(&staged)?!=prepared.output_sha256{return Err("暂存的黑白结果已改变，请重新开始。".into());}
            result["binarizationElapsedUs"]=json!(prepared.report.elapsed_us);
            result["binarizationReused"]=json!(true);
            Some(staged)
        } else {
        notify("binarizing", None, Some(doc.page_count));
        let output = work.path().join("binarized.pdf");
        let (tx, rx) = std::sync::mpsc::channel();
        worker.send(WorkerCommand::ProcessSelected {
            output: output.clone(),
            settings,
            selection,
            progress: Box::new(LocalProgress {
                cancel: cancel.clone(),
                notify: notify.clone(),
                page_count: doc.page_count,
            }),
            reply: tx,
        });
        let report = rx.recv().map_err(|_| "处理线程已停止。")?.map_err(|e| {
            if matches!(e, mpdf_core::error::CoreError::Cancelled) {
                "operation_cancelled".into()
            } else {
                e.to_string()
            }
        })?;
        result["binarizationElapsedUs"] = json!(report.elapsed_us);
        Some(output)
        }
    } else {
        None
    };
    check_cancel(&cancel)?;
    let completed = if let Some(session) = &plan.session {
        let output = work.path().join("with-bookmarks.pdf");
        let rt = runtime.ok_or("目录组件不可用。")?;
        let receipt=rt.run("save",work.path(),json!({"table_path":session.work.path().join("table.json"),"entries":plan.entries,"review_accepted":plan.review_accepted,"projection":plan.projection,"processed_source":processed,"output":output}),&cancel,&notify)?;
        result["bookmarksWritten"] = json!(plan.entries.len());
        result["bookmarkValidation"] = receipt;
        output
    } else {
        processed.ok_or("没有可保存的结果。")?
    };
    check_cancel(&cancel)?;
    notify("validating", None, Some(doc.page_count));
    if file_sha(&doc.input_path)? != source_hash {
        return Err("处理期间原 PDF 已改变，未保存结果。".into());
    }
    // Both staged files and destination share a filesystem. Atomic no-clobber
    // publication happens only after core/writer validations and cancellation.
    fs::hard_link(&completed, &plan.output).map_err(|e| format!("无法保存新 PDF：{e}"))?;
    result["outputBytes"] = json!(fs::metadata(&plan.output).map(|m| m.len()).unwrap_or(0));
    result["elapsedSeconds"] = json!(began.elapsed().as_secs_f64());
    result["sourceSha256"] = json!(source_hash);
    // Keep successful review provenance with the still-open document session.
    if let Some(session) = &plan.session {
        let review = work.path().join("reviewed-original.json");
        let record = session
            .work
            .path()
            .join(format!("saved-{}.json", began.elapsed().as_nanos()));
        let _ = write_new(
            &record,
            &json!({"result":result,"review":read_json(&review).ok()}),
        );
    }
    Ok(result)
}

pub fn validate_edits(entries: &[EntryEdit], page_count: u32) -> Result<(), String> {
    if entries.is_empty() || entries.len() > 4000 {
        return Err("目录为空或条目过多。".into());
    }
    let mut parents = std::collections::HashMap::new();
    for e in entries {
        if e.id.is_empty()
            || parents.insert(e.id.as_str(), e.parent.as_deref()).is_some()
            || e.title.trim().is_empty()
            || e.title.chars().any(char::is_control)
        {
            return Err("目录含重复条目或空标题。".into());
        }
        if e.target_pdf_page.is_none_or(|p| p >= page_count) {
            return Err("请为每个目录条目填写有效的 PDF 目标页。".into());
        }
    }
    for e in entries {
        let mut seen = std::collections::HashSet::new();
        let mut current = Some(e.id.as_str());
        while let Some(id) = current {
            if !seen.insert(id) {
                return Err("父节点设置形成循环。".into());
            }
            current = *parents.get(id).ok_or("父节点不存在。")?;
        }
    }
    Ok(())
}

fn file_sha(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn write_new(path: &Path, value: &Value) -> Result<(), String> {
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err("operation_cancelled".into())
    } else {
        Ok(())
    }
}
struct LocalProgress {
    cancel: Arc<AtomicBool>,
    notify: Notify,
    page_count: u32,
}
impl ProgressReporter for LocalProgress {
    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
    fn report(&self, event: ProgressEvent) {
        match event {
            ProgressEvent::PageStarted { page } => {
                (self.notify)("binarizing", Some(page), Some(self.page_count))
            }
            ProgressEvent::Validating => (self.notify)("validating", None, Some(self.page_count)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edit(id: &str, parent: Option<&str>, target: Option<u32>) -> EntryEdit {
        EntryEdit {
            id: id.into(),
            title: id.into(),
            parent: parent.map(str::to_string),
            target_pdf_page: target,
        }
    }
    #[test]
    fn review_validation_rejects_dangling_cycles_missing_targets_and_empty_titles() {
        assert!(validate_edits(
            &[edit("a", Some("b"), Some(1)), edit("b", Some("a"), Some(2))],
            10
        )
        .is_err());
        assert!(validate_edits(&[edit("a", Some("absent"), Some(1))], 10).is_err());
        assert!(validate_edits(&[edit("a", None, None)], 10).is_err());
        assert!(validate_edits(&[edit("a", None, Some(10))], 10).is_err());
        assert!(validate_edits(&[edit("a", None, Some(0)), edit("a", None, Some(1))], 10).is_err());
        assert!(validate_edits(
            &[edit("a", None, Some(0)), edit("b", Some("a"), Some(1))],
            10
        )
        .is_ok());
    }
    #[test]
    fn edited_payload_cannot_override_source_or_evidence() {
        assert!(serde_json::from_value::<EntryEdit>(
            json!({"id":"a","title":"A","parent":null,"target_pdf_page":1,"source":"other.pdf"})
        )
        .is_err());
    }
    /// Explicitly opt-in: reads the same existing PDFs, no new corpus or cloud.
    /// Generate first, inspect/edit outside this test, then run the save phase.
    #[test]
    #[ignore]
    fn existing_books_local_service() {
        let manifest = PathBuf::from(
            std::env::var("MPDF_LOCAL_TOOLS_CASES").expect("explicit fixture manifest"),
        );
        let config = read_json(&manifest).unwrap();
        let phase = std::env::var("MPDF_LOCAL_TOOLS_PHASE").unwrap_or_else(|_| "generate".into());
        let root = manifest.parent().unwrap();
        let rt = Runtime::resolve(None, root.join("runtime-cache")).unwrap();
        let pdfium = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/pdfium/libpdfium.dylib");
        let worker = WorkerHandle::spawn(Some(pdfium));
        for case in config.as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let source = PathBuf::from(case["source"].as_str().unwrap());
            let (tx, rx) = std::sync::mpsc::channel();
            worker.send(WorkerCommand::Open {
                path: source.clone(),
                password: None,
                reply: tx,
            });
            let opened = rx.recv().unwrap().unwrap();
            let doc = OpenDocumentState {
                source_sha256: opened.source_sha256,
                document_id: format!("test-{name}"),
                file_name: source.file_name().unwrap().to_string_lossy().into(),
                input_path: source,
                page_count: opened.info.page_count,
                password_protected_session: false,
            };
            let notify: Notify = Arc::new(|stage, page, _| println!("stage {stage} {page:?}"));
            let cancel = Arc::new(AtomicBool::new(false));
            if phase == "generate" {
                let pages: Vec<u32> = serde_json::from_value(case["pages"].clone()).unwrap();
                let session = generate(
                    &rt,
                    &worker,
                    &doc,
                    &pages,
                    case["mode"].as_str().unwrap(),
                    format!("session-{name}"),
                    cancel,
                    notify,
                )
                .unwrap();
                write_new(&root.join(format!("{name}-generated.json")),&json!({"documentId":doc.document_id,"sessionId":session.id,"table":session.table,"intake":session.intake})).unwrap();
                let kept = session.work.keep();
                println!("retained evidence {}", kept.display());
            } else {
                let generated = read_json(&root.join(format!("{name}-generated.json"))).unwrap();
                let tmp = tempfile::Builder::new()
                    .prefix("review-")
                    .tempdir_in(root)
                    .unwrap();
                write_new(&tmp.path().join("table.json"), &generated["table"]).unwrap();
                let session = Arc::new(BookmarkSession {
                    id: format!("session-{name}"),
                    document_id: doc.document_id.clone(),
                    table: generated["table"].clone(),
                    intake: generated["intake"].clone(),
                    work: tmp,
                });
                let entries: Vec<EntryEdit> = serde_json::from_value(
                    read_json(&root.join(format!("{name}-review.json"))).unwrap(),
                )
                .unwrap();
                let outcome = save_pdf(
                    Some(&rt),
                    &worker,
                    &doc,
                    SavePlan {
                        binarize_pages: None,
                        prepared: None,
                        output: root.join(format!("{name}-bookmarks.pdf")),
                        settings: None,
                        session: Some(session),
                        entries,
                        review_accepted: true,
                        projection: "navigation".into(),
                    },
                    cancel,
                    notify,
                )
                .unwrap();
                write_new(&root.join(format!("{name}-saved.json")), &outcome).unwrap();
            }
        }
    }
}
