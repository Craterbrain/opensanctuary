use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;
use zip::ZipArchive;

use crate::core::models::{Presentation, Slide, SlideBackground};

pub struct PptxImporter;

impl PptxImporter {
    /// Import a `.pptx` (Office Open XML Presentation) file into a `Presentation`,
    /// preserving each slide's run-level text formatting (bold/italic/underline/
    /// strikethrough/font/size/color) and paragraph alignment, plus an explicit slide
    /// background (solid color or picture fill) when the slide sets one directly.
    /// Does not resolve backgrounds inherited from a slide layout/master, reproduce
    /// PowerPoint's exact shape layout/positioning, or import speaker notes — the same
    /// text-and-formatting (not full-layout) fidelity level this app's other
    /// legacy-format importers aim for.
    ///
    /// `media_dir` is where an extracted background picture (embedded inside the
    /// .pptx) gets written to disk — pass the app's `web/media/images` directory so
    /// the resulting `/media/images/<file>` path is servable, matching how online
    /// media imports already work (see `import_online_media` in api/routes.rs).
    pub fn import_pptx_bytes(bytes: &[u8], file_stem: &str, media_dir: &Path) -> Result<Presentation, Box<dyn std::error::Error>> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| format!("Not a valid .pptx (zip) file: {}", e))?;

        let slide_paths = Self::resolve_slide_order(&mut archive)?;
        if slide_paths.is_empty() {
            return Err("No slides found in .pptx file (not a valid PowerPoint package?)".into());
        }

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

            let (runs, alignment) = extract_slide_runs_and_alignment(&slide_xml);
            let text = plain_text_from_runs(&runs);
            let tag = format!("S{}", idx + 1);

            let mut slide = Slide {
                text,
                header: Some(tag.clone()),
                label: Some(format!("Slide {}", idx + 1)),
                tag: Some(tag),
                ..Default::default()
            };

            if !runs.is_empty() {
                slide.elements = vec![crate::core::models::SlideElement::TextBlock {
                    id: format!("el_{}", uuid::Uuid::new_v4()),
                    transform: crate::core::models::ElementTransform::default(),
                    block: crate::core::models::TextBlock {
                        runs,
                        paragraph_style: crate::core::models::TextParagraphStyle {
                            align: alignment.unwrap_or_else(|| "center".to_string()),
                            ..Default::default()
                        },
                        effects: crate::core::models::ElementEffects::default(),
                        autofit: true,
                    },
                }];
                slide.slide_document_version = 1;
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
    let mut s = String::new();
    file.read_to_string(&mut s)?;
    Ok(s)
}

fn read_zip_bytes(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut file = archive.by_name(name)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
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

/// Extracts a slide's text as rich `TextRun`s (in document order, flattening separate
/// shapes into one text block per slide — the same level of fidelity `Song`/`ScheduleItem`
/// slides already use elsewhere in this app) plus the first paragraph's alignment as a
/// whole-block approximation, since this app's `TextParagraphStyle` is one style per text
/// element, not per PPTX paragraph. Paragraph breaks are represented as their own neutral
/// `"\n"` run rather than appended to adjacent runs' formatting.
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

fn plain_text_from_runs(runs: &[crate::core::models::TextRun]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
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

    let ext = Path::new(&media_path).extension().and_then(|e| e.to_str()).unwrap_or("png");
    std::fs::create_dir_all(media_dir)?;
    let file_name = format!("pptx_{}.{}", uuid::Uuid::new_v4(), ext);
    std::fs::write(media_dir.join(&file_name), &image_bytes)?;

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
