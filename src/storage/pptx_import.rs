use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use zip::ZipArchive;

use crate::core::models::{Presentation, Slide, SlideBackground};

pub struct PptxImporter;

impl PptxImporter {
    /// Import a `.pptx` (Office Open XML Presentation) file into a `Presentation`,
    /// preserving each slide's shapes as separate positioned elements -- a text box
    /// keeps its own authored position/size as a `TextBlock`, an inline picture
    /// becomes an `Image` element at its own position -- rather than flattening the
    /// whole slide into one full-bleed text block (docs/IMPORT_EXPORT_NOTES.md's
    /// "PPTX fidelity" discussion). Preserves run-level text formatting
    /// (bold/italic/underline/strikethrough/font/size/color) and paragraph alignment
    /// per shape, plus an explicit slide background (solid color or picture fill)
    /// when the slide sets one directly. Still out of scope: basic shapes/lines/
    /// tables (`<a:tbl>`), group shapes' child coordinate space (a grouped shape's
    /// position is read as if it weren't grouped -- wrong, but no worse than
    /// dropping it entirely), backgrounds inherited from a slide layout/master, and
    /// speaker notes.
    ///
    /// `media_dir` is where an extracted picture (background or inline, embedded
    /// inside the .pptx) gets written to disk — pass the app's `web/media/images`
    /// directory so the resulting `/media/images/<file>` path is servable, matching
    /// how online media imports already work (see `import_online_media` in
    /// api/routes.rs).
    pub fn import_pptx_bytes(bytes: &[u8], file_stem: &str, media_dir: &Path) -> Result<Presentation, Box<dyn std::error::Error>> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| format!("Not a valid .pptx (zip) file: {}", e))?;

        let slide_paths = Self::resolve_slide_order(&mut archive)?;
        if slide_paths.is_empty() {
            return Err("No slides found in .pptx file (not a valid PowerPoint package?)".into());
        }

        // Needed to convert each shape's EMU-based `<a:off>`/`<a:ext>` position into
        // the 0.0..1.0-relative coordinates `ElementTransform` uses. Falls back to
        // the standard 16:9 widescreen size (12192000x6858000 EMU) if the package
        // doesn't declare one -- better than refusing a shape's real position
        // entirely over a missing/malformed `<p:sldSz>`.
        let (slide_w_emu, slide_h_emu) = read_zip_text(&mut archive, "ppt/presentation.xml")
            .ok()
            .and_then(|xml| parse_slide_size(&xml))
            .unwrap_or((12192000.0, 6858000.0));

        let (title_opt, author_opt) = read_zip_text(&mut archive, "docProps/core.xml")
            .ok()
            .map(|xml| (extract_xml_tag_text(&xml, "dc:title"), extract_xml_tag_text(&xml, "dc:creator")))
            .unwrap_or((None, None));

        let title = title_opt
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| file_stem.to_string());
        let author = author_opt.unwrap_or_default();

        let mut slides = Vec::with_capacity(slide_paths.len());
        for (idx, path) in slide_paths.iter().enumerate() {
            let slide_xml = read_zip_text(&mut archive, path)
                .map_err(|e| format!("Could not read slide '{}': {}", path, e))?;

            let elements = extract_slide_elements(&mut archive, path, &slide_xml, slide_w_emu, slide_h_emu, media_dir)?;
            let tag = format!("S{}", idx + 1);

            let mut slide = Slide {
                header: Some(tag.clone()),
                label: Some(format!("Slide {}", idx + 1)),
                tag: Some(tag),
                ..Default::default()
            };

            if !elements.is_empty() {
                slide.elements = elements;
                slide.slide_document_version = 1;
                // Keeps the legacy flat `text` field correct for anything that
                // still reads it directly (search indexing, etc.) now that a
                // slide can carry more than one TextBlock -- joins every
                // TextBlock's text with "\n\n", skipping Image elements.
                slide.project_text_from_elements();
            }

            if let Some(bg) = extract_slide_background(&mut archive, path, &slide_xml, media_dir)? {
                slide.background = Some(match &bg {
                    SlideBackground::Solid(c) => c.clone(),
                    SlideBackground::Image { file_path, .. } => file_path.clone(),
                    _ => String::new(),
                });
                slide.background_v2 = Some(bg);
            }

            slides.push(slide);
        }

        Ok(Presentation {
            id: format!("pres_{}", uuid::Uuid::new_v4()),
            title,
            author,
            slides,
            theme_name: None,
        })
    }

    /// Resolves slide package paths (e.g. `ppt/slides/slide3.xml`) in actual presentation
    /// order. `slideN.xml` filenames don't reliably match visual order once slides have
    /// been reordered or deleted in the authoring tool, so the real source of truth is
    /// `ppt/presentation.xml`'s `<p:sldId r:id="rIdX"/>` list, resolved through
    /// `ppt/_rels/presentation.xml.rels`'s `rIdX -> target path` mapping.
    fn resolve_slide_order(archive: &mut ZipArchive<Cursor<&[u8]>>) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        if let (Ok(presentation_xml), Ok(rels_xml)) = (
            read_zip_text(archive, "ppt/presentation.xml"),
            read_zip_text(archive, "ppt/_rels/presentation.xml.rels"),
        ) {
            let rel_ids = extract_ordered_rel_ids(&presentation_xml);
            let rel_targets = extract_rel_targets(&rels_xml);

            let paths: Vec<String> = rel_ids
                .iter()
                .filter_map(|rid| rel_targets.get(rid))
                .map(|target| {
                    let trimmed = target.trim_start_matches('/').trim_start_matches("./");
                    if trimmed.starts_with("ppt/") {
                        trimmed.to_string()
                    } else {
                        format!("ppt/{}", trimmed)
                    }
                })
                .filter(|p| p.contains("/slides/"))
                .collect();

            if !paths.is_empty() {
                return Ok(paths);
            }
        }

        // Fallback: malformed/unusual package — sort by the numeric suffix in slideN.xml.
        let mut numbered: Vec<(u32, String)> = Vec::new();
        for i in 0..archive.len() {
            let name = archive.by_index(i)?.name().to_string();
            if let Some(rest) = name.strip_prefix("ppt/slides/slide") {
                if let Some(num_str) = rest.strip_suffix(".xml") {
                    if let Ok(n) = num_str.parse::<u32>() {
                        numbered.push((n, name));
                    }
                }
            }
        }
        numbered.sort_by_key(|(n, _)| *n);
        Ok(numbered.into_iter().map(|(_, name)| name).collect())
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

