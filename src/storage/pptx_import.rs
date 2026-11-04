use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use zip::ZipArchive;

use crate::core::models::{GradientStop, Presentation, Slide, SlideBackground, SlideElement};
use crate::storage::pptx_inherit::{self as inh, children, child, Bullet, ClrMap, Inherited, Palette, Ph, RunProps, Theme};
use crate::storage::xml_scan::{decode_xml_entities, extract_attr, find_all_tag_attrs, next_tag, next_tag_content};

pub struct PptxImporter;

impl PptxImporter {
    /// Import a `.pptx` (Office Open XML Presentation) file into a `Presentation`, one positioned
    /// element per shape: a text shape becomes a `TextBlock`, a picture an `Image`, an autoshape a
    /// `Shape` (behind its text, when it has both), a line or connector a `Line`, a table a
    /// `Table`, a group a `Group` (docs/IMPORT_EXPORT_NOTES.md, "PPTX fidelity").
    ///
    /// Everything a slide inherits is resolved (`pptx_inherit`): real decks put almost nothing on
    /// the slide itself -- a placeholder carries its text and little else, and takes its position,
    /// font, size, colour, alignment, bullets and vertical anchor from its layout, the master, the
    /// master's text styles and the theme. Theme colours go through the colour map with their
    /// lumMod/lumOff/tint/shade modifiers; a shape's `<p:style>` references (fill, line, font
    /// colour) are honoured; the background comes from the slide, else its layout, else the
    /// master, including theme background styles (`bgRef`) and gradients; and the layout's and
    /// master's own decorations (logos, rules, bands) are drawn behind the slide's shapes unless
    /// `showMasterSp="0"` hides them.
    ///
    /// Geometry: this app's canvas is 16:9 and 1080 units tall, so a deck of another shape is
    /// pillarboxed (or letterboxed) rather than stretched, and point sizes are scaled from the
    /// deck's own slide height to that canvas (`Geo`).
    ///
    /// Still not reconstructed: charts and SmartArt, per-paragraph spacing, and a rotated group's
    /// children (only its linear offset/scale is applied, see `GroupRemap`).
    ///
    /// `media_dir` is where an extracted picture gets written -- pass the app's `web/media/images`
    /// so the resulting `/media/images/<file>` path is servable.
    pub fn import_pptx_bytes(bytes: &[u8], file_stem: &str, media_dir: &Path) -> Result<Presentation, Box<dyn std::error::Error>> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| format!("Not a valid .pptx (zip) file: {}", e))?;

        let slide_paths = Self::resolve_slide_order(&mut archive)?;
        if slide_paths.is_empty() {
            return Err("No slides found in .pptx file (not a valid PowerPoint package?)".into());
        }

        let presentation_xml = read_zip_text(&mut archive, "ppt/presentation.xml").unwrap_or_default();
        // Falls back to the standard 16:9 widescreen size when the package doesn't declare one.
        let (slide_w_emu, slide_h_emu) = parse_slide_size(&presentation_xml).unwrap_or((12192000.0, 6858000.0));
        let geo = Geo::new(slide_w_emu, slide_h_emu);
        let default_style = next_tag(&presentation_xml, "p:defaultTextStyle").map(|(_, i, _)| inh::ListStyle::parse(i)).unwrap_or_default();

        let (title_opt, author_opt) = read_zip_text(&mut archive, "docProps/core.xml")
            .ok()
            .map(|xml| (extract_xml_tag_text(&xml, "dc:title"), extract_xml_tag_text(&xml, "dc:creator")))
            .unwrap_or((None, None));

        let title = title_opt
            .filter(|t| !t.trim().is_empty() && !is_generic_title(t))
            .unwrap_or_else(|| file_stem.to_string());
        let author = author_opt.unwrap_or_default();

        let media = RefCell::new(HashMap::new());
        let mut layouts: HashMap<String, LayoutInfo> = HashMap::new();
        let mut slides = Vec::with_capacity(slide_paths.len());
        for (idx, path) in slide_paths.iter().enumerate() {
            let slide_xml = read_zip_text(&mut archive, path)
                .map_err(|e| format!("Could not read slide '{}': {}", path, e))?;

            let layout_path = rel_target_of_type(&mut archive, path, "/slideLayout");
            let info = match &layout_path {
                Some(lp) => {
                    if !layouts.contains_key(lp) {
                        let info = LayoutInfo::load(&mut archive, lp, &default_style);
                        layouts.insert(lp.clone(), info);
                    }
                    layouts[lp].clone()
                }
                None => LayoutInfo { inherited: Inherited { default_style: default_style.clone(), ..Default::default() }, ..Default::default() },
            };
            let map = info.map.overridden(&slide_xml);
            let ctx = Ctx { geo, theme: &info.theme, map, inh: &info.inherited, media: &media, media_dir };

            // Decorations first (paint order: master, then layout, then the slide's own shapes).
            let mut elements = Vec::new();
            let show_on_slide = shows_master_shapes(&slide_xml, "p:sld");
            if show_on_slide && shows_master_shapes(&info.layout_xml, "p:sldLayout") {
                if let Some(tree) = next_tag_content(&info.master_xml, "p:spTree").map(|(c, _)| c) {
                    elements.extend(extract_shapes_in_scope(&mut archive, &info.master_path, tree, &ctx, None, true)?);
                }
            }
            if show_on_slide {
                if let Some(tree) = next_tag_content(&info.layout_xml, "p:spTree").map(|(c, _)| c) {
                    elements.extend(extract_shapes_in_scope(&mut archive, &info.layout_path, tree, &ctx, None, true)?);
                }
            }
            let tree = next_tag_content(&slide_xml, "p:spTree").map(|(c, _)| c).unwrap_or(&slide_xml);
            elements.extend(extract_shapes_in_scope(&mut archive, path, tree, &ctx, None, false)?);
            for (z, el) in elements.iter_mut().enumerate() {
                el.transform_mut().z_index = z as i32;
            }

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
                // Keeps the legacy flat `text` field (search indexing, etc.) in step with the elements.
                slide.project_text_from_elements();
            }

            let parts = [(path.as_str(), slide_xml.as_str()), (info.layout_path.as_str(), info.layout_xml.as_str()), (info.master_path.as_str(), info.master_xml.as_str())];
            if let Some(bg) = resolve_background(&mut archive, &parts, &ctx) {
                slide.background = Some(match &bg {
                    SlideBackground::Solid(c) => c.clone(),
                    SlideBackground::Image { file_path, .. } => file_path.clone(),
                    SlideBackground::Gradient { kind, stops, angle_deg } => gradient_css(kind, stops, *angle_deg),
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

/// The deck's own name for itself is often just the tool's default ("PowerPoint Presentation")
/// or the first slide's ("Slide 1", "Folie 1"); the file name says more.
fn is_generic_title(t: &str) -> bool {
    let t = t.trim().to_lowercase();
    if matches!(t.as_str(), "powerpoint presentation" | "powerpoint-präsentation" | "presentation" | "présentation powerpoint" | "presentación de powerpoint") {
        return true;
    }
    let mut words = t.split_whitespace();
    matches!((words.next(), words.next(), words.next()), (Some("slide" | "folie" | "diapositiva" | "diapositive" | "dia"), Some(n), None) if n.chars().all(|c| c.is_ascii_digit()))
}

/// Where a deck's slides land on this app's 16:9 canvas, and how big its points are there.
#[derive(Clone, Copy)]
struct Geo {
    canvas_w: f64,
    canvas_h: f64,
    x_off: f64,
    y_off: f64,
    /// Multiplier from the deck's points to this app's: text is sized against a 1080-unit-tall
    /// canvas where 1pt is 4/3 of a unit (web/src/core/slide_render.ts), so 11.25 / the canvas
    /// height in inches -- 1.5 for a standard 7.5in-tall slide.
    font_scale: f64,
}

impl Geo {
    fn new(slide_w: f64, slide_h: f64) -> Geo {
        let (canvas_w, canvas_h) = if slide_w / slide_h < 16.0 / 9.0 { (slide_h * 16.0 / 9.0, slide_h) } else { (slide_w, slide_w * 9.0 / 16.0) };
        Geo { canvas_w, canvas_h, x_off: (canvas_w - slide_w) / 2.0, y_off: (canvas_h - slide_h) / 2.0, font_scale: 11.25 / (canvas_h / 914_400.0) }
    }

    /// An EMU rectangle on the slide as a canvas-relative `ElementTransform`. Not clamped to the
    /// canvas: a rotated shape's unrotated box can legitimately start off the slide (a long thin
    /// rule turned on end), and clamping it would move the shape.
    fn transform(&self, [x, y, cx, cy]: [f64; 4], rotation_deg: f64) -> crate::core::models::ElementTransform {
        crate::core::models::ElementTransform {
            x: (x + self.x_off) / self.canvas_w,
            y: (y + self.y_off) / self.canvas_h,
            w: (cx / self.canvas_w).max(0.0),
            h: (cy / self.canvas_h).max(0.0),
            rotation_deg,
            ..Default::default()
        }
    }
}

/// A layout, its master and the master's theme -- parsed once per layout, shared by its slides.
#[derive(Clone, Default)]
struct LayoutInfo {
    layout_path: String,
    layout_xml: String,
    master_path: String,
    master_xml: String,
    theme: Theme,
    /// The master's colour map with the layout's override applied.
    map: ClrMap,
    inherited: Inherited,
}

impl LayoutInfo {
    fn load(archive: &mut ZipArchive<Cursor<&[u8]>>, layout_path: &str, default_style: &inh::ListStyle) -> LayoutInfo {
        let layout_xml = read_zip_text(archive, layout_path).unwrap_or_default();
        let master_path = rel_target_of_type(archive, layout_path, "/slideMaster").unwrap_or_default();
        let master_xml = read_zip_text(archive, &master_path).unwrap_or_default();
        let theme_path = rel_target_of_type(archive, &master_path, "/theme").unwrap_or_default();
        let theme = Theme::parse(&read_zip_text(archive, &theme_path).unwrap_or_default(), &theme_path);
        let master_map = next_tag(&master_xml, "p:clrMap").map(|(a, _, _)| ClrMap::from_attrs(a)).unwrap_or_default();
        let inherited = Inherited {
            layout_phs: inh::part_placeholders(&layout_xml),
            master_phs: inh::part_placeholders(&master_xml),
            title_style: inh::master_text_style(&master_xml, "p:titleStyle"),
            body_style: inh::master_text_style(&master_xml, "p:bodyStyle"),
            other_style: inh::master_text_style(&master_xml, "p:otherStyle"),
            default_style: default_style.clone(),
        };
        LayoutInfo { map: master_map.overridden(&layout_xml), layout_path: layout_path.to_string(), layout_xml, master_path, master_xml, theme, inherited }
    }
}

/// `showMasterSp="0"` on a slide (or layout) hides the shapes it would otherwise inherit.
fn shows_master_shapes(part_xml: &str, root: &str) -> bool {
    next_tag(part_xml, root).and_then(|(attrs, _, _)| extract_attr(attrs, "showMasterSp")).is_none_or(|v| v != "0" && v != "false")
}

/// Everything shape extraction needs for one slide.
struct Ctx<'a> {
    geo: Geo,
    theme: &'a Theme,
    map: ClrMap,
    inh: &'a Inherited,
    /// Package media path -> the file it was written to, so a master's logo is stored once, not per slide.
    media: &'a RefCell<HashMap<String, String>>,
    media_dir: &'a Path,
}

impl Ctx<'_> {
    fn palette(&self) -> Palette<'_> {
        Palette { theme: self.theme, map: &self.map }
    }

    /// `+mj-lt` / `+mn-lt` are the theme's heading and body fonts.
    fn font(&self, typeface: Option<&str>) -> String {
        let name = match typeface {
            Some("+mj-lt") => self.theme.major_font.as_deref(),
            Some("+mn-lt") | None => self.theme.minor_font.as_deref(),
            Some(f) => Some(f),
        };
        css_font_stack(name.unwrap_or("Calibri"))
    }
}

