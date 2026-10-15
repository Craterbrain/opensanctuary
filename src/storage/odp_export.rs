//! Exports a `Presentation` as an OpenDocument Presentation (`.odp`, ISO/IEC 26300) package,
//! the write side of `odp_import.rs` (see that module's doc comment for the shared scope).
//! `TextBlock`/`Image`/`Shape`/`Line`/`Table`/`Group` are all reconstructed; rotation and `Video`
//! are not (same gaps as the importer -- a rotated or video element only ever arrives via this
//! app's own editor, never via import, so there's nothing to round-trip either way).
//!
//! `Shape`'s exact `shape_kind` is preserved losslessly for a document that stays inside this app
//! (`rect`/`ellipse` become native `draw:rect`/`draw:ellipse`; everything else becomes a
//! `draw:custom-shape` whose `draw:enhanced-path` is this app's own `shape_library.ts` polygon in
//! the same 0..100 coordinate space, so this app's own in-editor render and the exported file's
//! render agree, and `draw:type` carries the literal `shape_kind` string back for re-import). A
//! `Line`'s `line_kind` likewise becomes a real `draw:line`/`draw:polyline`/`draw:path`, matching
//! `line_tool.ts`'s own fixed per-kind geometry.
//!
//! Every slide is written at one fixed 28cm x 15.75cm (16:9) page size -- the app's own default
//! aspect ratio (`web/src/app_core.ts`'s `appOptions.aspectRatio`) and also what current
//! LibreOffice itself defaults a new presentation to -- since `ElementTransform` only ever stores
//! 0.0..1.0-relative coordinates, not an authored absolute canvas size, there is no "original"
//! page size to round-trip for a presentation this app created or imported from a format that
//! itself normalizes everything relative (PPTX does carry a `<p:sldSz>`, but `pptx_import.rs`
//! doesn't keep it on `Presentation` either -- the same gap, not a new one).
//!
//! Correctness is favored over small files: one `style:style` per run/paragraph/background rather
//! than deduplicating identical ones, since a presentation realistically has tens, not thousands.
//! Validated against real LibreOffice (`soffice --headless --convert-to pdf`, see
//! `tests/core_tests.rs`), not just re-imported by this app's own (necessarily lenient) reader.

use std::io::Write as _;
use std::path::Path;

use zip::write::FileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::core::models::{Presentation, Slide, SlideBackground, SlideElement, TextParagraphStyle, TextRun};
use crate::storage::xml_scan::encode_xml_text;

pub struct OdpExporter;

/// 28cm x 15.75cm in the 1/100mm-free "cm" unit this module writes lengths in.
const PAGE_W_CM: f64 = 28.0;
const PAGE_H_CM: f64 = 15.75;

