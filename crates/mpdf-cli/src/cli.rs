//! Command-line argument definitions: parsing only, no execution logic.
//! See `src/commands/` for what each command actually does.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use mpdf_core::binarization::SauvolaParams;
use mpdf_core::cleanup::{CleanupSettings, DespeckleLevel};
use mpdf_core::pdfium_backend::PdfiumConfig;
use mpdf_core::report::PathMode;
use mpdf_core::settings::{
    BinarizationMethod, PreprocessingSettings, ProcessingSettings, SUPPORTED_DPI,
};
use mpdf_core::validation::ValidationMode;

#[derive(Parser)]
#[command(name = "mpdf", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Print project and build information.
    Info(InfoArgs),
    /// Inspect a PDF: page count, geometry, rotation, and render sizes.
    Inspect(InspectArgs),
    /// Render and measure a PDF through the real pipeline, without
    /// writing an output PDF.
    Analyze(AnalyzeArgs),
    /// Estimate output size by processing a deterministic sample of
    /// pages. Experimental; does not write a converted PDF.
    Estimate(EstimateArgs),
    /// Convert a PDF into a bilevel CCITT Group 4 PDF.
    Process(ProcessArgs),
    /// Render and process one page, saving a PNG preview.
    Preview(PreviewArgs),
    /// Ground-truth binarization-fidelity benchmarking. Benchmark
    /// requires pixel-accurate ground truth; `analyze` is not a
    /// benchmark and has no ground truth at all.
    #[command(subcommand)]
    Benchmark(BenchmarkCommand),
    /// Create or validate an MDP 0.1 evidence package.
    #[command(subcommand)]
    Package(PackageCommand),
    /// Inspect and exercise the local persistent job store (development API).
    #[command(subcommand)]
    Job(JobCommand),
    /// Run the local OCR routing pipeline and write an MDP OCR extension.
    Ocr(OcrArgs),
    /// Build deterministic AI-ready derived records and export them.
    Export(ExportArgs),
    /// Generate a typed local review queue from MDP/OCR evidence.
    Review(ReviewArgs),
    /// Append or inspect local revision overlays.
    #[command(subcommand)]
    Revision(RevisionCommand),
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ExportFormatArg {
    All,
    Json,
    Jsonl,
    Markdown,
    Txt,
    Html,
    Hocr,
    Alto,
}

impl ExportFormatArg {
    pub fn as_core(self) -> mpdf_core::derived::ExportFormat {
        match self {
            Self::All => unreachable!("all is expanded by the export command"),
            Self::Json => mpdf_core::derived::ExportFormat::Json,
            Self::Jsonl => mpdf_core::derived::ExportFormat::Jsonl,
            Self::Markdown => mpdf_core::derived::ExportFormat::Markdown,
            Self::Txt => mpdf_core::derived::ExportFormat::Text,
            Self::Html => mpdf_core::derived::ExportFormat::Html,
            Self::Hocr => mpdf_core::derived::ExportFormat::Hocr,
            Self::Alto => mpdf_core::derived::ExportFormat::Alto,
        }
    }
}

#[derive(Args)]
pub struct ExportArgs {
    /// Existing MDP package directory.
    pub input: PathBuf,
    /// Destination file (or directory with --format all).
    #[arg(long)]
    pub output: PathBuf,
    /// Derived output format.
    #[arg(long, value_enum)]
    pub format: ExportFormatArg,
    /// Replace existing export files atomically.
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Args)]
pub struct ReviewArgs {
    /// Existing MDP package directory.
    pub input: PathBuf,
    #[command(flatten)]
    pub output_mode: OutputArgs,
}

#[derive(Subcommand)]
pub enum RevisionCommand {
    /// Append a human or AI-suggested revision without changing OCR evidence.
    Add(RevisionAddArgs),
    /// List append-only revision records.
    List(RevisionListArgs),
}

