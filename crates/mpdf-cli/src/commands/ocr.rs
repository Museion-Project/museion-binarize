//! One-shot local OCR entry point. The reference provider is offline and
//! deterministic; RapidOCR is an explicitly configured executable invoked
//! directly (never through a shell).

use std::fs::File;
use std::io::{BufReader, Read};
use std::process::ExitCode;

use sha2::{Digest, Sha256};

use crate::cli::{OcrArgs, OcrProviderArg};
use crate::errors::ExitReason;
use crate::output;
use mpdf_core::document_package::DocumentPackage;
use mpdf_core::document_session::{PdfDocumentSession, PdfOpenOptions};
use mpdf_core::jobs::JobStore;
use mpdf_core::ocr::{
    self, PageOcrProvider, RapidOcrConfig, RapidOcrProvider, ReferenceOcrProvider,
};

pub fn run(args: OcrArgs) -> ExitCode {
    let options = PdfOpenOptions {
        password: super::password_from_env(),
        pdfium: args.pdfium.to_config(),
        compute_source_hash: true,
    };
    let session = match PdfDocumentSession::open(&args.input, &options) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitReason::InputError.exit_code();
        }
    };
    let mut provider: Box<dyn PageOcrProvider> = match args.provider {
        OcrProviderArg::Reference => Box::new(ReferenceOcrProvider),
        OcrProviderArg::Rapidocr => {
            let Some(executable) = args.provider_executable.clone() else {
                eprintln!("error: --provider-executable is required for rapidocr");
                return ExitReason::UsageError.exit_code();
            };
            let Some(model_dir) = args.model_dir.clone() else {
                eprintln!("error: --model-dir is required for rapidocr");
                return ExitReason::UsageError.exit_code();
            };
            Box::new(RapidOcrProvider::new(RapidOcrConfig {
                executable,
                model_dir,
            }))
        }
    };
    // Create the base package from the already-open session before any page
    // work. OCR never reopens the PDF or rereads the source path.
    let source_name = args
        .input
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned);
    if args.output.exists() {
        match DocumentPackage::read_from(&args.output) {
            Ok(package)
                if package.source.content_sha256
                    == session
                        .source_identity()
                        .content_sha256
                        .clone()
                        .unwrap_or_default()
                    && package.manifest.page_count == session.info().page_count => {}
            Ok(_) => {
                eprintln!("error: existing MDP package does not match this PDF");
                return ExitReason::InputError.exit_code();
            }
            Err(error) => {
                eprintln!("error: existing MDP package is invalid: {error}");
                return ExitReason::InputError.exit_code();
            }
        }
    } else {
        let package = match DocumentPackage::create_from_session(&session, source_name) {
            Ok(package) => package,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitReason::InputError.exit_code();
            }
        };
        if let Err(error) = package.write_to(&args.output) {
            eprintln!("error: {error}");
            return ExitReason::OutputError.exit_code();
        }
    }
    let store = match JobStore::open(&args.jobs_db) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("error: job database: {error}");
            return ExitReason::OutputError.exit_code();
        }
    };
    let provider_name = match args.provider {
        OcrProviderArg::Reference => "reference",
        OcrProviderArg::Rapidocr => "rapidocr",
    };
    let executable_fingerprint = match args.provider {
        OcrProviderArg::Reference => "reference-deterministic-v1".to_owned(),
        OcrProviderArg::Rapidocr => {
            let Some(executable) = args.provider_executable.as_ref() else {
                eprintln!("error: --provider-executable is required for rapidocr");
                return ExitReason::UsageError.exit_code();
            };
            match sha256_file(executable) {
                Ok(digest) => digest,
                Err(error) => {
                    eprintln!("error: RapidOCR executable is unavailable: {error}");
                    return ExitReason::UsageError.exit_code();
                }
            }
        }
    };
    let fingerprint = format!(
        "source={};provider={};dpi={};protocol={}@{};executable={};executable_sha256={};model_dir={};models={}",
        session
            .source_identity()
            .content_sha256
            .as_deref()
            .unwrap_or(""),
        provider_name,
        ocr::CANONICAL_OCR_DPI,
        ocr::OCR_PROTOCOL,
        ocr::OCR_PROTOCOL_VERSION,
        args.provider_executable
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        executable_fingerprint,
        args.model_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        match args.provider {
            OcrProviderArg::Reference => "reference-deterministic-v1".to_owned(),
            OcrProviderArg::Rapidocr => {
                let Some(model_dir) = args.model_dir.as_ref() else {
                    eprintln!("error: --model-dir is required for rapidocr");
                    return ExitReason::UsageError.exit_code();
                };
                let mut digests = Vec::new();
                for name in ocr::RAPIDOCR_MODEL_FILES {
                    let path = model_dir.join(name);
                    match sha256_file(&path) {
                        Ok(digest) => digests.push(format!("{name}:{digest}")),
                        Err(error) => {
                            eprintln!("error: RapidOCR model is unavailable: {error}");
                            return ExitReason::UsageError.exit_code();
                        }
                    }
                }
                digests.join(",")
            }
        },
    );
    let run_result = match ocr::run_session_durable(
        &session,
        provider.as_mut(),
        &store,
        &args.job_id,
        &fingerprint,
        &args.output,
        "mpdf-ocr-cli",
        ocr::CANONICAL_OCR_DPI,
    ) {
        Ok(run) => run,
        Err(mpdf_core::error::CoreError::Cancelled) => {
            eprintln!("OCR cancelled; completed pages were retained");
            return ExitReason::Cancelled.exit_code();
        }
        Err(error) => {
            eprintln!("error: {error}");
            return ExitReason::ProcessingError.exit_code();
        }
    };
    if args.output_mode.json {
        output::print_json(&run_result, args.output_mode.pretty);
    } else if !args.output_mode.quiet {
        println!(
            "Created local OCR MDP extension: {} ({} pages, {} errors)",
            args.output.display(),
            run_result.pages.len(),
            run_result.errors.len()
        );
    }
    if run_result.is_complete(session.info().page_count) {
        ExitReason::Success.exit_code()
    } else {
        ExitReason::ProcessingError.exit_code()
    }
}

fn sha256_file(path: &std::path::Path) -> Result<String, String> {
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
