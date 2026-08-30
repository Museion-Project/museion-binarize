//! Bounded, local-only OCR routing and evidence records.
//!
//! The M3 layer intentionally keeps OCR separate from the 0.1 core package
//! records.  `ocr/` is an MDP extension directory containing typed JSON
//! records; readers that only understand MDP 0.1 can still validate and use
//! the source/page evidence package.  No provider or model is downloaded by
//! this module.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use image::{DynamicImage, ImageFormat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

use crate::document_session::{DocumentSession, NativeTextPage};
use crate::error::CoreError;
use crate::jobs::{
    ExecutionLocation, JobStore, ProviderProvenance, ProviderResponse, JOB_PROTOCOL,
    JOB_PROTOCOL_VERSION,
};

pub const OCR_PROTOCOL: &str = "mpdf-ocr";
pub const OCR_PROTOCOL_VERSION: &str = "0.1";
const OCR_ACTIVE_RUN_PROTOCOL: &str = "mpdf-ocr-active-run";
pub const CANONICAL_OCR_DPI: u16 = 300;
pub const MAX_OCR_BLOCKS: usize = 4_096;
pub const MAX_OCR_LINES: usize = 16_384;
pub const MAX_OCR_WORDS: usize = 65_536;
pub const MAX_OCR_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_RAW_ARTIFACT_BYTES: usize = 1024 * 1024;
/// The ceiling on one *provider response* and one *page record*. This is the
/// untrusted-input boundary: it bounds what a sidecar process can hand back
/// for a single page.
pub const MAX_PROVIDER_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;
/// The ceiling on `ocr/summary.json`, which holds the whole document rather
/// than one page.
///
/// This is deliberately *not* [`MAX_PROVIDER_OUTPUT_BYTES`]. Reusing the
/// per-page bound for the aggregate made the pipeline fail on any real book:
/// a dense 300-dpi scanned page costs roughly 200 KB of typed evidence, so a
/// 160-page volume needs about 34 MB and hit the 8 MB per-response limit at
/// around page 40. Synthetic fixtures are a few sparse pages and never reach
/// it. The summary is this crate's own re-serialization of records that were
/// each already validated and size-checked on the way in, so the bound here
/// exists to fail closed on a corrupt or runaway workspace file, not to
/// police a provider.
pub const MAX_OCR_SUMMARY_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_PROVIDER_STDERR_BYTES: u64 = 16 * 1024;
pub const MAX_PROVIDER_RUNTIME: Duration = Duration::from_secs(120);
/// The language profiles the local sidecar understands.
///
/// These mirror `scripts/ocr/mpdf_ocr_sidecar.py`. "Greek" is deliberately not
/// one profile: polytonic Ancient Greek and monotonic Modern Greek need
/// different models, and treating them as interchangeable is what silently
/// destroys breathings and iota subscripts.
pub const OCR_LANGUAGE_PROFILES: [&str; 8] = [
    "auto",
    "greek-ancient",
    "greek-ancient-german-english",
    "greek-modern",
    "german",
    "german-english",
    "english",
    "latin-german-english",
];

/// The profile chosen when a caller expresses no preference.
pub const DEFAULT_OCR_LANGUAGE_PROFILE: &str = "auto";

/// The production model set this build expects, as pinned in
/// `distribution/ocr-models/manifest.toml`.
pub const PRODUCTION_MODEL_SET: &str = "tessdata_best";
pub const PRODUCTION_MODEL_SET_VERSION: &str = "4.1.0";

/// Model files a `tessdata` model directory must contain for a profile to be
/// runnable. `osd` is always required: the sidecar's orientation pass reads it
/// on every page.
pub fn required_model_files(language_profile: &str) -> Option<Vec<String>> {
    let languages: &[&str] = match language_profile {
        "auto" | "greek-ancient-german-english" => &["grc", "deu", "eng"],
        "greek-ancient" => &["grc"],
        "greek-modern" => &["ell"],
        "german" => &["deu"],
        "german-english" => &["deu", "eng"],
        "english" => &["eng"],
        "latin-german-english" => &["lat", "deu", "eng"],
        _ => return None,
    };
    let mut files: Vec<String> = languages
        .iter()
        .map(|language| format!("{language}.traineddata"))
        .collect();
    files.push("osd.traineddata".to_owned());
    files.push("manifest.json".to_owned());
    Some(files)
}

/// Which local engine the sidecar should drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OcrEngine {
    /// Tesseract 5 LSTM with a pinned `tessdata_best` model set. The only
    /// engine cleared as a production default by the gold evaluation.
    #[default]
    Tesseract,
    /// PaddleOCR/RapidOCR-style detector + region recognizer. Evaluation only:
    /// it fetches weights at first use and cannot represent polytonic Greek.
    PaddleOcr,
}

impl OcrEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tesseract => "tesseract",
            Self::PaddleOcr => "paddleocr",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "tesseract" => Some(Self::Tesseract),
            "paddleocr" => Some(Self::PaddleOcr),
            _ => None,
        }
    }

    /// Whether this engine may be a production default. Kept next to the
    /// engine list so a new engine cannot be adopted without an explicit
    /// decision here and a passing gold run.
    pub fn cleared_for_production(self) -> bool {
        matches!(self, Self::Tesseract)
    }
}