/// A font name as a CSS stack: the metric-compatible free fonts for Microsoft's (what a Linux
/// machine has instead), then a generic family so a missing font never falls back to the
/// browser's default serif.
fn css_font_stack(name: &str) -> String {
    let substitute = match name.to_ascii_lowercase().as_str() {
        "calibri" => Some("Carlito"),
        "cambria" => Some("Caladea"),
        "arial" | "helvetica" => Some("Liberation Sans"),
        "times new roman" | "times" => Some("Liberation Serif"),
        "courier new" => Some("Liberation Mono"),
        _ => None,
    };
    let lower = name.to_ascii_lowercase();
    let generic = if ["times", "georgia", "garamond", "cambria", "serif", "book", "palatino"].iter().any(|s| lower.contains(s)) && !lower.contains("sans") {
        "serif"
    } else if lower.contains("mono") || lower.contains("courier") || lower.contains("consolas") {
        "monospace"
    } else {
        "sans-serif"
    };
    let quote = |n: &str| if n.contains(' ') { format!("\"{n}\"") } else { n.to_string() };
    match substitute {
        Some(s) => format!("{}, {}, {}", quote(name), quote(s), generic),
        None => format!("{}, {}", quote(name), generic),
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

/// The package path of `part_path`'s first relationship whose type ends with `type_suffix`
/// (`/slideLayout`, `/slideMaster`, `/theme`).
fn rel_target_of_type(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, type_suffix: &str) -> Option<String> {
    let rels = read_zip_text(archive, &slide_rels_path(part_path)).ok()?;
    let target = find_all_tag_attrs(&rels, "Relationship")
        .into_iter()
        .find(|a| extract_attr(a, "Type").is_some_and(|t| t.ends_with(type_suffix)) && extract_attr(a, "TargetMode") != Some("External"))
        .and_then(|a| extract_attr(a, "Target"))?
        .to_string();
    let dir = part_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    Some(resolve_relative_path(dir, &target))
}

/// The package path and bytes of the picture behind relationship `rid` of `part_path`.
fn embedded_bytes(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, rid: &str) -> Option<(String, Vec<u8>)> {
    let rels_xml = read_zip_text(archive, &slide_rels_path(part_path)).ok()?;
    let target = extract_rel_targets(&rels_xml).get(rid)?.clone();
    let dir = part_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("ppt/slides");
    let media_path = resolve_relative_path(dir, &target);
    let bytes = read_zip_bytes(archive, &media_path).ok()?;
    Some((media_path, bytes))
}

/// Extracts the picture behind relationship `rid` of `part_path` into the media folder (once per
/// package picture, however many slides show it) and returns its served path.
fn extract_embedded_image(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, rid: &str, ctx: &Ctx) -> Option<String> {
    let (media_path, bytes) = embedded_bytes(archive, part_path, rid)?;
    if let Some(done) = ctx.media.borrow().get(&media_path) {
        return Some(done.clone());
    }
    // Extension comes from the bytes, not the archive entry's name -- see storage::media_sniff.
    let name = crate::storage::media_sniff::write_sniffed_media(ctx.media_dir, "pptx", &bytes)?;
    let served = format!("/media/images/{}", name);
    ctx.media.borrow_mut().insert(media_path, served.clone());
    Some(served)
}

fn extract_xml_tag_text(xml: &str, tag: &str) -> Option<String> {
    let (content, _) = next_tag_content(xml, tag)?;
    Some(decode_xml_entities(content))
}

/// Reads `<p:sldSz cx=".." cy=".."/>` from `ppt/presentation.xml` -- the slide size in EMU
/// (914400 per inch).
fn parse_slide_size(presentation_xml: &str) -> Option<(f64, f64)> {
    let (attrs, _, _) = next_tag(presentation_xml, "p:sldSz")?;
    let cx = extract_attr(attrs, "cx")?.parse::<f64>().ok()?;
    let cy = extract_attr(attrs, "cy")?.parse::<f64>().ok()?;
    if cx <= 0.0 || cy <= 0.0 {
        return None;
    }
    Some((cx, cy))
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


// ---------------------------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------------------------

/// PowerPoint's default text box insets: 0.1in left/right, 0.05in top/bottom.
const DEFAULT_INSETS: [f64; 4] = [91_440.0, 45_720.0, 91_440.0, 45_720.0];

/// A shape's own rectangle in slide EMU (remapped out of its group), if it states one.
fn own_rect(shape_inner: &str, remap: Option<&GroupRemap>) -> Option<([f64; 4], f64)> {
    let (_, sppr, _) = next_tag(shape_inner, "p:spPr").or_else(|| next_tag(shape_inner, "p:grpSpPr"))?;
    let (xfrm_attrs, _, _) = next_tag(sppr, "a:xfrm")?;
    let [x, y, cx, cy] = inh::shape_xfrm(shape_inner)?;
    let (x, y, cx, cy) = match remap {
        Some(r) => r.apply(x, y, cx, cy),
        None => (x, y, cx, cy),
    };
    // PPTX angles are in 60,000ths of a degree.
    let rot = extract_attr(xfrm_attrs, "rot").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0) / 60_000.0;
    Some(([x, y, cx, cy], rot))
}

/// A shape's rectangle: its own, else (a placeholder) the one its layout or master gives it.
fn shape_rect(shape_inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>) -> Option<([f64; 4], f64)> {
    own_rect(shape_inner, remap).or_else(|| ctx.inh.for_shape(Ph::of_shape(shape_inner).as_ref()).0.map(|r| (r, 0.0)))
}

/// One finished run's formatting, resolved to concrete values.
fn text_run(text: String, p: &RunProps, ctx: &Ctx, base_color: Option<&str>, size_scale: f64) -> crate::core::models::TextRun {
    let pal = ctx.palette();
    let color = p.color.as_deref().and_then(|c| pal.first_color(c, None))
        .or_else(|| base_color.map(str::to_string))
        .or_else(|| pal.color("a:schemeClr", r#"val="tx1""#, "", None))
        .unwrap_or_else(|| "#000000".to_string());
    let size = p.size_pt.unwrap_or(18.0) * size_scale * ctx.geo.font_scale;
    crate::core::models::TextRun {
        text,
        bold: p.bold.unwrap_or(false),
        italic: p.italic.unwrap_or(false),
        underline: p.underline.unwrap_or(false),
        strike: p.strike.unwrap_or(false),
        color,
        font_family: ctx.font(p.font.as_deref()),
        font_size_pt: (size * 10.0).round() / 10.0,
        baseline_shift: match p.baseline {
            Some(b) if b > 0.0 => "super".to_string(),
            Some(b) if b < 0.0 => "sub".to_string(),
            _ => "normal".to_string(),
        },
        ..Default::default()
    }
}

/// Builds a `TextBlock` from one `<p:sp>`, resolving every paragraph's level through the
/// inheritance chain (master text style -> master placeholder -> layout placeholder -> the
/// shape's own list style -> the paragraph -> the run). `None` when there's no text at all.
///
/// This app keeps one paragraph style per text block, so per-paragraph bullets and indents are
/// written into the text the way the slide editor itself writes them (`formatBulletLines`): the
/// bullet character and an indent of en spaces per level.
fn extract_text_shape_element(shape_inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>) -> Option<SlideElement> {
    let (_, txbody, _) = next_tag(shape_inner, "p:txBody")?;
    let ph = Ph::of_shape(shape_inner);
    let (inherited_rect, inherited_list, inherited_body) = ctx.inh.for_shape(ph.as_ref());
    let (own_list, own_body) = inh::shape_text_styles(shape_inner);
    let list = own_list.over(&inherited_list);
    let body = own_body.over(&inherited_body);
    let pal = ctx.palette();
    // An autoshape's <p:style><a:fontRef> gives its text a colour (white on a filled box, typically).
    let style_color = next_tag(shape_inner, "p:style").and_then(|(_, s, _)| next_tag(s, "a:fontRef")).and_then(|(_, i, _)| pal.first_color(i, None));
    let size_scale = body.font_scale.unwrap_or(1.0);

    let mut runs: Vec<crate::core::models::TextRun> = Vec::new();
    let mut first_level: Option<inh::Level> = None;
    // This app has one alignment per text box: the one most of the text uses.
    let mut align_weight: HashMap<String, usize> = HashMap::new();
    let mut counters = [0usize; 9];
    let mut any_text = false;
    for (pi, (_, _, p_inner)) in children(txbody).into_iter().filter(|(n, _, _)| *n == "a:p").enumerate() {
        let (ppr_attrs, ppr_inner) = child(p_inner, "a:pPr").map(|(_, a, i)| (a, i)).unwrap_or(("", ""));
        let lvl = extract_attr(ppr_attrs, "lvl").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0).min(8);
        let level = inh::Level::parse(ppr_attrs, ppr_inner).over(&list.0[lvl]);

        let mut para: Vec<crate::core::models::TextRun> = Vec::new();
        for (name, _, inner) in children(p_inner) {
            let rpr = child(inner, "a:rPr").map(|(_, a, i)| RunProps::parse(a, i)).unwrap_or_default().over(&level.run);
            match name {
                "a:r" | "a:fld" => {
                    let text: String = children(inner).into_iter().filter(|(n, _, _)| *n == "a:t").map(|(_, _, t)| decode_xml_entities(t)).collect();
                    if !text.is_empty() {
                        para.push(text_run(text, &rpr, ctx, style_color.as_deref(), size_scale));
                    }
                }
                // A line break inside a paragraph (Shift+Enter), not a new paragraph.
                "a:br" => para.push(text_run("\n".into(), &rpr, ctx, style_color.as_deref(), size_scale)),
                _ => {}
            }
        }

        if pi > 0 {
            let mut sep = para.first().cloned().unwrap_or_else(|| text_run(String::new(), &level.run, ctx, style_color.as_deref(), size_scale));
            sep.text = "\n".into();
            runs.push(sep);
        }
        let has_text = para.iter().any(|r| !r.text.trim().is_empty());
        if has_text {
            any_text = true;
            if first_level.is_none() {
                first_level = Some(level.clone());
            }
            *align_weight.entry(level.align.clone().unwrap_or_else(|| "l".into())).or_default() += para.iter().map(|r| r.text.chars().count()).sum::<usize>();
            let marker = match &level.bullet {
                Some(Bullet::Char(c)) => Some(inh::bullet_glyph(c, level.bullet_font.as_deref())),
                Some(Bullet::AutoNum(kind)) => {
                    counters[lvl] += 1;
                    Some(inh::autonum_label(kind, counters[lvl]))
                }
                _ => None,
            };
            if !matches!(level.bullet, Some(Bullet::AutoNum(_))) {
                counters[lvl] = 0;
            }
            counters.iter_mut().skip(lvl + 1).for_each(|c| *c = 0);
            // Indent by the level's own left margin beyond the first level's, in en spaces (half
            // the font size each), so levels a deck gives the same margin line up as they do there.
            let en_emu = level.run.size_pt.unwrap_or(18.0) * size_scale * 12_700.0 / 2.0;
            let indent_emu = match (level.mar_l, list.0[0].mar_l) {
                (Some(m), Some(base)) => (m - base).max(0.0),
                (Some(m), None) => m,
                _ => lvl as f64 * 2.0 * en_emu,
            };
            let indent = "\u{2002}".repeat((indent_emu / en_emu).round() as usize);
            if marker.is_some() || !indent.is_empty() {
                let mut lead = para[0].clone();
                lead.text = format!("{indent}{}", marker.map(|m| format!("{m} ")).unwrap_or_default());
                lead.bold = false;
                lead.italic = false;
                lead.underline = false;
                if let Some(c) = level.bullet_color.as_deref().and_then(|c| pal.first_color(c, None)) {
                    lead.color = c;
                }
                runs.push(lead);
            }
        }
        runs.extend(para);
    }
    if !any_text {
        return None;
    }

    let level = first_level.unwrap_or_default();
    let majority = align_weight.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0))).map(|(a, _)| a);
    let align = match majority.as_deref() {
        Some("ctr") => "center",
        Some("r") => "right",
        Some("just") | Some("justLow") | Some("dist") => "justify",
        _ => "left",
    };
    // PowerPoint's single spacing is about 1.2 lines of the font size.
    let line_height = ((level.line_spacing.unwrap_or(1.0) - body.line_reduction.unwrap_or(0.0)).max(0.5) * 1.2 * 100.0).round() / 100.0;
    let vertical_align = match body.anchor.as_deref() {
        Some("ctr") => "middle",
        Some("b") => "bottom",
        _ => "top",
    };

    let (rect, rot) = own_rect(shape_inner, remap).or_else(|| inherited_rect.map(|r| (r, 0.0)))
        .unwrap_or(([ctx.geo.canvas_w * 0.05 - ctx.geo.x_off, ctx.geo.canvas_h * 0.07 - ctx.geo.y_off, ctx.geo.canvas_w * 0.9, ctx.geo.canvas_h * 0.86], 0.0));
    // The text sits inside the box's insets; this app's text box has none, so the box shrinks.
    let ins: Vec<f64> = body.insets.iter().zip(DEFAULT_INSETS).map(|(v, d)| v.unwrap_or(d)).collect();
    let [x, y, cx, cy] = rect;
    let inner = [x + ins[0], y + ins[1], (cx - ins[0] - ins[2]).max(cx * 0.2), (cy - ins[1] - ins[3]).max(cy * 0.2)];

    Some(SlideElement::TextBlock {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform: ctx.geo.transform(inner, rot),
        block: crate::core::models::TextBlock {
            runs,
            paragraph_style: crate::core::models::TextParagraphStyle {
                align: align.to_string(),
                line_height,
                vertical_align: Some(vertical_align.to_string()),
                ..Default::default()
            },
            effects: crate::core::models::ElementEffects::default(),
            // Unchecked (docs/IMPORT_EXPORT_NOTES.md "Follow-on decision: autofit checkbox"):
            // imported text keeps its authored size and place; the editor's Autofit opts in.
            autofit: false,
        },
    })
}

