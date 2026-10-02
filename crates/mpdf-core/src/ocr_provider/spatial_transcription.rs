//! Opt-in support composition /2. The /1 provider factory remains unchanged.
//! Logical membership never supplies text geometry. PDF positioning uses each
//! support's measured box; a line envelope is only an OcrPage container.
use super::geometry_transcription::{
    self as old, GeometryBoundTranscription, GeometryLine, GeometryPage,
    GeometryTranscriptionError, GeometryValidationStatus, TranscribedLine,
};
use crate::ocr::{validate_ocr_page, OcrBox, OcrPage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
pub const CONTRACT: &str = "mpdf-spatial-transcription/2";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    contract: String,
    page_index: u32,
    image_sha256: String,
    spatial: Spatial,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spatial {
    version: String,
    page_id: String,
    provider: String,
    width: u32,
    height: u32,
    supports: Vec<Support>,
    units: Vec<Unit>,
    parent_geometry_sha256: String,
    spatial_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Support {
    support_id: String,
    reading_order: u32,
    fragment_ids: Vec<String>,
    bbox: [f64; 4],
    crop_bbox: [f64; 4],
    regions: Vec<Region>,
    column_id: Value,
    band_id: Value,
}
#[derive(Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Region {
    fragment_id: String,
    bbox: [f64; 4],
    polygon: Vec<[f64; 2]>,
    crop_bbox: [f64; 4],
    crop_parent_unit_id: String,
    crop_parent_bbox: [f64; 4],
    source_fragment_id: String,
    source_pointer: String,
    column_id: Value,
    band_id: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Unit {
    unit_id: String,
    reading_order: u32,
    support_ids: Vec<String>,
    column_id: Value,
    band_id: Value,
    membership_evidence: Value,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupportText {
    pub support_id: String,
    pub text: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub geometry_sha256: String,
    pub supports: Vec<SupportText>,
}
pub struct SupportPage {
    raw: String,
    envelope: Envelope,
}
/// Identities selected by the caller independently of submitted contract bytes.
/// The input manifest is produced by checked Python preparation, which verifies
/// the existing Python canonical hashes and the actual materialized PNGs.
/// These are content pins, not signatures or an authentication mechanism.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedSupportBindings {
    pub contract_sha256: String,
    pub parent_geometry_sha256: String,
    pub parent_artifact_sha256: String,
    pub spatial_artifact_sha256: String,
    pub image_sha256: String,
    pub input_manifest_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputManifest {
    schema: String,
    geometry_sha256: String,
    parent_geometry_sha256: String,
    parent_artifact_sha256: String,
    spatial_artifact_sha256: String,
    spatial_sha256: String,
    image_sha256: String,
    payload_sha256: String,
    inputs: Vec<InputRecord>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputRecord {
    support_id: String,
    png_sha256: String,
    regions: Vec<Region>,
}
/// Operational /2 entry point. Unlike SupportPage's historical structural
/// reader, this binds the selected contract, parent, image and input manifest.
pub struct CheckedSupportPage {
    page: SupportPage,
    input_manifest_json: String,
    expected: ExpectedSupportBindings,
}
fn invalid(message: &str) -> GeometryTranscriptionError {
    GeometryTranscriptionError::InvalidGeometry(message.into())
}
fn digest(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}
fn sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn contains(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] <= b[0] && a[1] <= b[1] && a[2] >= b[2] && a[3] >= b[3]
}
fn bounds<'a>(iter: impl Iterator<Item = &'a [f64; 4]>) -> [f64; 4] {
    iter.fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        },
    )
}
fn bbox(b: &[f64; 4]) -> OcrBox {
    OcrBox {
        x: b[0] as f32,
        y: b[1] as f32,
        width: (b[2] - b[0]) as f32,
        height: (b[3] - b[1]) as f32,
    }
}
impl SupportPage {
    /// Structural-only historical reader. This cannot independently verify the
    /// Python canonical hashes or the identity of a caller-selected parent.
    /// Operational callers should use CheckedSupportPage::from_json.
    pub fn from_json(raw: &str) -> Result<Self, GeometryTranscriptionError> {
        let e: Envelope = serde_json::from_str(raw).map_err(|_| invalid("Invalid /2 schema"))?;
        let g = &e.spatial;
        if e.contract != CONTRACT
            || g.version != "spatial-supports/1"
            || g.page_id.is_empty()
            || !sha(&g.parent_geometry_sha256)
            || !sha(&g.spatial_sha256)
            || g.units.is_empty()
            || g.units.len() > g.supports.len()
        {
            return Err(invalid("Invalid /2 identity"));
        }
        let valid = |b: &[f64; 4]| {
            b.iter().all(|x| x.is_finite())
                && 0. <= b[0]
                && 0. <= b[1]
                && b[0] < b[2]
                && b[1] < b[3]
                && b[2] <= g.width as f64
                && b[3] <= g.height as f64
        };
        let mut fragments = BTreeSet::new();
        for s in &g.supports {
            if s.regions.is_empty()
                || !valid(&s.bbox)
                || !valid(&s.crop_bbox)
                || s.bbox != bounds(s.regions.iter().map(|r| &r.bbox))
                || s.crop_bbox != bounds(s.regions.iter().map(|r| &r.crop_bbox))
                || s.fragment_ids
                    != s.regions
                        .iter()
                        .map(|r| r.fragment_id.clone())
                        .collect::<Vec<_>>()
            {
                return Err(invalid("Invalid supports"));
            }
            for r in &s.regions {
                if !valid(&r.bbox)
                    || !valid(&r.crop_bbox)
                    || !valid(&r.crop_parent_bbox)
                    || !contains(&r.crop_bbox, &r.bbox)
                    || !contains(&r.crop_parent_bbox, &r.crop_bbox)
                    || r.fragment_id.is_empty()
                    || !fragments.insert(&r.fragment_id)
                    || r.polygon.len() < 3
                    || r.polygon.iter().any(|p| {
                        !p.iter().all(|x| x.is_finite())
                            || p[0] < 0.
                            || p[1] < 0.
                            || p[0] > g.width as f64
                            || p[1] > g.height as f64
                    })
                    || r.source_pointer.is_empty()
                    || r.source_fragment_id.is_empty()
                    || r.crop_parent_unit_id.is_empty()
                    || r.column_id != s.column_id
                    || r.band_id != s.band_id
                {
                    return Err(invalid("Unsafe crop or provenance"));
                }
            }
        }
        let mut units = BTreeSet::new();
        let mut cursor = 0;
        for (i, u) in g.units.iter().enumerate() {
            if u.unit_id.is_empty()
                || u.unit_id.len() > 128
                || !units.insert(&u.unit_id)
                || u.reading_order != i as u32
                || u.support_ids.is_empty()
                || !u.membership_evidence.is_array()
            {
                return Err(invalid("Invalid unit"));
            }
            for id in &u.support_ids {
                let s = g
                    .supports
                    .get(cursor)
                    .ok_or_else(|| invalid("Invalid ownership"))?;
                if id != &s.support_id || u.column_id != s.column_id || u.band_id != s.band_id {
                    return Err(invalid("Ownership/order/region mismatch"));
                }
                cursor += 1;
            }
        }
        if cursor != g.supports.len() {
            return Err(invalid("Unowned supports"));
        }
        let result = Self {
            raw: raw.into(),
            envelope: e,
        };
        old::validate_geometry(&result.physical())?;
        Ok(result)
    }
    pub fn digest(&self) -> String {
        digest(self.raw.as_bytes())
    }
    fn physical(&self) -> GeometryPage {
        let e = &self.envelope;
        let g = &e.spatial;
        GeometryPage {
            page_index: e.page_index,
            width: g.width,
            height: g.height,
            image_sha256: e.image_sha256.clone(),
            provider_id: g.provider.clone(),
            provider_version: g.version.clone(),
            validation_status: GeometryValidationStatus::HistoricalMaterialNotValidated,
            lines: g
                .supports
                .iter()
                .map(|s| GeometryLine {
                    line_id: s.support_id.clone(),
                    reading_order: s.reading_order,
                    bbox: bbox(&s.bbox),
                    confidence: None,
                })
                .collect(),
        }
    }
    pub fn compose(
        &self,
        response: Response,
        provider: &str,
        model: &str,
        model_version: &str,
        usage: super::ProviderUsage,
    ) -> Result<OcrPage, GeometryTranscriptionError> {
        if response.geometry_sha256 != self.digest() {
            return Err(invalid("/2 digest mismatch"));
        }
        for (index, support) in response.supports.iter().enumerate() {
            crate::transcription_fidelity::require_lossless(
                &support.text,
                &format!("/2 support {index}"),
            )
            .map_err(|error| invalid(&error.to_string()))?;
        }
        let artifact=serde_json::json!({"contract":CONTRACT,"geometry_json":self.raw,"transcription":response,"usage":usage}).to_string();
        let geometry = self.physical();
        let text = GeometryBoundTranscription {
            page_index: geometry.page_index,
            geometry_sha256: geometry.digest()?,
            lines: response
                .supports
                .into_iter()
                .map(|t| TranscribedLine {
                    line_id: t.support_id,
                    text: t.text,
                    language: None,
                    confidence: None,
                })
                .collect(),
            provider_id: provider.into(),
            model: model.into(),
            model_version: model_version.into(),
            usage,
        };
        let mut page = old::compose_ocr_page(&geometry, &text)?;
        let originals = page.blocks[0].lines.clone();
        let mut cursor = 0;
        let mut lines = vec![];
        for unit in &self.envelope.spatial.units {
            let mut line = originals[cursor].clone();
            line.reading_order = unit.reading_order;
            line.words.clear();
            let end = cursor + unit.support_ids.len();
            for source in &originals[cursor..end] {
                let mut word = source.words[0].clone();
                word.reading_order = line.words.len() as u32;
                line.words.push(word);
            }
            let b = bounds(
                self.envelope.spatial.supports[cursor..end]
                    .iter()
                    .map(|s| &s.bbox),
            );
            line.bbox = bbox(&b);
            lines.push(line);
            cursor = end;
        }
        page.blocks[0].lines = lines;
        if let Some(p) = page.provider_provenance.as_mut() {
            p.version = CONTRACT.into();
            p.parameters.insert("contract".into(), CONTRACT.into());
            p.parameters.insert("geometry_sha256".into(), self.digest());
            p.parameters.insert(
                "spatial_artifact_sha256".into(),
                digest(artifact.as_bytes()),
            );
        }
        page.provider_raw_artifact = Some(artifact);
        validate_ocr_page(&page).map_err(GeometryTranscriptionError::Composition)?;
        Ok(page)
    }
}

