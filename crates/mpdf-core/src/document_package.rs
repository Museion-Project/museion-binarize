//! Machine-readable Document Package (MDP) 0.1.
//!
//! The package is deliberately provider-neutral: it records source evidence,
//! page geometry and typed future-evidence slots, but does not contain OCR
//! provider names, cloud credentials, or rendered copies of the source PDF.
//! Paths in the container are package-relative and every digest is SHA-256.

use std::collections::HashSet;
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::document_session::{PdfDocumentSession, PdfOpenOptions};
use crate::error::{CoreError, Result};

pub const MDP_SCHEMA: &str = "mpdf-document-package";
pub const MDP_SCHEMA_VERSION: &str = "0.1";
pub const CANONICAL_MASTER_DPI: u16 = 300;
pub const MAX_PAGES: usize = 10_000;
pub const MAX_ASSETS: usize = 10_000;
pub const MAX_PROVENANCE_STEPS: usize = 100_000;
pub const MAX_ASSET_BYTES: u64 = 1_073_741_824;
pub const MAX_TOTAL_ASSET_BYTES: u64 = 2_147_483_648;
const MAX_JSON_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema: String,
    pub schema_version: String,
    pub document_id: String,
    pub source_id: String,
    pub page_count: u32,
    pub asset_count: u32,
    /// Technical producer information only; no display/brand name is part
    /// of the stable document identity.
    pub tool: ToolInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SourceKind {
    #[serde(rename = "pdf")]
    Pdf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub source_id: String,
    pub kind: SourceKind,
    pub content_sha256: String,
    pub byte_len: u64,
    pub page_count: u32,
    /// A non-sensitive basename retained for human traceability. The PDF
    /// itself remains an external reference unless `packaged_path` is set.
    pub external_reference: Option<String>,
    pub packaged_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Page {
    pub page_id: String,
    pub physical_index: u32,
    pub order: u32,
    pub rotation_degrees: u16,
    pub master_space: CoordinateSpace,
    pub source_space: CoordinateSpace,
    pub transforms: Vec<AffineTransform>,
    pub printed_page_label: Option<PrintedPageLabel>,
    pub existing_outline_evidence: Vec<ExistingOutlineEvidence>,
    pub typography_evidence: Vec<TypographyEvidence>,
    pub region_evidence: Vec<RegionEvidence>,
    pub asset_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoordinateSpace {
    pub id: String,
    pub unit: CoordinateUnit,
    pub width: f64,
    pub height: f64,
    pub origin: Origin,
    pub pixels_per_inch: Option<u16>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CoordinateUnit {
    #[serde(rename = "pixels")]
    Pixels,
    #[serde(rename = "pdf_points")]
    PdfPoints,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Origin {
    #[serde(rename = "top_left")]
    TopLeft,
    #[serde(rename = "bottom_left")]
    BottomLeft,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AffineTransform {
    pub from_space: String,
    pub to_space: String,
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rect {
    pub space_id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrintedPageLabel {
    pub label: String,
    pub source: PrintedLabelSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PrintedLabelSource {
    #[serde(rename = "observed")]
    Observed,
    #[serde(rename = "inferred")]
    Inferred,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExistingOutlineEvidence {
    pub title: String,
    pub level: u16,
    pub target_page_id: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypographyEvidence {
    pub role: String,
    pub bounds: Rect,
    pub font_size_points: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegionEvidence {
    pub kind: String,
    pub bounds: Rect,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AssetKind {
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "master")]
    Master,
    #[serde(rename = "thumbnail")]
    Thumbnail,
    #[serde(rename = "bilevel")]
    Bilevel,
}

impl AssetKind {
    fn stable_name(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Master => "master",
            Self::Thumbnail => "thumbnail",
            Self::Bilevel => "bilevel",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Asset {
    pub asset_id: String,
    pub kind: AssetKind,
    pub path: String,
    pub mime_type: String,
    pub byte_len: u64,
    pub content_sha256: String,
    pub page_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProvenanceStep {
    pub step_id: String,
    pub operation: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub parameters: std::collections::BTreeMap<String, String>,
    pub software: String,
    pub software_version: String,
    pub execution: ExecutionKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionKind {
    #[serde(rename = "local")]
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationSummary {
    pub schema: String,
    pub schema_version: String,
    pub valid: bool,
    pub checked_pages: u32,
    pub checked_assets: u32,
    pub errors: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationIssue {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentPackage {
    pub manifest: Manifest,
    pub source: Source,
    pub pages: Vec<Page>,
    pub assets: Vec<Asset>,
    pub provenance: Vec<ProvenanceStep>,
    pub validation: ValidationSummary,
}

// Named views keep the six MDP concerns discoverable without splitting the
// schema across crates or introducing a provider-specific dependency.
pub mod manifest {
    pub use super::{Manifest, ToolInfo};
}
pub mod source {
    pub use super::{Source, SourceKind};
}
pub mod pages {
    pub use super::{AffineTransform, CoordinateSpace, CoordinateUnit, Origin, Page, Rect};
}
pub mod assets {
    pub use super::{Asset, AssetKind};
}
pub mod provenance {
    pub use super::{ExecutionKind, ProvenanceStep};
}
pub mod validation {
    pub use super::{ValidationIssue, ValidationSummary};
}

impl DocumentPackage {
    /// Builds an evidence-only package from the same persistent PDF session
    /// used by `inspect`. No PDF is copied into the package.
    pub fn create_from_pdf(input: &Path, options: &PdfOpenOptions) -> Result<Self> {
        let session = PdfDocumentSession::open(
            input,
            &PdfOpenOptions {
                compute_source_hash: true,
                ..options.clone()
            },
        )?;
        let source_name = input
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned);
        Self::create_from_session(&session, source_name)
    }

    /// Builds the base package from an already-open session. OCR and other
    /// page consumers use this entry point to guarantee one source snapshot
    /// and one PDFium document for the whole operation.
    pub fn create_from_session(
        session: &PdfDocumentSession,
        source_name: Option<String>,
    ) -> Result<Self> {
        let identity = session.source_identity();
        let digest = identity
            .content_sha256
            .clone()
            .ok_or_else(|| package_error("source digest was not captured"))?;
        let document_id = document_id(&digest);
        let native_outline = session.native_outline()?;
        let mut pages = Vec::with_capacity(session.info().pages.len());
        for page in &session.info().pages {
            let width = f64::from(page.geometry.width_points);
            let height = f64::from(page.geometry.height_points);
            let (master_width, master_height) = page.geometry.pixel_size(CANONICAL_MASTER_DPI)?;
            let master_width = f64::from(master_width);
            let master_height = f64::from(master_height);
            let master = CoordinateSpace {
                id: format!("page-{}-master", page.index + 1),
                unit: CoordinateUnit::Pixels,
                width: master_width,
                height: master_height,
                origin: Origin::TopLeft,
                pixels_per_inch: Some(CANONICAL_MASTER_DPI),
            };
            let source = CoordinateSpace {
                id: format!("page-{}-pdf", page.index + 1),
                unit: CoordinateUnit::PdfPoints,
                width,
                height,
                origin: Origin::BottomLeft,
                pixels_per_inch: None,
            };
            pages.push(Page {
                page_id: page_id(&digest, page.index),
                physical_index: page.index,
                order: page.index,
                rotation_degrees: page.source_rotation.degrees() as u16,
                master_space: master.clone(),
                source_space: source.clone(),
                transforms: vec![AffineTransform {
                    from_space: source.id,
                    to_space: master.id,
                    a: master_width / width,
                    b: 0.0,
                    c: 0.0,
                    d: -master_height / height,
                    e: 0.0,
                    f: master_height,
                }],
                printed_page_label: None,
                existing_outline_evidence: Vec::new(),
                typography_evidence: Vec::new(),
                region_evidence: Vec::new(),
                asset_ids: Vec::new(),
            });
        }
        let outline_evidence = native_outline
            .into_iter()
            .map(|item| {
                let target_page_id = pages
                    .get(item.page_index as usize)
                    .map(|page| page.page_id.clone())
                    .ok_or_else(|| package_error("source outline target page is out of range"))?;
                Ok(ExistingOutlineEvidence {
                    title: item.title,
                    level: item.level,
                    target_page_id: Some(target_page_id),
                    source: "source-pdf".to_owned(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if !outline_evidence.is_empty() {
            pages
                .first_mut()
                .ok_or_else(|| package_error("source outline exists without any pages"))?
                .existing_outline_evidence = outline_evidence;
        }
        let source = Source {
            source_id: source_id(&digest),
            kind: SourceKind::Pdf,
            content_sha256: digest,
            byte_len: identity.byte_len,
            page_count: session.info().page_count,
            external_reference: source_name,
            packaged_path: None,
        };
        let manifest = Manifest {
            schema: MDP_SCHEMA.to_owned(),
            schema_version: MDP_SCHEMA_VERSION.to_owned(),
            document_id,
            source_id: source.source_id.clone(),
            page_count: pages.len() as u32,
            asset_count: 0,
            tool: ToolInfo {
                name: "mpdf".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        };
        let provenance_inputs = vec![source.source_id.clone()];
        let provenance_outputs = pages.iter().map(|page| page.page_id.clone()).collect();
        let mut package = Self {
            manifest,
            source,
            pages,
            assets: Vec::new(),
            provenance: vec![ProvenanceStep {
                step_id: "step-source-inspect".to_owned(),
                operation: "source_inspect".to_owned(),
                inputs: provenance_inputs,
                outputs: provenance_outputs,
                parameters: std::collections::BTreeMap::new(),
                software: "mpdf".to_owned(),
                software_version: env!("CARGO_PKG_VERSION").to_owned(),
                execution: ExecutionKind::Local,
            }],
            validation: empty_validation(),
        };
        package.validate()?;
        package.validation = package.validation_report()?;
        Ok(package)
    }

    pub fn validate(&self) -> Result<()> {
        validate_version(&self.manifest.schema, &self.manifest.schema_version)?;
        if self.manifest.tool.name.is_empty() || self.manifest.tool.version.is_empty() {
            return Err(package_error(
                "manifest tool name/version must not be empty",
            ));
        }
        if self.manifest.document_id != document_id(&self.source.content_sha256) {
            return Err(package_error("document_id does not match source digest"));
        }
        if self.manifest.source_id != self.source.source_id
            || self.source.source_id != source_id(&self.source.content_sha256)
        {
            return Err(package_error("source_id does not match source digest"));
        }
        if self.pages.len() > MAX_PAGES || self.assets.len() > MAX_ASSETS {
            return Err(package_error(
                "page or asset count exceeds the package limit",
            ));
        }
        if self.provenance.len() > MAX_PROVENANCE_STEPS {
            return Err(package_error("provenance count exceeds the package limit"));
        }
        if self.manifest.page_count as usize != self.pages.len()
            || self.manifest.asset_count as usize != self.assets.len()
        {
            return Err(package_error(
                "manifest counts do not match package contents",
            ));
        }
        if self.source.page_count == 0 || self.pages.is_empty() {
            return Err(package_error("MDP packages must contain at least one page"));
        }
        if self.source.page_count != self.pages.len() as u32 {
            return Err(package_error("source page_count does not match pages"));
        }
        validate_digest(&self.source.content_sha256, "source")?;
        if let Some(path) = &self.source.packaged_path {
            validate_relative_path(path)?;
            if !path.starts_with("source/") {
                return Err(package_error("packaged source must be under source/"));
            }
        }
        let mut ids = HashSet::new();
        ids.insert(self.source.source_id.clone());
        let mut step_ids = HashSet::new();
        for (expected, page) in self.pages.iter().enumerate() {
            if page.physical_index as usize != expected || page.order as usize != expected {
                return Err(package_error(
                    "page order must be contiguous and deterministic",
                ));
            }
            if !ids.insert(page.page_id.clone()) {
                return Err(package_error("duplicate page ID"));
            }
            validate_stable_id(&page.page_id, "page")?;
            if page.page_id != page_id(&self.source.content_sha256, page.physical_index) {
                return Err(package_error(
                    "page ID does not match source digest and physical index",
                ));
            }
            validate_space(&page.master_space)?;
            validate_space(&page.source_space)?;
            if page.master_space.id == page.source_space.id
                || page.master_space.unit != CoordinateUnit::Pixels
                || page.master_space.origin != Origin::TopLeft
                || page.master_space.pixels_per_inch != Some(CANONICAL_MASTER_DPI)
                || page.source_space.unit != CoordinateUnit::PdfPoints
                || page.source_space.origin != Origin::BottomLeft
                || page.source_space.pixels_per_inch.is_some()
            {
                return Err(package_error(
                    "page coordinate spaces do not meet the MDP master/source contract",
                ));
            }
            if !matches!(page.rotation_degrees, 0 | 90 | 180 | 270) {
                return Err(package_error("page rotation must be 0, 90, 180, or 270"));
            }
            if page.transforms.len() != 1 {
                return Err(package_error(
                    "page must declare exactly one affine transform",
                ));
            }
            let mut source_to_master = 0;
            for transform in &page.transforms {
                validate_transform(transform, &page.master_space, &page.source_space)?;
                if transform.from_space == page.source_space.id
                    && transform.to_space == page.master_space.id
                {
                    source_to_master += 1;
                }
            }
            if source_to_master != 1 {
                return Err(package_error(
                    "page must declare one source-to-master transform",
                ));
            }
            for evidence in page
                .typography_evidence
                .iter()
                .map(|e| &e.bounds)
                .chain(page.region_evidence.iter().map(|e| &e.bounds))
            {
                validate_rect(evidence, &page.master_space, &page.source_space)?;
            }
            for evidence in &page.typography_evidence {
                if let Some(size) = evidence.font_size_points {
                    if !size.is_finite() || size < 0.0 {
                        return Err(package_error("typography evidence has invalid font size"));
                    }
                }
            }
        }
        let page_ids: HashSet<_> = self.pages.iter().map(|p| p.page_id.as_str()).collect();
        let mut asset_ids = HashSet::new();
        let mut asset_paths = HashSet::new();
        let mut total = 0u64;
        for asset in &self.assets {
            validate_digest(&asset.content_sha256, "asset")?;
            validate_relative_path(&asset.path)?;
            if !asset.path.starts_with("assets/") {
                return Err(package_error("asset must be under assets/"));
            }
            if asset.mime_type.is_empty() {
                return Err(package_error("asset MIME type must not be empty"));
            }
            validate_stable_id(&asset.asset_id, "asset")?;
            if !asset_ids.insert(asset.asset_id.clone()) || !ids.insert(asset.asset_id.clone()) {
                return Err(package_error("duplicate asset ID"));
            }
            if asset.asset_id
                != asset_id_for_sha256(asset.page_id.as_deref(), asset.kind, &asset.content_sha256)
            {
                return Err(package_error(
                    "asset ID does not match scope, kind, and digest",
                ));
            }
            if !asset_paths.insert(asset.path.clone()) {
                return Err(package_error("duplicate asset path"));
            }
            if asset.byte_len > MAX_ASSET_BYTES {
                return Err(package_error("asset exceeds the per-asset size limit"));
            }
            total = total
                .checked_add(asset.byte_len)
                .ok_or_else(|| package_error("asset size total overflow"))?;
            if total > MAX_TOTAL_ASSET_BYTES {
                return Err(package_error("assets exceed the total size limit"));
            }
            if let Some(page_id) = &asset.page_id {
                if !page_ids.contains(page_id.as_str()) {
                    return Err(package_error("asset references an unknown page"));
                }
            }
        }
        for page in &self.pages {
            let mut page_asset_ids = HashSet::new();
            for asset_id in &page.asset_ids {
                if !page_asset_ids.insert(asset_id) {
                    return Err(package_error("page asset IDs must be unique"));
                }
                let Some(asset) = self.assets.iter().find(|asset| &asset.asset_id == asset_id)
                else {
                    return Err(package_error("page references an unknown asset"));
                };
                if asset.page_id.as_deref() != Some(page.page_id.as_str()) {
                    return Err(package_error("page and asset page scopes disagree"));
                }
            }
            for outline in &page.existing_outline_evidence {
                if let Some(target) = &outline.target_page_id {
                    if !page_ids.contains(target.as_str()) {
                        return Err(package_error("outline evidence references an unknown page"));
                    }
                }
            }
        }
        for asset in &self.assets {
            if let Some(page_id) = &asset.page_id {
                let page = self
                    .pages
                    .iter()
                    .find(|page| &page.page_id == page_id)
                    .ok_or_else(|| package_error("asset references an unknown page"))?;
                if !page.asset_ids.iter().any(|id| id == &asset.asset_id) {
                    return Err(package_error("asset is not listed by its page"));
                }
            }
        }
        for step in &self.provenance {
            if step.step_id.is_empty()
                || step.operation.is_empty()
                || step.software.is_empty()
                || step.software_version.is_empty()
            {
                return Err(package_error("provenance fields must not be empty"));
            }
            if !step_ids.insert(step.step_id.clone()) {
                return Err(package_error("duplicate provenance step ID"));
            }
            for reference in step.inputs.iter().chain(step.outputs.iter()) {
                if !ids.contains(reference) {
                    return Err(package_error("provenance references an unknown ID"));
                }
            }
            let mut references = HashSet::new();
            for reference in step.inputs.iter().chain(step.outputs.iter()) {
                if !references.insert(reference) {
                    return Err(package_error(
                        "provenance references must be unique within a step",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn validation_report(&self) -> Result<ValidationSummary> {
        self.validate()?;
        Ok(ValidationSummary {
            schema: self.manifest.schema.clone(),
            schema_version: self.manifest.schema_version.clone(),
            valid: true,
            checked_pages: self.pages.len() as u32,
            checked_assets: self.assets.len() as u32,
            errors: Vec::new(),
        })
    }

    /// Atomically creates a new package directory. Existing destinations are
    /// never overwritten; the source PDF is represented by digest/reference.
    pub fn write_to(&self, output: &Path) -> Result<()> {
        self.validate()?;
        if fs::symlink_metadata(output).is_ok() {
            return Err(CoreError::DestinationConflict(format!(
                "destination already exists: {}",
                output.display()
            )));
        }
        let parent = output.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
        let temp = tempfile::tempdir_in(parent).map_err(|e| package_error(e.to_string()))?;
        fs::create_dir(temp.path().join("pages")).map_err(|e| CoreError::io(temp.path(), e))?;
        fs::create_dir(temp.path().join("assets")).map_err(|e| CoreError::io(temp.path(), e))?;
        write_json(&temp.path().join("manifest.json"), &self.manifest)?;
        write_json(&temp.path().join("source.json"), &self.source)?;
        write_json(&temp.path().join("assets.json"), &self.assets)?;
        write_json(&temp.path().join("provenance.json"), &self.provenance)?;
        write_json(
            &temp.path().join("validation.json"),
            &self.validation_report()?,
        )?;
        for page in &self.pages {
            write_json(
                &temp
                    .path()
                    .join("pages")
                    .join(format!("p{:06}.json", page.physical_index + 1)),
                page,
            )?;
        }
        fs::rename(temp.path(), output).map_err(|e| CoreError::io(output, e))?;
        Ok(())
    }

    pub fn read_from(root: &Path) -> Result<Self> {
        if fs::symlink_metadata(root)
            .map(|m| !m.is_dir() || m.file_type().is_symlink())
            .unwrap_or(true)
        {
            return Err(package_error(format!(
                "package directory does not exist: {}",
                root.display()
            )));
        }
        ensure_directory(root, "pages")?;
        ensure_directory(root, "assets")?;
        let manifest: Manifest = read_json(&root.join("manifest.json"))?;
        validate_version(&manifest.schema, &manifest.schema_version)?;
        let source: Source = read_json(&root.join("source.json"))?;
        let assets: Vec<Asset> = read_json(&root.join("assets.json"))?;
        let provenance: Vec<ProvenanceStep> = read_json(&root.join("provenance.json"))?;
        if assets.len() > MAX_ASSETS {
            return Err(package_error("asset count exceeds the package limit"));
        }
        if provenance.len() > MAX_PROVENANCE_STEPS {
            return Err(package_error("provenance count exceeds the package limit"));
        }
        let validation: ValidationSummary = read_json(&root.join("validation.json"))?;
        validate_version(&validation.schema, &validation.schema_version)?;
        if validation.schema != manifest.schema
            || validation.schema_version != manifest.schema_version
        {
            return Err(package_error(
                "validation schema/version does not match manifest",
            ));
        }
        if !validation.valid {
            return Err(package_error("validation summary is marked invalid"));
        }
        if manifest.page_count as usize > MAX_PAGES {
            return Err(package_error("page count exceeds the package limit"));
        }
        let mut pages = Vec::with_capacity(manifest.page_count as usize);
        for index in 0..manifest.page_count {
            pages.push(read_json(
                &root.join("pages").join(format!("p{:06}.json", index + 1)),
            )?);
        }
        let package = Self {
            manifest,
            source,
            pages,
            assets,
            provenance,
            validation,
        };
        package.validate_files(root)?;
        package.validate()?;
        if package.validation.checked_pages != package.pages.len() as u32
            || package.validation.checked_assets != package.assets.len() as u32
            || !package.validation.errors.is_empty()
        {
            return Err(package_error(
                "validation summary does not match package contents",
            ));
        }
        // M3 OCR records are an additive typed extension. Validate them when
        // present so `package validate` cannot bless a corrupt OCR summary,
        // while packages without the extension remain valid MDP 0.1.
        let ocr_dir = root.join("ocr");
        if let Ok(metadata) = fs::symlink_metadata(&ocr_dir) {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(package_error(
                    "OCR extension directory is not a real directory",
                ));
            }
        }
        let ocr_summary = ocr_dir.join("summary.json");
        if let Ok(metadata) = fs::symlink_metadata(&ocr_summary) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(package_error("OCR summary is not a real file"));
            }
            let ocr = crate::ocr::read_ocr_records(root)
                .map_err(|error| package_error(error.to_string()))?;
            if !ocr.is_complete(package.manifest.page_count) {
                return Err(package_error(
                    "OCR extension summary does not contain a complete run",
                ));
            }
            if ocr
                .pages
                .iter()
                .any(|page| page.page_index >= package.manifest.page_count)
                || ocr
                    .errors
                    .iter()
                    .any(|error| error.page_index >= package.manifest.page_count)
            {
                return Err(package_error("OCR extension references a missing page"));
            }
        }
        Ok(package)
    }

    fn validate_files(&self, root: &Path) -> Result<()> {
        if let Some(path) = &self.source.packaged_path {
            validate_relative_path(path)?;
            verify_file(
                root,
                path,
                self.source.byte_len,
                &self.source.content_sha256,
            )?;
        }
        let mut total = 0u64;
        for asset in &self.assets {
            verify_file(root, &asset.path, asset.byte_len, &asset.content_sha256)?;
            total = total
                .checked_add(asset.byte_len)
                .ok_or_else(|| package_error("asset size overflow"))?;
        }
        if total > MAX_TOTAL_ASSET_BYTES {
            return Err(package_error("assets exceed the total size limit"));
        }
        Ok(())
    }
}

fn ensure_directory(root: &Path, name: &str) -> Result<()> {
    let path = root.join(name);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| package_error(format!("missing package directory: {name}")))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(package_error(format!(
            "package directory is not a real directory: {name}"
        )));
    }
    Ok(())
}

pub fn validate_directory(root: &Path) -> Result<ValidationSummary> {
    let package = DocumentPackage::read_from(root)?;
    let ocr_dir = root.join("ocr");
    if fs::symlink_metadata(&ocr_dir).is_ok()
        && fs::symlink_metadata(ocr_dir.join("summary.json")).is_err()
    {
        return Err(package_error(
            "OCR extension is incomplete: summary.json is missing",
        ));
    }
    package.validation_report()
}

/// Computes the lowercase SHA-256 digest used in package records.
pub fn sha256_digest(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

/// Stable IDs are content-derived and therefore independent of package path
/// or creation time.
pub fn document_id_for_sha256(digest: &str) -> String {
    document_id(digest)
}
pub fn source_id_for_sha256(digest: &str) -> String {
    source_id(digest)
}
pub fn asset_id_for_sha256(page_id: Option<&str>, kind: AssetKind, digest: &str) -> String {
    let seed = format!(
        "{}:{}:{}",
        page_id.unwrap_or("document"),
        kind.stable_name(),
        digest
    );
    format!("asset-{}", sha256_hex(seed.as_bytes()))
}

fn validate_version(schema: &str, version: &str) -> Result<()> {
    if schema != MDP_SCHEMA {
        return Err(package_error(format!("unsupported schema {schema}")));
    }
    let mut parts = version.split('.');
    let major = parts.next().and_then(|n| n.parse::<u32>().ok());
    let minor = parts.next().and_then(|n| n.parse::<u32>().ok());
    if major != Some(0) || minor.is_none() || parts.next().is_some() {
        return Err(package_error(format!(
            "unsupported schema major version {version}"
        )));
    }
    Ok(())
}

fn validate_space(space: &CoordinateSpace) -> Result<()> {
    if space.id.is_empty() {
        return Err(package_error("coordinate space ID must not be empty"));
    }
    for (name, value) in [("width", space.width), ("height", space.height)] {
        if !value.is_finite() || value <= 0.0 {
            return Err(package_error(format!(
                "coordinate space {name} is not positive finite"
            )));
        }
    }
    Ok(())
}

fn validate_transform(
    transform: &AffineTransform,
    master: &CoordinateSpace,
    source: &CoordinateSpace,
) -> Result<()> {
    if transform.from_space != source.id || transform.to_space != master.id {
        return Err(package_error(
            "transform must map source space to master space",
        ));
    }
    let values = [
        transform.a,
        transform.b,
        transform.c,
        transform.d,
        transform.e,
        transform.f,
    ];
    if values.iter().any(|v| !v.is_finite()) {
        return Err(package_error("transform contains a non-finite value"));
    }
    if (transform.a * transform.d - transform.b * transform.c).abs() < f64::EPSILON {
        return Err(package_error("transform matrix is not invertible"));
    }
    Ok(())
}

fn validate_rect(rect: &Rect, master: &CoordinateSpace, source: &CoordinateSpace) -> Result<()> {
    let values = [rect.x, rect.y, rect.width, rect.height];
    if values.iter().any(|v| !v.is_finite()) || rect.width < 0.0 || rect.height < 0.0 {
        return Err(package_error("evidence bounds contain invalid coordinates"));
    }
    let space = if rect.space_id == master.id {
        master
    } else if rect.space_id == source.id {
        source
    } else {
        return Err(package_error(
            "evidence bounds reference an unknown page space",
        ));
    };
    if rect.x < 0.0
        || rect.y < 0.0
        || rect.x + rect.width > space.width
        || rect.y + rect.height > space.height
    {
        return Err(package_error(
            "evidence bounds exceed their coordinate space",
        ));
    }
    Ok(())
}

fn validate_stable_id(id: &str, kind: &str) -> Result<()> {
    let prefix = format!("{kind}-");
    if !id.starts_with(&prefix)
        || id.len() != prefix.len() + 64
        || !id[prefix.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(package_error(format!(
            "{kind} ID is not a stable SHA-256 ID"
        )));
    }
    Ok(())
}

fn validate_digest(digest: &str, label: &str) -> Result<()> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(package_error(format!(
            "{label} digest is not lowercase SHA-256 hex"
        )));
    }
    Ok(())
}

pub fn validate_relative_path(path: &str) -> Result<()> {
    if path.contains('\\') {
        return Err(package_error("package paths must use POSIX separators"));
    }
    let path = Path::new(path);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(package_error("package path must be non-empty and relative"));
    }
    for component in path.components() {
        if matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir | Component::CurDir
        ) {
            return Err(package_error(format!(
                "package path escapes its root: {path:?}"
            )));
        }
    }
    Ok(())
}

fn verify_file(
    root: &Path,
    relative: &str,
    expected_len: u64,
    expected_digest: &str,
) -> Result<()> {
    validate_relative_path(relative)?;
    let path = safe_resource_path(root, relative)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| package_error(format!("missing package resource: {relative}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(package_error(format!(
            "package resource is not a regular file: {relative}"
        )));
    }
    if metadata.len() != expected_len || metadata.len() > MAX_ASSET_BYTES {
        return Err(package_error(format!(
            "resource size mismatch or limit exceeded: {relative}"
        )));
    }
    let file = fs::File::open(&path).map_err(|e| CoreError::io(path.clone(), e))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut read_len = 0u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|e| CoreError::io(path.clone(), e))?;
        if count == 0 {
            break;
        }
        read_len = read_len.saturating_add(count as u64);
        if read_len > expected_len || read_len > MAX_ASSET_BYTES {
            return Err(package_error(format!(
                "resource size mismatch or limit exceeded: {relative}"
            )));
        }
        hasher.update(&buffer[..count]);
    }
    if read_len != expected_len || format_digest(hasher.finalize()) != expected_digest {
        return Err(package_error(format!(
            "resource digest mismatch: {relative}"
        )));
    }
    Ok(())
}

fn safe_resource_path(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative_path(relative)?;
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(part) = component else {
            return Err(package_error(
                "package path contains an unsupported component",
            ));
        };
        current.push(part);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| package_error(format!("missing package resource: {relative}")))?;
        if metadata.file_type().is_symlink() {
            return Err(package_error(format!(
                "package resource uses a symlink: {relative}"
            )));
        }
    }
    Ok(current)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| package_error(e.to_string()))?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, bytes).map_err(|e| CoreError::io(temp.clone(), e))?;
    fs::rename(&temp, path).map_err(|e| CoreError::io(path, e))?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let metadata = fs::symlink_metadata(path).map_err(|e| CoreError::io(path, e))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(package_error(format!(
            "JSON resource is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_JSON_BYTES {
        return Err(package_error(format!(
            "JSON resource exceeds size limit: {}",
            path.display()
        )));
    }
    let bytes = fs::read(path).map_err(|e| CoreError::io(path, e))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| package_error(format!("invalid JSON at {}: {e}", path.display())))
}

fn empty_validation() -> ValidationSummary {
    ValidationSummary {
        schema: MDP_SCHEMA.to_owned(),
        schema_version: MDP_SCHEMA_VERSION.to_owned(),
        valid: false,
        checked_pages: 0,
        checked_assets: 0,
        errors: Vec::new(),
    }
}

fn package_error(message: impl Into<String>) -> CoreError {
    CoreError::InvalidDocumentPackage(message.into())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format_digest(digest)
}

fn format_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn document_id(digest: &str) -> String {
    format!("doc-{digest}")
}
fn source_id(digest: &str) -> String {
    format!("source-{digest}")
}
fn page_id(digest: &str, index: u32) -> String {
    let seed = format!("{digest}:{index}");
    format!("page-{}", sha256_hex(seed.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DocumentPackage {
        let source_digest = sha256_hex(b"source");
        let page = Page {
            page_id: page_id(&source_digest, 0),
            physical_index: 0,
            order: 0,
            rotation_degrees: 0,
            master_space: CoordinateSpace {
                id: "master".into(),
                unit: CoordinateUnit::Pixels,
                width: 100.0,
                height: 200.0,
                origin: Origin::TopLeft,
                pixels_per_inch: Some(CANONICAL_MASTER_DPI),
            },
            source_space: CoordinateSpace {
                id: "pdf".into(),
                unit: CoordinateUnit::PdfPoints,
                width: 100.0,
                height: 200.0,
                origin: Origin::BottomLeft,
                pixels_per_inch: None,
            },
            transforms: vec![AffineTransform {
                from_space: "pdf".into(),
                to_space: "master".into(),
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: -1.0,
                e: 0.0,
                f: 200.0,
            }],
            printed_page_label: None,
            existing_outline_evidence: vec![],
            typography_evidence: vec![],
            region_evidence: vec![],
            asset_ids: vec![],
        };
        DocumentPackage {
            manifest: Manifest {
                schema: MDP_SCHEMA.into(),
                schema_version: MDP_SCHEMA_VERSION.into(),
                document_id: document_id(&source_digest),
                source_id: source_id(&source_digest),
                page_count: 1,
                asset_count: 0,
                tool: ToolInfo {
                    name: "mpdf".into(),
                    version: "0.1".into(),
                },
            },
            source: Source {
                source_id: source_id(&source_digest),
                kind: SourceKind::Pdf,
                content_sha256: source_digest,
                byte_len: 6,
                page_count: 1,
                external_reference: Some("source.pdf".into()),
                packaged_path: None,
            },
            pages: vec![page],
            assets: vec![],
            provenance: vec![],
            validation: empty_validation(),
        }
    }

    #[test]
    fn round_trip_is_deterministic_and_has_no_display_name() {
        let package = sample();
        package.validate().unwrap();
        let first = serde_json::to_vec(&package).unwrap();
        let second = serde_json::to_vec(&package).unwrap();
        assert_eq!(first, second);
        assert!(!String::from_utf8_lossy(&first).contains("M PDF Processor"));
    }

    #[test]
    fn rejects_unknown_major_and_escaping_paths() {
        let mut package = sample();
        package.manifest.schema_version = "1.0".into();
        assert!(package.validate().is_err());
        assert!(validate_relative_path("../outside").is_err());
        assert!(validate_relative_path("/outside").is_err());
        assert!(validate_relative_path("./inside").is_err());
        assert!(validate_relative_path("assets\\inside").is_err());
        assert!(validate_version(MDP_SCHEMA, "0.foo").is_err());
        assert!(validate_version(MDP_SCHEMA, "0").is_err());
        assert!(validate_version(MDP_SCHEMA, "0.1.extra").is_err());
    }

    #[test]
    fn rejects_empty_tool_mime_and_duplicate_page_asset_ids() {
        let mut package = sample();
        package.manifest.tool.name.clear();
        assert!(package.validate().is_err());
        let mut package = sample();
        package.pages[0].asset_ids = vec!["asset-".into(), "asset-".into()];
        assert!(package.validate().is_err());
    }

    #[test]
    fn rejects_non_finite_coordinates_and_singular_matrix() {
        let mut package = sample();
        package.pages[0].master_space.width = f64::NAN;
        assert!(package.validate().is_err());
        let mut package = sample();
        package.pages[0].transforms[0].a = 0.0;
        package.pages[0].transforms[0].d = 0.0;
        assert!(package.validate().is_err());
    }

    #[test]
    fn write_read_round_trip_refuses_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("book.mdp");
        let package = sample();
        package.write_to(&output).unwrap();
        assert_eq!(DocumentPackage::read_from(&output).unwrap().pages.len(), 1);
        assert!(package.write_to(&output).is_err());
    }

    #[test]
    fn schema_fixture_matches_manifest_serialization() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../schemas/mpdf-document-package-0.1.schema.json"
        ))
        .unwrap();
        assert_eq!(schema["properties"]["schema"]["const"], MDP_SCHEMA);
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "schema_version"));
        let manifest = serde_json::to_value(sample().manifest).unwrap();
        for required in schema["required"].as_array().unwrap() {
            assert!(manifest.get(required.as_str().unwrap()).is_some());
        }
        for definition in [
            "source",
            "page",
            "space",
            "transform",
            "asset",
            "provenance",
            "validation",
        ] {
            assert!(
                schema["$defs"].get(definition).is_some(),
                "missing schema definition {definition}"
            );
        }
        assert_eq!(
            schema["$defs"]["page"]["properties"]["rotation_degrees"]["enum"],
            serde_json::json!([0, 90, 180, 270])
        );
        let page = serde_json::to_value(&sample().pages[0]).unwrap();
        for field in [
            "page_id",
            "physical_index",
            "order",
            "rotation_degrees",
            "master_space",
            "source_space",
            "transforms",
            "printed_page_label",
            "existing_outline_evidence",
            "typography_evidence",
            "region_evidence",
            "asset_ids",
        ] {
            assert!(
                page.get(field).is_some(),
                "serialized page field drifted: {field}"
            );
            assert!(
                schema["$defs"]["page"]["properties"].get(field).is_some(),
                "schema page field missing: {field}"
            );
        }
        fn exact_keys(value: &serde_json::Value, definition: &serde_json::Value, name: &str) {
            let actual: std::collections::BTreeSet<_> = value.as_object().unwrap().keys().collect();
            let expected: std::collections::BTreeSet<_> = definition["properties"]
                .as_object()
                .unwrap()
                .keys()
                .collect();
            assert_eq!(actual, expected, "schema field drift in {name}");
        }
        let package = sample();
        let digest = sha256_hex(b"asset");
        let rect = Rect {
            space_id: "master".into(),
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let values = [
            (serde_json::to_value(&package.source).unwrap(), "source"),
            (serde_json::to_value(&package.pages[0]).unwrap(), "page"),
            (
                serde_json::to_value(&package.pages[0].master_space).unwrap(),
                "space",
            ),
            (
                serde_json::to_value(&package.pages[0].transforms[0]).unwrap(),
                "transform",
            ),
            (
                serde_json::to_value(PrintedPageLabel {
                    label: "i".into(),
                    source: PrintedLabelSource::Observed,
                })
                .unwrap(),
                "printed_label",
            ),
            (
                serde_json::to_value(ExistingOutlineEvidence {
                    title: "t".into(),
                    level: 1,
                    target_page_id: None,
                    source: "pdf".into(),
                })
                .unwrap(),
                "outline",
            ),
            (
                serde_json::to_value(TypographyEvidence {
                    role: "body".into(),
                    bounds: rect.clone(),
                    font_size_points: None,
                })
                .unwrap(),
                "typography",
            ),
            (
                serde_json::to_value(RegionEvidence {
                    kind: "text".into(),
                    bounds: rect,
                })
                .unwrap(),
                "region",
            ),
            (
                serde_json::to_value(Asset {
                    asset_id: asset_id_for_sha256(None, AssetKind::Original, &digest),
                    kind: AssetKind::Original,
                    path: "assets/a".into(),
                    mime_type: "application/octet-stream".into(),
                    byte_len: 5,
                    content_sha256: digest,
                    page_id: None,
                })
                .unwrap(),
                "asset",
            ),
            (
                serde_json::to_value(ProvenanceStep {
                    step_id: "step".into(),
                    operation: "op".into(),
                    inputs: vec![package.source.source_id.clone()],
                    outputs: vec![package.pages[0].page_id.clone()],
                    parameters: std::collections::BTreeMap::new(),
                    software: "mpdf".into(),
                    software_version: "0.1".into(),
                    execution: ExecutionKind::Local,
                })
                .unwrap(),
                "provenance",
            ),
            (
                serde_json::to_value(package.validation).unwrap(),
                "validation",
            ),
        ];
        for (value, name) in values {
            exact_keys(&value, &schema["$defs"][name], name);
        }
        assert_eq!(
            schema["$defs"]["asset"]["properties"]["kind"]["enum"],
            serde_json::json!(["original", "master", "thumbnail", "bilevel"])
        );
        assert_eq!(
            schema["$defs"]["space"]["properties"]["unit"]["enum"],
            serde_json::json!(["pixels", "pdf_points"])
        );
        assert_eq!(
            schema["$defs"]["printed_label"]["properties"]["source"]["enum"],
            serde_json::json!(["observed", "inferred"])
        );
    }

    #[test]
    fn rejects_duplicate_page_and_asset_ids_and_resource_limits() {
        let mut package = sample();
        package.pages.push(package.pages[0].clone());
        package.manifest.page_count = 2;
        package.source.page_count = 2;
        assert!(package.validate().is_err());

        let mut package = sample();
        let digest = sha256_hex(b"asset");
        package.assets.push(Asset {
            asset_id: "asset-duplicate".into(),
            kind: AssetKind::Original,
            path: "assets/a.bin".into(),
            mime_type: "application/octet-stream".into(),
            byte_len: MAX_ASSET_BYTES + 1,
            content_sha256: digest,
            page_id: None,
        });
        package.manifest.asset_count = 1;
        assert!(package.validate().is_err());
    }

    #[test]
    fn ids_are_content_and_scope_stable() {
        let digest = sha256_hex(b"source");
        assert_eq!(
            asset_id_for_sha256(Some("page-a"), AssetKind::Original, &digest),
            asset_id_for_sha256(Some("page-a"), AssetKind::Original, &digest)
        );
        assert_ne!(
            page_id(&digest, 0),
            page_id(&sha256_hex(b"changed source"), 0)
        );
        assert_ne!(page_id(&digest, 0), page_id(&digest, 1));
        assert_eq!(
            asset_id_for_sha256(Some("page-a"), AssetKind::Original, &digest),
            asset_id_for_sha256(Some("page-a"), AssetKind::Original, &digest)
        );
        assert_ne!(
            asset_id_for_sha256(Some("page-a"), AssetKind::Original, &digest),
            asset_id_for_sha256(Some("page-a"), AssetKind::Master, &digest)
        );
    }

    #[test]
    fn rejects_manifest_page_count_before_allocating_page_vector() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("huge.mdp");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        let digest = sha256_hex(b"source");
        let manifest = Manifest {
            schema: MDP_SCHEMA.into(),
            schema_version: MDP_SCHEMA_VERSION.into(),
            document_id: document_id(&digest),
            source_id: source_id(&digest),
            page_count: u32::MAX,
            asset_count: 0,
            tool: ToolInfo {
                name: "mpdf".into(),
                version: "0.1".into(),
            },
        };
        let source = Source {
            source_id: source_id(&digest),
            kind: SourceKind::Pdf,
            content_sha256: digest,
            byte_len: 6,
            page_count: u32::MAX,
            external_reference: None,
            packaged_path: None,
        };
        write_json(&root.join("manifest.json"), &manifest).unwrap();
        write_json(&root.join("source.json"), &source).unwrap();
        write_json(&root.join("assets.json"), &Vec::<Asset>::new()).unwrap();
        write_json(&root.join("provenance.json"), &Vec::<ProvenanceStep>::new()).unwrap();
        write_json(
            &root.join("validation.json"),
            &ValidationSummary {
                schema: MDP_SCHEMA.into(),
                schema_version: MDP_SCHEMA_VERSION.into(),
                valid: true,
                checked_pages: 0,
                checked_assets: 0,
                errors: vec![],
            },
        )
        .unwrap();
        assert!(DocumentPackage::read_from(&root).is_err());
    }

    fn package_with_asset() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("book.mdp");
        sample().write_to(&output).unwrap();
        let bytes = b"asset bytes";
        std::fs::write(output.join("assets/a.bin"), bytes).unwrap();
        let digest = sha256_hex(bytes);
        let asset = Asset {
            asset_id: asset_id_for_sha256(None, AssetKind::Original, &digest),
            kind: AssetKind::Original,
            path: "assets/a.bin".into(),
            mime_type: "application/octet-stream".into(),
            byte_len: bytes.len() as u64,
            content_sha256: digest,
            page_id: None,
        };
        let asset_json = serde_json::to_vec_pretty(&vec![asset]).unwrap();
        std::fs::write(output.join("assets.json"), asset_json).unwrap();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.join("manifest.json")).unwrap()).unwrap();
        manifest["asset_count"] = serde_json::json!(1);
        std::fs::write(
            output.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let mut validation: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.join("validation.json")).unwrap())
                .unwrap();
        validation["checked_assets"] = serde_json::json!(1);
        std::fs::write(
            output.join("validation.json"),
            serde_json::to_vec_pretty(&validation).unwrap(),
        )
        .unwrap();
        (dir, output)
    }

    #[test]
    fn validates_asset_digest_and_missing_resource() {
        let (dir, output) = package_with_asset();
        assert!(DocumentPackage::read_from(&output).is_ok());
        std::fs::write(output.join("assets/a.bin"), b"changed").unwrap();
        assert!(DocumentPackage::read_from(&output).is_err());
        drop(dir);

        let (_dir, output) = package_with_asset();
        std::fs::remove_file(output.join("assets/a.bin")).unwrap();
        assert!(DocumentPackage::read_from(&output).is_err());
        drop(_dir);
    }

    #[test]
    fn rejects_asset_path_escape_and_duplicate_id_on_disk() {
        let (_dir, output) = package_with_asset();
        let mut assets: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.join("assets.json")).unwrap()).unwrap();
        assets[0]["path"] = serde_json::json!("../outside");
        std::fs::write(
            output.join("assets.json"),
            serde_json::to_vec_pretty(&assets).unwrap(),
        )
        .unwrap();
        assert!(DocumentPackage::read_from(&output).is_err());
        drop(_dir);

        let (_dir, output) = package_with_asset();
        let mut assets: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.join("assets.json")).unwrap()).unwrap();
        let duplicate = assets[0].clone();
        assets.as_array_mut().unwrap().push(duplicate);
        std::fs::write(
            output.join("assets.json"),
            serde_json::to_vec_pretty(&assets).unwrap(),
        )
        .unwrap();
        assert!(DocumentPackage::read_from(&output).is_err());
    }
}
