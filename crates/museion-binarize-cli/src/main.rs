//! Command-line interface for Museion Binarize.
//!
//! This is the Milestone 2 harness: enough command surface to exercise and
//! verify the end-to-end PDF pipeline. The full command set (including
//! `analyze` and JSON reports) is Milestone 3.
//!
//! Convention: progress goes to stderr, the final report to stdout, and a
//! failure or cancellation exits non-zero.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use museion_binarize_core::binarization::{sauvola_window_for_dpi, SauvolaParams};
use museion_binarize_core::cleanup::{CleanupSettings, DespeckleLevel};
use museion_binarize_core::error::CoreError;
use museion_binarize_core::page_geometry::points_to_pixels;
use museion_binarize_core::pdfium_backend::PdfiumConfig;
use museion_binarize_core::pipeline::{self, PdfProcessingOptions};
use museion_binarize_core::progress::{ProgressEvent, ProgressReporter};
use museion_binarize_core::renderer::PdfOpenOptions;
use museion_binarize_core::settings::{
    BinarizationMethod, PreprocessingSettings, ProcessingSettings, SUPPORTED_DPI,
};
use museion_binarize_core::validation::ValidationMode;
use museion_binarize_core::ProjectInfo;

/// Environment variable through which a password may be supplied, so that
/// it never appears in a command line, shell history, or process listing.
const PASSWORD_ENV: &str = "MUSEION_PDF_PASSWORD";

#[derive(Parser)]
#[command(name = "museion-binarize", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print project information (name and current development phase).
    Info,
    /// Inspect a PDF: page count, geometry, rotation, and render sizes.
    Inspect(InspectArgs),
    /// Convert a PDF into a bilevel CCITT Group 4 PDF.
    Process(ProcessArgs),
    /// Render and process one page, saving a PNG preview.
    Preview(PreviewArgs),
}

#[derive(Args)]
struct InspectArgs {
    /// Input PDF.
    input: PathBuf,
    /// Path to a PDFium dynamic library.
    #[arg(long)]
    pdfium_library: Option<PathBuf>,
    /// Allow loading PDFium from the system library search path.
    #[arg(long)]
    allow_system_pdfium: bool,
}

#[derive(Args)]
struct ProcessArgs {
    /// Input PDF.
    input: PathBuf,
    /// Destination PDF.
    #[arg(long, short)]
    output: PathBuf,

    #[command(flatten)]
    settings: SettingsArgs,

    /// Replace the destination if it already exists.
    #[arg(long)]
    overwrite: bool,
    /// How thoroughly to validate the finished output.
    #[arg(long, value_enum, default_value_t = ValidateArg::Structural)]
    validate: ValidateArg,
    /// Path to a PDFium dynamic library.
    #[arg(long)]
    pdfium_library: Option<PathBuf>,
    /// Allow loading PDFium from the system library search path.
    #[arg(long)]
    allow_system_pdfium: bool,
}

#[derive(Args)]
struct PreviewArgs {
    /// Input PDF.
    input: PathBuf,
    /// One-based page number to preview.
    #[arg(long)]
    page: u32,
    /// Destination PNG.
    #[arg(long, short)]
    output: PathBuf,

    #[command(flatten)]
    settings: SettingsArgs,

    /// Replace the destination if it already exists.
    #[arg(long)]
    overwrite: bool,
    /// Path to a PDFium dynamic library.
    #[arg(long)]
    pdfium_library: Option<PathBuf>,
    /// Allow loading PDFium from the system library search path.
    #[arg(long)]
    allow_system_pdfium: bool,
}