impl CheckedSupportPage {
    pub fn from_json(
        raw: &str,
        input_manifest_json: &str,
        expected: &ExpectedSupportBindings,
    ) -> Result<Self, GeometryTranscriptionError> {
        if [
            &expected.contract_sha256,
            &expected.parent_geometry_sha256,
            &expected.parent_artifact_sha256,
            &expected.spatial_artifact_sha256,
            &expected.image_sha256,
            &expected.input_manifest_sha256,
        ]
        .into_iter()
        .any(|s| !sha(s))
            || digest(raw.as_bytes()) != expected.contract_sha256
            || digest(input_manifest_json.as_bytes()) != expected.input_manifest_sha256
        {
            return Err(invalid("Selected /2 contract or manifest digest mismatch"));
        }
        let page = SupportPage::from_json(raw)?;
        let manifest: InputManifest = serde_json::from_str(input_manifest_json)
            .map_err(|_| invalid("Invalid /2 input manifest"))?;
        let g = &page.envelope.spatial;
        if manifest.schema != "mpdf-spatial-input-manifest/1"
            || manifest.geometry_sha256 != expected.contract_sha256
            || g.parent_geometry_sha256 != expected.parent_geometry_sha256
            || manifest.parent_geometry_sha256 != expected.parent_geometry_sha256
            || manifest.parent_artifact_sha256 != expected.parent_artifact_sha256
            || manifest.spatial_artifact_sha256 != expected.spatial_artifact_sha256
            || manifest.image_sha256 != expected.image_sha256
            || page.envelope.image_sha256 != expected.image_sha256
            || manifest.spatial_sha256 != g.spatial_sha256
            || !sha(&manifest.payload_sha256)
            || manifest.inputs.len() != g.supports.len()
        {
            return Err(invalid(
                "Selected /2 parent, image or input identity mismatch",
            ));
        }
        for (input, support) in manifest.inputs.iter().zip(&g.supports) {
            if input.support_id != support.support_id
                || !sha(&input.png_sha256)
                || input.regions != support.regions
            {
                return Err(invalid("Materialized /2 support mapping mismatch"));
            }
        }
        Ok(Self {
            page,
            input_manifest_json: input_manifest_json.into(),
            expected: expected.clone(),
        })
    }

