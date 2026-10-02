//! Decode the actual PDF's text-show operands through its actual font maps.
//!
//! PDFium's text-page API is a layout interpreter: it generates separators,
//! dehyphenates and collapses spaces. It is not a literal string decoder.
//! This reader follows content/Form order and font state independently of
//! the writer. Unsupported representations fail closed. It never consumes
//! an expected-text map, a receipt, or custom metadata as decoded evidence.
use lopdf::{content::Content, Dictionary, Document, Encoding, Object};

use crate::document_session::NativeTextObject;
use crate::error::{CoreError, Result};

const LIMIT: usize = 32 * 1024 * 1024;

fn failure(reason: impl std::fmt::Display) -> CoreError {
    CoreError::OutputValidationFailed(format!("transcription_fidelity:PDF_LITERAL:{reason}"))
}

#[derive(Clone, Default)]
struct TextState {
    font: Option<Dictionary>,
    render_mode: i64,
    in_text: bool,
}

fn stream_data(stream: &lopdf::Stream) -> Result<Vec<u8>> {
    let bytes = if stream.dict.has(b"Filter") {
        stream
            .decompressed_content_with_limit(LIMIT)
            .map_err(failure)?
    } else {
        stream.content.clone()
    };
    if bytes.len() > LIMIT {
        return Err(failure("content_size_limit"));
    }
    Ok(bytes)
}

fn content_bytes(doc: &Document, object: &Object, depth: usize) -> Result<Vec<u8>> {
    if depth > 64 {
        return Err(failure("content_recursion_limit"));
    }
    let object = doc.dereference(object).map_err(failure)?.1;
    match object {
        Object::Stream(stream) => stream_data(stream),
        Object::Array(items) => {
            let mut bytes = Vec::new();
            for item in items {
                let content = content_bytes(doc, item, depth + 1)?;
                if bytes.len() + content.len() + 1 > LIMIT {
                    return Err(failure("content_size_limit"));
                }
                bytes.extend(content);
                bytes.push(b'\n');
            }
            Ok(bytes)
        }
        _ => Err(failure("unsupported_contents_object")),
    }
}

fn resource<'a>(
    doc: &'a Document,
    resources: &'a Dictionary,
    kind: &[u8],
    name: &[u8],
) -> Result<&'a Object> {
    let category = resources
        .get_deref(kind, doc)
        .map_err(failure)?
        .as_dict()
        .map_err(failure)?;
    category.get_deref(name, doc).map_err(failure)
}

fn decode_string(doc: &Document, font: &Dictionary, bytes: &[u8]) -> Result<String> {
    let subtype = font
        .get(b"Subtype")
        .and_then(Object::as_name)
        .map_err(failure)?;
    if subtype == b"Type0" {
        if font
            .get(b"Encoding")
            .and_then(Object::as_name)
            .map_err(failure)?
            != b"Identity-H"
            || !font.has(b"ToUnicode")
            || bytes.len() % 2 != 0
        {
            return Err(failure("unsupported_type0_encoding"));
        }
        // lopdf may fall back on an invalid font encoding. Require an actual
        // parsed Unicode map; never accept its fallback for a Type0 font.
        let Encoding::UnicodeMapEncoding(map) = font
            .get_font_encoding_with_limit(doc, LIMIT)
            .map_err(failure)?
        else {
            return Err(failure("missing_or_invalid_tounicode"));
        };
        let mut result = String::new();
        for code in bytes.chunks_exact(2) {
            let code = u16::from_be_bytes([code[0], code[1]]);
            let units = map
                .get(u32::from(code), 2)
                .ok_or_else(|| failure("unmapped_cid"))?;
            result.push_str(
                &String::from_utf16(&units).map_err(|_| failure("invalid_tounicode_utf16"))?,
            );
        }
        return Ok(result);
    }
    // Source-preserving PDFs may already contain native standard-font text.
    // Decode only explicitly supported standard encodings; arbitrary native
    // fonts without a usable map fail closed rather than being guessed.
    if subtype == b"Type1" {
        const STANDARD_FONTS: &[&[u8]] = &[
            b"Helvetica",
            b"Helvetica-Bold",
            b"Helvetica-Oblique",
            b"Helvetica-BoldOblique",
            b"Times-Roman",
            b"Times-Bold",
            b"Times-Italic",
            b"Times-BoldItalic",
            b"Courier",
            b"Courier-Bold",
            b"Courier-Oblique",
            b"Courier-BoldOblique",
        ];
        let base = font
            .get(b"BaseFont")
            .and_then(Object::as_name)
            .map_err(failure)?;
        if !STANDARD_FONTS.contains(&base) || font.has(b"ToUnicode") {
            return Err(failure("unsupported_native_font"));
        }
        if let Ok(encoding) = font.get(b"Encoding") {
            let name = encoding
                .as_name()
                .map_err(|_| failure("unsupported_native_encoding"))?;
            if ![
                b"StandardEncoding".as_slice(),
                b"WinAnsiEncoding",
                b"MacRomanEncoding",
            ]
            .contains(&name)
            {
                return Err(failure("unsupported_native_encoding"));
            }
        }
        return font
            .get_font_encoding_with_limit(doc, LIMIT)
            .map_err(failure)?
            .bytes_to_string(bytes)
            .map_err(failure);
    }
    Err(failure("unsupported_font_subtype"))
}