// ---------------------------------------------------------------------------------------------
// Pictures, shapes, lines, tables, groups
// ---------------------------------------------------------------------------------------------

/// Builds an `Image` from one `<p:pic>` -- resolves its `<a:blip r:embed>` through the part's own
/// relationships and extracts the picture. `None` (never an error) when it can't be resolved (a
/// linked-not-embedded picture, a broken relationship), so one bad picture doesn't fail the slide.
fn extract_picture_element(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, pic_inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>) -> Option<SlideElement> {
    let rid = next_tag(pic_inner, "a:blip").and_then(|(attrs, _, _)| extract_attr(attrs, "r:embed"))?.to_string();
    let (rect, rot) = shape_rect(pic_inner, ctx, remap)?;
    // A Windows Metafile (old clip art, drawn figures) can't be shown as a picture: its drawing
    // becomes elements instead.
    if let Some((_, bytes)) = embedded_bytes(archive, part_path, &rid).filter(|(_, b)| crate::storage::wmf::is_wmf(b)) {
        return metafile_group(&bytes, rect, rot, ctx);
    }
    let file_path = extract_embedded_image(archive, part_path, &rid, ctx)?;
    // <a:srcRect> crops in thousandths of a percent from each edge.
    let crop = next_tag(pic_inner, "a:srcRect").map(|(a, _, _)| {
        let f = |k: &str| extract_attr(a, k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100_000.0;
        let (l, t, r, b) = (f("l"), f("t"), f("r"), f("b"));
        crate::core::models::CropRect { x: l, y: t, w: (1.0 - l - r).clamp(0.01, 1.0), h: (1.0 - t - b).clamp(0.01, 1.0) }
    }).filter(|c| c.x != 0.0 || c.y != 0.0 || c.w != 1.0 || c.h != 1.0);
    Some(SlideElement::Image { id: format!("el_{}", uuid::Uuid::new_v4()), transform: ctx.geo.transform(rect, rot), file_path, crop, mask_shape: None, alt_text: None })
}

/// `Shape`/`Line`'s `stroke_width` is a percent of the shape's own box (its shorter side) -- both
/// `shape_library.ts` and `line_tool.ts` draw into a fixed `0 0 100 100` viewBox.
fn emu_to_relative_stroke(width_emu: f64, box_w_emu: f64, box_h_emu: f64) -> f64 {
    let base = box_w_emu.min(box_h_emu);
    if base <= 0.0 {
        return 0.0;
    }
    (width_emu / base) * 100.0
}

/// A shape's resolved fill/stroke/arrow-marker state.
struct ShapeStyle {
    fill_color: String,
    stroke_color: Option<String>,
    stroke_width_emu: f64,
    start_arrow: bool,
    end_arrow: bool,
}

/// A fill element's colour: a solid fill's, a gradient's first stop's, a pattern's foreground.
fn fill_color(name: &str, inner: &str, ctx: &Ctx, ph: Option<&str>) -> Option<String> {
    let pal = ctx.palette();
    match name {
        "a:solidFill" => pal.first_color(inner, ph),
        "a:gradFill" => next_tag(inner, "a:gs").and_then(|(_, gs, _)| pal.first_color(gs, ph)),
        "a:pattFill" => child(inner, "a:fgClr").and_then(|(_, _, i)| pal.first_color(i, ph)),
        _ => None,
    }
}

/// Reads a shape's fill and stroke: what its `<p:spPr>` states, else what its `<p:style>`'s
/// `fillRef`/`lnRef` take from the theme (index 0 meaning none). `<a:noFill/>` is "nothing here".
fn extract_shape_style(shape_inner: &str, ctx: &Ctx) -> ShapeStyle {
    let sppr = next_tag(shape_inner, "p:spPr").map(|(_, c, _)| c).unwrap_or("");
    let style = next_tag(shape_inner, "p:style").map(|(_, c, _)| c).unwrap_or("");
    let style_ref = |tag: &str| next_tag(style, tag).filter(|(a, _, _)| extract_attr(a, "idx").is_some_and(|i| i != "0")).and_then(|(_, i, _)| ctx.palette().first_color(i, None));

    let own: Vec<_> = children(sppr);
    let fill_el = own.iter().find(|(n, _, _)| matches!(*n, "a:noFill" | "a:solidFill" | "a:gradFill" | "a:pattFill" | "a:blipFill"));
    let fill = match fill_el {
        Some(("a:noFill", _, _)) => "none".to_string(),
        Some((n, _, i)) => fill_color(n, i, ctx, None).unwrap_or_else(|| "none".to_string()),
        None => style_ref("a:fillRef").unwrap_or_else(|| "none".to_string()),
    };

    let stroke_color;
    let mut stroke_width_emu = 0.0;
    let mut start_arrow = false;
    let mut end_arrow = false;
    let ln = own.iter().find(|(n, _, _)| *n == "a:ln");
    if let Some((_, ln_attrs, ln_inner)) = ln {
        stroke_width_emu = extract_attr(ln_attrs, "w").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        let ln_fill = children(ln_inner).into_iter().find(|(n, _, _)| matches!(*n, "a:noFill" | "a:solidFill" | "a:gradFill" | "a:pattFill"));
        stroke_color = match ln_fill {
            Some(("a:noFill", _, _)) => None,
            Some((n, _, i)) => fill_color(n, i, ctx, None),
            None => style_ref("a:lnRef"),
        };
        // headEnd = the line's first point, tailEnd = its last; type="none" is no marker.
        start_arrow = next_tag(ln_inner, "a:headEnd").map(|(a, _, _)| extract_attr(a, "type") != Some("none")).unwrap_or(false);
        end_arrow = next_tag(ln_inner, "a:tailEnd").map(|(a, _, _)| extract_attr(a, "type") != Some("none")).unwrap_or(false);
    } else {
        stroke_color = style_ref("a:lnRef");
    }
    if stroke_color.is_some() && stroke_width_emu == 0.0 {
        stroke_width_emu = 9_525.0; // PowerPoint's default 0.75pt line
    }
    ShapeStyle { fill_color: fill, stroke_color, stroke_width_emu, start_arrow, end_arrow }
}

/// SVG path data being built in an element's own 0..100 box.
#[derive(Default)]
struct PathData(String);

impl PathData {
    fn n(v: f64) -> String {
        let r = (v * 100.0).round() / 100.0;
        if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
    }
    fn cmd(&mut self, c: char, pts: &[(f64, f64)]) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        self.0.push(c);
        for (x, y) in pts {
            self.0.push_str(&format!(" {} {}", Self::n(*x), Self::n(*y)));
        }
    }
    fn arc(&mut self, rx: f64, ry: f64, large: bool, sweep: bool, to: (f64, f64)) {
        self.0.push_str(&format!(" A {} {} 0 {} {} {} {}", Self::n(rx), Self::n(ry), large as u8, sweep as u8, Self::n(to.0), Self::n(to.1)));
    }
    fn close(&mut self) {
        self.0.push_str(" Z");
    }
}