    pub fn digest(&self) -> &str {
        &self.expected.contract_sha256
    }

    pub fn compose(
        &self,
        response_json: &str,
        expected_response_sha256: &str,
        provider: &str,
        model: &str,
        model_version: &str,
        usage: super::ProviderUsage,
    ) -> Result<OcrPage, GeometryTranscriptionError> {
        if !sha(expected_response_sha256)
            || digest(response_json.as_bytes()) != expected_response_sha256
        {
            return Err(invalid("Selected /2 response digest mismatch"));
        }
        let response: Response = serde_json::from_str(response_json)
            .map_err(|_| invalid("Invalid selected /2 response"))?;
        let mut result = self
            .page
            .compose(response, provider, model, model_version, usage)?;
        let mut artifact: Value = serde_json::from_str(
            result
                .provider_raw_artifact
                .as_deref()
                .ok_or_else(|| invalid("Missing /2 composition artifact"))?,
        )
        .map_err(|_| invalid("Invalid /2 composition artifact"))?;
        artifact["input_manifest_json"] = Value::String(self.input_manifest_json.clone());
        artifact["response_json"] = Value::String(response_json.into());
        artifact["response_sha256"] = Value::String(expected_response_sha256.into());
        let artifact = artifact.to_string();
        if let Some(p) = result.provider_provenance.as_mut() {
            p.parameters.insert(
                "spatial_artifact_sha256".into(),
                digest(artifact.as_bytes()),
            );
            p.parameters.insert(
                "input_manifest_sha256".into(),
                self.expected.input_manifest_sha256.clone(),
            );
            p.parameters
                .insert("response_sha256".into(), expected_response_sha256.into());
            p.parameters.insert(
                "parent_geometry_sha256".into(),
                self.expected.parent_geometry_sha256.clone(),
            );
            p.parameters.insert(
                "parent_artifact_sha256".into(),
                self.expected.parent_artifact_sha256.clone(),
            );
            p.parameters
                .insert("evidence_binding".into(), "checked-spatial-inputs/1".into());
        }
        result.provider_raw_artifact = Some(artifact);
        validate_ocr_page(&result).map_err(GeometryTranscriptionError::Composition)?;
        Ok(result)
    }

