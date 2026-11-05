use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use zip::ZipArchive;

use crate::core::models::{GradientStop, Presentation, Slide, SlideBackground, SlideElement};
use crate::storage::pptx_geom;
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
    /// Still not reconstructed: a rotated group's children (only its linear offset/scale is
    /// applied, see `GroupRemap`), tiled picture fills, and EMF+-only pictures.
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
        let table_styles = read_zip_text(&mut archive, "ppt/tableStyles.xml").unwrap_or_default();
        let mut layouts: HashMap<String, std::rc::Rc<LayoutInfo>> = HashMap::new();
        let mut slides = Vec::with_capacity(slide_paths.len());
        for (idx, path) in slide_paths.iter().enumerate() {
            let slide_xml = read_zip_text(&mut archive, path)
                .map_err(|e| format!("Could not read slide '{}': {}", path, e))?;

            let layout_path = rel_target_of_type(&mut archive, path, "/slideLayout");
            let info = match &layout_path {
                Some(lp) => {
                    if !layouts.contains_key(lp) {
                        let info = LayoutInfo::load(&mut archive, lp, &default_style);
                        layouts.insert(lp.clone(), std::rc::Rc::new(info));
                    }
                    layouts[lp].clone()
                }
                None => std::rc::Rc::new(LayoutInfo { inherited: Inherited { default_style: default_style.clone(), ..Default::default() }, ..Default::default() }),
            };
            let map = info.map.overridden(&slide_xml);
            let mut ctx = Ctx { geo, theme: &info.theme, map, inh: &info.inherited, media: &media, media_dir, table_styles: &table_styles, bg_fill: None };
            let parts = [(path.as_str(), slide_xml.as_str()), (info.layout_path.as_str(), info.layout_xml.as_str()), (info.master_path.as_str(), info.master_xml.as_str())];
            let background = resolve_background(&mut archive, &parts, &ctx);
            ctx.bg_fill = match &background {
                Some(SlideBackground::Solid(c)) => Some(c.clone()),
                Some(SlideBackground::Gradient { stops, .. }) => stops.first().map(|s| s.color.clone()),
                _ => None,
            };

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
            // ActiveX controls (buttons, boxes) sit outside the shape tree; each keeps a picture of itself.
            if let Some((_, controls, _)) = next_tag(&slide_xml, "p:controls") {
                for (_, _, pic) in control_pics(controls) {
                    elements.extend(extract_picture_element(&mut archive, path, pic, &ctx, None));
                }
            }
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

            if let Some(bg) = background {
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
pub(crate) fn is_generic_title(t: &str) -> bool {
    let t = t.trim().to_lowercase();
    if matches!(t.as_str(), "powerpoint presentation" | "powerpoint-präsentation" | "presentation" | "présentation powerpoint" | "presentación de powerpoint") {
        return true;
    }
    let mut words = t.split_whitespace();
    matches!((words.next(), words.next(), words.next()), (Some("slide" | "folie" | "diapositiva" | "diapositive" | "dia"), Some(n), None) if n.chars().all(|c| c.is_ascii_digit()))
}

/// Where a deck's slides land on this app's 16:9 canvas, and how big its points are there.
#[derive(Clone, Copy)]
pub(crate) struct Geo {
    pub(crate) canvas_w: f64,
    pub(crate) canvas_h: f64,
    pub(crate) x_off: f64,
    pub(crate) y_off: f64,
    /// Multiplier from the deck's points to this app's: text is sized against a 1080-unit-tall
    /// canvas where 1pt is 4/3 of a unit (web/src/core/slide_render.ts), so 11.25 / the canvas
    /// height in inches -- 1.5 for a standard 7.5in-tall slide.
    pub(crate) font_scale: f64,
}

impl Geo {
    pub(crate) fn new(slide_w: f64, slide_h: f64) -> Geo {
        let (canvas_w, canvas_h) = if slide_w / slide_h < 16.0 / 9.0 { (slide_h * 16.0 / 9.0, slide_h) } else { (slide_w, slide_w * 9.0 / 16.0) };
        Geo { canvas_w, canvas_h, x_off: (canvas_w - slide_w) / 2.0, y_off: (canvas_h - slide_h) / 2.0, font_scale: 11.25 / (canvas_h / 914_400.0) }
    }

    /// An EMU rectangle on the slide as a canvas-relative `ElementTransform`. Not clamped to the
    /// canvas: a rotated shape's unrotated box can legitimately start off the slide (a long thin
    /// rule turned on end), and clamping it would move the shape.
    pub(crate) fn transform(&self, [x, y, cx, cy]: [f64; 4], rotation_deg: f64) -> crate::core::models::ElementTransform {
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
            layout_phs: inh::part_placeholders(&layout_xml, layout_path),
            master_phs: inh::part_placeholders(&master_xml, &master_path),
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
    /// `ppt/tableStyles.xml`, for tables' looks.
    table_styles: &'a str,
    /// The slide background's colour (a gradient's first stop), for shapes with `useBgFill`.
    bg_fill: Option<String>,
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
pub(crate) fn css_font_stack(name: &str) -> String {
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

/// A picture part's bytes, an SVG already drawn to PNG (see storage::svg_raster).
pub(crate) fn read_picture_bytes(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(crate::storage::svg_raster::as_raster(read_zip_bytes(archive, name)?))
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

/// The package path of the picture behind relationship `rid` of `part_path`.
fn embedded_path(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, rid: &str) -> Option<String> {
    let rels_xml = read_zip_text(archive, &slide_rels_path(part_path)).ok()?;
    let target = extract_rel_targets(&rels_xml).get(rid)?.clone();
    let dir = part_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("ppt/slides");
    Some(resolve_relative_path(dir, &target))
}

/// A picture the package embeds: already extracted (its served path), or its bytes, read once.
enum Embedded {
    Served(String),
    Bytes(String, Vec<u8>),
}

fn embedded(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, rid: &str, ctx: &Ctx) -> Option<Embedded> {
    let media_path = embedded_path(archive, part_path, rid)?;
    if let Some(done) = ctx.media.borrow().get(&media_path) {
        return Some(Embedded::Served(done.clone()));
    }
    let bytes = read_picture_bytes(archive, &media_path).ok()?;
    Some(Embedded::Bytes(media_path, bytes))
}

/// Writes picture bytes into the media folder once per `key` (a package path, or a content hash)
/// and returns the served path.
fn store_media(key: String, bytes: &[u8], ctx: &Ctx) -> Option<String> {
    if let Some(done) = ctx.media.borrow().get(&key) {
        return Some(done.clone());
    }
    // Extension comes from the bytes, not the archive entry's name -- see storage::media_sniff.
    let name = crate::storage::media_sniff::write_sniffed_media(ctx.media_dir, "pptx", bytes)?;
    let served = format!("/media/images/{}", name);
    ctx.media.borrow_mut().insert(key, served.clone());
    Some(served)
}

/// Extracts the picture behind relationship `rid` of `part_path` into the media folder (once per
/// package picture, however many slides show it) and returns its served path.
fn extract_embedded_image(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, rid: &str, ctx: &Ctx) -> Option<String> {
    match embedded(archive, part_path, rid, ctx)? {
        Embedded::Served(path) => Some(path),
        Embedded::Bytes(media_path, bytes) => store_media(media_path, &bytes, ctx),
    }
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
    let (inherited_rect, mut inherited_list, inherited_body) = ctx.inh.for_shape(ph.as_ref());
    let (own_list, own_body) = inh::shape_text_styles(shape_inner);
    let pal = ctx.palette();
    // An autoshape's <p:style><a:fontRef> gives its text a colour (white on a filled box, typically)
    // that outranks the inherited text styles' colours, though not the shape's own.
    let style_color = next_tag(shape_inner, "p:style").and_then(|(_, s, _)| next_tag(s, "a:fontRef")).and_then(|(_, i, _)| pal.first_color(i, None));
    if style_color.is_some() {
        inherited_list.0.iter_mut().for_each(|l| l.run.color = None);
    }
    let list = own_list.over(&inherited_list);
    let body = own_body.over(&inherited_body);
    // SmartArt shapes carry their text box separately (`txXfrm`), unrotated even when the shape is turned.
    let tx_rect = next_tag(shape_inner, "p:txXfrm").and_then(|(_, x, _)| {
        let n = |tag: &str, k: &str| next_tag(x, tag).and_then(|(a, _, _)| extract_attr(a, k)?.parse::<f64>().ok());
        let (rx, ry, rw, rh) = (n("a:off", "x")?, n("a:off", "y")?, n("a:ext", "cx")?, n("a:ext", "cy")?);
        let (rx, ry, rw, rh) = match remap { Some(r) => r.apply(rx, ry, rw, rh), None => (rx, ry, rw, rh) };
        Some(([rx, ry, rw, rh], 0.0))
    });
    let has_tx_rect = tx_rect.is_some();
    let (mut rect, rot) = tx_rect.or_else(|| own_rect(shape_inner, remap)).or_else(|| inherited_rect.map(|r| (r, 0.0)))
        .unwrap_or(([ctx.geo.canvas_w * 0.05 - ctx.geo.x_off, ctx.geo.canvas_h * 0.07 - ctx.geo.y_off, ctx.geo.canvas_w * 0.9, ctx.geo.canvas_h * 0.86], 0.0));
    // Text sits in the geometry's own text rectangle: an ellipse's inscribed box, an arrow's shaft.
    if !has_tx_rect {
        rect = geometry_text_rect(shape_inner, ctx, rect, rot);
    }
    text_block_element(txbody, &list, &body, style_color, rect, rot, ctx)
}

/// A geometry evaluated for a `w` x `h` box, remembered for the next call about the same one: a
/// shape's text box and its drawing both need it, and evaluating every guide twice is waste.
/// Keyed by the geometry's own markup (`source`: the preset and its adjust values, or the custom
/// geometry -- the shape's own or the one its placeholder inherits), not by where it sits.
fn evaluated(source: &GeomSource, w: f64, h: f64, flips: (bool, bool), eval: impl FnOnce() -> Option<pptx_geom::Geometry>) -> Option<pptx_geom::Geometry> {
    type Key = (u64, u64, u64, (bool, bool));
    thread_local!(static LAST: RefCell<Option<(Key, Option<pptx_geom::Geometry>)>> = const { RefCell::new(None) });
    let digest = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        match source {
            GeomSource::Custom(c) => ("custom", *c, "").hash(&mut h),
            GeomSource::Preset(p, av) => ("preset", *p, *av).hash(&mut h),
        }
        h.finish()
    };
    let key: Key = (digest, w.to_bits(), h.to_bits(), flips);
    if let Some(hit) = LAST.with(|l| l.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, g)| g.clone())) {
        return hit;
    }
    let g = eval();
    LAST.with(|l| *l.borrow_mut() = Some((key, g.clone())));
    g
}

/// `rect` (a shape's box, turned `rot` degrees about its centre) narrowed to its geometry's text
/// rectangle, still turning about the same point. A rectangle's text box is the whole shape.
fn geometry_text_rect(shape_inner: &str, ctx: &Ctx, rect: [f64; 4], rot: f64) -> [f64; 4] {
    let [x, y, w, h] = rect;
    let source = shape_geometry(shape_inner, ctx);
    let geometry = match source {
        GeomSource::Preset("rect", _) => return rect,
        GeomSource::Preset(prst, av) => evaluated(&source, w, h, (false, false), || pptx_geom::preset(prst, av, w, h, (false, false))),
        GeomSource::Custom(cust) => evaluated(&source, w, h, (false, false), || Some(pptx_geom::custom(cust, w, h, (false, false)))),
    };
    let Some([l, t, r, b]) = geometry.and_then(|g| g.text_rect) else { return rect };
    if r - l <= 0.0 || b - t <= 0.0 {
        return rect;
    }
    let (flip_h, flip_v) = xfrm_flips(shape_inner);
    let (l, r) = if flip_h { (w - r, w - l) } else { (l, r) };
    let (t, b) = if flip_v { (h - b, h - t) } else { (t, b) };
    // The text box's centre, moved from the shape's centre and turned with the shape.
    let (dx, dy) = ((l + r) / 2.0 - w / 2.0, (t + b) / 2.0 - h / 2.0);
    let (sin, cos) = rot.to_radians().sin_cos();
    let (cx, cy) = (x + w / 2.0 + dx * cos - dy * sin, y + h / 2.0 + dx * sin + dy * cos);
    let (tw, th) = (r - l, b - t);
    [cx - tw / 2.0, cy - th / 2.0, tw, th]
}

/// A text body (`<p:txBody>`, a table cell's `<a:txBody>`) as a `TextBlock` in the EMU box `rect`,
/// its paragraph levels resolved over `list`. `None` when it has no text.
fn text_block_element(txbody: &str, list: &inh::ListStyle, body: &inh::BodyProps, style_color: Option<String>, rect: [f64; 4], rot: f64, ctx: &Ctx) -> Option<SlideElement> {
    let pal = ctx.palette();
    let size_scale = body.font_scale.unwrap_or(1.0);

    let mut runs: Vec<crate::core::models::TextRun> = Vec::new();
    let mut first_level: Option<inh::Level> = None;
    let mut first_font_pt = 0.0;
    // Whether any paragraph starts with a lead run (indent, bullet): then each one gets one, empty
    // where it has none, so the renderer can hang wrapped lines after it.
    let mut any_lead = false;
    let mut no_lead: Vec<usize> = Vec::new();
    // This app has one alignment per text box: the one most of the text uses.
    let mut align_weight: HashMap<String, usize> = HashMap::new();
    let mut counters = [0usize; 9];
    let mut any_text = false;
    // (space before, space after) per paragraph with text, in canvas points, weighted by its length.
    let mut gaps: Vec<(f64, f64, usize)> = Vec::new();
    for (pi, (_, _, p_inner)) in children(txbody).into_iter().filter(|(n, _, _)| *n == "a:p").enumerate() {
        let (ppr_attrs, ppr_inner) = child(p_inner, "a:pPr").map(|(_, a, i)| (a, i)).unwrap_or(("", ""));
        let lvl = extract_attr(ppr_attrs, "lvl").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0).min(8);
        let level = inh::Level::parse(ppr_attrs, ppr_inner).over(&list.0[lvl]);

        let mut para: Vec<crate::core::models::TextRun> = Vec::new();
        // A line break (Shift+Enter) joins its neighbour's text, so a run that is exactly "\n"
        // only ever separates paragraphs -- which is where the renderer puts paragraph spacing.
        // Breaks before any text wait for it; a paragraph of breaks alone ends with a zero-width
        // space, so its run is never exactly "\n".
        let mut pending_breaks = 0usize;
        for (name, _, inner) in children(p_inner) {
            let rpr = child(inner, "a:rPr").map(|(_, a, i)| RunProps::parse(a, i)).unwrap_or_default().over(&level.run);
            match name {
                "a:r" | "a:fld" => {
                    let text: String = children(inner).into_iter().filter(|(n, _, _)| *n == "a:t").map(|(_, _, t)| decode_xml_entities(t)).collect();
                    if !text.is_empty() {
                        let text = format!("{}{text}", "\n".repeat(std::mem::take(&mut pending_breaks)));
                        para.push(text_run(text, &rpr, ctx, style_color.as_deref(), size_scale));
                    }
                }
                // A line break inside a paragraph (Shift+Enter), not a new paragraph.
                "a:br" => match para.last_mut() {
                    Some(prev) => prev.text.push('\n'),
                    None => pending_breaks = (pending_breaks + 1).min(1000),
                },
                _ => {}
            }
        }
        if pending_breaks > 0 {
            para.push(text_run(format!("{}\u{200b}", "\n".repeat(pending_breaks)), &level.run, ctx, style_color.as_deref(), size_scale));
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
                first_font_pt = para.iter().map(|r| r.font_size_pt).fold(0.0, f64::max);
            }
            let weight = para.iter().map(|r| r.text.chars().count()).sum::<usize>();
            *align_weight.entry(level.align.clone().unwrap_or_else(|| "l".into())).or_default() += weight;
            // Spacing shrinks with the body's autofit (`lnSpcReduction`), like the line spacing.
            let font_pt = para.iter().map(|r| r.font_size_pt).fold(0.0, f64::max);
            let squeeze = (1.0 - body.line_reduction.unwrap_or(0.0)).max(0.0);
            let gap = |s: Option<inh::Spacing>| s.map_or(0.0, |s| match s {
                inh::Spacing::Points(p) => p * ctx.geo.font_scale,
                inh::Spacing::OfFont(_) => s.points(font_pt),
            } * squeeze);
            gaps.push((gap(level.space_before), gap(level.space_after), weight));
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
            // Bounded: a malformed size or margin can't ask for millions of spaces.
            let spaces = if en_emu > 0.0 && indent_emu.is_finite() { (indent_emu / en_emu).round().clamp(0.0, 40.0) as usize } else { 2 * lvl };
            let indent = "\u{2002}".repeat(spaces);
            if marker.is_some() || !indent.is_empty() {
                any_lead = true;
                let mut lead = para[0].clone();
                lead.text = format!("{indent}{}", marker.map(|m| format!("{m} ")).unwrap_or_default());
                lead.bold = false;
                lead.italic = false;
                lead.underline = false;
                if let Some(c) = level.bullet_color.as_deref().and_then(|c| pal.first_color(c, None)) {
                    lead.color = c;
                }
                runs.push(lead);
            } else {
                no_lead.push(runs.len());
            }
        }
        runs.extend(para);
    }
    if !any_text {
        return None;
    }
    if any_lead {
        for at in no_lead.into_iter().rev() {
            let mut empty = runs[at].clone();
            empty.text = String::new();
            runs.insert(at, empty);
        }
    }

    let level = first_level.unwrap_or_default();
    let majority = align_weight.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0))).map(|(a, _)| a);
    let align = match majority.as_deref() {
        Some("ctr") => "center",
        Some("r") => "right",
        Some("just") | Some("justLow") | Some("dist") => "justify",
        _ => "left",
    };
    // One gap per text box (this app's paragraph style is per box): the length-weighted mean of
    // the gaps that actually fall between paragraphs -- not before the first or after the last.
    let (space_before_pt, space_after_pt) = paragraph_gaps(&gaps);
    // PowerPoint's single spacing is about 1.2 lines of the font size.
    // Exact spacing (points) is that many points per line of the first line's text.
    let line_height = match level.line_spacing {
        Some(inh::Spacing::Points(p)) if first_font_pt > 0.0 => (p * ctx.geo.font_scale / first_font_pt).clamp(0.5, 5.0),
        Some(inh::Spacing::OfFont(k)) => (k - body.line_reduction.unwrap_or(0.0)).max(0.5) * 1.2,
        _ => (1.0 - body.line_reduction.unwrap_or(0.0)).max(0.5) * 1.2,
    };
    let line_height = (line_height * 100.0).round() / 100.0;
    let vertical_align = match body.anchor.as_deref() {
        Some("ctr") => "middle",
        Some("b") => "bottom",
        _ => "top",
    };

    // The text sits inside the box's insets; this app's text box has none, so the box shrinks.
    let ins: Vec<f64> = body.insets.iter().zip(DEFAULT_INSETS).map(|(v, d)| v.unwrap_or(d)).collect();
    let [x, y, cx, cy] = rect;
    let inner = [x + ins[0], y + ins[1], (cx - ins[0] - ins[2]).max(cx * 0.2), (cy - ins[1] - ins[3]).max(cy * 0.2)];

    Some(SlideElement::TextBlock {
        id: new_id(),
        transform: ctx.geo.transform(inner, rot),
        block: crate::core::models::TextBlock {
            runs,
            paragraph_style: crate::core::models::TextParagraphStyle {
                align: align.to_string(),
                line_height,
                space_before_pt,
                space_after_pt,
                vertical_align: Some(vertical_align.to_string()),
                hanging_indent: any_lead,
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
    let mut el = picture_element(archive, part_path, &rid, rect, rot, pic_inner, ctx)?;
    // "Crop to shape": the picture shows through its geometry (a circle, a rounded box...).
    if let SlideElement::Image { mask_shape, .. } = &mut el {
        *mask_shape = geometry_clip(pic_inner, ctx, rect);
    }
    Some(el)
}

/// A shape's or picture's outline as a clip polygon for a picture in its box; `None` for a rectangle.
fn geometry_clip(shape_inner: &str, ctx: &Ctx, rect: [f64; 4]) -> Option<String> {
    let flips = xfrm_flips(shape_inner);
    match shape_geometry(shape_inner, ctx) {
        GeomSource::Preset("rect", _) => None,
        GeomSource::Preset(prst, av) => pptx_geom::preset(prst, av, rect[2], rect[3], flips).as_ref().and_then(pptx_geom::clip_polygon),
        GeomSource::Custom(cust) => pptx_geom::clip_polygon(&pptx_geom::custom(cust, rect[2], rect[3], flips)),
    }
}

/// The picture relationship `rid` of `part_path` drawn in `rect`, cropped as `pic_inner`'s
/// `<a:srcRect>` says, with its "set transparent colour" (`<a:clrChange>`) applied.
fn picture_element(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, rid: &str, rect: [f64; 4], rot: f64, pic_inner: &str, ctx: &Ctx) -> Option<SlideElement> {
    // <a:srcRect> crops in thousandths of a percent from each edge.
    let src = next_tag(pic_inner, "a:srcRect").map(|(a, _, _)| {
        let f = |k: &str| extract_attr(a, k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100_000.0;
        [f("l"), f("t"), f("r"), f("b")]
    }).filter(|c| c.iter().any(|v| *v != 0.0));
    let clear = transparent_color(pic_inner, ctx);
    let recolor = next_tag(pic_inner, "a:blip").and_then(|(_, b, _)| Recolor::of(b, ctx, None));
    let media_path = embedded_path(archive, part_path, rid)?;
    let key = match (&clear, &recolor) {
        (None, None) => media_path.clone(),
        (c, r) => format!("{media_path}#clear{}#{}", c.as_deref().unwrap_or(""), r.as_ref().map(Recolor::key).unwrap_or_default()),
    };
    let cached = ctx.media.borrow().get(&key).cloned();
    let file_path = match cached {
        Some(done) => done,
        None => {
            let bytes = read_picture_bytes(archive, &media_path).ok()?;
            // A Windows or Enhanced Metafile (clip art, drawn figures, pasted charts and tables)
            // can't be shown as a picture: its drawing becomes elements instead.
            if crate::storage::wmf::is_wmf(&bytes) || crate::storage::emf::is_emf(&bytes) {
                return metafile_group(&bytes, rect, rot, ctx, src, clear.as_deref(), recolor.as_ref());
            }
            // A see-through colour and a recolour are both applied to one decoded picture.
            let drawn = (clear.is_some() || recolor.is_some()).then(|| {
                let mut img = image::load_from_memory(&bytes).ok()?.to_rgba8();
                if let Some(c) = &clear {
                    clear_color(&mut img, c);
                }
                if let Some(r) = &recolor {
                    r.apply(&mut img);
                }
                encode_picture(img, is_jpeg(&bytes))
            }).flatten();
            store_media(key, drawn.as_deref().unwrap_or(&bytes), ctx)?
        }
    };
    let crop = src.map(|[l, t, r, b]| crate::core::models::CropRect { x: l, y: t, w: (1.0 - l - r).clamp(0.01, 1.0), h: (1.0 - t - b).clamp(0.01, 1.0) });
    Some(SlideElement::Image { id: new_id(), transform: ctx.geo.transform(rect, rot), file_path, crop, mask_shape: None, alt_text: None, fit: Some("stretch".into()) })
}

/// A picture's "set transparent colour" (`<a:clrChange>` to a see-through colour): the colour
/// that disappears, as `#rrggbb`.
fn transparent_color(pic_inner: &str, ctx: &Ctx) -> Option<String> {
    let (_, blip, _) = next_tag(pic_inner, "a:blip")?;
    let (_, change, _) = next_tag(blip, "a:clrChange")?;
    let pal = ctx.palette();
    let (_, _, to) = child(change, "a:clrTo")?;
    let (_, alpha) = children(to).into_iter().find_map(|(n, a, i)| pal.color_alpha(n, a, i, None))?;
    if alpha >= 0.5 {
        return None;
    }
    child(change, "a:clrFrom").and_then(|(_, _, i)| pal.first_color(i, None))
}

/// Makes every pixel of (about) `color` (`#rrggbb`) see-through.
fn clear_color(img: &mut image::RgbaImage, color: &str) {
    let hex = color.trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok().map(i32::from);
    let (Some(r), Some(g), Some(b)) = (c(0), c(2), c(4)) else { return };
    for p in img.pixels_mut() {
        if (i32::from(p[0]) - r).abs() <= 2 && (i32::from(p[1]) - g).abs() <= 2 && (i32::from(p[2]) - b).abs() <= 2 {
            p[3] = 0;
        }
    }
}

/// A picture's average colour as `#rrggbb`, from every pixel on a grid about `steps` x `steps`
/// across it (1: every pixel).
pub(crate) fn average_hex(img: &image::RgbImage, steps: u32) -> String {
    let (w, h) = img.dimensions();
    let (sx, sy) = if steps <= 1 { (1, 1) } else { ((w / steps).max(1) as usize, (h / steps).max(1) as usize) };
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for y in (0..h).step_by(sy) {
        for x in (0..w).step_by(sx) {
            let p = img.get_pixel(x, y);
            (0..3).for_each(|i| sum[i] += p[i] as u64);
            n += 1;
        }
    }
    let n = n.max(1);
    format!("#{:02x}{:02x}{:02x}", sum[0] / n, sum[1] / n, sum[2] / n)
}

/// Whether picture bytes are a JPEG (a photograph, as a rule).
pub(crate) fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

/// A shape whose own fill is a picture (`<p:spPr><a:blipFill>`): the picture stretched over the
/// shape's box and clipped to its outline (a polygon following the geometry; a shape that can't be
/// one polygon, such as a donut, shows the whole box). Its outline and text are drawn as usual.
fn picture_filled_shape(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, shape_inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>) -> Option<SlideElement> {
    // The fill comes from the first of the shape, its layout's placeholder and its master's that
    // states one, or a theme fill style one names (as extract_shape_style reads it); a picture fill
    // is resolved against the part it's written in.
    let own = (next_tag(shape_inner, "p:spPr").map_or("", |(_, c, _)| c), next_tag(shape_inner, "p:style").map_or("", |(_, c, _)| c), part_path);
    let mut chain = vec![own];
    chain.extend(ctx.inh.shape_parts_in(Ph::of_shape(shape_inner).as_ref()));
    let (fill, part_path) = chain.into_iter().find_map(|(sppr, style, part)| {
        if let Some((name, _, inner)) = children(sppr).into_iter().find(|(n, _, _)| matches!(*n, "a:noFill" | "a:solidFill" | "a:gradFill" | "a:pattFill" | "a:blipFill")) {
            return Some((name == "a:blipFill").then_some((inner, part)));
        }
        let (a, _, _) = next_tag(style, "a:fillRef")?;
        let idx = extract_attr(a, "idx").and_then(|v| v.parse::<usize>().ok()).filter(|i| *i > 0)?;
        let theme_fill = if idx >= 1001 { ctx.theme.bg_fills.get(idx - 1001) } else { ctx.theme.fills.get(idx - 1) }?;
        let (name, _, inner) = children(theme_fill).into_iter().next()?;
        Some((name == "a:blipFill").then_some((inner, ctx.theme.path.as_str())))
    }).flatten()?;
    let rid = next_tag(fill, "a:blip").and_then(|(a, _, _)| extract_attr(a, "r:embed"))?.to_string();
    let (rect, rot) = shape_rect(shape_inner, ctx, remap)?;
    // A texture is tiled over the shape's box at the canvas's resolution (recoloured as its blip
    // says); one this can't decode, or an untiled picture, is drawn stretched.
    let tiled = next_tag(fill, "a:tile").map(|(a, _, _)| a).and_then(|tile| {
        let recolor = next_tag(fill, "a:blip").and_then(|(_, b, _)| Recolor::of(b, ctx, None));
        drawn_picture(archive, part_path, &rid, Some(tile), recolor.as_ref(), [rect[2], rect[3]], [0.0, 0.0, rect[2], rect[3]], ctx)
    });
    let mut el = match tiled {
        Some(file_path) => SlideElement::Image { id: new_id(), transform: ctx.geo.transform(rect, rot), file_path, crop: None, mask_shape: None, alt_text: None, fit: Some("stretch".into()) },
        None => picture_element(archive, part_path, &rid, rect, rot, fill, ctx)?,
    };
    if let SlideElement::Image { mask_shape, .. } = &mut el {
        *mask_shape = geometry_clip(shape_inner, ctx, rect);
    }
    Some(el)
}

/// The pictures of a slide's `<p:controls>`: each control's own, taken from its
/// `mc:AlternateContent`'s Fallback (else its Choice) only -- a writer may give both a copy.
fn control_pics(controls: &str) -> Vec<(&str, &str, &str)> {
    children(controls).into_iter().flat_map(|(name, _, inner)| {
        let part = if name == "mc:AlternateContent" {
            child(inner, "mc:Fallback").or_else(|| child(inner, "mc:Choice")).map_or("", |(_, _, i)| i)
        } else {
            inner
        };
        find_pics(part)
    }).collect()
}

/// Every `<p:pic>` in some markup, outermost first, in document order (bounded by the markup).
fn find_pics(xml: &str) -> Vec<(&str, &str, &str)> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some((attrs, inner, after)) = next_tag(rest, "p:pic") {
        out.push(("p:pic", attrs, inner));
        rest = after;
    }
    out
}

/// An embedded object (`<p:oleObj>`: a chart from another program, an old slide, an equation) as
/// the preview picture its file keeps for it: a `<p:pic>` inside the object (PowerPoint 2010 and
/// later), else the picture its legacy VML drawing gives the shape `spid` names.
fn ole_preview(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>) -> Option<SlideElement> {
    let rect = frame_rect(inner, remap)?;
    // The picture can sit in an <mc:Fallback> beside a VML-only <mc:Choice>: look through the frame.
    if let Some((_, pic, _)) = next_tag(inner, "p:pic") {
        let rid = next_tag(pic, "a:blip").and_then(|(a, _, _)| extract_attr(a, "r:embed"))?.to_string();
        return picture_element(archive, part_path, &rid, rect, 0.0, pic, ctx);
    }
    let (ole_attrs, _, _) = next_tag(inner, "p:oleObj")?;
    let spid = extract_attr(ole_attrs, "spid")?;
    let vml_path = rel_target_of_type(archive, part_path, "/vmlDrawing")?;
    let vml = String::from_utf8_lossy(&read_zip_bytes(archive, &vml_path).ok()?).into_owned();
    // VML is loose markup: find the shape by its id, then its picture before the shape ends.
    let at = vml.find(&format!(r#"id="{spid}""#))?;
    let shape = &vml[at..at + vml[at..].find("</v:shape>")?];
    let rid = next_tag(shape, "v:imagedata").and_then(|(a, _, _)| extract_attr(a, "o:relid").or_else(|| extract_attr(a, "r:id")))?.to_string();
    picture_element(archive, &vml_path, &rid, rect, 0.0, "", ctx)
}

/// `Shape`/`Line`'s `stroke_width` is a percent of the shape's own box (its shorter side) -- both
/// `shape_library.ts` and `line_tool.ts` draw into a fixed `0 0 100 100` viewBox.
pub(crate) fn emu_to_relative_stroke(width_emu: f64, box_w_emu: f64, box_h_emu: f64) -> f64 {
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

/// A shape fill element as this app's `fill_color`: a colour (`#rrggbbaa` when see-through), or a
/// gradient as CSS (`linear-gradient(...)`, which the renderer draws as an SVG gradient). A
/// pattern is drawn as its foreground colour; a picture fill isn't a colour (`None`).
fn fill_paint(name: &str, inner: &str, ctx: &Ctx, ph: Option<&str>) -> Option<String> {
    match name {
        "a:solidFill" => ctx.palette().first_color_with_alpha(inner, ph),
        "a:gradFill" => match gradient_of(inner, ctx, ph)? {
            SlideBackground::Gradient { kind, stops, angle_deg } => Some(gradient_css(&kind, &stops, angle_deg)),
            SlideBackground::Solid(c) => Some(c),
            _ => None,
        },
        "a:pattFill" => child(inner, "a:fgClr").and_then(|(_, _, i)| ctx.palette().first_color_with_alpha(i, ph)),
        _ => None,
    }
}

/// A theme style reference (`<a:fillRef idx="3"><a:schemeClr val="accent1"/></a:fillRef>`): the
/// theme's fill style `idx` (1001 and up: its background fills) painted in the colour it passes
/// in. Index 0 is no fill.
fn theme_fill(ref_attrs: &str, ref_inner: &str, ctx: &Ctx) -> Option<String> {
    let idx = extract_attr(ref_attrs, "idx").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    if idx == 0 {
        return None;
    }
    let ph = ctx.palette().first_color_with_alpha(ref_inner, None);
    let fill = if idx >= 1001 { ctx.theme.bg_fills.get(idx - 1001) } else { ctx.theme.fills.get(idx - 1) };
    fill.and_then(|f| children(f).into_iter().next())
        .and_then(|(n, _, i)| fill_paint(n, i, ctx, ph.as_deref()))
        .or(ph)
}

/// Reads a shape's fill and stroke. Each comes from the first of these that states it: the
/// shape's `<p:spPr>`, its `<p:style>`'s `fillRef`/`lnRef` (theme styles; index 0 meaning none),
/// then -- for a placeholder -- the same pair on its layout's placeholder, then its master's.
/// `<a:noFill/>` is "nothing here"; `useBgFill` paints the shape with the slide's background.
fn extract_shape_style(shape_inner: &str, ctx: &Ctx, use_bg_fill: bool) -> ShapeStyle {
    let own = (next_tag(shape_inner, "p:spPr").map(|(_, c, _)| c).unwrap_or(""), next_tag(shape_inner, "p:style").map(|(_, c, _)| c).unwrap_or(""));
    let mut chain = vec![own];
    chain.extend(ctx.inh.shape_parts(Ph::of_shape(shape_inner).as_ref()));
    let style_ref = |style: &str, tag: &str| -> Option<Option<String>> {
        let (a, i, _) = next_tag(style, tag)?;
        Some(extract_attr(a, "idx").filter(|i| *i != "0").and_then(|_| ctx.palette().first_color(i, None)))
    };

    let fill = if use_bg_fill {
        ctx.bg_fill.clone().unwrap_or_else(|| "none".to_string())
    } else {
        chain.iter().find_map(|(sppr, style)| {
            let own = children(sppr).into_iter().find(|(n, _, _)| matches!(*n, "a:noFill" | "a:solidFill" | "a:gradFill" | "a:pattFill" | "a:blipFill"));
            let style_fill = || next_tag(style, "a:fillRef").map(|(a, i, _)| theme_fill(a, i, ctx));
            match own {
                Some(("a:noFill", _, _)) => Some(None),
                // A gradient without stops (some writers emit one) leaves the colour to the style.
                Some(("a:gradFill", _, i)) if next_tag(i, "a:gs").is_none() => style_fill(),
                Some((n, _, i)) => Some(fill_paint(n, i, ctx, None)),
                None => style_fill(),
            }
        }).flatten().unwrap_or_else(|| "none".to_string())
    };

    // The outline: the nearest `<a:ln>` gives width and arrowheads; its colour is the nearest
    // stated one, an `<a:ln>`'s fill or a style's `lnRef`.
    let mut stroke_width_emu = 0.0;
    let mut start_arrow = false;
    let mut end_arrow = false;
    let mut stroke_color = None;
    let mut have_ln = false;
    for (sppr, style) in &chain {
        let ln = children(sppr).into_iter().find(|(n, _, _)| *n == "a:ln");
        if let Some((_, ln_attrs, ln_inner)) = ln {
            if !have_ln {
                have_ln = true;
                stroke_width_emu = extract_attr(ln_attrs, "w").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                // headEnd = the line's first point, tailEnd = its last; type="none" is no marker.
                start_arrow = next_tag(ln_inner, "a:headEnd").map(|(a, _, _)| extract_attr(a, "type") != Some("none")).unwrap_or(false);
                end_arrow = next_tag(ln_inner, "a:tailEnd").map(|(a, _, _)| extract_attr(a, "type") != Some("none")).unwrap_or(false);
            }
            let ln_fill = children(ln_inner).into_iter().find(|(n, _, _)| matches!(*n, "a:noFill" | "a:solidFill" | "a:gradFill" | "a:pattFill"));
            match ln_fill {
                Some(("a:noFill", _, _)) => break,
                Some((n, _, i)) => { stroke_color = fill_color(n, i, ctx, None); break }
                None => {}
            }
        }
        if let Some(c) = style_ref(style, "a:lnRef") {
            stroke_color = c;
            break;
        }
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
    geo_stroke(width_emu, &ctx.geo)
}

/// A line `width_emu` wide as the canvas draws it (at least half a point).
pub(crate) fn geo_stroke(width_emu: f64, geo: &Geo) -> f64 {
    ((width_emu.max(6_350.0) / geo.canvas_h * 1080.0) * 100.0).round() / 100.0
}

/// A table cell's border in `rect` (EMU), each side (top, right, bottom, left) its own colour and
/// width or none: one outlined box when all four agree, else a line per side.
pub(crate) fn cell_edges(rect: [f64; 4], sides: &[Option<(String, f64)>; 4], geo: &Geo) -> Vec<SlideElement> {
    let [x, y, w, h] = rect;
    if sides.iter().all(|s| s.is_some() && *s == sides[0]) {
        let (color, width) = sides[0].clone().expect("checked");
        return vec![SlideElement::Shape { id: new_id(), transform: geo.transform(rect, 0.0), shape_kind: "path".into(), fill_color: "none".into(), stroke_color: Some(color), stroke_width: geo_stroke(width, geo), path: Some("M 0 0 L 100 0 L 100 100 L 0 100 Z".into()) }];
    }
    let ends = [((x, y), (x + w, y)), ((x + w, y), (x + w, y + h)), ((x, y + h), (x + w, y + h)), ((x, y), (x, y + h))];
    sides.iter().zip(ends).filter_map(|(side, ((x1, y1), (x2, y2)))| {
        let (color, width) = side.clone()?;
        let horizontal = y1 == y2;
        Some(SlideElement::Line {
            id: new_id(),
            transform: geo.transform([x1, y1, x2 - x1, y2 - y1], 0.0),
            line_kind: "straight".into(),
            color,
            stroke_width: geo_stroke(width, geo),
            start_arrow: false,
            end_arrow: false,
            path: Some(if horizontal { "M 0 0 L 100 0" } else { "M 0 0 L 0 100" }.into()),
        })
    }).collect()
}

fn xfrm_flips(shape_inner: &str) -> (bool, bool) {
    let attrs = next_tag(shape_inner, "p:spPr").and_then(|(_, sppr, _)| next_tag(sppr, "a:xfrm")).map(|(a, _, _)| a).unwrap_or("");
    (extract_attr(attrs, "flipH") == Some("1"), extract_attr(attrs, "flipV") == Some("1"))
}

/// Builds a `Shape` from a `<p:sp>`. Its geometry -- a preset or a freeform (`<a:custGeom>`) --
/// is evaluated for the shape's size into exact path data (`pptx_geom`); only a plain rectangle or
/// ellipse stays a library shape. A geometry whose parts are painted differently (a can's lighter
/// lid, an outline-only stroke over a fill) comes back as a `Group` of one element per look, and an
/// unfilled one as a `Line` (an arrow, often). `None` when it would draw nothing at all.
fn extract_autoshape_element(shape_inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>, use_bg_fill: bool) -> Option<SlideElement> {
    let style = extract_shape_style(shape_inner, ctx, use_bg_fill);
    if style.fill_color == "none" && style.stroke_color.is_none() {
        return None;
    }
    let (rect, rot) = shape_rect(shape_inner, ctx, remap)?;
    let transform = ctx.geo.transform(rect, rot);
    let flips = xfrm_flips(shape_inner);
    let source = shape_geometry(shape_inner, ctx);
    let geometry = match source {
        GeomSource::Custom(cust) => evaluated(&source, rect[2], rect[3], flips, || Some(pptx_geom::custom(cust, rect[2], rect[3], flips)))?,
        GeomSource::Preset(prst, av) => match prst {
            "rect" | "flowChartProcess" | "ellipse" | "flowChartConnector" => {
                let stroke_width = if style.stroke_color.is_some() { emu_to_relative_stroke(style.stroke_width_emu, rect[2], rect[3]) } else { 0.0 };
                let shape_kind = if matches!(prst, "rect" | "flowChartProcess") { "rect" } else { "ellipse" };
                return Some(SlideElement::Shape { id: new_id(), transform, shape_kind: shape_kind.into(), fill_color: style.fill_color, stroke_color: style.stroke_color, stroke_width, path: None });
            }
            _ => match evaluated(&source, rect[2], rect[3], flips, || pptx_geom::preset(prst, av, rect[2], rect[3], flips)) {
                Some(g) => g,
                None => {
                    let stroke_width = if style.stroke_color.is_some() { emu_to_relative_stroke(style.stroke_width_emu, rect[2], rect[3]) } else { 0.0 };
                    return Some(SlideElement::Shape { id: new_id(), transform, shape_kind: map_prst_geom_type(prst), fill_color: style.fill_color, stroke_color: style.stroke_color, stroke_width, path: None });
                }
            },
        },
    };
    geometry_element(&geometry, &style, transform, ctx)
}

/// One block's paragraph spacing from each paragraph's (space before, space after, text length):
/// the length-weighted mean of the gaps between paragraphs (space before the first and after the
/// last don't count), to a tenth of a point.
pub(crate) fn paragraph_gaps(gaps: &[(f64, f64, usize)]) -> (f64, f64) {
    let mean = |v: Vec<(f64, usize)>| {
        let total: usize = v.iter().map(|(_, w)| *w).sum();
        if total == 0 { 0.0 } else { (v.iter().map(|(g, w)| g * *w as f64).sum::<f64>() / total as f64 * 10.0).round() / 10.0 }
    };
    let before = mean(gaps.iter().skip(1).map(|(b, _, w)| (*b, *w)).collect());
    let after = mean(gaps.iter().take(gaps.len().saturating_sub(1)).map(|(_, a, w)| (*a, *w)).collect());
    (before, after)
}

/// A fresh element id (every importer's elements use this one form).
pub(crate) fn new_id() -> String {
    format!("el_{}", uuid::Uuid::new_v4())
}

enum GeomSource<'a> {
    Custom(&'a str),
    /// A preset's name and the shape's adjust values (`<a:avLst>` contents).
    Preset(&'a str, &'a str),
}

/// A shape's geometry: its own, else (a placeholder) its layout's or master's, else a rectangle.
fn shape_geometry<'a>(shape_inner: &'a str, ctx: &Ctx<'a>) -> GeomSource<'a> {
    let own_sppr = next_tag(shape_inner, "p:spPr").map(|(_, c, _)| c).unwrap_or("");
    let inherited = ctx.inh.shape_parts(Ph::of_shape(shape_inner).as_ref());
    for sppr in std::iter::once(own_sppr).chain(inherited.iter().map(|(sp, _)| *sp)) {
        if let Some((_, cust, _)) = next_tag(sppr, "a:custGeom") {
            return GeomSource::Custom(cust);
        }
        if let Some((attrs, inner, _)) = next_tag(sppr, "a:prstGeom") {
            let av = next_tag(inner, "a:avLst").map(|(_, i, _)| i).unwrap_or("");
            return GeomSource::Preset(extract_attr(attrs, "prst").unwrap_or("rect"), av);
        }
    }
    GeomSource::Preset("rect", "")
}

/// The elements an evaluated geometry draws, painted with `style`: consecutive paths that look
/// alike share one element; a filled part is a `Shape`, an outline-only part a `Line`.
fn geometry_element(geometry: &pptx_geom::Geometry, style: &ShapeStyle, transform: crate::core::models::ElementTransform, ctx: &Ctx) -> Option<SlideElement> {
    let mut parts: Vec<(Option<String>, Option<String>, String)> = Vec::new();
    for p in &geometry.paths {
        let fill = (p.fill != pptx_geom::FillMode::None && style.fill_color != "none").then(|| pptx_geom::shade(&style.fill_color, p.fill));
        let stroke = if p.stroke { style.stroke_color.clone() } else { None };
        if fill.is_none() && stroke.is_none() {
            continue;
        }
        match parts.last_mut() {
            Some((f, s, d)) if *f == fill && *s == stroke => {
                d.push(' ');
                d.push_str(&p.d);
            }
            _ => parts.push((fill, stroke, p.d.clone())),
        }
    }
    let stroke_width = canvas_stroke(style.stroke_width_emu, ctx);
    let mut elements: Vec<SlideElement> = parts.into_iter().map(|(fill, stroke, d)| match fill {
        Some(fill_color) => SlideElement::Shape { id: new_id(), transform: transform.clone(), shape_kind: "path".into(), fill_color, stroke_width: if stroke.is_some() { stroke_width } else { 0.0 }, stroke_color: stroke, path: Some(d) },
        None => SlideElement::Line { id: new_id(), transform: transform.clone(), line_kind: "curved".into(), color: stroke.unwrap_or_default(), stroke_width, start_arrow: style.start_arrow, end_arrow: style.end_arrow, path: Some(d) },
    }).collect();
    match elements.len() {
        0 => None,
        1 => elements.pop(),
        _ => Some(SlideElement::Group { id: new_id(), transform: crate::core::models::ElementTransform { rotation_deg: 0.0, ..transform }, children: elements }),
    }
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
    let style = extract_shape_style(inner, ctx, false);
    // No line colour at all (`<a:ln><a:noFill/>`, or nothing styled): an invisible guide, not a line.
    let color = style.stroke_color.clone()?;
    let (rect, rot) = shape_rect(inner, ctx, remap)?;
    Some(SlideElement::Line {
        id: new_id(),
        transform: ctx.geo.transform(rect, rot),
        line_kind: line_kind.to_string(),
        color,
        stroke_width: canvas_stroke(style.stroke_width_emu, ctx),
        start_arrow: style.start_arrow,
        end_arrow: style.end_arrow,
        path: Some(d.0),
    })
}

/// A Windows or Enhanced Metafile picture drawn as elements (see `metafile_group`), for importers
/// other than this one: `rect` in EMU on a slide mapped by `geo`, turned `rot` degrees, bitmaps
/// stored through `media`.
pub(crate) fn metafile_elements(bytes: &[u8], rect: [f64; 4], rot: f64, geo: Geo, media: &RefCell<HashMap<String, String>>, media_dir: &Path) -> Option<SlideElement> {
    let (theme, inherited) = (Theme::default(), Inherited::default());
    let ctx = Ctx { geo, theme: &theme, map: ClrMap::default(), inh: &inherited, media, media_dir, table_styles: "", bg_fill: None };
    metafile_group(bytes, rect, rot, &ctx, None, None, None)
}

/// A DrawingML chart part drawn as elements (see `pptx_chart`), for importers other than this
/// one: series without their own colour take LibreOffice's default chart colours in turn.
pub(crate) fn chart_elements(chart_xml: &str, rect: [f64; 4], geo: Geo, media: &RefCell<HashMap<String, String>>, media_dir: &Path) -> Option<SlideElement> {
    let mut theme = Theme::default();
    for (i, c) in ["#004586", "#ff420e", "#ffd320", "#579d1c", "#7e0021", "#83caff"].iter().enumerate() {
        theme.colors.insert(format!("accent{}", i + 1), c.to_string());
    }
    let inherited = Inherited::default();
    let ctx = Ctx { geo, theme: &theme, map: ClrMap::default(), inh: &inherited, media, media_dir, table_styles: "", bg_fill: None };
    chart::chart_group(chart_xml, rect, &ctx)
}

/// A Windows Metafile picture as a group of ordinary elements (`wmf`): each path a `Shape` (or a
/// `Line` when open), each string a `TextBlock`, each bitmap an `Image`, laid out over the
/// picture's own rectangle, the group turned as the picture is. `None` when the metafile draws
/// nothing this can read.
#[allow(clippy::too_many_arguments)]
fn metafile_group(bytes: &[u8], rect: [f64; 4], rot: f64, ctx: &Ctx, crop: Option<[f64; 4]>, clear: Option<&str>, recolor: Option<&Recolor>) -> Option<SlideElement> {
    use crate::storage::wmf::WmfItem;
    let wmf = if crate::storage::emf::is_emf(bytes) { crate::storage::emf::parse(bytes)? } else { crate::storage::wmf::parse(bytes)? };
    let (mut fx, mut fy, mut fw, mut fh) = wmf.frame;
    // A crop shows part of the picture's frame in the box (what lies outside isn't clipped).
    if let Some([l, t, r, b]) = crop {
        if l + r < 1.0 && t + b < 1.0 {
            (fx, fy, fw, fh) = (fx + l * fw, fy + t * fh, fw * (1.0 - l - r), fh * (1.0 - t - b));
        }
    }
    // The picture's transparent colour paints nothing.
    // A colour the picture makes see-through is dropped; the rest are recoloured as it says.
    let keep = |c: Option<String>| c.filter(|c| clear.is_none_or(|k| !c.eq_ignore_ascii_case(k))).map(|c| recolor.map_or(c.clone(), |r| r.hex(&c)));
    let (sx, sy) = (rect[2] / fw, rect[3] / fh);
    let at = |(x, y): (f64, f64)| (rect[0] + (x - fx) * sx, rect[1] + (y - fy) * sy);
    let id = new_id;
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
                let (fill, stroke) = (keep(fill), stroke.and_then(|(c, w)| Some((keep(Some(c))?, w))));
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
                let (fill, stroke) = (keep(fill), stroke.and_then(|(c, w)| Some((keep(Some(c))?, w))));
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
                let Some(color) = keep(Some(color)) else { continue };
                let size_emu = size * sy;
                let (w, h) = (text.chars().count() as f64 * size_emu * 0.62 + size_emu, size_emu * 1.3);
                let (ax, ay) = at((x, y));
                let left = match align { "center" => ax - w / 2.0, "right" => ax - w, _ => ax };
                let top = if baseline { ay - size_emu * 0.85 } else { ay };
                // GDI turns text about its anchor point; this app turns a box about its centre,
                // so move the centre where turning about the anchor puts it.
                let (left, top) = {
                    let (sin, cos) = (-angle_deg).to_radians().sin_cos();
                    let (dx, dy) = (left + w / 2.0 - ax, top + h / 2.0 - ay);
                    (ax + dx * cos - dy * sin - w / 2.0, ay + dx * sin + dy * cos - h / 2.0)
                };
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
                // Keyed by content: a master's metafile logo is drawn on every slide, decoded and
                // stored once.
                let key = { use std::hash::{Hash, Hasher}; let mut h = std::collections::hash_map::DefaultHasher::new(); bmp.hash(&mut h); format!("wmf-bitmap:{:x}:{}:{}:{}", h.finish(), bmp.len(), clear.unwrap_or(""), recolor.map(Recolor::key).unwrap_or_default()) };
                let stored = ctx.media.borrow().get(&key).cloned();
                let file_path = stored.or_else(|| {
                    let mut img = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp).ok()?.to_rgba8();
                    if let Some(c) = clear {
                        clear_color(&mut img, c);
                    }
                    if let Some(r) = recolor {
                        r.apply(&mut img);
                    }
                    store_media(key, &encode_picture(img, false)?, ctx)
                });
                if let Some(file_path) = file_path {
                    let (x0, y0) = at((x, y));
                    children.push(SlideElement::Image { id: id(), transform: ctx.geo.transform([x0, y0, w * sx, h * sy], 0.0), file_path, crop: None, mask_shape: None, alt_text: None, fit: Some("stretch".into()) });
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
    // The group's box is the picture's, turned as the picture is: the renderer turns a group's box
    // with its children inside it.
    Some(SlideElement::Group { id: id(), transform: ctx.geo.transform(rect, rot), children })
}

/// One part of a table style (`wholeTbl`, `firstRow`, `band1H`, ...), or a cell's own `<a:tcPr>`.
#[derive(Default, Clone)]
struct CellLook {
    fill: Option<Option<String>>,
    text_color: Option<String>,
    bold: Option<bool>,
    /// A style part's borders as it states them (a line, or `None` for none): left, right, top,
    /// bottom, then inside horizontal and inside vertical.
    borders: [Option<Option<(String, f64)>>; 6],
}

type Edge = Option<Option<(String, f64)>>;

impl CellLook {
    fn over(&self, base: &CellLook) -> CellLook {
        CellLook {
            fill: self.fill.clone().or_else(|| base.fill.clone()),
            text_color: self.text_color.clone().or_else(|| base.text_color.clone()),
            bold: self.bold.or(base.bold),
            // Borders don't merge: each part's apply over the region it covers (see `sides`).
            borders: Default::default(),
        }
    }

    /// The sides (top, right, bottom, left) this part gives the cell spanning rows `r..r2` and
    /// columns `c..c2`, the part covering rows `rr` and columns `cc`: its outer lines on the
    /// region's edges, its inside lines between.
    fn sides(&self, (r, r2, c, c2): (usize, usize, usize, usize), rr: (usize, usize), cc: (usize, usize)) -> [Edge; 4] {
        let b = &self.borders;
        let pick = |outer: bool, side: usize, inside: usize| if outer { b[side].clone() } else { b[inside].clone() };
        [pick(r <= rr.0, 2, 4), pick(c2 >= cc.1, 1, 5), pick(r2 >= rr.1, 3, 4), pick(c <= cc.0, 0, 5)]
    }
}

/// A line (`<a:ln>` content) as a colour and width, `None` when it's `noFill`.
fn border_of(ln_attrs: &str, ln_inner: &str, ctx: &Ctx) -> Option<(String, f64)> {
    let fill = children(ln_inner).into_iter().find(|(n, _, _)| matches!(*n, "a:noFill" | "a:solidFill"))?;
    if fill.0 == "a:noFill" {
        return None;
    }
    let color = ctx.palette().first_color(fill.2, None)?;
    Some((color, extract_attr(ln_attrs, "w").and_then(|w| w.parse::<f64>().ok()).unwrap_or(12_700.0)))
}

/// A table-style part: its text (`tcTxStyle`) and cell (`tcStyle`: fill, borders) look.
fn style_part(part_inner: &str, ctx: &Ctx) -> CellLook {
    let pal = ctx.palette();
    let mut look = CellLook::default();
    if let Some((_, attrs, tx)) = child(part_inner, "a:tcTxStyle") {
        look.bold = extract_attr(attrs, "b").map(|b| b == "on");
        // The text colour is the style's own colour element, else its fontRef's.
        look.text_color = children(tx).into_iter().filter(|(n, _, _)| *n != "a:fontRef" && *n != "a:font")
            .find_map(|(n, a, i)| pal.color(n, a, i, None))
            .or_else(|| child(tx, "a:fontRef").and_then(|(_, _, i)| pal.first_color(i, None)));
    }
    if let Some((_, _, tc)) = child(part_inner, "a:tcStyle") {
        // A fill or line is stated outright, or a reference into the theme with its colour.
        if let Some((_, _, fill)) = child(tc, "a:fill") {
            look.fill = children(fill).into_iter().next().map(|(n, _, i)| if n == "a:noFill" { None } else { fill_color(n, i, ctx, None) });
        } else if let Some((_, a, i)) = child(tc, "a:fillRef") {
            look.fill = Some(if extract_attr(a, "idx") == Some("0") { None } else { pal.first_color(i, None) });
        }
        if let Some((_, _, bdr)) = child(tc, "a:tcBdr") {
            for (i, name) in ["a:left", "a:right", "a:top", "a:bottom", "a:insideH", "a:insideV"].iter().enumerate() {
                let Some((_, _, side)) = child(bdr, name) else { continue };
                look.borders[i] = Some(if let Some((_, a, l)) = child(side, "a:ln") {
                    border_of(a, l, ctx)
                } else if let Some((_, a, l)) = child(side, "a:lnRef") {
                    if extract_attr(a, "idx") == Some("0") { None } else { pal.first_color(l, None).map(|c| (c, 12_700.0)) }
                } else {
                    None
                });
            }
        }
    }
    look
}

/// PowerPoint's built-in table styles (`pptx_table_styles.xml`), for a deck that names one by id
/// without defining it.
static BUILTIN_TABLE_STYLES: &str = include_str!("pptx_table_styles.xml");

/// The contents of the `<a:tblStyle>` with this id in a `tableStyles.xml`.
fn table_style<'a>(list: &'a str, id: &str) -> Option<&'a str> {
    children(next_tag(list, "a:tblStyleLst").map(|(_, i, _)| i).unwrap_or(""))
        .into_iter()
        .find(|(n, a, _)| *n == "a:tblStyle" && extract_attr(a, "styleId") == Some(id))
        .map(|(_, _, i)| i)
}

/// One of PowerPoint's built-in table styles by id, from an index built once.
fn builtin_table_style(id: &str) -> Option<&'static str> {
    static INDEX: std::sync::OnceLock<HashMap<&'static str, &'static str>> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| {
        children(next_tag(BUILTIN_TABLE_STYLES, "a:tblStyleLst").map(|(_, i, _)| i).unwrap_or(""))
            .into_iter()
            .filter(|(n, _, _)| *n == "a:tblStyle")
            .filter_map(|(_, a, i)| Some((extract_attr(a, "styleId")?, i)))
            .collect()
    }).get(id).copied()
}

/// About how tall a text body wraps to at `width` EMU: each paragraph word-wrapped by its runs'
/// fonts (theme fonts resolved through the deck's theme), in lines 1.2 of its size apart.
fn estimated_text_height(txbody: &str, width: f64, ctx: &Ctx) -> f64 {
    use crate::storage::text_metrics::{wrapped_lines, Span};
    let mut total = 0.0;
    for (_, _, p) in children(txbody).into_iter().filter(|(nm, _, _)| *nm == "a:p") {
        let size_of = |rpr: Option<&str>| rpr.and_then(|a| extract_attr(a, "sz")?.parse::<f64>().ok()).map(|sz| sz / 100.0 * 12_700.0);
        let para_size = size_of(next_tag(p, "a:rPr").or_else(|| next_tag(p, "a:endParaRPr")).map(|(a, _, _)| a)).unwrap_or(18.0 * 12_700.0);
        let indent = next_tag(p, "a:pPr").and_then(|(a, _, _)| extract_attr(a, "marL")?.parse::<f64>().ok()).unwrap_or(0.0);
        let wrap_width = (width - indent).max(para_size);
        let mut texts: Vec<(String, String, bool, f64)> = Vec::new();
        let mut lines = 0usize;
        let flush = |texts: &mut Vec<(String, String, bool, f64)>, lines: &mut usize| {
            let spans: Vec<Span> = texts.iter().map(|(t, f, b, s)| Span { text: t, family: f, bold: *b, size: *s }).collect();
            *lines += wrapped_lines(&spans, wrap_width);
            texts.clear();
        };
        for (nm, _, r) in children(p) {
            match nm {
                "a:r" | "a:fld" => {
                    let rpr = next_tag(r, "a:rPr");
                    let family = ctx.font(rpr.and_then(|(_, i, _)| next_tag(i, "a:latin")).and_then(|(a, _, _)| extract_attr(a, "typeface")));
                    let bold = rpr.and_then(|(a, _, _)| extract_attr(a, "b")).is_some_and(|b| b == "1" || b == "true");
                    let text = next_tag_content(r, "a:t").map_or(String::new(), |(t, _)| decode_xml_entities(t));
                    texts.push((text, family, bold, size_of(rpr.map(|(a, _, _)| a)).unwrap_or(para_size)));
                }
                "a:br" => flush(&mut texts, &mut lines),
                _ => {}
            }
        }
        flush(&mut texts, &mut lines);
        total += lines as f64 * para_size * 1.2;
    }
    total
}

/// Builds a table (`<p:graphicFrame>` around an `<a:tbl>`) as a group of its cells: each a filled,
/// outlined box with its own formatted text, sized by the real column widths and row heights, merged
/// cells spanning. The look comes from the deck's table style (whole table, header row, banded rows
/// and columns, first/last column and row, as the table turns them on) under each cell's own.
fn extract_table_element(gf_inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>) -> Option<SlideElement> {
    let (_, xfrm_content, _) = next_tag(gf_inner, "p:xfrm")?;
    let (_, tbl, _) = next_tag(gf_inner, "a:tbl")?;
    let n = |a: &str, k: &str| extract_attr(a, k).and_then(|s| s.parse::<f64>().ok());
    let (off, _, _) = next_tag(xfrm_content, "a:off")?;
    let (ext, _, _) = next_tag(xfrm_content, "a:ext")?;
    let (mut x0, mut y0, mut fw, mut fh) = (n(off, "x")?, n(off, "y")?, n(ext, "cx")?, n(ext, "cy")?);
    let mut sx = 1.0;
    let mut sy = 1.0;
    if let Some(r) = remap {
        let (x, y, w, h) = r.apply(x0, y0, fw, fh);
        sx = if fw > 0.0 { w / fw } else { 1.0 };
        sy = if fh > 0.0 { h / fh } else { 1.0 };
        (x0, y0, fw, fh) = (x, y, w, h);
    }
    let _ = (fw, fh);

    let (tbl_pr_attrs, tbl_pr) = child(tbl, "a:tblPr").map(|(_, a, i)| (a, i)).unwrap_or(("", ""));
    let on = |k: &str| extract_attr(tbl_pr_attrs, k) == Some("1");
    // The table's own style, else the deck's default (`<a:tblStyleLst def>`), as LibreOffice applies it.
    let style_id = child(tbl_pr, "a:tableStyleId").map(|(_, _, i)| i.trim().to_string())
        .or_else(|| next_tag(ctx.table_styles, "a:tblStyleLst").and_then(|(a, _, _)| extract_attr(a, "def")).map(str::to_string));
    // The deck's own definition, else PowerPoint's built-in one: decks refer to built-in styles by
    // id without always writing them out.
    let style_xml = style_id.as_deref().and_then(|id| table_style(ctx.table_styles, id).or_else(|| builtin_table_style(id))).unwrap_or("");
    let part = |name: &str| child(style_xml, name).map(|(_, _, i)| style_part(i, ctx)).unwrap_or_default();
    let (whole, first_row, last_row, first_col, last_col, band1h, band1v) =
        (part("a:wholeTbl"), part("a:firstRow"), part("a:lastRow"), part("a:firstCol"), part("a:lastCol"), part("a:band1H"), part("a:band1V"));

    let cols: Vec<f64> = next_tag(tbl, "a:tblGrid").map(|(_, g, _)| children(g).into_iter().filter_map(|(_, a, _)| n(a, "w")).map(|w| w * sx).collect()).unwrap_or_default();
    let mut rows: Vec<(f64, &str)> = children(tbl).into_iter().filter(|(nm, _, _)| *nm == "a:tr").map(|(_, a, i)| (n(a, "h").unwrap_or(370_840.0) * sy, i)).collect();
    if cols.is_empty() || rows.is_empty() {
        return None;
    }
    let col_x: Vec<f64> = std::iter::once(0.0).chain(cols.iter().scan(0.0, |acc, w| { *acc += w; Some(*acc) })).collect();
    // A row's height is a minimum: it grows to fit its text, as PowerPoint lays tables out.
    for (h, row) in rows.iter_mut() {
        for (c, (_, a, tc)) in children(row).into_iter().filter(|(nm, _, _)| *nm == "a:tc").enumerate().take(cols.len()) {
            if n(a, "rowSpan").unwrap_or(1.0) > 1.0 || extract_attr(a, "hMerge") == Some("1") || extract_attr(a, "vMerge") == Some("1") {
                continue;
            }
            let span = (n(a, "gridSpan").unwrap_or(1.0).max(1.0) as usize + c).min(cols.len());
            let tcpr = child(tc, "a:tcPr").map(|(_, a, _)| a).unwrap_or("");
            let m = |k: &str, d: f64| n(tcpr, k).unwrap_or(d);
            let width = col_x[span] - col_x[c] - m("marL", 91_440.0) - m("marR", 91_440.0);
            let needed = next_tag(tc, "a:txBody").map(|(_, b, _)| estimated_text_height(b, width, ctx)).unwrap_or(0.0) + m("marT", 45_720.0) + m("marB", 45_720.0);
            *h = h.max(needed);
        }
    }
    let row_y: Vec<f64> = std::iter::once(0.0).chain(rows.iter().scan(0.0, |acc, (h, _)| { *acc += h; Some(*acc) })).collect();
    let (nrows, ncols) = (rows.len(), cols.len());

    let id = new_id;
    let mut children_out = Vec::new();
    let mut edges = Vec::new();
    for (r, (_, row)) in rows.iter().enumerate() {
        for (c, (_, tc_attrs, tc)) in children(row).into_iter().filter(|(nm, _, _)| *nm == "a:tc").enumerate().take(ncols) {
            if extract_attr(tc_attrs, "hMerge") == Some("1") || extract_attr(tc_attrs, "vMerge") == Some("1") {
                continue;
            }
            let span_c = n(tc_attrs, "gridSpan").unwrap_or(1.0).max(1.0) as usize;
            let span_r = n(tc_attrs, "rowSpan").unwrap_or(1.0).max(1.0) as usize;
            let (c2, r2) = ((c + span_c).min(ncols), (r + span_r).min(nrows));
            let rect = [x0 + col_x[c], y0 + row_y[r], col_x[c2] - col_x[c], row_y[r2] - row_y[r]];

            // Low to high: whole table, banding, first/last column, last row, header row, the cell's own.
            // Each part's borders apply over the region it covers (its row, its column, the table).
            let cell = (r, r2, c, c2);
            let mut look = whole.clone();
            let mut sides = whole.sides(cell, (0, nrows), (0, ncols));
            let mut apply = |part: &CellLook, rr: (usize, usize), cc: (usize, usize), look: &mut CellLook| {
                *look = part.over(look);
                for (side, edge) in sides.iter_mut().zip(part.sides(cell, rr, cc)) {
                    if edge.is_some() {
                        *side = edge;
                    }
                }
            };
            // Bands count from the first row (column) after the header, if there is one.
            let (band_r, band_c) = (r as isize - isize::from(on("firstRow")), c as isize - isize::from(on("firstCol")));
            if on("bandRow") && band_r >= 0 && band_r % 2 == 0 {
                apply(&band1h, (r, r2), (0, ncols), &mut look);
            }
            if on("bandCol") && band_c >= 0 && band_c % 2 == 0 {
                apply(&band1v, (0, nrows), (c, c2), &mut look);
            }
            if on("firstCol") && c == 0 { apply(&first_col, (0, nrows), (0, 1), &mut look); }
            if on("lastCol") && c2 == ncols { apply(&last_col, (0, nrows), (ncols - 1, ncols), &mut look); }
            if on("lastRow") && r2 == nrows { apply(&last_row, (nrows - 1, nrows), (0, ncols), &mut look); }
            if on("firstRow") && r == 0 { apply(&first_row, (0, 1), (0, ncols), &mut look); }
            let (tcpr_attrs, tcpr) = child(tc, "a:tcPr").map(|(_, a, i)| (a, i)).unwrap_or(("", ""));
            let mut own = CellLook::default();
            if let Some((name, _, inner)) = children(tcpr).into_iter().find(|(nm, _, _)| matches!(*nm, "a:noFill" | "a:solidFill" | "a:gradFill")) {
                own.fill = Some(if name == "a:noFill" { None } else { fill_color(name, inner, ctx, None) });
            }
            // The cell's own lines, side by side, over the style's.
            for (side, tag) in sides.iter_mut().zip(["a:lnT", "a:lnR", "a:lnB", "a:lnL"]) {
                if let Some((_, a, i)) = child(tcpr, tag) {
                    *side = Some(border_of(a, i, ctx));
                }
            }
            let look = own.over(&look);

            if let Some(fill) = look.fill.clone().flatten() {
                children_out.push(SlideElement::Shape { id: id(), transform: ctx.geo.transform(rect, 0.0), shape_kind: "path".into(), fill_color: fill, stroke_width: 0.0, stroke_color: None, path: Some("M 0 0 L 100 0 L 100 100 L 0 100 Z".into()) });
            }
            // Borders go over every cell's fill (a neighbour's would hide a shared edge).
            edges.extend(cell_edges(rect, &sides.map(Option::flatten), &ctx.geo));

            if let Some((_, body, _)) = next_tag(tc, "a:txBody") {
                let mut list = ctx.inh.default_style.clone();
                for l in list.0.iter_mut() {
                    if look.text_color.is_some() {
                        l.run.color = None;
                    }
                    if look.bold.is_some() {
                        l.run.bold = look.bold;
                    }
                }
                let own_list = child(body, "a:lstStyle").map(|(_, _, i)| inh::ListStyle::parse(i)).unwrap_or_default();
                let list = own_list.over(&list);
                let margins = |k: &str, d: f64| Some(n(tcpr_attrs, k).unwrap_or(d));
                let props = inh::BodyProps {
                    anchor: extract_attr(tcpr_attrs, "anchor").map(str::to_string),
                    insets: [margins("marL", 91_440.0), margins("marT", 45_720.0), margins("marR", 91_440.0), margins("marB", 45_720.0)],
                    ..Default::default()
                };
                children_out.extend(text_block_element(body, &list, &props, look.text_color.clone(), rect, 0.0, ctx));
            }
        }
    }
    children_out.extend(edges);
    if children_out.is_empty() {
        return None;
    }
    for (z, el) in children_out.iter_mut().enumerate() {
        el.transform_mut().z_index = z as i32;
    }
    let (w, h) = (col_x[ncols], row_y[nrows]);
    Some(SlideElement::Group { id: id(), transform: ctx.geo.transform([x0, y0, w, h], 0.0), children: children_out })
}

/// A graphic frame's slide rectangle (EMU), out of its group if it's in one.
fn frame_rect(inner: &str, remap: Option<&GroupRemap>) -> Option<[f64; 4]> {
    let (_, xfrm, _) = next_tag(inner, "p:xfrm")?;
    let n = |tag: &str, k: &str| next_tag(xfrm, tag).and_then(|(a, _, _)| extract_attr(a, k)?.parse::<f64>().ok());
    let (x, y, w, h) = (n("a:off", "x")?, n("a:off", "y")?, n("a:ext", "cx")?, n("a:ext", "cy")?);
    let (x, y, w, h) = match remap { Some(r) => r.apply(x, y, w, h), None => (x, y, w, h) };
    Some([x, y, w, h])
}

#[path = "pptx_chart.rs"]
mod chart;

/// A `<p:graphicFrame>`: a table, or a SmartArt diagram drawn from the copy PowerPoint saves of it.
fn extract_graphic_frame(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>, decorations_only: bool) -> Result<Option<SlideElement>, Box<dyn std::error::Error>> {
    let uri = next_tag(inner, "a:graphicData").and_then(|(a, _, _)| extract_attr(a, "uri")).unwrap_or("");
    if uri.ends_with("/diagram") {
        return extract_smartart(archive, part_path, inner, ctx, remap, decorations_only);
    }
    if uri.ends_with("/ole") {
        return Ok(ole_preview(archive, part_path, inner, ctx, remap));
    }
    if uri.ends_with("/chart") {
        let Some(rid) = next_tag(inner, "c:chart").and_then(|(a, _, _)| extract_attr(a, "r:id")) else { return Ok(None) };
        let Some(chart_path) = embedded_path(archive, part_path, rid) else { return Ok(None) };
        let Ok(chart_xml) = read_zip_text(archive, &chart_path) else { return Ok(None) };
        let Some(rect) = frame_rect(inner, remap) else { return Ok(None) };
        return Ok(chart::chart_group(&chart_xml, rect, ctx));
    }
    Ok(extract_table_element(inner, ctx, remap))
}

/// A SmartArt diagram, from the drawing PowerPoint saves alongside its data (`ppt/diagrams/drawingN.xml`):
/// ordinary shapes in the frame's own coordinates. The data part names that drawing
/// (`<dsp:dataModelExt relId>`, a relationship of the slide); the diagram's frame places it.
fn extract_smartart(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, inner: &str, ctx: &Ctx, remap: Option<&GroupRemap>, decorations_only: bool) -> Result<Option<SlideElement>, Box<dyn std::error::Error>> {
    let Some(data_rid) = next_tag(inner, "dgm:relIds").and_then(|(a, _, _)| extract_attr(a, "r:dm")) else { return Ok(None) };
    let Some(data_path) = embedded_path(archive, part_path, data_rid) else { return Ok(None) };
    let data_xml = read_zip_text(archive, &data_path).unwrap_or_default();
    let Some((_, xfrm, _)) = next_tag(inner, "p:xfrm") else { return Ok(None) };
    let n = |tag: &str, k: &str| next_tag(xfrm, tag).and_then(|(a, _, _)| extract_attr(a, k)?.parse::<f64>().ok());
    let (Some(x), Some(y), Some(w), Some(h)) = (n("a:off", "x"), n("a:off", "y"), n("a:ext", "cx"), n("a:ext", "cy")) else { return Ok(None) };
    let own = GroupRemap { off_x: x, off_y: y, ext_cx: w, ext_cy: h, ch_off_x: 0.0, ch_off_y: 0.0, ch_ext_cx: w, ch_ext_cy: h };
    let frame = match remap {
        Some(r) => {
            let (ox, oy, cw, ch) = r.apply(x, y, w, h);
            GroupRemap { off_x: ox, off_y: oy, ext_cx: cw, ext_cy: ch, ..own }
        }
        None => own,
    };
    let rect = [frame.off_x, frame.off_y, frame.ext_cx, frame.ext_cy];
    // The drawing PowerPoint saves with the diagram (its shapes as laid out), when there is one.
    let drawing = next_tag(&data_xml, "dsp:dataModelExt")
        .and_then(|(a, _, _)| extract_attr(a, "relId").map(str::to_string))
        .and_then(|rid| embedded_path(archive, part_path, &rid))
        .and_then(|path| Some((read_zip_text(archive, &path).ok()?, path)));
    let mut children = Vec::new();
    if let Some((drawing, drawing_path)) = drawing {
        // The drawing's shapes are the same markup under another prefix, placed from the frame's corner.
        let drawing = drawing.replace("<dsp:", "<p:").replace("</dsp:", "</p:");
        if let Some(tree) = next_tag_content(&drawing, "p:spTree").map(|(c, _)| c) {
            // The drawing's own shapes are its picture, not placeholders to skip.
            let _ = decorations_only;
            children = extract_shapes_in_scope(archive, &drawing_path, tree, ctx, Some(&frame), false)?;
        }
    }
    if children.is_empty() {
        let layout = next_tag(inner, "dgm:relIds").and_then(|(a, _, _)| extract_attr(a, "r:lo"))
            .and_then(|rid| embedded_path(archive, part_path, rid))
            .and_then(|path| read_zip_text(archive, &path).ok())
            .and_then(|xml| next_tag(&xml, "dgm:layoutDef").and_then(|(a, _, _)| extract_attr(a, "uniqueId").map(|u| u.rsplit('/').next().unwrap_or(u).to_string())))
            .unwrap_or_default();
        children = smartart_outline(&data_xml, &layout, rect, ctx);
    }
    if children.is_empty() {
        return Ok(None);
    }
    Ok(Some(SlideElement::Group { id: new_id(), transform: ctx.geo.transform(rect, 0.0), children }))
}

/// A diagram saved without its drawing (other programs write only the data): its content laid out
/// plainly in `rect` so nothing it says is lost, though its exact layout is. Block lists (the
/// `layout` families `default`, horizontal and vertical lists and processes) get a box per
/// top-level item -- in a grid, a row or a column -- with its sub-items as bullet lines; any other
/// layout (cycles, pyramids, hierarchies...) is drawn as just its text, an indented outline.
fn smartart_outline(data_xml: &str, layout: &str, rect: [f64; 4], ctx: &Ctx) -> Vec<SlideElement> {
    let Some((_, model, _)) = next_tag(data_xml, "dgm:dataModel") else { return Vec::new() };
    let mut text: HashMap<&str, String> = HashMap::new();
    let mut doc = None;
    for (_, a, i) in next_tag(model, "dgm:ptLst").map(|(_, l, _)| children(l)).unwrap_or_default() {
        let (Some(id), kind) = (extract_attr(a, "modelId"), extract_attr(a, "type").unwrap_or("node")) else { continue };
        match kind {
            "doc" => doc = Some(id),
            "node" => {
                let t = next_tag(i, "dgm:t").map(|(_, t, _)| children(t).into_iter().filter(|(n, _, _)| *n == "a:p").map(|(_, _, p)| {
                    children(p).into_iter().filter(|(n, _, _)| *n == "a:r").flat_map(|(_, _, r)| children(r).into_iter().filter(|(n, _, _)| *n == "a:t").map(|(_, _, t)| decode_xml_entities(t))).collect::<String>()
                }).filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                text.insert(id, t);
            }
            _ => {}
        }
    }
    // Parent -> children, in their order.
    let mut kids: HashMap<&str, Vec<(i64, &str)>> = HashMap::new();
    for (_, a, _) in next_tag(model, "dgm:cxnLst").map(|(_, l, _)| children(l)).unwrap_or_default() {
        if extract_attr(a, "type").is_some_and(|t| t != "parOf") {
            continue;
        }
        if let (Some(src), Some(dest)) = (extract_attr(a, "srcId"), extract_attr(a, "destId")) {
            let ord = extract_attr(a, "srcOrd").and_then(|v| v.parse().ok()).unwrap_or(0);
            kids.entry(src).or_default().push((ord, dest));
        }
    }
    kids.values_mut().for_each(|v| v.sort());
    let child_ids = |id: &str| kids.get(id).map(|v| v.iter().map(|(_, d)| *d).filter(|d| text.contains_key(d)).collect::<Vec<_>>()).unwrap_or_default();
    let Some(doc) = doc else { return Vec::new() };
    let top = child_ids(doc);
    if top.is_empty() {
        return Vec::new();
    }
    let n = top.len().min(64);
    let pal = ctx.palette();
    let family = layout.trim_end_matches(|c: char| c.is_ascii_digit() || c == '#');
    let cols = match family {
        // A grid about as wide as the frame is.
        "default" => ((n as f64 * rect[2] / rect[3].max(1.0)).sqrt().round() as usize).clamp(1, n),
        "hList" | "hProcess" | "lProcess" | "process" => n,
        "vList" | "vProcess" | "bList" | "list" => 1,
        _ => {
            // Just the text: each item, its sub-items indented beneath it.
            let ink = pal.color("a:schemeClr", r#"val="tx1""#, "", None).unwrap_or_else(|| "#000000".into());
            let lines: Vec<(usize, String)> = top.iter().take(n).flat_map(|id| {
                std::iter::once((0, text[id].clone())).chain(child_ids(id).into_iter().take(32).map(|c| (1, text[c].clone())))
            }).filter(|(_, t)| !t.trim().is_empty()).collect();
            if lines.is_empty() {
                return Vec::new();
            }
            let font_pt = (rect[3] / 12_700.0 / (lines.len() as f64 * 1.5 + 0.5)).clamp(8.0, 32.0) * ctx.geo.font_scale;
            let mut runs = Vec::new();
            for (i, (depth, t)) in lines.into_iter().enumerate() {
                let run = |t: String| crate::core::models::TextRun { text: t, color: ink.clone(), font_family: ctx.font(None), font_size_pt: (font_pt * 10.0).round() / 10.0, ..Default::default() };
                if i > 0 {
                    runs.push(run("\n".into()));
                }
                runs.push(run(if depth == 0 { t } else { format!("\u{2002}\u{2002}• {t}") }));
            }
            return vec![SlideElement::TextBlock {
                id: new_id(),
                transform: ctx.geo.transform(rect, 0.0),
                block: crate::core::models::TextBlock {
                    runs,
                    paragraph_style: crate::core::models::TextParagraphStyle { align: "left".into(), line_height: 1.2, vertical_align: Some("middle".into()), ..Default::default() },
                    effects: Default::default(),
                    autofit: false,
                },
            }];
        }
    };
    let rows = n.div_ceil(cols);
    // A small gap between boxes, never more than a fifth of the room each row or column has (many
    // items in one line would otherwise leave the boxes negative).
    let gap = (rect[2].min(rect[3]) * 0.03).min(rect[2] / cols as f64 * 0.2).min(rect[3] / rows as f64 * 0.2);
    let (bw, bh) = ((rect[2] - gap * (cols - 1) as f64) / cols as f64, (rect[3] - gap * (rows - 1) as f64) / rows as f64);
    let fill = pal.color("a:schemeClr", r#"val="accent1""#, "", None).unwrap_or_else(|| "#4472c4".into());
    let ink = pal.color("a:schemeClr", r#"val="lt1""#, "", None).unwrap_or_else(|| "#ffffff".into());
    let mut out = Vec::new();
    for (k, id) in top.into_iter().take(n).enumerate() {
        let (c, r) = (k % cols, k / cols);
        let b = [rect[0] + c as f64 * (bw + gap), rect[1] + r as f64 * (bh + gap), bw, bh];
        out.push(SlideElement::Shape { id: new_id(), transform: ctx.geo.transform(b, 0.0), shape_kind: "rounded_rect".into(), fill_color: fill.clone(), stroke_color: None, stroke_width: 0.0, path: None });
        let mut lines = vec![text.get(id).cloned().unwrap_or_default()];
        lines.extend(child_ids(id).into_iter().take(32).map(|c| format!("• {}", text[c])));
        let lines: Vec<String> = lines.into_iter().filter(|l| !l.trim().is_empty()).collect();
        if lines.is_empty() {
            continue;
        }
        // Sized to fit the box: its height shared between the lines, a little smaller for sub-items.
        let font_pt = (bh / 12_700.0 / (lines.len() as f64 * 1.5 + 0.5)).clamp(8.0, 40.0) * ctx.geo.font_scale;
        let runs = lines.iter().enumerate().flat_map(|(i, l)| {
            let size = if i == 0 { font_pt } else { font_pt * 0.8 };
            let run = |t: String| crate::core::models::TextRun { text: t, color: ink.clone(), font_family: ctx.font(None), font_size_pt: (size * 10.0).round() / 10.0, ..Default::default() };
            let mut v = Vec::new();
            if i > 0 {
                v.push(run("\n".into()));
            }
            v.push(run(l.clone()));
            v
        }).collect();
        let inset = gap.max(45_720.0);
        out.push(SlideElement::TextBlock {
            id: new_id(),
            transform: ctx.geo.transform([b[0] + inset, b[1] + inset, (b[2] - 2.0 * inset).max(1.0), (b[3] - 2.0 * inset).max(1.0)], 0.0),
            block: crate::core::models::TextBlock {
                runs,
                paragraph_style: crate::core::models::TextParagraphStyle { align: "center".into(), line_height: 1.2, vertical_align: Some("middle".into()), ..Default::default() },
                effects: Default::default(),
                autofit: false,
            },
        });
    }
    out
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
    Ok(Some(SlideElement::Group { id: new_id(), transform, children }))
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
    // Nesting is bounded: groups within groups, fallbacks and SmartArt drawings (one can name
    // itself) stop drawing past a depth no real deck reaches, rather than overflow the stack.
    const MAX_DEPTH: usize = 32;
    thread_local!(static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });
    struct Level;
    impl Drop for Level {
        fn drop(&mut self) {
            DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        }
    }
    if DEPTH.with(|d| d.get()) >= MAX_DEPTH {
        return Ok(Vec::new());
    }
    DEPTH.with(|d| d.set(d.get() + 1));
    let _level = Level;
    let mut elements = Vec::new();
    for (name, attrs, inner) in children(scope_xml) {
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
                // A shape filled with a picture: the picture, cut to the shape's outline, under the rest.
                elements.extend(picture_filled_shape(archive, part_path, inner, ctx, remap));
                // A filled or outlined box with text in it is two elements: the box, then its text.
                let use_bg_fill = extract_attr(attrs, "useBgFill").is_some_and(|v| v == "1" || v == "true");
                elements.extend(extract_autoshape_element(inner, ctx, remap, use_bg_fill));
                elements.extend(text);
            }
            "p:pic" => elements.extend(extract_picture_element(archive, part_path, inner, ctx, remap)),
            "p:cxnSp" => elements.extend(extract_line_element(inner, ctx, remap)),
            "p:graphicFrame" => elements.extend(extract_graphic_frame(archive, part_path, inner, ctx, remap, decorations_only)?),
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
        // An empty `<p:bg/>` states nothing: the layout's or master's background shows.
    }
    None
}

fn background_from_fill(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, name: &str, _attrs: &str, inner: &str, ctx: &Ctx, ph: Option<&str>) -> Option<SlideBackground> {
    let pal = ctx.palette();
    match name {
        "a:solidFill" => pal.first_color(inner, ph).map(SlideBackground::Solid),
        "a:pattFill" => child(inner, "a:fgClr").and_then(|(_, _, i)| pal.first_color(i, ph)).map(SlideBackground::Solid),
        "a:gradFill" => gradient_of(inner, ctx, ph),
        "a:blipFill" => {
            let (blip_attrs, blip_inner, _) = next_tag(inner, "a:blip")?;
            let rid = extract_attr(blip_attrs, "r:embed")?.to_string();
            // A see-through picture (`<a:alphaModFix amt>`, thousandths of a percent of opaque).
            let opacity = next_tag(blip_inner, "a:alphaModFix").and_then(|(a, _, _)| extract_attr(a, "amt")?.parse::<f64>().ok()).map_or(1.0, |v| (v / 100_000.0).clamp(0.0, 1.0));
            let recolor = Recolor::of(blip_inner, ctx, ph);
            let tile = next_tag(inner, "a:tile").map(|(a, _, _)| a);
            let drawn = if tile.is_some() || recolor.is_some() { {
                // The whole canvas, the first tile aligned on the slide (not the pillarbox).
                let geo = ctx.geo;
                drawn_picture(archive, part_path, &rid, tile, recolor.as_ref(), [geo.canvas_w, geo.canvas_h], [geo.x_off, geo.y_off, geo.canvas_w - 2.0 * geo.x_off, geo.canvas_h - 2.0 * geo.y_off], ctx)
            } } else { None };
            drawn.or_else(|| extract_embedded_image(archive, part_path, &rid, ctx)).map(|file_path| SlideBackground::Image { file_path, opacity })
        }
        _ => None,
    }
}

/// A `<a:gradFill>` as a gradient (a single stop is just a colour). Stops keep their opacity.
fn gradient_of(inner: &str, ctx: &Ctx, ph: Option<&str>) -> Option<SlideBackground> {
    let pal = ctx.palette();
    let mut stops: Vec<GradientStop> = next_tag(inner, "a:gsLst")
        .map(|(_, l, _)| children(l).into_iter().filter_map(|(_, a, i)| {
            let pos = extract_attr(a, "pos").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100_000.0;
            Some(GradientStop { color: pal.first_color_with_alpha(i, ph)?, offset: pos })
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

/// A picture's recolouring (`<a:blip>`'s `<a:duotone>` or `<a:grayscl/>`), as PowerPoint's
/// "Recolor" and theme background textures use: each pixel's brightness picks a colour between
/// the two (or a grey).
#[derive(Clone, Debug, PartialEq)]
enum Recolor {
    Duotone([f64; 3], [f64; 3]),
    Grayscale,
}

impl Recolor {
    /// `ph` stands in for `phClr` (a theme background style passes its colour in).
    fn of(blip_inner: &str, ctx: &Ctx, ph: Option<&str>) -> Option<Recolor> {
        if child(blip_inner, "a:grayscl").is_some() {
            return Some(Recolor::Grayscale);
        }
        let (_, _, duo) = child(blip_inner, "a:duotone")?;
        let pal = ctx.palette();
        let colors: Vec<[f64; 3]> = children(duo).into_iter().filter_map(|(n, a, i)| pal.color(n, a, i, ph)).take(2).map(|c| {
            let v = |k: usize| u8::from_str_radix(c.get(k..k + 2).unwrap_or("00"), 16).unwrap_or(0) as f64;
            [v(1), v(3), v(5)]
        }).collect();
        (colors.len() == 2).then(|| Recolor::Duotone(colors[0], colors[1]))
    }

    fn key(&self) -> String {
        format!("{self:?}")
    }

    /// One colour (by its luminance) as this recolour makes it.
    fn rgb(&self, [r, g, b]: [u8; 3]) -> [u8; 3] {
        let lum = (0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64) / 255.0;
        let c = match self {
            Recolor::Grayscale => [lum * 255.0; 3],
            Recolor::Duotone(a, b) => [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * lum),
        };
        c.map(|v| v.round().clamp(0.0, 255.0) as u8)
    }

    fn apply(&self, img: &mut image::RgbaImage) {
        for p in img.pixels_mut() {
            let [r, g, b] = self.rgb([p[0], p[1], p[2]]);
            (p[0], p[1], p[2]) = (r, g, b);
        }
    }

    /// A `#rrggbb[aa]` colour recoloured (its alpha kept); anything else as it is.
    fn hex(&self, color: &str) -> String {
        let h = color.trim_start_matches('#');
        let v = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
        let (Some(r), Some(g), Some(b)) = (v(0), v(2), v(4)) else { return color.to_string() };
        let [r, g, b] = self.rgb([r, g, b]);
        format!("#{r:02x}{g:02x}{b:02x}{}", h.get(6..8).unwrap_or(""))
    }
}

/// A drawn picture as file bytes: a JPEG (quality 90) for an opaque one made from a photograph
/// (`photo`, its source a JPEG), else a lossless PNG -- diagrams, text and line art keep their edges.
pub(crate) fn encode_picture(img: image::RgbaImage, photo: bool) -> Option<Vec<u8>> {
    let mut encoded = Cursor::new(Vec::new());
    if photo && img.pixels().all(|p| p[3] == 255) {
        let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 90).encode_image(&rgb).ok()?;
    } else {
        img.write_to(&mut encoded, image::ImageFormat::Png).ok()?;
    }
    Some(encoded.into_inner())
}

/// A picture that needs drawing at import -- a background's or a shape's fill: recoloured, and/or
/// tiled (`<a:tile>`) into a picture `size` EMU big, the first tile placed within `area` (see
/// `tile_picture`), so it needs nothing new to render. `None` (show it as it is) for a picture
/// that can't be decoded, such as a metafile.
#[allow(clippy::too_many_arguments)]
fn drawn_picture(archive: &mut ZipArchive<Cursor<&[u8]>>, part_path: &str, rid: &str, tile: Option<&str>, recolor: Option<&Recolor>, size: [f64; 2], area: [f64; 4], ctx: &Ctx) -> Option<String> {
    let media_path = embedded_path(archive, part_path, rid)?;
    let key = format!("{media_path}#tile:{}#{}#{}x{}:{:?}", tile.unwrap_or(""), recolor.map(Recolor::key).unwrap_or_default(), size[0].round(), size[1].round(), area.map(f64::round));
    if let Some(done) = ctx.media.borrow().get(&key) {
        return Some(done.clone());
    }
    let bytes = read_picture_bytes(archive, &media_path).ok()?;
    let mut src = image::load_from_memory(&bytes).ok()?.to_rgba8();
    if let Some(r) = recolor {
        r.apply(&mut src);
    }
    let photo = is_jpeg(&bytes);
    let Some(tile) = tile else { return store_media(key, &encode_picture(src, photo)?, ctx) };
    let out = tile_picture(&src, image_dpi(&bytes), tile, size, area, 1920.0 / ctx.geo.canvas_w);
    store_media(key, &encode_picture(out, photo)?, ctx)
}

/// `src` tiled over a box `size` EMU big (`<a:tile>`, as PowerPoint and LibreOffice tile): each tile
/// is the picture's own size (from its DPI, else 96 dpi) scaled by `sx`/`sy`; the first one is
/// placed by `algn` within `area` (x, y, w, h in the box) and moved by `tx`/`ty` (EMU); every
/// other one is mirrored as `flip` says. Drawn at `scale` pixels per EMU.
pub(crate) fn tile_picture(src: &image::RgbaImage, dpi: Option<(f64, f64)>, tile: &str, size: [f64; 2], area: [f64; 4], scale: f64) -> image::RgbaImage {
    let (dpi_x, dpi_y) = dpi.unwrap_or((96.0, 96.0));
    let num = |k: &str, d: f64| extract_attr(tile, k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
    let (sx, sy) = (num("sx", 100_000.0).abs() / 100_000.0, num("sy", 100_000.0).abs() / 100_000.0);
    // Bounded: a box is drawn at most 1920 x 1920 pixels.
    let (out_w, out_h) = ((size[0] * scale).round().clamp(1.0, 1920.0) as u32, (size[1] * scale).round().clamp(1.0, 1920.0) as u32);
    // A tile may be bigger than the box (only part of it shows); one bigger than about twice the
    // box's pixels is sampled from the picture as it is rather than resized first, so memory stays
    // in proportion to what is drawn.
    let tile_w = (src.width() as f64 / dpi_x * 914_400.0 * sx * scale).round().clamp(1.0, 100_000.0) as u32;
    let tile_h = (src.height() as f64 / dpi_y * 914_400.0 * sy * scale).round().clamp(1.0, 100_000.0) as u32;
    let resized = ((tile_w as u64) * (tile_h as u64) <= 2 * (out_w as u64) * (out_h as u64)).then(|| image::imageops::resize(src, tile_w, tile_h, image::imageops::FilterType::Triangle));
    let pixel = |u: u32, v: u32| match &resized {
        Some(t) => *t.get_pixel(u, v),
        None => *src.get_pixel((u as u64 * src.width() as u64 / tile_w as u64) as u32, (v as u64 * src.height() as u64 / tile_h as u64) as u32),
    };
    let (tw, th) = (tile_w as f64 / scale, tile_h as f64 / scale);
    let algn = extract_attr(tile, "algn").unwrap_or("tl");
    let [ax0, ay0, aw, ah] = area;
    let ax = if algn.ends_with('r') { aw - tw } else if matches!(algn, "t" | "b" | "ctr") { (aw - tw) / 2.0 } else { 0.0 };
    let ay = if algn.starts_with('b') { ah - th } else if matches!(algn, "l" | "r" | "ctr") { (ah - th) / 2.0 } else { 0.0 };
    let ox = ((ax0 + ax + num("tx", 0.0)) * scale).round() as i64;
    let oy = ((ay0 + ay + num("ty", 0.0)) * scale).round() as i64;
    let flip = extract_attr(tile, "flip").unwrap_or("none");
    let (flip_x, flip_y) = (flip == "x" || flip == "xy", flip == "y" || flip == "xy");
    let (tw, th) = (tile_w as i64, tile_h as i64);
    let mut out = image::RgbaImage::new(out_w, out_h);
    for (x, y, px) in out.enumerate_pixels_mut() {
        let (dx, dy) = (x as i64 - ox, y as i64 - oy);
        let (cx, cy) = (dx.div_euclid(tw), dy.div_euclid(th));
        let (mut u, mut v) = (dx.rem_euclid(tw), dy.rem_euclid(th));
        if flip_x && cx.rem_euclid(2) == 1 {
            u = tw - 1 - u;
        }
        if flip_y && cy.rem_euclid(2) == 1 {
            v = th - 1 - v;
        }
        *px = pixel(u as u32, v as u32);
    }
    out
}

/// A PNG's (`pHYs`) or JPEG's (JFIF) stated resolution in dots per inch, when it states one.
pub(crate) fn image_dpi(bytes: &[u8]) -> Option<(f64, f64)> {
    let be32 = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as f64);
    let be16 = |at: usize| Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as f64);
    let sane = |x: f64, y: f64| (x >= 10.0 && y >= 10.0 && x <= 10_000.0 && y <= 10_000.0).then_some((x, y));
    if bytes.starts_with(b"\x89PNG") {
        // Chunks after the 8-byte signature: length, type, data, CRC. pHYs comes before the image data.
        let mut at = 8;
        while at + 8 <= bytes.len() {
            let len = be32(at)? as usize;
            match bytes.get(at + 4..at + 8)? {
                b"pHYs" if bytes.get(at + 16) == Some(&1) => return sane(be32(at + 8)? * 0.0254, be32(at + 12)? * 0.0254),
                b"IDAT" | b"IEND" => return None,
                _ => at = at.checked_add(12 + len)?,
            }
        }
        return None;
    }
    if bytes.starts_with(&[0xFF, 0xD8]) && bytes.get(6..11) == Some(b"JFIF\0") {
        let (unit, x, y) = (*bytes.get(13)?, be16(14)?, be16(16)?);
        return match unit {
            1 => sane(x, y),
            2 => sane(x * 2.54, y * 2.54),
            _ => None,
        };
    }
    None
}

/// A gradient background as CSS, for the legacy `background` string.
pub(crate) fn gradient_css(kind: &str, stops: &[GradientStop], angle_deg: Option<f64>) -> String {
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