/// Finds the next occurrence of a tag named `tag` (matching both `<tag>`/`<tag attr="...">`
/// and their self-closing `.../>` forms), returning its raw attribute substring, its inner
/// content (empty for a self-closing tag), and the remainder of `xml` just past the tag so
/// the caller can keep scanning forward. PPTX doesn't nest same-named tags within a single
/// paragraph/run/property group, so a plain "next close tag" search is sufficient without a
/// real XML parser.
fn next_tag<'a>(xml: &'a str, tag: &str) -> Option<(&'a str, &'a str, &'a str)> {
    let open_needle = format!("<{}", tag);
    let mut search_from = 0;
    loop {
        let rel_pos = xml[search_from..].find(&open_needle)?;
        let abs_pos = search_from + rel_pos;
        let after_tag_name = abs_pos + open_needle.len();
        let next_char = xml[after_tag_name..].chars().next();
        // Reject partial matches, e.g. tag "a:t" must not match "a:txBody" or "a:tc".
        if !matches!(next_char, Some('>') | Some(' ') | Some('/')) {
            search_from = after_tag_name;
            continue;
        }
        let gt_pos = abs_pos + xml[abs_pos..].find('>')?;
        let attrs = &xml[after_tag_name..gt_pos];
        if xml[..gt_pos].ends_with('/') {
            let attrs = attrs.strip_suffix('/').unwrap_or(attrs);
            return Some((attrs, "", &xml[gt_pos + 1..]));
        }
        let close_needle = format!("</{}>", tag);
        let content_start = gt_pos + 1;
        let close_pos = content_start + xml[content_start..].find(&close_needle)?;
        return Some((attrs, &xml[content_start..close_pos], &xml[close_pos + close_needle.len()..]));
    }
}

