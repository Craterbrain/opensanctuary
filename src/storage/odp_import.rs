//! Imports an OpenDocument Presentation (`.odp`, ISO/IEC 26300) into a `Presentation`,
//! following the same "one positioned element per shape" fidelity rule `pptx_import.rs`
//! uses (docs/IMPORT_EXPORT_NOTES.md), and the same scope: basic shapes/lines/tables and a
//! shape's position inside a group are not reconstructed, same as PPTX's `<p:grpSp>`/`<a:tbl>`
//! gap -- a slide with one just silently drops that one shape rather than mis-parsing it.
//!
//! Unlike PPTX (one file per slide), ODP keeps every slide's content in one `content.xml`, as
//! `<draw:page>` siblings of `<office:presentation>`. Style lookup is two-level: a run or
//! paragraph names a `style:name` (`text:style-name="T1"`), which is defined once, either in
//! `content.xml`'s `<office:automatic-styles>` (where authoring tools put per-element direct
//! formatting -- the common case) or in `styles.xml`'s `<office:styles>`/`<office:automatic-styles>`
//! (named/common styles, and page layouts). Both are scanned into one lookup before any page is
//! read. Real producer output was used to build this against (`testdata/odp/fixture.odp`, built
//! by actual LibreOffice, not hand-authored XML), not just what the spec says is legal.
//!
//! Basic shapes/lines/tables/groups ARE reconstructed -- this app's own `SlideElement` model
//! supports them (`Shape`, `Line`, `Table`, `Group`), so dropping one on import would make a
//! round trip silently lossy for something this app can otherwise render and re-export fine.
//! `draw:rect`/`draw:ellipse` map directly to `Shape`; other preset/custom shapes (`draw:custom-
//! shape`'s `draw:enhanced-geometry draw:type="..."`) carry that type name through as `shape_kind`
//! (translated to this app's own vocabulary -- see `map_custom_shape_type` -- for the handful of
//! ODF preset names that mean the same thing under a different spelling), independent of whatever
//! exact glyph `draw:enhanced-path` describes: this importer does not evaluate that path, so an
//! exotic preset this app can't render natively (anything `shape_library.ts`'s own switch doesn't
//! recognize) still keeps its position/size/color, falling back to that renderer's default (plain
//! rect) glyph rather than disappearing. `draw:line`/`draw:polyline`/`draw:path` map to `Line`;
//! `draw:g` maps to `Group` (recurses into this same element-scanning logic, any depth, via
//! `xml_scan::extract_balanced_tag` since `draw:g` can nest inside itself); a `<table:table>`
//! inside a `draw:frame` maps to `Table` (a cell's own run formatting isn't kept -- `Table`'s cells
//! are plain strings -- and a merged/covered cell, `table:covered-table-cell`, is simply not counted
//! as a cell, same "skip rather than mis-parse" rule as everything else undocumented here).
//!
//! Out of scope: rotated shapes (`draw:transform`'s `rotate()` replaces `svg:x`/`svg:y` entirely
//! per spec -- treated as unrotated/un-positioned rather than guessed; unlike PPTX, which carries
//! an `a:xfrm`'s `rot` straight into `ElementTransform::rotation_deg`, this is a real gap here, not
//! parity), `draw:connector` (an anchored connector between two other shapes by ID, which this
//! app's `Line` has no equivalent for), gradient/bitmap slide backgrounds (solid only), and
//! master-page-inherited content other than page size.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use zip::ZipArchive;

use crate::core::models::{
    CropRect, ElementEffects, ElementTransform, Presentation, Slide, SlideBackground, SlideElement, TextBlock, TextParagraphStyle, TextRun,
};
use crate::storage::xml_scan::{decode_xml_entities, extract_attr, extract_balanced_tag, find_all_tag_attrs, find_tag_open_pos, next_tag, next_tag_content};

pub struct OdpImporter;

#[derive(Default, Clone)]
struct TextStyle {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    color: Option<String>,
    font: Option<String>,
    size_pt: Option<f64>,
}

#[derive(Default, Clone)]
struct ParaStyle {
    align: Option<String>,
}

#[derive(Default, Clone)]
struct PageStyle {
    fill_color: Option<String>,
    fill_image_name: Option<String>,
}

/// A `style:family="graphic"` style's own (not inherited) drawing fill/stroke, for
/// `Shape`/`Line`/`draw:custom-shape`. A real producer's shape gallery (confirmed against an
/// actual user-authored file, see `testdata/odp/README.md`) routinely leaves fill/stroke
/// completely unset on the shape's own `draw:style-name` and relies on `style:parent-style-name`
/// (almost always `"standard"`) for the actual color -- `resolve_graphic_style` walks that chain;
/// this struct only holds what one single named style itself says.
#[derive(Default, Clone)]
struct GraphicStyle {
    /// `Some` when this style has an explicit `draw:fill-color` and `draw:fill` isn't `"none"`
    /// (a `draw:fill-color` with no `draw:fill` attribute at all -- the common real-producer case
    /// -- is still treated as solid-filled with that color, same as every ODF-compliant reader).
    fill_color: Option<String>,
    /// `draw:fill="none"` was explicit at this level -- stops inheritance from the parent, unlike
    /// `fill_color: None`, which just means this level didn't say anything either way.
    fill_none: bool,
    stroke_color: Option<String>,
    stroke_width_cm: Option<f64>,
    /// `draw:stroke="none"` was explicit at this level -- same role as `fill_none`.
    stroke_none: bool,
    marker_start: bool,
    marker_end: bool,
    /// `style:parent-style-name`, if any -- the next link `resolve_graphic_style` follows.
    parent: Option<String>,
}

