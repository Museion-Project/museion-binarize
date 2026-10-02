//! Shared CLI/Desktop operational factory for frozen Surya + checked /2.
//! The retained broker-test product mode is user-managed and unreleased; this
//! factory drives the real orchestrator, never the historical /1 provider.
use base64::{engine::general_purpose::STANDARD, Engine};
use image::ImageFormat;
use mpdf_core::{
    derived::DerivedDocument,
    document_package::DocumentPackage,
    error::{CoreError, Result},
    jobs::ExecutionLocation,
    ocr::{self, OcrError, OcrRun, PageOcrProvider},
    ocr_provider::{
        alignment::DecisionSummary,
        geometry_transcription::GEMINI_TRANSCRIPTION_MODEL as MODEL,
        spatial_transcription::{CheckedSupportPage, ExpectedSupportBindings, CONTRACT},
        CloudFallback, GeometrySource, JobPreparation, JobSettlement, JobTicket, OcrProvider,
        OcrProviderCapabilities, OcrProviderMode, PageOcrOutcome, PageOcrRequest,
        ProviderConfigError, ProviderUsage,
    },
    orchestrator::CloudProviderFactory,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const VERSION: &str = "frozen-surya-spatial-factory/1";
const PROTOCOL: &str = "mpdf-spatial-factory-broker/1";
const MAX_REPLY: u64 = 256 * 1024 * 1024;
macro_rules! pinned {
    ($path:literal) => {
        (
            $path,
            include_bytes!(concat!("../../../", $path)).as_slice(),
        )
    };
}
fn runtime_sources() -> Vec<(&'static str, &'static [u8])> {
    vec![
        pinned!("scripts/ocr/broker/spatial_factory_runtime.py"),
        pinned!("scripts/ocr/broker/spatial_factory_broker.py"),
        pinned!("scripts/ocr/broker/spatial_verification.py"),
        pinned!("scripts/ocr/broker/spatial_contract.py"),
        pinned!("scripts/ocr/broker/spatial_batch_strategy.py"),
        pinned!("scripts/ocr/broker/spatial_batch_audit.py"),
        pinned!("scripts/ocr/broker/toc_leader_projection.py"),
        pinned!("scripts/ocr/broker/server.py"),
        pinned!("scripts/ocr/geometry/finalist_adapters.py"),
        pinned!("scripts/ocr/geometry/shared_geometry_adapter.py"),
        pinned!("scripts/ocr/geometry/frozen_local_proposal_adapter.py"),
        pinned!("scripts/ocr/geometry/spatial_supports.py"),
        pinned!("scripts/ocr/geometry/local_geometry_redetect.py"),
        pinned!("scripts/ocr/geometry/integrate_local_geometry.py"),
        pinned!("scripts/ocr/geometry/local_component_recovery.py"),
        pinned!("scripts/ocr/broker/transcription_fidelity.py"),
        pinned!("scripts/ocr/geometry/run_dev_geometry_bakeoff.py"),
        pinned!("scripts/ocr/geometry/run_geometry_bakeoff.py"),
        pinned!("scripts/ocr/geometry/run_detector_adapter_challenge.py"),
        pinned!("scripts/ocr/geometry/score_finalists.py"),
        pinned!("docs/evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json"),
    ]
}
fn invalid(s: impl Into<String>) -> CoreError {
    CoreError::InvalidDocument(s.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn bytes(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|_| invalid(format!("Missing spatial evidence: {}", path.display())))
}
fn text(path: &Path) -> Result<String> {
    String::from_utf8(bytes(path)?).map_err(|_| invalid("Invalid spatial UTF-8"))
}
fn read(path: &Path) -> Result<Value> {
    serde_json::from_slice(&bytes(path)?).map_err(|_| invalid("Invalid spatial evidence JSON"))
}
fn field(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("Missing spatial field {key}")))
}
fn file_hash(path: &Path) -> Result<String> {
    Ok(hash(&bytes(path)?))
}
fn save(path: &Path, raw: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path).map_err(|_| {
        invalid(format!(
            "Spatial evidence exists or cannot be written: {}",
            path.display()
        ))
    })?;
    f.write_all(raw)
        .and_then(|_| f.sync_all())
        .map_err(|_| invalid("Cannot persist spatial evidence"))
}
fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    save(
        path,
        &serde_json::to_vec(value).map_err(|_| invalid("Cannot serialize spatial evidence"))?,
    )
}
fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(invalid(message))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialFactoryConfig {
    pub schema: String,
    pub test_only: bool,
    pub run_id: String,
    pub document_sha256: String,
    pub language_profile: String,
    pub endpoint: String,
    pub client_token_file: PathBuf,
    pub python: PathBuf,
    pub runtime_root: PathBuf,
    pub model_dir: PathBuf,
    pub evidence_dir: PathBuf,
    pub max_pages: u32,
    pub max_batches: u32,
    pub input_stop: u64,
    /// Explicit source-reviewed complete TOC pages within this pinned PDF.
    /// All other pages receive identity projection; no global dot stripping.
    #[serde(default)]
    pub toc_page_indices: Vec<u32>,
}
impl SpatialFactoryConfig {
    fn validate(&self) -> Result<()> {
        require(
            self.schema == "mpdf-spatial-factory-config/1" && self.test_only,
            "Use the explicit spatial factory config; historical /1 configs cannot dispatch",
        )?;
        require(
            !self.run_id.is_empty()
                && self.run_id.len() <= 80
                && self
                    .run_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
            "Invalid run id",
        )?;
        require(
            self.document_sha256.len() == 64
                && self.document_sha256.bytes().all(|c| c.is_ascii_hexdigit()),
            "Document digest required",
        )?;
        require(
            !self.language_profile.trim().is_empty()
                && self.max_pages > 0
                && self.max_batches > 0
                && self.input_stop > 0,
            "Explicit spatial budget required",
        )?;
        require(
            self.toc_page_indices.iter().collect::<BTreeSet<_>>().len()
                == self.toc_page_indices.len(),
            "Duplicate TOC page scope",
        )?;
        for path in [
            &self.python,
            &self.runtime_root,
            &self.model_dir,
            &self.evidence_dir,
            &self.client_token_file,
        ] {
            require(path.is_absolute(), "Spatial factory paths must be absolute")?;
        }
        crate::geometry_broker::GeometryBroker::local(
            &self.endpoint,
            crate::Secret::new("validate-only"),
        )
        .map_err(|e| invalid(e.to_string()))?;
        Ok(())
    }
    fn runtime_hashes(&self) -> Result<BTreeMap<String, String>> {
        let mut values = BTreeMap::new();
        for (name, compiled) in runtime_sources() {
            let actual = file_hash(&self.runtime_root.join(name))?;
            require(
                actual == hash(compiled),
                "Selected spatial runtime differs from compiled recipe",
            )?;
            values.insert(name.into(), actual);
        }
        let freeze: Value = serde_json::from_slice(include_bytes!(
            "../../../docs/evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json"
        ))
        .map_err(|_| invalid("Invalid model freeze"))?;
        for (name, expected) in freeze["model_sha256"]
            .as_object()
            .ok_or_else(|| invalid("Missing model freeze"))?
        {
            let actual = file_hash(&self.model_dir.join(name))?;
            require(
                Some(actual.as_str()) == expected.as_str(),
                "Frozen Surya model mismatch",
            )?;
            values.insert(format!("model/{name}"), actual);
        }
        Ok(values)
    }
    fn worker(&self, directory: &Path, mode: &str, page_index: u32, toc: bool) -> Result<()> {
        let log_path = directory.join(format!(
            "runtime-{mode}-{}.log",
            if mode == "verify" { "audit" } else { "initial" }
        ));
        // A verify-only recheck is safe and cannot invoke the network. Its log
        // is append-only; preparation/inference logs remain exclusive attempts.
        let mut options = OpenOptions::new();
        options.write(true);
        if mode == "verify" {
            options.create(true).append(true);
        } else {
            options.create_new(true);
        }
        let log = options
            .open(&log_path)
            .map_err(|_| invalid("Previous spatial runtime attempt exists"))?;
        let mut command = Command::new(&self.python);
        command
            .arg(
                self.runtime_root
                    .join("scripts/ocr/broker/spatial_factory_runtime.py"),
            )
            .arg(mode)
            .arg("--directory")
            .arg(directory)
            .arg("--page-index")
            .arg(page_index.to_string())
            .arg("--model-dir")
            .arg(&self.model_dir)
            .current_dir(&self.runtime_root)
            .env(
                "PYTHONPATH",
                self.runtime_root.join(".runtime/surya-compat"),
            )
            .env_remove("PYTHONOPTIMIZE")
            .env_remove("GEMINI_API_KEY")
            .env_remove("GOOGLE_API_KEY")
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|_| invalid("Runtime log"))?)
            .stderr(log);
        if toc {
            command.arg("--toc");
        }
        let mut child = command
            .spawn()
            .map_err(|_| invalid("Cannot launch spatial runtime"))?;
        let started = Instant::now();
        loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|_| invalid("Cannot wait for spatial runtime"))?
            {
                return require(
                    status.success(),
                    &format!("Spatial runtime {mode} failed; see {}", log_path.display()),
                );
            }
            if started.elapsed() > Duration::from_secs(600) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(invalid("Spatial runtime deadline"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

pub struct SpatialFactory {
    config: SpatialFactoryConfig,
    digest: String,
}
impl SpatialFactory {
    /// Read policy only. No detector, token or network access during CLI dry-run.
    pub fn from_config_file(path: &Path) -> Result<Self> {
        let raw = bytes(path)?;
        require(raw.len() <= 65536, "Spatial config too large")?;
        let config: SpatialFactoryConfig =
            serde_json::from_slice(&raw).map_err(|_| invalid("Invalid spatial factory config"))?;
        config.validate()?;
        let mut identity = serde_json::to_vec(&config).map_err(|_| invalid("Invalid config"))?;
        identity.extend_from_slice(include_bytes!("spatial_factory.rs"));
        identity.extend_from_slice(include_bytes!("../../mpdf-core/src/orchestrator.rs"));
        identity.extend_from_slice(include_bytes!(
            "../../mpdf-core/src/ocr_provider/spatial_transcription.rs"
        ));
        identity.extend_from_slice(include_bytes!(
            "../../mpdf-core/src/derived/toc_projection.rs"
        ));
        for (_, raw) in runtime_sources() {
            identity.extend_from_slice(hash(raw).as_bytes());
        }
        Ok(Self {
            config,
            digest: hash(&identity),
        })
    }
}
impl CloudProviderFactory for SpatialFactory {
    fn mode(&self) -> OcrProviderMode {
        OcrProviderMode::BrokerTest
    }
    fn provider_label(&self) -> &str {
        "frozen-surya+checked-spatial/2+small-batch/1+gemini"
    }
    fn config_digest(&self) -> String {
        self.digest.clone()
    }
    fn fingerprint_contribution(&self) -> String {
        format!("{VERSION}|{CONTRACT}|{MODEL}|{}", self.digest)
    }
    fn max_credits(&self) -> u64 {
        0
    }
    fn fallback(&self) -> CloudFallback {
        CloudFallback::Fail
    }
    fn build(&self, _local: Box<dyn PageOcrProvider>) -> Result<Box<dyn OcrProvider>> {
        let hashes = self.config.runtime_hashes()?;
        Ok(Box::new(SpatialProvider {
            config: self.config.clone(),
            digest: self.digest.clone(),
            hashes,
            job: None,
            used_batches: 0,
        }))
    }
    fn derive_document(&self, package: &DocumentPackage, ocr: &OcrRun) -> Result<DerivedDocument> {
        require(
            package.source.content_sha256 == self.config.document_sha256,
            "Spatial source document changed",
        )?;
        self.config.runtime_hashes()?;
        let mut inputs = Vec::new();
        let mut artifact_pins = Vec::new();
        let mut projections = Vec::new();
        let mut projection_pins = Vec::new();
        for page in &ocr.pages {
            let provenance = page
                .provider_provenance
                .as_ref()
                .ok_or_else(|| invalid("Spatial page lacks operational provenance"))?;
            require(
                provenance.parameters.get("factory_config_sha256") == Some(&self.digest),
                "OCR checkpoint belongs to another factory/config",
            )?;
            let directory = self.config.evidence_dir.join(format!(
                "page-{:04}-{}",
                page.page_index, provenance.input_asset_sha256
            ));
            let pins = read(&directory.join("consumer-pins.json"))?;
            require(
                pins["factory_config_sha256"] == self.digest
                    && pins["document_sha256"] == self.config.document_sha256,
                "Consumer pin origin mismatch",
            )?;
            for name in [
                "expected-inputs.json",
                "dispatch-pins.json",
                "result-pins.json",
                "audit.json",
            ] {
                require(
                    pins["files"][name] == file_hash(&directory.join(name))?,
                    "Operational stage pin changed",
                )?;
            }
            self.config
                .worker(&directory, "verify", page.page_index, false)?;
            let expected: ExpectedSupportBindings =
                serde_json::from_value(pins["expected"].clone())
                    .map_err(|_| invalid("Missing checked expected bindings"))?;
            let checked = CheckedSupportPage::from_json(
                &text(&directory.join("contract.json"))?,
                &text(&directory.join("input-manifest.json"))?,
                &expected,
            )
            .map_err(|e| invalid(e.to_string()))?;
            inputs.push(checked);
            artifact_pins.push(field(&pins, "artifact_sha256")?);
            projections.push(text(&directory.join("projection.json"))?);
            projection_pins.push(field(&pins, "projection_sha256")?);
        }
        let checked: Vec<_> = inputs
            .iter()
            .zip(&artifact_pins)
            .map(|(a, b)| (a, b.as_str()))
            .collect();
        let projected: Vec<_> = projections
            .iter()
            .zip(&projection_pins)
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        DerivedDocument::from_checked_spatial_projection(package, ocr, &checked, &projected)
    }
}

struct SpatialProvider {
    config: SpatialFactoryConfig,
    digest: String,
    hashes: BTreeMap<String, String>,
    job: Option<JobPreparation>,
    used_batches: u32,
}
impl SpatialProvider {
    fn recognize(&mut self, r: &PageOcrRequest<'_>) -> Result<PageOcrOutcome> {
        let job = self
            .job
            .as_ref()
            .ok_or_else(|| invalid("Spatial job not prepared"))?;
        require(
            job.document_sha256 == r.document_sha256
                && job.language_profile == r.language_profile
                && job.dpi == r.dpi
                && job.page_indices.contains(&r.page_index),
            "Page outside selected spatial job",
        )?;
        require(
            self.config.runtime_hashes()? == self.hashes,
            "Runtime changed during job",
        )?;
        let mut encoded = Cursor::new(Vec::new());
        r.page_image
            .write_to(&mut encoded, ImageFormat::Png)
            .map_err(|_| invalid("Cannot encode original raster"))?;
        let png = encoded.into_inner();
        require(
            hash(&png) == r.page_image_sha256,
            "Original raster digest mismatch",
        )?;
        let directory = self
            .config
            .evidence_dir
            .join(format!("page-{:04}-{}", r.page_index, r.page_image_sha256));
        fs::create_dir(&directory)
            .map_err(|_| invalid("Spatial page already attempted; no implicit retry"))?;
        save(&directory.join("input.png"), &png)?;
        self.config
            .worker(&directory, "geometry", r.page_index, false)?;
        let index = read(&directory.join("geometry-index.json"))?;
        require(
            index["runtime"] == VERSION
                && index["image_sha256"] == r.page_image_sha256
                && index["page_index"] == r.page_index,
            "Runtime source identity mismatch",
        )?;
        for (name, digest) in index["files"]
            .as_object()
            .ok_or_else(|| invalid("Missing geometry files"))?
        {
            require(
                Path::new(name).file_name().and_then(|s| s.to_str()) == Some(name.as_str()),
                "Unsafe geometry filename",
            )?;
            require(
                digest.as_str() == Some(file_hash(&directory.join(name))?.as_str()),
                "Runtime geometry artifact changed",
            )?;
        }
        let parent = read(&directory.join("parent.json"))?;
        let mut receipt_hashes = BTreeMap::new();
        let mut receipts = BTreeMap::new();
        for name in index["receipt_paths"]
            .as_array()
            .ok_or_else(|| invalid("Missing receipt index"))?
        {
            let name = name
                .as_str()
                .ok_or_else(|| invalid("Invalid receipt filename"))?;
            require(
                name.starts_with("local-")
                    && name.ends_with(".json")
                    && Path::new(name).file_name().and_then(|s| s.to_str()) == Some(name),
                "Unsafe receipt path",
            )?;
            receipt_hashes.insert(name.to_owned(), file_hash(&directory.join(name))?);
            receipts.insert(name.to_owned(), text(&directory.join(name))?);
        }
        // Stage-selected pins are retained BEFORE Python preparation or network
        // input. They never come from the eventual broker response/artifact.
        let base_expected = json!({"contract_sha256":file_hash(&directory.join("contract.json"))?,
            "parent_artifact_sha256":file_hash(&directory.join("parent.json"))?,
            "parent_geometry_sha256":field(&parent,"geometry_sha256")?,
            "spatial_artifact_sha256":file_hash(&directory.join("spatial.json"))?,"image_sha256":r.page_image_sha256});
        let mut expected_file = base_expected.clone();
        expected_file["receipt_sha256"] = serde_json::to_value(&receipt_hashes).unwrap();
        save_json(&directory.join("expected-inputs.json"), &expected_file)?;
        self.config.worker(
            &directory,
            "prepare",
            r.page_index,
            self.config.toc_page_indices.contains(&r.page_index),
        )?;
        let preparation = read(&directory.join("preparation.json"))?;
        let mut expected = base_expected.clone();
        expected["input_manifest_sha256"] =
            json!(file_hash(&directory.join("input-manifest.json"))?);
        let expected: ExpectedSupportBindings =
            serde_json::from_value(expected).map_err(|_| invalid("Invalid checked bindings"))?;
        let checked = CheckedSupportPage::from_json(
            &text(&directory.join("contract.json"))?,
            &text(&directory.join("input-manifest.json"))?,
            &expected,
        )
        .map_err(|e| invalid(e.to_string()))?;
        let plan = read(&directory.join("plan.json"))?;
        let count = u32::try_from(
            plan["batches"]
                .as_array()
                .ok_or_else(|| invalid("Missing batches"))?
                .len(),
        )
        .map_err(|_| invalid("Batch overflow"))?;
        require(
            count > 0 && self.used_batches.saturating_add(count) <= self.config.max_batches,
            "Configured job batch ceiling exceeded",
        )?;
        let plan_sha = file_hash(&directory.join("plan.json"))?;
        let context_sha = file_hash(&directory.join("context.json"))?;
        require(
            preparation["plan_sha256"] == plan_sha
                && preparation["context_sha256"] == context_sha
                && preparation["input_manifest_sha256"] == expected.input_manifest_sha256,
            "Prepared artifacts changed",
        )?;
        save_json(
            &directory.join("dispatch-pins.json"),
            &json!({"plan_sha256":plan_sha,"context_sha256":context_sha,"input_manifest_sha256":expected.input_manifest_sha256}),
        )?;
        let request = json!({"protocol":PROTOCOL,"idempotency_key":r.idempotency_key,"model":MODEL,
            "geometry_json":text(&directory.join("contract.json"))?,"parent_json":text(&directory.join("parent.json"))?,
            "spatial_json":text(&directory.join("spatial.json"))?,"image_base64":STANDARD.encode(&png),
            "expected":base_expected,"receipts":receipts,"plan_sha256":plan_sha,"max_batches":count,"input_stop":self.config.input_stop});
        save_json(&directory.join("dispatch-request.json"), &request)?;
        // Mark the whole plan before dispatch. A partial/uncertain page has no
        // automatic retry, even if some batches have not reached the vendor.
        self.used_batches += count;
        save_json(
            &directory.join("dispatch-attempt.json"),
            &json!({"attempt":1,"batch_ceiling":count,"automatic_retry":false,"factory_config_sha256":self.digest}),
        )?;
        let token = crate::broker_test::token(&self.config.client_token_file)
            .map_err(|e| invalid(e.to_string()))?;
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| invalid("Spatial HTTP client"))?;
        let response = client
            .post(format!(
                "{}/v2/spatial/pages",
                self.config.endpoint.trim_end_matches('/')
            ))
            .bearer_auth(token.as_str())
            .timeout(Duration::from_secs(7200))
            .json(&request)
            .send()
            .map_err(|_| invalid("Spatial broker response uncertain; no automatic retry"))?;
        let status = response.status();
        let mut raw = Vec::new();
        response
            .take(MAX_REPLY + 1)
            .read_to_end(&mut raw)
            .map_err(|_| invalid("Spatial response read failed; no retry"))?;
        require(raw.len() as u64 <= MAX_REPLY, "Spatial reply too large")?;
        save(&directory.join("broker-response.json"), &raw)?;
        require(
            status.is_success(),
            &format!("Spatial broker HTTP {status}; no /1 fallback or retry"),
        )?;
        let result: Value =
            serde_json::from_slice(&raw).map_err(|_| invalid("Invalid spatial response"))?;
        require(
            result["protocol"] == PROTOCOL
                && result["model"] == MODEL
                && result["idempotency_key"] == r.idempotency_key
                && result["plan_sha256"] == plan_sha
                && result["complete_page"] == true,
            "Spatial response identity mismatch",
        )?;
        save_json(
            &directory.join("result-pins.json"),
            &json!({"broker_response_sha256":hash(&raw)}),
        )?;
        persist_execution(&directory, &result, count)?;
        self.config
            .worker(&directory, "verify", r.page_index, false)?;
        let audited = read(&directory.join("audit.json"))?;
        let response_text = text(&directory.join("execution/response.json"))?;
        let response_sha = field(&audited, "response_sha256")?;
        let usage_json = &audited["usage"];
        let input = usage_json["promptTokenCount"]
            .as_u64()
            .ok_or_else(|| invalid("Missing usage"))?;
        let output = usage_json["candidatesTokenCount"]
            .as_u64()
            .and_then(|n| {
                usage_json["thoughtsTokenCount"]
                    .as_u64()
                    .and_then(|m| n.checked_add(m))
            })
            .ok_or_else(|| invalid("Missing output usage"))?;
        let usage = ProviderUsage {
            input_tokens: input,
            output_tokens: output,
            credits_charged: 0,
            requests: count,
        };
        let page = checked
            .compose(
                &response_text,
                &response_sha,
                "google-gemini",
                MODEL,
                MODEL,
                usage,
            )
            .map_err(|e| invalid(e.to_string()))?;
        let mut outcome = PageOcrOutcome {
            page,
            provider_mode: OcrProviderMode::BrokerTest,
            provider_name: "surya-line-geometry".into(),
            model: MODEL.into(),
            model_version: MODEL.into(),
            raw_transcription_digest: Some(hash(&raw)),
            geometry_source: GeometrySource::DeterministicGeometryBoundText,
            alignment_version: CONTRACT,
            prompt_config_digest: Some(self.digest.clone()),
            usage,
            warnings: vec![],
            fallback_reason: None,
            execution_location: ExecutionLocation::BrokeredCloud,
            decisions: DecisionSummary::default(),
        };
        let mut params = outcome.provenance_parameters();
        params.insert("factory_config_sha256".into(), self.digest.clone());
        params.insert("factory_version".into(), VERSION.into());
        params.insert(
            "transcription_strategy".into(),
            "mpdf-spatial-small-batch/1".into(),
        );
        params.insert(
            "raw_transcription_digest_kind".into(),
            "broker_vendor_archive_sha256".into(),
        );
        params.insert("batch_plan_sha256".into(), plan_sha);
        params.insert(
            "semantic_projection_sha256".into(),
            field(&audited, "projection_sha256")?,
        );
        params.insert("nfc_changes".into(), audited["nfc_changes"].to_string());
        params.insert("billing".into(), "user_managed".into());
        outcome
            .page
            .provider_provenance
            .as_mut()
            .ok_or_else(|| invalid("Missing composed provenance"))?
            .parameters
            .extend(params);
        ocr::validate_ocr_page(&outcome.page).map_err(|e| invalid(e.to_string()))?;
        let artifact = outcome
            .page
            .provider_raw_artifact
            .as_deref()
            .ok_or_else(|| invalid("Missing raw spatial artifact"))?;
        let mut files = BTreeMap::new();
        for name in [
            "expected-inputs.json",
            "dispatch-pins.json",
            "result-pins.json",
            "audit.json",
        ] {
            files.insert(name, file_hash(&directory.join(name))?);
        }
        save_json(
            &directory.join("consumer-pins.json"),
            &json!({"schema":"mpdf-spatial-factory-consumer/1","factory_config_sha256":self.digest,
            "document_sha256":self.config.document_sha256,"expected":expected,"artifact_sha256":hash(artifact.as_bytes()),
            "projection_sha256":field(&audited,"projection_sha256")?,"files":files}),
        )?;
        save_json(&directory.join("composed-page.json"), &outcome.page)?;
        Ok(outcome)
    }
}

fn persist_execution(directory: &Path, result: &Value, count: u32) -> Result<()> {
    let files = result["files"]
        .as_object()
        .ok_or_else(|| invalid("Missing complete execution archive"))?;
    let mut expected: BTreeSet<String> = ["plan.json", "response.json", "aggregation.json"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    for i in 0..count {
        for name in [
            "payload.json",
            "attempt.json",
            "vendor-response.raw.json",
            "raw-response.json",
            "normalized-response.json",
            "normalization.json",
            "accepted.json",
        ] {
            expected.insert(format!("batch-{i:04}/{name}"));
        }
    }
    require(
        files.keys().cloned().collect::<BTreeSet<_>>() == expected,
        "Incomplete or unsafe execution archive",
    )?;
    let out = directory.join("execution");
    fs::create_dir(&out).map_err(|_| invalid("Execution exists"))?;
    for i in 0..count {
        fs::create_dir(out.join(format!("batch-{i:04}")))
            .map_err(|_| invalid("Cannot create batch evidence"))?;
    }
    for (name, value) in files {
        save(
            &out.join(name),
            value
                .as_str()
                .ok_or_else(|| invalid("Invalid execution file"))?
                .as_bytes(),
        )?;
    }
    Ok(())
}
impl OcrProvider for SpatialProvider {
    fn capabilities(&self) -> OcrProviderCapabilities {
        let mut value = crate::broker_test::capabilities();
        value.confidence_metadata = mpdf_core::ocr_provider::MetadataSupport::Unsupported;
        value.language_metadata = mpdf_core::ocr_provider::MetadataSupport::Unsupported;
        value
    }
    fn requires_raster_recognition(&self) -> bool {
        true
    }
    fn validate_configuration(&self) -> std::result::Result<(), ProviderConfigError> {
        self.config
            .validate()
            .map_err(|e| ProviderConfigError::Invalid(e.to_string()))
    }
    fn prepare_job(&mut self, job: &JobPreparation) -> std::result::Result<JobTicket, OcrError> {
        let action = || -> Result<()> {
            require(
                job.document_sha256 == self.config.document_sha256
                    && job.language_profile == self.config.language_profile
                    && job.max_credits == 0
                    && job.page_indices.len() <= self.config.max_pages as usize,
                "Spatial document/language/budget mismatch",
            )?;
            require(
                job.page_indices.iter().collect::<BTreeSet<_>>().len() == job.page_indices.len()
                    && self
                        .config
                        .toc_page_indices
                        .iter()
                        .all(|p| job.page_indices.contains(p)),
                "Spatial page scope mismatch",
            )?;
            fs::create_dir_all(&self.config.evidence_dir)
                .map_err(|_| invalid("Cannot create factory evidence directory"))?;
            let run = json!({"factory":VERSION,"config":self.config,"config_sha256":self.digest,"runtime_hashes":self.hashes,"page_indices":job.page_indices,"mode":"broker-test","billing":"user_managed"});
            let path = self.config.evidence_dir.join("run.json");
            if path.exists() {
                require(read(&path)? == run, "Existing run pin mismatch")?;
            } else {
                save_json(&path, &run)?;
            }
            Ok(())
        };
        action().map_err(|e| OcrError::InvalidEvidence(e.to_string()))?;
        // A resumed document retains the already attempted batch reservations.
        // Durable OCR can skip committed pages, but cannot reset this ceiling.
        self.used_batches = 0;
        for entry in fs::read_dir(&self.config.evidence_dir)
            .map_err(|_| OcrError::InvalidEvidence("Cannot read spatial budget ledger".into()))?
        {
            let entry = entry.map_err(|_| {
                OcrError::InvalidEvidence("Cannot read spatial budget entry".into())
            })?;
            if !entry.file_name().to_string_lossy().starts_with("page-") {
                continue;
            }
            let path = entry.path().join("dispatch-attempt.json");
            if path.exists() {
                let prior = read(&path).map_err(|e| OcrError::InvalidEvidence(e.to_string()))?;
                if prior["factory_config_sha256"] != self.digest {
                    return Err(OcrError::InvalidEvidence(
                        "Prior batch reservation belongs to another config".into(),
                    ));
                }
                let count = prior["batch_ceiling"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| {
                        OcrError::InvalidEvidence("Invalid prior batch reservation".into())
                    })?;
                self.used_batches = self.used_batches.checked_add(count).ok_or_else(|| {
                    OcrError::InvalidEvidence("Batch reservation overflow".into())
                })?;
            }
        }
        if self.used_batches > self.config.max_batches {
            return Err(OcrError::InvalidEvidence(
                "Retained spatial batch ceiling exceeded".into(),
            ));
        }
        self.job = Some(job.clone());
        Ok(JobTicket {
            job_id: job.job_id.clone(),
            reservation_id: None,
            reserved_credits: 0,
            estimated_credits: 0,
        })
    }
    fn recognize_page(
        &mut self,
        r: &PageOcrRequest<'_>,
    ) -> std::result::Result<PageOcrOutcome, OcrError> {
        self.recognize(r)
            .map_err(|e| OcrError::InvalidEvidence(e.to_string()))
    }
    fn cancel_job(&mut self, _: &JobTicket) -> std::result::Result<(), OcrError> {
        self.job = None;
        Ok(())
    }
    fn finalize_job(&mut self, _: &JobTicket) -> std::result::Result<JobSettlement, OcrError> {
        self.job = None;
        Ok(JobSettlement::default())
    }
    fn fingerprint_contribution(&self) -> String {
        format!("{VERSION}|{}", self.digest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_and_traversing_archives_fail_before_writing() {
        for files in [
            json!({}),
            json!({"../escape": "bad"}),
            json!({"plan.json":"{}", "response.json":"{}", "aggregation.json":"{}"}),
        ] {
            let temp = tempfile::tempdir().unwrap();
            assert!(persist_execution(temp.path(), &json!({"files":files}), 1).is_err());
            assert!(!temp.path().join("execution").exists());
        }
    }
    #[test]
    fn archive_is_complete_exclusive_and_preserves_vendor_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let mut files = serde_json::Map::new();
        for name in ["plan.json", "response.json", "aggregation.json"] {
            files.insert(name.into(), json!("{}"));
        }
        for name in [
            "payload.json",
            "attempt.json",
            "vendor-response.raw.json",
            "raw-response.json",
            "normalized-response.json",
            "normalization.json",
            "accepted.json",
        ] {
            files.insert(
                format!("batch-0000/{name}"),
                json!(" {\"text\":\"e\u{301}\"}\n"),
            );
        }
        let result = json!({"files":files});
        persist_execution(temp.path(), &result, 1).unwrap();
        assert_eq!(
            text(
                &temp
                    .path()
                    .join("execution/batch-0000/vendor-response.raw.json")
            )
            .unwrap(),
            " {\"text\":\"e\u{301}\"}\n"
        );
        assert!(persist_execution(temp.path(), &result, 1).is_err());
    }
}
