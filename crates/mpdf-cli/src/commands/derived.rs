//! CLI wiring for deterministic M4 derived exports and revision overlays.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use mpdf_core::derived::{self, DerivedDocument, ExportFormat, RevisionKind, RevisionRecord};
use mpdf_core::document_package::DocumentPackage;
use sha2::{Digest, Sha256};

use crate::cli::{
    ExportArgs, ExportFormatArg, ReviewArgs, RevisionAddArgs, RevisionKindArg, RevisionListArgs,
};
use crate::errors::ExitReason;
use crate::output;

pub fn export(args: ExportArgs) -> ExitCode {
    let document = match load_document(&args.input) {
        Ok(document) => document,
        Err(error) => return fail(&error),
    };
    if matches!(args.format, ExportFormatArg::All) {
        if let Err(error) = write_all_exports(&document, &args.output, args.overwrite) {
            return fail(&error);
        }
    } else {
        let format = args.format.as_core();
        let content = match derived::export(&document, format) {
            Ok(content) => content,
            Err(error) => return fail(&error),
        };
        if let Err(error) = derived::write_export(&args.output, &content, args.overwrite) {
            return fail(&error);
        }
    }
    println!("exported derived document: {}", args.output.display());
    ExitReason::Success.exit_code()
}

pub fn review(args: ReviewArgs) -> ExitCode {
    let document = match load_document(&args.input) {
        Ok(document) => document,
        Err(error) => return fail(&error),
    };
    match derived::review_queue(&document) {
        Ok(issues) => {
            if args.output_mode.json {
                output::print_json(&issues, args.output_mode.pretty);
            } else {
                println!("{} review issue(s)", issues.len());
                for issue in issues {
                    println!(
                        "{:?}: page {} — {}",
                        issue.kind,
                        issue.page_index + 1,
                        issue.reason
                    );
                }
            }
            ExitReason::Success.exit_code()
        }
        Err(error) => fail(&error),
    }
}

pub fn revision_add(args: RevisionAddArgs) -> ExitCode {
    let mut document = match load_document(&args.input) {
        Ok(document) => document,
        Err(error) => return fail(&error),
    };
    let mut store = match derived::load_revisions(&args.input) {
        Ok(store) => store,
        Err(error) => return fail(&error),
    };
    let kind = match args.kind {
        RevisionKindArg::Human => RevisionKind::Human,
        RevisionKindArg::AiSuggested => RevisionKind::AiSuggested,
    };
    let revision_id = args.revision_id.unwrap_or_else(|| {
        derived::deterministic_revision_id(
            &args.target_ref,
            &args.base_evidence_digest,
            kind,
            &args.text,
        )
    });
    store.revisions.push(RevisionRecord {
        revision_id,
        target_ref: args.target_ref,
        kind,
        text: args.text,
        base_evidence_digest: args.base_evidence_digest,
    });
    if let Err(error) = document.apply_revisions(&store) {
        return fail(&error);
    }
    if let Err(error) = derived::save_revisions(&args.input, &store) {
        return fail(&error);
    }
    println!("revision appended");
    ExitReason::Success.exit_code()
}

pub fn revision_list(args: RevisionListArgs) -> ExitCode {
    match derived::load_revisions(&args.input) {
        Ok(store) => {
            if args.output_mode.json {
                output::print_json(&store, args.output_mode.pretty);
            } else {
                for revision in store.revisions {
                    println!(
                        "{}\t{:?}\t{}",
                        revision.revision_id, revision.kind, revision.target_ref
                    );
                }
            }
            ExitReason::Success.exit_code()
        }
        Err(error) => fail(&error),
    }
}

