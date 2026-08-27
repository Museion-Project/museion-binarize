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
pub const CANONICAL_OCR_DPI: u16 = 300;
pub const MAX_OCR_BLOCKS: usize = 4_096;
pub const MAX_OCR_LINES: usize = 16_384;
pub const MAX_OCR_WORDS: usize = 65_536;
pub const MAX_OCR_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_RAW_ARTIFACT_BYTES: usize = 1024 * 1024;
pub const MAX_PROVIDER_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_PROVIDER_STDERR_BYTES: u64 = 16 * 1024;
pub const MAX_PROVIDER_RUNTIME: Duration = Duration::from_secs(120);
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
    pub execution_location: String,
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
            validate_page(page)?;
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

fn validate_page(page: &OcrPage) -> std::result::Result<(), OcrError> {
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
            || provenance.execution_location.is_empty()
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
    fn recognize(
        &mut self,
        page_index: u32,
        image: &DynamicImage,
        input_asset_sha256: &str,
    ) -> std::result::Result<OcrPage, OcrError>;
}

/// Deterministic provider used by integration tests and development builds.
/// It never contacts a service and deliberately emits a small, inspectable
/// block so the complete routing/package path can be exercised without a
/// model download.
#[derive(Debug, Default)]
pub struct ReferenceOcrProvider;

impl PageOcrProvider for ReferenceOcrProvider {
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
                execution_location: "local".into(),
            }),
            provider_raw_artifact: Some("reference-provider".into()),
        })
    }
}

#[derive(Debug, Clone)]
pub struct RapidOcrConfig {
    pub executable: PathBuf,
    pub model_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RapidOcrProvider {
    config: RapidOcrConfig,
}

impl RapidOcrProvider {
    pub fn new(config: RapidOcrConfig) -> Self {
        Self { config }
    }
}

#[derive(Debug, Serialize)]
struct RapidRequest<'a> {
    protocol: &'static str,
    protocol_version: &'static str,
    page_index: u32,
    input_asset_sha256: &'a str,
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
    execution_location: String,
}