#[derive(Args)]
pub struct RevisionAddArgs {
    /// Existing MDP package directory.
    pub input: PathBuf,
    /// Stable word/block/line reference from `mpdf export --format json`.
    #[arg(long)]
    pub target_ref: String,
    /// Evidence digest shown by the derived record for this page.
    #[arg(long)]
    pub base_evidence_digest: String,
    /// Replacement or suggested text.
    #[arg(long)]
    pub text: String,
    /// Human revisions are applied by default; AI suggestions are preserved.
    #[arg(long, value_enum, default_value = "human")]
    pub kind: RevisionKindArg,
    /// Stable caller-supplied revision id.
    #[arg(long)]
    pub revision_id: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum RevisionKindArg {
    Human,
    AiSuggested,
}

#[derive(Args)]
pub struct RevisionListArgs {
    /// Existing MDP package directory.
    pub input: PathBuf,
    #[command(flatten)]
    pub output_mode: OutputArgs,
}

#[derive(Subcommand)]
pub enum PackageCommand {
    /// Create an evidence-only package from a PDF. The source PDF is not copied.
    Create(PackageCreateArgs),
    /// Validate a package directory, including paths, references and digests.
    Validate(PackageValidateArgs),
}

#[derive(Subcommand)]
pub enum JobCommand {
    /// Create an empty persistent job with one queued page record per page.
    Create(JobCreateArgs),
    /// Print durable job/page progress as JSON.
    Status(JobStatusArgs),
    /// Request cancellation while retaining already committed pages.
    Cancel(JobCancelArgs),
}

#[derive(Clone, Copy, ValueEnum)]
pub enum OcrProviderArg {
    /// Deterministic offline provider for development and tests.
    Reference,
    /// Explicit RapidOCR/ONNX sidecar executable.
    Rapidocr,
}

#[derive(Args)]
pub struct OcrArgs {
    /// Input PDF.
    pub input: PathBuf,
    /// New MDP package directory.
    #[arg(long)]
    pub output: PathBuf,
    /// SQLite WAL job database used for page checkpoints.
    #[arg(long)]
    pub jobs_db: PathBuf,
    /// Stable job identifier.
    #[arg(long)]
    pub job_id: String,
    /// Production default. `reference` is an offline deterministic provider
    /// intended for development and tests only.
    #[arg(long, value_enum, default_value = "rapidocr")]
    pub provider: OcrProviderArg,
    /// RapidOCR/ONNX executable (required with --provider rapidocr).
    #[arg(long)]
    pub provider_executable: Option<PathBuf>,
    /// RapidOCR model directory (required with --provider rapidocr).
    #[arg(long)]
    pub model_dir: Option<PathBuf>,
    #[command(flatten)]
    pub pdfium: PdfiumArgs,
    #[command(flatten)]
    pub output_mode: OutputArgs,
}

#[derive(Args)]
pub struct JobCreateArgs {
    /// SQLite database path. The database uses WAL mode.
    #[arg(long)]
    pub db: PathBuf,
    /// Stable job identifier.
    #[arg(long)]
    pub job_id: String,
    /// Number of pages to queue.
    #[arg(long)]
    pub pages: u32,
}

#[derive(Args)]
pub struct JobStatusArgs {
    /// SQLite database path.
    #[arg(long)]
    pub db: PathBuf,
    /// Stable job identifier.
    #[arg(long)]
    pub job_id: String,
}

#[derive(Args)]
pub struct JobCancelArgs {
    /// SQLite database path.
    #[arg(long)]
    pub db: PathBuf,
    /// Stable job identifier.
    #[arg(long)]
    pub job_id: String,
}

#[derive(Args)]
pub struct PackageCreateArgs {
    /// Input PDF.
    pub input: PathBuf,
    /// New package directory.
    #[arg(long)]
    pub output: PathBuf,
    #[command(flatten)]
    pub pdfium: PdfiumArgs,
    #[command(flatten)]
    pub output_mode: OutputArgs,
}

#[derive(Args)]
pub struct PackageValidateArgs {
    /// Existing package directory.
    pub input: PathBuf,
    #[command(flatten)]
    pub output_mode: OutputArgs,
}

#[derive(Subcommand)]
pub enum BenchmarkCommand {
    /// Runs a benchmark: every `[[runs]]` in a profile against every
    /// page in a dataset, comparing output to ground truth.
    Run(BenchmarkRunArgs),
    /// Validates a dataset and/or profile manifest without processing
    /// anything: schema, file existence, ground-truth validity,
    /// dimensions, ROI bounds, and settings.
    Validate(BenchmarkValidateArgs),
}

#[derive(Args)]
pub struct BenchmarkRunArgs {
    /// Path to a `mpdf-benchmark-dataset` manifest.
    #[arg(long)]
    pub dataset: PathBuf,
    /// Path to a `mpdf-benchmark-profile` manifest.
    #[arg(long)]
    pub profile: PathBuf,