/// A `GraphicStyle` already resolved up its full `style:parent-style-name` chain -- what callers
/// actually want (see `resolve_graphic_style`).
#[derive(Default)]
struct ResolvedGraphicStyle {
    fill_color: Option<String>,
    stroke_color: Option<String>,
    stroke_width_cm: Option<f64>,
    marker_start: bool,
    marker_end: bool,
}

#[derive(Default)]
struct Styles {
    text: HashMap<String, TextStyle>,
    para: HashMap<String, ParaStyle>,
    page: HashMap<String, PageStyle>,
    graphic: HashMap<String, GraphicStyle>,
    /// `draw:fill-image` definitions: name -> the `Pictures/...` path in `xlink:href`.
    fill_images: HashMap<String, String>,
    /// `style:page-layout` name -> (width_cm, height_cm).
    page_layouts: HashMap<String, (f64, f64)>,
    /// `style:master-page` name -> its page-layout name.
    master_pages: HashMap<String, String>,
}

impl OdpImporter {
    /// `media_dir` is the app's `web/media/images` directory, same convention as
    /// `PptxImporter::import_pptx_bytes`.
    pub fn import_odp_bytes(bytes: &[u8], file_stem: &str, media_dir: &Path) -> Result<Presentation, Box<dyn std::error::Error>> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor).map_err(|e| format!("Not a valid .odp (zip) file: {}", e))?;

        let mimetype = read_zip_text(&mut archive, "mimetype").unwrap_or_default();
        if !mimetype.trim().starts_with("application/vnd.oasis.opendocument.presentation") {
            return Err(format!("not an OpenDocument Presentation (mimetype: '{}')", mimetype.trim()).into());
        }

        let content_xml = read_zip_text(&mut archive, "content.xml").map_err(|e| format!("Could not read content.xml: {}", e))?;
        let styles_xml = read_zip_text(&mut archive, "styles.xml").unwrap_or_default();
        let meta_xml = read_zip_text(&mut archive, "meta.xml").unwrap_or_default();

        let mut styles = Styles::default();
        for doc in [&content_xml, &styles_xml] {
            collect_styles(doc, &mut styles);
        }

        let title = extract_xml_tag_text(&meta_xml, "dc:title")
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| file_stem.to_string());
        let author = extract_xml_tag_text(&meta_xml, "meta:initial-creator")
            .or_else(|| extract_xml_tag_text(&meta_xml, "dc:creator"))
            .unwrap_or_default();

        let (_, presentation_inner, _) =
            next_tag(&content_xml, "office:presentation").ok_or("no <office:presentation> in content.xml (not a presentation document?)")?;

        let mut slides = Vec::new();
        let mut rest = presentation_inner;
        let mut page_num = 0usize;
        while let Some((page_attrs, page_inner, remainder)) = next_tag(rest, "draw:page") {
            rest = remainder;
            page_num += 1;

            // Speaker notes live in a nested <presentation:notes>, which itself contains its own
            // draw:frame shapes (a thumbnail + a placeholder) -- these are never slide content, so
            // the main body is scanned only up to where that block starts (always last, in every
            // real producer's output and per the schema: draw:page's content model ends with it).
            let notes_start = page_inner.find("<presentation:notes");
            let body = notes_start.map(|i| &page_inner[..i]).unwrap_or(page_inner);
            let notes_block = notes_start.map(|i| &page_inner[i..]);

            let (page_w, page_h) = page_dimensions(page_attrs, &styles);
            let elements = extract_page_elements(&mut archive, body, page_w, page_h, &styles, media_dir)?;
            let speaker_notes = notes_block.and_then(extract_notes_text);

            let tag = format!("S{}", page_num);
            let mut slide = Slide { header: Some(tag.clone()), label: Some(format!("Slide {}", page_num)), tag: Some(tag), speaker_notes, ..Default::default() };

            if !elements.is_empty() {
                slide.elements = elements;
                slide.slide_document_version = 1;
                slide.project_text_from_elements();
            }

            if let Some(bg) = extract_page_background(page_attrs, &styles, &mut archive, media_dir) {
                slide.background = Some(match &bg {
                    SlideBackground::Solid(c) => c.clone(),
                    SlideBackground::Image { file_path, .. } => file_path.clone(),
                    _ => String::new(),
                });
                slide.background_v2 = Some(bg);
            }

            slides.push(slide);
        }

        if slides.is_empty() {
            return Err("No slides found in .odp file".into());
        }

        Ok(Presentation { id: format!("pres_{}", uuid::Uuid::new_v4()), title, author, slides, theme_name: None })
    }
}

fn read_zip_text(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut file = archive.by_name(name)?;
    Ok(crate::storage::media_sniff::read_capped_string(&mut file, crate::storage::media_sniff::MAX_ZIP_TEXT_BYTES)?)
}

fn read_zip_bytes(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut file = archive.by_name(name)?;
    Ok(crate::storage::media_sniff::read_capped_bytes(&mut file, crate::storage::media_sniff::MAX_ZIP_MEDIA_BYTES)?)
}

fn extract_xml_tag_text(xml: &str, tag: &str) -> Option<String> {
    let (content, _) = next_tag_content(xml, tag)?;
    Some(decode_xml_entities(content))
}

/// A length attribute (`svg:x="2.5cm"`, `fo:font-size="12pt"`, ...) converted to centimeters.
/// ODF allows `cm`/`mm`/`in`/`pt`/`pc`/`px` suffixes; `px` is treated as 1/96in (the CSS/ODF
/// reference-pixel convention), which only matters for the rare producer that uses it for position.
fn length_cm(s: &str) -> Option<f64> {
    let s = s.trim();
    let split_at = s.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')?;
    let (num, unit) = s.split_at(split_at);
    let value: f64 = num.parse().ok()?;
    let per_cm = match unit {
        "cm" => 1.0,
        "mm" => 0.1,
        "in" | "\"" => 2.54,
        "pt" => 2.54 / 72.0,
        "pc" => 2.54 / 6.0, // 1 pica = 12pt
        "px" => 2.54 / 96.0,
        _ => return None,
    };
    Some(value * per_cm)
}