/// Content-only convenience wrapper around `next_tag`, for tags whose attributes aren't
/// needed (e.g. `<a:t>`, `<a:p>`).
fn next_tag_content<'a>(xml: &'a str, tag: &str) -> Option<(&'a str, &'a str)> {
    let (_, content, remainder) = next_tag(xml, tag)?;
    Some((content, remainder))
}

/// Iterates all occurrences of `<tag_name ...>` / `<tag_name .../>`, yielding the raw
/// attribute substring between the tag name and its closing `>`/`/>` for each.
fn find_all_tag_attrs<'a>(xml: &'a str, tag_name: &str) -> Vec<&'a str> {
    let mut results = Vec::new();
    let open_needle = format!("<{}", tag_name);
    let mut search_from = 0;
    while let Some(rel_pos) = xml[search_from..].find(&open_needle) {
        let abs_pos = search_from + rel_pos;
        let after_tag_name = abs_pos + open_needle.len();
        let next_char = xml[after_tag_name..].chars().next();
        if !matches!(next_char, Some('>') | Some(' ') | Some('/')) {
            search_from = after_tag_name;
            continue;
        }
        let Some(gt_rel) = xml[abs_pos..].find('>') else { break; };
        let gt_pos = abs_pos + gt_rel;
        results.push(&xml[after_tag_name..gt_pos]);
        search_from = gt_pos + 1;
    }
    results
}

fn extract_attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{}=\"", name);
    let start = attrs.find(&needle)? + needle.len();
    let end = start + attrs[start..].find('"')?;
    Some(&attrs[start..end])
}

fn extract_ordered_rel_ids(xml: &str) -> Vec<String> {
    find_all_tag_attrs(xml, "p:sldId")
        .into_iter()
        .filter_map(|attrs| extract_attr(attrs, "r:id").map(|s| s.to_string()))
        .collect()
}

fn extract_rel_targets(xml: &str) -> HashMap<String, String> {
    find_all_tag_attrs(xml, "Relationship")
        .into_iter()
        .filter_map(|attrs| {
            let id = extract_attr(attrs, "Id")?;
            let target = extract_attr(attrs, "Target")?;
            Some((id.to_string(), target.to_string()))
        })
        .collect()
}

fn extract_xml_tag_text(xml: &str, tag: &str) -> Option<String> {
    let (content, _) = next_tag_content(xml, tag)?;
    Some(decode_xml_entities(content))
}

/// Extracts one shape's (`<p:txBody>`'s) text as rich `TextRun`s, in document order,
/// plus its first paragraph's alignment as a whole-shape approximation, since this
/// app's `TextParagraphStyle` is one style per text element, not per PPTX paragraph.
/// Paragraph breaks within the shape are represented as their own neutral `"\n"` run
/// rather than appended to adjacent runs' formatting. Called once per shape (see
/// `extract_slide_elements`) -- a slide with multiple text shapes gets one `TextBlock`
/// each, not one flattened block for the whole slide.
fn extract_slide_runs_and_alignment(xml: &str) -> (Vec<crate::core::models::TextRun>, Option<String>) {
    let mut paragraphs: Vec<Vec<crate::core::models::TextRun>> = Vec::new();
    let mut alignment: Option<String> = None;
    let mut rest = xml;

    while let Some((_p_attrs, p_inner, remainder)) = next_tag(rest, "a:p") {
        if alignment.is_none() {
            if let Some((pp_attrs, _, _)) = next_tag(p_inner, "a:pPr") {
                if let Some(algn) = extract_attr(pp_attrs, "algn") {
                    alignment = Some(match algn {
                        "l" => "left",
                        "r" => "right",
                        "just" | "justLow" => "justify",
                        _ => "center",
                    }.to_string());
                }
            }
        }

        let mut para_runs = Vec::new();
        let mut p_rest = p_inner;
        while let Some((_r_attrs, r_inner, r_remainder)) = next_tag(p_rest, "a:r") {
            let (bold, italic, underline, strike, color, font, size) = match next_tag(r_inner, "a:rPr") {
                Some((rp_attrs, rp_content, _)) => extract_run_props(rp_attrs, rp_content),
                None => Default::default(),
            };

            let mut text = String::new();
            let mut t_rest = r_inner;
            while let Some((t_content, t_remainder)) = next_tag_content(t_rest, "a:t") {
                text.push_str(&decode_xml_entities(t_content));
                t_rest = t_remainder;
            }

            if !text.is_empty() {
                let mut run = crate::core::models::TextRun { text, ..Default::default() };
                run.bold = bold;
                run.italic = italic;
                run.underline = underline;
                run.strike = strike;
                if let Some(c) = color {
                    run.color = c;
                }
                if let Some(f) = font {
                    run.font_family = f;
                }
                if let Some(s) = size {
                    run.font_size_pt = s;
                }
                para_runs.push(run);
            }
            p_rest = r_remainder;
        }
        paragraphs.push(para_runs);
        rest = remainder;
    }

    let mut runs = Vec::new();
    for (idx, para) in paragraphs.into_iter().enumerate() {
        if idx > 0 {
            runs.push(crate::core::models::TextRun { text: "\n".to_string(), ..Default::default() });
        }
        runs.extend(para);
    }
    (runs, alignment)
}