/// Processing parameters shared by `process` and `preview`.
///
/// Every default here comes from the core (`ProcessingSettings`,
/// `PreprocessingSettings`, `CleanupSettings`, `SauvolaParams`) rather than
/// being reinvented in the CLI.
#[derive(Args)]
struct SettingsArgs {
    /// Output resolution.
    #[arg(long, default_value_t = 400)]
    dpi: u16,
    /// Binarization method.
    #[arg(long, value_enum, default_value_t = MethodArg::Sauvola)]
    method: MethodArg,
    /// Fixed threshold for `--method manual`.
    #[arg(long)]
    threshold: Option<u8>,
    /// Sauvola sensitivity constant.
    #[arg(long)]
    sauvola_k: Option<f32>,
    /// Sauvola window size (odd). Defaults to a value derived from DPI.
    #[arg(long)]
    sauvola_window: Option<u32>,
    /// Grayscale contrast adjustment, -1.0 to 1.0.
    #[arg(long, default_value_t = 0.0)]
    contrast: f32,
    /// Enable the 3x3 median denoise preprocessing step.
    #[arg(long)]
    median_denoise: bool,
    /// Enable background normalization.
    #[arg(long)]
    background_normalization: bool,
    /// Box-filter radius used by background normalization.
    #[arg(long)]
    background_radius: Option<u32>,
    /// Despeckle cleanup level.
    #[arg(long, value_enum, default_value_t = DespeckleArg::Off)]
    despeckle: DespeckleArg,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum MethodArg {
    Otsu,
    Sauvola,
    Manual,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum DespeckleArg {
    Off,
    Conservative,
    Strong,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum ValidateArg {
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
    fn to_settings(&self) -> Result<ProcessingSettings, String> {
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
                    .unwrap_or_else(|| sauvola_window_for_dpi(self.dpi)),
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

fn pdfium_config(path: Option<PathBuf>, allow_system: bool) -> PdfiumConfig {
    PdfiumConfig {
        library_path: path,
        allow_system_library: allow_system,
    }
}

/// Password read from the environment, never from a command-line flag.
fn password_from_env() -> Option<String> {
    std::env::var(PASSWORD_ENV).ok().filter(|p| !p.is_empty())
}

/// Writes progress lines to stderr.
struct StderrProgress;

impl ProgressReporter for StderrProgress {
    fn report(&self, event: ProgressEvent) {
        let mut err = std::io::stderr();
        let _ = match event {
            ProgressEvent::Started { total_pages } => {
                writeln!(err, "Processing {total_pages} page(s)...")
            }
            ProgressEvent::PageStarted { page } => write!(err, "  page {page}: "),
            ProgressEvent::StageChanged { stage, .. } => write!(err, "{stage:?} "),
            ProgressEvent::PageFinished {
                compressed_bytes, ..
            } => writeln!(err, "-> {compressed_bytes} bytes"),
            ProgressEvent::Validating => writeln!(err, "Validating output..."),
            ProgressEvent::Finished => writeln!(err, "Done."),
            ProgressEvent::Cancelled => writeln!(err, "Cancelled."),
        };
        let _ = err.flush();
    }

    fn is_cancelled(&self) -> bool {
        false
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Info) | None => {
            print_info();
            Ok(())
        }
        Some(Command::Inspect(args)) => run_inspect(args),
        Some(Command::Process(args)) => run_process(args),
        Some(Command::Preview(args)) => run_preview(args),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn print_info() {
    let info = ProjectInfo::current();
    println!("{} — {}", info.name, info.phase);
    println!("PDF conversion: inspect, process, preview (see --help).");
    println!("Desktop GUI is not connected yet; see docs/limitations.md.");
}

fn run_inspect(args: InspectArgs) -> Result<(), String> {
    let options = PdfOpenOptions {
        password: password_from_env(),
        pdfium: pdfium_config(args.pdfium_library, args.allow_system_pdfium),
    };
    let library = pipeline::describe_pdfium_library(&options.pdfium).map_err(describe_error)?;
    let info = pipeline::inspect_pdf(&args.input, &options).map_err(describe_error)?;

    println!("File:        {}", args.input.display());
    println!("Source size: {} bytes", info.source_bytes);
    println!("Pages:       {}", info.page_count);
    println!("PDFium:      {library}");
    if let Some(title) = &info.metadata.title {
        println!("Title:       {title}");
    }
    if let Some(author) = &info.metadata.author {
        println!("Author:      {author}");
    }
    println!();

    for page in &info.pages {
        let g = &page.geometry;
        let (w, h) = (g.display_width_points(), g.display_height_points());
        println!(
            "Page {}: {:.2} x {:.2} pt ({:.1} x {:.1} mm), rotation {}°",
            page.page_number(),
            w,
            h,
            w * 25.4 / 72.0,
            h * 25.4 / 72.0,
            g.rotation.degrees()
        );
        for dpi in SUPPORTED_DPI {
            match (points_to_pixels(w, dpi), points_to_pixels(h, dpi)) {
                (Ok(pw), Ok(ph)) => println!("    {dpi} DPI: {pw} x {ph} px"),
                _ => println!("    {dpi} DPI: unavailable (page geometry out of range)"),
            }
        }
    }
    Ok(())
}

fn run_process(args: ProcessArgs) -> Result<(), String> {
    let settings = args.settings.to_settings()?;
    let options = PdfProcessingOptions {
        password: password_from_env(),
        overwrite: args.overwrite,
        validation: args.validate.into(),
        pdfium: pdfium_config(args.pdfium_library, args.allow_system_pdfium),
    };

    let report = pipeline::process_pdf(
        &args.input,
        &args.output,
        &settings,
        &options,
        &StderrProgress,
    )
    .map_err(describe_error)?;

    println!("Output:      {}", args.output.display());
    println!("Pages:       {}", report.pages_processed);
    println!("Original:    {} bytes", report.original_bytes);
    println!("Output size: {} bytes", report.output_bytes);
    if let Some(reduction) = report.reduction_percent() {
        println!("Reduction:   {reduction:.1}%");
    }
    if let Some(average) = report.average_bytes_per_page() {
        println!("Per page:    {average:.0} bytes");
    }
    println!("Elapsed:     {:.2}s", report.elapsed.as_secs_f64());
    println!("PDFium:      {}", report.pdfium_library);
    Ok(())
}

fn run_preview(args: PreviewArgs) -> Result<(), String> {
    let settings = args.settings.to_settings()?;
    if args.page == 0 {
        return Err("page numbers are one-based; --page 0 does not exist".to_string());
    }
    if args.output.exists() && !args.overwrite {
        return Err(format!(
            "{} already exists; pass --overwrite to replace it",
            args.output.display()
        ));
    }

    let options = PdfOpenOptions {
        password: password_from_env(),
        pdfium: pdfium_config(args.pdfium_library, args.allow_system_pdfium),
    };
    let preview = pipeline::preview_page(&args.input, args.page, &settings, &options)
        .map_err(describe_error)?;
    preview.save(&args.output).map_err(|e| {
        format!(
            "could not write the preview to {}: {e}",
            args.output.display()
        )
    })?;

    println!("Preview:     {}", args.output.display());
    println!("Page:        {}", args.page);
    println!("Size:        {} x {} px", preview.width(), preview.height());
    Ok(())
}

/// Turns a core error into a user-facing message. Cancellation is reported
/// distinctly so scripts can tell it apart from a genuine failure.
fn describe_error(error: CoreError) -> String {
    match error {
        CoreError::Cancelled => "processing was cancelled; no output was written".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn cancellation_is_described_distinctly() {
        assert!(describe_error(CoreError::Cancelled).contains("cancelled"));
    }
}
