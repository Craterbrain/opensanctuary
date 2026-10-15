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
//! Out of scope, matching PPTX's own documented gaps: `draw:rect`/`draw:ellipse`/`draw:line`/
//! `draw:custom-shape`/tables, a shape inside `draw:g` (group) at its grouped position, rotated
//! shapes (`draw:transform`'s `rotate()` replaces `svg:x`/`svg:y` entirely per spec -- treated as
//! unrotated/un-positioned rather than guessed), gradient/bitmap slide backgrounds (solid only),
//! and master-page-inherited content other than page size.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use zip::ZipArchive;

use crate::core::models::{
    CropRect, ElementEffects, ElementTransform, Presentation, Slide, SlideBackground, SlideElement, TextBlock, TextParagraphStyle, TextRun,
};
use crate::storage::xml_scan::{decode_xml_entities, extract_attr, find_tag_open_pos, next_tag, next_tag_content};

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

#[derive(Default)]
struct Styles {
    text: HashMap<String, TextStyle>,
    para: HashMap<String, ParaStyle>,
    page: HashMap<String, PageStyle>,
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

/// Walks a page's own content (notes already excluded by the caller) in document order, emitting
/// one positioned `SlideElement` per `draw:frame` that holds either a `draw:image` or a
/// `draw:text-box` -- a shape kind this importer doesn't handle (`draw:rect`/`draw:ellipse`/
/// `draw:custom-shape`/`draw:line`/`draw:g`, see module doc) is simply never matched by
/// `next_tag(.., "draw:frame")`'s target, so it's skipped rather than mis-parsed, same as PPTX's
/// own gaps. `z_index` is assigned densely over only the shapes that actually produced an element.
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
    while let Some((attrs, inner, remainder)) = next_tag(rest, "draw:frame") {
        rest = remainder;
        let transform = frame_transform(attrs, page_w, page_h);
        if let Some((img_attrs, _, _)) = next_tag(inner, "draw:image") {
            if let Some(mut el) = extract_image_element(archive, img_attrs, transform, media_dir) {
                el.transform_mut().z_index = z;
                elements.push(el);
                z += 1;
            }
        } else if let Some((_, box_inner, _)) = next_tag(inner, "draw:text-box") {
            if let Some(mut el) = extract_text_element(box_inner, transform, styles) {
                el.transform_mut().z_index = z;
                elements.push(el);
                z += 1;
            }
        }
    }
    Ok(elements)
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
