//! Bounded developer preparation; same PDFium raster seam as the product.
use mpdf_core::document_session::{PdfDocumentSession, PdfOpenOptions};
use serde_json::{json, Value};
use std::{fs, path::Path, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<_> = std::env::args().collect();
    if a.len() != 3 {
        return Err("render_bookmark_pages CASES.json NEW_DIR".into());
    }
    let cases: Vec<Value> = serde_json::from_slice(&fs::read(&a[1])?)?;
    fs::create_dir(&a[2])?;
    let mut rows = vec![];
    for case in cases {
        let start = Instant::now();
        let path = case["source"].as_str().ok_or("source")?;
        let page = case["page_index"].as_u64().ok_or("page")? as u32;
        let session = PdfDocumentSession::open(
            Path::new(path),
            &PdfOpenOptions {
                compute_source_hash: true,
                ..Default::default()
            },
        )?;
        let info = &session.info().pages[page as usize];
        let png = Path::new(&a[2]).join(format!("{}.png", case["id"].as_str().ok_or("id")?));
        session.render_page(page, 150)?.save(&png)?;
        rows.push(json!({"id":case["id"],"source":path,"source_sha256":session.source_identity().content_sha256,
           "page_index":page,"page_count":session.info().pages.len(),"width":info.geometry.width_points,
           "height":info.geometry.height_points,"image_path":fs::canonicalize(png)?,"render_seconds":start.elapsed().as_secs_f64()}));
    }
    fs::write(
        Path::new(&a[2]).join("pages.json"),
        serde_json::to_vec_pretty(&rows)?,
    )?;
    Ok(())
}
