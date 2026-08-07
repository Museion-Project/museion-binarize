//! `render_preview`: original or processed page rendering, for both the
//! thumbnail sidebar and the main preview pane.
//!
//! Images cross the IPC boundary as base64-encoded PNG bytes in the
//! command's own return value — never as a filesystem path and never as
//! a raw pixel array. See `docs/desktop.md`, "Image transfer strategy",
//! for why: it needs no temp-file cleanup, exposes no path the frontend
//! could otherwise walk, and a preview-sized PNG is small enough that the
//! extra base64/JSON overhead is not worth avoiding for this milestone.

use base64::Engine;
use tauri::State;

use crate::dto::{PreviewRequestDto, PreviewResultDto, UiErrorDto};
use crate::errors::{classify_core_error, request_error};
use crate::settings::to_processing_settings;
use crate::state::AppState;
use crate::worker::WorkerCommand;

#[tauri::command]
pub async fn render_preview(
    request: PreviewRequestDto,
    state: State<'_, AppState>,
) -> Result<PreviewResultDto, UiErrorDto> {
    {
        let document = state.document.lock().unwrap();
        match document.as_ref() {
            Some(doc) if doc.document_id == request.document_id => {}
            Some(_) => {
                return Err(request_error(
                    "document_stale",
                    "that document is no longer open",
                ))
            }
            None => {
                return Err(request_error(
                    "document_not_open",
                    "no document is currently open",
                ))
            }
        }
    }

    if request.page_number == 0 {
        return Err(request_error(
            "invalid_page",
            "page numbers are one-based; page 0 does not exist",
        ));
    }
    let page_index = request.page_number - 1;

    let processed = match request.kind.as_str() {
        "original" => None,
        "processed" => {
            let settings_dto = request.settings.as_ref().ok_or_else(|| {
                request_error(
                    "invalid_request",
                    "settings are required when kind is \"processed\"",
                )
            })?;
            let settings = to_processing_settings(settings_dto)
                .map_err(|message| request_error("invalid_settings", message))?;
            Some(settings)
        }
        other => {
            return Err(request_error(
                "invalid_request",
                format!("unknown preview kind \"{other}\""),
            ))
        }
    };

    let dpi = request.dpi;
    let rendered = state
        .worker
        .call(move |reply| WorkerCommand::RenderPage {
            page_index,
            dpi,
            processed,
            reply,
        })
        .await
        .map_err(|e| classify_core_error(&e))?;

    let (final_image, is_reduced) = match request.max_dimension {
        Some(max_dimension) => downscale(rendered.image, max_dimension),
        None => (rendered.image, false),
    };

    let png_bytes = encode_png(&final_image)
        .map_err(|e| request_error("image_error", format!("could not encode preview: {e}")))?;

    Ok(PreviewResultDto {
        request_id: request.request_id,
        page_number: request.page_number,
        kind: request.kind,
        width: final_image.width(),
        height: final_image.height(),
        png_base64: base64::engine::general_purpose::STANDARD.encode(png_bytes),
        render_dpi: dpi,
        is_reduced_resolution: is_reduced,
    })
}

/// Downscales `image` so its longest side is at most `max_dimension`,
/// preserving aspect ratio. A no-op (and `is_reduced_resolution = false`)
/// when the image is already within bounds.
fn downscale(image: image::DynamicImage, max_dimension: u32) -> (image::DynamicImage, bool) {
    let longest = image.width().max(image.height());
    if longest <= max_dimension || max_dimension == 0 {
        return (image, false);
    }
    let scale = f64::from(max_dimension) / f64::from(longest);
    let width = ((f64::from(image.width()) * scale).round() as u32).max(1);
    let height = ((f64::from(image.height()) * scale).round() as u32).max(1);
    (
        image.resize(width, height, image::imageops::FilterType::Triangle),
        true,
    )
}

fn encode_png(image: &image::DynamicImage) -> Result<Vec<u8>, image::ImageError> {
    let mut bytes: Vec<u8> = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )?;
    Ok(bytes)
}