pub const RAPIDOCR_MODEL_FILES: [&str; 3] = [
    "ch_PP-OCRv4_det_infer.onnx",
    "ch_PP-OCRv4_rec_infer.onnx",
    "ch_ppocr_mobile_v2.0_cls_infer.onnx",
];

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("OCR provider unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("OCR provider failed on page {page}: {reason}")]
    ProviderFailed { page: u32, reason: String },
    #[error("invalid OCR evidence: {0}")]
    InvalidEvidence(String),
    #[error("OCR page {page} failed: {reason}")]
    PageFailed { page: u32, reason: String },
    #[error("OCR package extension error: {0}")]
    Package(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrRoute {
    NativeText,
    Ocr { reason: OcrRouteReason },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrRouteReason {
    MissingText,
    TooLittleText,
    GarbledText,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrWord {
    pub text: String,
    pub normalized_text: String,
    pub bbox: OcrBox,
    pub confidence: f32,
    pub reading_order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrLine {
    pub bbox: OcrBox,
    pub confidence: f32,
    pub reading_order: u32,
    pub words: Vec<OcrWord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrBlock {
    pub bbox: OcrBox,
    pub confidence: f32,
    pub reading_order: u32,
    pub lines: Vec<OcrLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrPage {
    pub page_index: u32,
    pub route: OcrRoute,
    pub width: u32,
    pub height: u32,
    pub blocks: Vec<OcrBlock>,
    /// Optional later human/AI revisions. The source word text and its
    /// normalized form above are never overwritten by a revision.
    pub revisions: Vec<OcrRevision>,
    pub provider_provenance: Option<OcrProviderProvenance>,
    /// Bounded provider output retained for auditability. This is metadata,
    /// not an untyped replacement for the block/line/word records.
    pub provider_raw_artifact: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OcrRevision {
    pub revision_id: String,
    pub kind: OcrRevisionKind,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrRevisionKind {
    Human,
    AiSuggested,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OcrProviderProvenance {
    pub engine: String,
    pub model: String,
    pub version: String,
    pub parameters: BTreeMap<String, String>,
    pub input_asset_sha256: String,
    pub execution_location: ExecutionLocation,
    /// The language profile the page was recognized under. A page recognized
    /// with the wrong profile is the single largest source of the Greek and
    /// German failures this layer exists to prevent, so it is recorded as a
    /// first-class field rather than left in `parameters`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_profile: Option<String>,
    /// `language:sha256` for every model file that actually took part.
    /// Changing a model therefore changes the evidence fingerprint, which
    /// invalidates downstream checkpoints and candidate digests by design.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_digest: Option<String>,
    /// SPDX-ish license expression for the model weights.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_license: Option<String>,
    /// Pinned model-set name and version, e.g. `tessdata_best 4.1.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_set: Option<String>,
}

impl OcrProviderProvenance {
    /// Builds provenance from a provider response, promoting the identity
    /// parameters the sidecar reports into their own fields.
    fn from_response(
        engine: String,
        model: String,
        version: String,
        parameters: BTreeMap<String, String>,
        input_asset_sha256: String,
        execution_location: ExecutionLocation,
    ) -> Self {
        let take = |key: &str| parameters.get(key).filter(|v| !v.is_empty()).cloned();
        let model_set = match (take("model_set"), take("model_set_version")) {
            (Some(name), Some(version)) => Some(format!("{name} {version}")),
            (Some(name), None) => Some(name),
            _ => None,
        };
        Self {
            language_profile: take("language_profile"),
            model_digest: take("model_files_sha256"),
            model_license: take("model_license"),
            model_set,
            engine,
            model,
            version,
            parameters,
            input_asset_sha256,
            execution_location,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OcrPageError {
    pub page_index: u32,
    pub route_reason: OcrRouteReason,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrRun {
    pub protocol: String,
    pub protocol_version: String,
    pub pages: Vec<OcrPage>,
    pub errors: Vec<OcrPageError>,
}

impl OcrRun {
    pub fn is_complete(&self, expected_pages: u32) -> bool {
        self.errors.is_empty() && self.pages.len() == expected_pages as usize
    }

    pub fn validate(&self) -> std::result::Result<(), OcrError> {
        validate_protocol(&self.protocol, &self.protocol_version)?;
        if self.pages.len() > expected_limit() {
            return Err(OcrError::InvalidEvidence("too many OCR pages".into()));
        }
        let mut seen = std::collections::HashSet::new();
        for page in &self.pages {
            if !seen.insert(page.page_index) {
                return Err(OcrError::InvalidEvidence("duplicate page index".into()));
            }
            validate_ocr_page(page)?;
        }
        for error in &self.errors {
            if error.message.is_empty()
                || error.message.len() > 1024
                || error.code.is_empty()
                || error.code.len() > 128
            {
                return Err(OcrError::InvalidEvidence(
                    "invalid page error message".into(),
                ));
            }
        }
        Ok(())
    }
}

fn expected_limit() -> usize {
    // The package validator's page cap is intentionally not duplicated as a
    // public dependency here; this still prevents unbounded extension reads.
    100_000
}

fn validate_protocol(protocol: &str, version: &str) -> std::result::Result<(), OcrError> {
    if protocol != OCR_PROTOCOL {
        return Err(OcrError::InvalidEvidence(format!(
            "unsupported OCR protocol {protocol}"
        )));
    }
    let mut parts = version.split('.');
    let major = parts.next().and_then(|part| part.parse::<u16>().ok());
    let minor = parts.next().and_then(|part| part.parse::<u16>().ok());
    if major != Some(0)
        || minor.is_none()
        || parts.next().is_some()
        || format!(
            "{}.{}",
            major.unwrap_or_default(),
            minor.unwrap_or_default()
        ) != version
    {
        return Err(OcrError::InvalidEvidence(format!(
            "unsupported OCR protocol version {version}"
        )));
    }
    Ok(())
}

/// Validates one canonical coordinate-bearing OCR page.
///
/// Public so complete providers and text-enhancement application can share the
/// exact same evidence boundary instead of maintaining weaker copies.
pub fn validate_ocr_page(page: &OcrPage) -> std::result::Result<(), OcrError> {
    if page.width == 0 || page.height == 0 || page.blocks.len() > MAX_OCR_BLOCKS {
        return Err(OcrError::InvalidEvidence(
            "invalid OCR page dimensions/count".into(),
        ));
    }
    if page.revisions.len() > 64
        || page.revisions.iter().any(|revision| {
            revision.revision_id.is_empty()
                || revision.revision_id.len() > 256
                || revision.text.len() > MAX_OCR_TEXT_BYTES
        })
    {
        return Err(OcrError::InvalidEvidence(
            "OCR revisions exceed limits".into(),
        ));
    }
    let mut lines = 0usize;
    let mut words = 0usize;
    let mut text_bytes = 0usize;
    let mut block_orders = HashSet::new();
    for block in &page.blocks {
        validate_box(&block.bbox, page.width, page.height)?;
        validate_confidence(block.confidence)?;
        if block.reading_order as usize >= MAX_OCR_BLOCKS
            || !block_orders.insert(block.reading_order)
        {
            return Err(OcrError::InvalidEvidence(
                "OCR block reading order exceeds limit".into(),
            ));
        }
        lines = lines.saturating_add(block.lines.len());
        let mut line_orders = HashSet::new();
        for line in &block.lines {
            validate_box(&line.bbox, page.width, page.height)?;
            validate_confidence(line.confidence)?;
            if line.reading_order as usize >= MAX_OCR_LINES
                || !line_orders.insert(line.reading_order)
            {
                return Err(OcrError::InvalidEvidence(
                    "OCR line reading order is invalid or duplicated".into(),
                ));
            }
            words = words.saturating_add(line.words.len());
            let mut word_orders = HashSet::new();
            for word in &line.words {
                validate_box(&word.bbox, page.width, page.height)?;
                validate_confidence(word.confidence)?;
                if word.reading_order as usize >= MAX_OCR_WORDS
                    || !word_orders.insert(word.reading_order)
                {
                    return Err(OcrError::InvalidEvidence(
                        "OCR word reading order is invalid or duplicated".into(),
                    ));
                }
                if word.text.is_empty() || word.text.len() > 16 * 1024 {
                    return Err(OcrError::InvalidEvidence(
                        "OCR word length out of range".into(),
                    ));
                }
                if word.normalized_text.len() > 16 * 1024 {
                    return Err(OcrError::InvalidEvidence(
                        "normalized OCR word too long".into(),
                    ));
                }
                text_bytes = text_bytes
                    .checked_add(word.text.len() + word.normalized_text.len())
                    .ok_or_else(|| OcrError::InvalidEvidence("OCR text size overflow".into()))?;
            }
        }
    }
    if lines > MAX_OCR_LINES || words > MAX_OCR_WORDS || text_bytes > MAX_OCR_TEXT_BYTES {
        return Err(OcrError::InvalidEvidence(
            "OCR evidence exceeds resource limits".into(),
        ));
    }
    if page
        .provider_raw_artifact
        .as_ref()
        .is_some_and(|artifact| artifact.is_empty() || artifact.len() > MAX_RAW_ARTIFACT_BYTES)
    {
        return Err(OcrError::InvalidEvidence(
            "provider artifact is too large".into(),
        ));
    }
    if let Some(provenance) = &page.provider_provenance {
        if provenance.engine.is_empty()
            || provenance.engine.len() > 256
            || provenance.model.is_empty()
            || provenance.model.len() > 256
            || provenance.version.is_empty()
            || provenance.version.len() > 64
            || provenance.input_asset_sha256.len() != 64
            || !provenance
                .input_asset_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(OcrError::InvalidEvidence(
                "OCR provider provenance is incomplete".into(),
            ));
        }
        for (value, limit) in [
            (&provenance.language_profile, 128usize),
            (&provenance.model_digest, 4096),
            (&provenance.model_license, 256),
            (&provenance.model_set, 256),
        ] {
            if value
                .as_ref()
                .is_some_and(|text| text.is_empty() || text.len() > limit)
            {
                return Err(OcrError::InvalidEvidence(
                    "OCR model provenance field is out of range".into(),
                ));
            }
        }
        if provenance.parameters.len() > 128
            || provenance
                .parameters
                .iter()
                .any(|(key, value)| key.is_empty() || key.len() > 256 || value.len() > 4096)
        {
            return Err(OcrError::InvalidEvidence(
                "OCR provider parameters exceed limits".into(),
            ));
        }
    }
    Ok(())
}

fn validate_box(bbox: &OcrBox, width: u32, height: u32) -> std::result::Result<(), OcrError> {
    let values = [bbox.x, bbox.y, bbox.width, bbox.height];
    let right = bbox.x + bbox.width;
    let bottom = bbox.y + bbox.height;
    if values
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
        || !right.is_finite()
        || !bottom.is_finite()
        || right > width as f32 + 1.0
        || bottom > height as f32 + 1.0
    {
        return Err(OcrError::InvalidEvidence("OCR bbox is outside page".into()));
    }
    Ok(())
}

fn validate_confidence(confidence: f32) -> std::result::Result<(), OcrError> {
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(OcrError::InvalidEvidence(
            "OCR confidence is not in [0,1]".into(),
        ));
    }
    Ok(())
}

pub trait PageOcrProvider {
    /// Where this provider executes when recognition is attempted. This is
    /// deliberately mandatory: a future network provider must make the
    /// choice explicitly, so even failed attempts cannot silently inherit a
    /// local default in durable provenance.
    fn execution_location(&self) -> ExecutionLocation;

    fn recognize(
        &mut self,
        page_index: u32,
        image: &DynamicImage,
        input_asset_sha256: &str,
    ) -> std::result::Result<OcrPage, OcrError>;
}

/// Provider used by the base package when the optional offline OCR plugin is
/// absent. Reliable native text pages never call it; a scanned/image-only page
/// receives the supplied actionable diagnostic.
#[derive(Debug, Clone)]
pub struct OptionalOcrPluginUnavailable {
    pub diagnostic: String,
}

impl PageOcrProvider for OptionalOcrPluginUnavailable {
    fn execution_location(&self) -> ExecutionLocation {
        ExecutionLocation::Local
    }

    fn recognize(
        &mut self,
        _page_index: u32,
        _image: &DynamicImage,
        _input_asset_sha256: &str,
    ) -> std::result::Result<OcrPage, OcrError> {
        Err(OcrError::ProviderUnavailable(self.diagnostic.clone()))
    }
}

/// Deterministic provider used by integration tests and development builds.
/// It never contacts a service and deliberately emits a small, inspectable
/// block so the complete routing/package path can be exercised without a
/// model download.
#[derive(Debug, Default)]
pub struct ReferenceOcrProvider;

impl PageOcrProvider for ReferenceOcrProvider {
    fn execution_location(&self) -> ExecutionLocation {
        ExecutionLocation::Local
    }

    fn recognize(
        &mut self,
        page_index: u32,
        image: &DynamicImage,
        input_asset_sha256: &str,
    ) -> std::result::Result<OcrPage, OcrError> {
        let (width, height) = (image.width(), image.height());
        let text = format!("reference-page-{}", page_index + 1);
        let word = OcrWord {
            normalized_text: normalize_text(&text),
            text,
            bbox: OcrBox {
                x: 0.0,
                y: 0.0,
                width: (width as f32).min(240.0),
                height: (height as f32).min(40.0),
            },
            confidence: 1.0,
            reading_order: 0,
        };
        let line = OcrLine {
            bbox: word.bbox.clone(),
            confidence: 1.0,
            reading_order: 0,
            words: vec![word],
        };
        let block = OcrBlock {
            bbox: line.bbox.clone(),
            confidence: 1.0,
            reading_order: 0,
            lines: vec![line],
        };
        Ok(OcrPage {
            page_index,
            route: OcrRoute::Ocr {
                reason: OcrRouteReason::MissingText,
            },
            width,
            height,
            blocks: vec![block],
            revisions: Vec::new(),
            provider_provenance: Some(OcrProviderProvenance {
                engine: "reference".into(),
                model: "deterministic".into(),
                version: "0.1".into(),
                parameters: BTreeMap::new(),
                input_asset_sha256: input_asset_sha256.into(),
                execution_location: ExecutionLocation::Local,
                language_profile: None,
                model_digest: None,
                model_license: None,
                model_set: None,
            }),
            provider_raw_artifact: Some("reference-provider".into()),
        })
    }
}

/// Legacy RapidOCR sidecar configuration.
///
/// Retained so an expert can still reproduce the old path while debugging;
/// `scripts/ocr/rapidocr_sidecar.py` is no longer a supported default for any
/// language. New callers use [`SidecarOcrConfig`].
#[derive(Debug, Clone)]
pub struct RapidOcrConfig {
    pub executable: PathBuf,
    pub model_dir: PathBuf,
}

/// Configuration for `scripts/ocr/mpdf_ocr_sidecar.py`.
#[derive(Debug, Clone)]
pub struct SidecarOcrConfig {
    pub executable: PathBuf,
    pub model_dir: PathBuf,
    /// Optional Tesseract executable passed to the sidecar. `None` preserves
    /// the developer-configured PATH behavior; bundled runtimes always set it
    /// to their executable-adjacent engine path.
    pub engine_binary: Option<PathBuf>,
    pub engine: OcrEngine,
    pub language_profile: String,
    /// Files that must exist in `model_dir` before the provider is considered
    /// available. Empty means "do not pre-check" (legacy behavior).
    pub required_files: Vec<String>,
    /// Optional recognition passes, passed to the sidecar explicitly rather
    /// than left to its defaults.
    ///
    /// They are on the command line, and in the job fingerprint, for the same
    /// reason: they change what the evidence *is*. A page recognized with the
    /// small-type pass on is not the same evidence as one recognized without
    /// it, so flipping either must invalidate the checkpoint rather than let
    /// a resumed run mix the two.
    pub passes: SidecarPasses,
}

/// Which optional sidecar passes are enabled. Both default to off, matching
/// the sidecar's own defaults; see docs/ocr-engines.md for why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SidecarPasses {
    /// Block-level Greek script routing (`--routing`).
    pub greek_routing: bool,
    /// Small-type Latin re-recognition (`--small-type-latin`).
    pub small_type_latin: bool,
    /// Detached Greek accent band re-recognition (`--detached-greek-accents`).
    pub detached_greek_accents: bool,
}

impl SidecarPasses {
    pub fn routing_arg(self) -> &'static str {
        if self.greek_routing {
            "block-script-v1"
        } else {
            "off"
        }
    }

    pub fn small_type_arg(self) -> &'static str {
        if self.small_type_latin {
            "on"
        } else {
            "off"
        }
    }

    pub fn detached_accents_arg(self) -> &'static str {
        if self.detached_greek_accents {
            "on"
        } else {
            "off"
        }
    }

    /// Stable identity for the job fingerprint.
    pub fn fingerprint(self) -> String {
        format!(
            "routing={}|small_type_latin={}|detached_greek_accents={}",
            self.routing_arg(),
            self.small_type_arg(),
            self.detached_accents_arg()
        )
    }
}

impl SidecarOcrConfig {
    /// The production configuration: Tesseract LSTM, the pinned model set, and
    /// the requested language profile.
    pub fn tesseract(
        executable: PathBuf,
        model_dir: PathBuf,
        language_profile: &str,
    ) -> std::result::Result<Self, OcrError> {
        let required = required_model_files(language_profile).ok_or_else(|| {
            OcrError::ProviderUnavailable(format!(
                "unknown OCR language profile: {language_profile}"
            ))
        })?;
        Ok(Self {
            executable,
            model_dir,
            engine_binary: None,
            engine: OcrEngine::Tesseract,
            language_profile: language_profile.to_owned(),
            required_files: required,
            passes: SidecarPasses::default(),
        })
    }
}

fn sidecar_command(config: &SidecarOcrConfig, input: &Path) -> Command {
    let mut command = Command::new(&config.executable);
    command
        .arg("--protocol")
        .arg(OCR_PROTOCOL)
        .arg("--protocol-version")
        .arg(OCR_PROTOCOL_VERSION)
        .arg("--model-dir")
        .arg(&config.model_dir)
        .arg("--input")
        .arg(input)
        .arg("--engine")
        .arg(config.engine.as_str())
        .arg("--language-profile")
        .arg(&config.language_profile)
        .arg("--routing")
        .arg(config.passes.routing_arg())
        .arg("--small-type-latin")
        .arg(config.passes.small_type_arg())
        .arg("--detached-greek-accents")
        .arg(config.passes.detached_accents_arg());
    if let Some(path) = &config.engine_binary {
        command.arg("--engine-binary").arg(path);
    }
    command
}

#[derive(Debug, Clone)]
pub struct RapidOcrProvider {
    config: SidecarOcrConfig,
}

impl RapidOcrProvider {
    /// Legacy constructor: the old RapidOCR sidecar, whose model files are
    /// fixed and whose language profile is not selectable.
    pub fn new(config: RapidOcrConfig) -> Self {
        Self {
            config: SidecarOcrConfig {
                executable: config.executable,
                model_dir: config.model_dir,
                engine_binary: None,
                engine: OcrEngine::PaddleOcr,
                language_profile: DEFAULT_OCR_LANGUAGE_PROFILE.to_owned(),
                required_files: RAPIDOCR_MODEL_FILES
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
                passes: SidecarPasses::default(),
            },
        }
    }

    pub fn from_sidecar(config: SidecarOcrConfig) -> Self {
        Self { config }
    }

    pub fn language_profile(&self) -> &str {
        &self.config.language_profile
    }
}

/// The general local sidecar provider. `RapidOcrProvider` is the same type
/// under its historical name; this alias is what new code should use.
pub type SidecarOcrProvider = RapidOcrProvider;

#[derive(Debug, Serialize)]
struct RapidRequest<'a> {
    protocol: &'static str,
    protocol_version: &'static str,
    page_index: u32,
    input_asset_sha256: &'a str,
    /// Echoed to the sidecar so a page can never be recognized under a
    /// different profile than the one this run recorded.
    language_profile: &'a str,
}

#[derive(Debug, Deserialize)]
struct RapidResponse {
    protocol: String,
    protocol_version: String,
    page_index: u32,
    input_asset_sha256: String,
    width: u32,
    height: u32,
    blocks: Vec<OcrBlock>,
    engine: String,
    model: String,
    version: String,
    parameters: BTreeMap<String, String>,
    execution_location: ExecutionLocation,
}

impl PageOcrProvider for RapidOcrProvider {
    fn execution_location(&self) -> ExecutionLocation {
        ExecutionLocation::Local
    }

    fn recognize(
        &mut self,
        page_index: u32,
        image: &DynamicImage,
        input_asset_sha256: &str,
    ) -> std::result::Result<OcrPage, OcrError> {
        if !self.config.executable.is_file() {
            return Err(OcrError::ProviderUnavailable(format!(
                "executable is missing: {}",
                self.config.executable.display()
            )));
        }
        if !self.config.model_dir.is_dir() {
            return Err(OcrError::ProviderUnavailable(format!(
                "model directory is missing: {}",
                self.config.model_dir.display()
            )));
        }
        if let Some(name) = self
            .config
            .required_files
            .iter()
            .find(|name| !self.config.model_dir.join(name).is_file())
        {
            return Err(OcrError::ProviderUnavailable(format!(
                "local OCR model file is missing: {name}"
            )));
        }
        let temp = tempfile::NamedTempFile::new()
            .map_err(|error| OcrError::ProviderUnavailable(error.to_string()))?;
        let mut png = Cursor::new(Vec::new());
        image
            .write_to(&mut png, ImageFormat::Png)
            .map_err(|error| OcrError::ProviderFailed {
                page: page_index,
                reason: error.to_string(),
            })?;
        temp.as_file()
            .write_all(png.get_ref())
            .map_err(|error| OcrError::ProviderFailed {
                page: page_index,
                reason: error.to_string(),
            })?;
        let request = serde_json::to_string(&RapidRequest {
            protocol: OCR_PROTOCOL,
            protocol_version: OCR_PROTOCOL_VERSION,
            page_index,
            input_asset_sha256,
            language_profile: &self.config.language_profile,
        })
        .map_err(|error| OcrError::ProviderFailed {
            page: page_index,
            reason: error.to_string(),
        })?;
        // Command::new executes the configured executable directly. No shell,
        // interpolation, PATH fallback, or network operation is involved.
        let mut command = sidecar_command(&self.config, temp.path());
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Provider diagnostics may contain document text. Do not expose
            // them through the parent process or risk a stderr pipe deadlock.
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| OcrError::ProviderUnavailable(error.to_string()))?;
        let mut stdin = child.stdin.take().ok_or_else(|| OcrError::ProviderFailed {
            page: page_index,
            reason: "provider stdin unavailable".into(),
        })?;
        writeln!(stdin, "{request}").map_err(|error| OcrError::ProviderFailed {
            page: page_index,
            reason: error.to_string(),
        })?;
        drop(stdin);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OcrError::ProviderFailed {
                page: page_index,
                reason: "provider stdout unavailable".into(),
            })?;
        // Drain stdout concurrently so a provider cannot deadlock while the
        // parent waits for it. The parent owns the timeout and kills a hung
        // process; the bounded reader prevents unbounded allocation.
        let reader = thread::spawn(move || {
            let mut output = Vec::new();
            let result = BufReader::new(stdout)
                .take(MAX_PROVIDER_OUTPUT_BYTES + 1)
                .read_to_end(&mut output);
            (result, output)
        });
        let deadline = Instant::now() + MAX_PROVIDER_RUNTIME;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(OcrError::ProviderFailed {
                        page: page_index,
                        reason: "provider timed out".into(),
                    });
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(OcrError::ProviderFailed {
                        page: page_index,
                        reason: error.to_string(),
                    });
                }
            }
        };
        let (read_result, output) = reader.join().map_err(|_| OcrError::ProviderFailed {
            page: page_index,
            reason: "provider output reader failed".into(),
        })?;
        read_result.map_err(|error| OcrError::ProviderFailed {
            page: page_index,
            reason: error.to_string(),
        })?;
        if output.len() as u64 > MAX_PROVIDER_OUTPUT_BYTES {
            return Err(OcrError::ProviderFailed {
                page: page_index,
                reason: "provider output exceeds limit".into(),
            });
        }
        if !status.success() {
            if status.code() == Some(78) {
                return Err(OcrError::ProviderUnavailable(
                    "local OCR provider dependencies are unavailable".into(),
                ));
            }
            return Err(OcrError::ProviderFailed {
                page: page_index,
                reason: format!("provider exited with {status}"),
            });
        }
        let mut lines = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty());
        let line = lines.next().ok_or_else(|| OcrError::ProviderFailed {
            page: page_index,
            reason: "provider returned no response".into(),
        })?;
        if lines.next().is_some() {
            return Err(OcrError::ProviderFailed {
                page: page_index,
                reason: "provider returned multiple non-empty responses".into(),
            });
        }
        let response: RapidResponse =
            serde_json::from_slice(line).map_err(|error| OcrError::ProviderFailed {
                page: page_index,
                reason: format!("invalid provider response: {error}"),
            })?;
        validate_protocol(&response.protocol, &response.protocol_version)?;
        if response.page_index != page_index
            || response.input_asset_sha256 != input_asset_sha256
            || response.width != image.width()
            || response.height != image.height()
        {
            return Err(OcrError::ProviderFailed {
                page: page_index,
                reason: "provider response identity mismatch".into(),
            });
        }
        let raw_response =
            String::from_utf8(line.to_vec()).map_err(|_| OcrError::ProviderFailed {
                page: page_index,
                reason: "provider response is not UTF-8".into(),
            })?;
        let page = OcrPage {
            page_index,
            route: OcrRoute::Ocr {
                reason: OcrRouteReason::MissingText,
            },
            width: response.width,
            height: response.height,
            blocks: response.blocks,
            revisions: Vec::new(),
            provider_provenance: Some(OcrProviderProvenance::from_response(
                response.engine,
                response.model,
                response.version,
                response.parameters,
                input_asset_sha256.into(),
                response.execution_location,
            )),
            // Preserve the exact bounded provider JSON response; the page
            // writer also stores it under ocr/raw/ without logging it.
            provider_raw_artifact: Some(raw_response),
        };
        validate_ocr_page(&page)?;
        Ok(page)
    }
}

fn normalize_text(text: &str) -> String {
    text.nfc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn native_is_reliable(native: &NativeTextPage) -> std::result::Result<(), OcrRouteReason> {
    let trimmed = native.text.trim();
    if trimmed.is_empty() {
        return Err(OcrRouteReason::MissingText);
    }
    if trimmed.contains('\u{fffd}')
        || trimmed
            .chars()
            .filter(|character| character.is_control() && !character.is_whitespace())
            .count()
            > 2
    {
        return Err(OcrRouteReason::GarbledText);
    }
    if trimmed
        .chars()
        .filter(|character| !character.is_whitespace())
        .count()
        < 8
    {
        return Err(OcrRouteReason::TooLittleText);
    }
    Ok(())
}

fn native_page(page_index: u32, native: &NativeTextPage, width: u32, height: u32) -> OcrPage {
    // PDFium's text extraction seam currently does not expose stable glyph
    // rectangles. Keep the text structure honest (line/word records), while
    // marking boxes as page-relative approximations in the M3 documentation.
    let source_lines: Vec<&str> = native
        .text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let line_count = source_lines.len().max(1);
    let line_height = (height as f32 / line_count as f32).min(80.0);
    let mut blocks = Vec::with_capacity(source_lines.len());
    for (line_index, source_line) in source_lines.iter().enumerate() {
        let source_words: Vec<&str> = source_line.split_whitespace().collect();
        let word_count = source_words.len().max(1);
        let words = source_words
            .iter()
            .enumerate()
            .map(|(word_index, text)| {
                let word_width = width as f32 / word_count as f32;
                OcrWord {
                    normalized_text: normalize_text(text),
                    text: (*text).to_owned(),
                    bbox: OcrBox {
                        x: word_index as f32 * word_width,
                        y: line_index as f32 * line_height,
                        width: word_width,
                        height: line_height,
                    },
                    confidence: 1.0,
                    reading_order: word_index as u32,
                }
            })
            .collect::<Vec<_>>();
        let line_box = OcrBox {
            x: 0.0,
            y: line_index as f32 * line_height,
            width: width as f32,
            height: line_height,
        };
        let line = OcrLine {
            bbox: line_box.clone(),
            confidence: 1.0,
            reading_order: 0,
            words,
        };
        blocks.push(OcrBlock {
            bbox: line_box,
            confidence: 1.0,
            reading_order: line_index as u32,
            lines: vec![line],
        });
    }
    OcrPage {
        page_index,
        route: OcrRoute::NativeText,
        width,
        height,
        blocks,
        revisions: Vec::new(),
        provider_provenance: None,
        provider_raw_artifact: None,
    }
}

pub fn run_session<S: DocumentSession, P: PageOcrProvider + ?Sized>(
    session: &S,
    provider: &mut P,
    dpi: u16,
) -> std::result::Result<OcrRun, CoreError> {
    if dpi == 0 {
        return Err(CoreError::InvalidParameter(
            "OCR DPI must be positive".into(),
        ));
    }
    let mut run = OcrRun {
        protocol: OCR_PROTOCOL.into(),
        protocol_version: OCR_PROTOCOL_VERSION.into(),
        pages: Vec::new(),
        errors: Vec::new(),
    };
    for page in &session.info().pages {
        let native = session.native_text(page.index)?;
        let (width, height) = page.geometry.pixel_size(dpi)?;
        let route_reason = native_is_reliable(&native)
            .err()
            .unwrap_or(OcrRouteReason::MissingText);
        if matches!(native_is_reliable(&native), Ok(())) {
            run.pages
                .push(native_page(page.index, &native, width, height));
            continue;
        }
        // Only one raster is retained at a time. The digest is over the
        // exact PNG sent to the provider and can be persisted in M2 runs.
        let image = match session.render_page(page.index, dpi) {
            Ok(image) => image,
            Err(error) => {
                run.errors.push(OcrPageError {
                    page_index: page.index,
                    route_reason: route_reason.clone(),
                    code: "render_failed".into(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let digest = image_sha256(&image).map_err(CoreError::Image)?;
        match provider.recognize(page.index, &image, &digest) {
            Ok(mut evidence) => {
                evidence.page_index = page.index;
                evidence.route = OcrRoute::Ocr {
                    reason: route_reason.clone(),
                };
                let validation =
                    validate_page_execution_location(&evidence, provider.execution_location())
                        .and_then(|()| validate_ocr_page(&evidence));
                if let Err(error) = validation {
                    run.errors.push(OcrPageError {
                        page_index: page.index,
                        route_reason: route_reason.clone(),
                        code: "invalid_provider_evidence".into(),
                        message: error.to_string(),
                    });
                } else {
                    run.pages.push(evidence);
                }
            }
            Err(error) => run.errors.push(OcrPageError {
                page_index: page.index,
                route_reason,
                code: if matches!(error, OcrError::ProviderUnavailable(_)) {
                    "provider_unavailable".into()
                } else {
                    "provider_failed".into()
                },
                message: error.to_string(),
            }),
        }
    }
    run.validate()
        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
    Ok(run)
}

/// Durable page-at-a-time OCR orchestration. A completed DB page is reusable
/// only when its on-disk typed record exists and its digest matches; otherwise
/// the operation fails closed. Provider execution is never performed for a
/// verified completed page.
#[allow(clippy::too_many_arguments)]
pub fn run_session_durable<S: DocumentSession, P: PageOcrProvider + ?Sized>(
    session: &S,
    provider: &mut P,
    store: &JobStore,
    job_id: &str,
    job_fingerprint: &str,
    output_root: &Path,
    owner: &str,
    dpi: u16,
) -> std::result::Result<OcrRun, CoreError> {
    run_session_durable_with_cancel(
        session,
        provider,
        store,
        job_id,
        job_fingerprint,
        output_root,
        owner,
        dpi,
        &|| false,
    )
}

/// Cancellation-aware durable OCR orchestration used by the full pipeline.
/// The callback is checked around rendering and provider execution; when it
/// fires the durable job is marked cancelled before control returns.
#[allow(clippy::too_many_arguments)]
pub fn run_session_durable_with_cancel<S: DocumentSession, P: PageOcrProvider + ?Sized>(
    session: &S,
    provider: &mut P,
    store: &JobStore,
    job_id: &str,
    job_fingerprint: &str,
    output_root: &Path,
    owner: &str,
    dpi: u16,
    cancelled: &dyn Fn() -> bool,
) -> std::result::Result<OcrRun, CoreError> {
    if dpi == 0 {
        return Err(CoreError::InvalidParameter(
            "OCR DPI must be positive".into(),
        ));
    }
    prepare_ocr_directory(output_root)
        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
    let run_root = prepare_durable_run_root(output_root, job_fingerprint)
        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
    activate_ocr_run(output_root, job_fingerprint)
        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
    validate_ocr_page_files(&run_root, session.info().page_count)
        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
    store
        .ensure_job(job_id, session.info().page_count, job_fingerprint)
        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
    let mut run = OcrRun {
        protocol: OCR_PROTOCOL.into(),
        protocol_version: OCR_PROTOCOL_VERSION.into(),
        pages: Vec::new(),
        errors: Vec::new(),
    };
    for page in &session.info().pages {
        check_durable_cancelled(store, job_id, cancelled)?;
        let status = store
            .job(job_id)
            .map_err(|error| CoreError::InvalidDocument(error.to_string()))?
            .ok_or_else(|| CoreError::InvalidDocument("OCR job disappeared".into()))?;
        if status.cancel_requested {
            return Err(CoreError::Cancelled);
        }
        if let Some(record) = store
            .page(job_id, page.index)
            .map_err(|error| CoreError::InvalidDocument(error.to_string()))?
        {
            if matches!(record.status, crate::jobs::PageStatus::Completed) {
                let existing = read_ocr_page_at_root(&run_root, page.index)
                    .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
                validate_page_execution_location(&existing, provider.execution_location())
                    .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
                let page_path = run_root
                    .join("ocr/pages")
                    .join(format!("p{:06}.json", page.index.saturating_add(1)));
                let bytes = read_bounded_file(&page_path, MAX_PROVIDER_OUTPUT_BYTES)
                    .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
                let digest = crate::document_package::sha256_digest(&bytes);
                if record.artifact_digest.as_deref() != Some(digest.as_str()) {
                    return Err(CoreError::InvalidDocument(format!(
                        "completed OCR page {} digest does not match its file",
                        page.index + 1
                    )));
                }
                run.pages.push(existing);
                continue;
            }
            // A crash may have committed the page JSON and raw artifact but
            // stopped before SQLite. The page JSON is the commit marker; adopt
            // it only after full typed/raw validation, then checkpoint it
            // without invoking the provider.
            let page_path = run_root
                .join("ocr/pages")
                .join(format!("p{:06}.json", page.index.saturating_add(1)));
            if matches!(
                record.status,
                crate::jobs::PageStatus::Queued | crate::jobs::PageStatus::Running
            ) && fs::symlink_metadata(&page_path).is_ok()
            {
                let orphan = read_ocr_page_at_root(&run_root, page.index)
                    .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
                validate_page_execution_location(&orphan, provider.execution_location())
                    .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
                let claimed = store
                    .claim_page_at(job_id, owner, page.index, unix_seconds()?, 3_600)
                    .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
                if claimed.is_none() {
                    return Err(CoreError::InvalidDocument(format!(
                        "could not claim adoptable OCR page {}",
                        page.index + 1
                    )));
                }
                let bytes = read_bounded_file(&page_path, MAX_PROVIDER_OUTPUT_BYTES)
                    .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
                let digest = crate::document_package::sha256_digest(&bytes);
                let checkpointed_at = unix_seconds()?;
                if let Some(response) = provider_response_for_page(&orphan, digest.clone()) {
                    // Preserve provider provenance when adopting a page that
                    // reached disk before the SQLite transaction. Otherwise
                    // a paid remote result would survive only as a generic
                    // checkpoint with no provider-run location.
                    store
                        .record_provider_success_and_checkpoint(
                            job_id,
                            page.index,
                            owner,
                            &format!("ocr-page-{}", page.index + 1),
                            &response,
                            checkpointed_at,
                            checkpointed_at,
                        )
                        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
                } else {
                    store
                        .checkpoint_page(
                            job_id,
                            page.index,
                            owner,
                            &format!("ocr-page-{}", page.index + 1),
                            &digest,
                            checkpointed_at,
                        )
                        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
                }
                run.pages.push(orphan);
                continue;
            }
        }
        let now = unix_seconds()?;
        let claimed = store
            .claim_page_at(job_id, owner, page.index, now, 3_600)
            .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        if claimed.is_none() {
            return Err(CoreError::InvalidDocument(format!(
                "could not claim OCR page {}",
                page.index + 1
            )));
        }
        let native = match session.native_text(page.index) {
            Ok(native) => native,
            Err(error) => {
                let message = error.to_string();
                store
                    .fail_page(job_id, page.index, owner, &message, false, unix_seconds()?)
                    .map_err(|db_error| CoreError::InvalidDocument(db_error.to_string()))?;
                run.errors.push(OcrPageError {
                    page_index: page.index,
                    route_reason: OcrRouteReason::MissingText,
                    code: "native_text_failed".into(),
                    message,
                });
                break;
            }
        };
        let route_reason = native_is_reliable(&native)
            .err()
            .unwrap_or(OcrRouteReason::MissingText);
        check_durable_cancelled(store, job_id, cancelled)?;
        let evidence = if native_is_reliable(&native).is_ok() {
            let (width, height) = page.geometry.pixel_size(dpi)?;
            native_page(page.index, &native, width, height)
        } else {
            let image = match session.render_page(page.index, dpi) {
                Ok(image) => image,
                Err(error) => {
                    let message = error.to_string();
                    store
                        .fail_page(job_id, page.index, owner, &message, false, unix_seconds()?)
                        .map_err(|db_error| CoreError::InvalidDocument(db_error.to_string()))?;
                    run.errors.push(OcrPageError {
                        page_index: page.index,
                        route_reason,
                        code: "render_failed".into(),
                        message,
                    });
                    break;
                }
            };
            check_durable_cancelled(store, job_id, cancelled)?;
            let input_digest = image_sha256(&image).map_err(CoreError::Image)?;
            match provider.recognize(page.index, &image, &input_digest) {
                Ok(mut evidence) => {
                    check_durable_cancelled(store, job_id, cancelled)?;
                    evidence.page_index = page.index;
                    evidence.route = OcrRoute::Ocr {
                        reason: route_reason,
                    };
                    evidence
                }
                Err(error) => {
                    check_durable_cancelled(store, job_id, cancelled)?;
                    let provenance = ProviderProvenance {
                        // The attempt can be local or remote. Until the page
                        // provider exposes richer failure identity, keep this
                        // label neutral and let the mandatory typed location
                        // carry the privacy/billing distinction.
                        engine: "ocr-provider".into(),
                        model: "unavailable".into(),
                        version: OCR_PROTOCOL_VERSION.into(),
                        parameters: BTreeMap::new(),
                        input_asset_sha256: input_digest,
                        execution_location: provider.execution_location(),
                    };
                    store
                        .record_provider_failure(
                            job_id,
                            page.index,
                            owner,
                            &provenance,
                            &error.to_string(),
                            true,
                            now,
                            unix_seconds()?,
                        )
                        .map_err(|db_error| CoreError::InvalidDocument(db_error.to_string()))?;
                    run.errors.push(OcrPageError {
                        page_index: page.index,
                        route_reason,
                        code: if matches!(error, OcrError::ProviderUnavailable(_)) {
                            "provider_unavailable".into()
                        } else {
                            "provider_failed".into()
                        },
                        message: error.to_string(),
                    });
                    break;
                }
            }
        };
        check_durable_cancelled(store, job_id, cancelled)?;
        validate_page_execution_location(&evidence, provider.execution_location())
            .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        validate_ocr_page(&evidence)
            .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        write_ocr_page(&run_root, &evidence)
            .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        // `write_ocr_page` persists pretty JSON; hash those exact canonical
        // bytes so a whitespace or byte-level tamper cannot be mistaken for
        // a valid completed checkpoint on resume.
        let bytes = serde_json::to_vec_pretty(&evidence)
            .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        let output_digest = crate::document_package::sha256_digest(&bytes);
        if let Some(response) = provider_response_for_page(&evidence, output_digest.clone()) {
            store
                .record_provider_success_and_checkpoint(
                    job_id,
                    page.index,
                    owner,
                    &format!("ocr-page-{}", page.index + 1),
                    &response,
                    now,
                    unix_seconds()?,
                )
                .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        } else {
            store
                .checkpoint_page(
                    job_id,
                    page.index,
                    owner,
                    &format!("ocr-page-{}", page.index + 1),
                    &output_digest,
                    unix_seconds()?,
                )
                .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        }
        run.pages.push(evidence);
    }
    run.validate()
        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
    if run.errors.is_empty() && run.pages.len() == session.info().page_count as usize {
        let summary_path = run_root.join("ocr/summary.json");
        if summary_path.exists() {
            let existing = read_ocr_records_at_root(&run_root)
                .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
            if existing != run {
                return Err(CoreError::InvalidDocument(
                    "existing OCR summary differs from durable pages".into(),
                ));
            }
        } else {
            write_ocr_summary(&run_root, &run)
                .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        }
        publish_active_ocr_run(output_root, &run_root, job_fingerprint, &run)
            .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
    }
    Ok(run)
}

fn provider_response_for_page(page: &OcrPage, output_digest: String) -> Option<ProviderResponse> {
    let provenance = page.provider_provenance.as_ref()?;
    Some(ProviderResponse {
        protocol: JOB_PROTOCOL.into(),
        protocol_version: JOB_PROTOCOL_VERSION.into(),
        output_digest,
        provenance: ProviderProvenance {
            engine: provenance.engine.clone(),
            model: provenance.model.clone(),
            version: provenance.version.clone(),
            parameters: provenance.parameters.clone(),
            input_asset_sha256: provenance.input_asset_sha256.clone(),
            execution_location: provenance.execution_location,
        },
    })
}

fn validate_page_execution_location(
    page: &OcrPage,
    declared_location: ExecutionLocation,
) -> std::result::Result<(), OcrError> {
    let Some(provenance) = &page.provider_provenance else {
        return Ok(());
    };
    if provenance.execution_location != declared_location {
        return Err(OcrError::InvalidEvidence(format!(
            "OCR provider execution location mismatch: provider declared {}, page recorded {}",
            declared_location.as_str(),
            provenance.execution_location.as_str()
        )));
    }
    Ok(())
}

fn check_durable_cancelled(
    store: &JobStore,
    job_id: &str,
    cancelled: &dyn Fn() -> bool,
) -> std::result::Result<(), CoreError> {
    if !cancelled() {
        return Ok(());
    }
    store
        .request_cancel(job_id)
        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
    Err(CoreError::Cancelled)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActiveOcrRun {
    protocol: String,
    protocol_version: String,
    fingerprint_sha256: String,
}

fn fingerprint_namespace(job_fingerprint: &str) -> String {
    crate::document_package::sha256_digest(job_fingerprint.as_bytes())
}

fn prepare_durable_run_root(
    output_root: &Path,
    job_fingerprint: &str,
) -> std::result::Result<PathBuf, OcrError> {
    let ocr_dir = output_root.join("ocr");
    let runs_dir = ocr_dir.join("runs");
    ensure_real_directory(&runs_dir, "OCR runs")?;
    let run_root = runs_dir.join(fingerprint_namespace(job_fingerprint));
    ensure_real_directory(&run_root, "OCR run")?;
    prepare_ocr_directory(&run_root)?;
    Ok(run_root)
}

fn ensure_real_directory(path: &Path, label: &str) -> std::result::Result<(), OcrError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(OcrError::Package(format!(
            "{label} is not a real directory"
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| OcrError::Package(error.to_string()))
        }
        Err(error) => Err(OcrError::Package(error.to_string())),
    }
}

fn active_ocr_storage_root(root: &Path) -> std::result::Result<PathBuf, OcrError> {
    let pointer_path = root.join("ocr/active-run.json");
    let metadata = match fs::symlink_metadata(&pointer_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(root.to_path_buf()),
        Err(error) => return Err(OcrError::Package(error.to_string())),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(OcrError::Package(
            "OCR active-run pointer is not a real file".into(),
        ));
    }
    let pointer: ActiveOcrRun = serde_json::from_slice(&read_bounded_file(
        &pointer_path,
        MAX_PROVIDER_OUTPUT_BYTES,
    )?)
    .map_err(|error| OcrError::Package(error.to_string()))?;
    if pointer.protocol != OCR_ACTIVE_RUN_PROTOCOL
        || pointer.protocol_version != OCR_PROTOCOL_VERSION
        || pointer.fingerprint_sha256.len() != 64
        || !pointer
            .fingerprint_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(OcrError::Package(
            "OCR active-run pointer is invalid".into(),
        ));
    }
    let run_root = root.join("ocr/runs").join(pointer.fingerprint_sha256);
    let metadata =
        fs::symlink_metadata(&run_root).map_err(|error| OcrError::Package(error.to_string()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(OcrError::Package(
            "OCR active run is not a real directory".into(),
        ));
    }
    Ok(run_root)
}

fn publish_active_ocr_run(
    output_root: &Path,
    run_root: &Path,
    job_fingerprint: &str,
    run: &OcrRun,
) -> std::result::Result<(), OcrError> {
    // Keep the legacy top-level summary as a compatibility marker for package
    // readers, then atomically switch the typed active-run pointer. Readers in
    // this crate resolve the pointer and never combine pages from two runs.
    atomic_replace_json(
        &output_root.join("ocr/summary.json"),
        run,
        MAX_OCR_SUMMARY_BYTES,
    )?;
    activate_ocr_run(output_root, job_fingerprint)?;
    let pointer = active_ocr_run(job_fingerprint);
    let expected_root = output_root
        .join("ocr/runs")
        .join(&pointer.fingerprint_sha256);
    if expected_root != run_root {
        return Err(OcrError::Package(
            "OCR active-run namespace does not match fingerprint".into(),
        ));
    }
    Ok(())
}

fn active_ocr_run(job_fingerprint: &str) -> ActiveOcrRun {
    ActiveOcrRun {
        protocol: OCR_ACTIVE_RUN_PROTOCOL.into(),
        protocol_version: OCR_PROTOCOL_VERSION.into(),
        fingerprint_sha256: fingerprint_namespace(job_fingerprint),
    }
}

fn activate_ocr_run(
    output_root: &Path,
    job_fingerprint: &str,
) -> std::result::Result<(), OcrError> {
    atomic_replace_json(
        &output_root.join("ocr/active-run.json"),
        &active_ocr_run(job_fingerprint),
        MAX_PROVIDER_OUTPUT_BYTES,
    )
}

fn unix_seconds() -> std::result::Result<i64, CoreError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .map_err(|error| CoreError::InvalidDocument(format!("system clock before epoch: {error}")))
}

fn image_sha256(image: &DynamicImage) -> std::result::Result<String, image::ImageError> {
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png)?;
    let mut hasher = Sha256::new();
    hasher.update(png.get_ref());
    Ok(format_digest(hasher.finalize().as_slice()))
}

fn format_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Writes the typed OCR extension records with create-new/no-clobber
/// installation. A partial provider run remains visibly incomplete in
/// `summary.json`; it is never silently promoted to a successful package.
pub fn write_ocr_records(root: &Path, run: &OcrRun) -> std::result::Result<(), OcrError> {
    run.validate()?;
    prepare_ocr_directory(root)?;
    let ocr_dir = root.join("ocr");
    let pages_dir = ocr_dir.join("pages");
    for page in &run.pages {
        let page_number = page
            .page_index
            .checked_add(1)
            .ok_or_else(|| OcrError::Package("OCR page index overflow".into()))?;
        let path = pages_dir.join(format!("p{page_number:06}.json"));
        write_raw_artifact(root, page)?;
        atomic_create_json(&path, page, MAX_PROVIDER_OUTPUT_BYTES)?;
    }
    atomic_create_json(&ocr_dir.join("summary.json"), run, MAX_OCR_SUMMARY_BYTES)?;
    Ok(())
}

pub fn prepare_ocr_directory(root: &Path) -> std::result::Result<(), OcrError> {
    let ocr_dir = root.join("ocr");
    let pages_dir = ocr_dir.join("pages");
    match fs::symlink_metadata(&ocr_dir) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(OcrError::Package(
                "OCR directory is not a real directory".into(),
            ))
        }
        Err(_) => fs::create_dir(&ocr_dir).map_err(|error| OcrError::Package(error.to_string()))?,
    }
    match fs::symlink_metadata(&pages_dir) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(OcrError::Package(
                "OCR pages is not a real directory".into(),
            ))
        }
        Err(_) => {
            fs::create_dir(&pages_dir).map_err(|error| OcrError::Package(error.to_string()))?
        }
    }
    let raw_dir = ocr_dir.join("raw");
    match fs::symlink_metadata(&raw_dir) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err(OcrError::Package("OCR raw is not a real directory".into())),
        Err(_) => fs::create_dir(&raw_dir).map_err(|error| OcrError::Package(error.to_string()))?,
    }
    Ok(())
}