    #[command(flatten)]
    pub output: OutputArgs,
    /// Write the benchmark report to this path (in addition to any
    /// stdout output), atomically, refusing to overwrite unless
    /// `--overwrite` is also given. Rejected if it would alias the
    /// dataset or profile manifest.
    #[arg(long)]
    pub report: Option<PathBuf>,
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Args)]
pub struct BenchmarkValidateArgs {
    /// Path to a `mpdf-benchmark-dataset` manifest.
    #[arg(long)]
    pub dataset: PathBuf,
    /// Path to a `mpdf-benchmark-profile` manifest. Optional
    /// — a dataset can be validated on its own.
    #[arg(long)]
    pub profile: Option<PathBuf>,

    #[command(flatten)]
    pub output: OutputArgs,
}

/// PDFium library location, shared verbatim by every command that opens a
/// document. Not duplicated per command with subtly different names or
/// defaults.
#[derive(Args, Clone)]
pub struct PdfiumArgs {
    /// Path to a PDFium dynamic library.
    #[arg(long)]
    pub pdfium_library: Option<PathBuf>,
    /// Allow loading PDFium from the system library search path.
    #[arg(long)]
    pub allow_system_pdfium: bool,
}

impl PdfiumArgs {
    pub fn to_config(&self) -> PdfiumConfig {
        PdfiumConfig {
            library_path: self.pdfium_library.clone(),
            allow_system_library: self.allow_system_pdfium,
        }
    }
}

/// JSON/quiet output mode, shared by every command that can emit a
/// machine-readable report.
#[derive(Args, Clone, Copy, Default)]
pub struct OutputArgs {
    /// Emit one machine-readable JSON report to stdout instead of
    /// human-readable text. Progress and diagnostics still go to stderr.
    #[arg(long)]
    pub json: bool,
    /// Pretty-print JSON output (two-space indentation). Has no effect
    /// without `--json` or `--report`.
    #[arg(long)]
    pub pretty: bool,
    /// Suppress human-readable progress and success messages. The final
    /// result (human or `--json`) is still printed.
    #[arg(long)]
    pub quiet: bool,
}

/// How a source path is rendered into a JSON report.
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Default)]
pub enum PathModeArg {
    #[default]
    Basename,
    Relative,
    Absolute,
    Omit,
}

impl From<PathModeArg> for PathMode {
    fn from(v: PathModeArg) -> Self {
        match v {
            PathModeArg::Basename => PathMode::Basename,
            PathModeArg::Relative => PathMode::Relative,
            PathModeArg::Absolute => PathMode::Absolute,
            PathModeArg::Omit => PathMode::Omit,
        }
    }
}

#[derive(Args)]
pub struct InfoArgs {
    #[command(flatten)]
    pub output: OutputArgs,
    /// Attempt to resolve and load a PDFium library, and report whether
    /// that succeeded. Without this flag, `info` never touches PDFium.
    #[arg(long)]
    pub probe_pdfium: bool,
    #[command(flatten)]
    pub pdfium: PdfiumArgs,
}

#[derive(Args)]
pub struct InspectArgs {
    /// Input PDF.
    pub input: PathBuf,
    #[command(flatten)]
    pub output: OutputArgs,
    /// How a source path is rendered in a JSON report.
    #[arg(long, value_enum, default_value_t = PathModeArg::Basename)]
    pub path_mode: PathModeArg,
    #[command(flatten)]
    pub pdfium: PdfiumArgs,
}

