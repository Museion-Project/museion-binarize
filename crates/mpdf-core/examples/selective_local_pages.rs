//! Bounded, real-PDF regression for the shared selective conversion path.
use mpdf_core::{
    document_session::{PdfDocumentSession, PdfOpenOptions},
    page_selection::PageSelection,
    pipeline::PdfProcessingOptions,
    progress::NullProgressReporter,
    selective_pdf::process_selected_with_open_session,
    settings::ProcessingSettings,
};
use std::{path::PathBuf, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    let mut options = PdfProcessingOptions::default();
    options.pdfium.library_path = Some(
        std::env::var_os("MPDF_PDFIUM_LIBRARY")
            .ok_or("MPDF_PDFIUM_LIBRARY required")?
            .into(),
    );
    let began = Instant::now();
    let session = PdfDocumentSession::open(
        &input,
        &PdfOpenOptions {
            pdfium: options.pdfium.clone(),
            compute_source_hash: true,
            ..Default::default()
        },
    )?;
    let selection = PageSelection::parse(&args[3], session.info().page_count)?;
    let report = process_selected_with_open_session(
        &session,
        &selection,
        &output,
        &ProcessingSettings {
            dpi: 400,
            method: mpdf_core::settings::BinarizationMethod::Sauvola(Default::default()),
            contrast: 0.,
            preprocessing: Default::default(),
            cleanup: Default::default(),
        },
        &options,
        &NullProgressReporter,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"input":input,"output":output,"selected":args[3],"wall_seconds":began.elapsed().as_secs_f64(),"report":report})
        )?
    );
    Ok(())
}