pub fn write_ocr_summary(root: &Path, run: &OcrRun) -> std::result::Result<(), OcrError> {
    run.validate()?;
    if !run.errors.is_empty() {
        return Err(OcrError::Package(
            "cannot finalize an incomplete OCR run".into(),
        ));
    }
    atomic_create_json(&root.join("ocr/summary.json"), run, MAX_OCR_SUMMARY_BYTES)
}

/// Installs exactly one page record. The durable orchestrator calls this
/// before its SQLite checkpoint; an interrupted write therefore cannot make
/// a completed database page appear successful on resume.
pub fn write_ocr_page(root: &Path, page: &OcrPage) -> std::result::Result<(), OcrError> {
    validate_ocr_page(page)?;
    let pages_dir = root.join("ocr/pages");
    if !pages_dir.is_dir() {
        return Err(OcrError::Package("OCR pages directory is missing".into()));
    }
    let page_number = page
        .page_index
        .checked_add(1)
        .ok_or_else(|| OcrError::Package("OCR page index overflow".into()))?;
    // Raw provider output is durable first; the page JSON is the commit
    // marker. A crash can therefore leave an orphan raw file but not a
    // database-completed page without its evidence.
    write_raw_artifact(root, page)?;
    atomic_create_json(
        &pages_dir.join(format!("p{page_number:06}.json")),
        page,
        MAX_PROVIDER_OUTPUT_BYTES,
    )
}