/// A line's width in canvas units (1080 to the canvas height), as path elements use.
fn canvas_stroke(width_emu: f64, ctx: &Ctx) -> f64 {
    ((width_emu.max(6_350.0) / ctx.geo.canvas_h * 1080.0) * 100.0).round() / 100.0
}

/// A shape's `<a:custGeom>` as path data in its own box: each `<a:path>` has its own coordinate
/// size (`w`/`h`, else the shape's), and commands moveTo, lnTo, cubicBezTo, quadBezTo, arcTo,
/// close. Returns the data and whether any part is filled (an outline with nothing filled is a
/// line, drawn with its arrowheads). Guide formulas aren't evaluated: a point that names one
/// makes the shape unreadable (`None`).
fn custom_geometry_path(cust: &str, ext: (f64, f64), flip: (bool, bool)) -> Option<(String, bool)> {
    let (_, path_lst, _) = next_tag(cust, "a:pathLst")?;
    let mut d = PathData::default();
    let mut filled = false;
    for (name, attrs, inner) in children(path_lst) {
        if name != "a:path" {
            continue;
        }
        let num = |k: &str| extract_attr(attrs, k).and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0);
        let (w, h) = (num("w").unwrap_or(ext.0), num("h").unwrap_or(ext.1));
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        filled |= extract_attr(attrs, "fill") != Some("none");
        let map = |(x, y): (f64, f64)| {
            let (mut x, mut y) = (x / w * 100.0, y / h * 100.0);
            if flip.0 { x = 100.0 - x }
            if flip.1 { y = 100.0 - y }
            (x, y)
        };
        let pts = |inner: &str| -> Option<Vec<(f64, f64)>> {
            children(inner).into_iter().filter(|(n, _, _)| *n == "a:pt").map(|(_, a, _)| {
                Some((extract_attr(a, "x")?.parse::<f64>().ok()?, extract_attr(a, "y")?.parse::<f64>().ok()?))
            }).collect()
        };
        let mut cur = (0.0, 0.0);
        for (cmd, a, i) in children(inner) {
            match cmd {
                "a:moveTo" | "a:lnTo" | "a:cubicBezTo" | "a:quadBezTo" => {
                    let p = pts(i)?;
                    let letter = match cmd { "a:moveTo" => 'M', "a:lnTo" => 'L', "a:cubicBezTo" => 'C', _ => 'Q' };
                    cur = *p.last()?;
                    d.cmd(letter, &p.into_iter().map(map).collect::<Vec<_>>());
                }
                "a:arcTo" => {
                    let g = |k: &str| extract_attr(a, k).and_then(|v| v.parse::<f64>().ok());
                    let (wr, hr, st, sw) = (g("wR")?, g("hR")?, g("stAng")? / 60_000.0, g("swAng")? / 60_000.0);
                    let (st_r, end_r) = (st.to_radians(), (st + sw).to_radians());
                    let center = (cur.0 - wr * st_r.cos(), cur.1 - hr * st_r.sin());
                    let end = (center.0 + wr * end_r.cos(), center.1 + hr * end_r.sin());
                    // A flip mirrors the arc, which turns its direction around (once per flip).
                    let sweep = (sw > 0.0) != (flip.0 != flip.1);
                    d.arc(wr / w * 100.0, hr / h * 100.0, sw.abs() > 180.0, sweep, map(end));
                    cur = end;
                }
                "a:close" => d.close(),
                _ => {}
            }
        }
    }
    (!d.0.is_empty()).then_some((d.0, filled))
}

