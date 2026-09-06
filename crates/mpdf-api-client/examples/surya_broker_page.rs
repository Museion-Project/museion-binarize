//! Local integration entrypoint; does not enable production Credits or BYOK.
use image::ImageReader;
use mpdf_api_client::{
    geometry_broker::{GeometryBroker, SuryaGeometry},
    Secret,
};
use mpdf_core::{
    ocr::PageOcrProvider, ocr_provider::geometry_transcription::GeometryTranscriptionPageProvider,
};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Duration};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        return Err("usage: surya_broker_page IMAGE GEOMETRY_JSON OUTPUT_JSON BROKER_ORIGIN CLIENT_TOKEN_FILE".into());
    }
    let output = PathBuf::from(&args[3]);
    if output.exists() {
        return Err("Output exists; refusing overwrite".into());
    }
    let tokenpath = PathBuf::from(&args[5]);
    let meta = std::fs::symlink_metadata(&tokenpath)?;
    if !meta.is_file() {
        return Err("Invalid client token file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("Client token must be mode 0600".into());
        }
    }
    let token = Secret::new(std::fs::read_to_string(tokenpath)?.trim().to_owned());
    let source = std::fs::read(&args[1])?;
    let source_hash = format!("{:x}", Sha256::digest(&source));
    let image = ImageReader::open(&args[1])?.decode()?;
    let geometry = SuryaGeometry::load(&PathBuf::from(&args[2]))?;
    let evidence = geometry.raw_evidence.clone();
    let index = geometry.geometry.page_index;
    let broker = GeometryBroker::local(&args[4], token)?;
    let mut pipeline = GeometryTranscriptionPageProvider::new(
        Box::new(geometry),
        Box::new(broker),
        "mixed-scholarly",
        Duration::from_secs(240),
        "local-surya-broker/1",
    )?;
    let mut page = pipeline.recognize(index, &image, &source_hash)?;
    page.provider_raw_artifact = Some(evidence);
    mpdf_core::ocr::validate_ocr_page(&page)?;
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    serde_json::to_writer_pretty(f, &page)?;
    println!("Composed OcrPage saved; immutable Surya geometry and raw evidence retained.");
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("Local broker pipeline failed: {e}");
        std::process::exit(1);
    }
}