fn raw_artifact_path(root: &Path, page_index: u32) -> PathBuf {
    root.join("ocr/raw")
        .join(format!("p{:06}.raw", page_index.saturating_add(1)))
}

fn write_raw_artifact(root: &Path, page: &OcrPage) -> std::result::Result<(), OcrError> {
    let Some(raw) = &page.provider_raw_artifact else {
        return Ok(());
    };
    let path = raw_artifact_path(root, page.page_index);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > MAX_RAW_ARTIFACT_BYTES as u64
            {
                return Err(OcrError::Package("OCR raw artifact is unsafe".into()));
            }
            if read_bounded_file(&path, MAX_RAW_ARTIFACT_BYTES as u64)? != raw.as_bytes() {
                return Err(OcrError::Package(
                    "existing OCR raw artifact differs".into(),
                ));
            }
            Ok(())
        }
        Err(_) => atomic_create_bytes(
            &path,
            raw.as_bytes(),
            MAX_RAW_ARTIFACT_BYTES as u64,
            "OCR raw artifact is too large",
        ),
    }
}

fn verify_raw_artifact(root: &Path, page: &OcrPage) -> std::result::Result<(), OcrError> {
    let Some(raw) = &page.provider_raw_artifact else {
        return Ok(());
    };
    let path = raw_artifact_path(root, page.page_index);
    let metadata =
        fs::symlink_metadata(&path).map_err(|error| OcrError::Package(error.to_string()))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_RAW_ARTIFACT_BYTES as u64
    {
        return Err(OcrError::Package(
            "OCR raw artifact is missing or unsafe".into(),
        ));
    }
    let bytes = read_bounded_file(&path, MAX_RAW_ARTIFACT_BYTES as u64)?;
    if bytes != raw.as_bytes() {
        return Err(OcrError::Package(
            "OCR raw artifact differs from page record".into(),
        ));
    }
    Ok(())
}

