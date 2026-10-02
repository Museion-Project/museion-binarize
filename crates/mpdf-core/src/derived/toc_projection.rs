//! Opt-in semantic projection over an already checked /2 page.
//! The caller pins the image-derived sidecar independently. Python verifies its
//! pixel derivation; this boundary additionally forbids text/identity/geometry
//! edits outside the declared suffix and title sub-box. Hashes are not Gold.
use super::{Bbox, DerivedDocument};
use crate::document_package::DocumentPackage;
use crate::error::{CoreError, Result};
use crate::ocr::{OcrPage, OcrRun};
use crate::ocr_provider::spatial_transcription::CheckedSupportPage;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Projection {
    schema: String,
    policy: String,
    page_index: u32,
    geometry_sha256: String,
    response_sha256: String,
    input_manifest_sha256: String,
    context_sha256: String,
    evidence_sha256: String,
    supports: Vec<ProjectedSupport>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectedSupport {
    support_id: String,
    source_text: String,
    semantic_text: String,
    removed_span: [usize; 2],
    render_bbox: [f64; 4],
    leader_present: bool,
    leader_bbox: Option<[f64; 4]>,
}

fn invalid(s: &str) -> CoreError {
    CoreError::InvalidDocument(s.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn rectangle(b: &[f64; 4]) -> bool {
    b.iter().all(|x| x.is_finite()) && b[0] < b[2] && b[1] < b[3]
}

fn validate_edit(row: &ProjectedSupport, original: [f64; 4], crop: [f64; 4]) -> Result<()> {
    let chars: Vec<char> = row.source_text.chars().collect();
    let [start, end] = row.removed_span;
    if end != chars.len()
        || start > end
        || row.semantic_text.is_empty()
        || !rectangle(&row.render_bbox)
    {
        return Err(invalid("Invalid semantic span or title bounds"));
    }
    if !row.leader_present {
        if start != end
            || row.source_text != row.semantic_text
            || row.render_bbox != original
            || row.leader_bbox.is_some()
        {
            return Err(invalid("Unconfirmed support changed by projection"));
        }
        return Ok(());
    }
    if chars[..start].iter().collect::<String>() != row.semantic_text
        || !chars[start..].iter().all(|c| matches!(c, '.' | ' ' | '\t'))
    {
        return Err(invalid(
            "Projection must only remove declared terminal leader punctuation",
        ));
    }
    let b = row.render_bbox;
    let leader = row
        .leader_bbox
        .ok_or_else(|| invalid("Missing leader evidence bounds"))?;
    if !rectangle(&leader)
        || b[0] != original[0]
        || b[1] != original[1]
        || b[3] != original[3]
        || b[2] >= original[2]
        || b[2] > leader[0]
        || leader[0] < crop[0]
        || leader[1] < crop[1]
        || leader[2] > crop[2]
        || leader[3] > crop[3]
    {
        return Err(invalid(
            "Title/leader projection is outside source support bounds",
        ));
    }
    Ok(())
}

impl DerivedDocument {
    /// All ordinary /2 checks run first. `projections` contains exact sidecar
    /// bytes plus caller-selected SHA per page; no implicit normalization path.
    pub fn from_checked_spatial_projection(
        package: &DocumentPackage,
        ocr: &OcrRun,
        checked: &[(&CheckedSupportPage, &str)],
        projections: &[(&str, &str)],
    ) -> Result<Self> {
        if projections.len() != ocr.pages.len() {
            return Err(invalid("Incomplete semantic projection bindings"));
        }
        let mut document = Self::from_checked_spatial_package(package, ocr, checked)?;
        let mut projection_hashes = Vec::new();
        for (page, (raw, expected)) in ocr.pages.iter().zip(projections) {
            if hash(raw.as_bytes()) != *expected {
                return Err(invalid("Selected semantic projection digest mismatch"));
            }
            document.apply_toc_projection(page, raw, expected)?;
            projection_hashes.push(*expected);
        }
        document.manifest.exporter_version.push_str("+toc-leader/1");
        document.manifest.revision_digest = hash(
            format!(
                "{}:{}",
                document.manifest.revision_digest,
                projection_hashes.join(":")
            )
            .as_bytes(),
        );
        document.validate()?;
        Ok(document)
    }

    fn apply_toc_projection(&mut self, source: &OcrPage, raw: &str, pin: &str) -> Result<()> {
        let projection: Projection =
            serde_json::from_str(raw).map_err(|_| invalid("Invalid typed TOC projection"))?;
        let artifact: Value = serde_json::from_str(
            source
                .provider_raw_artifact
                .as_deref()
                .ok_or_else(|| invalid("Missing checked source artifact"))?,
        )
        .map_err(|_| invalid("Invalid source artifact"))?;
        let geometry_raw = artifact["geometry_json"]
            .as_str()
            .ok_or_else(|| invalid("Missing geometry"))?;
        let manifest_raw = artifact["input_manifest_json"]
            .as_str()
            .ok_or_else(|| invalid("Missing manifest"))?;
        let geometry: Value =
            serde_json::from_str(geometry_raw).map_err(|_| invalid("Invalid geometry"))?;
        let expected_supports = geometry["spatial"]["supports"]
            .as_array()
            .ok_or_else(|| invalid("Missing supports"))?;
        if projection.schema != "mpdf-toc-semantic-projection/1"
            || projection.policy != "toc-dotted-leader/1"
            || projection.page_index != source.page_index
            || projection.geometry_sha256 != hash(geometry_raw.as_bytes())
            || projection.input_manifest_sha256 != hash(manifest_raw.as_bytes())
            || artifact["response_sha256"].as_str() != Some(&projection.response_sha256)
            || !is_hash(&projection.context_sha256)
            || !is_hash(&projection.evidence_sha256)
            || projection.supports.len() != expected_supports.len()
        {
            return Err(invalid("Semantic projection source/contract mismatch"));
        }
        let page = self
            .pages
            .iter_mut()
            .find(|p| p.page_index == source.page_index)
            .ok_or_else(|| invalid("Projected page missing"))?;
        let words: Vec<_> = page
            .blocks
            .iter_mut()
            .flat_map(|b| b.lines.iter_mut())
            .flat_map(|l| l.words.iter_mut())
            .collect();
        if words.len() != projection.supports.len() {
            return Err(invalid("Projected support count differs"));
        }
        for ((word, row), support) in words
            .into_iter()
            .zip(&projection.supports)
            .zip(expected_supports)
        {
            if support["support_id"].as_str() != Some(&row.support_id)
                || word.source_text != row.source_text
                || word.effective_text != row.source_text
            {
                return Err(invalid("Projected support ID/order/source text differs"));
            }
            let original: [f64; 4] = serde_json::from_value(support["bbox"].clone())
                .map_err(|_| invalid("Invalid original support bbox"))?;
            let crop: [f64; 4] = serde_json::from_value(support["crop_bbox"].clone())
                .map_err(|_| invalid("Invalid source crop bbox"))?;
            validate_edit(row, original, crop)?;
            let b = row.render_bbox;
            let sx = page.bbox.width / f64::from(source.width);
            let sy = page.bbox.height / f64::from(source.height);
            word.bbox = Bbox {
                x: b[0] * sx,
                y: b[1] * sy,
                width: (b[2] - b[0]) * sx,
                height: (b[3] - b[1]) * sy,
            };
            word.effective_text = row.semantic_text.clone();
            word.effective_normalized_text = row.semantic_text.clone();
            word.text = row.semantic_text.clone();
            word.normalized_text = row.semantic_text.clone();
            // source_text and source_normalized_text intentionally remain raw.
        }
        page.evidence_digest = hash(format!("{}:{pin}", page.evidence_digest).as_bytes());
        let page_id = page.page_id.clone();
        let words: std::collections::BTreeMap<_, _> = page
            .blocks
            .iter()
            .flat_map(|b| b.lines.iter())
            .flat_map(|l| l.words.iter())
            .map(|w| (w.id.as_str(), w.effective_normalized_text.as_str()))
            .collect();
        for chunk in self.chunks.iter_mut().filter(|c| c.page_id == page_id) {
            let texts: Vec<_> = chunk
                .constituent_word_refs
                .iter()
                .map(|id| {
                    words
                        .get(id.as_str())
                        .copied()
                        .ok_or_else(|| invalid("Chunk projection source missing"))
                })
                .collect::<Result<_>>()?;
            chunk.text = texts.join(" ");
            chunk.id = format!(
                "chunk-{}",
                &hash(format!("{}:{pin}", chunk.id).as_bytes())[..24]
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row() -> ProjectedSupport {
        ProjectedSupport {
            support_id: "s".into(),
            source_text: "F. Title...".into(),
            semantic_text: "F. Title".into(),
            removed_span: [8, 11],
            render_bbox: [0., 0., 40., 20.],
            leader_present: true,
            leader_bbox: Some([45., 10., 98., 15.]),
        }
    }
    #[test]
    fn source_punctuation_and_title_subbox_are_preserved() {
        assert!(validate_edit(&row(), [0., 0., 100., 20.], [0., 0., 100., 20.]).is_ok());
    }
    #[test]
    fn letter_deletion_and_full_envelope_stretch_are_rejected() {
        let mut r = row();
        r.removed_span[0] = 7;
        r.semantic_text = "F. Titl".into();
        assert!(validate_edit(&r, [0., 0., 100., 20.], [0., 0., 100., 20.]).is_err());
        let mut r = row();
        r.render_bbox[2] = 100.;
        assert!(validate_edit(&r, [0., 0., 100., 20.], [0., 0., 100., 20.]).is_err());
    }
    #[test]
    fn uncertain_cannot_change_text_or_bbox() {
        let mut r = row();
        r.leader_present = false;
        assert!(validate_edit(&r, [0., 0., 100., 20.], [0., 0., 100., 20.]).is_err());
    }
}
