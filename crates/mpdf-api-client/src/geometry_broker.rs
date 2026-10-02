//! Local broker transport for the frozen Surya geometry-bound contract.
//! Only a scoped broker token is read here; the Gemini key stays in the broker.
use crate::Secret;
use base64::Engine;
use image::ImageFormat;
use mpdf_core::ocr_provider::geometry_transcription::{
    self as gt, GeometryBoundTranscription, GeometryPage, GeometryProvider, GeometryRequest,
    GeometryTranscriptionError as Error, TranscriptionProvider, TranscriptionRequest,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    path::Path,
    time::Duration,
};

pub const SURYA_ID: &str = "surya-line-geometry";
pub const SURYA_VERSION: &str = "surya-0.17.0/text_detection-2025_05_07/r3.0-apparatus-repair";
fn invalid(message: &str) -> Error {
    Error::InvalidConfiguration(message.into())
}

/// The runtime bridge preserves the complete D-ready envelope separately from
/// the narrower immutable GeometryPage used by the core compositor.
pub struct SuryaGeometry {
    pub geometry: GeometryPage,
    pub evidence: Value,
    pub raw_evidence: String,
}
impl SuryaGeometry {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let bytes = std::fs::read(path).map_err(|_| invalid("Cannot read Surya evidence"))?;
        #[derive(serde::Deserialize)]
        struct RawEnvelope {
            evidence: Box<serde_json::value::RawValue>,
        }
        let raw: RawEnvelope =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid raw evidence"))?;
        let raw_evidence = raw.evidence.get().to_owned();
        let v: Value =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid Surya envelope"))?;
        let geometry: GeometryPage = serde_json::from_value(v["geometry"].clone())
            .map_err(|_| invalid("Invalid core geometry"))?;
        let evidence = v["evidence"].clone();
        gt::validate_geometry(&geometry)?;
        if geometry.provider_id != SURYA_ID
            || geometry.provider_version != SURYA_VERSION
            || evidence["provider"] != "surya"
            || evidence["adapter_version"] != "r3.0-apparatus-repair"
            || evidence["provenance"]["image_sha256"] != geometry.image_sha256
        {
            return Err(invalid("Evidence is not from the frozen Surya path"));
        }
        let freeze: Value = serde_json::from_str(include_str!(
            "../../../docs/evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json"
        ))
        .map_err(|_| invalid("Invalid frozen provider manifest"))?;
        if evidence["provenance"]["adapter_sha256"] != freeze["adapter_sha256"] {
            return Err(invalid("Adapter hash differs from freeze"));
        }
        for field in [
            "source_fragments",
            "fragments",
            "layout_regions",
            "column_bands",
            "margin_lanes",
            "derivations",
        ] {
            if !evidence[field].is_array() {
                return Err(invalid("Missing D-ready evidence"));
            }
        }
        let sources = evidence["source_fragments"].as_array().unwrap();
        let raw = evidence["provider_raw"]["detection"]["bboxes"]
            .as_array()
            .ok_or_else(|| invalid("Missing raw detector payload"))?;
        if sources.len() != raw.len() {
            return Err(invalid("Raw detector evidence was dropped"));
        }
        for (source, detected) in sources.iter().zip(raw) {
            if source["bbox"] != detected["bbox"]
                || source["polygon"] != detected["polygon"]
                || source["confidence"] != detected["confidence"]
            {
                return Err(invalid("Raw detector geometry was altered"));
            }
        }
        let lines = evidence["logical_lines"]
            .as_array()
            .ok_or_else(|| invalid("Missing logical evidence"))?;
        if lines.len() != geometry.lines.len() {
            return Err(invalid("Logical evidence count differs"));
        }
        for (line, expected) in geometry.lines.iter().zip(lines) {
            let b = &line.bbox;
            let bbox = expected["bbox"]
                .as_array()
                .ok_or_else(|| invalid("Missing evidence box"))?;
            let values: Option<Vec<f64>> = bbox.iter().map(Value::as_f64).collect();
            if expected["line_id"] != line.line_id
                || expected["reading_order"].as_u64() != Some(u64::from(line.reading_order))
                || values
                    != Some(vec![
                        b.x as f64,
                        b.y as f64,
                        (b.x + b.width) as f64,
                        (b.y + b.height) as f64,
                    ])
            {
                return Err(invalid("Core geometry differs from logical evidence"));
            }
        }
        Ok(Self {
            geometry,
            evidence,
            raw_evidence,
        })
    }
}
impl GeometryProvider for SuryaGeometry {
    fn provider_id(&self) -> &str {
        SURYA_ID
    }
    fn provider_version(&self) -> &str {
        SURYA_VERSION
    }
    fn fingerprint_contribution(&self) -> String {
        format!("{SURYA_VERSION}|{}", self.evidence["geometry_sha256"])
    }
    fn detect_geometry(&mut self, r: &GeometryRequest<'_>) -> Result<GeometryPage, Error> {
        if r.page_index != self.geometry.page_index
            || r.page_image_sha256 != self.geometry.image_sha256
            || r.page_image.width() != self.geometry.width
            || r.page_image.height() != self.geometry.height
        {
            return Err(invalid("Image does not match Surya evidence"));
        }
        Ok(self.geometry.clone())
    }
}