fn read_bounded_file(path: &Path, limit: u64) -> std::result::Result<Vec<u8>, OcrError> {
    let mut bytes = Vec::new();
    BufReader::new(File::open(path).map_err(|error| OcrError::Package(error.to_string()))?)
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| OcrError::Package(error.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(OcrError::Package(
            "OCR record exceeds its byte limit".into(),
        ));
    }
    Ok(bytes)
}

pub fn read_ocr_page(root: &Path, page_index: u32) -> std::result::Result<OcrPage, OcrError> {
    let storage_root = active_ocr_storage_root(root)?;
    read_ocr_page_at_root(&storage_root, page_index)
}

fn read_ocr_page_at_root(root: &Path, page_index: u32) -> std::result::Result<OcrPage, OcrError> {
    let page_number = page_index
        .checked_add(1)
        .ok_or_else(|| OcrError::Package("OCR page index overflow".into()))?;
    let path = root
        .join("ocr/pages")
        .join(format!("p{page_number:06}.json"));
    let metadata =
        fs::symlink_metadata(&path).map_err(|error| OcrError::Package(error.to_string()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(OcrError::Package(
            "OCR page record is not a real file".into(),
        ));
    }
    if metadata.len() > MAX_PROVIDER_OUTPUT_BYTES {
        return Err(OcrError::Package("OCR page record is too large".into()));
    }
    let page: OcrPage =
        serde_json::from_slice(&read_bounded_file(&path, MAX_PROVIDER_OUTPUT_BYTES)?)
            .map_err(|error| OcrError::Package(error.to_string()))?;
    if page.page_index != page_index {
        return Err(OcrError::Package(
            "OCR page index does not match path".into(),
        ));
    }
    validate_ocr_page(&page)?;
    verify_raw_artifact(root, &page)?;
    Ok(page)
}

pub fn read_ocr_records(root: &Path) -> std::result::Result<OcrRun, OcrError> {
    let storage_root = active_ocr_storage_root(root)?;
    read_ocr_records_at_root(&storage_root)
}

fn read_ocr_records_at_root(root: &Path) -> std::result::Result<OcrRun, OcrError> {
    let summary_path = root.join("ocr/summary.json");
    let summary_metadata = fs::symlink_metadata(&summary_path)
        .map_err(|error| OcrError::Package(error.to_string()))?;
    if !summary_metadata.is_file()
        || summary_metadata.file_type().is_symlink()
        || summary_metadata.len() > MAX_OCR_SUMMARY_BYTES
    {
        return Err(OcrError::Package(
            "OCR summary is missing, unsafe, or too large".into(),
        ));
    }
    let run: OcrRun =
        serde_json::from_slice(&read_bounded_file(&summary_path, MAX_OCR_SUMMARY_BYTES)?)
            .map_err(|error| OcrError::Package(error.to_string()))?;
    run.validate()?;
    let pages_dir = root.join("ocr/pages");
    if let Ok(metadata) = fs::symlink_metadata(&pages_dir) {
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(OcrError::Package(
                "OCR pages is not a real directory".into(),
            ));
        }
    } else if !run.pages.is_empty() {
        return Err(OcrError::Package("OCR pages directory is missing".into()));
    }
    for expected in &run.pages {
        let page_number = expected
            .page_index
            .checked_add(1)
            .ok_or_else(|| OcrError::Package("OCR page index overflow".into()))?;
        let path = pages_dir.join(format!("p{page_number:06}.json"));
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| OcrError::Package(error.to_string()))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(OcrError::Package(
                "OCR page record is not a real file".into(),
            ));
        }
        if metadata.len() > MAX_PROVIDER_OUTPUT_BYTES {
            return Err(OcrError::Package("OCR page record is too large".into()));
        }
        let actual: OcrPage =
            serde_json::from_slice(&read_bounded_file(&path, MAX_PROVIDER_OUTPUT_BYTES)?)
                .map_err(|error| OcrError::Package(error.to_string()))?;
        if &actual != expected {
            return Err(OcrError::Package(
                "OCR page record differs from summary".into(),
            ));
        }
        verify_raw_artifact(root, &actual)?;
    }
    let mut expected_names = HashSet::new();
    for page in &run.pages {
        let number = page
            .page_index
            .checked_add(1)
            .ok_or_else(|| OcrError::Package("OCR page index overflow".into()))?;
        expected_names.insert(format!("p{number:06}.json"));
    }
    let entries = fs::read_dir(&pages_dir).map_err(|error| OcrError::Package(error.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|error| OcrError::Package(error.to_string()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !expected_names.contains(&name) {
            return Err(OcrError::Package(
                "OCR pages contains an unexpected record".into(),
            ));
        }
    }
    Ok(run)
}

/// Rejects page records that cannot belong to the current source before any
/// durable checkpoint is considered reusable. This also rejects symlinks and
/// malformed names in a partial (summary-less) run.
fn validate_ocr_page_files(root: &Path, page_count: u32) -> std::result::Result<(), OcrError> {
    let pages_dir = root.join("ocr/pages");
    let entries = fs::read_dir(&pages_dir).map_err(|error| OcrError::Package(error.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|error| OcrError::Package(error.to_string()))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| OcrError::Package(error.to_string()))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(OcrError::Package(
                "OCR pages contains an unsafe entry".into(),
            ));
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(number) = name
            .strip_prefix('p')
            .and_then(|value| value.strip_suffix(".json"))
            .filter(|value| value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|value| value.parse::<u32>().ok())
        else {
            return Err(OcrError::Package(
                "OCR pages contains an unexpected record".into(),
            ));
        };
        if number == 0 || number > page_count {
            return Err(OcrError::Package(
                "OCR page record is outside the source".into(),
            ));
        }
    }
    Ok(())
}

