use crate::cli::{
    BookmarkEditArgs, BookmarkGenerateArgs, BookmarkListArgs, BookmarkMutationArgs,
    BookmarkReparentArgs,
};
use crate::errors::{self, ExitReason};
use crate::output;
use mpdf_core::bookmarks::{self, ReviewAction};
use mpdf_core::derived::DerivedDocument;
use mpdf_core::document_package::DocumentPackage;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

pub fn generate(a: BookmarkGenerateArgs) -> ExitCode {
    let p = match DocumentPackage::read_from(&a.input) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let d = match load_derived(&a.input, &p) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let s = match bookmarks::generate(&p, d.as_ref()) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    if let Err(e) = bookmarks::save_snapshot(&a.input, &s, a.overwrite) {
        return fail(&e);
    };
    if a.output_mode.json {
        output::print_json(&s, a.output_mode.pretty)
    } else if !a.output_mode.quiet {
        println!("generated {} bookmark candidate(s)", s.candidates.len())
    }
    ExitReason::Success.exit_code()
}
pub fn list(a: BookmarkListArgs) -> ExitCode {
    let s = match bookmarks::load_snapshot(&a.input) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let r = match bookmarks::load_reviews(&a.input, &s) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let e = match bookmarks::effective(&s, &r) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    if a.output_mode.json {
        output::print_json(&e, a.output_mode.pretty)
    } else if !a.output_mode.quiet {
        for c in e {
            println!(
                "{}\t{:?}\t{}\tpage {}",
                c.candidate_id,
                c.status,
                c.effective_title,
                c.physical_page_index + 1
            )
        }
    }
    ExitReason::Success.exit_code()
}
pub fn confirm(a: BookmarkMutationArgs) -> ExitCode {
    mutate(a.input, a.candidate, ReviewAction::Confirm, a.output_mode)
}
pub fn reject(a: BookmarkMutationArgs) -> ExitCode {
    mutate(a.input, a.candidate, ReviewAction::Reject, a.output_mode)
}
pub fn edit(a: BookmarkEditArgs) -> ExitCode {
    mutate(
        a.input,
        a.candidate,
        ReviewAction::Edit { title: a.title },
        a.output_mode,
    )
}
pub fn reparent(a: BookmarkReparentArgs) -> ExitCode {
    mutate(
        a.input,
        a.candidate,
        ReviewAction::Reparent {
            parent_id: a.parent,
            level: a.level,
        },
        a.output_mode,
    )
}
fn mutate(
    root: std::path::PathBuf,
    id: String,
    action: ReviewAction,
    o: crate::cli::OutputArgs,
) -> ExitCode {
    let s = match bookmarks::load_snapshot(&root) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let mut r = match bookmarks::load_reviews(&root, &s) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    if let Err(e) = bookmarks::append(&s, &mut r, id, action) {
        return fail(&e);
    };
    if let Err(e) = bookmarks::save_reviews(&root, &r) {
        return fail(&e);
    };
    if o.json {
        output::print_json(&r, o.pretty)
    } else if !o.quiet {
        println!("bookmark review appended")
    }
    ExitReason::Success.exit_code()
}
fn load_derived(
    root: &Path,
    p: &DocumentPackage,
) -> Result<Option<DerivedDocument>, mpdf_core::error::CoreError> {
    let d = root.join("ocr");
    match fs::symlink_metadata(&d) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(mpdf_core::error::CoreError::io(&d, e)),
        Ok(m) if !m.is_dir() || m.file_type().is_symlink() => Err(
            mpdf_core::error::CoreError::InvalidDocument("OCR directory is unsafe".into()),
        ),
        Ok(_) => {
            let o = mpdf_core::ocr::read_ocr_records(root)
                .map_err(|e| mpdf_core::error::CoreError::InvalidDocument(e.to_string()))?;
            let mut x = DerivedDocument::from_package(p, Some(&o))?;
            let rs = mpdf_core::derived::load_revisions(root)?;
            x.apply_revisions(&rs)?;
            Ok(Some(x))
        }
    }
}
fn fail(e: &mpdf_core::error::CoreError) -> ExitCode {
    eprintln!("error: {}", e);
    errors::classify(e).1.exit_code()
}
