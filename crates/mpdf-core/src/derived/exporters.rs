use super::*;
use crate::document_package::{validate_relative_path, DocumentPackage};
use crate::error::{CoreError, Result};
use crate::ocr::OcrRun;
use serde_json;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Jsonl,
    Markdown,
    Text,
    Html,
    Hocr,
    Alto,
}
impl ExportFormat {
    pub fn parse(v: &str) -> Option<Self> {
        Some(match v {
            "json" => Self::Json,
            "jsonl" => Self::Jsonl,
            "md" | "markdown" => Self::Markdown,
            "txt" | "text" => Self::Text,
            "html" => Self::Html,
            "hocr" => Self::Hocr,
            "alto" | "alto_xml" => Self::Alto,
            _ => return None,
        })
    }
}
pub fn export(d: &DerivedDocument, f: ExportFormat) -> Result<String> {
    d.validate()?;
    Ok(match f {
        ExportFormat::Json => format!(
            "{}\n",
            serde_json::to_string_pretty(d).map_err(|e| err(e.to_string()))?
        ),
        ExportFormat::Jsonl => {
            d.chunks
                .iter()
                .map(|x| serde_json::to_string(x).map_err(|e| err(e.to_string())))
                .collect::<Result<Vec<_>>>()?
                .join("\n")
                + "\n"
        }
        ExportFormat::Markdown => markdown(d),
        ExportFormat::Text => text(d),
        ExportFormat::Html => html(d, false),
        ExportFormat::Hocr => html(d, true),
        ExportFormat::Alto => alto(d),
    })
}
pub fn write_export(p: &Path, c: &str, overwrite: bool) -> Result<()> {
    if let Ok(m) = fs::symlink_metadata(p) {
        if m.file_type().is_symlink() {
            return Err(err("export destination is symlink".into()));
        }
        if !overwrite {
            return Err(CoreError::DestinationConflict(p.display().to_string()));
        }
    }
    let parent = p.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
    let mut t = tempfile::NamedTempFile::new_in(parent).map_err(|e| CoreError::io(parent, e))?;
    t.write_all(c.as_bytes()).map_err(|e| CoreError::io(p, e))?;
    t.as_file().sync_all().map_err(|e| CoreError::io(p, e))?;
    if overwrite {
        t.persist(p).map_err(|e| CoreError::io(p, e.error))?;
    } else {
        fs::hard_link(t.path(), p).map_err(|e| CoreError::io(p, e))?;
    }
    Ok(())
}
pub fn verify_bundle(
    root: &Path,
    p: &DocumentPackage,
    o: Option<&OcrRun>,
    r: &crate::derived::RevisionStore,
) -> Result<BundleStatus> {
    let m = root.join("derived-manifest.json");
    let md = fs::symlink_metadata(&m).map_err(|_| err("bundle manifest missing".into()))?;
    if md.file_type().is_symlink() || !md.is_file() || md.len() > 1_048_576 {
        return Ok(BundleStatus::Corrupt);
    };
    let stored: DerivedManifest =
        serde_json::from_slice(&fs::read(&m).map_err(|e| CoreError::io(&m, e))?)
            .map_err(|e| err(e.to_string()))?;
    if stored.schema != DERIVED_SCHEMA
        || stored.schema_version != DERIVED_SCHEMA_VERSION
        || stored.artifacts.len() != 7
    {
        return Ok(BundleStatus::Corrupt);
    }
    let cur = DerivedDocument::from_package(p, o)?;
    let rd = digest(&serde_json::to_vec(r).map_err(|e| err(e.to_string()))?);
    if stored.document_id != cur.manifest.document_id
        || stored.source_digest != cur.manifest.source_digest
        || stored.package_digest != cur.manifest.package_digest
        || stored.ocr_digest != cur.manifest.ocr_digest
        || stored.revision_digest != rd
        || stored.exporter_version != cur.manifest.exporter_version
    {
        return Ok(BundleStatus::Stale);
    };
    let mut expected = std::collections::HashMap::from([
        ("derived.json", "json"),
        ("derived.jsonl", "jsonl"),
        ("derived.md", "md"),
        ("derived.txt", "txt"),
        ("derived.html", "html"),
        ("derived.hocr.html", "hocr.html"),
        ("derived.alto.xml", "alto.xml"),
    ]);
    let mut names = std::collections::HashSet::from(["derived-manifest.json".to_string()]);
    for a in stored.artifacts {
        let Some(expected_format) = expected.remove(a.path.as_str()) else {
            return Ok(BundleStatus::Corrupt);
        };
        if a.format != expected_format
            || validate_relative_path(&a.path).is_err()
            || a.sha256.len() != 64
            || !a
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Ok(BundleStatus::Corrupt);
        };
        let q = root.join(&a.path);
        let x = fs::symlink_metadata(&q).map_err(|_| err("artifact missing".into()))?;
        if x.file_type().is_symlink()
            || !x.is_file()
            || x.len() != a.byte_len
            || digest(&fs::read(&q).map_err(|e| CoreError::io(&q, e))?) != a.sha256
        {
            return Ok(BundleStatus::Corrupt);
        };
        names.insert(a.path);
    }
    if !expected.is_empty() {
        return Ok(BundleStatus::Corrupt);
    }
    for e in fs::read_dir(root).map_err(|e| CoreError::io(root, e))? {
        let e = e.map_err(|e| CoreError::io(root, e))?;
        if !names.contains(&e.file_name().to_string_lossy().to_string()) {
            return Ok(BundleStatus::Corrupt);
        }
    }
    Ok(BundleStatus::Current)
}
fn digest(v: &[u8]) -> String {
    Sha256::digest(v)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn err(s: String) -> CoreError {
    CoreError::InvalidDocument(s)
}
fn markdown(d: &DerivedDocument) -> String {
    d.pages
        .iter()
        .map(|p| {
            format!(
                "## Page {} [{}]\n\n{}\n---\n\n",
                p.page_index + 1,
                p.page_id,
                p.blocks
                    .iter()
                    .flat_map(|b| b.lines.iter())
                    .map(|l| l
                        .words
                        .iter()
                        .map(|w| esc_md(&w.effective_normalized_text))
                        .collect::<Vec<_>>()
                        .join(" "))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })
        .collect()
}
fn text(d: &DerivedDocument) -> String {
    d.pages
        .iter()
        .map(|p| {
            format!(
                "=== Page {} [{}] ===\n{}\n",
                p.page_index + 1,
                p.page_id,
                d.chunks
                    .iter()
                    .filter(|c| c.page_id == p.page_id)
                    .map(|c| c.text.clone())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })
        .collect()
}
fn html(d: &DerivedDocument, h: bool) -> String {
    let mut s = format!(
        "<!doctype html><html><body data-document-id=\"{}\">\n",
        esc(&d.manifest.document_id)
    );
    for p in &d.pages {
        s+=&format!("<section class=\"{}\" id=\"{}\" data-page-id=\"{}\" data-coordinate-space=\"{}\" title=\"bbox {}\">\n",if h{"ocr_page"}else{"mpdf-page"},esc(&p.page_id),esc(&p.page_id),esc(&p.coordinate_space),bb(p.bbox));
        for b in &p.blocks {
            s += &format!(
                "<div class=\"{}\" id=\"{}\" title=\"bbox {}\">",
                if h { "ocr_carea" } else { "mpdf-block" },
                esc(&b.id),
                bb(b.bbox)
            );
            for l in &b.lines {
                s += &format!(
                    "<span class=\"{}\" id=\"{}\" title=\"bbox {}\">",
                    if h { "ocr_line" } else { "mpdf-line" },
                    esc(&l.id),
                    bb(l.bbox)
                );
                for w in &l.words {
                    s += &format!(
                        "<span class=\"{}\" id=\"{}\" title=\"bbox {}{}\">{}</span> ",
                        if h { "ocrx_word" } else { "mpdf-word" },
                        esc(&w.id),
                        bb(w.bbox),
                        if h {
                            format!(" x_wconf {}", w.confidence * 100.0)
                        } else {
                            String::new()
                        },
                        esc(&w.effective_text)
                    );
                }
                s += "</span>";
            }
            s += "</div></section>\n";
        }
    }
    s += "</body></html>\n";
    s
}
fn alto(d: &DerivedDocument) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\"?><alto xmlns=\"http://www.loc.gov/standards/alto/ns-v4#\" DOCUMENT_ID=\"{}\"><Layout>",
        esc(&d.manifest.document_id)
    );
    for p in &d.pages {
        s += &format!(
            "<Page ID=\"{}\" WIDTH=\"{}\" HEIGHT=\"{}\"><PrintSpace>",
            esc(&p.page_id),
            p.bbox.width,
            p.bbox.height
        );
        for b in &p.blocks {
            s += &format!("<TextBlock ID=\"{}\">", esc(&b.id));
            for l in &b.lines {
                s += &format!("<TextLine ID=\"{}\">", esc(&l.id));
                for w in &l.words {
                    s+=&format!("<String ID=\"{}\" HPOS=\"{}\" VPOS=\"{}\" WIDTH=\"{}\" HEIGHT=\"{}\" WC=\"{}\" CONTENT=\"{}\"/>",esc(&w.id),w.bbox.x,w.bbox.y,w.bbox.width,w.bbox.height,w.confidence,esc(&w.effective_text));
                }
                s += "</TextLine>";
            }
            s += "</TextBlock>";
        }
        s += "</PrintSpace></Page>";
    }
    s += "</Layout></alto>\n";
    s
}
fn esc(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn esc_md(v: &str) -> String {
    v.chars()
        .flat_map(|c| {
            if "\\`*_[]#<>|".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}
fn bb(b: Bbox) -> String {
    format!("{} {} {} {}", b.x, b.y, b.x + b.width, b.y + b.height)
}