fn xfrm_flips(shape_inner: &str) -> (bool, bool) {
    let attrs = next_tag(shape_inner, "p:spPr").and_then(|(_, sppr, _)| next_tag(sppr, "a:xfrm")).map(|(a, _, _)| a).unwrap_or("");
    (extract_attr(attrs, "flipH") == Some("1"), extract_attr(attrs, "flipV") == Some("1"))
}

/// Builds a `Shape` from a `<p:sp>`: a preset geometry, or a freeform (`<a:custGeom>`) as a path.
/// An unfilled freeform is a drawn line (an arrow, often) and comes back as a `Line`. `None` when
/// there's no geometry (a placeholder with nothing drawn) or it would draw nothing at all.
fn extract_autoshape_element(shape_inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>) -> Option<SlideElement> {
    let (_, sppr_content, _) = next_tag(shape_inner, "p:spPr")?;
    let style = extract_shape_style(shape_inner, ctx);
    if style.fill_color == "none" && style.stroke_color.is_none() {
        return None;
    }
    let (rect, rot) = shape_rect(shape_inner, ctx, remap)?;
    let id = format!("el_{}", uuid::Uuid::new_v4());
    let transform = ctx.geo.transform(rect, rot);
    if let Some((_, cust, _)) = next_tag(sppr_content, "a:custGeom") {
        let (d, filled) = custom_geometry_path(cust, (rect[2], rect[3]), xfrm_flips(shape_inner))?;
        let stroke_width = canvas_stroke(style.stroke_width_emu, ctx);
        if !filled || style.fill_color == "none" {
            let color = style.stroke_color.clone()?;
            return Some(SlideElement::Line { id, transform, line_kind: "curved".into(), color, stroke_width, start_arrow: style.start_arrow, end_arrow: style.end_arrow, path: Some(d) });
        }
        return Some(SlideElement::Shape { id, transform, shape_kind: "path".into(), fill_color: style.fill_color, stroke_width: if style.stroke_color.is_some() { stroke_width } else { 0.0 }, stroke_color: style.stroke_color, path: Some(d) });
    }
    let (geom_attrs, geom_inner, _) = next_tag(sppr_content, "a:prstGeom")?;
    let prst = extract_attr(geom_attrs, "prst")?;
    if prst == "arc" {
        // An open arc of the ellipse filling the box, clockwise from adj1 to adj2 (60000ths of a
        // degree; 270 to 0 when unset): a line, so it keeps its arrowheads.
        let adj = |name: &str, default: f64| {
            next_tag(geom_inner, "a:avLst").map(|(_, i, _)| children(i)).unwrap_or_default().into_iter()
                .find(|(n, a, _)| *n == "a:gd" && extract_attr(a, "name") == Some(name))
                .and_then(|(_, a, _)| extract_attr(a, "fmla")?.strip_prefix("val ")?.trim().parse::<f64>().ok())
                .unwrap_or(default) / 60_000.0
        };
        let (st, end) = (adj("adj1", 16_200_000.0), adj("adj2", 0.0));
        let sweep = (end - st).rem_euclid(360.0);
        let point = |deg: f64| (50.0 + 50.0 * deg.to_radians().cos(), 50.0 + 50.0 * deg.to_radians().sin());
        let (flip_h, flip_v) = xfrm_flips(shape_inner);
        let f = |(x, y): (f64, f64)| (if flip_h { 100.0 - x } else { x }, if flip_v { 100.0 - y } else { y });
        let mut d = PathData::default();
        d.cmd('M', &[f(point(st))]);
        d.arc(50.0, 50.0, sweep > 180.0, flip_h == flip_v, f(point(st + sweep)));
        let color = style.stroke_color.clone()?;
        return Some(SlideElement::Line { id, transform, line_kind: "curved".into(), color, stroke_width: canvas_stroke(style.stroke_width_emu, ctx), start_arrow: style.start_arrow, end_arrow: style.end_arrow, path: Some(d.0) });
    }
    let stroke_width = if style.stroke_color.is_some() { emu_to_relative_stroke(style.stroke_width_emu, rect[2], rect[3]) } else { 0.0 };
    Some(SlideElement::Shape { id, transform, shape_kind: map_prst_geom_type(prst), fill_color: style.fill_color, stroke_color: style.stroke_color, stroke_width, path: None })
}