/// Reads a run's `<a:rPr>` attributes/content for bold/italic/underline/strikethrough,
/// size (`sz`, in hundredths of a point), fill color (`<a:solidFill><a:srgbClr val="RRGGBB"/>`
/// — theme-color references via `<a:schemeClr>` aren't resolved, matching the "explicit
/// values only" scope this whole importer uses for backgrounds too), and font
/// (`<a:latin typeface="...">`).
fn extract_run_props(attrs: &str, content: &str) -> (bool, bool, bool, bool, Option<String>, Option<String>, Option<f64>) {
    let bold = matches!(extract_attr(attrs, "b"), Some("1") | Some("true"));
    let italic = matches!(extract_attr(attrs, "i"), Some("1") | Some("true"));
    let underline = extract_attr(attrs, "u").map(|v| v != "none").unwrap_or(false);
    let strike = extract_attr(attrs, "strike")
        .map(|v| v.to_lowercase().contains("strike") && v.to_lowercase() != "nostrike")
        .unwrap_or(false);
    let size = extract_attr(attrs, "sz").and_then(|s| s.parse::<f64>().ok()).map(|centipoints| centipoints / 100.0);

    let color = next_tag(content, "a:solidFill").and_then(|(_, fill_content, _)| {
        next_tag(fill_content, "a:srgbClr").and_then(|(clr_attrs, _, _)| {
            extract_attr(clr_attrs, "val").map(|v| format!("#{}", v.to_lowercase()))
        })
    });
    let font = next_tag(content, "a:latin").and_then(|(latin_attrs, _, _)| {
        extract_attr(latin_attrs, "typeface").map(|s| s.to_string())
    });

    (bold, italic, underline, strike, color, font, size)
}

/// Reads `<p:sldSz cx=".." cy=".."/>` from `ppt/presentation.xml` -- the slide
/// canvas size in EMU (English Metric Units, 914400 per inch), needed to convert a
/// shape's absolute `<a:off>`/`<a:ext>` position into the 0.0..1.0-relative
/// coordinates `ElementTransform` uses.
fn parse_slide_size(presentation_xml: &str) -> Option<(f64, f64)> {
    let (attrs, _, _) = next_tag(presentation_xml, "p:sldSz")?;
    let cx = extract_attr(attrs, "cx")?.parse::<f64>().ok()?;
    let cy = extract_attr(attrs, "cy")?.parse::<f64>().ok()?;
    if cx <= 0.0 || cy <= 0.0 {
        return None;
    }
    Some((cx, cy))
}