impl OdpExporter {
    /// `media_root` is the app's media directory (the parent of `images/`/`videos/`/...) -- the
    /// same directory an `Image` element's `/media/images/<file>` path is served relative to.
    pub fn export_presentation(pres: &Presentation, media_root: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut text_styles = String::new();
        let mut para_styles = String::new();
        let mut page_styles = String::new();
        let mut graphic_styles = String::new();
        let mut pages = String::new();
        let mut pictures: Vec<(String, Vec<u8>, &'static str)> = Vec::new();
        let mut next_id = 0usize;
        let mut fresh_name = || {
            next_id += 1;
            next_id
        };

        for slide in &pres.slides {
            let (page_style_name, page_body) =
                render_slide(slide, media_root, &mut text_styles, &mut para_styles, &mut page_styles, &mut graphic_styles, &mut pictures, &mut fresh_name)?;
            pages.push_str(&format!(
                r#"<draw:page draw:name="{name}" draw:style-name="{ps}" draw:master-page-name="Default">{body}</draw:page>"#,
                name = encode_xml_text(slide.label.as_deref().unwrap_or("Slide")),
                ps = page_style_name,
                body = page_body,
            ));
        }

        let content_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" office:version="1.2">
<office:automatic-styles><style:style style:name="grNoFill" style:family="graphic"><style:graphic-properties draw:fill="none" draw:stroke="none" draw:auto-grow-height="false" draw:auto-grow-width="false"/></style:style>{text_styles}{para_styles}{page_styles}{graphic_styles}</office:automatic-styles>
<office:body><office:presentation>{pages}</office:presentation></office:body>
</office:document-content>"#
        );

        // office:styles carries one fixed entry: "osArrow", a simple filled-triangle arrowhead
        // (same shape/name real LibreOffice itself ships as "Arrow" -- see
        // docs/IMPORT_EXPORT_NOTES.md), referenced by a Line's graphic style via draw:marker-
        // start/draw:marker-end when this app's own start_arrow/end_arrow is set. Never put "--"
        // inside an XML comment in one of these templates (illegal per the XML spec, and real
        // LibreOffice refuses to even open the file) -- this note lives here instead.
        let styles_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.2">
<office:styles>
<draw:marker draw:name="osArrow" svg:viewBox="0 0 20 30" svg:d="M10 0l-10 30h20z"/>
</office:styles>
<office:automatic-styles>
<style:page-layout style:name="PM1"><style:page-layout-properties fo:page-width="{w}cm" fo:page-height="{h}cm" fo:margin-top="0cm" fo:margin-bottom="0cm" fo:margin-left="0cm" fo:margin-right="0cm" style:print-orientation="landscape"/></style:page-layout>
</office:automatic-styles>
<office:master-styles><style:master-page style:name="Default" style:page-layout-name="PM1"/></office:master-styles>
</office:document-styles>"#,
            w = PAGE_W_CM,
            h = PAGE_H_CM,
        );

        let meta_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" office:version="1.2">
<office:meta><dc:title>{title}</dc:title><dc:creator>{author}</dc:creator><meta:generator>OpenSanctuary</meta:generator></office:meta>
</office:document-meta>"#,
            title = encode_xml_text(&pres.title),
            author = encode_xml_text(&pres.author),
        );

        let mut manifest_entries = String::new();
        manifest_entries.push_str(r#"<manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.presentation"/>"#);
        for (name, media_type) in [("content.xml", "text/xml"), ("styles.xml", "text/xml"), ("meta.xml", "text/xml")] {
            manifest_entries.push_str(&format!(r#"<manifest:file-entry manifest:full-path="{name}" manifest:media-type="{media_type}"/>"#));
        }
        for (name, _, media_type) in &pictures {
            manifest_entries.push_str(&format!(r#"<manifest:file-entry manifest:full-path="Pictures/{name}" manifest:media-type="{media_type}"/>"#));
        }
        let manifest_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">{manifest_entries}</manifest:manifest>"#
        );

        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = ZipWriter::new(&mut buf);
            // The ODF package's own mimetype entry MUST be first and stored (uncompressed) --
            // it's how a generic file-type sniffer identifies the package without inflating it.
            w.start_file("mimetype", FileOptions::default().compression_method(CompressionMethod::Stored))?;
            w.write_all(b"application/vnd.oasis.opendocument.presentation")?;

            let text_opts = FileOptions::default().compression_method(CompressionMethod::Deflated);
            w.start_file("META-INF/manifest.xml", text_opts)?;
            w.write_all(manifest_xml.as_bytes())?;
            w.start_file("content.xml", text_opts)?;
            w.write_all(content_xml.as_bytes())?;
            w.start_file("styles.xml", text_opts)?;
            w.write_all(styles_xml.as_bytes())?;
            w.start_file("meta.xml", text_opts)?;
            w.write_all(meta_xml.as_bytes())?;
            for (name, bytes, _) in &pictures {
                w.start_file(format!("Pictures/{name}"), text_opts)?;
                w.write_all(bytes)?;
            }
            w.finish()?;
        }
        Ok(buf.into_inner())
    }
}

/// Extension -> ODF media type for a picture already in the library (every image this app ever
/// saves has one of these, via `media_sniff::sniff_media_ext` on the way in).
fn picture_media_type(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "heic" => "image/heic",
        _ => "application/octet-stream",
    }
}