    /// Verify retained artifact bytes against an independently retained output
    /// digest and this checked input. The expected output digest must not be
    /// derived from the artifact being verified.
    pub fn verify_artifact(
        &self,
        artifact: &str,
        expected_artifact_sha256: &str,
    ) -> Result<(), GeometryTranscriptionError> {
        if !sha(expected_artifact_sha256) || digest(artifact.as_bytes()) != expected_artifact_sha256
        {
            return Err(invalid("Retained /2 artifact digest mismatch"));
        }
        let value: Value =
            serde_json::from_str(artifact).map_err(|_| invalid("Invalid retained /2 artifact"))?;
        if value.as_object().map(|o| o.len()) != Some(7)
            || value["contract"] != CONTRACT
            || value["geometry_json"] != self.page.raw
            || value["input_manifest_json"] != self.input_manifest_json
            || !value.get("usage").is_some_and(Value::is_object)
        {
            return Err(invalid("Retained /2 artifact input mismatch"));
        }
        let raw_response = value["response_json"]
            .as_str()
            .ok_or_else(|| invalid("Missing retained /2 response bytes"))?;
        if value["response_sha256"] != digest(raw_response.as_bytes())
            || serde_json::from_str::<Value>(raw_response)
                .map_err(|_| invalid("Invalid retained /2 response bytes"))?
                != value["transcription"]
        {
            return Err(invalid("Retained /2 response digest mismatch"));
        }
        let response: Response = serde_json::from_str(raw_response)
            .map_err(|_| invalid("Invalid retained /2 response"))?;
        self.page.compose(
            response,
            "artifact-verification",
            "fixture",
            "fixture",
            Default::default(),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_contract_response_and_retained_artifact_require_external_pins() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/evidence/spatial-support-contract-2026-09-08/phase2/surya/brisson-le-meme-pdf013.contract.json");
        let raw = std::fs::read_to_string(path).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        // Manifest is a unit fixture, not real materialization evidence. The
        // separate full50 integration uses actual Python-materialized PNGs.
        let manifest = serde_json::json!({
            "schema":"mpdf-spatial-input-manifest/1", "geometry_sha256":digest(raw.as_bytes()),
            "parent_geometry_sha256":value["spatial"]["parent_geometry_sha256"],
            "parent_artifact_sha256":"1".repeat(64), "spatial_artifact_sha256":"2".repeat(64),
            "spatial_sha256":value["spatial"]["spatial_sha256"], "image_sha256":value["image_sha256"],
            "payload_sha256":"3".repeat(64),
            "inputs":value["spatial"]["supports"].as_array().unwrap().iter().map(|s|
                serde_json::json!({"support_id":s["support_id"],"regions":s["regions"],"png_sha256":"4".repeat(64)})).collect::<Vec<_>>()
        }).to_string();
        let expected = ExpectedSupportBindings {
            contract_sha256: digest(raw.as_bytes()),
            parent_geometry_sha256: value["spatial"]["parent_geometry_sha256"]
                .as_str()
                .unwrap()
                .into(),
            parent_artifact_sha256: "1".repeat(64),
            spatial_artifact_sha256: "2".repeat(64),
            image_sha256: value["image_sha256"].as_str().unwrap().into(),
            input_manifest_sha256: digest(manifest.as_bytes()),
        };
        let checked = CheckedSupportPage::from_json(&raw, &manifest, &expected).unwrap();
        for field in ["parent_geometry_sha256", "spatial_sha256"] {
            let mut changed = value.clone();
            changed["spatial"][field] = Value::String("0".repeat(64));
            assert!(
                CheckedSupportPage::from_json(&changed.to_string(), &manifest, &expected).is_err()
            );
        }
        let mut wrong = expected.clone();
        wrong.parent_geometry_sha256 = "0".repeat(64);
        assert!(CheckedSupportPage::from_json(&raw, &manifest, &wrong).is_err());
        wrong = expected.clone();
        wrong.image_sha256 = "0".repeat(64);
        assert!(CheckedSupportPage::from_json(&raw, &manifest, &wrong).is_err());
        let mut changed: Value = serde_json::from_str(&manifest).unwrap();
        changed["inputs"][0]["png_sha256"] = Value::String("0".repeat(64));
        assert!(CheckedSupportPage::from_json(&raw, &changed.to_string(), &expected).is_err());
        let response=serde_json::json!({"geometry_sha256":checked.digest(),"supports":value["spatial"]["supports"].as_array().unwrap().iter()
            .map(|s|serde_json::json!({"support_id":s["support_id"],"text":"OFFLINE FIXTURE"})).collect::<Vec<_>>()}).to_string();
        let response_hash = digest(response.as_bytes());
        // The selected digest can be valid while the literal transcription
        // still requires review or an explicit representation decision.
        for (text, classification) in [
            ("x\0y", "INVALID"),
            ("x\u{fffd}", "NEEDS_REVIEW"),
            ("a\nb", "CANONICAL_REQUIRED"),
            ("e\u{301}", "CANONICAL_REQUIRED"),
            (r"\frac{1}{2}", "CANONICAL_REQUIRED"),
        ] {
            let mut candidate: Value = serde_json::from_str(&response).unwrap();
            candidate["supports"][0]["text"] = Value::String(text.into());
            let raw = candidate.to_string();
            let error = checked.compose(
                &raw, &digest(raw.as_bytes()), "fixture", "fixture", "fixture",
                Default::default(),
            ).unwrap_err();
            assert!(error.to_string().contains(classification), "{error}");
        }
        let page = checked
            .compose(
                &response,
                &response_hash,
                "fixture",
                "fixture",
                "fixture",
                Default::default(),
            )
            .unwrap();
        assert!(checked
            .compose(
                &response.replace("OFFLINE FIXTURE", "SUBSTITUTED TEXT"),
                &response_hash,
                "fixture",
                "fixture",
                "fixture",
                Default::default()
            )
            .is_err());
        let artifact = page.provider_raw_artifact.unwrap();
        let output_hash =
            page.provider_provenance.unwrap().parameters["spatial_artifact_sha256"].clone();
        checked.verify_artifact(&artifact, &output_hash).unwrap();
        assert!(checked
            .verify_artifact(
                &artifact.replace("OFFLINE FIXTURE", "ALTERED"),
                &output_hash
            )
            .is_err());
        let a: Value = serde_json::from_str(&artifact).unwrap();
        assert_eq!(a["geometry_json"], raw);
        assert_eq!(a["input_manifest_json"], manifest);
        assert_eq!(a["response_json"], response);
    }
    #[test]
    fn full_candidate_replay_and_reject_untrusted_text() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/evidence/spatial-support-contract-2026-09-08/phase2");
        let mut pages = 0;
        for provider in ["surya", "paddle"] {
            for entry in std::fs::read_dir(root.join(provider)).unwrap() {
                let path = entry.unwrap().path();
                if !path.to_string_lossy().ends_with(".contract.json") {
                    continue;
                }
                let raw = std::fs::read_to_string(path).unwrap();
                let g = SupportPage::from_json(&raw).unwrap();
                let response = || Response {
                    geometry_sha256: g.digest(),
                    supports: g
                        .envelope
                        .spatial
                        .supports
                        .iter()
                        .enumerate()
                        .map(|(i, s)| SupportText {
                            support_id: s.support_id.clone(),
                            text: format!("TEST {i}"),
                        })
                        .collect(),
                };
                let result = g
                    .compose(
                        response(),
                        "offline-fixture",
                        "fixture",
                        "fixture/1",
                        Default::default(),
                    )
                    .unwrap();
                assert_eq!(result.blocks[0].lines.len(), g.envelope.spatial.units.len());
                let words: Vec<_> = result.blocks[0]
                    .lines
                    .iter()
                    .flat_map(|l| &l.words)
                    .collect();
                assert_eq!(words.len(), g.envelope.spatial.supports.len());
                for (w, s) in words.iter().zip(&g.envelope.spatial.supports) {
                    assert_eq!(w.bbox, bbox(&s.bbox));
                }
                let mut bad = response();
                bad.supports.reverse();
                assert!(g
                    .compose(bad, "fixture", "fixture", "fixture", Default::default())
                    .is_err());
                let mut bad = response();
                bad.geometry_sha256 = "0".repeat(64);
                assert!(g
                    .compose(bad, "fixture", "fixture", "fixture", Default::default())
                    .is_err());
                let mut tampered: Value = serde_json::from_str(&raw).unwrap();
                tampered["spatial"]["units"][0]["support_ids"] = serde_json::json!([]);
                assert!(SupportPage::from_json(&tampered.to_string()).is_err());
                let bad = r#"{"geometry_sha256":"x","supports":[{"support_id":"x","text":"a","bbox":[0,0,1,1]}]}"#;
                assert!(serde_json::from_str::<Response>(bad).is_err());
                pages += 1;
            }
        }
        assert_eq!(pages, 50);
    }
}