/// A top-level shape found while scanning a slide's `<p:spTree>` -- carries its raw
/// inner XML, not yet parsed into a `SlideElement` (that happens in
/// `extract_text_shape_element`/`extract_picture_element`, which need different
/// resources: a picture needs the zip archive to resolve/extract its image bytes, a
/// text shape doesn't).
enum SlideShapeXml<'a> {
    TextShape(&'a str),
    Picture(&'a str),
}

/// Position of the next `<tag` occurrence that's actually a tag-name match (not a
/// same-prefixed longer name, e.g. `p:sp` must not match inside `p:spPr`) -- the
/// same partial-match guard `next_tag` itself uses, exposed separately so
/// `next_slide_shape` can compare *where* two different tag names would each be
/// found without consuming either match.
fn find_tag_open_pos(xml: &str, tag: &str) -> Option<usize> {
    let needle = format!("<{}", tag);
    let mut search_from = 0;
    loop {
        let rel = xml[search_from..].find(&needle)?;
        let abs = search_from + rel;
        let after = abs + needle.len();
        match xml[after..].chars().next() {
            Some('>') | Some(' ') | Some('/') => return Some(abs),
            _ => search_from = after,
        }
    }
}

/// Finds whichever of `<p:sp>` (a text/generic shape) or `<p:pic>` (a picture) comes
/// first in `xml`, preserving real document order (= PPTX's own back-to-front paint
/// order) even though the two tag names are scanned for independently. Shape types
/// this importer doesn't handle yet (`<p:cxnSp>` connectors/lines, `<a:tbl>` tables)
/// are simply never matched, so they're skipped rather than mis-parsed -- the same
/// "drop what we don't understand" degradation the rest of this file already uses
/// for e.g. theme-color fills.
fn next_slide_shape<'a>(xml: &'a str) -> Option<(SlideShapeXml<'a>, &'a str)> {
    let sp_pos = find_tag_open_pos(xml, "p:sp");
    let pic_pos = find_tag_open_pos(xml, "p:pic");
    let want_sp = match (sp_pos, pic_pos) {
        (None, None) => return None,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (Some(s), Some(p)) => s <= p,
    };
    if want_sp {
        let (_, inner, rest) = next_tag(xml, "p:sp")?;
        Some((SlideShapeXml::TextShape(inner), rest))
    } else {
        let (_, inner, rest) = next_tag(xml, "p:pic")?;
        Some((SlideShapeXml::Picture(inner), rest))
    }
}

/// Reads a shape's own `<p:spPr><a:xfrm>` (position/size/rotation), normalized into
/// `ElementTransform`'s 0.0..1.0-relative coordinates. Falls back to
/// `ElementTransform::default()` (the same full-bleed-ish box this importer always
/// used before this rewrite) when the shape has no explicit transform of its own --
/// true for some placeholder shapes, which inherit their position from the slide
/// layout/master instead, a level this importer doesn't resolve.
fn extract_shape_transform(shape_inner: &str, slide_w: f64, slide_h: f64) -> crate::core::models::ElementTransform {
    let mut transform = crate::core::models::ElementTransform::default();
    let Some((_, sppr_content, _)) = next_tag(shape_inner, "p:spPr") else { return transform };
    let Some((xfrm_attrs, xfrm_content, _)) = next_tag(sppr_content, "a:xfrm") else { return transform };

    if let (Some((off_attrs, _, _)), Some((ext_attrs, _, _))) =
        (next_tag(xfrm_content, "a:off"), next_tag(xfrm_content, "a:ext"))
    {
        if let (Some(x), Some(y), Some(cx), Some(cy)) = (
            extract_attr(off_attrs, "x").and_then(|s| s.parse::<f64>().ok()),
            extract_attr(off_attrs, "y").and_then(|s| s.parse::<f64>().ok()),
            extract_attr(ext_attrs, "cx").and_then(|s| s.parse::<f64>().ok()),
            extract_attr(ext_attrs, "cy").and_then(|s| s.parse::<f64>().ok()),
        ) {
            transform.x = x / slide_w;
            transform.y = y / slide_h;
            transform.w = cx / slide_w;
            transform.h = cy / slide_h;
        }
    }
    if let Some(rot) = extract_attr(xfrm_attrs, "rot").and_then(|s| s.parse::<f64>().ok()) {
        // PPTX angles are in 60,000ths of a degree.
        transform.rotation_deg = rot / 60_000.0;
    }
    transform.normalized()
}

