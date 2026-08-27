//! Low-level source-preserving searchable-PDF editor.
//!
//! The editor uses lopdf's object model (never renders or rasterizes pages),
//! appending one invisible text stream per page and replacing only page
//! dictionaries/resources plus the catalog outline reference.
use crate::bookmarks::{self, BookmarkCandidate};
use crate::derived::DerivedDocument;
use crate::document_package::DocumentPackage;
use crate::error::{CoreError, Result};
use lopdf::{dictionary, Document, Object, ObjectId, Stream, StringFormat};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};

const FONT: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");

struct EmbeddedFont {
    object_id: ObjectId,
    cids: BTreeMap<char, u16>,
    advances: BTreeMap<u16, f64>,
}

#[derive(Clone, Copy)]
struct PagePlacement {
    llx: f64,
    lly: f64,
    visible_width: f64,
    visible_height: f64,
    rotation: u16,
}

pub fn build(
    source: &[u8],
    package: &DocumentPackage,
    candidates: &[BookmarkCandidate],
    derived: Option<&DerivedDocument>,
) -> Result<Vec<u8>> {
    build_with_cancel(source, package, candidates, derived, &|| false)
}

pub fn build_with_cancel(
    source: &[u8],
    package: &DocumentPackage,
    candidates: &[BookmarkCandidate],
    derived: Option<&DerivedDocument>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>> {
    if cancelled() {
        return Err(CoreError::Cancelled);
    }
    package.validate()?;
    if let Some(document) = derived {
        document.validate()?;
        if document.manifest.source_digest != package.source.content_sha256 {
            return Err(CoreError::InvalidDocument(
                "derived document does not match MDP source".into(),
            ));
        }
    }
    let source_digest = Sha256::digest(source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if source_digest != package.source.content_sha256 {
        return Err(CoreError::InvalidDocument(
            "source PDF digest does not match MDP source binding".into(),
        ));
    }
    let mut pdf =
        Document::load_mem(source).map_err(|e| CoreError::PdfConstructionFailed(e.to_string()))?;
    let pages = pdf.get_pages();
    if pages.len() != package.pages.len() {
        return Err(CoreError::InvalidDocument(
            "source PDF page count does not match MDP".into(),
        ));
    }
    for candidate in candidates {
        candidate.validate()?;
        let page = package
            .pages
            .get(candidate.physical_page_index as usize)
            .ok_or_else(|| CoreError::InvalidDocument("bookmark page index is invalid".into()))?;
        if page.page_id != candidate.target_page_id {
            return Err(CoreError::InvalidDocument(
                "bookmark target page and physical index disagree".into(),
            ));
        }
    }
    let mut placements = BTreeMap::new();
    for (one_based, page_id) in &pages {
        if cancelled() {
            return Err(CoreError::Cancelled);
        }
        let package_page = package
            .pages
            .get(*one_based as usize - 1)
            .ok_or_else(|| CoreError::InvalidDocument("MDP page is missing".into()))?;
        placements.insert(*one_based, page_placement(&pdf, *page_id, package_page)?);
    }
    let mut scalars = BTreeSet::new();
    if let Some(d) = derived {
        for p in &d.pages {
            for l in p.blocks.iter().flat_map(|b| b.lines.iter()) {
                for w in &l.words {
                    scalars.extend(w.effective_text.chars());
                }
            }
        }
    }
    let font = add_font(&mut pdf, &scalars)?;
    if let Some(d) = derived {
        for (one, id) in &pages {
            if cancelled() {
                return Err(CoreError::Cancelled);
            }
            let pidx = *one as usize - 1;
            let Some(dp) = d.pages.iter().find(|p| p.page_index == pidx as u32) else {
                continue;
            };
            let placement = placements
                .get(one)
                .ok_or_else(|| CoreError::InvalidDocument("page placement is missing".into()))?;
            let bytes = text_stream(dp, &package.pages[pidx], *placement, &font)?;
            if bytes.is_empty() {
                continue;
            }
            let sid = pdf.add_object(Stream::new(dictionary! {}, bytes));
            add_page_stream(&mut pdf, *id, font.object_id, sid)?;
        }
    }
    let confirmed: Vec<_> = candidates
        .iter()
        .filter(|c| matches!(c.status, bookmarks::BookmarkStatus::Confirmed))
        .collect();
    if !confirmed.is_empty() {
        add_outline(&mut pdf, &pages, &placements, package, &confirmed)?;
    }
    if cancelled() {
        return Err(CoreError::Cancelled);
    }
    let mut out = Vec::new();
    pdf.save_to(&mut out)
        .map_err(|e| CoreError::PdfConstructionFailed(e.to_string()))?;
    Ok(out)
}

fn page_placement(
    pdf: &Document,
    page_id: ObjectId,
    page: &crate::document_package::Page,
) -> Result<PagePlacement> {
    let box_object = match inherited(pdf, page_id, b"CropBox")? {
        Some(value) => value,
        None => inherited(pdf, page_id, b"MediaBox")?
            .ok_or_else(|| CoreError::InvalidDocument("source page has no MediaBox".into()))?,
    };
    let values = box_object
        .as_array()
        .map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
    if values.len() != 4 {
        return Err(CoreError::InvalidDocument(
            "source page box must contain four coordinates".into(),
        ));
    }
    let number = |object: &Object| {
        object
            .as_float()
            .map(f64::from)
            .map_err(|e| CoreError::InvalidDocument(e.to_string()))
    };
    let (llx, lly, urx, ury) = (
        number(&values[0])?,
        number(&values[1])?,
        number(&values[2])?,
        number(&values[3])?,
    );
    let raw_width = urx - llx;
    let raw_height = ury - lly;
    let rotation = inherited(pdf, page_id, b"Rotate")?
        .map(|object| {
            object
                .as_i64()
                .map_err(|e| CoreError::InvalidDocument(e.to_string()))
        })
        .transpose()?
        .unwrap_or(0)
        .rem_euclid(360) as u16;
    if !matches!(rotation, 0 | 90 | 180 | 270)
        || ![llx, lly, raw_width, raw_height]
            .iter()
            .all(|value| value.is_finite())
        || raw_width <= 0.0
        || raw_height <= 0.0
        || rotation != page.rotation_degrees
    {
        return Err(CoreError::InvalidPageGeometry(
            "source page box or rotation does not match the MDP".into(),
        ));
    }
    let (visible_width, visible_height) = if matches!(rotation, 90 | 270) {
        (raw_height, raw_width)
    } else {
        (raw_width, raw_height)
    };
    if (visible_width - page.source_space.width).abs() > 0.05
        || (visible_height - page.source_space.height).abs() > 0.05
    {
        return Err(CoreError::InvalidPageGeometry(
            "source page geometry does not match the MDP".into(),
        ));
    }
    Ok(PagePlacement {
        llx,
        lly,
        visible_width,
        visible_height,
        rotation,
    })
}

fn inherited(pdf: &Document, page_id: ObjectId, key: &[u8]) -> Result<Option<Object>> {
    let mut current = page_id;
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(current) {
            return Err(CoreError::InvalidDocument(
                "cycle in source page tree".into(),
            ));
        }
        let dictionary = pdf
            .get_dictionary(current)
            .map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
        if let Ok(value) = dictionary.get(key) {
            let (_, resolved) = pdf
                .dereference(value)
                .map_err(|e| CoreError::InvalidDocument(e.to_string()))?;
            return Ok(Some(resolved.clone()));
        }
        match dictionary.get(b"Parent").and_then(Object::as_reference) {
            Ok(parent) => current = parent,
            Err(_) => return Ok(None),
        }
    }
}

fn placed_bbox(
    page: &crate::document_package::Page,
    placement: PagePlacement,
    bbox: crate::derived::Bbox,
) -> Result<crate::derived::Bbox> {
    let visible = bookmarks::master_bbox_to_visible_pdf(page, bbox)?;
    let (x, y, width, height) = bookmarks::visible_to_default(
        placement.rotation,
        placement.visible_width,
        placement.visible_height,
        visible.x,
        visible.y,
        visible.width,
        visible.height,
    );
    Ok(crate::derived::Bbox {
        x: x + placement.llx,
        y: y + placement.lly,
        width,
        height,
    })
}

fn add_font(pdf: &mut Document, scalars: &BTreeSet<char>) -> Result<EmbeddedFont> {
    if scalars.len() > 65_534 {
        return Err(CoreError::PdfConstructionFailed(
            "text layer exceeds the 16-bit CID limit".into(),
        ));
    }
    let face = ttf_parser::Face::parse(FONT, 0).map_err(|_| {
        CoreError::PdfConstructionFailed("bundled Noto Sans font is invalid".into())
    })?;
    let units = face.units_per_em() as f32;
    let mut gids = Vec::new();
    let mut widths = Vec::new();
    let mut cids = BTreeMap::new();
    let mut advances = BTreeMap::new();
    for (i, c) in scalars.iter().enumerate() {
        let cid = i as u16 + 1;
        let gid = face.glyph_index(*c).map(|x| x.0).unwrap_or(0);
        let width = face
            .glyph_hor_advance(ttf_parser::GlyphId(gid))
            .unwrap_or(0) as f32
            * 1000.0
            / units;
        gids.push(gid);
        widths.push((i as i64 + 1, width.round() as i64));
        cids.insert(*c, cid);
        advances.insert(cid, f64::from(width));
    }
    let mut cmap = Vec::new();
    cmap.extend_from_slice(b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /MPDFUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
    for chunk in gids.iter().enumerate().collect::<Vec<_>>().chunks(100) {
        cmap.extend_from_slice(format!("{} beginbfchar\n", chunk.len()).as_bytes());
        for (i, _) in chunk {
            let c = scalars.iter().nth(*i).unwrap_or(&' ');
            cmap.extend_from_slice(format!("<{:04X}> <{}>\n", i + 1, utf16_hex(*c)).as_bytes());
        }
        cmap.extend_from_slice(b"endbfchar\n");
    }
    cmap.extend_from_slice(b"endcmap\nCMapName currentdict /CMap defineresource pop\nend end\n");
    let cmap_id = pdf.add_object(Stream::new(dictionary! {}, cmap));
    let mut cid_map = vec![0u8; (gids.len() + 1) * 2];
    for (i, gid) in gids.iter().enumerate() {
        let off = (i + 1) * 2;
        cid_map[off..off + 2].copy_from_slice(&gid.to_be_bytes());
    }
    let cid_map_id = pdf.add_object(Stream::new(dictionary! {}, cid_map));
    let fontfile = pdf.add_object(Stream::new(
        dictionary! {"Length1"=>FONT.len() as i64},
        FONT.to_vec(),
    ));
    let desc=pdf.add_object(dictionary!{"Type"=>"FontDescriptor","FontName"=>"NotoSans","Flags"=>32,"FontBBox"=>vec![(-1000i64).into(),(-500i64).into(),(2000i64).into(),(1200i64).into()],"ItalicAngle"=>0,"Ascent"=>1069,"Descent"=>-293,"CapHeight"=>714,"StemV"=>80,"FontFile2"=>fontfile});
    let mut w = Vec::new();
    for (cid, width) in widths {
        w.push(cid.into());
        w.push(Object::Array(vec![width.into()]));
    }
    let cid_system_info = dictionary! {
        "Registry" => Object::String(b"Adobe".to_vec(), StringFormat::Literal),
        "Ordering" => Object::String(b"Identity".to_vec(), StringFormat::Literal),
        "Supplement" => 0,
    };
    let cidfont=pdf.add_object(dictionary!{"Type"=>"Font","Subtype"=>"CIDFontType2","BaseFont"=>"NotoSans","CIDSystemInfo"=>cid_system_info,"FontDescriptor"=>desc,"CIDToGIDMap"=>cid_map_id,"DW"=>1000,"W"=>Object::Array(w)});
    let object_id=pdf.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type0","BaseFont"=>"NotoSans","Encoding"=>"Identity-H","DescendantFonts"=>vec![cidfont.into()],"ToUnicode"=>cmap_id});
    Ok(EmbeddedFont {
        object_id,
        cids,
        advances,
    })
}
fn utf16_hex(c: char) -> String {
    let mut b = [0u16; 2];
    let n = c.encode_utf16(&mut b).len();
    b[..n].iter().map(|x| format!("{x:04X}")).collect()
}
fn text_stream(
    dp: &crate::derived::DerivedPage,
    page: &crate::document_package::Page,
    placement: PagePlacement,
    font: &EmbeddedFont,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for l in dp.blocks.iter().flat_map(|b| b.lines.iter()) {
        for w in &l.words {
            let b = placed_bbox(page, placement, w.bbox)?;
            let mut cid = Vec::new();
            let mut advance = 0.0;
            for c in w.effective_text.chars() {
                let mapped = font.cids.get(&c).copied().ok_or_else(|| {
                    CoreError::PdfConstructionFailed("text scalar has no CID mapping".into())
                })?;
                cid.extend_from_slice(&mapped.to_be_bytes());
                advance += font.advances.get(&mapped).copied().unwrap_or(0.0) / 1000.0;
            }
            if cid.is_empty() || advance <= 0.0 || b.width <= 0.0 || b.height <= 0.0 {
                continue;
            }
            let horizontal_scale = b.width / advance;
            out.extend_from_slice(
                format!(
                    "BT /MPDFHiddenM5 1 Tf 3 Tr {:.6} 0 0 {:.6} {:.4} {:.4} Tm <{}> Tj ET\n",
                    horizontal_scale,
                    b.height,
                    b.x,
                    b.y,
                    cid.iter().map(|x| format!("{x:02X}")).collect::<String>()
                )
                .as_bytes(),
            );
        }
    }
    Ok(out)
}
fn add_page_stream(
    pdf: &mut Document,
    id: ObjectId,
    font: ObjectId,
    stream: ObjectId,
) -> Result<()> {
    // Materialize the effective inherited resource dictionary on the page
    // before adding our font. Replacing an inherited Resources entry with a
    // font-only dictionary would make the original visible content lose its
    // XObjects, color spaces, or original fonts.
    let mut resources = inherited(pdf, id, b"Resources")?
        .map(|object| {
            object
                .as_dict()
                .cloned()
                .map_err(|e| CoreError::PdfConstructionFailed(e.to_string()))
        })
        .transpose()?
        .unwrap_or_default();
    let mut fonts = match resources.get(b"Font") {
        Ok(value) => pdf
            .dereference(value)
            .map_err(|e| CoreError::PdfConstructionFailed(e.to_string()))?
            .1
            .as_dict()
            .cloned()
            .map_err(|e| CoreError::PdfConstructionFailed(e.to_string()))?,
        Err(_) => lopdf::Dictionary::new(),
    };
    if fonts.has(b"MPDFHiddenM5") {
        return Err(CoreError::PdfConstructionFailed(
            "source resource name MPDFHiddenM5 is already in use".into(),
        ));
    }
    fonts.set("MPDFHiddenM5", font);
    resources.set("Font", fonts);
    pdf.get_dictionary_mut(id)
        .map_err(|e| CoreError::PdfConstructionFailed(e.to_string()))?
        .set("Resources", resources);
    let page = pdf
        .get_dictionary_mut(id)
        .map_err(|e| CoreError::PdfConstructionFailed(e.to_string()))?;
    let old = page.get(b"Contents").ok().cloned();
    page.set(
        "Contents",
        match old {
            Some(Object::Array(mut a)) => {
                a.push(stream.into());
                Object::Array(a)
            }
            Some(x) => Object::Array(vec![x, stream.into()]),
            None => stream.into(),
        },
    );
    Ok(())
}
fn add_outline(
    pdf: &mut Document,
    pages: &BTreeMap<u32, ObjectId>,
    placements: &BTreeMap<u32, PagePlacement>,
    package: &DocumentPackage,
    candidates: &[&BookmarkCandidate],
) -> Result<()> {
    let root_id = pdf.add_object(dictionary! {"Type"=>"Outlines","Count"=>candidates.len() as i64});
    let mut ids = Vec::new();
    let mut by_candidate = std::collections::HashMap::new();
    for c in candidates {
        let pid = *pages
            .get(&(c.physical_page_index + 1))
            .ok_or_else(|| CoreError::InvalidDocument("bookmark target page missing".into()))?;
        let b = c.master_bbox.unwrap_or(crate::derived::Bbox {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });
        let placement = placements
            .get(&(c.physical_page_index + 1))
            .ok_or_else(|| CoreError::InvalidDocument("bookmark page placement missing".into()))?;
        let dest = placed_bbox(
            &package.pages[c.physical_page_index as usize],
            *placement,
            b,
        )?;
        let id=pdf.add_object(dictionary!{"Title"=>utf16_obj(&c.effective_title),"Parent"=>root_id,"Dest"=>vec![pid.into(),"XYZ".into(),dest.x.into(),(dest.y + dest.height).into(),Object::Null]});
        by_candidate.insert(c.candidate_id.as_str(), id);
        ids.push(id);
    }
    // Attach each node to its effective parent and link siblings. The
    // generated candidate is retained as the source of hierarchy; only the
    // effective review overlay controls this PDF tree.
    for (c, id) in candidates.iter().zip(ids.iter()) {
        let parent = c
            .effective_parent_id
            .as_deref()
            .and_then(|p| by_candidate.get(p).copied())
            .unwrap_or(root_id);
        pdf.get_dictionary_mut(*id)
            .map_err(|e| CoreError::PdfConstructionFailed(e.to_string()))?
            .set("Parent", parent);
    }
    let mut children: std::collections::HashMap<ObjectId, Vec<ObjectId>> =
        std::collections::HashMap::new();
    for (c, id) in candidates.iter().zip(ids.iter()) {
        let parent = c
            .effective_parent_id
            .as_deref()
            .and_then(|p| by_candidate.get(p).copied())
            .unwrap_or(root_id);
        children.entry(parent).or_default().push(*id);
    }
    for (parent, list) in &children {
        {
            let d = pdf
                .get_dictionary_mut(*parent)
                .map_err(|e| CoreError::PdfConstructionFailed(e.to_string()))?;
            d.set("First", list[0]);
            d.set("Last", *list.last().unwrap());
            if *parent == root_id {
                d.set("Count", list.len() as i64);
            }
        }
        for (i, id) in list.iter().enumerate() {
            let x = pdf
                .get_dictionary_mut(*id)
                .map_err(|e| CoreError::PdfConstructionFailed(e.to_string()))?;
            if i > 0 {
                x.set("Prev", list[i - 1]);
            }
            if i + 1 < list.len() {
                x.set("Next", list[i + 1]);
            }
        }
    }
    pdf.catalog_mut()
        .map_err(|e| CoreError::PdfConstructionFailed(e.to_string()))?
        .set("Outlines", root_id);
    Ok(())
}
fn utf16_obj(s: &str) -> Object {
    let mut b = vec![0xFE, 0xFF];
    for u in s.encode_utf16() {
        b.extend_from_slice(&u.to_be_bytes());
    }
    Object::String(b, StringFormat::Literal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_package::{AffineTransform, CoordinateSpace, CoordinateUnit, Origin, Page};

    fn package_page() -> Page {
        Page {
            page_id: "page".into(),
            physical_index: 0,
            order: 0,
            rotation_degrees: 90,
            master_space: CoordinateSpace {
                id: "master".into(),
                unit: CoordinateUnit::Pixels,
                width: 360.0,
                height: 160.0,
                origin: Origin::TopLeft,
                pixels_per_inch: Some(72),
            },
            source_space: CoordinateSpace {
                id: "source".into(),
                unit: CoordinateUnit::PdfPoints,
                width: 360.0,
                height: 160.0,
                origin: Origin::BottomLeft,
                pixels_per_inch: None,
            },
            transforms: vec![AffineTransform {
                from_space: "source".into(),
                to_space: "master".into(),
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: -1.0,
                e: 0.0,
                f: 160.0,
            }],
            printed_page_label: None,
            existing_outline_evidence: vec![],
            typography_evidence: vec![],
            region_evidence: vec![],
            asset_ids: vec![],
        }
    }

    fn inherited_fixture() -> (Document, ObjectId) {
        let mut pdf = Document::with_version("1.7");
        let resources = pdf.add_object(dictionary! {
            "XObject" => dictionary! { "VisibleImage" => (99, 0) },
        });
        let parent = pdf.new_object_id();
        let page = pdf.new_object_id();
        pdf.objects.insert(
            parent,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page.into()],
                "Count" => 1,
                "MediaBox" => vec![10.into(), 20.into(), 210.into(), 420.into()],
                "CropBox" => vec![20.into(), 30.into(), 180.into(), 390.into()],
                "Rotate" => 90,
                "Resources" => resources,
            }),
        );
        pdf.objects.insert(
            page,
            Object::Dictionary(dictionary! { "Type" => "Page", "Parent" => parent }),
        );
        (pdf, page)
    }

    #[test]
    fn inherited_nonzero_crop_box_and_rotation_are_resolved() {
        let (pdf, page_id) = inherited_fixture();
        let placement = page_placement(&pdf, page_id, &package_page()).unwrap();
        assert_eq!(placement.llx, 20.0);
        assert_eq!(placement.lly, 30.0);
        assert_eq!(placement.visible_width, 360.0);
        assert_eq!(placement.visible_height, 160.0);
        let mapped = placed_bbox(
            &package_page(),
            placement,
            crate::derived::Bbox {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 40.0,
            },
        )
        .unwrap();
        assert_eq!(mapped.x, 40.0);
        assert_eq!(mapped.y, 40.0);
    }

    #[test]
    fn adding_text_materializes_and_preserves_inherited_resources() {
        let (mut pdf, page_id) = inherited_fixture();
        let font = pdf.add_object(dictionary! { "Type" => "Font" });
        let stream = pdf.add_object(Stream::new(dictionary! {}, b"BT ET".to_vec()));
        add_page_stream(&mut pdf, page_id, font, stream).unwrap();
        let page = pdf.get_dictionary(page_id).unwrap();
        let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
        assert!(resources.has(b"XObject"));
        let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
        assert_eq!(
            fonts.get(b"MPDFHiddenM5").unwrap(),
            &Object::Reference(font)
        );
    }

    #[test]
    fn cid_system_info_uses_required_pdf_strings() {
        let mut pdf = Document::with_version("1.7");
        add_font(&mut pdf, &BTreeSet::from(['A'])).unwrap();
        let cid_font = pdf
            .objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .find(|dict| {
                dict.get(b"Subtype")
                    .ok()
                    .and_then(|value| value.as_name().ok())
                    == Some(b"CIDFontType2".as_slice())
            })
            .unwrap();
        let system_info = cid_font.get(b"CIDSystemInfo").unwrap().as_dict().unwrap();
        assert!(matches!(
            system_info.get(b"Registry").unwrap(),
            Object::String(value, StringFormat::Literal) if value == b"Adobe"
        ));
        assert!(matches!(
            system_info.get(b"Ordering").unwrap(),
            Object::String(value, StringFormat::Literal) if value == b"Identity"
        ));
    }
}
