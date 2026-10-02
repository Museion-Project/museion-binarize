//! Explicit, test-only product integration for the frozen Surya stack.
//! CLI and Desktop call this factory; neither reconstructs the OCR pipeline.
use crate::{
    geometry_broker::{GeometryBroker, SuryaGeometry, SURYA_ID, SURYA_VERSION},
    Secret,
};
use image::ImageFormat;
use mpdf_core::{
    error::{CoreError, Result as CoreResult},
    jobs::ExecutionLocation,
    ocr::{OcrError, PageOcrProvider},
    ocr_provider::{
        alignment::DecisionSummary,
        geometry_transcription::{
            self as gt, GeometryBoundTranscription, GeometryPage, GeometryProvider,
            GeometryRequest, GeometryTranscriptionError as Error,
            GeometryTranscriptionPageProvider, TranscriptionProvider, TranscriptionRequest,
        },
        BillingModel, CloudFallback, CoordinateGranularity, GeometrySource, JobPreparation,
        JobSettlement, JobTicket, MetadataSupport, OcrProvider, OcrProviderCapabilities,
        OcrProviderMode, PageOcrOutcome, PageOcrRequest, ProviderConfigError, ProviderRole,
        ReadingOrderCapability, TextGeometryMapping,
    },
    orchestrator::CloudProviderFactory,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Cursor, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const FACTORY_VERSION: &str = "frozen-surya-broker-test/1";
const TEST_REASON: &str =
    "test-only real broker integration; not released or commercially billed through Credits";
const RUNTIME_FILES: &[&str] = &[
    "scripts/ocr/broker/surya_geometry.py",
    "scripts/ocr/geometry/finalist_adapters.py",
    "docs/evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json",
    "schemas/museion-geometry-evidence-1.schema.json",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerTestConfig {
    pub schema: String,
    pub test_only: bool,
    /// A new id means an explicitly new run, not an automatic retry.
    pub run_id: String,
    pub document_sha256: String,
    pub language_profile: String,
    pub endpoint: String,
    pub client_token_file: PathBuf,
    pub python: PathBuf,
    pub runtime_root: PathBuf,
    pub model_dir: PathBuf,
    pub evidence_dir: PathBuf,
    /// Maximum unique page requests for this document. Not a Credits balance.
    pub max_requests: u32,
}

fn invalid(s: impl Into<String>) -> Error {
    Error::InvalidConfiguration(s.into())
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn file_sha(path: &Path) -> Result<String, Error> {
    let bytes = fs::read(path).map_err(|_| invalid("Cannot read pinned runtime artifact"))?;
    Ok(sha(&bytes))
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| invalid("Evidence path exists or is not writable"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| invalid("Cannot persist evidence"))
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<(), Error> {
    write_new(
        path,
        &serde_json::to_vec_pretty(value).map_err(|_| invalid("Cannot encode evidence"))?,
    )
}
pub(crate) fn token(path: &Path) -> Result<Secret, Error> {
    // Never read a model key here. The broker owns that credential.
    let meta = fs::symlink_metadata(path).map_err(|_| invalid("Client token unavailable"))?;
    if !meta.is_file() || meta.len() > 4096 {
        return Err(invalid("Invalid client token file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(invalid("Client token must be private (0600)"));
        }
    }
    let text = fs::read_to_string(path).map_err(|_| invalid("Cannot read client token"))?;
    if text.trim().len() < 32 || text.trim().chars().any(char::is_whitespace) {
        return Err(invalid("Invalid client token"));
    }
    Ok(Secret::new(text.trim().to_owned()))
}

impl BrokerTestConfig {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let bytes = fs::read(path).map_err(|_| invalid("Cannot read broker-test config"))?;
        if bytes.len() > 65536 {
            return Err(invalid("Broker-test config too large"));
        }
        let config: Self =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid broker-test config"))?;
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<(), Error> {
        if self.schema != "mpdf-broker-test-config/1"
            || !self.test_only
            || self.run_id.is_empty()
            || self.run_id.len() > 80
            || !self
                .run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || self.document_sha256.len() != 64
            || !self.document_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || self.language_profile.trim().is_empty()
            || self.max_requests == 0
        {
            return Err(invalid("Test-only declaration, run identity, document digest, language and request limit are required"));
        }
        for p in [
            &self.python,
            &self.runtime_root,
            &self.model_dir,
            &self.evidence_dir,
            &self.client_token_file,
        ] {
            if !p.is_absolute() {
                return Err(invalid("Broker-test paths must be absolute"));
            }
        }
        // Only validates the origin; no credential access or request during dry-run.
        GeometryBroker::local(&self.endpoint, Secret::new("configuration-validation-only"))?;
        Ok(())
    }
    fn runtime_hashes(&self) -> Result<BTreeMap<String, String>, Error> {
        let mut hashes = BTreeMap::new();
        for name in RUNTIME_FILES {
            hashes.insert((*name).into(), file_sha(&self.runtime_root.join(name))?);
        }
        let frozen: Value = serde_json::from_str(include_str!(
            "../../../docs/evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json"
        ))
        .map_err(|_| invalid("Invalid compiled provider freeze"))?;
        if hashes["scripts/ocr/geometry/finalist_adapters.py"]
            != frozen["adapter_sha256"].as_str().unwrap_or("")
            || hashes["docs/evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json"]
                != sha(include_bytes!(
                    "../../../docs/evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json"
                ))
            || hashes["scripts/ocr/broker/surya_geometry.py"]
                != sha(include_bytes!(
                    "../../../scripts/ocr/broker/surya_geometry.py"
                ))
            || hashes["schemas/museion-geometry-evidence-1.schema.json"]
                != sha(include_bytes!(
                    "../../../schemas/museion-geometry-evidence-1.schema.json"
                ))
        {
            return Err(invalid(
                "Frozen geometry runtime differs from compiled stack",
            ));
        }
        for (name, expected) in frozen["model_sha256"]
            .as_object()
            .ok_or_else(|| invalid("Invalid model freeze"))?
        {
            let actual = file_sha(&self.model_dir.join(name))?;
            if Some(actual.as_str()) != expected.as_str() {
                return Err(invalid("Frozen detector hash mismatch"));
            }
            hashes.insert(format!("model/{name}"), actual);
        }
        Ok(hashes)
    }
}

pub struct BrokerTestFactory {
    config: BrokerTestConfig,
    digest: String,
}
impl BrokerTestFactory {
    /// Policy parsing only: does not read the input PDF, token or model.
    pub fn from_config_file(path: &Path) -> Result<Self, Error> {
        let config = BrokerTestConfig::load(path)?;
        let mut identity = serde_json::to_vec(&config).map_err(|_| invalid("Invalid config"))?;
        for implementation in [
            include_bytes!("broker_test.rs").as_slice(),
            include_bytes!("geometry_broker.rs").as_slice(),
            include_bytes!("../../../scripts/ocr/broker/server.py").as_slice(),
            include_bytes!("../../mpdf-core/src/ocr_provider/geometry_transcription.rs").as_slice(),
            include_bytes!("../../../schemas/mpdf-ocr-provider-0.2.schema.json").as_slice(),
        ] {
            identity.extend_from_slice(sha(implementation).as_bytes());
        }
        let digest = sha(&identity);
        Ok(Self { config, digest })
    }
}
impl CloudProviderFactory for BrokerTestFactory {
    fn mode(&self) -> OcrProviderMode {
        OcrProviderMode::BrokerTest
    }
    fn provider_label(&self) -> &str {
        "surya-line-geometry+google-gemini"
    }
    fn config_digest(&self) -> String {
        self.digest.clone()
    }
    fn fingerprint_contribution(&self) -> String {
        format!(
            "{FACTORY_VERSION}|{SURYA_VERSION}|{}|{}",
            gt::GEMINI_TRANSCRIPTION_MODEL,
            self.digest
        )
    }
    fn max_credits(&self) -> u64 {
        0
    }
    fn fallback(&self) -> CloudFallback {
        CloudFallback::Fail
    }
    fn build(&self, _local: Box<dyn PageOcrProvider>) -> CoreResult<Box<dyn OcrProvider>> {
        // The historical local OCR argument is intentionally not the detector.
        // This factory constructs the frozen geometry runtime itself.
        TestProvider::new(self.config.clone(), self.digest.clone())
            .map(|p| Box::new(p) as Box<dyn OcrProvider>)
            .map_err(|e| CoreError::InvalidParameter(e.to_string()))
    }
}

#[derive(Clone)]
struct PageEvidence {
    directory: PathBuf,
    raw: String,
    geometry: GeometryPage,
}
type SharedPage = Arc<Mutex<Option<PageEvidence>>>;
struct RuntimeSurya {
    config: BrokerTestConfig,
    hashes: BTreeMap<String, String>,
    page: SharedPage,
}
impl GeometryProvider for RuntimeSurya {
    fn provider_id(&self) -> &str {
        SURYA_ID
    }
    fn provider_version(&self) -> &str {
        SURYA_VERSION
    }
    fn fingerprint_contribution(&self) -> String {
        format!(
            "{SURYA_VERSION}|{}",
            sha(&serde_json::to_vec(&self.hashes).expect("hash map serializes"))
        )
    }
    fn detect_geometry(&mut self, r: &GeometryRequest<'_>) -> Result<GeometryPage, Error> {
        if self.config.runtime_hashes()? != self.hashes {
            return Err(invalid("Runtime changed during run"));
        }
        let mut png = Cursor::new(Vec::new());
        r.page_image
            .write_to(&mut png, ImageFormat::Png)
            .map_err(|_| invalid("Cannot encode product raster"))?;
        if sha(png.get_ref()) != r.page_image_sha256 {
            return Err(invalid("Product raster digest mismatch"));
        }
        let directory = self
            .config
            .evidence_dir
            .join(format!("page-{:04}-{}", r.page_index, r.page_image_sha256));
        fs::create_dir_all(&directory)
            .map_err(|_| invalid("Cannot create page evidence directory"))?;
        let raster = directory.join("input.png");
        if raster.exists() {
            if file_sha(&raster)? != r.page_image_sha256 {
                return Err(invalid("Saved product raster was changed"));
            }
        } else {
            write_new(&raster, png.get_ref())?;
        }
        let output = directory.join("geometry.json");
        if !output.exists() {
            let log = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("surya.log"))
                .map_err(|_| {
                    invalid("Previous geometry attempt incomplete; inspect its evidence")
                })?;
            let mut child = Command::new(&self.config.python)
                .arg(
                    self.config
                        .runtime_root
                        .join("scripts/ocr/broker/surya_geometry.py"),
                )
                .arg(&raster)
                .arg(&output)
                .arg("--page-index")
                .arg(r.page_index.to_string())
                .arg("--model-dir")
                .arg(&self.config.model_dir)
                .env(
                    "PYTHONPATH",
                    self.config.runtime_root.join(".runtime/surya-compat"),
                )
                .env_remove("GEMINI_API_KEY")
                .env_remove("GOOGLE_API_KEY")
                .stdin(Stdio::null())
                .stdout(
                    log.try_clone()
                        .map_err(|_| invalid("Cannot create geometry log"))?,
                )
                .stderr(log)
                .spawn()
                .map_err(|_| invalid("Cannot start frozen Surya runtime"))?;
            let started = Instant::now();
            loop {
                if let Some(status) = child
                    .try_wait()
                    .map_err(|_| invalid("Cannot wait for Surya"))?
                {
                    if !status.success() {
                        return Err(invalid("Frozen Surya runtime failed; see page surya.log"));
                    }
                    break;
                }
                if started.elapsed() > Duration::from_secs(300) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(invalid("Frozen Surya runtime exceeded deadline"));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            write_json(
                &directory.join("geometry-execution.json"),
                &json!({
                    "runtime":SURYA_VERSION,"runtime_hashes":self.hashes,"elapsed_ms":started.elapsed().as_millis(),
                    "page_index":r.page_index,"raster_sha256":r.page_image_sha256,"exit_code":0,
                    "geometry_artifact_sha256":file_sha(&output)?,"cached_detector_output":false
                }),
            )?;
        }
        let execution: Value = serde_json::from_slice(
            &fs::read(directory.join("geometry-execution.json"))
                .map_err(|_| invalid("Geometry execution evidence missing"))?,
        )
        .map_err(|_| invalid("Invalid geometry execution evidence"))?;
        if execution["geometry_artifact_sha256"] != file_sha(&output)? {
            return Err(invalid("Geometry artifact changed"));
        }
        let mut detected = SuryaGeometry::load(&output)?;
        let geometry = detected.detect_geometry(r)?;
        // Each fragment belongs to exactly one logical line. Split parents
        // remain provenance only, never additional transcription units.
        let e = &detected.evidence;
        let fragments: BTreeSet<_> = e["fragments"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["fragment_id"].as_str())
            .collect();
        let mut used = BTreeSet::new();
        for line in e["logical_lines"].as_array().unwrap() {
            for id in line["fragment_ids"]
                .as_array()
                .ok_or_else(|| invalid("Missing logical fragment binding"))?
            {
                let id = id
                    .as_str()
                    .ok_or_else(|| invalid("Invalid fragment identity"))?;
                if !fragments.contains(id) || !used.insert(id) {
                    return Err(invalid("Logical fragments are missing or duplicated"));
                }
            }
        }
        if used != fragments {
            return Err(invalid("Unbound geometry fragments"));
        }
        for event in e["derivations"].as_array().unwrap() {
            if let Some(parent) = event["source_fragment_id"].as_str() {
                if used.contains(parent) {
                    return Err(invalid("Split parent appears as a transcription unit"));
                }
            }
        }
        *self
            .page
            .lock()
            .map_err(|_| invalid("Page evidence lock failed"))? = Some(PageEvidence {
            directory,
            raw: detected.raw_evidence,
            geometry: geometry.clone(),
        });
        Ok(geometry)
    }
}

struct RecordingBroker {
    broker: GeometryBroker,
    page: SharedPage,
}
impl TranscriptionProvider for RecordingBroker {
    fn provider_id(&self) -> &str {
        self.broker.provider_id()
    }
    fn model(&self) -> &str {
        self.broker.model()
    }
    fn model_version(&self) -> &str {
        self.broker.model_version()
    }
    fn fingerprint_contribution(&self) -> String {
        self.broker.fingerprint_contribution()
    }
    fn transcribe(
        &mut self,
        r: &TranscriptionRequest<'_>,
    ) -> Result<GeometryBoundTranscription, Error> {
        let evidence = self
            .page
            .lock()
            .map_err(|_| invalid("Page evidence lock failed"))?
            .clone()
            .ok_or_else(|| invalid("Geometry must run before transcription"))?;
        let request = json!({"idempotency_key":r.idempotency_key,"geometry_json":serde_json::to_string(r.geometry).map_err(|_| invalid("Invalid geometry"))?,
            "geometry_sha256":r.geometry.digest()?,"raster_sha256":r.geometry.image_sha256,"model":self.model(),"language_profile":r.language_profile});
        let request_file = evidence.directory.join("transcription-request.json");
        if request_file.exists() {
            let prior: Value = serde_json::from_slice(
                &fs::read(&request_file).map_err(|_| invalid("Cannot read prior request"))?,
            )
            .map_err(|_| invalid("Invalid prior request"))?;
            if prior != request {
                return Err(invalid("Conflicting prior transcription request"));
            }
        } else {
            write_json(&request_file, &request)?;
        }
        let result = self.broker.transcribe(r)?;
        gt::validate_transcription(r.geometry, &result)?;
        let receipt = self.broker.receipt(r.idempotency_key)?;
        if receipt["idempotency_key"] != r.idempotency_key
            || receipt["geometry_sha256"] != r.geometry.digest()?
            || receipt["state"] != "done"
            || receipt["model_version"] != self.model_version()
            || receipt["request"]["upload_sha256"] != r.geometry.image_sha256
        {
            return Err(invalid("Broker receipt binding mismatch"));
        }
        // Preserve the first successful result. A replay must be identical.
        let response_file = evidence.directory.join("transcription.json");
        if response_file.exists() {
            let previous: GeometryBoundTranscription = serde_json::from_slice(
                &fs::read(&response_file).map_err(|_| invalid("Cannot read prior result"))?,
            )
            .map_err(|_| invalid("Invalid prior result"))?;
            if previous != result {
                return Err(invalid("Broker replay changed transcription"));
            }
        } else {
            write_json(&response_file, &result)?;
        }
        if !evidence.directory.join("broker-receipt.json").exists() {
            write_json(&evidence.directory.join("broker-receipt.json"), &receipt)?;
        }
        Ok(result)
    }
}

struct TestProvider {
    config: BrokerTestConfig,
    digest: String,
    pipeline: GeometryTranscriptionPageProvider,
    page: SharedPage,
    prepared: Option<JobPreparation>,
}
impl TestProvider {
    fn new(config: BrokerTestConfig, digest: String) -> Result<Self, Error> {
        let hashes = config.runtime_hashes()?;
        let broker = GeometryBroker::local(&config.endpoint, token(&config.client_token_file)?)?;
        let page: SharedPage = Arc::new(Mutex::new(None));
        let pipeline = GeometryTranscriptionPageProvider::new(
            Box::new(RuntimeSurya {
                config: config.clone(),
                hashes,
                page: page.clone(),
            }),
            Box::new(RecordingBroker {
                broker,
                page: page.clone(),
            }),
            &config.language_profile,
            Duration::from_secs(240),
            format!(
                "{FACTORY_VERSION}|{}|{}|{digest}",
                config.run_id, config.document_sha256
            ),
        )?;
        Ok(Self {
            config,
            digest,
            pipeline,
            page,
            prepared: None,
        })
    }
}
impl OcrProvider for TestProvider {
    fn capabilities(&self) -> OcrProviderCapabilities {
        capabilities()
    }
    fn validate_configuration(&self) -> Result<(), ProviderConfigError> {
        self.config
            .validate()
            .map_err(|e| ProviderConfigError::Invalid(e.to_string()))
    }
    fn prepare_job(&mut self, job: &JobPreparation) -> Result<JobTicket, OcrError> {
        if job.document_sha256 != self.config.document_sha256
            || job.language_profile != self.config.language_profile
            || job.page_indices.len() > self.config.max_requests as usize
            || job.max_credits != 0
            || job.page_indices.iter().collect::<BTreeSet<_>>().len() != job.page_indices.len()
        {
            return Err(OcrError::ProviderUnavailable(
                "Broker-test document, language or request budget mismatch".into(),
            ));
        }
        fs::create_dir_all(&self.config.evidence_dir)
            .map_err(|_| OcrError::InvalidEvidence("Cannot create run evidence".into()))?;
        let run = json!({"factory":FACTORY_VERSION,"config":self.config,"config_sha256":self.digest,
            "pipeline_fingerprint":self.pipeline.fingerprint_contribution(),"capabilities":self.capabilities(),
            "page_indices":job.page_indices,"max_requests":self.config.max_requests,"max_output_tokens_per_request":16384,
            "monetary_cost":null,"cost_status":"provider invoice not reported; see actual token receipts; not Credits"});
        let path = self.config.evidence_dir.join("run.json");
        if path.exists() {
            let previous: Value = serde_json::from_slice(
                &fs::read(&path)
                    .map_err(|_| OcrError::InvalidEvidence("Cannot read run evidence".into()))?,
            )
            .map_err(|_| OcrError::InvalidEvidence("Invalid run evidence".into()))?;
            if previous != run {
                return Err(OcrError::InvalidEvidence(
                    "Run evidence/config conflict; use the original configuration".into(),
                ));
            }
        } else {
            write_json(&path, &run).map_err(|e| OcrError::InvalidEvidence(e.to_string()))?;
        }
        self.prepared = Some(job.clone());
        Ok(JobTicket {
            job_id: job.job_id.clone(),
            reservation_id: None,
            reserved_credits: 0,
            estimated_credits: 0,
        })
    }
    fn recognize_page(&mut self, r: &PageOcrRequest<'_>) -> Result<PageOcrOutcome, OcrError> {
        let job = self
            .prepared
            .as_ref()
            .ok_or_else(|| OcrError::ProviderUnavailable("Broker-test job not prepared".into()))?;
        if r.document_sha256 != job.document_sha256
            || r.language_profile != job.language_profile
            || r.dpi != job.dpi
            || !job.page_indices.contains(&r.page_index)
        {
            return Err(OcrError::InvalidEvidence(
                "Page is outside authorized broker-test job".into(),
            ));
        }
        let mut page = self
            .pipeline
            .recognize(r.page_index, r.page_image, r.page_image_sha256)?;
        let saved = self
            .page
            .lock()
            .map_err(|_| OcrError::InvalidEvidence("Page evidence lock failed".into()))?
            .clone()
            .ok_or_else(|| OcrError::InvalidEvidence("Page evidence missing".into()))?;
        let transcription: GeometryBoundTranscription = serde_json::from_slice(
            &fs::read(saved.directory.join("transcription.json"))
                .map_err(|_| OcrError::InvalidEvidence("Transcription evidence missing".into()))?,
        )
        .map_err(|_| OcrError::InvalidEvidence("Invalid transcription evidence".into()))?;
        page.provider_raw_artifact = Some(saved.raw);
        let parameters = &mut page
            .provider_provenance
            .as_mut()
            .expect("strict compositor supplies provenance")
            .parameters;
        parameters.insert("provider_mode".into(), "broker-test".into());
        parameters.insert(
            "usage_input_tokens".into(),
            transcription.usage.input_tokens.to_string(),
        );
        parameters.insert(
            "usage_output_tokens".into(),
            transcription.usage.output_tokens.to_string(),
        );
        parameters.insert(
            "usage_requests".into(),
            transcription.usage.requests.to_string(),
        );
        parameters.insert("usage_credits_charged".into(), "0".into());
        parameters.insert("billing".into(), "user_managed".into());
        parameters.insert(
            "monetary_cost_status".into(),
            "not_reported_by_provider".into(),
        );
        parameters.insert(
            "geometry_line_ids_sha256".into(),
            sha(&serde_json::to_vec(
                &saved
                    .geometry
                    .lines
                    .iter()
                    .map(|l| &l.line_id)
                    .collect::<Vec<_>>(),
            )
            .unwrap()),
        );
        parameters.insert("factory_config_sha256".into(), self.digest.clone());
        mpdf_core::ocr::validate_ocr_page(&page)?;
        let path = saved.directory.join("composed-page.json");
        if !path.exists() {
            write_json(&path, &page).map_err(|e| OcrError::InvalidEvidence(e.to_string()))?;
        }
        Ok(PageOcrOutcome {
            page,
            provider_mode: OcrProviderMode::BrokerTest,
            provider_name: SURYA_ID.into(),
            model: gt::GEMINI_TRANSCRIPTION_MODEL.into(),
            model_version: gt::GEMINI_TRANSCRIPTION_MODEL.into(),
            raw_transcription_digest: Some(sha(&serde_json::to_vec(&transcription).unwrap())),
            geometry_source: GeometrySource::DeterministicGeometryBoundText,
            alignment_version: gt::COMPOSITION_CONTRACT,
            prompt_config_digest: Some(self.digest.clone()),
            usage: transcription.usage,
            warnings: vec![TEST_REASON.into()],
            fallback_reason: None,
            execution_location: ExecutionLocation::BrokeredCloud,
            decisions: DecisionSummary::default(),
        })
    }
    fn cancel_job(&mut self, _ticket: &JobTicket) -> Result<(), OcrError> {
        self.prepared = None;
        Ok(())
    }
    fn finalize_job(&mut self, _ticket: &JobTicket) -> Result<JobSettlement, OcrError> {
        self.prepared = None;
        // No Credits reservation exists, so do not claim one was settled.
        Ok(JobSettlement::default())
    }
    fn fingerprint_contribution(&self) -> String {
        format!("{FACTORY_VERSION}|{}", self.digest)
    }
}

pub fn capabilities() -> OcrProviderCapabilities {
    OcrProviderCapabilities {
        mode: OcrProviderMode::BrokerTest,
        provider_name: SURYA_ID.into(),
        model: gt::GEMINI_TRANSCRIPTION_MODEL.into(),
        model_version: gt::GEMINI_TRANSCRIPTION_MODEL.into(),
        role: ProviderRole::CompleteOcr,
        coordinate_granularity: CoordinateGranularity::Line,
        reading_order: ReadingOrderCapability::Stable,
        text_geometry_mapping: TextGeometryMapping::Direct,
        confidence_metadata: MetadataSupport::Line,
        language_metadata: MetadataSupport::Line,
        billing: BillingModel::UserManaged,
        requires_explicit_consent: true,
        requires_cost_limit: true,
        uses_network: true,
        requires_credential: false,
        supports_structured_bbox: true,
        structured_bbox_default_enabled: false,
        production_ready: false,
        not_production_ready_reason: Some(TEST_REASON.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_mode_is_complete_but_cannot_claim_production_or_credits() {
        let c = capabilities();
        assert!(c.validate_complete_ocr().is_ok());
        assert!(!OcrProviderMode::PRODUCT_MODES.contains(&c.mode));
        let mut changed = c.clone();
        changed.production_ready = true;
        assert!(changed.validate_complete_ocr().is_err());
        changed = c;
        changed.billing = BillingModel::BrokeredCredits;
        assert!(changed.validate_complete_ocr().is_err());
    }
}