#[derive(Args)]
pub struct AnalyzeArgs {
    /// Input PDF.
    pub input: PathBuf,

    #[command(flatten)]
    pub settings: SettingsArgs,

    /// Pages to analyze: "all", "3", "1-5", or "1,3,8-12" (one-based).
    #[arg(long, default_value = "all")]
    pub pages: String,
    /// Also run CCITT Group 4 encoding to report its size (discarded,
    /// never written). Off by default: extra work `analyze`'s core
    /// purpose does not require.
    #[arg(long)]
    pub encode: bool,

    #[command(flatten)]
    pub output: OutputArgs,
    /// How the source path is rendered in a JSON report.
    #[arg(long, value_enum, default_value_t = PathModeArg::Basename)]
    pub path_mode: PathModeArg,
    /// Write the analysis report to this path (in addition to any
    /// stdout output), atomically, refusing to overwrite unless
    /// `--overwrite` is also given.
    #[arg(long)]
    pub report: Option<PathBuf>,
    #[arg(long)]
    pub overwrite: bool,

    #[command(flatten)]
    pub pdfium: PdfiumArgs,
}

#[derive(Args)]
pub struct EstimateArgs {
    /// Input PDF.
    pub input: PathBuf,

    #[command(flatten)]
    pub settings: SettingsArgs,

    /// How many pages to sample. Clamped to the document's actual page
    /// count when it has fewer pages than this; otherwise validated
    /// against a documented minimum and maximum (see docs/cli.md).
    #[arg(long, default_value_t = mpdf_core::estimation::DEFAULT_SAMPLE_COUNT)]
    pub samples: u32,

    #[command(flatten)]
    pub output: OutputArgs,
    /// How the source path is rendered in a JSON report.
    #[arg(long, value_enum, default_value_t = PathModeArg::Basename)]
    pub path_mode: PathModeArg,
    /// Write the estimate report to this path (in addition to any
    /// stdout output), atomically, refusing to overwrite unless
    /// `--overwrite` is also given. Rejected if it would alias `input`
    /// — including a not-yet-existing path spelled differently, the same
    /// normalization the M3 review added for `process --report`.
    #[arg(long)]
    pub report: Option<PathBuf>,
    #[arg(long)]
    pub overwrite: bool,

    #[command(flatten)]
    pub pdfium: PdfiumArgs,
}

#[derive(Args)]
pub struct ProcessArgs {
    /// Input PDF.
    pub input: PathBuf,
    /// Destination PDF.
    #[arg(long, short)]
    pub output: PathBuf,

    #[command(flatten)]
    pub settings: SettingsArgs,

    /// Replace the destination if it already exists.
    #[arg(long)]
    pub overwrite: bool,
    /// How thoroughly to validate the finished output.
    #[arg(long, value_enum, default_value_t = ValidateArg::Structural)]
    pub validate: ValidateArg,

    #[command(flatten)]
    pub output_mode: OutputArgs,
    /// Write the process report to this path, atomically, subject to the
    /// same `--overwrite` flag as the output PDF.
    #[arg(long)]
    pub report: Option<PathBuf>,

    #[command(flatten)]
    pub pdfium: PdfiumArgs,
}

#[derive(Args)]
pub struct PreviewArgs {
    /// Input PDF.
    pub input: PathBuf,
    /// One-based page number to preview.
    #[arg(long)]
    pub page: u32,
    /// Destination PNG.
    #[arg(long, short)]
    pub output: PathBuf,

    #[command(flatten)]
    pub settings: SettingsArgs,

    /// Replace the destination if it already exists.
    #[arg(long)]
    pub overwrite: bool,

    #[command(flatten)]
    pub output_mode: OutputArgs,

    #[command(flatten)]
    pub pdfium: PdfiumArgs,
}

