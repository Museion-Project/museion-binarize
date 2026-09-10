//! Binarize selected physical pages through the shared image pipeline, keeping
//! every other page and its original PDF objects. This is not page extraction.
use crate::{
    document::PdfDocumentInfo,
    document_session::{DocumentSession, PdfDocumentSession, PdfOpenOptions},
    error::{CoreError, Result},
    page_selection::PageSelection,
    pipeline::{self, PdfProcessingOptions, ProcessingReport},
    progress::{ProgressEvent, ProgressReporter},
    settings::ProcessingSettings,
    source_identity::SourceIdentity,
};
use image::DynamicImage;
use lopdf::{Document, Object};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, time::Instant};

fn invalid(message: impl ToString) -> CoreError {
    CoreError::OutputValidationFailed(message.to_string())
}
fn cancelled(progress: &dyn ProgressReporter) -> Result<()> {
    if progress.is_cancelled() {
        Err(CoreError::Cancelled)
    } else {
        Ok(())
    }
}
struct Selected<'a, S> {
    source: &'a S,
    info: PdfDocumentInfo,
}
impl<S: DocumentSession> DocumentSession for Selected<'_, S> {
    fn info(&self) -> &PdfDocumentInfo {
        &self.info
    }
    fn source_identity(&self) -> &SourceIdentity {
        self.source.source_identity()
    }
    fn pdfium_library_description(&self) -> String {
        self.source.pdfium_library_description()
    }
    fn render_page(&self, index: u32, dpi: u16) -> Result<DynamicImage> {
        self.source.render_page(index, dpi)
    }
}
struct StagingProgress<'a>(&'a dyn ProgressReporter);
impl ProgressReporter for StagingProgress<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
    fn report(&self, event: ProgressEvent) {
        if event != ProgressEvent::Finished {
            self.0.report(event);
        }
    }
}