/// Builds a `Line` from a `<p:sp prst="line">` or a `<p:cxnSp>` connector. Its course is drawn
/// corner to corner of its box (the connector's elbow or curve in between), turned by the shape's
/// flips -- a connector's box says where it starts and ends only together with `flipH`/`flipV`.
fn extract_line_element(inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>) -> Option<SlideElement> {
    let (_, sppr_content, _) = next_tag(inner, "p:spPr")?;
    let (geom_attrs, geom_inner) = next_tag(sppr_content, "a:prstGeom").map(|(a, i, _)| (a, i)).unwrap_or(("", ""));
    let prst = extract_attr(geom_attrs, "prst").unwrap_or("line");
    let adj = |name: &str, default: f64| {
        next_tag(geom_inner, "a:avLst").map(|(_, i, _)| children(i)).unwrap_or_default().into_iter()
            .find(|(n, a, _)| *n == "a:gd" && extract_attr(a, "name") == Some(name))
            .and_then(|(_, a, _)| extract_attr(a, "fmla")?.strip_prefix("val ")?.trim().parse::<f64>().ok())
            .map(|v| v / 1000.0)
            .unwrap_or(default)
    };
    let (flip_h, flip_v) = xfrm_flips(inner);
    let f = |(x, y): (f64, f64)| (if flip_h { 100.0 - x } else { x }, if flip_v { 100.0 - y } else { y });
    let mut d = PathData::default();
    d.cmd('M', &[f((0.0, 0.0))]);
    let line_kind = if prst.starts_with("bentConnector") {
        if prst == "bentConnector2" {
            d.cmd('L', &[f((100.0, 0.0)), f((100.0, 100.0))]);
        } else {
            let x = adj("adj1", 50.0);
            d.cmd('L', &[f((x, 0.0)), f((x, 100.0)), f((100.0, 100.0))]);
        }
        "elbow"
    } else if prst.starts_with("curvedConnector") {
        if prst == "curvedConnector2" {
            d.cmd('Q', &[f((100.0, 0.0)), f((100.0, 100.0))]);
        } else {
            let x = adj("adj1", 50.0);
            d.cmd('C', &[f((x, 0.0)), f((x, 100.0)), f((100.0, 100.0))]);
        }
        "curved"
    } else {
        d.cmd('L', &[f((100.0, 100.0))]);
        "straight"
    };
    let style = extract_shape_style(inner, ctx);
    let (rect, rot) = shape_rect(inner, ctx, remap)?;
    Some(SlideElement::Line {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform: ctx.geo.transform(rect, rot),
        line_kind: line_kind.to_string(),
        color: style.stroke_color.clone().unwrap_or_else(|| "#000000".to_string()),
        stroke_width: canvas_stroke(style.stroke_width_emu, ctx),
        start_arrow: style.start_arrow,
        end_arrow: style.end_arrow,
        path: Some(d.0),
    })
}