/// Processing parameters shared by `analyze`, `process`, and `preview`.
///
/// Every default here comes from the core (`ProcessingSettings`,
/// `PreprocessingSettings`, `CleanupSettings`, `SauvolaParams`) rather than
/// being reinvented in the CLI, so there is exactly one place that decides
/// what "default Sauvola window" or "default background radius" means.
#[derive(Args)]
pub struct SettingsArgs {
    /// Output resolution.
    #[arg(long, default_value_t = 400)]
    pub dpi: u16,
    /// Binarization method.
    #[arg(long, value_enum, default_value_t = MethodArg::Sauvola)]
    pub method: MethodArg,
    /// Fixed threshold for `--method manual`.
    #[arg(long)]
    pub threshold: Option<u8>,
    /// Sauvola sensitivity constant.
    #[arg(long)]
    pub sauvola_k: Option<f32>,
    /// Sauvola window size (odd). Defaults to a value derived from DPI.
    #[arg(long)]
    pub sauvola_window: Option<u32>,
    /// Grayscale contrast adjustment, -1.0 to 1.0.
    #[arg(long, default_value_t = 0.0)]
    pub contrast: f32,
    /// Enable the 3x3 median denoise preprocessing step.
    #[arg(long)]
    pub median_denoise: bool,
    /// Enable background normalization.
    #[arg(long)]
    pub background_normalization: bool,
    /// Box-filter radius used by background normalization.
    #[arg(long)]
    pub background_radius: Option<u32>,
    /// Despeckle cleanup level.
    #[arg(long, value_enum, default_value_t = DespeckleArg::Off)]
    pub despeckle: DespeckleArg,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum MethodArg {
    Otsu,
    Sauvola,
    Manual,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum DespeckleArg {
    Off,
    Conservative,
    Strong,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum ValidateArg {
    Structural,
    RenderAll,
}

impl From<ValidateArg> for ValidationMode {
    fn from(v: ValidateArg) -> Self {
        match v {
            ValidateArg::Structural => ValidationMode::Structural,
            ValidateArg::RenderAll => ValidationMode::RenderAll,
        }
    }
}

impl From<DespeckleArg> for DespeckleLevel {
    fn from(d: DespeckleArg) -> Self {
        match d {
            DespeckleArg::Off => DespeckleLevel::Off,
            DespeckleArg::Conservative => DespeckleLevel::Conservative,
            DespeckleArg::Strong => DespeckleLevel::Strong,
        }
    }
}

impl SettingsArgs {
    /// Builds core settings, rejecting parameter combinations that would
    /// otherwise be silently ignored.
    pub fn to_settings(&self) -> Result<ProcessingSettings, String> {
        if !SUPPORTED_DPI.contains(&self.dpi) {
            return Err(format!(
                "unsupported --dpi {}; expected one of {:?}",
                self.dpi, SUPPORTED_DPI
            ));
        }

        if self.threshold.is_some() && self.method != MethodArg::Manual {
            return Err(
                "--threshold only applies to --method manual; remove it or change the method"
                    .to_string(),
            );
        }
        if (self.sauvola_k.is_some() || self.sauvola_window.is_some())
            && self.method != MethodArg::Sauvola
        {
            return Err(
                "--sauvola-k and --sauvola-window only apply to --method sauvola".to_string(),
            );
        }
        if let Some(window) = self.sauvola_window {
            if window == 0 || window % 2 == 0 {
                return Err(format!(
                    "--sauvola-window must be a positive odd number, got {window}"
                ));
            }
        }
        if self.background_radius.is_some() && !self.background_normalization {
            return Err("--background-radius requires --background-normalization".to_string());
        }
        if !(-1.0..=1.0).contains(&self.contrast) {
            return Err(format!(
                "--contrast must be between -1.0 and 1.0, got {}",
                self.contrast
            ));
        }

        let defaults = SauvolaParams::default();
        let method = match self.method {
            MethodArg::Otsu => BinarizationMethod::Otsu,
            MethodArg::Manual => BinarizationMethod::Manual {
                threshold: self
                    .threshold
                    .ok_or_else(|| "--method manual requires --threshold <0-255>".to_string())?,
            },
            MethodArg::Sauvola => BinarizationMethod::Sauvola(SauvolaParams {
                window_size: self
                    .sauvola_window
                    .unwrap_or_else(|| mpdf_core::binarization::sauvola_window_for_dpi(self.dpi)),
                k: self.sauvola_k.unwrap_or(defaults.k),
                dynamic_range: defaults.dynamic_range,
            }),
        };

        let preprocessing_defaults = PreprocessingSettings::default();
        Ok(ProcessingSettings {
            dpi: self.dpi,
            method,
            contrast: self.contrast,
            preprocessing: PreprocessingSettings {
                median_denoise: self.median_denoise,
                background_normalization: self.background_normalization,
                background_radius: self
                    .background_radius
                    .unwrap_or(preprocessing_defaults.background_radius),
            },
            cleanup: CleanupSettings {
                despeckle: self.despeckle.into(),
                max_component_pixels: None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpdf_core::binarization::sauvola_window_for_dpi;

    fn args(method: MethodArg) -> SettingsArgs {
        SettingsArgs {
            dpi: 400,
            method,
            threshold: None,
            sauvola_k: None,
            sauvola_window: None,
            contrast: 0.0,
            median_denoise: false,
            background_normalization: false,
            background_radius: None,
            despeckle: DespeckleArg::Off,
        }
    }

    #[test]
    fn rejects_an_unsupported_dpi() {
        let mut a = args(MethodArg::Otsu);
        a.dpi = 150;
        assert!(a.to_settings().unwrap_err().contains("unsupported --dpi"));
    }

    #[test]
    fn rejects_threshold_without_manual_mode() {
        let mut a = args(MethodArg::Otsu);
        a.threshold = Some(128);
        assert!(a.to_settings().unwrap_err().contains("--threshold"));
    }

    #[test]
    fn requires_threshold_for_manual_mode() {
        let a = args(MethodArg::Manual);
        assert!(a
            .to_settings()
            .unwrap_err()
            .contains("requires --threshold"));
    }

    #[test]
    fn rejects_sauvola_parameters_without_sauvola_mode() {
        let mut a = args(MethodArg::Otsu);
        a.sauvola_k = Some(0.3);
        assert!(a.to_settings().unwrap_err().contains("--sauvola-k"));

        let mut a = args(MethodArg::Manual);
        a.threshold = Some(100);
        a.sauvola_window = Some(15);
        assert!(a.to_settings().unwrap_err().contains("--sauvola-window"));
    }

    #[test]
    fn rejects_an_even_sauvola_window() {
        let mut a = args(MethodArg::Sauvola);
        a.sauvola_window = Some(16);
        assert!(a.to_settings().unwrap_err().contains("odd"));
    }

    #[test]
    fn rejects_out_of_range_contrast() {
        let mut a = args(MethodArg::Otsu);
        a.contrast = 2.0;
        assert!(a.to_settings().unwrap_err().contains("--contrast"));
    }

    #[test]
    fn rejects_background_radius_without_normalization() {
        let mut a = args(MethodArg::Otsu);
        a.background_radius = Some(20);
        assert!(a
            .to_settings()
            .unwrap_err()
            .contains("--background-normalization"));
    }

    #[test]
    fn sauvola_window_defaults_come_from_the_core() {
        let a = args(MethodArg::Sauvola);
        let settings = a.to_settings().unwrap();
        match settings.method {
            BinarizationMethod::Sauvola(p) => {
                assert_eq!(p.window_size, sauvola_window_for_dpi(400));
                assert_eq!(p.k, SauvolaParams::default().k);
                assert_eq!(p.dynamic_range, SauvolaParams::default().dynamic_range);
            }
            other => panic!("unexpected method: {other:?}"),
        }
    }

    #[test]
    fn accepted_settings_pass_core_validation() {
        for method in [MethodArg::Otsu, MethodArg::Sauvola] {
            let settings = args(method).to_settings().unwrap();
            assert!(settings.validate().is_ok());
        }
        let mut manual = args(MethodArg::Manual);
        manual.threshold = Some(128);
        assert!(manual.to_settings().unwrap().validate().is_ok());
    }

    #[test]
    fn every_supported_dpi_is_accepted() {
        for dpi in SUPPORTED_DPI {
            let mut a = args(MethodArg::Otsu);
            a.dpi = dpi;
            assert!(a.to_settings().is_ok(), "dpi {dpi} should be accepted");
        }
    }

    #[test]
    fn path_mode_arg_maps_to_the_core_path_mode() {
        assert_eq!(PathMode::from(PathModeArg::Basename), PathMode::Basename);
        assert_eq!(PathMode::from(PathModeArg::Relative), PathMode::Relative);
        assert_eq!(PathMode::from(PathModeArg::Absolute), PathMode::Absolute);
        assert_eq!(PathMode::from(PathModeArg::Omit), PathMode::Omit);
    }

    #[test]
    fn cli_parses_without_a_subcommand() {
        use clap::Parser;
        let cli = Cli::parse_from(["mpdf"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn cli_help_does_not_panic() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn package_commands_parse_the_documented_arguments() {
        use clap::Parser;
        let create = Cli::try_parse_from([
            "mpdf", "package", "create", "book.pdf", "--output", "book.mdp",
        ])
        .unwrap();
        assert!(matches!(
            create.command,
            Some(Command::Package(PackageCommand::Create(_)))
        ));
        let validate = Cli::try_parse_from(["mpdf", "package", "validate", "book.mdp"]).unwrap();
        assert!(matches!(
            validate.command,
            Some(Command::Package(PackageCommand::Validate(_)))
        ));
    }

    #[test]
    fn derived_export_review_and_revision_commands_parse() {
        use clap::Parser;
        let export = Cli::try_parse_from([
            "mpdf",
            "export",
            "book.mdp",
            "--output",
            "book.html",
            "--format",
            "html",
        ])
        .unwrap();
        assert!(matches!(export.command, Some(Command::Export(_))));
        let review = Cli::try_parse_from(["mpdf", "review", "book.mdp", "--json"]).unwrap();
        assert!(matches!(review.command, Some(Command::Review(_))));
        let revision = Cli::try_parse_from([
            "mpdf",
            "revision",
            "add",
            "book.mdp",
            "--target-ref",
            "word-abc",
            "--base-evidence-digest",
            &"a".repeat(64),
            "--text",
            "corrected",
            "--revision-id",
            "r1",
        ])
        .unwrap();
        assert!(matches!(revision.command, Some(Command::Revision(_))));
    }

    #[test]
    fn job_development_commands_parse_the_documented_arguments() {
        use clap::Parser;
        let create = Cli::try_parse_from([
            "mpdf",
            "job",
            "create",
            "--db",
            "jobs.sqlite",
            "--job-id",
            "demo",
            "--pages",
            "500",
        ])
        .unwrap();
        assert!(matches!(
            create.command,
            Some(Command::Job(JobCommand::Create(_)))
        ));
        let status = Cli::try_parse_from([
            "mpdf",
            "job",
            "status",
            "--db",
            "jobs.sqlite",
            "--job-id",
            "demo",
        ])
        .unwrap();
        assert!(matches!(
            status.command,
            Some(Command::Job(JobCommand::Status(_)))
        ));
        let cancel = Cli::try_parse_from([
            "mpdf",
            "job",
            "cancel",
            "--db",
            "jobs.sqlite",
            "--job-id",
            "demo",
        ])
        .unwrap();
        assert!(matches!(
            cancel.command,
            Some(Command::Job(JobCommand::Cancel(_)))
        ));
    }

    #[test]
    fn local_ocr_command_parses_provider_and_job_arguments() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "mpdf",
            "ocr",
            "book.pdf",
            "--output",
            "book.mdp",
            "--jobs-db",
            "jobs.sqlite",
            "--job-id",
            "demo",
            "--provider",
            "reference",
        ])
        .unwrap();
        assert!(matches!(cli.command, Some(Command::Ocr(_))));

        let default_cli = Cli::try_parse_from([
            "mpdf",
            "ocr",
            "book.pdf",
            "--output",
            "book.mdp",
            "--jobs-db",
            "jobs.sqlite",
            "--job-id",
            "demo",
        ])
        .unwrap();
        match default_cli.command {
            Some(Command::Ocr(args)) => assert!(matches!(args.provider, OcrProviderArg::Rapidocr)),
            _ => panic!("expected OCR command"),
        }
    }
}
