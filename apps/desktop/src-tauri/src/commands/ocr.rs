//! Desktop-only provider readiness wiring. Actual page work remains in the
//! core/CLI M3 path; this command lets the UI explain missing local models
//! before starting a durable job.

use std::path::Path;

use mpdf_core::jobs::JobStore;
use mpdf_core::ocr::RAPIDOCR_MODEL_FILES;

use crate::dto::{
    LocalOcrJobStatusDto, LocalOcrPageErrorDto, LocalOcrProviderStatusDto, LocalOcrSettingsDto,
    PersistentJobProgressDto,
};

#[tauri::command]
pub fn local_ocr_provider_status(settings: LocalOcrSettingsDto) -> LocalOcrProviderStatusDto {
    let (available, diagnostic) = match settings.provider.as_str() {
        "reference" => (true, "offline reference provider is available".to_owned()),
        "rapidocr" => match (settings.provider_executable, settings.model_dir) {
            (Some(executable), Some(model_dir))
                if std::path::Path::new(&executable).is_file()
                    && std::path::Path::new(&model_dir).is_dir()
                    && RAPIDOCR_MODEL_FILES
                        .iter()
                        .all(|name| std::path::Path::new(&model_dir).join(name).is_file()) =>
            {
                (true, "configured local RapidOCR executable and model files".to_owned())
            }
            _ => (
                false,
                "RapidOCR requires an existing executable and all three model files; no download is attempted".to_owned(),
            ),
        },
        _ => (false, "unknown local OCR provider".to_owned()),
    };
    LocalOcrProviderStatusDto {
        provider: settings.provider,
        available,
        diagnostic,
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn rapid_settings(executable: &Path, model_dir: &Path) -> LocalOcrSettingsDto {
        LocalOcrSettingsDto {
            provider: "rapidocr".into(),
            provider_executable: Some(executable.display().to_string()),
            model_dir: Some(model_dir.display().to_string()),
            jobs_db: String::new(),
            output_path: String::new(),
        }
    }

    #[test]
    fn rapidocr_readiness_requires_every_model_file() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("rapidocr-sidecar");
        std::fs::write(&executable, b"sidecar").unwrap();
        for name in RAPIDOCR_MODEL_FILES.iter().take(2) {
            std::fs::write(directory.path().join(name), b"model").unwrap();
        }
        let status = local_ocr_provider_status(rapid_settings(&executable, directory.path()));
        assert!(!status.available);
    }

    #[test]
    fn rapidocr_readiness_accepts_executable_and_complete_model_set() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("rapidocr-sidecar");
        std::fs::write(&executable, b"sidecar").unwrap();
        for name in RAPIDOCR_MODEL_FILES {
            std::fs::write(directory.path().join(name), b"model").unwrap();
        }
        let status = local_ocr_provider_status(rapid_settings(&executable, directory.path()));
        assert!(status.available);
    }
}