/// Builds a `TextBlock` element from one `<p:sp>`'s inner XML, or `None` if the shape
/// has no text content at all (a purely decorative autoshape, e.g.) -- matches this
/// importer's existing "nothing to show, nothing to add" behavior for empty shapes.
fn extract_text_shape_element(shape_inner: &str, z_index: i32, slide_w: f64, slide_h: f64) -> Option<crate::core::models::SlideElement> {
    let (_, txbody_content, _) = next_tag(shape_inner, "p:txBody")?;
    let (runs, alignment) = extract_slide_runs_and_alignment(txbody_content);
    if runs.is_empty() {
        return None;
    }
    let mut transform = extract_shape_transform(shape_inner, slide_w, slide_h);
    transform.z_index = z_index;
    Some(crate::core::models::SlideElement::TextBlock {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform,
        block: crate::core::models::TextBlock {
            runs,
            paragraph_style: crate::core::models::TextParagraphStyle {
                align: alignment.unwrap_or_else(|| "center".to_string()),
                ..Default::default()
            },
            effects: crate::core::models::ElementEffects::default(),
            // Unchecked by default (docs/IMPORT_EXPORT_NOTES.md "Follow-on
            // decision: autofit checkbox") -- imported PPTX text keeps its
            // authored position instead of silently reflowing/shrinking
            // against the background. The Slide Editor's Autofit checkbox
            // (properties_panel.ts) lets the operator opt in.
            autofit: false,
        },
    })
}

/// Builds an `Image` element from one `<p:pic>`'s inner XML -- resolves its
/// `<a:blip r:embed="rIdX">` through the slide's own `_rels/slideN.xml.rels` and
/// extracts the embedded picture into `media_dir`, the exact same resolution chain
/// `extract_slide_background` already uses for a `<p:bg>` picture fill. Returns
/// `None` (never an error) if the picture can't be resolved for any reason -- a
/// linked-not-embedded image (`r:link` instead of `r:embed`), a broken relationship,
/// anything -- so one bad picture reference doesn't fail the whole slide's import.
fn extract_picture_element(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    slide_path: &str,
    pic_inner: &str,
    z_index: i32,
    slide_w: f64,
    slide_h: f64,
    media_dir: &Path,
) -> Result<Option<crate::core::models::SlideElement>, Box<dyn std::error::Error>> {
    let rid = match next_tag(pic_inner, "a:blip").and_then(|(attrs, _, _)| extract_attr(attrs, "r:embed")) {
        Some(r) => r.to_string(),
        None => return Ok(None),
    };

    let rels_path = slide_rels_path(slide_path);
    let rels_xml = match read_zip_text(archive, &rels_path) {
        Ok(xml) => xml,
        Err(_) => return Ok(None),
    };
    let targets = extract_rel_targets(&rels_xml);
    let target = match targets.get(&rid) {
        Some(t) => t.clone(),
        None => return Ok(None),
    };

    let slide_dir = slide_path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("ppt/slides");
    let media_path = resolve_relative_path(slide_dir, &target);
    let image_bytes = match read_zip_bytes(archive, &media_path) {
        Ok(b) => b,
        Err(_) => return Ok(None),
    };

    // Extension comes from the bytes, not the archive entry's name -- see storage::media_sniff.
    let file_name = match crate::storage::media_sniff::write_sniffed_media(media_dir, "pptx", &image_bytes) {
        Some(n) => n,
        None => return Ok(None),
    };

    let mut transform = extract_shape_transform(pic_inner, slide_w, slide_h);
    transform.z_index = z_index;

    Ok(Some(crate::core::models::SlideElement::Image {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform,
        file_path: format!("/media/images/{}", file_name),
        crop: None,
        mask_shape: None,
        alt_text: None,
    }))
}

