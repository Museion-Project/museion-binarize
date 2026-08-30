//! Deterministic geometry followed by geometry-bound transcription.
//!
//! This is the production architecture for scanned pages.  A
//! [`GeometryProvider`] observes line rectangles and reading order without
//! making its text authoritative.  A [`TranscriptionProvider`] receives that
//! immutable geometry and returns text keyed by the supplied line ids.  The
//! compositor accepts only an exact bijection: transcription can neither add
//! a line nor silently omit one, and it can never return a rectangle.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use image::DynamicImage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::jobs::ExecutionLocation;
use crate::ocr::{
    validate_ocr_page, OcrBlock, OcrBox, OcrError, OcrLine, OcrPage, OcrProviderProvenance,
    OcrRoute, OcrRouteReason, OcrWord, PageOcrProvider,
};

use super::{ProviderUsage, BYOK_DISABLED_MESSAGE};

pub const COMPOSITION_CONTRACT: &str = "mpdf-geometry-transcription/1";
pub const GEMINI_TRANSCRIPTION_MODEL: &str = "gemini-3.7-flash";
pub const GEMINI_TRANSCRIPTION_PROMPT_VERSION: &str = "geometry-bound-lines/1";
const MAX_LINES: usize = 16_384;
const MAX_LINE_TEXT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryLine {
    pub line_id: String,
    pub reading_order: u32,
    pub bbox: OcrBox,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryPage {
    pub page_index: u32,
    pub width: u32,
    pub height: u32,
    pub image_sha256: String,
    pub provider_id: String,
    pub provider_version: String,
    pub lines: Vec<GeometryLine>,
}

impl GeometryPage {
    /// Canonical digest supplied to the transcriber and required back in its
    /// response.  Text is absent by construction.
    pub fn digest(&self) -> Result<String, GeometryTranscriptionError> {
        validate_geometry(self)?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| GeometryTranscriptionError::InvalidGeometry(error.to_string()))?;
        Ok(hex_sha256(&bytes))
    }
}

pub struct GeometryRequest<'a> {
    pub page_index: u32,
    pub page_image: &'a DynamicImage,
    pub page_image_sha256: &'a str,
}

/// Produces repeatable, text-independent line geometry for one page.
pub trait GeometryProvider {
    fn provider_id(&self) -> &str;
    fn provider_version(&self) -> &str;
    fn fingerprint_contribution(&self) -> String;
    fn detect_geometry(
        &mut self,
        request: &GeometryRequest<'_>,
    ) -> Result<GeometryPage, GeometryTranscriptionError>;
}

/// Adapter for existing local OCR engines.  Their text is deliberately
/// discarded; only measured line rectangles, order and confidence cross the
/// geometry boundary.
pub struct LocalOcrGeometryProvider {
    inner: Box<dyn PageOcrProvider>,
    provider_id: String,
    provider_version: String,
}

impl LocalOcrGeometryProvider {
    pub fn new(
        inner: Box<dyn PageOcrProvider>,
        provider_id: impl Into<String>,
        provider_version: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            provider_id: provider_id.into(),
            provider_version: provider_version.into(),
        }
    }
}

