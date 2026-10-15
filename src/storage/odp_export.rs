//! Exports a `Presentation` as an OpenDocument Presentation (`.odp`, ISO/IEC 26300) package,
//! the write side of `odp_import.rs` (see that module's doc comment for the shared scope: basic
//! shapes/lines/tables/groups and rotation aren't reconstructed on import, so they are not
//! something a round trip could preserve either -- this writer only ever emits `TextBlock` and
//! `Image` elements, which is also everything a plain `Slide` without `elements` synthesizes via
//! `synthesize_v2_if_needed`).
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
        let mut pages = String::new();
        let mut pictures: Vec<(String, Vec<u8>, &'static str)> = Vec::new();
        let mut next_id = 0usize;
        let mut fresh_name = || {
            next_id += 1;
            next_id
        };

        for slide in &pres.slides {
            let (page_style_name, page_body) = render_slide(slide, media_root, &mut text_styles, &mut para_styles, &mut page_styles, &mut pictures, &mut fresh_name)?;
            pages.push_str(&format!(
                r#"<draw:page draw:name="{name}" draw:style-name="{ps}" draw:master-page-name="Default">{body}</draw:page>"#,
                name = encode_xml_text(slide.label.as_deref().unwrap_or("Slide")),
                ps = page_style_name,
                body = page_body,
            ));
        }

        let content_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" office:version="1.2">
<office:automatic-styles><style:style style:name="grNoFill" style:family="graphic"><style:graphic-properties draw:fill="none" draw:stroke="none" draw:auto-grow-height="false" draw:auto-grow-width="false"/></style:style>{text_styles}{para_styles}{page_styles}</office:automatic-styles>
<office:body><office:presentation>{pages}</office:presentation></office:body>
</office:document-content>"#
        );

        let styles_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.2">
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
        match el {
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
                body.push_str(&format!(
                    r#"<draw:frame draw:style-name="grNoFill" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><draw:text-box>{paragraphs}</draw:text-box></draw:frame>"#,
                    x = escaped_cm(t.x, PAGE_W_CM),
                    y = escaped_cm(t.y, PAGE_H_CM),
                    w = escaped_cm(t.w, PAGE_W_CM),
                    h = escaped_cm(t.h, PAGE_H_CM),
                ));
            }
            SlideElement::Image { transform, file_path, .. } => {
                let Some(rel) = file_path.strip_prefix("/media/") else { continue };
                let Ok(bytes) = std::fs::read(media_root.join(rel)) else { continue }; // a missing/broken reference is dropped, not a hard error
                let ext = Path::new(file_path).extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
                let name = format!("img{}.{}", fresh_id(), ext);
                let t = transform;
                body.push_str(&format!(
                    r#"<draw:frame draw:style-name="grNoFill" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><draw:image xlink:href="Pictures/{name}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad" draw:mime-type="{mt}"/></draw:frame>"#,
                    x = escaped_cm(t.x, PAGE_W_CM),
                    y = escaped_cm(t.y, PAGE_H_CM),
                    w = escaped_cm(t.w, PAGE_W_CM),
                    h = escaped_cm(t.h, PAGE_H_CM),
                    mt = picture_media_type(&ext),
                ));
                pictures.push((name, bytes, picture_media_type(&ext)));
            }
            // Shape/Line/Table/Video/Group: not reconstructed on import either (module doc), so
            // there's nothing meaningful to write for one that only ever arrived via that importer
            // -- and for one authored natively in this app's own editor, dropping it here (rather
            // than exporting an approximation this app itself can't read back) keeps the format
            // honest about what a round trip actually preserves.
            _ => {}
        }
    }
    Ok((page_style_name, body))
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
    text_styles.push_str(&format!(
        r#"<style:style style:name="{name}" style:family="text"><style:text-properties fo:font-weight="{weight}" fo:font-style="{style}" style:text-underline-style="{underline}" style:text-line-through-style="{strike}" fo:color="{color}" style:font-name="{font}" fo:font-size="{size}pt"/></style:style>"#,
        name = name,
        weight = if run.bold { "bold" } else { "normal" },
        style = if run.italic { "italic" } else { "normal" },
        underline = if run.underline { "solid" } else { "none" },
        strike = if run.strike { "solid" } else { "none" },
        color = encode_xml_text(&run.color),
        font = encode_xml_text(&run.font_family),
        size = run.font_size_pt,
    ));
    format!(r#"<text:span text:style-name="{name}">{text}</text:span>"#, text = encode_xml_text(&run.text))
}
