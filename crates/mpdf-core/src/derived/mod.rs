//! Deterministic AI-ready derivation from MDP/OCR evidence.
pub mod exporters;
pub mod model;
pub mod review;
pub mod revisions;
use crate::document_package::{DocumentPackage, Rect};
use crate::error::{CoreError, Result};
use crate::ocr::{OcrBlock, OcrLine, OcrRun, OcrWord};
pub use exporters::*;
#[cfg(test)]
mod tests;
pub use model::*;
pub use review::*;
pub use revisions::*;
use sha2::{Digest, Sha256};
pub const MAX_BUNDLE_ARTIFACTS: usize = 32;
impl DerivedDocument {
    pub fn from_package(p: &DocumentPackage, ocr: Option<&OcrRun>) -> Result<Self> {
        p.validate()?;
        if let Some(r) = ocr {
            r.validate()
                .map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
            if !r.is_complete(p.manifest.page_count) {
                return Err(CoreError::InvalidDocument(
                    "OCR evidence is incomplete".into(),
                ));
            }
            if r.pages.iter().any(|page| {
                page.page_index >= p.manifest.page_count
                    || !p
                        .pages
                        .iter()
                        .any(|source| source.physical_index == page.page_index)
            }) {
                return Err(CoreError::InvalidDocument(
                    "OCR evidence page identity does not match package".into(),
                ));
            }
        }
        let pd =
            digest(&serde_json::to_vec(p).map_err(|e| CoreError::InvalidDocument(e.to_string()))?);
        let od = ocr.map(|r| digest(&serde_json::to_vec(r).unwrap()));
        let mut pages = Vec::new();
        let mut chunks = Vec::new();
        for pge in &p.pages {
            let ev = ocr.and_then(|r| r.pages.iter().find(|x| x.page_index == pge.physical_index));
            let ed = ev
                .map(|x| digest(&serde_json::to_vec(x).unwrap()))
                .unwrap_or_else(|| digest(pge.page_id.as_bytes()));
            let blocks: Vec<DerivedBlock> = ev
                .map(|x| {
                    x.blocks
                        .iter()
                        .enumerate()
                        .map(|(i, b)| {
                            block(
                                &pge.page_id,
                                &pge.master_space.id,
                                i,
                                b,
                                x.width,
                                x.height,
                                pge.master_space.width,
                                pge.master_space.height,
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            for b in &blocks {
                for l in &b.lines {
                    let t = l
                        .words
                        .iter()
                        .map(|w| w.effective_normalized_text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !t.is_empty() {
                        chunks.push(DerivedChunk {
                            id: stable("chunk", &[&pge.page_id, &l.id, &t]),
                            document_id: p.manifest.document_id.clone(),
                            page_id: pge.page_id.clone(),
                            page_index: pge.physical_index,
                            bbox: l.bbox,
                            coordinate_space: l.coordinate_space.clone(),
                            structural_path: format!("{}.chunk", l.structural_path),
                            constituent_word_refs: l.words.iter().map(|w| w.id.clone()).collect(),
                            text: t,
                            reading_order: l.reading_order,
                        });
                    }
                }
            }
            pages.push(DerivedPage {
                page_id: pge.page_id.clone(),
                page_index: pge.physical_index,
                bbox: Bbox {
                    x: 0.,
                    y: 0.,
                    width: pge.master_space.width,
                    height: pge.master_space.height,
                },
                coordinate_space: pge.master_space.id.clone(),
                evidence_digest: ed,
                blocks,
                regions: pge
                    .region_evidence
                    .iter()
                    .enumerate()
                    .map(|(i, r)| DerivedRegion {
                        id: stable("region", &[&pge.page_id, &i.to_string(), &r.kind]),
                        page_id: pge.page_id.clone(),
                        bbox: rect_master(&r.bounds, pge),
                        kind: r.kind.clone(),
                        bounds: r.bounds.clone(),
                    })
                    .collect(),
                outline_evidence: pge
                    .existing_outline_evidence
                    .iter()
                    .map(|x| OutlineEvidenceRef {
                        title: x.title.clone(),
                        level: x.level,
                        target_page_id: x.target_page_id.clone(),
                        source: x.source.clone(),
                    })
                    .collect(),
                printed_page_label: pge.printed_page_label.clone(),
                existing_outline_evidence: pge.existing_outline_evidence.clone(),
                typography_evidence: pge.typography_evidence.clone(),
                region_evidence: pge.region_evidence.clone(),
            });
        }
        chunks.sort_by(|a, b| {
            (a.page_index, &a.structural_path).cmp(&(b.page_index, &b.structural_path))
        });
        Ok(Self {
            manifest: DerivedManifest {
                schema: DERIVED_SCHEMA.into(),
                schema_version: DERIVED_SCHEMA_VERSION.into(),
                source_digest: p.source.content_sha256.clone(),
                document_id: p.manifest.document_id.clone(),
                package_digest: pd,
                ocr_digest: od,
                revision_digest: digest(b"[]"),
                exporter_version: DERIVED_EXPORTER_VERSION.into(),
                artifacts: vec![],
            },
            pages,
            chunks,
        })
    }

    /// Compare all inputs that affect a derived bundle.  Exporter version is
    /// part of the identity so upgrading formatting code invalidates old
    /// artifacts instead of silently mixing generations.
    pub fn is_stale(
        &self,
        package_digest: &str,
        ocr_digest: Option<&str>,
        revision_digest: &str,
        exporter_version: &str,
    ) -> bool {
        self.manifest.package_digest != package_digest
            || self.manifest.ocr_digest.as_deref() != ocr_digest
            || self.manifest.revision_digest != revision_digest
            || self.manifest.exporter_version != exporter_version
    }

    pub fn is_stale_with_revisions(
        &self,
        package_digest: &str,
        ocr_digest: Option<&str>,
        revisions: &RevisionStore,
        exporter_version: &str,
    ) -> Result<bool> {
        revisions.validate()?;
        let revision_digest = digest(
            &serde_json::to_vec(revisions)
                .map_err(|e| CoreError::InvalidDocument(e.to_string()))?,
        );
        Ok(self.is_stale(
            package_digest,
            ocr_digest,
            &revision_digest,
            exporter_version,
        ))
    }
}
#[allow(clippy::too_many_arguments)]
fn block(
    pid: &str,
    space: &str,
    i: usize,
    b: &OcrBlock,
    sw: u32,
    sh: u32,
    mw: f64,
    mh: f64,
) -> DerivedBlock {
    let path = format!("p{pid}/b{i:06}");
    let lines = b
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| line(pid, space, &path, i, l, sw, sh, mw, mh))
        .collect();
    DerivedBlock {
        id: stable("block", &[&path, &bbox_key(&b.bbox)]),
        page_id: pid.into(),
        bbox: scale(&b.bbox, sw, sh, mw, mh),
        coordinate_space: space.into(),
        structural_path: path,
        reading_order: b.reading_order,
        lines,
    }
}
#[allow(clippy::too_many_arguments)]
fn line(
    pid: &str,
    space: &str,
    parent: &str,
    i: usize,
    l: &OcrLine,
    sw: u32,
    sh: u32,
    mw: f64,
    mh: f64,
) -> DerivedLine {
    let path = format!("{parent}/l{i:06}");
    let words = l
        .words
        .iter()
        .enumerate()
        .map(|(i, w)| word(pid, space, &path, i, w, sw, sh, mw, mh))
        .collect();
    DerivedLine {
        id: stable("line", &[&path, &bbox_key(&l.bbox)]),
        page_id: pid.into(),
        bbox: scale(&l.bbox, sw, sh, mw, mh),
        coordinate_space: space.into(),
        structural_path: path,
        reading_order: l.reading_order,
        words,
    }
}
#[allow(clippy::too_many_arguments)]
fn word(
    pid: &str,
    space: &str,
    parent: &str,
    i: usize,
    w: &OcrWord,
    sw: u32,
    sh: u32,
    mw: f64,
    mh: f64,
) -> DerivedWord {
    let path = format!("{parent}/w{i:06}");
    DerivedWord {
        id: stable(
            "word",
            &[parent, &i.to_string(), &bbox_key(&w.bbox), &w.text],
        ),
        page_id: pid.into(),
        bbox: scale(&w.bbox, sw, sh, mw, mh),
        coordinate_space: space.into(),
        structural_path: path,
        source_text: w.text.clone(),
        source_normalized_text: w.normalized_text.clone(),
        effective_text: w.text.clone(),
        effective_normalized_text: w.normalized_text.clone(),
        text: w.text.clone(),
        normalized_text: w.normalized_text.clone(),
        confidence: w.confidence,
        reading_order: w.reading_order,
    }
}
fn scale(b: &crate::ocr::OcrBox, sw: u32, sh: u32, mw: f64, mh: f64) -> Bbox {
    Bbox {
        x: b.x as f64 * mw / sw.max(1) as f64,
        y: b.y as f64 * mh / sh.max(1) as f64,
        width: b.width as f64 * mw / sw.max(1) as f64,
        height: b.height as f64 * mh / sh.max(1) as f64,
    }
}
fn rect_master(r: &Rect, p: &crate::document_package::Page) -> Bbox {
    if r.space_id == p.master_space.id {
        return Bbox {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        };
    }
    let t = &p.transforms[0];
    let c = [
        (r.x, r.y),
        (r.x + r.width, r.y),
        (r.x, r.y + r.height),
        (r.x + r.width, r.y + r.height),
    ]
    .map(|(x, y)| (t.a * x + t.c * y + t.e, t.b * x + t.d * y + t.f));
    let xs = c.iter().map(|x| x.0);
    let ys = c.iter().map(|x| x.1);
    let minx = xs.clone().fold(f64::INFINITY, f64::min);
    let maxx = xs.fold(f64::NEG_INFINITY, f64::max);
    let miny = ys.clone().fold(f64::INFINITY, f64::min);
    let maxy = ys.fold(f64::NEG_INFINITY, f64::max);
    Bbox {
        x: minx,
        y: miny,
        width: maxx - minx,
        height: maxy - miny,
    }
}
fn bbox_key(b: &crate::ocr::OcrBox) -> String {
    format!("{:.6},{:.6},{:.6},{:.6}", b.x, b.y, b.width, b.height)
}
fn stable(k: &str, p: &[&str]) -> String {
    let mut h = Sha256::new();
    h.update(k.as_bytes());
    for x in p {
        h.update([0]);
        h.update(x.as_bytes())
    }
    format!("{k}-{}", &digest(&h.finalize())[..24])
}
fn digest(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}
