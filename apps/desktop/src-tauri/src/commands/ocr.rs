//! Durable OCR job status for the desktop app.
//!
//! Provider readiness lives in `commands::local_pipeline::local_ocr_readiness`
//! and nowhere else. An earlier RapidOCR-only `local_ocr_provider_status`
//! command was removed rather than kept alongside it: two readiness answers
//! for one question is how a UI ends up telling the user that OCR is ready
//! when the engine it will actually run has no models.

use std::path::Path;

use mpdf_core::jobs::JobStore;

use crate::dto::{LocalOcrJobStatusDto, LocalOcrPageErrorDto, PersistentJobProgressDto};

fn validate_job_query(jobs_db: &str, job_id: &str) -> Result<(), String> {
    if jobs_db.is_empty() || jobs_db.len() > 4096 || job_id.is_empty() || job_id.len() > 256 {
        return Err("jobs database path or job id is out of bounds".into());
    }
    if job_id.bytes().any(|byte| byte == 0) {
        return Err("job id contains an invalid NUL byte".into());
    }
    Ok(())
}

/// Reads durable progress and page-level terminal errors after a restart.
#[tauri::command]
pub fn local_ocr_status(jobs_db: String, job_id: String) -> Result<LocalOcrJobStatusDto, String> {
    validate_job_query(&jobs_db, &job_id)?;
    let store = JobStore::open(Path::new(&jobs_db)).map_err(|error| error.to_string())?;
    let progress = store
        .progress(&job_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "OCR job does not exist".to_owned())?;
    let page_errors = store
        .page_errors(&job_id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|page| {
            page.error.map(|message| LocalOcrPageErrorDto {
                page_number: page.page_index.saturating_add(1),
                message,
            })
        })
        .collect();
    let progress_dto = PersistentJobProgressDto::from(progress);
    Ok(LocalOcrJobStatusDto {
        job_id: progress_dto.job_id,
        status: progress_dto.status,
        total_pages: progress_dto.total_pages,
        completed_pages: progress_dto.completed_pages,
        failed_pages: progress_dto.failed_pages,
        cancelled_pages: progress_dto.cancelled_pages,
        page_errors,
    })
}

/// Requests cancellation of a non-terminal durable OCR job. The worker checks
/// this flag before every page and retains already committed page records.
#[tauri::command]
pub fn local_ocr_cancel(jobs_db: String, job_id: String) -> Result<(), String> {
    validate_job_query(&jobs_db, &job_id)?;
    let store = JobStore::open(Path::new(&jobs_db)).map_err(|error| error.to_string())?;
    store
        .request_cancel(&job_id)
        .map_err(|error| error.to_string())
}