pub struct GeometryBroker {
    endpoint: String,
    token: Secret,
    client: reqwest::blocking::Client,
}
impl GeometryBroker {
    pub fn local(endpoint: &str, token: Secret) -> Result<Self, Error> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| invalid("Invalid broker origin"))?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.username() != ""
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(invalid(
                "Local broker must use an exact 127.0.0.1 HTTP origin",
            ));
        }
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| invalid("Cannot create broker client"))?;
        Ok(Self {
            endpoint: endpoint.trim_end_matches('/').into(),
            token,
            client,
        })
    }

    /// Authenticated, non-billable receipt readback. Contains no credential or
    /// source image bytes; the image hash binds it to the saved product raster.
    pub fn receipt(&self, idempotency_key: &str) -> Result<Value, Error> {
        if idempotency_key.len() != 64 || !idempotency_key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("Invalid broker receipt identity"));
        }
        let response = self
            .client
            .get(format!(
                "{}/v1/transcription/receipts/{idempotency_key}",
                self.endpoint
            ))
            .bearer_auth(self.token.as_str())
            .timeout(Duration::from_secs(15))
            .send()
            .map_err(|_| invalid("Broker receipt readback failed"))?;
        if !response.status().is_success() {
            return Err(invalid("Broker did not return a request receipt"));
        }
        let mut bytes = Vec::new();
        response
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid("Cannot read broker receipt"))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(invalid("Broker receipt too large"));
        }
        serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid broker receipt"))
    }
}
impl TranscriptionProvider for GeometryBroker {
    fn provider_id(&self) -> &str {
        "google-gemini"
    }
    fn model(&self) -> &str {
        gt::GEMINI_TRANSCRIPTION_MODEL
    }
    fn model_version(&self) -> &str {
        gt::GEMINI_TRANSCRIPTION_MODEL
    }
    fn fingerprint_contribution(&self) -> String {
        format!("geometry-broker/1|{}", gt::GEMINI_TRANSCRIPTION_MODEL)
    }
    fn transcribe(
        &mut self,
        r: &TranscriptionRequest<'_>,
    ) -> Result<GeometryBoundTranscription, Error> {
        let geometry_json =
            serde_json::to_string(r.geometry).map_err(|_| invalid("Invalid geometry"))?;
        let digest = r.geometry.digest()?;
        let mut png = Cursor::new(Vec::new());
        r.page_image
            .write_to(&mut png, ImageFormat::Png)
            .map_err(|_| invalid("Cannot encode page"))?;
        let png = png.into_inner();
        if png.len() > 24 * 1024 * 1024 {
            return Err(invalid("Page exceeds upload limit"));
        }
        let payload = serde_json::json!({"protocol":"museion-geometry-broker/1","geometry_json":geometry_json,"geometry_sha256":digest,
            "page_png_base64":base64::engine::general_purpose::STANDARD.encode(&png),"upload_sha256":format!("{:x}",Sha256::digest(&png)),
            "language_profile":r.language_profile,"idempotency_key":r.idempotency_key,"model":gt::GEMINI_TRANSCRIPTION_MODEL});
        // No automatic retry: an uncertain model call must not be billed twice.
        let response = self
            .client
            .post(format!("{}/v1/transcription/pages", self.endpoint))
            .bearer_auth(self.token.as_str())
            .timeout(r.deadline)
            .json(&payload)
            .send()
            .map_err(|_| {
                Error::TranscriptionTransport(
                    "Broker request failed; status uncertain, no automatic retry".into(),
                )
            })?;
        if !response.status().is_success() {
            return Err(Error::TranscriptionTransport(format!(
                "Broker HTTP {}",
                response.status().as_u16()
            )));
        }
        let mut bytes = Vec::new();
        response
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid("Cannot read broker response"))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(invalid("Broker response too large"));
        }
        let result: GeometryBoundTranscription = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("Malformed broker transcription"))?;
        if result.provider_id != "google-gemini"
            || result.model != self.model()
            || result.model_version != self.model_version()
        {
            return Err(invalid("Broker changed the pinned model"));
        }
        gt::validate_transcription(r.geometry, &result)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_broker_rejects_remote_origins_credentials_and_paths() {
        for endpoint in [
            "https://example.org",
            "http://localhost:8766",
            "http://127.0.0.1:8766/path",
            "http://user@127.0.0.1:8766",
            "http://127.0.0.1:8766/?key=value",
            "http://127.0.0.1:8766/#fragment",
        ] {
            assert!(GeometryBroker::local(endpoint, Secret::new("test-token")).is_err());
        }
        assert!(GeometryBroker::local("http://127.0.0.1:8766", Secret::new("test-token")).is_ok());
    }
}