fn show(
    doc: &Document,
    state: &TextState,
    bytes: &[u8],
    into: &mut Vec<NativeTextObject>,
) -> Result<()> {
    if !state.in_text || into.len() >= 1_000_000 {
        return Err(failure("text_outside_BT_or_span_limit"));
    }
    let font = state
        .font
        .as_ref()
        .ok_or_else(|| failure("text_without_font"))?;
    into.push(NativeTextObject {
        text: decode_string(doc, font, bytes)?,
        invisible: state.render_mode == 3,
    });
    Ok(())
}

fn walk(
    doc: &Document,
    bytes: &[u8],
    resources: &Dictionary,
    mut state: TextState,
    depth: usize,
    into: &mut Vec<NativeTextObject>,
) -> Result<()> {
    if depth > 64 {
        return Err(failure("form_recursion_limit"));
    }
    let content = Content::decode(bytes).map_err(failure)?;
    let mut stack = Vec::new();
    for op in content.operations {
        let args = &op.operands;
        let arity = |n| {
            if args.len() == n {
                Ok(())
            } else {
                Err(failure(format!("invalid_arity_{}", op.operator)))
            }
        };
        match op.operator.as_str() {
            "q" => {
                arity(0)?;
                stack.push(state.clone());
            }
            "Q" => {
                arity(0)?;
                state = stack.pop().ok_or_else(|| failure("unbalanced_Q"))?;
            }
            "BT" => {
                arity(0)?;
                if state.in_text {
                    return Err(failure("nested_BT"));
                }
                state.in_text = true;
            }
            "ET" => {
                arity(0)?;
                if !state.in_text {
                    return Err(failure("unbalanced_ET"));
                }
                state.in_text = false;
            }
            "Tf" => {
                arity(2)?;
                let name = args[0].as_name().map_err(failure)?;
                args[1].as_float().map_err(failure)?;
                state.font = Some(
                    resource(doc, resources, b"Font", name)?
                        .as_dict()
                        .map_err(failure)?
                        .clone(),
                );
            }
            "Tr" => {
                arity(1)?;
                state.render_mode = args[0].as_i64().map_err(failure)?;
                if !(0..=7).contains(&state.render_mode) {
                    return Err(failure("invalid_render_mode"));
                }
            }
            "Tj" | "'" => {
                arity(1)?;
                show(doc, &state, args[0].as_str().map_err(failure)?, into)?;
            }
            "\"" => {
                arity(3)?;
                args[0].as_float().map_err(failure)?;
                args[1].as_float().map_err(failure)?;
                show(doc, &state, args[2].as_str().map_err(failure)?, into)?;
            }
            "TJ" => {
                arity(1)?;
                let mut bytes = Vec::new();
                for part in args[0].as_array().map_err(failure)? {
                    match part {
                        Object::String(text, _) => bytes.extend_from_slice(text),
                        Object::Integer(_) | Object::Real(_) => {}
                        _ => return Err(failure("unsupported_TJ_operand")),
                    }
                }
                show(doc, &state, &bytes, into)?;
            }
            "Do" => {
                arity(1)?;
                let object = resource(
                    doc,
                    resources,
                    b"XObject",
                    args[0].as_name().map_err(failure)?,
                )?;
                let stream = object.as_stream().map_err(failure)?;
                match stream
                    .dict
                    .get(b"Subtype")
                    .and_then(Object::as_name)
                    .map_err(failure)?
                {
                    b"Image" => {}
                    b"Form" => {
                        let child_resources = if stream.dict.has(b"Resources") {
                            stream
                                .dict
                                .get_deref(b"Resources", doc)
                                .map_err(failure)?
                                .as_dict()
                                .map_err(failure)?
                        } else {
                            resources
                        };
                        walk(
                            doc,
                            &stream_data(stream)?,
                            child_resources,
                            state.clone(),
                            depth + 1,
                            into,
                        )?;
                    }
                    _ => return Err(failure("unsupported_XObject_subtype")),
                }
            }
            "gs" => {
                arity(1)?;
                let dict = resource(
                    doc,
                    resources,
                    b"ExtGState",
                    args[0].as_name().map_err(failure)?,
                )?
                .as_dict()
                .map_err(failure)?;
                if dict.has(b"Font") {
                    return Err(failure("unsupported_ExtGState_font"));
                }
            }
            "BDC" => {
                arity(2)?;
                let props = match &args[1] {
                    Object::Dictionary(dict) => dict,
                    Object::Name(name) => resource(doc, resources, b"Properties", name)?
                        .as_dict()
                        .map_err(failure)?,
                    _ => return Err(failure("unsupported_marked_properties")),
                };
                if props.has(b"ActualText") {
                    return Err(failure("unsupported_ActualText"));
                }
            }
            "BMC" => {
                arity(1)?;
                args[0].as_name().map_err(failure)?;
            }
            "EMC" => {
                arity(0)?;
            }
            // These operators change placement/painting, not encoded text.
            "cm" | "Tm" => {
                arity(6)?;
                for a in args {
                    a.as_float().map_err(failure)?;
                }
            }
            "Td" | "TD" | "m" | "l" => {
                arity(2)?;
                for a in args {
                    a.as_float().map_err(failure)?;
                }
            }
            "Tc" | "Tw" | "Tz" | "TL" | "Ts" | "w" | "J" | "j" | "M" | "i" | "G" | "g" => {
                arity(1)?;
                args[0].as_float().map_err(failure)?;
            }
            "c" => {
                arity(6)?;
                for a in args {
                    a.as_float().map_err(failure)?;
                }
            }
            "v" | "y" | "re" | "K" | "k" => {
                arity(4)?;
                for a in args {
                    a.as_float().map_err(failure)?;
                }
            }
            "RG" | "rg" => {
                arity(3)?;
                for a in args {
                    a.as_float().map_err(failure)?;
                }
            }
            "T*" | "h" | "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" | "W"
            | "W*" => {
                arity(0)?;
            }
            "CS" | "cs" | "ri" => {
                arity(1)?;
                args[0].as_name().map_err(failure)?;
            }
            "SC" | "SCN" | "sc" | "scn" => {
                for arg in args {
                    if !matches!(arg, Object::Integer(_) | Object::Real(_) | Object::Name(_)) {
                        return Err(failure("invalid_color_operand"));
                    }
                }
            }
            "d" => {
                arity(2)?;
                args[0].as_array().map_err(failure)?;
                args[1].as_float().map_err(failure)?;
            }
            _ => return Err(failure(format!("unsupported_operator_{}", op.operator))),
        }
    }
    if state.in_text || !stack.is_empty() {
        return Err(failure("unbalanced_text_or_graphics_state"));
    }
    Ok(())
}

