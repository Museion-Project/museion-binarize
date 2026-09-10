//! Fixed-page local diagnostic: real PDFium/image pipeline, no output rewrite.
use mpdf_core::{
    binarization::SauvolaParams,
    cleanup::CleanupSettings,
    document_session::{PdfDocumentSession, PdfOpenOptions},
    image_pipeline::process_rendered_page,
    pdf_writer::EncodedPage,
    settings::{BinarizationMethod, PreprocessingSettings, ProcessingSettings},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::Path, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 4 {
        return Err("SOURCE PAGES NEW_JSON".into());
    }
    let source = Path::new(&a[1]);
    let started = Instant::now();
    let session = PdfDocumentSession::open(
        source,
        &PdfOpenOptions {
            compute_source_hash: true,
            ..Default::default()
        },
    )?;
    let settings = ProcessingSettings {
        dpi: 400,
        method: BinarizationMethod::Sauvola(SauvolaParams::default()),
        contrast: 0.,
        preprocessing: PreprocessingSettings::default(),
        cleanup: CleanupSettings::default(),
    };
    let mut rows = vec![];
    for page in a[2].split(',').map(str::parse::<u32>) {
        let p = page? - 1;
        let info = &session.info().pages[p as usize];
        for dpi in [
            400,
            ((1400. * 72. / info.geometry.width_points.max(info.geometry.height_points)).ceil()
                as u16)
                .min(400),
        ] {
            let t = Instant::now();
            let raster = session.render_page(p, dpi)?;
            let render = t.elapsed().as_secs_f64();
            let t = Instant::now();
            let resized = raster.resize(1400, 1400, image::imageops::FilterType::Triangle);
            let resize = t.elapsed().as_secs_f64();
            let t = Instant::now();
            let mut png = vec![];
            resized.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)?;
            let encode = t.elapsed().as_secs_f64();
            rows.push(json!({"page":p+1,"kind":"original_preview","dpi":dpi,"render_s":render,"resize_s":resize,"png_s":encode,"total_s":render+resize+encode,"png_bytes":png.len()}));
        }
        let t = Instant::now();
        let raster = session.render_page(p, settings.dpi)?;
        let render = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let processed = process_rendered_page(&raster, &settings)?;
        let process = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let encoded = EncodedPage::encode(
            &processed.bilevel,
            info.geometry.width_points,
            info.geometry.height_points,
        )?;
        let encoding = t.elapsed().as_secs_f64();
        rows.push(json!({"page":p+1,"kind":"binarize","dpi":settings.dpi,"render_s":render,"processing_s":process,"encoding_s":encoding,"stages":processed.stage_durations,"total_s":render+process+encoding,"ccitt_sha256":format!("{:x}",Sha256::digest(&encoded.ccitt_data)),"ccitt_bytes":encoded.ccitt_data.len()}));
    }
    let output = json!({"debug_assertions":cfg!(debug_assertions),"source_sha256":session.source_identity().content_sha256,"elapsed_s":started.elapsed().as_secs_f64(),"rows":rows});
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&a[3])?;
    use std::io::Write;
    f.write_all(&serde_json::to_vec_pretty(&output)?)?;
    Ok(())
}