/// Partial conversion requires a hashed open snapshot and a new destination.
/// All-pages conversion uses the existing compact full-document writer.
pub fn process_selected_with_open_session(
    session: &impl DocumentSession,
    selection: &PageSelection,
    output: &Path,
    settings: &ProcessingSettings,
    options: &PdfProcessingOptions,
    progress: &dyn ProgressReporter,
) -> Result<ProcessingReport> {
    settings.validate()?;
    cancelled(progress)?;
    if selection.is_empty()
        || selection
            .indices()
            .iter()
            .any(|&i| i >= session.info().page_count)
    {
        return Err(CoreError::InvalidParameter(
            "请选择至少一个有效的黑白处理页。".into(),
        ));
    }
    if selection.len() == session.info().page_count as usize {
        return pipeline::process_with_open_session(session, output, settings, options, progress);
    }
    if output.exists() || options.overwrite {
        return Err(CoreError::InvalidParameter(
            "局部黑白处理只保存为新文件。".into(),
        ));
    }
    let began = Instant::now();
    let source = &session.source_identity().canonical_path;
    let source_bytes = std::fs::read(source).map_err(|e| CoreError::io(source, e))?;
    let hash = format!("{:x}", Sha256::digest(&source_bytes));
    if session.source_identity().content_sha256.as_deref() != Some(hash.as_str()) {
        return Err(invalid("原 PDF 已改变或未记录打开时的校验值，请重新打开。"));
    }
    let original = Document::load_mem(&source_bytes).map_err(invalid)?;
    // An encrypted source cannot be serialized as an ordinary PDF without
    // decrypting all of its objects. Do not silently damage preserved pages.
    if original.is_encrypted() {
        return Err(CoreError::InvalidParameter(
            "加密 PDF 暂不支持局部黑白处理，请先另存为未加密副本。".into(),
        ));
    }
    let original_pages = original.get_pages();
    if original_pages.len() != session.info().page_count as usize {
        return Err(invalid("原 PDF 页面树不一致。"));
    }
    let work = tempfile::Builder::new()
        .prefix(".mpdf-range-")
        .tempdir_in(output.parent().unwrap_or(Path::new(".")))
        .map_err(|e| CoreError::io(output, e))?;
    let subset_path = work.path().join("selected.pdf");
    let mut info = session.info().clone();
    info.pages = selection
        .indices()
        .iter()
        .map(|&i| info.pages[i as usize].clone())
        .collect();
    info.page_count = selection.len() as u32;
    let selected = Selected {
        source: session,
        info,
    };
    let mut subset_options = options.clone();
    subset_options.prior_estimate = None;
    let mut report = pipeline::process_with_open_session(
        &selected,
        &subset_path,
        settings,
        &subset_options,
        &StagingProgress(progress),
    )?;
    cancelled(progress)?;
    let mut subset = Document::load(&subset_path).map_err(invalid)?;
    // Trailer /Size can reserve unused IDs well above the last actual object.
    // Start after the actual objects to avoid sparse new xref sections that
    // strict downstream readers cannot enumerate while checking preservation.
    let last_original = original.objects.keys().map(|id| id.0).max().unwrap_or(0);
    subset.renumber_objects_with(last_original + 1);
    let subset_pages = subset.get_pages();
    let mut merged = original.clone();
    merged.objects.extend(subset.objects.clone());
    merged.max_id = subset.max_id;
    let mut replaced = BTreeSet::new();
    for (n, &index) in selection.indices().iter().enumerate() {
        let target = original_pages[&(index + 1)];
        let replacement = subset
            .get_object(subset_pages[&(n as u32 + 1)])
            .and_then(Object::as_dict)
            .map_err(invalid)?;
        let page = merged
            .get_object_mut(target)
            .and_then(Object::as_dict_mut)
            .map_err(invalid)?;
        for key in [b"Contents".as_slice(), b"Resources", b"MediaBox"] {
            page.set(key, replacement.get(key).map_err(invalid)?.clone());
        }
        page.set(
            "CropBox",
            replacement.get(b"MediaBox").map_err(invalid)?.clone(),
        );
        page.set("Rotate", 0);
        page.set("UserUnit", 1);
        for key in [
            b"Annots".as_slice(),
            b"Thumb",
            b"BleedBox",
            b"TrimBox",
            b"ArtBox",
        ] {
            page.remove(key);
        }
        replaced.insert(target);
    }
    // Materialize unused object-number slots as PDF null. PDFium accepts sparse
    // xref subsections, but MuPDF's strict xref enumeration errors on missing
    // slots; the bookmark preservation verifier must be able to inspect them.
    if merged.max_id > 1_000_000 {
        return Err(invalid("PDF 对象编号过于稀疏，暂不支持局部处理。"));
    }
    let occupied: BTreeSet<_> = merged.objects.keys().map(|id| id.0).collect();
    for id in 1..=merged.max_id {
        if !occupied.contains(&id) {
            merged.objects.insert((id, 0), Object::Null);
        }
    }
    let staged = work.path().join("mixed.pdf");
    merged
        .save(&staged)
        .map_err(|e| CoreError::io(&staged, e))?;
    cancelled(progress)?;
    progress.report(ProgressEvent::Validating);
    // Verify original object preservation after serialization, including raw
    // content/image streams and unselected page dictionaries. No renumbering
    // of original page IDs: outline and link page targets retain their identity.
    let reopened = Document::load(&staged).map_err(invalid)?;
    if reopened.get_pages() != original_pages {
        return Err(invalid("保存后的页数、顺序或页面标识改变。"));
    }
    for (id, object) in &original.objects {
        cancelled(progress)?;
        if !replaced.contains(id) && reopened.objects.get(id) != Some(object) {
            return Err(invalid(format!("未处理页面的原对象发生改变：{id:?}")));
        }
    }
    let open = PdfOpenOptions {
        pdfium: options.pdfium.clone(),
        ..Default::default()
    };
    let output_session = PdfDocumentSession::open(&staged, &open)?;
    let subset_session = PdfDocumentSession::open(&subset_path, &open)?;
    if output_session.info().page_count != session.info().page_count {
        return Err(invalid("PDFium 核验页数失败。"));
    }
    for (index, page) in session.info().pages.iter().enumerate() {
        cancelled(progress)?;
        let actual = &output_session.info().pages[index].geometry;
        if (actual.width_points - page.geometry.width_points).abs() > 0.1
            || (actual.height_points - page.geometry.height_points).abs() > 0.1
        {
            return Err(invalid(format!("第 {} 页尺寸改变。", index + 1)));
        }
    }
    // Only selected pages need a raster check here; all original objects of
    // retained pages were compared above. This avoids rendering the whole book.
    for (n, &index) in selection.indices().iter().enumerate() {
        cancelled(progress)?;
        if output_session.render_page(index, 72)?.to_rgb8()
            != subset_session.render_page(n as u32, 72)?.to_rgb8()
        {
            return Err(invalid(format!(
                "第 {} 页混合保存的图像核验失败。",
                index + 1
            )));
        }
    }
    cancelled(progress)?;
    let latest = std::fs::read(source).map_err(|e| CoreError::io(source, e))?;
    if format!("{:x}", Sha256::digest(&latest)) != hash {
        return Err(invalid("保存前原 PDF 已改变。"));
    }
    std::fs::hard_link(&staged, output).map_err(|e| CoreError::io(output, e))?;
    report.output_bytes = std::fs::metadata(output)
        .map_err(|e| CoreError::io(output, e))?
        .len();
    report.elapsed_us = began.elapsed().as_micros() as u64;
    report.absolute_bytes_saved = report.original_bytes.saturating_sub(report.output_bytes);
    report.size_reduction_fraction = (report.original_bytes > 0)
        .then(|| 1.0 - report.output_bytes as f64 / report.original_bytes as f64);
    report.input_to_output_ratio = (report.output_bytes > 0)
        .then(|| report.original_bytes as f64 / report.output_bytes as f64);
    report.estimate_comparison = None;
    progress.report(ProgressEvent::Finished);
    Ok(report)
}