/// `limit` is passed explicitly because a single page record and a
/// whole-document summary are bounded differently; see
/// [`MAX_OCR_SUMMARY_BYTES`].
fn atomic_create_json<T: Serialize>(
    path: &Path,
    value: &T,
    limit: u64,
) -> std::result::Result<(), OcrError> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| OcrError::Package(error.to_string()))?;
    atomic_create_bytes(path, &bytes, limit, "OCR JSON record is too large")
}

fn atomic_replace_json<T: Serialize>(
    path: &Path,
    value: &T,
    limit: u64,
) -> std::result::Result<(), OcrError> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| OcrError::Package(error.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(OcrError::Package("OCR JSON record is too large".into()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| OcrError::Package("OCR path has no parent".into()))?;
    let mut temp = tempfile::Builder::new()
        .prefix(".mpdf-ocr-active-")
        .tempfile_in(parent)
        .map_err(|error| OcrError::Package(error.to_string()))?;
    temp.write_all(&bytes)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|error| OcrError::Package(error.to_string()))?;
    temp.persist(path)
        .map_err(|error| OcrError::Package(error.error.to_string()))?;
    Ok(())
}

fn atomic_create_bytes(
    path: &Path,
    bytes: &[u8],
    limit: u64,
    too_large: &str,
) -> std::result::Result<(), OcrError> {
    if bytes.len() as u64 > limit {
        return Err(OcrError::Package(too_large.into()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| OcrError::Package("OCR path has no parent".into()))?;
    let temp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| OcrError::Package(error.to_string()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| OcrError::Package(error.to_string()))?;
    match fs::hard_link(&temp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&temp);
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(OcrError::Package(error.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{PdfDocumentInfo, PdfPageInfo};
    use crate::document_session::DocumentSession;
    use crate::error::Result;
    use crate::page_geometry::{PageGeometry, PageRotation};
    use crate::source_identity::SourceIdentity;

    struct MockSession {
        info: PdfDocumentInfo,
        text: Vec<String>,
    }
    impl DocumentSession for MockSession {
        fn info(&self) -> &PdfDocumentInfo {
            &self.info
        }
        fn source_identity(&self) -> &SourceIdentity {
            panic!("not needed")
        }
        fn pdfium_library_description(&self) -> String {
            "mock".into()
        }
        fn render_page(&self, _index: u32, _dpi: u16) -> Result<DynamicImage> {
            Ok(DynamicImage::new_rgb8(100, 100))
        }
        fn native_text(&self, index: u32) -> Result<NativeTextPage> {
            Ok(NativeTextPage {
                text: self.text[index as usize].clone(),
            })
        }
    }

    fn session(text: &[&str]) -> MockSession {
        MockSession {
            info: PdfDocumentInfo {
                page_count: text.len() as u32,
                pages: text
                    .iter()
                    .enumerate()
                    .map(|(index, _)| PdfPageInfo {
                        index: index as u32,
                        geometry: PageGeometry::new(100.0, 100.0).unwrap(),
                        source_rotation: PageRotation::None,
                    })
                    .collect(),
                metadata: Default::default(),
                source_bytes: 1,
            },
            text: text.iter().map(|value| (*value).into()).collect(),
        }
    }

    #[test]
    fn bundled_engine_is_bound_to_sidecar_argv_while_legacy_config_stays_optional() {
        let mut config = SidecarOcrConfig::tesseract(
            PathBuf::from("/bundle/ocr-runtime/bin/mpdf-ocr-sidecar"),
            PathBuf::from("/bundle/ocr-runtime/tessdata"),
            "english",
        )
        .unwrap();
        let legacy = sidecar_command(&config, Path::new("/tmp/page.png"));
        assert!(!legacy.get_args().any(|arg| arg == "--engine-binary"));

        config.engine_binary = Some(PathBuf::from("/bundle/ocr-runtime/bin/tesseract"));
        let bundled = sidecar_command(&config, Path::new("/tmp/page.png"));
        let args: Vec<_> = bundled
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let position = args
            .iter()
            .position(|arg| arg == "--engine-binary")
            .expect("bundled engine must be explicit in sidecar argv");
        assert_eq!(args[position + 1], "/bundle/ocr-runtime/bin/tesseract");
    }

    #[test]
    fn native_text_is_not_sent_to_provider() {
        let mut provider = ReferenceOcrProvider;
        let run = run_session(
            &session(&["reliable native text layer"]),
            &mut provider,
            300,
        )
        .unwrap();
        assert!(matches!(run.pages[0].route, OcrRoute::NativeText));
    }

    #[test]
    fn base_package_without_plugin_keeps_native_text_but_explains_scanned_pages() {
        let diagnostic = "scanned pages require the optional offline OCR plugin";
        let mut provider = OptionalOcrPluginUnavailable {
            diagnostic: diagnostic.into(),
        };
        let native = run_session(
            &session(&["reliable native text layer"]),
            &mut provider,
            300,
        )
        .unwrap();
        assert!(native.errors.is_empty());
        assert!(matches!(native.pages[0].route, OcrRoute::NativeText));

        let scanned = run_session(&session(&[""]), &mut provider, 300).unwrap();
        assert!(scanned.pages.is_empty());
        assert_eq!(scanned.errors[0].code, "provider_unavailable");
        assert!(scanned.errors[0].message.contains(diagnostic));
    }

    #[test]
    fn native_text_builds_approximate_line_and_word_structure() {
        let mut provider = ReferenceOcrProvider;
        let run = run_session(&session(&["one two\nthree e\u{301}"]), &mut provider, 300).unwrap();
        let page = &run.pages[0];
        assert_eq!(page.blocks.len(), 2);
        assert_eq!(page.blocks[0].lines[0].words.len(), 2);
        assert_eq!(page.blocks[1].lines[0].words[1].normalized_text, "é");
    }

    #[test]
    fn missing_text_uses_reference_provider() {
        let mut provider = ReferenceOcrProvider;
        let run = run_session(&session(&[""]), &mut provider, 300).unwrap();
        assert!(matches!(run.pages[0].route, OcrRoute::Ocr { .. }));
        run.validate().unwrap();
    }

    #[test]
    fn normalized_text_uses_unicode_nfc() {
        let composed = normalize_text("é");
        let decomposed = normalize_text("e\u{301}");
        assert_eq!(composed, decomposed);
    }

    #[test]
    fn invalid_confidence_is_rejected() {
        let mut provider = ReferenceOcrProvider;
        let mut run = run_session(&session(&[""]), &mut provider, 300).unwrap();
        run.pages[0].blocks[0].confidence = f32::NAN;
        assert!(run.validate().is_err());
    }

    #[test]
    fn rapidocr_missing_installation_is_diagnostic_and_offline() {
        let dir = tempfile::tempdir().unwrap();
        let mut provider = RapidOcrProvider::new(RapidOcrConfig {
            executable: dir.path().join("missing-provider"),
            model_dir: dir.path().join("missing-models"),
        });
        let error = provider
            .recognize(0, &DynamicImage::new_rgb8(10, 10), &"a".repeat(64))
            .unwrap_err();
        assert!(matches!(error, OcrError::ProviderUnavailable(_)));
        assert!(error.to_string().contains("missing"));
    }

    #[test]
    fn extension_is_no_clobber_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut provider = ReferenceOcrProvider;
        let run = run_session(&session(&[""]), &mut provider, 300).unwrap();
        write_ocr_records(dir.path(), &run).unwrap();
        assert!(write_ocr_records(dir.path(), &run).is_err());
        assert_eq!(read_ocr_records(dir.path()).unwrap(), run);
    }

    #[test]
    fn execution_location_round_trips_in_ocr_records_and_reads_legacy_values() {
        let dir = tempfile::tempdir().unwrap();
        let mut provider = ReferenceOcrProvider;
        let mut run = run_session(&session(&["", "", ""]), &mut provider, 300).unwrap();
        for (page, location) in run.pages.iter_mut().zip([
            ExecutionLocation::Local,
            ExecutionLocation::BrokeredCloud,
            ExecutionLocation::LegacyUserKey,
        ]) {
            page.provider_provenance
                .as_mut()
                .unwrap()
                .execution_location = location;
        }

        write_ocr_records(dir.path(), &run).unwrap();
        assert_eq!(read_ocr_records(dir.path()).unwrap(), run);

        let canonical = serde_json::to_string(&run.pages[2]).unwrap();
        assert!(canonical.contains("remote:user-key"));
        let legacy = canonical.replace("remote:user-key", "remote_user_key");
        let migrated: OcrPage = serde_json::from_str(&legacy).unwrap();
        assert_eq!(
            migrated.provider_provenance.unwrap().execution_location,
            ExecutionLocation::LegacyUserKey
        );
    }

    #[test]
    fn a_whole_book_summary_is_not_bounded_by_the_single_response_limit() {
        // The regression this pins: `ocr/summary.json` holds every page, so
        // bounding it with the per-page provider limit made the pipeline die
        // partway through any real book. A dense 300-dpi scanned page costs
        // roughly 200 KB of typed evidence, so a modest volume is already far
        // past 8 MB. Synthetic fixtures are small enough never to notice.
        const REAL_PAGE_EVIDENCE_BYTES: u64 = 200 * 1024;
        const A_MODEST_BOOK: u64 = 160;
        // These are facts about the constants, so they are checked when the
        // crate is built rather than when the suite is run: a future edit that
        // reintroduces the bug fails to compile.
        const _: () = assert!(
            REAL_PAGE_EVIDENCE_BYTES * A_MODEST_BOOK > MAX_PROVIDER_OUTPUT_BYTES,
            "the per-response limit must not be able to hold a whole book, \
             or this test is not measuring the bug it exists for"
        );
        const _: () = assert!(
            MAX_OCR_SUMMARY_BYTES > REAL_PAGE_EVIDENCE_BYTES * A_MODEST_BOOK,
            "the summary bound must accommodate a real book"
        );
        // A 1000-page volume must still fit.
        const _: () = assert!(MAX_OCR_SUMMARY_BYTES >= REAL_PAGE_EVIDENCE_BYTES * 1000);
    }

    #[test]
    fn typed_json_records_are_not_limited_to_raw_artifact_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.json");
        let value = "x".repeat(MAX_RAW_ARTIFACT_BYTES + 1);
        atomic_create_json(&path, &value, MAX_PROVIDER_OUTPUT_BYTES).unwrap();
        assert!(path.metadata().unwrap().len() > MAX_RAW_ARTIFACT_BYTES as u64);

        let raw_path = dir.path().join("large.raw");
        let error = atomic_create_bytes(
            &raw_path,
            value.as_bytes(),
            MAX_RAW_ARTIFACT_BYTES as u64,
            "OCR raw artifact is too large",
        )
        .unwrap_err();
        assert!(error.to_string().contains("raw artifact is too large"));
        assert!(!raw_path.exists());
    }

    struct CountingProvider {
        calls: usize,
    }
    impl PageOcrProvider for CountingProvider {
        fn execution_location(&self) -> ExecutionLocation {
            ExecutionLocation::Local
        }

        fn recognize(
            &mut self,
            page_index: u32,
            image: &DynamicImage,
            digest: &str,
        ) -> std::result::Result<OcrPage, OcrError> {
            self.calls += 1;
            ReferenceOcrProvider.recognize(page_index, image, digest)
        }
    }

    struct LocatedProvider {
        calls: usize,
        location: ExecutionLocation,
        fail: bool,
    }

    impl PageOcrProvider for LocatedProvider {
        fn execution_location(&self) -> ExecutionLocation {
            self.location
        }

        fn recognize(
            &mut self,
            page_index: u32,
            image: &DynamicImage,
            digest: &str,
        ) -> std::result::Result<OcrPage, OcrError> {
            self.calls += 1;
            if self.fail {
                return Err(OcrError::ProviderFailed {
                    page: page_index,
                    reason: "remote fixture failed".into(),
                });
            }
            let mut page = ReferenceOcrProvider.recognize(page_index, image, digest)?;
            page.provider_provenance
                .as_mut()
                .unwrap()
                .execution_location = self.location;
            Ok(page)
        }
    }

    struct MismatchedLocationProvider;

    impl PageOcrProvider for MismatchedLocationProvider {
        fn execution_location(&self) -> ExecutionLocation {
            ExecutionLocation::BrokeredCloud
        }

        fn recognize(
            &mut self,
            page_index: u32,
            image: &DynamicImage,
            digest: &str,
        ) -> std::result::Result<OcrPage, OcrError> {
            // Reference evidence is explicitly local, contradicting this
            // provider's brokered-cloud declaration.
            ReferenceOcrProvider.recognize(page_index, image, digest)
        }
    }

    #[test]
    fn durable_run_rejects_a_remote_provider_that_labels_its_page_local() {
        let dir = tempfile::tempdir().unwrap();
        let store = JobStore::open(&dir.path().join("jobs.sqlite")).unwrap();
        let mut provider = MismatchedLocationProvider;
        let error = run_session_durable(
            &session(&[""]),
            &mut provider,
            &store,
            "mismatched-location",
            "mismatched-location-fingerprint",
            dir.path(),
            "worker",
            300,
        )
        .unwrap_err();
        assert!(error.to_string().contains("execution location mismatch"));
        assert!(store
            .provider_runs("mismatched-location")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn durable_remote_success_and_failure_keep_their_execution_location() {
        let dir = tempfile::tempdir().unwrap();
        let store = JobStore::open(&dir.path().join("jobs.sqlite")).unwrap();
        let document = session(&[""]);

        let mut success = LocatedProvider {
            calls: 0,
            location: ExecutionLocation::BrokeredCloud,
            fail: false,
        };
        let run = run_session_durable(
            &document,
            &mut success,
            &store,
            "brokered-success",
            "brokered-success-fingerprint",
            dir.path(),
            "worker-success",
            300,
        )
        .unwrap();
        assert_eq!(
            run.pages[0]
                .provider_provenance
                .as_ref()
                .unwrap()
                .execution_location,
            ExecutionLocation::BrokeredCloud
        );
        assert_eq!(
            store.provider_runs("brokered-success").unwrap()[0].execution_location,
            ExecutionLocation::BrokeredCloud
        );

        let mut failure = LocatedProvider {
            calls: 0,
            location: ExecutionLocation::BrokeredCloud,
            fail: true,
        };
        let failed = run_session_durable(
            &document,
            &mut failure,
            &store,
            "brokered-failure",
            "brokered-failure-fingerprint",
            dir.path(),
            "worker-failure",
            300,
        )
        .unwrap();
        assert_eq!(failed.errors.len(), 1);
        let attempts = store.provider_runs("brokered-failure").unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].outcome, crate::jobs::ProviderOutcome::Failed);
        assert_eq!(
            attempts[0].execution_location,
            ExecutionLocation::BrokeredCloud
        );
    }

    #[test]
    fn adopting_a_remote_page_restores_provider_run_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let store = JobStore::open(&dir.path().join("jobs.sqlite")).unwrap();
        let document = session(&[""]);
        let job_id = "brokered-adoption";
        let fingerprint = "brokered-adoption-fingerprint";
        store.ensure_job(job_id, 1, fingerprint).unwrap();

        prepare_ocr_directory(dir.path()).unwrap();
        let run_root = prepare_durable_run_root(dir.path(), fingerprint).unwrap();
        let image = document.render_page(0, 300).unwrap();
        let digest = image_sha256(&image).unwrap();
        let mut page = ReferenceOcrProvider.recognize(0, &image, &digest).unwrap();
        page.provider_provenance
            .as_mut()
            .unwrap()
            .execution_location = ExecutionLocation::BrokeredCloud;
        write_ocr_page(&run_root, &page).unwrap();

        let mut provider = LocatedProvider {
            calls: 0,
            location: ExecutionLocation::BrokeredCloud,
            fail: false,
        };
        let adopted = run_session_durable(
            &document,
            &mut provider,
            &store,
            job_id,
            fingerprint,
            dir.path(),
            "worker",
            300,
        )
        .unwrap();
        assert_eq!(provider.calls, 0);
        assert_eq!(adopted.pages, vec![page]);
        let attempts = store.provider_runs(job_id).unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].outcome, crate::jobs::ProviderOutcome::Succeeded);
        assert_eq!(
            attempts[0].execution_location,
            ExecutionLocation::BrokeredCloud
        );
    }

    #[test]
    fn adopting_a_remote_job_rejects_an_orphan_page_labelled_local() {
        let dir = tempfile::tempdir().unwrap();
        let store = JobStore::open(&dir.path().join("jobs.sqlite")).unwrap();
        let document = session(&[""]);
        let job_id = "brokered-adoption-mismatch";
        let fingerprint = "brokered-adoption-mismatch-fingerprint";
        store.ensure_job(job_id, 1, fingerprint).unwrap();

        prepare_ocr_directory(dir.path()).unwrap();
        let run_root = prepare_durable_run_root(dir.path(), fingerprint).unwrap();
        let image = document.render_page(0, 300).unwrap();
        let digest = image_sha256(&image).unwrap();
        let local_page = ReferenceOcrProvider.recognize(0, &image, &digest).unwrap();
        write_ocr_page(&run_root, &local_page).unwrap();

        let mut provider = LocatedProvider {
            calls: 0,
            location: ExecutionLocation::BrokeredCloud,
            fail: false,
        };
        let error = run_session_durable(
            &document,
            &mut provider,
            &store,
            job_id,
            fingerprint,
            dir.path(),
            "worker",
            300,
        )
        .unwrap_err();
        assert!(error.to_string().contains("execution location mismatch"));
        assert_eq!(provider.calls, 0);
        assert!(store.provider_runs(job_id).unwrap().is_empty());
    }

    #[test]
    fn durable_rerun_skips_verified_completed_pages() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("jobs.sqlite");
        let store = JobStore::open(&database).unwrap();
        let document = session(&vec![""; 100]);
        let mut first = CountingProvider { calls: 0 };
        let first_run = run_session_durable(
            &document,
            &mut first,
            &store,
            "ocr-demo",
            "source-and-provider-v1",
            dir.path(),
            "worker",
            300,
        )
        .unwrap();
        assert_eq!(first.calls, 100);
        assert_eq!(first_run.pages.len(), 100);
        let mut second = CountingProvider { calls: 0 };
        let second_run = run_session_durable(
            &document,
            &mut second,
            &store,
            "ocr-demo",
            "source-and-provider-v1",
            dir.path(),
            "worker",
            300,
        )
        .unwrap();
        assert_eq!(second.calls, 0);
        assert_eq!(second_run.pages, first_run.pages);
    }

    struct CancelingProvider {
        calls: usize,
        database: PathBuf,
    }

    struct ExternalCancelingProvider {
        calls: usize,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl PageOcrProvider for ExternalCancelingProvider {
        fn execution_location(&self) -> ExecutionLocation {
            ExecutionLocation::Local
        }

        fn recognize(
            &mut self,
            page_index: u32,
            image: &DynamicImage,
            digest: &str,
        ) -> std::result::Result<OcrPage, OcrError> {
            self.calls += 1;
            self.cancelled
                .store(true, std::sync::atomic::Ordering::SeqCst);
            ReferenceOcrProvider.recognize(page_index, image, digest)
        }
    }

    impl PageOcrProvider for CancelingProvider {
        fn execution_location(&self) -> ExecutionLocation {
            ExecutionLocation::Local
        }

        fn recognize(
            &mut self,
            page_index: u32,
            image: &DynamicImage,
            digest: &str,
        ) -> std::result::Result<OcrPage, OcrError> {
            self.calls += 1;
            if page_index == 1 {
                let store = JobStore::open(&self.database).unwrap();
                store.request_cancel("cancel-demo").unwrap();
            }
            ReferenceOcrProvider.recognize(page_index, image, digest)
        }
    }

    #[test]
    fn durable_cancel_retains_committed_pages_and_stops_before_next_page() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("jobs.sqlite");
        let store = JobStore::open(&database).unwrap();
        let document = session(&["", "", ""]);
        let mut provider = CancelingProvider {
            calls: 0,
            database: database.clone(),
        };
        let error = run_session_durable(
            &document,
            &mut provider,
            &store,
            "cancel-demo",
            "source-and-provider-v1",
            dir.path(),
            "worker",
            300,
        )
        .unwrap_err();
        assert!(matches!(error, CoreError::Cancelled));
        assert_eq!(provider.calls, 2);
        let progress = store.progress("cancel-demo").unwrap().unwrap();
        assert_eq!(progress.completed_pages, 2);
        assert_eq!(progress.cancelled_pages, 1);
        assert!(read_ocr_page(dir.path(), 0).is_ok());
        assert!(read_ocr_page(dir.path(), 1).is_ok());
        assert!(read_ocr_page(dir.path(), 2).is_err());
    }

    #[test]
    fn external_cancel_marks_the_durable_job_and_does_not_commit_the_inflight_page() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("jobs.sqlite");
        let store = JobStore::open(&database).unwrap();
        let document = session(&["", ""]);
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut provider = ExternalCancelingProvider {
            calls: 0,
            cancelled: cancelled.clone(),
        };

        let error = run_session_durable_with_cancel(
            &document,
            &mut provider,
            &store,
            "external-cancel",
            "source-and-provider-v1",
            dir.path(),
            "worker",
            300,
            &|| cancelled.load(std::sync::atomic::Ordering::SeqCst),
        )
        .unwrap_err();

        assert!(matches!(error, CoreError::Cancelled));
        assert_eq!(provider.calls, 1);
        assert!(
            store
                .job("external-cancel")
                .unwrap()
                .unwrap()
                .cancel_requested
        );
        assert!(read_ocr_page(dir.path(), 0).is_err());
    }

    #[test]
    fn durable_resume_fails_closed_when_completed_page_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("jobs.sqlite");
        let store = JobStore::open(&database).unwrap();
        let document = session(&[""]);
        let mut first = CountingProvider { calls: 0 };
        run_session_durable(
            &document,
            &mut first,
            &store,
            "mismatch-demo",
            "source-and-provider-v1",
            dir.path(),
            "worker",
            300,
        )
        .unwrap();
        let page_path = prepare_durable_run_root(dir.path(), "source-and-provider-v1")
            .unwrap()
            .join("ocr/pages/p000001.json");
        OpenOptions::new()
            .append(true)
            .open(&page_path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        let mut second = CountingProvider { calls: 0 };
        assert!(run_session_durable(
            &document,
            &mut second,
            &store,
            "mismatch-demo",
            "source-and-provider-v1",
            dir.path(),
            "worker",
            300,
        )
        .is_err());
        assert_eq!(second.calls, 0);
    }

    struct FailOnceProvider {
        calls: usize,
        failed: bool,
    }

    impl PageOcrProvider for FailOnceProvider {
        fn execution_location(&self) -> ExecutionLocation {
            ExecutionLocation::Local
        }

        fn recognize(
            &mut self,
            page_index: u32,
            image: &DynamicImage,
            digest: &str,
        ) -> std::result::Result<OcrPage, OcrError> {
            self.calls += 1;
            if !self.failed {
                self.failed = true;
                return Err(OcrError::ProviderFailed {
                    page: page_index,
                    reason: "transient test failure".into(),
                });
            }
            ReferenceOcrProvider.recognize(page_index, image, digest)
        }
    }

    #[test]
    fn durable_provider_failure_is_retryable_and_preserves_both_runs() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("jobs.sqlite");
        let store = JobStore::open(&database).unwrap();
        let document = session(&[""]);
        let mut provider = FailOnceProvider {
            calls: 0,
            failed: false,
        };
        let first = run_session_durable(
            &document,
            &mut provider,
            &store,
            "retry-demo",
            "source-and-provider-v1",
            dir.path(),
            "worker",
            300,
        )
        .unwrap();
        assert!(!first.is_complete(1));
        assert_eq!(
            store.page("retry-demo", 0).unwrap().unwrap().status,
            crate::jobs::PageStatus::Queued
        );
        let second = run_session_durable(
            &document,
            &mut provider,
            &store,
            "retry-demo",
            "source-and-provider-v1",
            dir.path(),
            "worker",
            300,
        )
        .unwrap();
        assert!(second.is_complete(1));
        assert_eq!(provider.calls, 2);
        let runs = store.provider_runs("retry-demo").unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].outcome, crate::jobs::ProviderOutcome::Failed);
        assert_eq!(runs[1].outcome, crate::jobs::ProviderOutcome::Succeeded);
    }

    #[test]
    fn durable_new_job_does_not_adopt_a_legacy_page() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("jobs.sqlite");
        let store = JobStore::open(&database).unwrap();
        let document = session(&[""]);
        prepare_ocr_directory(dir.path()).unwrap();
        let image = document.render_page(0, 300).unwrap();
        let digest = image_sha256(&image).unwrap();
        let page = ReferenceOcrProvider.recognize(0, &image, &digest).unwrap();
        write_ocr_page(dir.path(), &page).unwrap();
        let mut provider = CountingProvider { calls: 0 };
        let run = run_session_durable(
            &document,
            &mut provider,
            &store,
            "new-job-after-crash",
            "source-and-provider-v1",
            dir.path(),
            "worker",
            300,
        )
        .unwrap();
        assert!(run.is_complete(1));
        assert_eq!(provider.calls, 1);
        assert_eq!(
            store
                .page("new-job-after-crash", 0)
                .unwrap()
                .unwrap()
                .status,
            crate::jobs::PageStatus::Completed
        );
    }

    #[test]
    fn durable_different_fingerprints_use_distinct_page_namespaces() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("jobs.sqlite");
        let store = JobStore::open(&database).unwrap();
        let document = session(&[""]);
        let mut first = CountingProvider { calls: 0 };
        run_session_durable(
            &document,
            &mut first,
            &store,
            "local-job",
            "local-fingerprint",
            dir.path(),
            "worker-local",
            300,
        )
        .unwrap();
        let mut second = CountingProvider { calls: 0 };
        run_session_durable(
            &document,
            &mut second,
            &store,
            "cloud-job",
            "cloud-fingerprint",
            dir.path(),
            "worker-cloud",
            300,
        )
        .unwrap();

        assert_eq!(first.calls, 1);
        assert_eq!(second.calls, 1);
        let local_page = prepare_durable_run_root(dir.path(), "local-fingerprint")
            .unwrap()
            .join("ocr/pages/p000001.json");
        let cloud_page = prepare_durable_run_root(dir.path(), "cloud-fingerprint")
            .unwrap()
            .join("ocr/pages/p000001.json");
        assert!(local_page.is_file());
        assert!(cloud_page.is_file());
        assert_ne!(local_page, cloud_page);
        assert!(read_ocr_records(dir.path()).unwrap().is_complete(1));
    }

    #[test]
    fn durable_reuses_orphan_raw_without_treating_it_as_a_page() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("jobs.sqlite");
        let store = JobStore::open(&database).unwrap();
        let document = session(&[""]);
        prepare_ocr_directory(dir.path()).unwrap();
        fs::write(
            dir.path().join("ocr/raw/p000001.raw"),
            b"reference-provider",
        )
        .unwrap();
        let mut provider = CountingProvider { calls: 0 };
        let run = run_session_durable(
            &document,
            &mut provider,
            &store,
            "orphan-raw",
            "source-and-provider-v1",
            dir.path(),
            "worker",
            300,
        )
        .unwrap();
        assert!(run.is_complete(1));
        assert_eq!(provider.calls, 1);
    }
}