/// Decode all page text in actual content/Form order, without layout-generated
/// characters. Unsupported native fonts, mappings or operators are errors.
pub fn decode_pdf_literal_spans(bytes: &[u8]) -> Result<Vec<Vec<NativeTextObject>>> {
    let doc = Document::load_mem(bytes).map_err(failure)?;
    let mut pages = Vec::new();
    for (_, id) in doc.get_pages() {
        let page = doc.get_dictionary(id).map_err(failure)?;
        let mut current = page;
        let mut resources = None;
        for _ in 0..65 {
            if current.has(b"Resources") {
                resources = Some(
                    current
                        .get_deref(b"Resources", &doc)
                        .map_err(failure)?
                        .as_dict()
                        .map_err(failure)?,
                );
                break;
            }
            match current.get(b"Parent") {
                Ok(parent) => {
                    current = doc
                        .dereference(parent)
                        .map_err(failure)?
                        .1
                        .as_dict()
                        .map_err(failure)?
                }
                Err(_) => break,
            }
        }
        let empty = Dictionary::new();
        let resources = resources.unwrap_or(&empty);
        let mut spans = Vec::new();
        if let Ok(contents) = page.get(b"Contents") {
            walk(
                &doc,
                &content_bytes(&doc, contents, 0)?,
                resources,
                TextState::default(),
                0,
                &mut spans,
            )?;
        }
        pages.push(spans);
    }
    Ok(pages)
}