/// A bare point size (`fo:font-size="12pt"`, occasionally unitless) in points.
fn length_pt(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix("pt") {
        return num.parse().ok();
    }
    length_cm(s).map(|cm| cm / (2.54 / 72.0))
}

/// Scans one document's `<office:styles>` and `<office:automatic-styles>` blocks (there can be
/// one of each, or neither, per document) into `out`. Called once for `content.xml` and once for
/// `styles.xml`; a name already present (content.xml wins, since it's scanned first and this never
/// overwrites) is left alone -- content.xml's automatic-styles are where direct per-element
/// formatting lives, which should win over any same-named common style.
fn collect_styles(xml: &str, out: &mut Styles) {
    for section_tag in ["office:styles", "office:automatic-styles"] {
        let Some((_, section, _)) = next_tag(xml, section_tag) else { continue };
        let mut rest = section;
        while let Some((attrs, content, remainder)) = next_tag(rest, "style:style") {
            rest = remainder;
            let Some(name) = extract_attr(attrs, "style:name") else { continue };
            match extract_attr(attrs, "style:family") {
                Some("text") => {
                    out.text.entry(name.to_string()).or_insert_with(|| parse_text_style(content));
                }
                Some("paragraph") => {
                    out.para.entry(name.to_string()).or_insert_with(|| parse_para_style(content));
                }
                Some("drawing-page") => {
                    out.page.entry(name.to_string()).or_insert_with(|| parse_page_style(content));
                }
                Some("graphic") => {
                    out.graphic.entry(name.to_string()).or_insert_with(|| parse_graphic_style(attrs, content));
                }
                _ => {}
            }
        }
    }
    // page-layouts and master-pages aren't nested inside office:styles/automatic-styles (they're
    // direct children of office:document-styles), so they're scanned over the whole document instead.
    let mut rest = xml;
    while let Some((attrs, content, remainder)) = next_tag(rest, "style:page-layout") {
        rest = remainder;
        let Some(name) = extract_attr(attrs, "style:name") else { continue };
        if let Some((props, _, _)) = next_tag(content, "style:page-layout-properties") {
            if let (Some(w), Some(h)) = (
                extract_attr(props, "fo:page-width").and_then(length_cm),
                extract_attr(props, "fo:page-height").and_then(length_cm),
            ) {
                out.page_layouts.entry(name.to_string()).or_insert((w, h));
            }
        }
    }
    let mut rest = xml;
    while let Some((attrs, _, remainder)) = next_tag(rest, "style:master-page") {
        rest = remainder;
        if let (Some(name), Some(layout)) = (extract_attr(attrs, "style:name"), extract_attr(attrs, "style:page-layout-name")) {
            out.master_pages.entry(name.to_string()).or_insert(layout.to_string());
        }
    }
    let mut rest = xml;
    while let Some((attrs, _, remainder)) = next_tag(rest, "draw:fill-image") {
        rest = remainder;
        if let (Some(name), Some(href)) = (extract_attr(attrs, "draw:name"), extract_attr(attrs, "xlink:href")) {
            out.fill_images.entry(name.to_string()).or_insert(href.to_string());
        }
    }
}

fn parse_text_style(content: &str) -> TextStyle {
    let mut t = TextStyle::default();
    let Some((attrs, _, _)) = next_tag(content, "style:text-properties") else { return t };
    t.bold = extract_attr(attrs, "fo:font-weight").map(|v| v == "bold");
    t.italic = extract_attr(attrs, "fo:font-style").map(|v| v == "italic");
    t.underline = extract_attr(attrs, "style:text-underline-style").map(|v| v != "none");
    t.strike = extract_attr(attrs, "style:text-line-through-style").map(|v| v != "none");
    t.color = extract_attr(attrs, "fo:color").map(|v| v.to_lowercase());
    t.font = extract_attr(attrs, "style:font-name").map(|v| v.to_string());
    t.size_pt = extract_attr(attrs, "fo:font-size").and_then(length_pt);
    t
}

fn parse_para_style(content: &str) -> ParaStyle {
    let mut p = ParaStyle::default();
    if let Some((attrs, _, _)) = next_tag(content, "style:paragraph-properties") {
        p.align = extract_attr(attrs, "fo:text-align").map(|v| match v {
            "right" | "end" => "right",
            "left" | "start" => "left",
            "justify" => "justify",
            _ => "center",
        }.to_string());
    }
    p
}

fn parse_graphic_style(style_attrs: &str, content: &str) -> GraphicStyle {
    let mut g = GraphicStyle { parent: extract_attr(style_attrs, "style:parent-style-name").map(|v| v.to_string()), ..Default::default() };
    let Some((attrs, _, _)) = next_tag(content, "style:graphic-properties") else { return g };
    if extract_attr(attrs, "draw:fill") == Some("none") {
        g.fill_none = true;
    } else {
        g.fill_color = extract_attr(attrs, "draw:fill-color").map(|v| v.to_lowercase());
    }
    if extract_attr(attrs, "draw:stroke") == Some("none") {
        g.stroke_none = true;
    } else {
        g.stroke_color = extract_attr(attrs, "svg:stroke-color").map(|v| v.to_lowercase());
        g.stroke_width_cm = extract_attr(attrs, "svg:stroke-width").and_then(length_cm);
    }
    g.marker_start = extract_attr(attrs, "draw:marker-start").is_some();
    g.marker_end = extract_attr(attrs, "draw:marker-end").is_some();
    g
}