/// A Windows Metafile picture as a group of ordinary elements (`wmf`): each path a `Shape` (or a
/// `Line` when open), each string a `TextBlock`, each bitmap an `Image`, laid out over the
/// picture's own rectangle. `None` when the metafile draws nothing this can read.
fn metafile_group(bytes: &[u8], rect: [f64; 4], rot: f64, ctx: &Ctx) -> Option<SlideElement> {
    use crate::storage::wmf::WmfItem;
    let wmf = crate::storage::wmf::parse(bytes)?;
    let (fx, fy, fw, fh) = wmf.frame;
    let (sx, sy) = (rect[2] / fw, rect[3] / fh);
    let at = |(x, y): (f64, f64)| (rect[0] + (x - fx) * sx, rect[1] + (y - fy) * sy);
    let id = || format!("el_{}", uuid::Uuid::new_v4());
    // Path data in a box: points relative to the box, as 0..100.
    let local = |b: [f64; 4], (x, y): (f64, f64)| ((x - b[0]) / b[2] * 100.0, (y - b[1]) / b[3] * 100.0);
    let bbox = |pts: &mut dyn Iterator<Item = (f64, f64)>| {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in pts {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        // A perfectly flat drawing still needs a box to scale through.
        [x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0)]
    };
    let pen = |stroke: &Option<(String, f64)>| stroke.as_ref().map(|(c, w)| (c.clone(), canvas_stroke(w * (sx + sy) / 2.0, ctx)));
    let mut children = Vec::new();
    for item in wmf.items {
        match item {
            WmfItem::Path { subpaths, closed, fill, stroke } => {
                let abs: Vec<Vec<(f64, f64)>> = subpaths.iter().map(|s| s.iter().map(|p| at(*p)).collect()).collect();
                let b = bbox(&mut abs.iter().flatten().copied());
                let mut d = PathData::default();
                for s in &abs {
                    d.cmd('M', &[local(b, s[0])]);
                    d.cmd('L', &s[1..].iter().map(|p| local(b, *p)).collect::<Vec<_>>());
                    if closed {
                        d.close();
                    }
                }
                let stroke = pen(&stroke);
                let transform = ctx.geo.transform(b, 0.0);
                match (closed, fill) {
                    (true, fill) if fill.is_some() || stroke.is_some() => children.push(SlideElement::Shape {
                        id: id(), transform, shape_kind: "path".into(), fill_color: fill.unwrap_or_else(|| "none".into()),
                        stroke_width: stroke.as_ref().map_or(0.0, |s| s.1), stroke_color: stroke.map(|s| s.0), path: Some(d.0),
                    }),
                    (false, _) => if let Some((color, w)) = stroke {
                        children.push(SlideElement::Line { id: id(), transform, line_kind: "curved".into(), color, stroke_width: w, start_arrow: false, end_arrow: false, path: Some(d.0) });
                    },
                    _ => {}
                }
            }
            WmfItem::Arc { center, rx, ry, from, to, large, closed, pie, fill, stroke } => {
                let (c, f, t) = (at(center), at(from), at(to));
                let (rx, ry) = (rx * sx, ry * sy);
                let b = [c.0 - rx, c.1 - ry, (2.0 * rx).max(1.0), (2.0 * ry).max(1.0)];
                let (lrx, lry) = (rx / b[2] * 100.0, ry / b[3] * 100.0);
                let mut d = PathData::default();
                d.cmd('M', &[local(b, f)]);
                if (f.0 - t.0).abs() < 1e-6 && (f.1 - t.1).abs() < 1e-6 {
                    // A whole ellipse: two half arcs (one arc can't end where it starts).
                    let opposite = (2.0 * c.0 - f.0, 2.0 * c.1 - f.1);
                    d.arc(lrx, lry, false, false, local(b, opposite));
                    d.arc(lrx, lry, false, false, local(b, f));
                } else {
                    d.arc(lrx, lry, large, false, local(b, t));
                    if pie {
                        d.cmd('L', &[local(b, c)]);
                    }
                }
                if closed {
                    d.close();
                }
                let stroke = pen(&stroke);
                let transform = ctx.geo.transform(b, 0.0);
                if closed && (fill.is_some() || stroke.is_some()) {
                    children.push(SlideElement::Shape { id: id(), transform, shape_kind: "path".into(), fill_color: fill.unwrap_or_else(|| "none".into()), stroke_width: stroke.as_ref().map_or(0.0, |s| s.1), stroke_color: stroke.map(|s| s.0), path: Some(d.0) });
                } else if let Some((color, w)) = stroke {
                    children.push(SlideElement::Line { id: id(), transform, line_kind: "curved".into(), color, stroke_width: w, start_arrow: false, end_arrow: false, path: Some(d.0) });
                }
            }
            WmfItem::Text { x, y, text, size, color, font, bold, italic, align, baseline, angle_deg } => {
                let size_emu = size * sy;
                let (w, h) = (text.chars().count() as f64 * size_emu * 0.62 + size_emu, size_emu * 1.3);
                let (ax, ay) = at((x, y));
                let left = match align { "center" => ax - w / 2.0, "right" => ax - w, _ => ax };
                let top = if baseline { ay - size_emu * 0.85 } else { ay };
                let run = crate::core::models::TextRun {
                    text, bold, italic, color, font_family: css_font_stack(&font),
                    font_size_pt: ((size_emu / 12_700.0 * ctx.geo.font_scale) * 10.0).round() / 10.0,
                    ..Default::default()
                };
                children.push(SlideElement::TextBlock {
                    id: id(),
                    transform: ctx.geo.transform([left, top, w, h], -angle_deg),
                    block: crate::core::models::TextBlock {
                        runs: vec![run],
                        paragraph_style: crate::core::models::TextParagraphStyle { align: align.to_string(), line_height: 1.2, vertical_align: Some("top".into()), ..Default::default() },
                        effects: Default::default(),
                        autofit: false,
                    },
                });
            }
            WmfItem::Bitmap { x, y, w, h, bmp } => {
                let png = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp).ok().and_then(|img| {
                    let mut out = Cursor::new(Vec::new());
                    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
                    Some(out.into_inner())
                });
                if let Some(name) = png.and_then(|png| crate::storage::media_sniff::write_sniffed_media(ctx.media_dir, "pptx", &png)) {
                    let (x0, y0) = at((x, y));
                    children.push(SlideElement::Image { id: id(), transform: ctx.geo.transform([x0, y0, w * sx, h * sy], 0.0), file_path: format!("/media/images/{name}"), crop: None, mask_shape: None, alt_text: None });
                }
            }
        }
    }
    if children.is_empty() {
        return None;
    }
    for (z, c) in children.iter_mut().enumerate() {
        c.transform_mut().z_index = z as i32;
    }
    // The group's own box is the picture's; rotation of the whole picture isn't applied to its parts.
    let _ = rot;
    Some(SlideElement::Group { id: id(), transform: ctx.geo.transform(rect, 0.0), children })
}

