//! Independent PDFium consumer check against the requested bookmark table.
use mpdf_core::document_session::{PdfDocumentSession, PdfOpenOptions};
use serde_json::{json, Value};
use std::{fs, path::Path};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<_> = std::env::args().collect();
    if a.len() != 3 {
        return Err("verify_bookmark_outline TABLE.json OUTPUT.pdf".into());
    }
    let t: Value = serde_json::from_slice(&fs::read(&a[1])?)?;
    let session = PdfDocumentSession::open(Path::new(&a[2]), &PdfOpenOptions::default())?;
    let actual = session.native_outline()?;
    let requested = t["entries"].as_array().ok_or("entries")?;
    if actual.len() != requested.len() {
        return Err("outline count mismatch".into());
    }
    for (o, e) in actual.iter().zip(requested) {
        if o.title != e["title"].as_str().ok_or("title")?
            || o.page_index != e["target_pdf_page"].as_u64().ok_or("page")? as u32
            || o.level != e["level"].as_u64().ok_or("level")? as u16
        {
            return Err(format!("outline mismatch: {:?} / {}", o, e).into());
        }
    }
    println!(
        "{}",
        json!({"consumer":"PDFium","outline_entries":actual.len(),"pages":session.info().pages.len(),"titles_hierarchy_and_destinations":"pass"})
    );
    Ok(())
}
