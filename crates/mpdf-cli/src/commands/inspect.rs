use std::process::ExitCode;

use mpdf_core::document::PdfDocumentInfo;
use mpdf_core::document_session::PdfOpenOptions;
use mpdf_core::page_geometry::points_to_pixels;
use mpdf_core::pipeline;
use mpdf_core::report::{display_path, PathMode, ReportEnvelope};
use mpdf_core::settings::SUPPORTED_DPI;
use serde::Serialize;

use crate::cli::InspectArgs;
use crate::errors::{self, ExitReason};
use crate::output;

pub const INSPECT_REPORT_SCHEMA: &str = "mpdf-inspect";
pub const INSPECT_REPORT_SCHEMA_VERSION: &str = "1.0";

#[derive(Serialize)]
struct InspectReport {
    source_path: Option<String>,
    source_bytes: u64,
    page_count: u32,
    title: Option<String>,
    author: Option<String>,
    subject: Option<String>,
    keywords: Option<String>,
    pdfium_library: String,
    pages: Vec<PageReport>,
}

#[derive(Serialize)]
struct PageReport {
    page_number: u32,
    width_points: f32,
    height_points: f32,
    source_rotation_degrees: u32,
    render_sizes: Vec<RenderSize>,
}

#[derive(Serialize)]
struct RenderSize {
    dpi: u16,
    pixel_width: Option<u32>,
    pixel_height: Option<u32>,
}

pub fn run(args: InspectArgs) -> ExitCode {
    let options = PdfOpenOptions {
        password: super::password_from_env(),
        pdfium: args.pdfium.to_config(),
        compute_source_hash: false,
    };

    let pdfium_library = match pipeline::describe_pdfium_library(&options.pdfium) {
        Ok(d) => d,
        Err(e) => return fail(&e, &args.input, args.output.json, args.output.pretty),
    };
    let info = match pipeline::inspect_pdf(&args.input, &options) {
        Ok(info) => info,
        Err(e) => return fail(&e, &args.input, args.output.json, args.output.pretty),
    };

    let report = build_report(&args.input, &info, pdfium_library, args.path_mode.into());

    if args.output.json {
        output::print_json(&report, args.output.pretty);
    } else if !args.output.quiet {
        print_human(&args.input, &report.result);
    }
    ExitReason::Success.exit_code()
}

fn fail(
    error: &mpdf_core::error::CoreError,
    input: &std::path::Path,
    json: bool,
    pretty: bool,
) -> ExitCode {
    let (_, reason) = errors::classify(error);
    if json {
        let envelope =
            errors::core_error_envelope(error, &[("input", input.display().to_string())]);
        output::print_json(&envelope, pretty);
    } else {
        eprintln!("error: {}", errors::describe_core_error(error));
    }
    reason.exit_code()
}

fn build_report(
    input: &std::path::Path,
    info: &PdfDocumentInfo,
    pdfium_library: String,
    path_mode: PathMode,
) -> ReportEnvelope<InspectReport> {
    let pages = info
        .pages
        .iter()
        .map(|page| {
            let (w, h) = (page.geometry.width_points, page.geometry.height_points);
            let render_sizes = SUPPORTED_DPI
                .iter()
                .map(|&dpi| {
                    let (pw, ph) = match (points_to_pixels(w, dpi), points_to_pixels(h, dpi)) {
                        (Ok(pw), Ok(ph)) => (Some(pw), Some(ph)),
                        _ => (None, None),
                    };
                    RenderSize {
                        dpi,
                        pixel_width: pw,
                        pixel_height: ph,
                    }
                })
                .collect();
            PageReport {
                page_number: page.page_number(),
                width_points: w,
                height_points: h,
                source_rotation_degrees: page.source_rotation.degrees(),
                render_sizes,
            }
        })
        .collect();

    let result = InspectReport {
        source_path: display_path(input, path_mode),
        source_bytes: info.source_bytes,
        page_count: info.page_count,
        title: info.metadata.title.clone(),
        author: info.metadata.author.clone(),
        subject: info.metadata.subject.clone(),
        keywords: info.metadata.keywords.clone(),
        pdfium_library,
        pages,
    };
    ReportEnvelope::new(INSPECT_REPORT_SCHEMA, INSPECT_REPORT_SCHEMA_VERSION, result)
}

fn print_human(input: &std::path::Path, report: &InspectReport) {
    println!("File:        {}", input.display());
    println!("Source size: {} bytes", report.source_bytes);
    println!("Pages:       {}", report.page_count);
    println!("PDFium:      {}", report.pdfium_library);
    if let Some(title) = &report.title {
        println!("Title:       {title}");
    }
    if let Some(author) = &report.author {
        println!("Author:      {author}");
    }
    println!();

    for page in &report.pages {
        println!(
            "Page {}: {:.2} x {:.2} pt ({:.1} x {:.1} mm) visible, source rotation {}°",
            page.page_number,
            page.width_points,
            page.height_points,
            page.width_points * 25.4 / 72.0,
            page.height_points * 25.4 / 72.0,
            page.source_rotation_degrees
        );
        for size in &page.render_sizes {
            match (size.pixel_width, size.pixel_height) {
                (Some(w), Some(h)) => println!("    {} DPI: {w} x {h} px", size.dpi),
                _ => println!(
                    "    {} DPI: unavailable (page geometry out of range)",
                    size.dpi
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpdf_core::document::{PdfMetadata, PdfPageInfo};
    use mpdf_core::page_geometry::{PageGeometry, PageRotation};

    fn sample_info() -> PdfDocumentInfo {
        PdfDocumentInfo {
            page_count: 1,
            pages: vec![PdfPageInfo {
                index: 0,
                geometry: PageGeometry::new(595.0, 842.0).unwrap(),
                source_rotation: PageRotation::None,
            }],
            metadata: PdfMetadata::default(),
            source_bytes: 100,
        }
    }

    #[test]
    fn report_has_the_documented_schema() {
        let report = build_report(
            std::path::Path::new("/a/book.pdf"),
            &sample_info(),
            "test".to_string(),
            PathMode::Basename,
        );
        assert_eq!(report.schema, INSPECT_REPORT_SCHEMA);
        assert_eq!(report.schema_version, INSPECT_REPORT_SCHEMA_VERSION);
    }

    #[test]
    fn path_mode_is_honoured() {
        let report = build_report(
            std::path::Path::new("/a/book.pdf"),
            &sample_info(),
            "test".to_string(),
            PathMode::Omit,
        );
        assert_eq!(report.result.source_path, None);
    }

    #[test]
    fn render_sizes_cover_every_supported_dpi() {
        let report = build_report(
            std::path::Path::new("/a/book.pdf"),
            &sample_info(),
            "test".to_string(),
            PathMode::Basename,
        );
        let dpis: Vec<u16> = report.result.pages[0]
            .render_sizes
            .iter()
            .map(|r| r.dpi)
            .collect();
        assert_eq!(dpis, SUPPORTED_DPI.to_vec());
    }
}
