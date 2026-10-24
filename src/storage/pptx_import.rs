use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use zip::ZipArchive;

use crate::core::models::{Presentation, Slide, SlideBackground};
use crate::storage::xml_scan::{decode_xml_entities, extract_attr, extract_balanced_tag, find_all_tag_attrs, find_tag_open_pos, next_tag, next_tag_content};

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
    /// when the slide sets one directly.
    ///
    /// `Shape`/`Line`/`Table`/`Group` are reconstructed too (ported from the ODP
    /// importer's equivalent work, see docs/IMPORT_EXPORT_NOTES.md): an autoshape
    /// with no text (`<p:sp>` + `<a:prstGeom>`) becomes a `Shape`, its OOXML preset
    /// name translated to this app's `shape_kind` vocabulary (`map_prst_geom_type`,
    /// verified against Microsoft's own published `ShapeTypeValues` enumeration, not
    /// guessed -- OOXML's names often differ a lot from ODF's for the same shape,
    /// e.g. `donut` not `ring`, `plus` not `cross`, `lightningBolt` not `lightning`);
    /// a plain `<p:sp>` with `prst="line"` (how LibreOffice's own OOXML export
    /// represents a simple line shape, confirmed against real output, not assumed)
    /// or an actual `<p:cxnSp>` connector becomes a `Line`; a `<p:graphicFrame>`
    /// wrapping an `<a:tbl>` becomes a `Table`; a `<p:grpSp>` becomes a `Group`,
    /// recursing to any depth via `xml_scan::extract_balanced_tag` (a group can
    /// nest inside itself) and remapping each child's local coordinates through the
    /// group's own `<a:off>`/`<a:ext>`/`<a:chOff>`/`<a:chExt>` (`GroupRemap`) --
    /// OOXML's group model needs this extra step ODF's `draw:g` never did, since an
    /// ODF group's children are already in absolute page coordinates.
    ///
    /// Two deliberate, documented simplifications, not oversights:
    /// - **Only explicit, literal color is ever read** (`<a:solidFill><a:srgbClr
    ///   val="RRGGBB"/></a:solidFill>`, same policy `extract_run_props` already uses
    ///   for text) -- never a theme color (`<a:schemeClr>`) and never a shape's
    ///   `<p:style>`/`<a:fillRef>` reference into the theme's format scheme. This is
    ///   a real, common gap, not a rare corner case: a shape inserted via the
    ///   authoring tool's UI without manually overriding its fill very often has
    ///   *no* explicit `<a:solidFill>` at all, only a `fillRef` -- such a shape
    ///   imports with no fill color (falls back to this app's own default), same
    ///   honest "nothing to show, nothing invented" rule used everywhere else here.
    /// - **A rotated group's children are not re-composed around the rotation.**
    ///   `GroupRemap` only applies the group's own linear offset/scale
    ///   (off/ext/chOff/chExt); a `<p:grpSpPr>`'s own `rot` is not read at all. This
    ///   is a genuinely hard problem -- even mature implementations have long-
    ///   standing bugs around rotated+grouped shape positions -- and an unrotated
    ///   group (the overwhelmingly common case) is unaffected.
    ///
    /// Still out of scope: backgrounds inherited from a slide layout/master, and
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
            let name = archive.by_index(i)?.name()?.to_string();
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