impl GeometryProvider for LocalOcrGeometryProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn provider_version(&self) -> &str {
        &self.provider_version
    }

    fn fingerprint_contribution(&self) -> String {
        format!(
            "{COMPOSITION_CONTRACT}|geometry={}@{}",
            self.provider_id, self.provider_version
        )
    }

    fn detect_geometry(
        &mut self,
        request: &GeometryRequest<'_>,
    ) -> Result<GeometryPage, GeometryTranscriptionError> {
        let page = self
            .inner
            .recognize(
                request.page_index,
                request.page_image,
                request.page_image_sha256,
            )
            .map_err(GeometryTranscriptionError::GeometryProvider)?;
        validate_ocr_page(&page).map_err(GeometryTranscriptionError::GeometryProvider)?;

        let mut ordered = Vec::new();
        for block in &page.blocks {
            for line in &block.lines {
                ordered.push((block.reading_order, line.reading_order, line));
            }
        }
        ordered.sort_by_key(|(block, line, _)| (*block, *line));
        let lines = ordered
            .into_iter()
            .enumerate()
            .map(|(ordinal, (_, _, line))| GeometryLine {
                line_id: format!("p{:06}-l{:05}", page.page_index, ordinal),
                reading_order: ordinal as u32,
                bbox: line.bbox.clone(),
                confidence: Some(line.confidence),
            })
            .collect();
        let geometry = GeometryPage {
            page_index: page.page_index,
            width: page.width,
            height: page.height,
            image_sha256: request.page_image_sha256.to_owned(),
            provider_id: self.provider_id.clone(),
            provider_version: self.provider_version.clone(),
            lines,
        };
        validate_geometry(&geometry)?;
        Ok(geometry)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscribedLine {
    pub line_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryBoundTranscription {
    pub page_index: u32,
    pub geometry_sha256: String,
    pub lines: Vec<TranscribedLine>,
    pub provider_id: String,
    pub model: String,
    pub model_version: String,
    #[serde(default)]
    pub usage: ProviderUsage,
}

pub struct TranscriptionRequest<'a> {
    pub page_image: &'a DynamicImage,
    pub geometry: &'a GeometryPage,
    pub language_profile: &'a str,
    pub deadline: std::time::Duration,
    pub idempotency_key: &'a str,
}

/// A text provider cannot express geometry.  Its only join key is a line id
/// minted by the geometry provider.
pub trait TranscriptionProvider {
    fn provider_id(&self) -> &str;
    fn model(&self) -> &str;
    fn model_version(&self) -> &str;
    fn fingerprint_contribution(&self) -> String;
    fn transcribe(
        &mut self,
        request: &TranscriptionRequest<'_>,
    ) -> Result<GeometryBoundTranscription, GeometryTranscriptionError>;
}

/// Candidate-neutral page provider that connects the split contract to the
/// existing durable OCR loop.
///
/// The durable runner already consumes [`PageOcrProvider`]. This adapter is
/// therefore the single composition seam a future Credits factory needs to
/// build: geometry runs first, its identity is frozen and hashed, transcription
/// is bound to those line ids, and only the strict compositor can originate an
/// [`OcrPage`]. Neither the runner nor the PDF writer needs a candidate-specific
/// branch.
pub struct GeometryTranscriptionPageProvider {
    geometry: Box<dyn GeometryProvider>,
    transcriber: Box<dyn TranscriptionProvider>,
    language_profile: String,
    deadline: Duration,
    idempotency_namespace: String,
}

impl GeometryTranscriptionPageProvider {
    pub fn new(
        geometry: Box<dyn GeometryProvider>,
        transcriber: Box<dyn TranscriptionProvider>,
        language_profile: impl Into<String>,
        deadline: Duration,
        idempotency_namespace: impl Into<String>,
    ) -> Result<Self, GeometryTranscriptionError> {
        let language_profile = language_profile.into();
        let idempotency_namespace = idempotency_namespace.into();
        if language_profile.trim().is_empty()
            || deadline.is_zero()
            || idempotency_namespace.trim().is_empty()
        {
            return Err(GeometryTranscriptionError::InvalidConfiguration(
                "language profile, non-zero deadline, and idempotency namespace are required"
                    .into(),
            ));
        }
        Ok(Self {
            geometry,
            transcriber,
            language_profile,
            deadline,
            idempotency_namespace,
        })
    }

    /// Builds the split pipeline from the same boxed local provider currently
    /// handed to cloud factories by the orchestrator. This is the narrow
    /// connection point for Tesseract today and any future local geometry
    /// adapter; choosing a different detector does not change the durable OCR
    /// or PDF-writing layers.
    #[allow(clippy::too_many_arguments)]
    pub fn from_local_ocr(
        local: Box<dyn PageOcrProvider>,
        geometry_provider_id: impl Into<String>,
        geometry_provider_version: impl Into<String>,
        transcriber: Box<dyn TranscriptionProvider>,
        language_profile: impl Into<String>,
        deadline: Duration,
        idempotency_namespace: impl Into<String>,
    ) -> Result<Self, GeometryTranscriptionError> {
        Self::new(
            Box::new(LocalOcrGeometryProvider::new(
                local,
                geometry_provider_id,
                geometry_provider_version,
            )),
            transcriber,
            language_profile,
            deadline,
            idempotency_namespace,
        )
    }

    /// Non-secret identity suitable for a durable job fingerprint.
    pub fn fingerprint_contribution(&self) -> String {
        format!(
            "{COMPOSITION_CONTRACT}|{}|{}|language_profile={}",
            self.geometry.fingerprint_contribution(),
            self.transcriber.fingerprint_contribution(),
            self.language_profile
        )
    }

    fn idempotency_key(&self, page_index: u32, image_sha256: &str) -> String {
        hex_sha256(
            format!(
                "{COMPOSITION_CONTRACT}|{}|{page_index}|{image_sha256}|{}",
                self.idempotency_namespace,
                self.fingerprint_contribution()
            )
            .as_bytes(),
        )
    }
}

impl PageOcrProvider for GeometryTranscriptionPageProvider {
    fn execution_location(&self) -> ExecutionLocation {
        // The final page contains brokered-cloud transcription even though its
        // geometry was measured locally. Detailed provenance records both.
        ExecutionLocation::BrokeredCloud
    }

    fn recognize(
        &mut self,
        page_index: u32,
        image: &DynamicImage,
        input_asset_sha256: &str,
    ) -> Result<OcrPage, OcrError> {
        let geometry = self
            .geometry
            .detect_geometry(&GeometryRequest {
                page_index,
                page_image: image,
                page_image_sha256: input_asset_sha256,
            })
            .map_err(|error| split_provider_error(page_index, error))?;
        if geometry.page_index != page_index
            || geometry.width != image.width()
            || geometry.height != image.height()
            || geometry.image_sha256 != input_asset_sha256
        {
            return Err(OcrError::InvalidEvidence(
                "geometry response does not identify the requested page raster".into(),
            ));
        }
        validate_geometry(&geometry).map_err(|error| split_provider_error(page_index, error))?;
        let idempotency_key = self.idempotency_key(page_index, input_asset_sha256);
        let transcription = self
            .transcriber
            .transcribe(&TranscriptionRequest {
                page_image: image,
                geometry: &geometry,
                language_profile: &self.language_profile,
                deadline: self.deadline,
                idempotency_key: &idempotency_key,
            })
            .map_err(|error| split_provider_error(page_index, error))?;
        let mut page = compose_ocr_page(&geometry, &transcription)
            .map_err(|error| split_provider_error(page_index, error))?;
        if let Some(provenance) = page.provider_provenance.as_mut() {
            provenance.language_profile = Some(self.language_profile.clone());
            provenance.parameters.insert(
                "pipeline_fingerprint".into(),
                self.fingerprint_contribution(),
            );
            provenance
                .parameters
                .insert("idempotency_key".into(), idempotency_key);
        }
        Ok(page)
    }
}

/// Safe request handed to a Gemini transport.  Credential material belongs to
/// the transport/service and is impossible to serialize through this type.
pub struct GeminiRequest<'a> {
    pub model: &'a str,
    pub model_version: &'a str,
    pub prompt: String,
    pub page_image: &'a DynamicImage,
    pub page_index: u32,
    pub geometry_sha256: &'a str,
    pub deadline: std::time::Duration,
    pub idempotency_key: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeminiResponse {
    pub body: String,
    pub model_version: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub trait GeminiTransport {
    fn transcribe_lines(
        &mut self,
        request: &GeminiRequest<'_>,
    ) -> Result<GeminiResponse, GeometryTranscriptionError>;
}

pub struct GeminiGeometryTranscriber<T> {
    transport: T,
    model_version: String,
}

impl<T> GeminiGeometryTranscriber<T> {
    pub fn new(transport: T, model_version: impl Into<String>) -> Self {
        Self {
            transport,
            model_version: model_version.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeminiBody {
    page_index: u32,
    geometry_sha256: String,
    lines: Vec<TranscribedLine>,
}

impl<T: GeminiTransport> TranscriptionProvider for GeminiGeometryTranscriber<T> {
    fn provider_id(&self) -> &str {
        "google-gemini"
    }

    fn model(&self) -> &str {
        GEMINI_TRANSCRIPTION_MODEL
    }

    fn model_version(&self) -> &str {
        &self.model_version
    }

    fn fingerprint_contribution(&self) -> String {
        format!(
            "{COMPOSITION_CONTRACT}|transcriber=google-gemini|model={}|model_version={}|prompt={}",
            GEMINI_TRANSCRIPTION_MODEL, self.model_version, GEMINI_TRANSCRIPTION_PROMPT_VERSION
        )
    }

    fn transcribe(
        &mut self,
        request: &TranscriptionRequest<'_>,
    ) -> Result<GeometryBoundTranscription, GeometryTranscriptionError> {
        let geometry_sha256 = request.geometry.digest()?;
        let prompt = gemini_prompt(request.geometry, &geometry_sha256, request.language_profile)?;
        let response = self.transport.transcribe_lines(&GeminiRequest {
            model: GEMINI_TRANSCRIPTION_MODEL,
            model_version: &self.model_version,
            prompt,
            page_image: request.page_image,
            page_index: request.geometry.page_index,
            geometry_sha256: &geometry_sha256,
            deadline: request.deadline,
            idempotency_key: request.idempotency_key,
        })?;
        if response.model_version != self.model_version {
            return Err(GeometryTranscriptionError::InvalidTranscription(
                "response model version differs from the pinned request".into(),
            ));
        }
        let body: GeminiBody = serde_json::from_str(&response.body)
            .map_err(|error| GeometryTranscriptionError::InvalidTranscription(error.to_string()))?;
        let transcription = GeometryBoundTranscription {
            page_index: body.page_index,
            geometry_sha256: body.geometry_sha256,
            lines: body.lines,
            provider_id: "google-gemini".to_owned(),
            model: GEMINI_TRANSCRIPTION_MODEL.to_owned(),
            model_version: response.model_version,
            usage: ProviderUsage {
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
                credits_charged: 0,
                requests: 1,
            },
        };
        validate_transcription(request.geometry, &transcription)?;
        Ok(transcription)
    }
}

pub fn gemini_prompt(
    geometry: &GeometryPage,
    geometry_sha256: &str,
    language_profile: &str,
) -> Result<String, GeometryTranscriptionError> {
    validate_geometry(geometry)?;
    let line_contract: Vec<_> = geometry
        .lines
        .iter()
        .map(|line| {
            serde_json::json!({
                "line_id": line.line_id,
                "reading_order": line.reading_order,
                "bbox": [line.bbox.x, line.bbox.y, line.bbox.width, line.bbox.height],
            })
        })
        .collect();
    let contract = serde_json::to_string(&line_contract)
        .map_err(|error| GeometryTranscriptionError::InvalidGeometry(error.to_string()))?;
    Ok(format!(
        "Transcribe only the printed text inside each supplied line box. The boxes and order are immutable. Return exactly one JSON object with only page_index, geometry_sha256, and lines. Each lines item may contain only line_id, text, language, and confidence. Return every supplied line_id exactly once in the supplied order; never add, omit, merge, split, reorder, or return coordinates. Preserve characters exactly as printed, including polytonic Greek diacritics, punctuation, Latin, German, abbreviations, and footnote markers. Do not translate, normalize orthography, or add commentary. Use U+FFFD for one illegible character. language is a BCP-47-style label or null; confidence is 0..1 or null. page_index={}; geometry_sha256={}; language_profile={}; lines={}",
        geometry.page_index, geometry_sha256, language_profile, contract
    ))
}

pub fn validate_geometry(page: &GeometryPage) -> Result<(), GeometryTranscriptionError> {
    if page.width == 0
        || page.height == 0
        || page.lines.is_empty()
        || page.lines.len() > MAX_LINES
        || page.provider_id.is_empty()
        || page.provider_version.is_empty()
        || !is_sha256(&page.image_sha256)
    {
        return Err(GeometryTranscriptionError::InvalidGeometry(
            "page identity, dimensions, provider, image digest, or line count is invalid".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    for (expected_order, line) in page.lines.iter().enumerate() {
        if line.line_id.is_empty()
            || line.line_id.len() > 128
            || !ids.insert(&line.line_id)
            || line.reading_order != expected_order as u32
            || !valid_box(&line.bbox, page.width, page.height)
            || line
                .confidence
                .is_some_and(|value| !(0.0..=1.0).contains(&value))
        {
            return Err(GeometryTranscriptionError::InvalidGeometry(format!(
                "line {expected_order} has an invalid id, order, box, or confidence"
            )));
        }
    }
    Ok(())
}

pub fn validate_transcription(
    geometry: &GeometryPage,
    transcription: &GeometryBoundTranscription,
) -> Result<(), GeometryTranscriptionError> {
    validate_geometry(geometry)?;
    if transcription.page_index != geometry.page_index
        || transcription.geometry_sha256 != geometry.digest()?
        || transcription.provider_id.is_empty()
        || transcription.model.is_empty()
        || transcription.model_version.is_empty()
        || transcription.lines.len() != geometry.lines.len()
    {
        return Err(GeometryTranscriptionError::InvalidTranscription(
            "page identity, geometry digest, provider identity, or line count differs".into(),
        ));
    }
    for (expected, actual) in geometry.lines.iter().zip(&transcription.lines) {
        let nfc: String = actual.text.nfc().collect();
        if actual.line_id != expected.line_id
            || actual.text.is_empty()
            || actual.text.len() > MAX_LINE_TEXT_BYTES
            || nfc != actual.text
            || actual
                .language
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > 64)
            || actual
                .confidence
                .is_some_and(|value| !(0.0..=1.0).contains(&value))
        {
            return Err(GeometryTranscriptionError::InvalidTranscription(format!(
                "line {} is missing, reordered, non-NFC, empty, or malformed",
                expected.line_id
            )));
        }
    }
    Ok(())
}

/// Joins geometry and text only after the strict line-id contract passes.
pub fn compose_ocr_page(
    geometry: &GeometryPage,
    transcription: &GeometryBoundTranscription,
) -> Result<OcrPage, GeometryTranscriptionError> {
    validate_transcription(geometry, transcription)?;
    let lines: Vec<OcrLine> = geometry
        .lines
        .iter()
        .zip(&transcription.lines)
        .map(|(shape, text)| {
            let confidence = text.confidence.or(shape.confidence).unwrap_or(0.0);
            OcrLine {
                bbox: shape.bbox.clone(),
                confidence,
                reading_order: shape.reading_order,
                words: vec![OcrWord {
                    text: text.text.clone(),
                    normalized_text: text.text.clone(),
                    bbox: shape.bbox.clone(),
                    confidence,
                    reading_order: 0,
                }],
            }
        })
        .collect();
    let block_box = union_box(geometry.lines.iter().map(|line| &line.bbox));
    let block_confidence =
        lines.iter().map(|line| line.confidence).sum::<f32>() / lines.len() as f32;
    let mut parameters = BTreeMap::new();
    parameters.insert("contract".into(), COMPOSITION_CONTRACT.into());
    parameters.insert("geometry_provider".into(), geometry.provider_id.clone());
    parameters.insert(
        "geometry_provider_version".into(),
        geometry.provider_version.clone(),
    );
    parameters.insert("geometry_sha256".into(), geometry.digest()?);
    parameters.insert(
        "transcription_provider".into(),
        transcription.provider_id.clone(),
    );
    parameters.insert(
        "transcription_model_version".into(),
        transcription.model_version.clone(),
    );
    parameters.insert(
        "input_tokens".into(),
        transcription.usage.input_tokens.to_string(),
    );
    parameters.insert(
        "output_tokens".into(),
        transcription.usage.output_tokens.to_string(),
    );
    parameters.insert(
        "credits_charged".into(),
        transcription.usage.credits_charged.to_string(),
    );
    parameters.insert("requests".into(), transcription.usage.requests.to_string());
    let page = OcrPage {
        page_index: geometry.page_index,
        route: OcrRoute::Ocr {
            reason: OcrRouteReason::MissingText,
        },
        width: geometry.width,
        height: geometry.height,
        blocks: vec![OcrBlock {
            bbox: block_box,
            confidence: block_confidence,
            reading_order: 0,
            lines,
        }],
        revisions: Vec::new(),
        provider_provenance: Some(OcrProviderProvenance {
            engine: format!("{}+{}", geometry.provider_id, transcription.provider_id),
            model: transcription.model.clone(),
            version: COMPOSITION_CONTRACT.into(),
            parameters,
            input_asset_sha256: geometry.image_sha256.clone(),
            execution_location: ExecutionLocation::BrokeredCloud,
            language_profile: None,
            model_digest: None,
            model_license: None,
            model_set: Some(format!(
                "{} {}",
                transcription.model, transcription.model_version
            )),
        }),
        provider_raw_artifact: None,
    };
    validate_ocr_page(&page).map_err(GeometryTranscriptionError::Composition)?;
    Ok(page)
}

#[derive(Debug, thiserror::Error)]
pub enum GeometryTranscriptionError {
    #[error("invalid geometry/transcription configuration: {0}")]
    InvalidConfiguration(String),
    #[error("geometry provider failed: {0}")]
    GeometryProvider(OcrError),
    #[error("invalid deterministic geometry: {0}")]
    InvalidGeometry(String),
    #[error("Gemini transcription failed: {0}")]
    TranscriptionTransport(String),
    #[error("invalid geometry-bound transcription: {0}")]
    InvalidTranscription(String),
    #[error("composed OCR page is invalid: {0}")]
    Composition(OcrError),
    #[error("{BYOK_DISABLED_MESSAGE}")]
    ByokDisabled,
}

fn split_provider_error(page: u32, error: GeometryTranscriptionError) -> OcrError {
    match error {
        GeometryTranscriptionError::GeometryProvider(error)
        | GeometryTranscriptionError::Composition(error) => error,
        other => OcrError::ProviderFailed {
            page,
            reason: other.to_string(),
        },
    }
}

fn valid_box(bbox: &OcrBox, width: u32, height: u32) -> bool {
    bbox.x.is_finite()
        && bbox.y.is_finite()
        && bbox.width.is_finite()
        && bbox.height.is_finite()
        && bbox.x >= 0.0
        && bbox.y >= 0.0
        && bbox.width > 0.0
        && bbox.height > 0.0
        && bbox.x + bbox.width <= width as f32
        && bbox.y + bbox.height <= height as f32
}

fn union_box<'a>(mut boxes: impl Iterator<Item = &'a OcrBox>) -> OcrBox {
    let first = boxes.next().expect("validated geometry is non-empty");
    let (mut left, mut top) = (first.x, first.y);
    let (mut right, mut bottom) = (first.x + first.width, first.y + first.height);
    for bbox in boxes {
        left = left.min(bbox.x);
        top = top.min(bbox.y);
        right = right.max(bbox.x + bbox.width);
        bottom = bottom.max(bbox.y + bbox.height);
    }
    OcrBox {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeGeminiTransport {
        response: GeminiResponse,
    }

    struct FixedGeometry;

    impl GeometryProvider for FixedGeometry {
        fn provider_id(&self) -> &str {
            "fixed-geometry"
        }

        fn provider_version(&self) -> &str {
            "test-1"
        }

        fn fingerprint_contribution(&self) -> String {
            "fixed-geometry@test-1".into()
        }

        fn detect_geometry(
            &mut self,
            request: &GeometryRequest<'_>,
        ) -> Result<GeometryPage, GeometryTranscriptionError> {
            let mut page = geometry();
            page.page_index = request.page_index;
            page.image_sha256 = request.page_image_sha256.into();
            page.provider_id = self.provider_id().into();
            page.provider_version = self.provider_version().into();
            for (order, line) in page.lines.iter_mut().enumerate() {
                line.line_id = format!("p{:06}-l{order:05}", request.page_index);
            }
            Ok(page)
        }
    }

    struct FixedTranscriber;

    impl TranscriptionProvider for FixedTranscriber {
        fn provider_id(&self) -> &str {
            "fixed-transcriber"
        }

        fn model(&self) -> &str {
            GEMINI_TRANSCRIPTION_MODEL
        }

        fn model_version(&self) -> &str {
            "test-1"
        }

        fn fingerprint_contribution(&self) -> String {
            "fixed-transcriber@test-1".into()
        }

        fn transcribe(
            &mut self,
            request: &TranscriptionRequest<'_>,
        ) -> Result<GeometryBoundTranscription, GeometryTranscriptionError> {
            Ok(GeometryBoundTranscription {
                page_index: request.geometry.page_index,
                geometry_sha256: request.geometry.digest()?,
                lines: request
                    .geometry
                    .lines
                    .iter()
                    .map(|line| TranscribedLine {
                        line_id: line.line_id.clone(),
                        text: if line.reading_order == 0 {
                            "Die λέξις".into()
                        } else {
                            "est française".into()
                        },
                        language: Some("mixed".into()),
                        confidence: Some(0.9),
                    })
                    .collect(),
                provider_id: self.provider_id().into(),
                model: self.model().into(),
                model_version: self.model_version().into(),
                usage: ProviderUsage {
                    input_tokens: 10,
                    output_tokens: 4,
                    credits_charged: 1,
                    requests: 1,
                },
            })
        }
    }

    impl GeminiTransport for FakeGeminiTransport {
        fn transcribe_lines(
            &mut self,
            _request: &GeminiRequest<'_>,
        ) -> Result<GeminiResponse, GeometryTranscriptionError> {
            Ok(self.response.clone())
        }
    }

    fn geometry() -> GeometryPage {
        GeometryPage {
            page_index: 7,
            width: 100,
            height: 200,
            image_sha256: "ab".repeat(32),
            provider_id: "local-layout".into(),
            provider_version: "frozen-1".into(),
            lines: vec![
                GeometryLine {
                    line_id: "p000007-l00000".into(),
                    reading_order: 0,
                    bbox: OcrBox {
                        x: 10.0,
                        y: 20.0,
                        width: 60.0,
                        height: 10.0,
                    },
                    confidence: Some(0.9),
                },
                GeometryLine {
                    line_id: "p000007-l00001".into(),
                    reading_order: 1,
                    bbox: OcrBox {
                        x: 10.0,
                        y: 40.0,
                        width: 70.0,
                        height: 10.0,
                    },
                    confidence: Some(0.8),
                },
            ],
        }
    }

    #[test]
    fn split_provider_is_connected_to_page_ocr_contract() {
        let mut provider = GeometryTranscriptionPageProvider::new(
            Box::new(FixedGeometry),
            Box::new(FixedTranscriber),
            "deu+fra+eng+grc",
            Duration::from_secs(30),
            "job-test",
        )
        .unwrap();
        let page = provider
            .recognize(7, &DynamicImage::new_rgb8(100, 200), &"ab".repeat(32))
            .unwrap();

        assert_eq!(
            provider.execution_location(),
            ExecutionLocation::BrokeredCloud
        );
        assert_eq!(page.blocks[0].lines.len(), 2);
        assert_eq!(page.blocks[0].lines[0].bbox, geometry().lines[0].bbox);
        assert_eq!(page.blocks[0].lines[0].words[0].text, "Die λέξις");
        assert_eq!(page.blocks[0].lines[1].words[0].text, "est française");
        let provenance = page.provider_provenance.unwrap();
        assert_eq!(provenance.parameters["geometry_provider"], "fixed-geometry");
        assert_eq!(
            provenance.parameters["transcription_provider"],
            "fixed-transcriber"
        );
    }

    fn transcription(geometry: &GeometryPage) -> GeometryBoundTranscription {
        GeometryBoundTranscription {
            page_index: geometry.page_index,
            geometry_sha256: geometry.digest().unwrap(),
            lines: vec![
                TranscribedLine {
                    line_id: "p000007-l00000".into(),
                    text: "Ἐν ἀρχῇ".into(),
                    language: Some("grc".into()),
                    confidence: None,
                },
                TranscribedLine {
                    line_id: "p000007-l00001".into(),
                    text: "λόγος".into(),
                    language: Some("grc".into()),
                    confidence: Some(0.95),
                },
            ],
            provider_id: "google-gemini".into(),
            model: GEMINI_TRANSCRIPTION_MODEL.into(),
            model_version: "2026-08-30".into(),
            usage: ProviderUsage {
                input_tokens: 100,
                output_tokens: 20,
                credits_charged: 1,
                requests: 1,
            },
        }
    }

    #[test]
    fn composition_preserves_geometry_byte_for_byte() {
        let geometry = geometry();
        let page = compose_ocr_page(&geometry, &transcription(&geometry)).unwrap();
        assert_eq!(page.blocks[0].lines[0].bbox, geometry.lines[0].bbox);
        assert_eq!(page.blocks[0].lines[1].bbox, geometry.lines[1].bbox);
        assert_eq!(page.blocks[0].lines[0].words[0].text, "Ἐν ἀρχῇ");
        assert_eq!(
            page.provider_provenance.unwrap().version,
            COMPOSITION_CONTRACT
        );
    }

    #[test]
    fn missing_reordered_and_unknown_line_ids_are_rejected() {
        let geometry = geometry();
        let mut value = transcription(&geometry);
        value.lines.swap(0, 1);
        assert!(matches!(
            validate_transcription(&geometry, &value),
            Err(GeometryTranscriptionError::InvalidTranscription(_))
        ));
        value = transcription(&geometry);
        value.lines.pop();
        assert!(validate_transcription(&geometry, &value).is_err());
        value = transcription(&geometry);
        value.lines[0].line_id = "invented".into();
        assert!(validate_transcription(&geometry, &value).is_err());
    }

    #[test]
    fn geometry_digest_changes_when_a_box_moves() {
        let first = geometry();
        let mut second = first.clone();
        second.lines[0].bbox.x += 1.0;
        assert_ne!(first.digest().unwrap(), second.digest().unwrap());
    }

    #[test]
    fn prompt_forbids_the_model_from_generating_geometry() {
        let geometry = geometry();
        let prompt = gemini_prompt(&geometry, &geometry.digest().unwrap(), "grc").unwrap();
        assert!(prompt.contains("never add, omit, merge, split, reorder, or return coordinates"));
        assert!(prompt.contains("p000007-l00000"));
        assert!(prompt.contains(&geometry.digest().unwrap()));
    }

    #[test]
    fn gemini_transcriber_accepts_only_geometry_bound_json() {
        let geometry = geometry();
        let expected = transcription(&geometry);
        let body = serde_json::json!({
            "page_index": geometry.page_index,
            "geometry_sha256": geometry.digest().unwrap(),
            "lines": expected.lines,
        });
        let transport = FakeGeminiTransport {
            response: GeminiResponse {
                body: serde_json::to_string(&body).unwrap(),
                model_version: "2026-08-30".into(),
                input_tokens: 123,
                output_tokens: 45,
            },
        };
        let image = DynamicImage::new_rgb8(100, 200);
        let mut provider = GeminiGeometryTranscriber::new(transport, "2026-08-30");
        let result = provider
            .transcribe(&TranscriptionRequest {
                page_image: &image,
                geometry: &geometry,
                language_profile: "grc",
                deadline: std::time::Duration::from_secs(30),
                idempotency_key: "page-7",
            })
            .unwrap();
        assert_eq!(result.lines.len(), 2);
        assert_eq!(result.usage.input_tokens, 123);
        assert_eq!(result.usage.requests, 1);
    }

    #[test]
    fn gemini_transcriber_rejects_unpinned_response_version() {
        let geometry = geometry();
        let transport = FakeGeminiTransport {
            response: GeminiResponse {
                body: "{}".into(),
                model_version: "floating-new-version".into(),
                input_tokens: 0,
                output_tokens: 0,
            },
        };
        let image = DynamicImage::new_rgb8(100, 200);
        let mut provider = GeminiGeometryTranscriber::new(transport, "2026-08-30");
        assert!(matches!(
            provider.transcribe(&TranscriptionRequest {
                page_image: &image,
                geometry: &geometry,
                language_profile: "grc",
                deadline: std::time::Duration::from_secs(30),
                idempotency_key: "page-7",
            }),
            Err(GeometryTranscriptionError::InvalidTranscription(_))
        ));
    }
}