fn escaped_cm(fraction: f64, total_cm: f64) -> String {
    format!("{:.4}cm", (fraction * total_cm).max(0.0))
}

#[allow(clippy::too_many_arguments)]
fn render_slide(
    slide: &Slide,
    media_root: &Path,
    text_styles: &mut String,
    para_styles: &mut String,
    page_styles: &mut String,
    graphic_styles: &mut String,
    pictures: &mut Vec<(String, Vec<u8>, &'static str)>,
    fresh_id: &mut impl FnMut() -> usize,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let page_style_name = format!("dp{}", fresh_id());
    let fill_xml = match &slide.background_v2 {
        Some(SlideBackground::Solid(c)) => format!(r#"draw:fill="solid" draw:fill-color="{}""#, encode_xml_text(c)),
        _ => r#"draw:fill="none""#.to_string(),
    };
    page_styles.push_str(&format!(
        r#"<style:style style:name="{page_style_name}" style:family="drawing-page"><style:drawing-page-properties {fill_xml}/></style:style>"#
    ));

    // Sorted by z-index so the written document order is the paint order, same invariant the
    // importer relies on (ODF has no explicit z-index: paint order *is* document order).
    let mut ordered: Vec<&SlideElement> = slide.elements.iter().collect();
    ordered.sort_by_key(|e| e.transform().z_index);

    let mut body = String::new();
    for el in ordered {
        body.push_str(&render_element(el, media_root, text_styles, para_styles, graphic_styles, pictures, fresh_id)?);
    }
    Ok((page_style_name, body))
}

/// Renders one `SlideElement` to its ODF XML. Pulled out of `render_slide` so `Group` can call it
/// recursively for its own children (an ODF `draw:g` is just its children written in sequence,
/// same as this app's own `Group` carries no transform of its own onto them -- module doc).
#[allow(clippy::too_many_arguments)]
fn render_element(
    el: &SlideElement,
    media_root: &Path,
    text_styles: &mut String,
    para_styles: &mut String,
    graphic_styles: &mut String,
    pictures: &mut Vec<(String, Vec<u8>, &'static str)>,
    fresh_id: &mut impl FnMut() -> usize,
) -> Result<String, Box<dyn std::error::Error>> {
    Ok(match el {
        SlideElement::TextBlock { transform, block, .. } => {
            let t = transform;
            let para_style_name = format!("P{}", fresh_id());
            para_styles.push_str(&format!(
                r#"<style:style style:name="{para_style_name}" style:family="paragraph"><style:paragraph-properties fo:text-align="{}"/></style:style>"#,
                odf_align(&block.paragraph_style)
            ));
            let mut paragraphs = String::new();
            paragraphs.push_str(&format!(r#"<text:p text:style-name="{para_style_name}">"#));
            for run in &block.runs {
                if run.text == "\n" {
                    paragraphs.push_str("</text:p>");
                    paragraphs.push_str(&format!(r#"<text:p text:style-name="{para_style_name}">"#));
                    continue;
                }
                paragraphs.push_str(&run_span(run, text_styles, fresh_id));
            }
            paragraphs.push_str("</text:p>");
            format!(
                r#"<draw:frame draw:style-name="grNoFill" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><draw:text-box>{paragraphs}</draw:text-box></draw:frame>"#,
                x = escaped_cm(t.x, PAGE_W_CM),
                y = escaped_cm(t.y, PAGE_H_CM),
                w = escaped_cm(t.w, PAGE_W_CM),
                h = escaped_cm(t.h, PAGE_H_CM),
            )
        }
        SlideElement::Image { transform, file_path, .. } => {
            let Some(rel) = file_path.strip_prefix("/media/") else { return Ok(String::new()) };
            let Ok(bytes) = std::fs::read(media_root.join(rel)) else { return Ok(String::new()) }; // a missing/broken reference is dropped, not a hard error
            let ext = Path::new(file_path).extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
            let name = format!("img{}.{}", fresh_id(), ext);
            let t = transform;
            let xml = format!(
                r#"<draw:frame draw:style-name="grNoFill" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><draw:image xlink:href="Pictures/{name}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad" draw:mime-type="{mt}"/></draw:frame>"#,
                x = escaped_cm(t.x, PAGE_W_CM),
                y = escaped_cm(t.y, PAGE_H_CM),
                w = escaped_cm(t.w, PAGE_W_CM),
                h = escaped_cm(t.h, PAGE_H_CM),
                mt = picture_media_type(&ext),
            );
            pictures.push((name, bytes, picture_media_type(&ext)));
            xml
        }
        SlideElement::Shape { transform, shape_kind, fill_color, stroke_color, stroke_width, .. } => {
            let t = transform;
            let gname = format!("shp{}", fresh_id());
            let normal_stroke_attr = || match stroke_color {
                Some(c) if *stroke_width > 0.0 => {
                    let cm = relative_stroke_to_cm(*stroke_width, t.w * PAGE_W_CM, t.h * PAGE_H_CM);
                    format!(r#"draw:stroke="solid" svg:stroke-color="{}" svg:stroke-width="{:.4}cm""#, encode_xml_text(c), cm)
                }
                _ => r#"draw:stroke="none""#.to_string(),
            };
            // A ring has no "inside" to fill -- real LibreOffice does not treat two opposite-
            // wound subpaths of one draw:enhanced-path as a hole the way SVG's nonzero fill rule
            // would (confirmed by rendering one, not assumed), so it's drawn as an unfilled,
            // thick-stroked outline instead, with fill_color repurposed as that stroke's color
            // (this shape's own stroke_color/stroke_width, if any, go unused).
            let (fill_attr, stroke_attr) = if shape_kind == "ring" {
                let cm = relative_stroke_to_cm(16.0, t.w * PAGE_W_CM, t.h * PAGE_H_CM);
                (r#"draw:fill="none""#.to_string(), format!(r#"draw:stroke="solid" svg:stroke-color="{}" svg:stroke-width="{:.4}cm""#, encode_xml_text(fill_color), cm))
            } else if fill_color.eq_ignore_ascii_case("none") || fill_color.eq_ignore_ascii_case("transparent") {
                (r#"draw:fill="none""#.to_string(), normal_stroke_attr())
            } else {
                (format!(r#"draw:fill="solid" draw:fill-color="{}""#, encode_xml_text(fill_color)), normal_stroke_attr())
            };
            graphic_styles.push_str(&format!(
                r#"<style:style style:name="{gname}" style:family="graphic"><style:graphic-properties {fill_attr} {stroke_attr} draw:auto-grow-height="false" draw:auto-grow-width="false"/></style:style>"#
            ));
            let (x, y, w, h) = (escaped_cm(t.x, PAGE_W_CM), escaped_cm(t.y, PAGE_H_CM), escaped_cm(t.w, PAGE_W_CM), escaped_cm(t.h, PAGE_H_CM));
            match shape_kind.as_str() {
                "rect" | "" => format!(r#"<draw:rect draw:style-name="{gname}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"/>"#),
                "ellipse" | "circle" => format!(r#"<draw:ellipse draw:style-name="{gname}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"/>"#),
                other => format!(
                    r#"<draw:custom-shape draw:style-name="{gname}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><draw:enhanced-geometry svg:viewBox="0 0 100 100" draw:type="{kind}" draw:enhanced-path="{path}"/></draw:custom-shape>"#,
                    kind = encode_xml_text(other),
                    path = shape_enhanced_path(other),
                ),
            }
        }
        SlideElement::Line { transform, line_kind, color, stroke_width, start_arrow, end_arrow, .. } => {
            let t = transform;
            let gname = format!("ln{}", fresh_id());
            let sw_cm = relative_stroke_to_cm(*stroke_width, t.w * PAGE_W_CM, t.h * PAGE_H_CM);
            let marker_w = (sw_cm * 3.0).max(0.2);
            let mut marker_attrs = String::new();
            if *start_arrow {
                marker_attrs.push_str(&format!(r#" draw:marker-start="osArrow" draw:marker-start-width="{marker_w:.4}cm""#));
            }
            if *end_arrow || line_kind == "arrow" {
                marker_attrs.push_str(&format!(r#" draw:marker-end="osArrow" draw:marker-end-width="{marker_w:.4}cm""#));
            }
            graphic_styles.push_str(&format!(
                r#"<style:style style:name="{gname}" style:family="graphic"><style:graphic-properties draw:stroke="solid" svg:stroke-color="{color}" svg:stroke-width="{sw_cm:.4}cm"{marker_attrs}/></style:style>"#,
                color = encode_xml_text(color),
            ));
            let (x, y, w, h) = (escaped_cm(t.x, PAGE_W_CM), escaped_cm(t.y, PAGE_H_CM), escaped_cm(t.w, PAGE_W_CM), escaped_cm(t.h, PAGE_H_CM));
            match line_kind.as_str() {
                "elbow" => {
                    format!(r#"<draw:polyline draw:style-name="{gname}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}" svg:viewBox="0 0 100 100" draw:points="5,5 50,5 50,95 95,95"/>"#)
                }
                "curved" => {
                    format!(r#"<draw:path draw:style-name="{gname}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}" svg:viewBox="0 0 100 100" svg:d="M 5 5 C 35 65 65 80 95 50"/>"#)
                }
                _ => {
                    // straight/arrow: line_tool.ts always draws a horizontal midline within the box.
                    let (x1, y1) = (escaped_cm(t.x, PAGE_W_CM), escaped_cm(t.y + t.h / 2.0, PAGE_H_CM));
                    let (x2, y2) = (escaped_cm(t.x + t.w, PAGE_W_CM), escaped_cm(t.y + t.h / 2.0, PAGE_H_CM));
                    format!(r#"<draw:line draw:style-name="{gname}" svg:x1="{x1}" svg:y1="{y1}" svg:x2="{x2}" svg:y2="{y2}"/>"#)
                }
            }
        }
        SlideElement::Table { transform, rows, cols, cells, .. } => {
            let t = transform;
            let mut table_xml = String::from("<table:table>");
            for _ in 0..*cols {
                table_xml.push_str("<table:table-column/>");
            }
            for r in 0..*rows {
                table_xml.push_str("<table:table-row>");
                for c in 0..*cols {
                    let text = cells.get(r).and_then(|row| row.get(c)).map(|s| s.as_str()).unwrap_or("");
                    table_xml.push_str(&format!("<table:table-cell><text:p>{}</text:p></table:table-cell>", encode_xml_text(text)));
                }
                table_xml.push_str("</table:table-row>");
            }
            table_xml.push_str("</table:table>");
            format!(
                r#"<draw:frame draw:style-name="grNoFill" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}" presentation:class="table">{table_xml}</draw:frame>"#,
                x = escaped_cm(t.x, PAGE_W_CM),
                y = escaped_cm(t.y, PAGE_H_CM),
                w = escaped_cm(t.w, PAGE_W_CM),
                h = escaped_cm(t.h, PAGE_H_CM),
            )
        }
        SlideElement::Group { children, .. } => {
            let mut inner = String::new();
            for child in children {
                inner.push_str(&render_element(child, media_root, text_styles, para_styles, graphic_styles, pictures, fresh_id)?);
            }
            format!("<draw:g>{inner}</draw:g>")
        }
        // Video: not reconstructed on import either (module doc) -- it only ever arrives via this
        // app's own editor, so there's no round trip to preserve, and dropping it here (rather than
        // writing something this app itself can't read back) keeps the format honest about what a
        // round trip actually preserves.
        SlideElement::Video { .. } => String::new(),
    })
}

/// A fixed `draw:enhanced-path` (no modifiers/handles -- see docs/IMPORT_EXPORT_NOTES.md for why
/// those aren't needed for a non-interactive export) in a `0 0 100 100` viewBox, deliberately using
/// the exact same point coordinates as this app's own SVG renderer (`shape_library.ts`) for every
/// `shape_kind` it knows, so the in-app render and the exported file's render agree. An unrecognized
/// kind (including "rounded_rect", approximated here as a chamfered rect, since the real rounded-
/// corner path needs enhanced-path's special arc commands, not plain lines) falls back to a plain
/// rect outline -- still positioned/colored/sized correctly, just not shaped exactly like the
/// in-app glyph.
fn shape_enhanced_path(kind: &str) -> &'static str {
    match kind {
        "triangle" => "M 50 5 L 95 95 L 5 95 Z N",
        "diamond" => "M 50 5 L 95 50 L 50 95 L 5 50 Z N",
        "star" => "M 50 5 L 62 38 L 97 38 L 68 59 L 80 95 L 50 72 L 20 95 L 32 59 L 3 38 L 38 38 Z N",
        "callout" => "M 5 5 L 95 5 L 95 75 L 50 75 L 25 95 L 35 75 L 5 75 Z N",
        "rounded_rect" => "M 15 5 L 85 5 L 95 15 L 95 85 L 85 95 L 15 95 L 5 85 L 5 15 Z N",
        // Same point sets as shape_library.ts's matching `case` arms, in this module's own
        // "M x y L x y ... Z N" dialect of the same 0..100 coordinate space.
        "trapezoid" => "M 10 5 L 90 5 L 75 95 L 25 95 Z N",
        "left-right-arrow" => "M 2 50 L 20 25 L 20 38 L 80 38 L 80 25 L 98 50 L 80 75 L 80 62 L 20 62 L 20 75 Z N",
        "parallelogram" => "M 30 5 L 95 5 L 70 95 L 5 95 Z N",
        "hexagon" => "M 25 5 L 75 5 L 95 50 L 75 95 L 25 95 L 5 50 Z N",
        "octagon" => "M 30 5 L 70 5 L 95 30 L 95 70 L 70 95 L 30 95 L 5 70 L 5 30 Z N",
        "cross" => "M 35 5 L 65 5 L 65 35 L 95 35 L 95 65 L 65 65 L 65 95 L 35 95 L 35 65 L 5 65 L 5 35 L 35 35 Z N",
        "pentagon" => "M 50 5 L 95 38 L 78 95 L 22 95 L 5 38 Z N",
        "heart" => "M 50 20 L 38 5 L 15 5 L 5 20 L 5 40 L 50 95 L 95 40 L 95 20 L 85 5 L 62 5 Z N",
        "lightning" => "M 60 5 L 25 55 L 50 55 L 35 95 L 85 40 L 55 40 L 70 5 Z N",
        "left-arrow" => "M 95 30 L 35 30 L 35 10 L 5 50 L 35 90 L 35 70 L 95 70 Z N",
        "right-arrow" => "M 5 30 L 65 30 L 65 10 L 95 50 L 65 90 L 65 70 L 5 70 Z N",
        "up-arrow" => "M 30 95 L 30 35 L 10 35 L 50 5 L 90 35 L 70 35 L 70 95 Z N",
        "down-arrow" => "M 30 5 L 30 65 L 10 65 L 50 95 L 90 65 L 70 65 L 70 5 Z N",
        "up-down-arrow" => "M 30 25 L 10 25 L 50 5 L 90 25 L 70 25 L 70 75 L 90 75 L 50 95 L 10 75 L 30 75 Z N",
        "quad-arrow" => {
            "M 50 5 L 65 20 L 58 20 L 58 42 L 80 42 L 80 35 L 95 50 L 80 65 L 80 58 L 58 58 L 58 80 L 65 80 L 50 95 \
             L 35 80 L 42 80 L 42 58 L 20 58 L 20 65 L 5 50 L 20 35 L 20 42 L 42 42 L 42 20 L 35 20 Z N"
        }
        "chevron" => "M 5 5 L 65 5 L 95 50 L 65 95 L 5 95 L 35 50 Z N",
        "notched-right-arrow" => "M 5 30 L 55 30 L 55 10 L 95 50 L 55 90 L 55 70 L 5 70 L 25 50 Z N",
        // Same simplification as shape_library.ts's matching case: the tail stripes are dropped,
        // same outline as right-arrow.
        "striped-right-arrow" => "M 5 30 L 65 30 L 65 10 L 95 50 L 65 90 L 65 70 L 5 70 Z N",
        "triangle-down" => "M 5 5 L 95 5 L 50 95 Z N",
        // A plain octagon outline (an approximate circle) -- "ring" is handled as a special
        // case in render_element (unfilled, thick-stroked), not through the usual fill path, so
        // this entry is only reached if that special case is ever bypassed; kept as a reasonable
        // outline rather than the generic rect fallback.
        "ring" => "M 50 5 L 82 18 L 95 50 L 82 82 L 50 95 L 18 82 L 5 50 L 18 18 Z N",
        _ => "M 5 5 L 95 5 L 95 95 L 5 95 Z N",
    }
}

/// The inverse of `odp_import.rs`'s `cm_to_relative_stroke`: `Shape`/`Line`'s `stroke_width` is a
/// percent of the shape's own box (its shorter side), not a physical unit -- see that function's
/// doc for why -- so converting it to a real `svg:stroke-width` needs the box's own size in cm.
fn relative_stroke_to_cm(stroke_width_units: f64, box_w_cm: f64, box_h_cm: f64) -> f64 {
    (stroke_width_units / 100.0) * box_w_cm.min(box_h_cm)
}

fn odf_align(p: &TextParagraphStyle) -> &'static str {
    match p.align.as_str() {
        "left" => "left",
        "right" => "right",
        "justify" => "justify",
        _ => "center",
    }
}

fn run_span(run: &TextRun, text_styles: &mut String, fresh_id: &mut impl FnMut() -> usize) -> String {
    let name = format!("T{}", fresh_id());
    // A `TextRun` has no way to tell "explicitly set to white" apart from "never styled, so it's
    // sitting at this struct's own default" (`models::default_text_color`/`default_font_family`,
    // chosen for this app's own usual case of text over a dark background/video). Writing that
    // placeholder color and a CSS font *stack* ("Inter, sans-serif", not a real, single font name)
    // into a real ODF file is actively wrong: opened in an ODF tool, which defaults an unstyled
    // page to white, white-on-white text becomes invisible -- a document that round-tripped
    // through this app would visibly lose text a human never asked to hide. Since a run that is
    // at *both* app defaults at once is the only case this module can tell apart with any real
    // confidence, that one case omits `fo:color`/`style:font-name` entirely instead, so the
    // opening application's own default (a real font, black-on-white) applies.
    let at_app_defaults = run.color == crate::core::models::default_text_color() && run.font_family == crate::core::models::default_font_family();
    let color_attr = if at_app_defaults { String::new() } else { format!(r#" fo:color="{}""#, encode_xml_text(&run.color)) };
    let font_attr = if at_app_defaults { String::new() } else { format!(r#" style:font-name="{}""#, encode_xml_text(&run.font_family)) };
    text_styles.push_str(&format!(
        r#"<style:style style:name="{name}" style:family="text"><style:text-properties fo:font-weight="{weight}" fo:font-style="{style}" style:text-underline-style="{underline}" style:text-line-through-style="{strike}"{color_attr}{font_attr} fo:font-size="{size}pt"/></style:style>"#,
        name = name,
        weight = if run.bold { "bold" } else { "normal" },
        style = if run.italic { "italic" } else { "normal" },
        underline = if run.underline { "solid" } else { "none" },
        strike = if run.strike { "solid" } else { "none" },
        size = run.font_size_pt,
    ));
    format!(r#"<text:span text:style-name="{name}">{text}</text:span>"#, text = encode_xml_text(&run.text))
}