fn parse_page_style(content: &str) -> PageStyle {
    let mut p = PageStyle::default();
    if let Some((attrs, _, _)) = next_tag(content, "style:drawing-page-properties") {
        if extract_attr(attrs, "draw:fill") == Some("solid") {
            p.fill_color = extract_attr(attrs, "draw:fill-color").map(|v| v.to_lowercase());
        } else if extract_attr(attrs, "draw:fill") == Some("bitmap") {
            p.fill_image_name = extract_attr(attrs, "draw:fill-image-name").map(|v| v.to_string());
        }
    }
    p
}

/// The page's own size via its master page's page-layout, falling back to the same 16:9 ratio
/// (at a different absolute size, which doesn't matter once everything is relative 0.0..1.0)
/// `pptx_import.rs` falls back to when a package doesn't declare one.
fn page_dimensions(page_attrs: &str, styles: &Styles) -> (f64, f64) {
    extract_attr(page_attrs, "draw:master-page-name")
        .and_then(|m| styles.master_pages.get(m))
        .and_then(|layout| styles.page_layouts.get(layout))
        .copied()
        .unwrap_or((28.0, 15.75))
}

/// The page-level shape tags this importer recognizes, as returned by `next_page_shape` --
/// `draw:frame` holds text/image/table content, the rest are direct page-level drawing shapes.
enum PageShapeXml<'a> {
    Frame(&'a str, &'a str),
    Rect(&'a str),
    Ellipse(&'a str),
    Line(&'a str),
    Polyline(&'a str),
    Path(&'a str),
    CustomShape(&'a str, &'a str),
    /// A `draw:g`'s own attributes carry nothing this importer uses (see `extract_group_element`),
    /// so only its already-isolated inner span is kept.
    Group(&'a str),
}

/// Finds whichever of the recognized page-level shape tags occurs earliest in `xml` (so document
/// order -- and therefore z-index and paint order -- is preserved across different tag kinds),
/// same "pick the earliest candidate" dispatch `pptx_import.rs`'s `next_slide_shape` uses. A
/// `draw:g`'s own opening tag always sorts before anything nested inside it, so this never
/// "sees into" a group's children early; they're only reached once the caller recurses into the
/// group's own already-isolated inner span.
fn next_page_shape<'a>(xml: &'a str) -> Option<(PageShapeXml<'a>, &'a str)> {
    const CANDIDATES: [&str; 8] = ["draw:frame", "draw:rect", "draw:ellipse", "draw:line", "draw:polyline", "draw:path", "draw:custom-shape", "draw:g"];
    let tag = CANDIDATES.into_iter().filter_map(|t| find_tag_open_pos(xml, t).map(|p| (p, t))).min_by_key(|(p, _)| *p)?.1;
    match tag {
        "draw:frame" => next_tag(xml, "draw:frame").map(|(a, c, r)| (PageShapeXml::Frame(a, c), r)),
        "draw:rect" => next_tag(xml, "draw:rect").map(|(a, _, r)| (PageShapeXml::Rect(a), r)),
        "draw:ellipse" => next_tag(xml, "draw:ellipse").map(|(a, _, r)| (PageShapeXml::Ellipse(a), r)),
        "draw:line" => next_tag(xml, "draw:line").map(|(a, _, r)| (PageShapeXml::Line(a), r)),
        "draw:polyline" => next_tag(xml, "draw:polyline").map(|(a, _, r)| (PageShapeXml::Polyline(a), r)),
        "draw:path" => next_tag(xml, "draw:path").map(|(a, _, r)| (PageShapeXml::Path(a), r)),
        "draw:custom-shape" => next_tag(xml, "draw:custom-shape").map(|(a, c, r)| (PageShapeXml::CustomShape(a, c), r)),
        "draw:g" => extract_balanced_tag(xml, "draw:g").map(|(_, c, r)| (PageShapeXml::Group(c), r)),
        _ => unreachable!(),
    }
}

/// Walks a page's own content (notes already excluded by the caller) in document order, emitting
/// one positioned `SlideElement` per recognized shape (see module doc for the full list and the
/// remaining gaps). `z_index` is assigned densely over only the shapes that actually produced an
/// element -- a tag this importer still doesn't recognize at all is simply never matched by
/// `next_page_shape`, so it's skipped rather than mis-parsed, same as PPTX's own gaps.
fn extract_page_elements(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    body: &str,
    page_w: f64,
    page_h: f64,
    styles: &Styles,
    media_dir: &Path,
) -> Result<Vec<SlideElement>, Box<dyn std::error::Error>> {
    let mut elements = Vec::new();
    let mut rest = body;
    let mut z = 0i32;
    let push = |el: Option<SlideElement>, elements: &mut Vec<SlideElement>, z: &mut i32| {
        if let Some(mut el) = el {
            el.transform_mut().z_index = *z;
            elements.push(el);
            *z += 1;
        }
    };
    while let Some((shape, remainder)) = next_page_shape(rest) {
        rest = remainder;
        match shape {
            PageShapeXml::Frame(attrs, inner) => {
                let transform = frame_transform(attrs, page_w, page_h);
                if let Some((img_attrs, _, _)) = next_tag(inner, "draw:image") {
                    push(extract_image_element(archive, img_attrs, transform, media_dir), &mut elements, &mut z);
                } else if let Some((_, box_inner, _)) = next_tag(inner, "draw:text-box") {
                    push(extract_text_element(box_inner, transform, styles), &mut elements, &mut z);
                } else if let Some((_, table_inner, _)) = next_tag(inner, "table:table") {
                    push(extract_table_element(table_inner, transform), &mut elements, &mut z);
                }
            }
            PageShapeXml::Rect(attrs) => {
                let transform = frame_transform(attrs, page_w, page_h);
                push(Some(extract_shape_element("rect", attrs, transform, page_w, page_h, styles)), &mut elements, &mut z);
            }
            PageShapeXml::Ellipse(attrs) => {
                let transform = frame_transform(attrs, page_w, page_h);
                push(Some(extract_shape_element("ellipse", attrs, transform, page_w, page_h, styles)), &mut elements, &mut z);
            }
            PageShapeXml::CustomShape(attrs, inner) => {
                let transform = frame_transform(attrs, page_w, page_h);
                let kind = next_tag(inner, "draw:enhanced-geometry")
                    .and_then(|(geo_attrs, _, _)| extract_attr(geo_attrs, "draw:type"))
                    .map(map_custom_shape_type)
                    .unwrap_or_else(|| "rect".to_string());
                push(Some(extract_shape_element(&kind, attrs, transform, page_w, page_h, styles)), &mut elements, &mut z);
            }
            PageShapeXml::Line(attrs) => {
                push(Some(extract_line_element(attrs, page_w, page_h, styles)), &mut elements, &mut z);
            }
            PageShapeXml::Polyline(attrs) => {
                let transform = frame_transform(attrs, page_w, page_h);
                let point_count = extract_attr(attrs, "draw:points").map(|p| p.split_whitespace().count()).unwrap_or(2);
                let kind = if point_count > 2 { "elbow" } else { "straight" };
                push(Some(extract_line_shape(kind, attrs, transform, page_w, page_h, styles)), &mut elements, &mut z);
            }
            PageShapeXml::Path(attrs) => {
                let transform = frame_transform(attrs, page_w, page_h);
                push(Some(extract_line_shape("curved", attrs, transform, page_w, page_h, styles)), &mut elements, &mut z);
            }
            PageShapeXml::Group(inner) => {
                push(extract_group_element(archive, inner, page_w, page_h, styles, media_dir)?, &mut elements, &mut z);
            }
        }
    }
    Ok(elements)
}

/// ODF preset/custom-shape type names that mean something this app's own `shape_kind` vocabulary
/// (`web/src/editor/shape_library.ts`) already has a name for, translated to that name; anything
/// else (including this app's own round-tripped `shape_kind` strings, which are never one of these
/// ODF spellings) passes through unchanged, so re-importing this app's own export reconstructs the
/// exact original `shape_kind`, not just a visually-similar one.
/// Every ODF preset/custom-shape type name mapped here was confirmed against real LibreOffice
/// Impress itself (not guessed from the spec or from memory): a probe `.odp` naming each candidate
/// via `draw:type` with no `draw:enhanced-path` at all was handed to real `soffice --convert-to`,
/// and only a name Impress itself renders a distinct default glyph for (proving it's one its own
/// shape gallery actually produces) is treated as confirmed here -- see
/// `docs/IMPORT_EXPORT_NOTES.md` for the full method and the complete confirmed/not-yet-confirmed
/// list. A handful of flowchart presets (`flowchart-data`, `-decision`, `-manual-operation`,
/// `-connector`, `-extract`, `-preparation`) turned out to be exactly this app's own
/// parallelogram/diamond/trapezoid/ellipse/triangle/hexagon shapes under ANSI flowchart names, so
/// they're mapped straight to those instead of carrying the flowchart name through.
fn map_custom_shape_type(t: &str) -> String {
    match t.to_ascii_lowercase().as_str() {
        "rectangle" | "square" | "flowchart-process" => "rect".to_string(),
        "round-rectangle" | "flowchart-alternate-process" => "rounded_rect".to_string(),
        "ellipse" | "circle" | "flowchart-connector" => "ellipse".to_string(),
        "diamond" | "flowchart-decision" => "diamond".to_string(),
        "triangle" | "isosceles-triangle" | "right-triangle" | "flowchart-extract" => "triangle".to_string(),
        "star4" | "star5" | "star6" | "star8" | "star10" | "star12" | "star24" => "star".to_string(),
        "rectangular-callout" | "round-rectangular-callout" | "oval-callout" | "line-callout-1" | "line-callout-2" => "callout".to_string(),
        "trapezoid" | "flowchart-manual-operation" => "trapezoid".to_string(),
        "left-right-arrow" => "left-right-arrow".to_string(),
        "parallelogram" | "flowchart-data" => "parallelogram".to_string(),
        "hexagon" | "flowchart-preparation" => "hexagon".to_string(),
        "octagon" => "octagon".to_string(),
        "cross" => "cross".to_string(),
        "pentagon" => "pentagon".to_string(),
        "heart" => "heart".to_string(),
        "lightning" => "lightning".to_string(),
        "left-arrow" => "left-arrow".to_string(),
        "right-arrow" => "right-arrow".to_string(),
        "up-arrow" => "up-arrow".to_string(),
        "down-arrow" => "down-arrow".to_string(),
        "up-down-arrow" => "up-down-arrow".to_string(),
        "quad-arrow" => "quad-arrow".to_string(),
        "chevron" => "chevron".to_string(),
        "notched-right-arrow" => "notched-right-arrow".to_string(),
        "striped-right-arrow" => "striped-right-arrow".to_string(),
        "ring" => "ring".to_string(),
        "flowchart-merge" => "triangle-down".to_string(),
        other => other.to_string(),
    }
}

fn frame_transform(attrs: &str, page_w: f64, page_h: f64) -> ElementTransform {
    let mut t = ElementTransform::default();
    // A rotated/sheared frame replaces svg:x/y/width/height with draw:transform entirely (an SVG-like
    // "rotate(radians) translate(x y)" string); not reconstructed here (module doc) -- left at the
    // default box rather than guessed from a transform this importer doesn't parse.
    if attrs.contains("draw:transform") {
        return t;
    }
    if let (Some(x), Some(y), Some(w), Some(h)) = (
        extract_attr(attrs, "svg:x").and_then(length_cm),
        extract_attr(attrs, "svg:y").and_then(length_cm),
        extract_attr(attrs, "svg:width").and_then(length_cm),
        extract_attr(attrs, "svg:height").and_then(length_cm),
    ) {
        if page_w > 0.0 && page_h > 0.0 {
            t.x = x / page_w;
            t.y = y / page_h;
            t.w = w / page_w;
            t.h = h / page_h;
        }
    }
    t.normalized()
}

/// One `<draw:text-box>`'s paragraphs/runs, as rich `TextRun`s in document order, plus the first
/// paragraph's alignment as a whole-shape approximation -- the same simplification
/// `extract_slide_runs_and_alignment` in `pptx_import.rs` makes, for the same reason (this app's
/// `TextParagraphStyle` is one style per text element, not per ODF paragraph).
fn extract_text_element(box_inner: &str, transform: ElementTransform, styles: &Styles) -> Option<SlideElement> {
    let mut paragraphs: Vec<Vec<TextRun>> = Vec::new();
    let mut alignment: Option<String> = None;
    let mut rest = box_inner;
    while let Some((p_attrs, p_inner, remainder)) = next_tag(rest, "text:p") {
        rest = remainder;
        if alignment.is_none() {
            alignment = extract_attr(p_attrs, "text:style-name")
                .and_then(|name| styles.para.get(name))
                .and_then(|p| p.align.clone());
        }
        paragraphs.push(runs_in_paragraph(p_inner, styles));
    }
    let mut runs = Vec::new();
    for (idx, para) in paragraphs.into_iter().enumerate() {
        if idx > 0 {
            runs.push(TextRun { text: "\n".to_string(), ..Default::default() });
        }
        runs.extend(para);
    }
    if runs.is_empty() {
        return None;
    }
    Some(SlideElement::TextBlock {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform,
        block: TextBlock {
            runs,
            paragraph_style: TextParagraphStyle { align: alignment.unwrap_or_else(|| "center".to_string()), ..Default::default() },
            effects: ElementEffects::default(),
            autofit: false,
        },
    })
}

/// A paragraph's runs, in order: bare text directly inside `<text:p>` (before the first
/// `<text:span>`, between spans, or the whole paragraph if there are none) becomes one neutral
/// run; each `<text:span text:style-name="..">` becomes a styled run via the resolved `TextStyle`.
/// An empty run (a span with no text) is skipped, same as PPTX's `<a:r>` handling.
fn runs_in_paragraph(p_inner: &str, styles: &Styles) -> Vec<TextRun> {
    let mut out = Vec::new();
    let mut rest = p_inner;
    let push_plain = |out: &mut Vec<TextRun>, text: &str| {
        let text = decode_xml_entities(text);
        if !text.is_empty() {
            out.push(TextRun { text, ..Default::default() });
        }
    };
    loop {
        let Some(span_pos) = find_tag_open_pos(rest, "text:span") else {
            push_plain(&mut out, rest);
            break;
        };
        push_plain(&mut out, &rest[..span_pos]);
        let (span_attrs, span_content, remainder) = next_tag(rest, "text:span").expect("find_tag_open_pos just found it");
        rest = remainder;
        let text = decode_xml_entities(span_content);
        if text.is_empty() {
            continue;
        }
        let mut run = TextRun { text, ..Default::default() };
        if let Some(style) = extract_attr(span_attrs, "text:style-name").and_then(|name| styles.text.get(name)) {
            if let Some(b) = style.bold {
                run.bold = b;
            }
            if let Some(i) = style.italic {
                run.italic = i;
            }
            if let Some(u) = style.underline {
                run.underline = u;
            }
            if let Some(s) = style.strike {
                run.strike = s;
            }
            if let Some(c) = &style.color {
                run.color = c.clone();
            }
            if let Some(f) = &style.font {
                run.font_family = f.clone();
            }
            if let Some(sz) = style.size_pt {
                run.font_size_pt = sz;
            }
        }
        out.push(run);
    }
    out
}

fn extract_image_element(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    img_attrs: &str,
    transform: ElementTransform,
    media_dir: &Path,
) -> Option<SlideElement> {
    let href = extract_attr(img_attrs, "xlink:href")?;
    let path = href.trim_start_matches("./");
    let image_bytes = read_zip_bytes(archive, path).ok()?;
    let file_name = crate::storage::media_sniff::write_sniffed_media(media_dir, "odp", &image_bytes)?;
    Some(SlideElement::Image {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform,
        file_path: format!("/media/images/{}", file_name),
        crop: None::<CropRect>,
        mask_shape: None,
        alt_text: None,
    })
}

/// Resolves `draw:style-name` against `styles.graphic`, defaulting to an unfilled, unstroked
/// shape when the style is missing or unrecognized (never a guessed color).
/// Resolves `draw:style-name` against `styles.graphic`, walking `style:parent-style-name` (a real
/// producer's shape gallery routinely leaves fill/stroke unset on the shape's own style and
/// relies entirely on its parent, almost always named `"standard"`, for the actual color -- see
/// `GraphicStyle`'s doc) until a level settles each property (an explicit `draw:fill="none"`/
/// `draw:stroke="none"` also settles it, at whatever `none` value that level means) or the chain
/// runs out. A cycle (a style naming itself, directly or indirectly, as its own ancestor -- never
/// legal ODF, but not this importer's job to assume) stops the walk rather than looping forever.
fn resolve_graphic_style(attrs: &str, styles: &Styles) -> ResolvedGraphicStyle {
    let mut resolved = ResolvedGraphicStyle::default();
    let mut fill_settled = false;
    let mut stroke_settled = false;
    let mut current = extract_attr(attrs, "draw:style-name").map(|s| s.to_string());
    let mut seen = std::collections::HashSet::new();
    while let Some(name) = current {
        if !seen.insert(name.clone()) {
            break;
        }
        let Some(s) = styles.graphic.get(&name) else { break };
        if !fill_settled {
            if s.fill_none {
                fill_settled = true;
            } else if s.fill_color.is_some() {
                resolved.fill_color = s.fill_color.clone();
                fill_settled = true;
            }
        }
        if !stroke_settled {
            if s.stroke_none {
                stroke_settled = true;
            } else if s.stroke_color.is_some() || s.stroke_width_cm.is_some() {
                resolved.stroke_color = s.stroke_color.clone();
                resolved.stroke_width_cm = s.stroke_width_cm;
                stroke_settled = true;
            }
        }
        resolved.marker_start = resolved.marker_start || s.marker_start;
        resolved.marker_end = resolved.marker_end || s.marker_end;
        current = s.parent.clone();
    }
    resolved
}

/// `Shape`/`Line`'s `stroke_width` is not a physical unit -- `shape_library.ts`/`line_tool.ts`
/// both draw into a fixed `0 0 100 100` SVG viewBox and pass `stroke_width` straight through as
/// that viewBox's own `stroke-width`, so it's really "percent of the shape's own box" (its
/// shorter side, since the box is rarely square and the viewBox is stretched to fit both
/// dimensions independently). Converts a real stroke width in cm to that same relative unit,
/// using the shape's own box (in cm, on this page) as the reference.
fn cm_to_relative_stroke(stroke_cm: f64, box_w_cm: f64, box_h_cm: f64) -> f64 {
    let base = box_w_cm.min(box_h_cm);
    if base <= 0.0 {
        return 0.0;
    }
    (stroke_cm / base) * 100.0
}

/// A `draw:rect`/`draw:ellipse`/`draw:custom-shape` as this app's `Shape`. `kind` is already this
/// app's own `shape_kind` vocabulary (the caller resolves `rect`/`ellipse` from the tag name itself,
/// or a custom shape's `draw:type` via `map_custom_shape_type`); the exact `draw:enhanced-path` a
/// custom shape carries is never evaluated -- this app renders by `shape_kind` alone (see module
/// doc), so an exotic preset still keeps its position/size/color even if its precise outline isn't
/// reproduced.
fn extract_shape_element(kind: &str, attrs: &str, transform: ElementTransform, page_w: f64, page_h: f64, styles: &Styles) -> SlideElement {
    let style = resolve_graphic_style(attrs, styles);
    // A "ring" (see odp_export.rs's render_element) has no fill at all -- it's exported as an
    // unfilled, thick-stroked outline with fill_color repurposed as the stroke's own color, so
    // reading it back takes the ring's color from the stroke instead of the (always empty) fill.
    let (fill_color, stroke_color, stroke_width) = if kind == "ring" {
        (style.stroke_color.unwrap_or_else(|| "none".to_string()), None, 0.0)
    } else {
        (
            style.fill_color.unwrap_or_else(|| "none".to_string()),
            style.stroke_color,
            style.stroke_width_cm.map(|cm| cm_to_relative_stroke(cm, transform.w * page_w, transform.h * page_h)).unwrap_or(0.0),
        )
    };
    SlideElement::Shape {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform,
        shape_kind: kind.to_string(),
        fill_color,
        stroke_color,
        stroke_width,
    }
}

/// A `draw:line` -- the only page-level shape whose position comes from two absolute endpoints
/// (`svg:x1/y1/x2/y2`) rather than a `svg:x/y/width/height` box -- as this app's `Line` (always
/// `line_kind: "straight"`; `Line`'s rendering of "arrow" is driven by `start_arrow`/`end_arrow`,
/// not the kind string, see `web/src/editor/line_tool.ts`). A perfectly horizontal or vertical
/// line has a zero-width or zero-height bounding box, which this app's `ElementTransform` can't
/// represent as a selectable element, so it's floored to a thin sliver instead of collapsing to
/// nothing.
fn extract_line_element(attrs: &str, page_w: f64, page_h: f64, styles: &Styles) -> SlideElement {
    let (x1, y1, x2, y2) = (
        extract_attr(attrs, "svg:x1").and_then(length_cm).unwrap_or(0.0),
        extract_attr(attrs, "svg:y1").and_then(length_cm).unwrap_or(0.0),
        extract_attr(attrs, "svg:x2").and_then(length_cm).unwrap_or(0.0),
        extract_attr(attrs, "svg:y2").and_then(length_cm).unwrap_or(0.0),
    );
    let mut transform = ElementTransform::default();
    if page_w > 0.0 && page_h > 0.0 {
        transform.x = (x1.min(x2)) / page_w;
        transform.y = (y1.min(y2)) / page_h;
        transform.w = ((x1 - x2).abs() / page_w).max(0.01);
        transform.h = ((y1 - y2).abs() / page_h).max(0.01);
    }
    extract_line_shape("straight", attrs, transform.normalized(), page_w, page_h, styles)
}

/// A `draw:polyline`/`draw:path` already placed via a `svg:x/y/width/height` box (`kind` already
/// decided by the caller from the point count or tag), or a `draw:line`'s computed box (via
/// `extract_line_element`), as this app's `Line`.
fn extract_line_shape(kind: &str, attrs: &str, transform: ElementTransform, page_w: f64, page_h: f64, styles: &Styles) -> SlideElement {
    let style = resolve_graphic_style(attrs, styles);
    let color = style.stroke_color.unwrap_or_else(|| "#000000".to_string());
    let stroke_width = style.stroke_width_cm.map(|cm| cm_to_relative_stroke(cm, transform.w * page_w, transform.h * page_h)).unwrap_or(3.0);
    let (start_arrow, end_arrow) = (style.marker_start, style.marker_end);
    SlideElement::Line {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform,
        line_kind: kind.to_string(),
        color,
        stroke_width,
        start_arrow,
        end_arrow,
    }
}

/// A `<table:table>` (already unwrapped from its `draw:frame`) as this app's `Table`. A cell's own
/// run formatting is not kept (`Table::cells` is plain strings, see module doc); a merged cell's
/// `table:covered-table-cell` placeholder is a different tag name than `table:table-cell`, so it's
/// simply not counted as a cell -- the row is padded back out to the table's column count with
/// empty strings afterward, rather than left short.
fn extract_table_element(table_inner: &str, transform: ElementTransform) -> Option<SlideElement> {
    let cols = find_all_tag_attrs(table_inner, "table:table-column").len().max(1);
    let mut cells: Vec<Vec<String>> = Vec::new();
    let mut rest = table_inner;
    while let Some((_, row_inner, remainder)) = next_tag(rest, "table:table-row") {
        rest = remainder;
        let mut row = Vec::new();
        let mut r2 = row_inner;
        while let Some((_, cell_inner, rem2)) = next_tag(r2, "table:table-cell") {
            r2 = rem2;
            row.push(cell_text(cell_inner));
        }
        row.resize(cols, String::new());
        cells.push(row);
    }
    if cells.is_empty() {
        return None;
    }
    let rows = cells.len();
    Some(SlideElement::Table { id: format!("el_{}", uuid::Uuid::new_v4()), transform, rows, cols, cells })
}

fn cell_text(cell_inner: &str) -> String {
    let mut paras = Vec::new();
    let mut rest = cell_inner;
    while let Some((_, p_inner, remainder)) = next_tag(rest, "text:p") {
        rest = remainder;
        paras.push(plain_text_of(p_inner));
    }
    paras.join("\n")
}

/// A `draw:g`'s already-isolated inner span (see `extract_balanced_tag`) as this app's `Group`:
/// its children keep their own absolute (page-relative) positions -- real producer output doesn't
/// apply an extra offset/scale at the group level for the common case this was built against (see
/// `testdata/odp/README.md`), matching how this app's own grouping (`group_elements` in
/// `src/core/engine/elements.rs`) already treats a group's children -- and the group's own
/// `transform` is purely their bounding box, used for selection, never applied to render them
/// (see `createElementNode`'s `'Group'` case in `web/src/core/slide_render.ts`).
fn extract_group_element(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    inner: &str,
    page_w: f64,
    page_h: f64,
    styles: &Styles,
    media_dir: &Path,
) -> Result<Option<SlideElement>, Box<dyn std::error::Error>> {
    let children = extract_page_elements(archive, inner, page_w, page_h, styles, media_dir)?;
    if children.is_empty() {
        return Ok(None);
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (1.0f64, 1.0f64, 0.0f64, 0.0f64);
    for child in &children {
        let t = child.transform();
        min_x = min_x.min(t.x);
        min_y = min_y.min(t.y);
        max_x = max_x.max(t.x + t.w);
        max_y = max_y.max(t.y + t.h);
    }
    let transform = ElementTransform {
        x: min_x.clamp(0.0, 1.0),
        y: min_y.clamp(0.0, 1.0),
        w: (max_x - min_x).clamp(0.0, 1.0),
        h: (max_y - min_y).clamp(0.0, 1.0),
        ..Default::default()
    };
    Ok(Some(SlideElement::Group { id: format!("el_{}", uuid::Uuid::new_v4()), transform, children }))
}

fn extract_page_background(
    page_attrs: &str,
    styles: &Styles,
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    media_dir: &Path,
) -> Option<SlideBackground> {
    let style_name = extract_attr(page_attrs, "draw:style-name")?;
    let page_style = styles.page.get(style_name)?;
    if let Some(color) = &page_style.fill_color {
        return Some(SlideBackground::Solid(color.clone()));
    }
    if let Some(image_name) = &page_style.fill_image_name {
        let href = styles.fill_images.get(image_name)?;
        let path = href.trim_start_matches("./");
        let image_bytes = read_zip_bytes(archive, path).ok()?;
        let file_name = crate::storage::media_sniff::write_sniffed_media(media_dir, "odp", &image_bytes)?;
        return Some(SlideBackground::Image { file_path: format!("/media/images/{}", file_name), opacity: 1.0 });
    }
    None
}

/// Plain text (paragraphs joined with "\n") from the notes block's own text frame
/// (`presentation:class="notes"`), ignoring the page-thumbnail sibling. `None` for an empty or
/// placeholder-only notes frame, matching every other "nothing to add" case in this importer.
fn extract_notes_text(notes_block: &str) -> Option<String> {
    let mut rest = notes_block;
    while let Some((frame_attrs, frame_inner, remainder)) = next_tag(rest, "draw:frame") {
        rest = remainder;
        if extract_attr(frame_attrs, "presentation:class") != Some("notes") {
            continue;
        }
        let (_, box_inner, _) = next_tag(frame_inner, "draw:text-box")?;
        let mut paras = Vec::new();
        let mut r = box_inner;
        while let Some((_, p_inner, pr)) = next_tag(r, "text:p") {
            r = pr;
            let text = plain_text_of(p_inner);
            if !text.is_empty() {
                paras.push(text);
            }
        }
        if paras.is_empty() {
            return None;
        }
        return Some(paras.join("\n"));
    }
    None
}

fn plain_text_of(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some((span_attrs, span_content, remainder)) = next_tag(rest, "text:span") {
        let before = &rest[..find_tag_open_pos(rest, "text:span").unwrap_or(0)];
        out.push_str(&decode_xml_entities(before));
        let _ = span_attrs;
        out.push_str(&decode_xml_entities(span_content));
        rest = remainder;
    }
    out.push_str(&decode_xml_entities(rest));
    out
}