fn load_document(root: &Path) -> Result<DerivedDocument, mpdf_core::error::CoreError> {
    let package = DocumentPackage::read_from(root)?;
    let ocr_dir = root.join("ocr");
    let ocr = match fs::symlink_metadata(&ocr_dir) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Some(
            mpdf_core::ocr::read_ocr_records(root)
                .map_err(|error| mpdf_core::error::CoreError::InvalidDocument(error.to_string()))?,
        ),
        Ok(_) => {
            return Err(mpdf_core::error::CoreError::InvalidDocument(
                "OCR directory is unsafe".into(),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(mpdf_core::error::CoreError::io(&ocr_dir, error)),
    };
    let mut document = DerivedDocument::from_package(&package, ocr.as_ref())?;
    let revisions = derived::load_revisions(root)?;
    document.apply_revisions(&revisions)?;
    Ok(document)
}

fn formats() -> [(ExportFormat, &'static str); 7] {
    [
        (ExportFormat::Json, "json"),
        (ExportFormat::Jsonl, "jsonl"),
        (ExportFormat::Markdown, "md"),
        (ExportFormat::Text, "txt"),
        (ExportFormat::Html, "html"),
        (ExportFormat::Hocr, "hocr.html"),
        (ExportFormat::Alto, "alto.xml"),
    ]
}

fn fail(error: &mpdf_core::error::CoreError) -> ExitCode {
    eprintln!("error: {error}");
    crate::errors::classify(error).1.exit_code()
}

fn write_all_exports(
    document: &DerivedDocument,
    output: &Path,
    overwrite: bool,
) -> Result<(), mpdf_core::error::CoreError> {
    if let Ok(metadata) = fs::symlink_metadata(output) {
        if metadata.file_type().is_symlink() {
            return Err(mpdf_core::error::CoreError::DestinationConflict(
                "export directory must not be a symlink".into(),
            ));
        }
        if !overwrite {
            return Err(mpdf_core::error::CoreError::DestinationConflict(format!(
                "destination already exists: {}",
                output.display()
            )));
        }
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| mpdf_core::error::CoreError::io(parent, error))?;
    let temporary = tempfile::tempdir_in(parent)
        .map_err(|error| mpdf_core::error::CoreError::io(parent, error))?;
    let mut artifacts = Vec::new();
    for (format, extension) in formats() {
        let content = derived::export(document, format)?;
        let filename = format!("derived.{extension}");
        let path = temporary.path().join(&filename);
        derived::write_export(&path, &content, false)?;
        let bytes =
            fs::read(&path).map_err(|error| mpdf_core::error::CoreError::io(&path, error))?;
        artifacts.push(mpdf_core::derived::DerivedArtifact {
            format: extension.into(),
            path: filename,
            sha256: digest(&bytes),
            byte_len: bytes.len() as u64,
        });
    }
    let mut manifest = document.manifest.clone();
    manifest.artifacts = artifacts;
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| mpdf_core::error::CoreError::InvalidDocument(error.to_string()))?;
    derived::write_export(
        &temporary.path().join("derived-manifest.json"),
        &format!("{manifest_json}\n"),
        false,
    )?;
    install_directory_atomically(temporary.path(), output)?;
    Ok(())
}

fn install_directory_atomically(
    temporary: &Path,
    output: &Path,
) -> Result<(), mpdf_core::error::CoreError> {
    // Rename the old directory out of the way first, then install the fully
    // synced temporary tree. If installation fails, restore the old tree.
    // This prevents a failed overwrite from leaving a mixed-generation bundle.
    if fs::symlink_metadata(output).is_err() {
        return fs::rename(temporary, output)
            .map_err(|error| mpdf_core::error::CoreError::io(output, error));
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let backup = tempfile::Builder::new()
        .prefix(".mpdf-derived-backup-")
        .tempdir_in(parent)
        .map_err(|error| mpdf_core::error::CoreError::io(parent, error))?;
    let backup_path = backup.path().to_path_buf();
    // tempdir itself is an existing directory; remove it so rename can use it
    // as a unique sibling path without ever selecting a user path.
    fs::remove_dir(&backup_path)
        .map_err(|error| mpdf_core::error::CoreError::io(&backup_path, error))?;
    fs::rename(output, &backup_path)
        .map_err(|error| mpdf_core::error::CoreError::io(output, error))?;
    match fs::rename(temporary, output) {
        Ok(()) => {
            let _ = fs::remove_dir_all(&backup_path);
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup_path, output);
            Err(mpdf_core::error::CoreError::io(output, error))
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