/// Builds a `Table` from a `<p:graphicFrame>` wrapping an `<a:tbl>`: columns from `<a:tblGrid>`,
/// rows from `<a:tr>`, each cell's plain text. Every column has its own `<a:tc>` even under a
/// merge (the covered ones are marked `hMerge`/`vMerge`), so a cell's position in the row is its
/// column; covered cells stay empty.
fn extract_table_element(gf_inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>) -> Option<SlideElement> {
    let (_, xfrm_content, _) = next_tag(gf_inner, "p:xfrm")?;
    let (_, graphic_content, _) = next_tag(gf_inner, "a:graphic")?;
    let (_, graphic_data_content, _) = next_tag(graphic_content, "a:graphicData")?;
    let (_, tbl_content, _) = next_tag(graphic_data_content, "a:tbl")?;
    let (_, grid_content, _) = next_tag(tbl_content, "a:tblGrid")?;
    let cols = find_all_tag_attrs(grid_content, "a:gridCol").len().max(1);

    let mut cells: Vec<Vec<String>> = Vec::new();
    let mut rest = tbl_content;
    while let Some((_, row_inner, remainder)) = next_tag(rest, "a:tr") {
        rest = remainder;
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
    let (off, _, _) = next_tag(xfrm_content, "a:off")?;
    let (ext, _, _) = next_tag(xfrm_content, "a:ext")?;
    let n = |a: &str, k: &str| extract_attr(a, k).and_then(|s| s.parse::<f64>().ok());
    let (x, y, cx, cy) = (n(off, "x")?, n(off, "y")?, n(ext, "cx")?, n(ext, "cy")?);
    let (x, y, cx, cy) = match remap {
        Some(r) => r.apply(x, y, cx, cy),
        None => (x, y, cx, cy),
    };
    // Cells keep plain text, so one colour and size for the table: its first run's, else the
    // theme's text colour and PowerPoint's 18pt default. (Table styles aren't resolved.)
    let first = next_tag(tbl_content, "a:rPr").map(|(a, i, _)| RunProps::parse(a, i)).unwrap_or_default();
    let pal = ctx.palette();
    let text_color = first.color.as_deref().and_then(|c| pal.first_color(c, None))
        .or_else(|| pal.color("a:schemeClr", r#"val="tx1""#, "", None))
        .unwrap_or_else(|| "#000000".to_string());
    let font_size_pt = ((first.size_pt.unwrap_or(18.0) * ctx.geo.font_scale) * 10.0).round() / 10.0;
    Some(SlideElement::Table {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform: ctx.geo.transform([x, y, cx, cy], 0.0),
        rows,
        cols,
        cells,
        border_color: Some(text_color.clone()),
        text_color: Some(text_color),
        font_size_pt: Some(font_size_pt),
    })
}

/// Plain text (paragraphs joined with "\n") from a table cell's `<a:txBody>`.
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

/// Builds a `Group` from a `<p:grpSp>`: reads its off/ext/chOff/chExt into a `GroupRemap` and
/// recurses into its children through it. The group's own `transform` is just its children's
/// bounding box (see `createElementNode`'s `'Group'` case in `web/src/core/slide_render.ts`).
fn extract_group_element(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, grp_inner: &str, ctx: &Ctx, outer_remap: Option<&GroupRemap>, decorations_only: bool) -> Result<Option<SlideElement>, Box<dyn std::error::Error>> {
    // A group with no usable transform still keeps its children, unremapped, rather than losing them.
    let own_remap = (|| {
        let (_, grpsppr_content, _) = next_tag(grp_inner, "p:grpSpPr")?;
        let (_, xfrm_content, _) = next_tag(grpsppr_content, "a:xfrm")?;
        let (off_attrs, _, _) = next_tag(xfrm_content, "a:off")?;
        let (ext_attrs, _, _) = next_tag(xfrm_content, "a:ext")?;
        let (choff_attrs, _, _) = next_tag(xfrm_content, "a:chOff")?;
        let (chext_attrs, _, _) = next_tag(xfrm_content, "a:chExt")?;
        let parse = |attrs: &str, name: &str| extract_attr(attrs, name).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        // A nested group's own off/ext are in its parent group's local space, so they go through
        // the parent's remap first.
        let (mut off_x, mut off_y, mut ext_cx, mut ext_cy) = (parse(off_attrs, "x"), parse(off_attrs, "y"), parse(ext_attrs, "cx"), parse(ext_attrs, "cy"));
        if let Some(r) = outer_remap {
            (off_x, off_y, ext_cx, ext_cy) = r.apply(off_x, off_y, ext_cx, ext_cy);
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

    let children = extract_shapes_in_scope(archive, part_path, grp_inner, ctx, own_remap.as_ref().or(outer_remap), decorations_only)?;
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
    Ok(Some(SlideElement::Group { id: format!("el_{}", uuid::Uuid::new_v4()), transform, children }))
}

/// Walks a shape tree (a slide's, a layout's, a master's, or a group's) in document order --
/// PPTX's own back-to-front paint order -- emitting one element per shape that draws something.
/// `decorations_only` (a layout or master) skips placeholders: those are prompts the slide's own
/// placeholders replace, not things to draw. `z_index` is numbered densely by the caller.
fn extract_shapes_in_scope(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    part_path: &str,
    scope_xml: &str,
    ctx: &Ctx,
    remap: Option<&GroupRemap>,
    decorations_only: bool,
) -> Result<Vec<SlideElement>, Box<dyn std::error::Error>> {
    let mut elements = Vec::new();
    for (name, _, inner) in children(scope_xml) {
        if decorations_only && Ph::of_shape(inner).is_some() {
            continue;
        }
        match name {
            "p:sp" => {
                let prst = next_tag(inner, "p:spPr").and_then(|(_, sppr, _)| next_tag(sppr, "a:prstGeom")).and_then(|(attrs, _, _)| extract_attr(attrs, "prst"));
                let text = extract_text_shape_element(inner, ctx, remap);
                if prst == Some("line") {
                    // Text on a line-preset shape is legal, and the text is what matters.
                    elements.extend(text.or_else(|| extract_line_element(inner, ctx, remap)));
                    continue;
                }
                // A filled or outlined box with text in it is two elements: the box, then its text.
                elements.extend(extract_autoshape_element(inner, ctx, remap));
                elements.extend(text);
            }
            "p:pic" => elements.extend(extract_picture_element(archive, part_path, inner, ctx, remap)),
            "p:cxnSp" => elements.extend(extract_line_element(inner, ctx, remap)),
            "p:graphicFrame" => elements.extend(extract_table_element(inner, ctx, remap)),
            "p:grpSp" => elements.extend(extract_group_element(archive, part_path, inner, ctx, remap, decorations_only)?),
            // mc:AlternateContent wraps newer shapes with a fallback older readers understand.
            "mc:AlternateContent" => {
                if let Some((_, _, fallback)) = child(inner, "mc:Fallback") {
                    elements.extend(extract_shapes_in_scope(archive, part_path, fallback, ctx, remap, decorations_only)?);
                }
            }
            _ => {}
        }
    }
    Ok(elements)
}

// ---------------------------------------------------------------------------------------------
// Backgrounds
// ---------------------------------------------------------------------------------------------

/// The slide's background: the first `<p:bg>` of the slide, its layout, its master.
fn resolve_background(archive: &mut ZipArchive<Cursor<&[u8]>>, parts: &[(&str, &str)], ctx: &Ctx) -> Option<SlideBackground> {
    for (path, xml) in parts {
        let Some((_, bg, _)) = next_tag(xml, "p:bg") else { continue };
        if let Some((_, _, bg_pr)) = child(bg, "p:bgPr") {
            let (name, attrs, inner) = children(bg_pr).into_iter().next()?;
            return background_from_fill(archive, path, name, attrs, inner, ctx, None);
        }
        if let Some((attrs, inner, _)) = next_tag(bg, "p:bgRef") {
            // A reference into the theme's background styles (1001 and up) or fill styles,
            // with the colour it passes in as `phClr`.
            let idx = extract_attr(attrs, "idx").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
            let ph = ctx.palette().first_color(inner, None);
            let fills = if idx >= 1001 { &ctx.theme.bg_fills } else { &ctx.theme.fills };
            let i = if idx >= 1001 { idx - 1001 } else { idx.saturating_sub(1) };
            let Some(fill) = fills.get(i) else { return ph.map(SlideBackground::Solid) };
            let (name, attrs, inner) = children(fill).into_iter().next()?;
            let theme_path = ctx.theme.path.clone();
            return background_from_fill(archive, &theme_path, name, attrs, inner, ctx, ph.as_deref()).or(ph.map(SlideBackground::Solid));
        }
        return None;
    }
    None
}

fn background_from_fill(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, name: &str, _attrs: &str, inner: &str, ctx: &Ctx, ph: Option<&str>) -> Option<SlideBackground> {
    let pal = ctx.palette();
    match name {
        "a:solidFill" => pal.first_color(inner, ph).map(SlideBackground::Solid),
        "a:pattFill" => child(inner, "a:fgClr").and_then(|(_, _, i)| pal.first_color(i, ph)).map(SlideBackground::Solid),
        "a:gradFill" => {
            let mut stops: Vec<GradientStop> = next_tag(inner, "a:gsLst")
                .map(|(_, l, _)| children(l).into_iter().filter_map(|(_, a, i)| {
                    let pos = extract_attr(a, "pos").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100_000.0;
                    Some(GradientStop { color: pal.first_color(i, ph)?, offset: pos })
                }).collect())
                .unwrap_or_default();
            stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
            match stops.len() {
                0 => None,
                1 => Some(SlideBackground::Solid(stops.remove(0).color)),
                _ => {
                    if child(inner, "a:path").is_some() {
                        Some(SlideBackground::Gradient { kind: "radial".into(), stops, angle_deg: None })
                    } else {
                        // OOXML's 0deg runs left to right; CSS's 90deg does.
                        let ang = child(inner, "a:lin").and_then(|(_, a, _)| extract_attr(a, "ang")).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 60_000.0;
                        Some(SlideBackground::Gradient { kind: "linear".into(), stops, angle_deg: Some((ang + 90.0) % 360.0) })
                    }
                }
            }
        }
        "a:blipFill" => {
            let rid = next_tag(inner, "a:blip").and_then(|(a, _, _)| extract_attr(a, "r:embed"))?.to_string();
            extract_embedded_image(archive, part_path, &rid, ctx).map(|file_path| SlideBackground::Image { file_path, opacity: 1.0 })
        }
        _ => None,
    }
}

/// A gradient background as CSS, for the legacy `background` string.
fn gradient_css(kind: &str, stops: &[GradientStop], angle_deg: Option<f64>) -> String {
    let list = stops.iter().map(|s| format!("{} {}%", s.color, (s.offset * 100.0).round())).collect::<Vec<_>>().join(", ");
    if kind == "radial" {
        format!("radial-gradient(circle, {list})")
    } else {
        format!("linear-gradient({}deg, {list})", angle_deg.unwrap_or(180.0).round())
    }
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

