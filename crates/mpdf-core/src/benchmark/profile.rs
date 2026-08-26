//! Versioned benchmark profile manifest: `mpdf-benchmark-profile`.
//!
//! A profile is a fixed, explicit list of `[[runs]]` (method/parameter/DPI
//! combinations) to execute against a dataset. This is deliberately not a
//! combinatorial sweep generator — see `docs/benchmark-datasets.md`,
//! "Why profiles enumerate runs" — so a profile file can never accidentally
//! expand into an unbounded number of benchmark runs.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::benchmark::limits::MAX_PROFILE_RUNS;
use crate::binarization::SauvolaParams;
use crate::cleanup::{CleanupSettings, DespeckleLevel};
use crate::error::{CoreError, Result};
use crate::settings::{BinarizationMethod, PreprocessingSettings, ProcessingSettings};

pub const PROFILE_SCHEMA: &str = "mpdf-benchmark-profile";
pub const PROFILE_SCHEMA_VERSION: &str = "1.0";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileManifest {
    pub schema: String,
    pub schema_version: String,
    pub id: String,
    #[serde(default)]
    pub description: String,
    pub runs: Vec<RunConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodConfig {
    Otsu,
    Sauvola,
    Manual,
}

/// One `[[runs]]` entry — a fully explicit, named method/parameter/DPI
/// combination. Fields not applicable to `method` must be absent, the
/// same discipline the CLI's `SettingsArgs::to_settings` already
/// enforces, so a profile can never silently ignore a parameter typo'd
/// against the wrong method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunConfig {
    pub id: String,
    pub dpi: u16,
    pub method: MethodConfig,
    #[serde(default)]
    pub threshold: Option<u8>,
    #[serde(default)]
    pub k: Option<f32>,
    #[serde(default)]
    pub window_size: Option<u32>,
    #[serde(default)]
    pub dynamic_range: Option<f32>,
    #[serde(default)]
    pub contrast: f32,
    #[serde(default)]
    pub median_denoise: bool,
    #[serde(default)]
    pub background_normalization: bool,
    #[serde(default)]
    pub background_radius: Option<u32>,
    #[serde(default)]
    pub despeckle: Option<DespeckleConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DespeckleConfig {
    Off,
    Conservative,
    Strong,
}

impl From<DespeckleConfig> for DespeckleLevel {
    fn from(value: DespeckleConfig) -> Self {
        match value {
            DespeckleConfig::Off => DespeckleLevel::Off,
            DespeckleConfig::Conservative => DespeckleLevel::Conservative,
            DespeckleConfig::Strong => DespeckleLevel::Strong,
        }
    }
}

impl RunConfig {
    /// Builds validated [`ProcessingSettings`] from this run, rejecting
    /// parameter combinations that do not apply to the chosen method
    /// (e.g. `k` set while `method = "otsu"`), then delegating final
    /// range validation to [`ProcessingSettings::validate`] — the exact
    /// same validation `process`/`analyze`/`estimate` use, not a second
    /// implementation.
    pub fn to_processing_settings(&self) -> Result<ProcessingSettings> {
        if self.threshold.is_some() && self.method != MethodConfig::Manual {
            return Err(CoreError::InvalidProfile(format!(
                "run '{}': threshold only applies to method = \"manual\"",
                self.id
            )));
        }
        if (self.k.is_some() || self.window_size.is_some() || self.dynamic_range.is_some())
            && self.method != MethodConfig::Sauvola
        {
            return Err(CoreError::InvalidProfile(format!(
                "run '{}': k/window_size/dynamic_range only apply to method = \"sauvola\"",
                self.id
            )));
        }
        if self.background_radius.is_some() && !self.background_normalization {
            return Err(CoreError::InvalidProfile(format!(
                "run '{}': background_radius requires background_normalization = true",
                self.id
            )));
        }
        if self.method == MethodConfig::Manual && self.threshold.is_none() {
            return Err(CoreError::InvalidProfile(format!(
                "run '{}': method = \"manual\" requires threshold",
                self.id
            )));
        }

        let method = match self.method {
            MethodConfig::Otsu => BinarizationMethod::Otsu,
            MethodConfig::Manual => BinarizationMethod::Manual {
                threshold: self.threshold.expect("checked above"),
            },
            MethodConfig::Sauvola => {
                let defaults = SauvolaParams::default();
                BinarizationMethod::Sauvola(SauvolaParams {
                    window_size: self.window_size.unwrap_or(defaults.window_size),
                    k: self.k.unwrap_or(defaults.k),
                    dynamic_range: self.dynamic_range.unwrap_or(defaults.dynamic_range),
                })
            }
        };

        let settings = ProcessingSettings {
            dpi: self.dpi,
            method,
            contrast: self.contrast,
            preprocessing: PreprocessingSettings {
                median_denoise: self.median_denoise,
                background_normalization: self.background_normalization,
                background_radius: self
                    .background_radius
                    .unwrap_or_else(|| PreprocessingSettings::default().background_radius),
            },
            cleanup: CleanupSettings {
                despeckle: self.despeckle.map(Into::into).unwrap_or_default(),
                max_component_pixels: None,
            },
        };
        settings.validate()?;
        Ok(settings)
    }
}

/// A profile that has been parsed and schema-checked, alongside the raw
/// bytes it was parsed from (for digest computation).
#[derive(Debug, Clone)]
pub struct LoadedProfile {
    pub manifest: ProfileManifest,
    manifest_bytes: Vec<u8>,
}

impl LoadedProfile {
    pub fn load(profile_path: &Path) -> Result<Self> {
        let manifest_bytes = std::fs::read(profile_path)
            .map_err(|e| CoreError::io(profile_path.to_path_buf(), e))?;
        let manifest_text = String::from_utf8_lossy(&manifest_bytes);
        let manifest: ProfileManifest = toml::from_str(&manifest_text).map_err(|e| {
            CoreError::InvalidProfile(format!("could not parse {}: {e}", profile_path.display()))
        })?;

        if manifest.schema != PROFILE_SCHEMA {
            return Err(CoreError::UnsupportedBenchmarkSchema(format!(
                "expected schema '{PROFILE_SCHEMA}', got '{}'",
                manifest.schema
            )));
        }
        if manifest.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(CoreError::UnsupportedBenchmarkSchema(format!(
                "expected profile schema version '{PROFILE_SCHEMA_VERSION}', got '{}'",
                manifest.schema_version
            )));
        }
        if manifest.runs.is_empty() {
            return Err(CoreError::InvalidProfile("profile has no runs".to_string()));
        }
        if manifest.runs.len() > MAX_PROFILE_RUNS {
            return Err(CoreError::ResourceLimitExceeded(format!(
                "profile declares {} runs, exceeding the maximum of {MAX_PROFILE_RUNS}",
                manifest.runs.len()
            )));
        }
        let mut seen = std::collections::BTreeSet::new();
        for run in &manifest.runs {
            if !seen.insert(run.id.as_str()) {
                return Err(CoreError::InvalidProfile(format!(
                    "duplicate run id '{}'",
                    run.id
                )));
            }
            // Validate eagerly so a bad profile is caught at load time,
            // not partway through a benchmark run.
            run.to_processing_settings()?;
        }

        Ok(Self {
            manifest,
            manifest_bytes,
        })
    }

    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&self.manifest_bytes);
        format!("{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn base_profile() -> String {
        r#"
schema = "mpdf-benchmark-profile"
schema_version = "1.0"
id = "default-method-comparison"

[[runs]]
id = "otsu-300"
dpi = 300
method = "otsu"

[[runs]]
id = "sauvola-300-default"
dpi = 300
method = "sauvola"
window_size = 25
k = 0.2

[[runs]]
id = "manual-300-180"
dpi = 300
method = "manual"
threshold = 180
"#
        .to_string()
    }

    #[test]
    fn loads_a_valid_profile_with_three_runs() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "profile.toml", &base_profile());
        let profile = LoadedProfile::load(&path).unwrap();
        assert_eq!(profile.manifest.runs.len(), 3);
    }

    #[test]
    fn otsu_run_builds_settings_with_no_extra_params() {
        let run = RunConfig {
            id: "x".into(),
            dpi: 300,
            method: MethodConfig::Otsu,
            threshold: None,
            k: None,
            window_size: None,
            dynamic_range: None,
            contrast: 0.0,
            median_denoise: false,
            background_normalization: false,
            background_radius: None,
            despeckle: None,
        };
        let settings = run.to_processing_settings().unwrap();
        assert!(matches!(settings.method, BinarizationMethod::Otsu));
    }

    #[test]
    fn rejects_threshold_on_a_non_manual_method() {
        let run = RunConfig {
            id: "x".into(),
            dpi: 300,
            method: MethodConfig::Otsu,
            threshold: Some(100),
            k: None,
            window_size: None,
            dynamic_range: None,
            contrast: 0.0,
            median_denoise: false,
            background_normalization: false,
            background_radius: None,
            despeckle: None,
        };
        assert!(matches!(
            run.to_processing_settings(),
            Err(CoreError::InvalidProfile(_))
        ));
    }

    #[test]
    fn rejects_manual_without_a_threshold() {
        let run = RunConfig {
            id: "x".into(),
            dpi: 300,
            method: MethodConfig::Manual,
            threshold: None,
            k: None,
            window_size: None,
            dynamic_range: None,
            contrast: 0.0,
            median_denoise: false,
            background_normalization: false,
            background_radius: None,
            despeckle: None,
        };
        assert!(matches!(
            run.to_processing_settings(),
            Err(CoreError::InvalidProfile(_))
        ));
    }

    #[test]
    fn rejects_background_radius_without_normalization() {
        let run = RunConfig {
            id: "x".into(),
            dpi: 300,
            method: MethodConfig::Otsu,
            threshold: None,
            k: None,
            window_size: None,
            dynamic_range: None,
            contrast: 0.0,
            median_denoise: false,
            background_normalization: false,
            background_radius: Some(10),
            despeckle: None,
        };
        assert!(run.to_processing_settings().is_err());
    }

    #[test]
    fn rejects_unsupported_dpi_via_shared_core_validation() {
        let run = RunConfig {
            id: "x".into(),
            dpi: 150,
            method: MethodConfig::Otsu,
            threshold: None,
            k: None,
            window_size: None,
            dynamic_range: None,
            contrast: 0.0,
            median_denoise: false,
            background_normalization: false,
            background_radius: None,
            despeckle: None,
        };
        assert!(run.to_processing_settings().is_err());
    }

    #[test]
    fn rejects_duplicate_run_ids() {
        let dir = tempfile::tempdir().unwrap();
        let mut text = base_profile();
        text.push_str("\n[[runs]]\nid = \"otsu-300\"\ndpi = 300\nmethod = \"otsu\"\n");
        let path = write(dir.path(), "profile.toml", &text);
        let err = LoadedProfile::load(&path).unwrap_err();
        assert!(matches!(err, CoreError::InvalidProfile(_)));
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let text = base_profile().replace("schema_version = \"1.0\"", "schema_version = \"2.0\"");
        let path = write(dir.path(), "profile.toml", &text);
        let err = LoadedProfile::load(&path).unwrap_err();
        assert!(matches!(err, CoreError::UnsupportedBenchmarkSchema(_)));
    }

    #[test]
    fn digest_is_stable_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "profile.toml", &base_profile());
        let a = LoadedProfile::load(&path).unwrap();
        let b = LoadedProfile::load(&path).unwrap();
        assert_eq!(a.digest(), b.digest());
    }

    #[test]
    fn rejects_a_profile_exceeding_the_run_limit() {
        let mut text = String::from(
            "schema = \"mpdf-benchmark-profile\"\nschema_version = \"1.0\"\nid = \"huge\"\n",
        );
        for i in 0..(MAX_PROFILE_RUNS + 1) {
            text.push_str(&format!(
                "\n[[runs]]\nid = \"run-{i}\"\ndpi = 300\nmethod = \"otsu\"\n"
            ));
        }
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "profile.toml", &text);
        let err = LoadedProfile::load(&path).unwrap_err();
        assert!(matches!(err, CoreError::ResourceLimitExceeded(_)));
    }
}