/// Walks a slide's `<p:spTree>` in document order, emitting one positioned
/// `SlideElement` per text shape or inline picture -- the core of this importer's
/// fidelity rewrite (docs/IMPORT_EXPORT_NOTES.md). `z_index` is assigned densely
/// (0, 1, 2, ...) over only the shapes that actually produced an element, so a shape
/// this importer skips (empty text box, unresolvable picture, a shape kind it
/// doesn't handle at all) doesn't leave a gap in the paint order of what's left.
fn extract_slide_elements(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    slide_path: &str,
    slide_xml: &str,
    slide_w: f64,
    slide_h: f64,
    media_dir: &Path,
) -> Result<Vec<crate::core::models::SlideElement>, Box<dyn std::error::Error>> {
    let sp_tree = next_tag_content(slide_xml, "p:spTree").map(|(c, _)| c).unwrap_or(slide_xml);

    let mut elements = Vec::new();
    let mut remaining = sp_tree;
    let mut z_index: i32 = 0;
    while let Some((shape, rest)) = next_slide_shape(remaining) {
        match shape {
            SlideShapeXml::TextShape(inner) => {
                if let Some(el) = extract_text_shape_element(inner, z_index, slide_w, slide_h) {
                    elements.push(el);
                    z_index += 1;
                }
            }
            SlideShapeXml::Picture(inner) => {
                if let Some(el) = extract_picture_element(archive, slide_path, inner, z_index, slide_w, slide_h, media_dir)? {
                    elements.push(el);
                    z_index += 1;
                }
            }
        }
        remaining = rest;
    }
    Ok(elements)
}

/// Resolves a slide's explicit background (`<p:bg>`), if it sets one directly — solid
/// fill or picture fill only; a background inherited from the slide layout/master isn't
/// resolved (see the module-level doc comment). A picture fill's embedded image is
/// extracted from the .pptx and written into `media_dir` so the resulting slide can
/// reference it as an ordinary served file, the same way an online media import works.
fn extract_slide_background(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    slide_path: &str,
    slide_xml: &str,
    media_dir: &Path,
) -> Result<Option<SlideBackground>, Box<dyn std::error::Error>> {
    let (_, bg_content, _) = match next_tag(slide_xml, "p:bg") {
        Some(t) => t,
        None => return Ok(None),
    };

    if let Some((clr_attrs, _, _)) = next_tag(bg_content, "a:srgbClr") {
        if let Some(val) = extract_attr(clr_attrs, "val") {
            return Ok(Some(SlideBackground::Solid(format!("#{}", val.to_lowercase()))));
        }
    }

    let rid = match next_tag(bg_content, "a:blip").and_then(|(blip_attrs, _, _)| extract_attr(blip_attrs, "r:embed")) {
        Some(r) => r.to_string(),
        None => return Ok(None),
    };

    let rels_path = slide_rels_path(slide_path);
    let rels_xml = match read_zip_text(archive, &rels_path) {
        Ok(xml) => xml,
        Err(_) => return Ok(None),
    };
    let targets = extract_rel_targets(&rels_xml);
    let target = match targets.get(&rid) {
        Some(t) => t.clone(),
        None => return Ok(None),
    };

    let slide_dir = slide_path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("ppt/slides");
    let media_path = resolve_relative_path(slide_dir, &target);
    let image_bytes = match read_zip_bytes(archive, &media_path) {
        Ok(b) => b,
        Err(_) => return Ok(None),
    };

    // Extension comes from the bytes, not the archive entry's name -- see storage::media_sniff.
    let file_name = match crate::storage::media_sniff::write_sniffed_media(media_dir, "pptx", &image_bytes) {
        Some(n) => n,
        None => return Ok(None),
    };

    Ok(Some(SlideBackground::Image { file_path: format!("/media/images/{}", file_name), opacity: 1.0 }))
}

/// `ppt/slides/slide3.xml` -> `ppt/slides/_rels/slide3.xml.rels`.
fn slide_rels_path(slide_path: &str) -> String {
    match slide_path.rsplit_once('/') {
        Some((dir, file)) => format!("{}/_rels/{}.rels", dir, file),
        None => format!("_rels/{}.rels", slide_path),
    }
}

/// Resolves a relationship `Target` (e.g. `../media/image1.png`) against the directory
/// that owns the relationship (e.g. `ppt/slides`), per OPC's relative-reference rules.
fn resolve_relative_path(base_dir: &str, target: &str) -> String {
    if let Some(rest) = target.strip_prefix('/') {
        return rest.to_string();
    }
    let mut parts: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Decodes the 5 standard XML entities. `&amp;` must be decoded last, otherwise a
/// literal `&amp;lt;` in the source would incorrectly decode to `<` instead of `&lt;`.
fn decode_xml_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}