impl PageOcrProvider for RapidOcrProvider {
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
        if let Some(name) = RAPIDOCR_MODEL_FILES
            .iter()
            .find(|name| !self.config.model_dir.join(name).is_file())
        {
            return Err(OcrError::ProviderUnavailable(format!(
                "RapidOCR model file is missing: {name}"
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
        })
        .map_err(|error| OcrError::ProviderFailed {
            page: page_index,
            reason: error.to_string(),
        })?;
        // Command::new executes the configured executable directly. No shell,
        // interpolation, PATH fallback, or network operation is involved.
        let mut child = Command::new(&self.config.executable)
            .arg("--protocol")
            .arg(OCR_PROTOCOL)
            .arg("--protocol-version")
            .arg(OCR_PROTOCOL_VERSION)
            .arg("--model-dir")
            .arg(&self.config.model_dir)
            .arg("--input")
            .arg(temp.path())
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
            provider_provenance: Some(OcrProviderProvenance {
                engine: response.engine,
                model: response.model,
                version: response.version,
                parameters: response.parameters,
                input_asset_sha256: input_asset_sha256.into(),
                execution_location: response.execution_location,
            }),
            // Preserve the exact bounded provider JSON response; the page
            // writer also stores it under ocr/raw/ without logging it.
            provider_raw_artifact: Some(raw_response),
        };
        validate_page(&page)?;
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
                if let Err(error) = validate_page(&evidence) {
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
    if dpi == 0 {
        return Err(CoreError::InvalidParameter(
            "OCR DPI must be positive".into(),
        ));
    }
    prepare_ocr_directory(output_root)
        .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
    validate_ocr_page_files(output_root, session.info().page_count)
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
                let existing = read_ocr_page(output_root, page.index)
                    .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
                let page_path = output_root
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
            let page_path = output_root
                .join("ocr/pages")
                .join(format!("p{:06}.json", page.index.saturating_add(1)));
            if matches!(
                record.status,
                crate::jobs::PageStatus::Queued | crate::jobs::PageStatus::Running
            ) && fs::symlink_metadata(&page_path).is_ok()
            {
                let orphan = read_ocr_page(output_root, page.index)
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
                store
                    .checkpoint_page(
                        job_id,
                        page.index,
                        owner,
                        &format!("ocr-page-{}", page.index + 1),
                        &digest,
                        unix_seconds()?,
                    )
                    .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
                run.pages.push(orphan);
                continue;
            }
        }
        let now = unix_seconds()?;
        let claimed = store
            .claim_page(job_id, owner, now, 3_600)
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
            let input_digest = image_sha256(&image).map_err(CoreError::Image)?;
            match provider.recognize(page.index, &image, &input_digest) {
                Ok(mut evidence) => {
                    evidence.page_index = page.index;
                    evidence.route = OcrRoute::Ocr {
                        reason: route_reason,
                    };
                    evidence
                }
                Err(error) => {
                    let provenance = ProviderProvenance {
                        engine: "local-ocr".into(),
                        model: "unavailable".into(),
                        version: OCR_PROTOCOL_VERSION.into(),
                        parameters: BTreeMap::new(),
                        input_asset_sha256: input_digest,
                        execution_location: ExecutionLocation::Local,
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
        validate_page(&evidence).map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        write_ocr_page(output_root, &evidence)
            .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        // `write_ocr_page` persists pretty JSON; hash those exact canonical
        // bytes so a whitespace or byte-level tamper cannot be mistaken for
        // a valid completed checkpoint on resume.
        let bytes = serde_json::to_vec_pretty(&evidence)
            .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        let output_digest = crate::document_package::sha256_digest(&bytes);
        if let Some(provenance) = &evidence.provider_provenance {
            let response = ProviderResponse {
                protocol: JOB_PROTOCOL.into(),
                protocol_version: JOB_PROTOCOL_VERSION.into(),
                output_digest,
                provenance: ProviderProvenance {
                    engine: provenance.engine.clone(),
                    model: provenance.model.clone(),
                    version: provenance.version.clone(),
                    parameters: provenance.parameters.clone(),
                    input_asset_sha256: provenance.input_asset_sha256.clone(),
                    execution_location: ExecutionLocation::Local,
                },
            };
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
        let summary_path = output_root.join("ocr/summary.json");
        if summary_path.exists() {
            let existing = read_ocr_records(output_root)
                .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
            if existing != run {
                return Err(CoreError::InvalidDocument(
                    "existing OCR summary differs from durable pages".into(),
                ));
            }
        } else {
            write_ocr_summary(output_root, &run)
                .map_err(|error| CoreError::InvalidDocument(error.to_string()))?;
        }
    }
    Ok(run)
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
        atomic_create_json(&path, page)?;
    }
    atomic_create_json(&ocr_dir.join("summary.json"), run)?;
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
    atomic_create_json(&root.join("ocr/summary.json"), run)
}

/// Installs exactly one page record. The durable orchestrator calls this
/// before its SQLite checkpoint; an interrupted write therefore cannot make
/// a completed database page appear successful on resume.
pub fn write_ocr_page(root: &Path, page: &OcrPage) -> std::result::Result<(), OcrError> {
    validate_page(page)?;
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
    atomic_create_json(&pages_dir.join(format!("p{page_number:06}.json")), page)
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
        Err(_) => atomic_create_bytes(&path, raw.as_bytes()),
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
    validate_page(&page)?;
    verify_raw_artifact(root, &page)?;
    Ok(page)
}

pub fn read_ocr_records(root: &Path) -> std::result::Result<OcrRun, OcrError> {
    let summary_path = root.join("ocr/summary.json");
    let summary_metadata = fs::symlink_metadata(&summary_path)
        .map_err(|error| OcrError::Package(error.to_string()))?;
    if !summary_metadata.is_file()
        || summary_metadata.file_type().is_symlink()
        || summary_metadata.len() > MAX_PROVIDER_OUTPUT_BYTES
    {
        return Err(OcrError::Package(
            "OCR summary is missing, unsafe, or too large".into(),
        ));
    }
    let run: OcrRun = serde_json::from_slice(&read_bounded_file(
        &summary_path,
        MAX_PROVIDER_OUTPUT_BYTES,
    )?)
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

fn atomic_create_json<T: Serialize>(path: &Path, value: &T) -> std::result::Result<(), OcrError> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| OcrError::Package(error.to_string()))?;
    atomic_create_bytes(path, &bytes)
}

fn atomic_create_bytes(path: &Path, bytes: &[u8]) -> std::result::Result<(), OcrError> {
    if bytes.len() > MAX_RAW_ARTIFACT_BYTES {
        return Err(OcrError::Package("OCR raw artifact is too large".into()));
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

    struct CountingProvider {
        calls: usize,
    }
    impl PageOcrProvider for CountingProvider {
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

    impl PageOcrProvider for CancelingProvider {
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
        let page_path = dir.path().join("ocr/pages/p000001.json");
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
    fn durable_adopts_page_written_before_database_checkpoint_and_new_job() {
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
        assert_eq!(provider.calls, 0);
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
