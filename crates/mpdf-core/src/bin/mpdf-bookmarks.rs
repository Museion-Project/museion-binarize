use std::{fs, io::Write};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 4 || a[1] != "compile" {
        return Err("usage: mpdf-bookmarks compile EVIDENCE_ARRAY.json NEW_TABLE.json".into());
    }
    let pages: Vec<mpdf_core::bookmarks::local::BookmarkEvidence> =
        serde_json::from_slice(&fs::read(&a[2])?)?;
    let result = mpdf_core::bookmarks::local::compile_local(&pages)?;
    let mut out = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&a[3])?;
    out.write_all(&serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}