/// A top-level shape found while scanning a slide's `<p:spTree>` (or a group's own
/// inner content, recursively) -- carries its raw inner XML, not yet parsed into a
/// `SlideElement` (that happens in the various `extract_*_element` functions, which
/// need different resources: a picture needs the zip archive to resolve/extract its
/// image bytes, a text shape doesn't). `Sp` covers `<p:sp>`'s three possible outcomes
/// (text shape, non-text autoshape, or a plain line) in one variant since they all
/// share the same tag and are disambiguated by content, not by tag name.
enum SlideShapeXml<'a> {
    Sp(&'a str),
    Picture(&'a str),
    Connector(&'a str),
    GraphicFrame(&'a str),
    Group(&'a str),
}

/// Finds whichever of the recognized page-level shape tags occurs earliest in `xml`
/// (so document order -- PPTX's own back-to-front paint order -- is preserved across
/// different tag kinds), same "pick the earliest candidate" dispatch
/// `odp_import.rs`'s `next_page_shape` uses. A `<p:grpSp>`'s own opening tag always
/// sorts before anything nested inside it, so this never "sees into" a group's
/// children early; they're only reached once the caller recurses into the group's
/// own already-isolated inner span. A tag this importer still doesn't recognize at
/// all (there is no OOXML equivalent of every ODF drawing element) is simply never
/// matched, so it's skipped rather than mis-parsed.
fn next_slide_shape<'a>(xml: &'a str) -> Option<(SlideShapeXml<'a>, &'a str)> {
    const CANDIDATES: [&str; 5] = ["p:sp", "p:pic", "p:cxnSp", "p:graphicFrame", "p:grpSp"];
    let tag = CANDIDATES.into_iter().filter_map(|t| find_tag_open_pos(xml, t).map(|p| (p, t))).min_by_key(|(p, _)| *p)?.1;
    match tag {
        "p:sp" => next_tag(xml, "p:sp").map(|(_, c, r)| (SlideShapeXml::Sp(c), r)),
        "p:pic" => next_tag(xml, "p:pic").map(|(_, c, r)| (SlideShapeXml::Picture(c), r)),
        "p:cxnSp" => next_tag(xml, "p:cxnSp").map(|(_, c, r)| (SlideShapeXml::Connector(c), r)),
        "p:graphicFrame" => next_tag(xml, "p:graphicFrame").map(|(_, c, r)| (SlideShapeXml::GraphicFrame(c), r)),
        "p:grpSp" => extract_balanced_tag(xml, "p:grpSp").map(|(_, c, r)| (SlideShapeXml::Group(c), r)),
        _ => unreachable!(),
    }
}

/// A `<p:grpSp>`'s own `<a:off>`/`<a:ext>`/`<a:chOff>`/`<a:chExt>`, which together
/// define the linear map from a child's own local EMU coordinates to absolute
/// slide-EMU coordinates: `absolute = group_off + (local - child_off) * (group_ext /
/// child_ext)`. Confirmed against real LibreOffice-produced output (not assumed):
/// the common case (an untouched, never resized-after-grouping group) has
/// `off == chOff` and `ext == chExt`, which makes the formula an identity and
/// children's own `a:off`/`a:ext` already absolute -- but the general formula (this
/// struct) subsumes that case and is what a moved/resized group actually needs.
/// Does not account for the group's own rotation (see module doc).
#[derive(Clone, Copy)]
struct GroupRemap {
    off_x: f64,
    off_y: f64,
    ext_cx: f64,
    ext_cy: f64,
    ch_off_x: f64,
    ch_off_y: f64,
    ch_ext_cx: f64,
    ch_ext_cy: f64,
}

impl GroupRemap {
    fn apply(&self, x: f64, y: f64, cx: f64, cy: f64) -> (f64, f64, f64, f64) {
        let sx = if self.ch_ext_cx != 0.0 { self.ext_cx / self.ch_ext_cx } else { 1.0 };
        let sy = if self.ch_ext_cy != 0.0 { self.ext_cy / self.ch_ext_cy } else { 1.0 };
        (self.off_x + (x - self.ch_off_x) * sx, self.off_y + (y - self.ch_off_y) * sy, cx * sx, cy * sy)
    }
}

/// Reads a shape's own `<p:spPr><a:xfrm>` (position/size/rotation), remapped through
/// `remap` when the shape is a group's child (see `GroupRemap`), then normalized into
/// `ElementTransform`'s 0.0..1.0-relative coordinates. Falls back to
/// `ElementTransform::default()` (the same full-bleed-ish box this importer always
/// used before this rewrite) when the shape has no explicit transform of its own --
/// true for some placeholder shapes, which inherit their position from the slide
/// layout/master instead, a level this importer doesn't resolve.
fn extract_shape_transform(shape_inner: &str, slide_w: f64, slide_h: f64, remap: Option<&GroupRemap>) -> crate::core::models::ElementTransform {
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
            let (x, y, cx, cy) = match remap {
                Some(r) => r.apply(x, y, cx, cy),
                None => (x, y, cx, cy),
            };
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
fn extract_text_shape_element(shape_inner: &str, z_index: i32, slide_w: f64, slide_h: f64, remap: Option<&GroupRemap>) -> Option<crate::core::models::SlideElement> {
    let (_, txbody_content, _) = next_tag(shape_inner, "p:txBody")?;
    let (runs, alignment) = extract_slide_runs_and_alignment(txbody_content);
    if runs.is_empty() {
        return None;
    }
    let mut transform = extract_shape_transform(shape_inner, slide_w, slide_h, remap);
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
#[allow(clippy::too_many_arguments)]
fn extract_picture_element(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    slide_path: &str,
    pic_inner: &str,
    z_index: i32,
    slide_w: f64,
    slide_h: f64,
    media_dir: &Path,
    remap: Option<&GroupRemap>,
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

    let mut transform = extract_shape_transform(pic_inner, slide_w, slide_h, remap);
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

/// A `Line`'s resolved `stroke_width`: the real width when `<a:ln w="...">` set one
/// (regardless of whether its color could be resolved -- see `extract_shape_style`),
/// or `line_tool.ts`'s own UI default (`3`, picked for a freshly drawn line) when it
/// didn't, rather than flooring an absent width to some arbitrary tiny nonzero value
/// that would render as a barely-visible hairline.
fn line_stroke_width(style: &ShapeStyle, box_w_emu: f64, box_h_emu: f64) -> f64 {
    if style.stroke_width_emu > 0.0 {
        emu_to_relative_stroke(style.stroke_width_emu, box_w_emu, box_h_emu)
    } else {
        3.0
    }
}

/// `Shape`/`Line`'s `stroke_width` is a percent of the shape's own box (its shorter
/// side), not a physical unit -- same fact `odp_import.rs`'s `cm_to_relative_stroke`
/// documents (both `shape_library.ts` and `line_tool.ts` draw into a fixed `0 0 100
/// 100` SVG viewBox and pass `stroke_width` straight through as that viewBox's own
/// `stroke-width`). Converts an OOXML `<a:ln w="...">` width (EMU) to that same unit.
fn emu_to_relative_stroke(width_emu: f64, box_w_emu: f64, box_h_emu: f64) -> f64 {
    let base = box_w_emu.min(box_h_emu);
    if base <= 0.0 {
        return 0.0;
    }
    (width_emu / base) * 100.0
}

/// A shape's resolved (explicit-only, see module doc) fill/stroke/arrow-marker state.
struct ShapeStyle {
    fill_color: String,
    stroke_color: Option<String>,
    stroke_width_emu: f64,
    start_arrow: bool,
    end_arrow: bool,
}

/// Reads a shape's own `<p:spPr>` for its *explicit, literal* fill/stroke -- never a
/// theme color (`<a:schemeClr>`) or a `<p:style>`/`<a:fillRef>` reference into the
/// theme's format scheme (see module doc: this is a real, common gap, not a rare
/// corner case). `<a:noFill/>` (fill or stroke) is a deliberate "nothing here",
/// distinct from "no fill/stroke information at all" the same way it is for ODP.
fn extract_shape_style(sppr_content: &str) -> ShapeStyle {
    // Fill lives directly in spPr, stroke lives inside <a:ln> -- both use the same
    // <a:solidFill><a:srgbClr val="RRGGBB"/></a:solidFill> shape, so the search for
    // fill is scoped to *before* <a:ln> starts, or it would find the stroke's color
    // instead whenever a fill is actually absent.
    let ln_pos = find_tag_open_pos(sppr_content, "a:ln");
    let fill_scope = ln_pos.map(|p| &sppr_content[..p]).unwrap_or(sppr_content);
    let fill_color = if find_tag_open_pos(fill_scope, "a:noFill").is_some() {
        "none".to_string()
    } else {
        next_tag(fill_scope, "a:solidFill")
            .and_then(|(_, c, _)| next_tag(c, "a:srgbClr"))
            .and_then(|(attrs, _, _)| extract_attr(attrs, "val"))
            .map(|v| format!("#{}", v.to_lowercase()))
            .unwrap_or_else(|| "none".to_string())
    };

    let mut stroke_color = None;
    let mut stroke_width_emu = 0.0;
    let mut start_arrow = false;
    let mut end_arrow = false;
    if let Some((ln_attrs, ln_content, _)) = next_tag(sppr_content, "a:ln") {
        // The stroke's own width (`w`, a literal EMU number) has no theme/scheme
        // equivalent and so no "is it explicit" ambiguity -- read regardless of
        // whether the stroke's *color* could be resolved, so a real author-set
        // width isn't lost just because the color happens to come from the theme.
        stroke_width_emu = extract_attr(ln_attrs, "w").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        if find_tag_open_pos(ln_content, "a:noFill").is_none() {
            stroke_color = next_tag(ln_content, "a:solidFill")
                .and_then(|(_, c, _)| next_tag(c, "a:srgbClr"))
                .and_then(|(attrs, _, _)| extract_attr(attrs, "val"))
                .map(|v| format!("#{}", v.to_lowercase()));
        }
        // headEnd = the line's first point (a:off), tailEnd = its last (a:off + a:ext) --
        // either can specify type="none" explicitly (no marker) even when the element
        // itself is present, so absence and an explicit "none" are both "no arrow".
        start_arrow = next_tag(ln_content, "a:headEnd").map(|(a, _, _)| extract_attr(a, "type") != Some("none")).unwrap_or(false);
        end_arrow = next_tag(ln_content, "a:tailEnd").map(|(a, _, _)| extract_attr(a, "type") != Some("none")).unwrap_or(false);
    }
    ShapeStyle { fill_color, stroke_color, stroke_width_emu, start_arrow, end_arrow }
}

/// Every OOXML preset shape name mapped here is taken verbatim from Microsoft's own
/// published `DocumentFormat.OpenXml.Drawing.ShapeTypeValues` enumeration (the exact
/// serialized `prst` string each member documents), not guessed from memory or
/// copied from ODF's equivalent -- the two vocabularies often diverge a lot for the
/// same shape (`donut` not `ring`, `plus` not `cross`, `lightningBolt` not
/// `lightning`, `nonIsoscelesTrapezoid` as a *separate* name from `trapezoid`).
/// `rect`/`ellipse`/`line` are handled by the caller before this is ever reached
/// (see `next_slide_shape`); everything else not named here (the vast majority of
/// the ~180 presets -- most flowchart/callout/ribbon/math/action-button variants)
/// falls back to a plain rect, same honest degrade `odp_import.rs`'s equivalent uses.
fn map_prst_geom_type(prst: &str) -> String {
    match prst {
        "roundRect" | "round1Rect" | "round2SameRect" | "round2DiagRect" | "flowChartAlternateProcess" => "rounded_rect".to_string(),
        "triangle" | "rtTriangle" => "triangle".to_string(),
        "diamond" | "flowChartDecision" => "diamond".to_string(),
        "star4" | "star5" | "star6" | "star7" | "star8" | "star10" | "star12" | "star16" | "star24" | "star32" => "star".to_string(),
        "wedgeRectCallout" | "wedgeRoundRectCallout" | "wedgeEllipseCallout" | "cloudCallout" | "callout1" | "callout2" | "callout3" | "accentCallout1"
        | "accentCallout2" | "accentCallout3" | "borderCallout1" | "borderCallout2" | "borderCallout3" | "accentBorderCallout1" | "accentBorderCallout2"
        | "accentBorderCallout3" => "callout".to_string(),
        "trapezoid" | "nonIsoscelesTrapezoid" | "flowChartManualOperation" => "trapezoid".to_string(),
        "leftRightArrow" => "left-right-arrow".to_string(),
        "parallelogram" | "flowChartInputOutput" => "parallelogram".to_string(),
        "hexagon" | "flowChartPreparation" => "hexagon".to_string(),
        "octagon" => "octagon".to_string(),
        "plus" => "cross".to_string(),
        "pentagon" | "homePlate" => "pentagon".to_string(),
        "heart" => "heart".to_string(),
        "lightningBolt" => "lightning".to_string(),
        "leftArrow" => "left-arrow".to_string(),
        "rightArrow" | "flowChartExtract" => "right-arrow".to_string(),
        "upArrow" => "up-arrow".to_string(),
        "downArrow" => "down-arrow".to_string(),
        "upDownArrow" => "up-down-arrow".to_string(),
        "quadArrow" => "quad-arrow".to_string(),
        "chevron" => "chevron".to_string(),
        "notchedRightArrow" => "notched-right-arrow".to_string(),
        "stripedRightArrow" => "striped-right-arrow".to_string(),
        "donut" => "ring".to_string(),
        "flowChartMerge" => "triangle-down".to_string(),
        "flowChartProcess" => "rect".to_string(),
        "flowChartConnector" => "ellipse".to_string(),
        other => other.to_string(),
    }
}

/// Builds a `Shape` from one `<p:sp>`'s inner XML -- called only when the shape has
/// no text at all (see `extract_slide_elements`'s dispatch: a shape with real text
/// keeps becoming a `TextBlock`, same as before this was added) and its own
/// `<a:prstGeom prst="...">` isn't `"line"` (that case is a `Line`, see
/// `extract_line_from_autoshape_element`). `None` only when the shape has no
/// `<a:prstGeom>` at all (not a real autoshape -- a placeholder with nothing drawn).
fn extract_autoshape_element(shape_inner: &str, z_index: i32, slide_w: f64, slide_h: f64, remap: Option<&GroupRemap>) -> Option<crate::core::models::SlideElement> {
    let (_, sppr_content, _) = next_tag(shape_inner, "p:spPr")?;
    let (geom_attrs, _, _) = next_tag(sppr_content, "a:prstGeom")?;
    let prst = extract_attr(geom_attrs, "prst")?;
    let kind = map_prst_geom_type(prst);
    let style = extract_shape_style(sppr_content);
    let mut transform = extract_shape_transform(shape_inner, slide_w, slide_h, remap);
    transform.z_index = z_index;
    let stroke_width = if style.stroke_color.is_some() {
        emu_to_relative_stroke(style.stroke_width_emu, transform.w * slide_w, transform.h * slide_h)
    } else {
        0.0
    };
    Some(crate::core::models::SlideElement::Shape {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform,
        shape_kind: kind,
        fill_color: style.fill_color,
        stroke_color: style.stroke_color,
        stroke_width,
    })
}

/// Builds a `Line` from a plain `<p:sp>` whose `<a:prstGeom prst="line">` (how
/// LibreOffice's own OOXML export represents a simple straight line shape --
/// confirmed against real output, not assumed) -- its bounding box is read the same
/// way as any other shape (`line_tool.ts`'s own "straight" rendering draws a
/// horizontal midline within whatever box it's given, so the box alone is enough).
fn extract_line_from_autoshape_element(shape_inner: &str, z_index: i32, slide_w: f64, slide_h: f64, remap: Option<&GroupRemap>) -> Option<crate::core::models::SlideElement> {
    let (_, sppr_content, _) = next_tag(shape_inner, "p:spPr")?;
    let style = extract_shape_style(sppr_content);
    let mut transform = extract_shape_transform(shape_inner, slide_w, slide_h, remap);
    transform.z_index = z_index;
    let stroke_width = line_stroke_width(&style, transform.w * slide_w, transform.h * slide_h);
    Some(crate::core::models::SlideElement::Line {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform,
        line_kind: "straight".to_string(),
        color: style.stroke_color.unwrap_or_else(|| "#000000".to_string()),
        stroke_width,
        start_arrow: style.start_arrow,
        end_arrow: style.end_arrow,
    })
}

/// Builds a `Line` from an actual `<p:cxnSp>` connector -- PowerPoint's own native
/// "Connector" tool, as opposed to LibreOffice's "`<p:sp>` with `prst=\"line\"`"
/// convention above (both are real, both appear in the wild, so both are handled).
/// `line_kind` comes from the connector's own preset family (Microsoft's published
/// `ShapeTypeValues`: `straightConnector1` vs `bentConnectorN` vs `curvedConnectorN`),
/// defaulting to `"straight"` for any connector preset not in that family.
fn extract_connector_element(cxn_inner: &str, z_index: i32, slide_w: f64, slide_h: f64, remap: Option<&GroupRemap>) -> Option<crate::core::models::SlideElement> {
    let (_, sppr_content, _) = next_tag(cxn_inner, "p:spPr")?;
    let prst = next_tag(sppr_content, "a:prstGeom").and_then(|(attrs, _, _)| extract_attr(attrs, "prst")).unwrap_or("straightConnector1");
    let line_kind = if prst.starts_with("bentConnector") {
        "elbow"
    } else if prst.starts_with("curvedConnector") {
        "curved"
    } else {
        "straight"
    };
    let style = extract_shape_style(sppr_content);
    let mut transform = extract_shape_transform(cxn_inner, slide_w, slide_h, remap);
    transform.z_index = z_index;
    let stroke_width = line_stroke_width(&style, transform.w * slide_w, transform.h * slide_h);
    Some(crate::core::models::SlideElement::Line {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform,
        line_kind: line_kind.to_string(),
        color: style.stroke_color.unwrap_or_else(|| "#000000".to_string()),
        stroke_width,
        start_arrow: style.start_arrow,
        end_arrow: style.end_arrow,
    })
}

/// Builds a `Table` from a `<p:graphicFrame>` wrapping an `<a:tbl>` -- column count
/// from `<a:tblGrid>`'s `<a:gridCol>` children, row count from `<a:tr>` children, a
/// cell's plain text (not its run-level formatting -- `Table`'s cells are plain
/// strings, same simplification `odp_import.rs`'s table handling already uses) from
/// its own `<a:txBody>`. A merged cell's continuation markers (`hMerge`/`vMerge`,
/// OOXML's equivalent of ODF's `table:covered-table-cell`) aren't real cells
/// (`a:tc` still appears for them, unlike ODF's distinctly-named covered cell, so
/// they're identified by those attributes instead and skipped); a row short a real
/// cell because of one is padded back out to the column count, not left short.
fn extract_table_element(gf_inner: &str, z_index: i32, slide_w: f64, slide_h: f64, remap: Option<&GroupRemap>) -> Option<crate::core::models::SlideElement> {
    // p:xfrm on a graphicFrame carries no rotation in practice, so only its content
    // (off/ext) is needed -- its own attributes are discarded, not read.
    let (_xfrm_attrs, xfrm_content, _) = next_tag(gf_inner, "p:xfrm")?;
    let (_, graphic_content, _) = next_tag(gf_inner, "a:graphic")?;
    let (_, graphic_data_content, _) = next_tag(graphic_content, "a:graphicData")?;
    let (_, tbl_content, _) = next_tag(graphic_data_content, "a:tbl")?;
    let (_, grid_content, _) = next_tag(tbl_content, "a:tblGrid")?;
    let cols = find_all_tag_attrs(grid_content, "a:gridCol").len().max(1);

    let mut cells: Vec<Vec<String>> = Vec::new();
    let mut rest = tbl_content;
    while let Some((_, row_inner, remainder)) = next_tag(rest, "a:tr") {
        rest = remainder;
        // A row always carries exactly one <a:tc> per grid column, full stop --
        // `gridSpan` on a merge's origin cell is purely descriptive of how many
        // columns it visually covers; it does NOT mean fewer <a:tc> elements are
        // present for those columns. Every column still gets its own <a:tc>, with
        // the covered ones marked hMerge="1" (horizontal) or vMerge="1" (vertical,
        // continuing a merge from an earlier row) instead of holding real content.
        // So each <a:tc>'s own ordinal position in the row *is* its column index --
        // gridSpan plays no part in placement and must never be used to skip
        // ahead, or every real cell after a merge would land one column too far
        // over instead of in its own real column.
        let mut row = vec![String::new(); cols];
        let mut r = row_inner;
        for slot in row.iter_mut() {
            let Some((cell_attrs, cell_inner, rem2)) = next_tag(r, "a:tc") else { break };
            r = rem2;
            if extract_attr(cell_attrs, "hMerge") != Some("1") && extract_attr(cell_attrs, "vMerge") != Some("1") {
                *slot = next_tag(cell_inner, "a:txBody").map(|(_, c, _)| plain_cell_text(c)).unwrap_or_default();
            }
        }
        cells.push(row);
    }
    if cells.is_empty() {
        return None;
    }
    let rows = cells.len();

    let mut transform = crate::core::models::ElementTransform::default();
    if let (Some((off_attrs, _, _)), Some((ext_attrs, _, _))) = (next_tag(xfrm_content, "a:off"), next_tag(xfrm_content, "a:ext")) {
        if let (Some(x), Some(y), Some(cx), Some(cy)) = (
            extract_attr(off_attrs, "x").and_then(|s| s.parse::<f64>().ok()),
            extract_attr(off_attrs, "y").and_then(|s| s.parse::<f64>().ok()),
            extract_attr(ext_attrs, "cx").and_then(|s| s.parse::<f64>().ok()),
            extract_attr(ext_attrs, "cy").and_then(|s| s.parse::<f64>().ok()),
        ) {
            let (x, y, cx, cy) = match remap {
                Some(r) => r.apply(x, y, cx, cy),
                None => (x, y, cx, cy),
            };
            transform.x = x / slide_w;
            transform.y = y / slide_h;
            transform.w = cx / slide_w;
            transform.h = cy / slide_h;
        }
    }
    transform = transform.normalized();
    transform.z_index = z_index;

    Some(crate::core::models::SlideElement::Table { id: format!("el_{}", uuid::Uuid::new_v4()), transform, rows, cols, cells })
}

/// Plain text (paragraphs joined with "\n") from a table cell's `<a:txBody>` --
/// concatenates every `<a:t>` per `<a:p>`, ignoring run-level formatting (see
/// `extract_table_element`'s doc).
fn plain_cell_text(txbody_content: &str) -> String {
    let mut paras = Vec::new();
    let mut rest = txbody_content;
    while let Some((_, p_inner, remainder)) = next_tag(rest, "a:p") {
        rest = remainder;
        let mut text = String::new();
        let mut t_rest = p_inner;
        while let Some((t_content, t_remainder)) = next_tag_content(t_rest, "a:t") {
            text.push_str(&decode_xml_entities(t_content));
            t_rest = t_remainder;
        }
        paras.push(text);
    }
    paras.join("\n")
}

/// Builds a `Group` from a `<p:grpSp>`'s already-isolated inner span (see
/// `next_slide_shape`'s use of `extract_balanced_tag`) -- reads the group's own
/// `<p:grpSpPr><a:xfrm>` off/ext/chOff/chExt into a `GroupRemap`, then recurses into
/// the same shape-scanning logic (`extract_shapes_in_scope`) for its children, each
/// remapped through it. Mirrors `odp_import.rs`'s `extract_group_element`: children
/// keep their own independent z-order (recursing starts a fresh 0-based sequence,
/// exactly like a nested slide), and the group's own `transform` is purely their
/// bounding box (selection convenience, never applied to render them -- see
/// `createElementNode`'s `'Group'` case in `web/src/core/slide_render.ts`).
#[allow(clippy::too_many_arguments)]
fn extract_group_element(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    slide_path: &str,
    grp_inner: &str,
    slide_w: f64,
    slide_h: f64,
    media_dir: &Path,
    outer_remap: Option<&GroupRemap>,
) -> Result<Option<crate::core::models::SlideElement>, Box<dyn std::error::Error>> {
    // A group's own transform is read on a strictly best-effort basis: unlike every
    // other shape here (which falls back to ElementTransform::default() and still
    // keeps the element when its own transform is missing/unparsable), a group with
    // no real transform information at all has nothing sensible to remap its
    // children *through* -- but still scans and keeps them, unremapped (their own
    // raw local coordinates used as-is), rather than silently dropping the whole
    // subtree over one malformed/missing piece. Real producers always emit a
    // group's own full off/ext/chOff/chExt, so this path is a safety net, not the
    // common case.
    let own_remap = (|| {
        let (_, grpsppr_content, _) = next_tag(grp_inner, "p:grpSpPr")?;
        let (_, xfrm_content, _) = next_tag(grpsppr_content, "a:xfrm")?;
        let (off_attrs, _, _) = next_tag(xfrm_content, "a:off")?;
        let (ext_attrs, _, _) = next_tag(xfrm_content, "a:ext")?;
        let (choff_attrs, _, _) = next_tag(xfrm_content, "a:chOff")?;
        let (chext_attrs, _, _) = next_tag(xfrm_content, "a:chExt")?;
        let parse = |attrs: &str, name: &str| extract_attr(attrs, name).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        // This group's own off/ext are expressed in whatever coordinate space it
        // itself sits in -- absolute slide EMU at the top level, but the OUTER
        // group's local space when this group is nested inside another one, which
        // must be remapped through that outer transform before this group's own
        // chOff/chExt (its children's local space, always self-contained, never
        // remapped) can be used to build the remap its own children need.
        let (mut off_x, mut off_y, mut ext_cx, mut ext_cy) = (parse(off_attrs, "x"), parse(off_attrs, "y"), parse(ext_attrs, "cx"), parse(ext_attrs, "cy"));
        if let Some(r) = outer_remap {
            let (x, y, cx, cy) = r.apply(off_x, off_y, ext_cx, ext_cy);
            off_x = x;
            off_y = y;
            ext_cx = cx;
            ext_cy = cy;
        }
        Some(GroupRemap {
            off_x,
            off_y,
            ext_cx,
            ext_cy,
            ch_off_x: parse(choff_attrs, "x"),
            ch_off_y: parse(choff_attrs, "y"),
            ch_ext_cx: parse(chext_attrs, "cx"),
            ch_ext_cy: parse(chext_attrs, "cy"),
        })
    })();

    let children = extract_shapes_in_scope(archive, slide_path, grp_inner, slide_w, slide_h, media_dir, own_remap.as_ref().or(outer_remap))?;
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
    let transform = crate::core::models::ElementTransform {
        x: min_x.clamp(0.0, 1.0),
        y: min_y.clamp(0.0, 1.0),
        w: (max_x - min_x).clamp(0.0, 1.0),
        h: (max_y - min_y).clamp(0.0, 1.0),
        ..Default::default()
    };
    Ok(Some(crate::core::models::SlideElement::Group { id: format!("el_{}", uuid::Uuid::new_v4()), transform, children }))
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
    extract_shapes_in_scope(archive, slide_path, sp_tree, slide_w, slide_h, media_dir, None)
}

/// The actual shape-scanning loop `extract_slide_elements` runs over a slide's own
/// `<p:spTree>` -- pulled into its own function, parameterized by an optional
/// `GroupRemap`, so `extract_group_element` can recurse into a `<p:grpSp>`'s own
/// inner content through the exact same logic (see that function's doc).
fn extract_shapes_in_scope(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    slide_path: &str,
    scope_xml: &str,
    slide_w: f64,
    slide_h: f64,
    media_dir: &Path,
    remap: Option<&GroupRemap>,
) -> Result<Vec<crate::core::models::SlideElement>, Box<dyn std::error::Error>> {
    let mut elements = Vec::new();
    let mut remaining = scope_xml;
    let mut z_index: i32 = 0;
    while let Some((shape, rest)) = next_slide_shape(remaining) {
        match shape {
            SlideShapeXml::Sp(inner) => {
                // Real text always wins first (a "line" preset shape can still carry
                // a <p:txBody>, legally -- nothing OOXML-side stops it), same
                // priority this importer already used before Shape/Line existed;
                // only once there's no text does prst="line" vs. anything else
                // decide whether the shape becomes a Line or a Shape.
                let el = extract_text_shape_element(inner, z_index, slide_w, slide_h, remap).or_else(|| {
                    let prst_line = next_tag(inner, "p:spPr")
                        .and_then(|(_, sppr, _)| next_tag(sppr, "a:prstGeom"))
                        .and_then(|(attrs, _, _)| extract_attr(attrs, "prst"))
                        == Some("line");
                    if prst_line {
                        extract_line_from_autoshape_element(inner, z_index, slide_w, slide_h, remap)
                    } else {
                        extract_autoshape_element(inner, z_index, slide_w, slide_h, remap)
                    }
                });
                if let Some(el) = el {
                    elements.push(el);
                    z_index += 1;
                }
            }
            SlideShapeXml::Picture(inner) => {
                if let Some(el) = extract_picture_element(archive, slide_path, inner, z_index, slide_w, slide_h, media_dir, remap)? {
                    elements.push(el);
                    z_index += 1;
                }
            }
            SlideShapeXml::Connector(inner) => {
                if let Some(el) = extract_connector_element(inner, z_index, slide_w, slide_h, remap) {
                    elements.push(el);
                    z_index += 1;
                }
            }
            SlideShapeXml::GraphicFrame(inner) => {
                if let Some(el) = extract_table_element(inner, z_index, slide_w, slide_h, remap) {
                    elements.push(el);
                    z_index += 1;
                }
            }
            SlideShapeXml::Group(inner) => {
                if let Some(el) = extract_group_element(archive, slide_path, inner, slide_w, slide_h, media_dir, remap)? {
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


