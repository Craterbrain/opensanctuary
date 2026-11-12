//! Imports an OpenDocument Presentation (`.odp`, ISO/IEC 26300) into a `Presentation`, one
//! positioned element per shape, the way `pptx_import.rs` does for PowerPoint
//! (docs/IMPORT_EXPORT_NOTES.md).
//!
//! ODP keeps every slide in one `content.xml` (`<draw:page>` siblings), its styles there and in
//! `styles.xml`, and the master pages (their backgrounds, decorations and text styles) in
//! `styles.xml`. Everything a slide inherits is resolved:
//!
//! - **Styles** are property bags with parents (`style:parent-style-name`), ending at the
//!   family's default style. A run's look is its span's style, then its paragraph's, then (in an
//!   outline) the master's style for that outline level, then its frame's presentation or graphic
//!   style, then the defaults -- percentages (`fo:font-size="80%"`) apply to what lies beneath.
//! - **Lists**: bullets and numbers from the list style (`text:list`, or the one a presentation
//!   style carries), by nesting level, indented by its spacing; wrapped lines hang after them.
//! - **Geometry**: the page is mapped onto this app's 16:9 canvas (pillarboxed or letterboxed),
//!   point sizes scaled to it (`pptx_import::Geo`); `draw:transform` (rotate, translate, scale)
//!   is applied; custom shapes are evaluated from their enhanced geometry (`odp_geom`), PowerPoint
//!   presets LibreOffice carried over drawn exactly; paths, polygons, polylines, regular
//!   polygons, connectors and measure lines from their own points.
//! - **Paint**: solid, gradient (linear, axial, radial...), bitmap (stretched or tiled) and
//!   hatched fills with their opacity; outlines with arrowheads; pictures (metafiles drawn as
//!   elements, crops from `fo:clip`).
//! - **Pages**: the background (the page's, else its master's), the master's own shapes (unless
//!   the page hides them), its footer and page number when the page shows them, tables with their
//!   cell styles, speaker notes.
//!
//! A file this app exported itself (`meta:generator` "OpenSanctuary") is read back as written:
//! its custom shapes carry this app's own `shape_kind` names and its tables are `Table`s.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use zip::ZipArchive;

use crate::core::models::{
    CropRect, ElementEffects, ElementTransform, GradientStop, Presentation, Slide, SlideBackground, SlideElement, TextBlock, TextParagraphStyle, TextRun,
};
use crate::storage::odp_geom;
use crate::storage::pptx_geom::{self, FillMode, Geometry};
use crate::storage::pptx_import::{self as pptx, Geo};
use crate::storage::pptx_inherit::children;
use crate::storage::xml_scan::{decode_xml_entities, extract_attr, next_tag, next_tag_content};

pub struct OdpImporter;

/// One style: its properties (every `style:*-properties` child's attributes, merged), its parent,
/// and a list style it carries or names.
#[derive(Default, Clone, Debug)]
struct Style {
    parent: Option<String>,
    props: HashMap<String, String>,
    /// `style:list-style-name`, or the name a `<text:list-style>` inside it was stored under.
    list: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Content,
    Master,
}

#[derive(Default)]
struct Styles {
    /// `office:styles` (common styles), keyed "family:name".
    common: HashMap<String, Style>,
    /// `office:automatic-styles` of content.xml (pages) and styles.xml (master pages).
    content_auto: HashMap<String, Style>,
    master_auto: HashMap<String, Style>,
    /// `style:default-style` by family.
    defaults: HashMap<String, Style>,
    /// List styles' contents by name.
    lists: HashMap<String, String>,
    /// Font face name -> family.
    fonts: HashMap<String, String>,
    gradients: HashMap<String, String>,
    fill_images: HashMap<String, String>,
    hatches: HashMap<String, String>,
    /// Page layout name -> (width, height) in EMU.
    page_layouts: HashMap<String, (f64, f64)>,
    /// Master page name -> (page layout, drawing-page style, its content).
    masters: HashMap<String, (String, Option<String>, String)>,
    /// Footer / date-time declarations by name (`presentation:footer-decl`).
    footers: HashMap<String, String>,
    /// Table templates by name: each part ("first-row", "odd-rows", "body"...) -> cell style.
    templates: HashMap<String, HashMap<String, String>>,
}

const EMU_PER_CM: f64 = 360_000.0;

impl Styles {
    fn collect_section(xml: &str, tag: &str, into: &mut HashMap<String, Style>, lists: &mut HashMap<String, String>, defaults: Option<&mut HashMap<String, Style>>) {
        let Some((_, section, _)) = next_tag(xml, tag) else { return };
        let mut defaults = defaults;
        for (name, attrs, inner) in children(section) {
            match name {
                "style:style" | "style:default-style" => {
                    let family = extract_attr(attrs, "style:family").unwrap_or("");
                    let mut st = Style { parent: extract_attr(attrs, "style:parent-style-name").map(str::to_string), list: extract_attr(attrs, "style:list-style-name").map(str::to_string), ..Default::default() };
                    for (pn, pa, pi) in children(inner) {
                        if pn.ends_with("-properties") {
                            for (k, v) in attr_pairs(pa) {
                                st.props.entry(k).or_insert(v);
                            }
                            // A presentation style can carry its own list style.
                            if let Some((_, list_inner, _)) = next_tag(pi, "text:list-style") {
                                let key = format!("@{}:{}", family, extract_attr(attrs, "style:name").unwrap_or(""));
                                lists.insert(key.clone(), list_inner.to_string());
                                st.list.get_or_insert(key);
                            }
                        }
                    }
                    if name == "style:default-style" {
                        if let Some(d) = defaults.as_deref_mut() {
                            d.entry(family.to_string()).or_insert(st);
                        }
                    } else if let Some(n) = extract_attr(attrs, "style:name") {
                        into.entry(format!("{family}:{n}")).or_insert(st);
                    }
                }
                "text:list-style" => {
                    if let Some(n) = extract_attr(attrs, "style:name") {
                        lists.entry(n.to_string()).or_insert_with(|| inner.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    fn parse(content: &str, styles: &str) -> Styles {
        let mut s = Styles::default();
        Self::collect_section(styles, "office:styles", &mut s.common, &mut s.lists, Some(&mut s.defaults));
        Self::collect_section(content, "office:styles", &mut s.common, &mut s.lists, Some(&mut s.defaults));
        Self::collect_section(content, "office:automatic-styles", &mut s.content_auto, &mut s.lists, None);
        Self::collect_section(styles, "office:automatic-styles", &mut s.master_auto, &mut s.lists, None);
        for doc in [content, styles] {
            if let Some((_, decls, _)) = next_tag(doc, "office:font-face-decls") {
                for (_, a, _) in children(decls) {
                    if let (Some(n), Some(f)) = (extract_attr(a, "style:name"), extract_attr(a, "svg:font-family")) {
                        s.fonts.entry(n.to_string()).or_insert_with(|| decode_xml_entities(f).trim_matches(|c| c == '\'' || c == '"').to_string());
                    }
                }
            }
            for st in ["office:styles", "office:automatic-styles"].iter().filter_map(|t| next_tag(doc, t).map(|(_, i, _)| i)) {
                for (n, a, inner) in children(st) {
                    if n == "table:table-template" {
                        // Named by `table:name` (ODF 1.3), or `text:style-name` as LibreOffice once wrote it.
                        if let Some(name) = extract_attr(a, "table:name").or_else(|| extract_attr(a, "text:style-name")) {
                            let parts = children(inner).into_iter().filter_map(|(pn, pa, _)| Some((pn.trim_start_matches("table:").to_string(), extract_attr(pa, "table:style-name")?.to_string()))).collect();
                            s.templates.entry(name.to_string()).or_insert(parts);
                        }
                        continue;
                    }
                    let Some(name) = extract_attr(a, "draw:name") else { continue };
                    match n {
                        "draw:gradient" => { s.gradients.entry(name.to_string()).or_insert_with(|| a.to_string()); }
                        "draw:fill-image" => { if let Some(h) = extract_attr(a, "xlink:href") { s.fill_images.entry(name.to_string()).or_insert_with(|| h.to_string()); } }
                        "draw:hatch" => { if let Some(c) = extract_attr(a, "draw:color") { s.hatches.entry(name.to_string()).or_insert_with(|| c.to_lowercase()); } }
                        _ => {}
                    }
                }
            }
        }
        // Page layouts and master pages sit outside the style sections.
        for (_, a, i) in all_tags(styles, "style:page-layout") {
            let Some(name) = extract_attr(a, "style:name") else { continue };
            if let Some((p, _, _)) = next_tag(i, "style:page-layout-properties") {
                if let (Some(w), Some(h)) = (extract_attr(p, "fo:page-width").and_then(length_emu), extract_attr(p, "fo:page-height").and_then(length_emu)) {
                    s.page_layouts.entry(name.to_string()).or_insert((w, h));
                }
            }
        }
        if let Some((_, ms, _)) = next_tag(styles, "office:master-styles") {
            for (n, a, i) in children(ms) {
                if n == "style:master-page" {
                    if let Some(name) = extract_attr(a, "style:name") {
                        s.masters.entry(name.to_string()).or_insert((extract_attr(a, "style:page-layout-name").unwrap_or("").to_string(), extract_attr(a, "draw:style-name").map(str::to_string), i.to_string()));
                    }
                }
            }
        }
        if let Some((_, pres, _)) = next_tag(content, "office:presentation") {
            for (n, a, i) in children(pres) {
                if n == "presentation:footer-decl" {
                    if let Some(name) = extract_attr(a, "presentation:name") {
                        s.footers.insert(name.to_string(), decode_xml_entities(i));
                    }
                }
            }
        }
        s
    }

    fn get(&self, scope: Scope, family: &str, name: &str) -> Option<&Style> {
        let key = format!("{family}:{name}");
        let auto = if scope == Scope::Master { &self.master_auto } else { &self.content_auto };
        auto.get(&key).or_else(|| self.common.get(&key))
    }

    /// A style and its ancestors, nearest first (bounded, so a loop of parents can't hang).
    fn chain(&self, scope: Scope, family: &str, name: Option<&str>) -> Vec<&Style> {
        let mut out = Vec::new();
        let mut next = name.map(str::to_string);
        while let Some(n) = next {
            if out.len() >= 16 {
                break;
            }
            let Some(st) = self.get(scope, family, &n) else { break };
            out.push(st);
            next = st.parent.clone();
        }
        out
    }
}

/// An element's attributes as owned pairs.
fn attr_pairs(attrs: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = attrs;
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim().to_string();
        let after = &rest[eq + 1..];
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let Some(end) = after[1..].find(q) else { break };
        out.push((key, decode_xml_entities(&after[1..1 + end])));
        rest = &after[end + 2..];
    }
    out
}

/// Every element named `tag` anywhere in `xml`, outermost first.
fn all_tags<'a>(xml: &'a str, tag: &'a str) -> Vec<(&'a str, &'a str, &'a str)> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some((a, i, after)) = next_tag(rest, tag) {
        out.push((tag, a, i));
        rest = after;
    }
    out
}

/// A length (`2.5cm`, `12pt`, `0.1in`...) in EMU.
fn length_emu(s: &str) -> Option<f64> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+' && c != 'e')?;
    let (n, unit) = s.split_at(split);
    let v: f64 = n.parse().ok()?;
    let per_cm = match unit.trim() {
        "cm" => 1.0,
        "mm" => 0.1,
        "in" | "inch" => 2.54,
        "pt" => 2.54 / 72.0,
        "pc" => 2.54 / 6.0,
        "px" => 2.54 / 96.0,
        _ => return None,
    };
    Some(v * per_cm * EMU_PER_CM)
}

fn length_pt(s: &str) -> Option<f64> {
    length_emu(s).map(|e| e / 12_700.0)
}

fn read_zip_text(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut file = archive.by_name(name)?;
    Ok(crate::storage::media_sniff::read_capped_string(&mut file, crate::storage::media_sniff::MAX_ZIP_TEXT_BYTES)?)
}

/// Everything shape extraction needs for one page.
struct Ctx<'a> {
    geo: Geo,
    styles: &'a Styles,
    scope: Scope,
    media: &'a RefCell<HashMap<String, String>>,
    media_dir: &'a Path,
    own_file: bool,
    /// Point sizes onto this app's canvas: the page's scale, or 1 for a file an earlier version of
    /// this app exported (it wrote this app's own sizes unscaled).
    font_scale: f64,
    /// The master's name (presentation styles are named after it: "<master>-outline2").
    master: String,
    page_number: usize,
    footer: Option<String>,
    /// The page's background colour, behind text with "automatic" colour.
    backdrop: Option<String>,
}

impl<'a> Ctx<'a> {
    /// The same page, with `fill` behind its text (a filled shape's), when it has one.
    fn over(&self, fill: Option<&str>) -> Ctx<'a> {
        Ctx { master: self.master.clone(), footer: self.footer.clone(), backdrop: fill.filter(|f| f.starts_with('#')).map(str::to_string).or_else(|| self.backdrop.clone()), ..*self }
    }

    /// A graphic or presentation style chain for a shape's attributes.
    fn shape_chain(&self, attrs: &str) -> Vec<&Style> {
        let mut c = self.styles.chain(self.scope, "presentation", extract_attr(attrs, "presentation:style-name"));
        c.extend(self.styles.chain(self.scope, "graphic", extract_attr(attrs, "draw:style-name")));
        c
    }

    fn defaults(&self, family: &str) -> Option<&Style> {
        self.styles.defaults.get(family)
    }

    fn store(&self, key: String, bytes: &[u8]) -> Option<String> {
        if let Some(done) = self.media.borrow().get(&key) {
            return Some(done.clone());
        }
        let name = crate::storage::media_sniff::write_sniffed_media(self.media_dir, "odp", bytes)?;
        let served = format!("/media/images/{name}");
        self.media.borrow_mut().insert(key, served.clone());
        Some(served)
    }
}

/// The first value of `key` along `chains` (in order), then the given families' defaults.
fn prop<'s>(chains: &[&'s Style], key: &str) -> Option<&'s str> {
    chains.iter().find_map(|s| s.props.get(key).map(String::as_str))
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
        let styles = Styles::parse(&content_xml, &styles_xml);
        let generator = next_tag_content(&meta_xml, "meta:generator").map(|(g, _)| g.trim().to_string()).unwrap_or_default();
        let own_file = generator.starts_with("OpenSanctuary");
        // Before "OpenSanctuary/2" this app wrote its own point sizes unscaled.
        let legacy_sizes = generator == "OpenSanctuary";

        let text_of = |tag: &str| next_tag_content(&meta_xml, tag).map(|(c, _)| decode_xml_entities(c));
        let title = text_of("dc:title").filter(|t| !t.trim().is_empty() && !pptx::is_generic_title(t)).unwrap_or_else(|| file_stem.to_string());
        let author = text_of("meta:initial-creator").or_else(|| text_of("dc:creator")).unwrap_or_default();

        let (_, presentation_inner, _) = next_tag(&content_xml, "office:presentation").ok_or("no <office:presentation> in content.xml (not a presentation document?)")?;
        let media = RefCell::new(HashMap::new());
        let mut averages: HashMap<String, Option<String>> = HashMap::new();
        let mut slides = Vec::new();
        let pages: Vec<_> = children(presentation_inner).into_iter().filter(|(n, _, _)| *n == "draw:page").collect();
        for (idx, (_, page_attrs, page_inner)) in pages.into_iter().enumerate() {
            // Speaker notes are a nested <presentation:notes> block, never slide content.
            let notes_start = page_inner.find("<presentation:notes");
            let body = notes_start.map(|i| &page_inner[..i]).unwrap_or(page_inner);
            let notes_block = notes_start.map(|i| &page_inner[i..]);

            let master_name = extract_attr(page_attrs, "draw:master-page-name").unwrap_or("").to_string();
            let master = styles.masters.get(&master_name);
            // A page layout with no usable size (0cm, say) falls back to the 16:9 default.
            let (page_w, page_h) = master.and_then(|(layout, _, _)| styles.page_layouts.get(layout)).copied()
                .filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 1_000.0 && *h > 1_000.0)
                .unwrap_or((28.0 * EMU_PER_CM, 15.75 * EMU_PER_CM));
            let geo = Geo::new(page_w, page_h);
            let page_style = styles.chain(Scope::Content, "drawing-page", extract_attr(page_attrs, "draw:style-name"));
            let page_flag = |k: &str| prop(&page_style, k);
            let footer = (page_flag("presentation:display-footer") == Some("true"))
                .then(|| extract_attr(page_attrs, "presentation:use-footer-name").and_then(|n| styles.footers.get(n)).cloned())
                .flatten();
            let font_scale = if legacy_sizes { 1.0 } else { geo.font_scale };
            let mut ctx = Ctx { geo, styles: &styles, scope: Scope::Master, media: &media, media_dir, own_file, font_scale, master: master_name.clone(), page_number: idx + 1, footer, backdrop: None };

            // The background: the page's own fill, else its master's.
            let background = if page_flag("presentation:background-visible") == Some("false") {
                None
            } else {
                let own = page_background(&mut archive, &page_style, &ctx);
                own.or_else(|| {
                    let mctx = Ctx { scope: Scope::Master, footer: None, backdrop: None, master: String::new(), ..ctx };
                    let mstyle = styles.chain(Scope::Master, "drawing-page", master.and_then(|(_, s, _)| s.as_deref()));
                    // The master's page style, else its "<master>-background" presentation style.
                    page_background(&mut archive, &mstyle, &mctx).or_else(|| {
                        let bstyle = styles.chain(Scope::Master, "presentation", Some(&format!("{master_name}-background")));
                        page_background(&mut archive, &bstyle, &mctx)
                    })
                })
            };
            ctx.backdrop = match &background {
                Some(SlideBackground::Solid(c)) => Some(c.clone()),
                Some(SlideBackground::Gradient { stops, .. }) => stops.first().map(|s| s.color.clone()),
                // A picture: its average colour, worked out once per picture (slides share masters').
                Some(SlideBackground::Image { file_path, .. }) => averages
                    .entry(file_path.clone())
                    .or_insert_with(|| average_color(&media_dir.join(file_path.trim_start_matches("/media/images/"))))
                    .clone(),
                _ => None,
            };
            let mut elements = Vec::new();
            // The master's own shapes, behind the page's (unless the page hides them), with its
            // footer and page number when the page shows them.
            if let Some((_, _, master_inner)) = master {
                if page_flag("presentation:background-objects-visible") != Some("false") {
                    let show_number = page_flag("presentation:display-page-number") == Some("true");
                    for (name, attrs, inner) in children(master_inner) {
                        let class = extract_attr(attrs, "presentation:class");
                        let keep = match class {
                            None => true,
                            Some("footer") => ctx.footer.is_some(),
                            Some("page-number") => show_number,
                            _ => false,
                        };
                        if keep {
                            elements.extend(shape_elements(&mut archive, name, attrs, inner, &ctx));
                        }
                    }
                }
            }
            ctx.scope = Scope::Content;
            for (name, attrs, inner) in children(body) {
                // An empty placeholder's prompt isn't stored; its frame (fill, outline) still shows.
                elements.extend(shape_elements(&mut archive, name, attrs, inner, &ctx));
            }
            for (z, el) in elements.iter_mut().enumerate() {
                el.transform_mut().z_index = z as i32;
            }

            let tag = format!("S{}", idx + 1);
            let mut slide = Slide { header: Some(tag.clone()), label: Some(format!("Slide {}", idx + 1)), tag: Some(tag), speaker_notes: notes_block.and_then(extract_notes_text), ..Default::default() };
            if !elements.is_empty() {
                slide.elements = elements;
                slide.slide_document_version = 1;
                slide.project_text_from_elements();
            }

            if let Some(bg) = background {
                slide.background = Some(match &bg {
                    SlideBackground::Solid(c) => c.clone(),
                    SlideBackground::Image { file_path, .. } => file_path.clone(),
                    SlideBackground::Gradient { kind, stops, angle_deg } => pptx::gradient_css(kind, stops, *angle_deg),
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

// ---------------------------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------------------------

/// A shape's box in page EMU and its rotation (degrees, clockwise): `svg:x/y/width/height`, or a
/// `draw:transform` (rotate, translate, scale, skewX/Y, matrix -- applied in the order written, as
/// LibreOffice writes them) placing the unrotated `width` x `height` box.
fn shape_box(attrs: &str) -> Option<([f64; 4], f64)> {
    let n = |k: &str| extract_attr(attrs, k).and_then(length_emu);
    let (w, h) = (n("svg:width").unwrap_or(0.0), n("svg:height").unwrap_or(0.0));
    let Some(t) = extract_attr(attrs, "draw:transform") else {
        return Some(([n("svg:x").unwrap_or(0.0), n("svg:y").unwrap_or(0.0), w, h], 0.0));
    };
    // The affine map, as (a, b, c, d, e, f): x' = a x + c y + e, y' = b x + d y + f.
    let mut m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let then = |m: [f64; 6], n: [f64; 6]| [n[0] * m[0] + n[2] * m[1], n[1] * m[0] + n[3] * m[1], n[0] * m[2] + n[2] * m[3], n[1] * m[2] + n[3] * m[3], n[0] * m[4] + n[2] * m[5] + n[4], n[1] * m[4] + n[3] * m[5] + n[5]];
    let mut rest = t;
    // Bounded: each step consumes one "name (args)".
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else { break };
        let args: Vec<&str> = rest[open + 1..open + close].split(|c: char| c.is_whitespace() || c == ',').filter(|a| !a.is_empty()).collect();
        rest = &rest[open + close + 1..];
        let num = |i: usize| args.get(i).and_then(|a| a.parse::<f64>().ok()).unwrap_or(0.0);
        let len = |i: usize| args.get(i).and_then(|a| length_emu(a).or_else(|| a.parse::<f64>().ok().map(|v| v * 360.0))).unwrap_or(0.0);
        let step = match name {
            // ODF's rotate turns counter-clockwise on screen.
            "rotate" => {
                let a = num(0);
                [a.cos(), -a.sin(), a.sin(), a.cos(), 0.0, 0.0]
            }
            "translate" => [1.0, 0.0, 0.0, 1.0, len(0), len(1)],
            "scale" => [num(0), 0.0, 0.0, if args.len() > 1 { num(1) } else { num(0) }, 0.0, 0.0],
            "skewX" => [1.0, 0.0, num(0).tan(), 1.0, 0.0, 0.0],
            "skewY" => [1.0, num(0).tan(), 0.0, 1.0, 0.0, 0.0],
            "matrix" => [num(0), num(1), num(2), num(3), len(4), len(5)],
            _ => continue,
        };
        m = then(m, step);
    }
    let (cx, cy) = (m[0] * w / 2.0 + m[2] * h / 2.0 + m[4], m[1] * w / 2.0 + m[3] * h / 2.0 + m[5]);
    let (sx, sy) = ((m[0] * m[0] + m[1] * m[1]).sqrt(), (m[2] * m[2] + m[3] * m[3]).sqrt());
    let (w, h) = (w * sx, h * sy);
    let rot = m[1].atan2(m[0]).to_degrees();
    Some(([cx - w / 2.0, cy - h / 2.0, w, h], if rot.abs() < 1e-6 { 0.0 } else { rot }))
}

// ---------------------------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------------------------

/// Runs `f` one level of nesting deeper, or gives `R::default()` (nothing) past a depth no real file
/// reaches: groups within groups, lists within lists and spans within spans are each read by recursion,
/// and a crafted file nesting thousands deep would otherwise overflow the stack and abort the server
/// (as `pptx_import`'s `MAX_DEPTH` guards the PowerPoint side). The depth is per thread, so imports on
/// other threads don't count.
fn nested<R: Default>(f: impl FnOnce() -> R) -> R {
    const MAX_NESTING: usize = 32;
    thread_local!(static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });
    struct Level;
    impl Drop for Level {
        fn drop(&mut self) {
            DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        }
    }
    if DEPTH.with(|d| d.get()) >= MAX_NESTING {
        return R::default();
    }
    DEPTH.with(|d| d.set(d.get() + 1));
    let _level = Level;
    f()
}

/// The elements one page-level tag draws.
fn shape_elements(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str, attrs: &str, inner: &str, ctx: &Ctx) -> Vec<SlideElement> {
    nested(|| shape_elements_here(archive, name, attrs, inner, ctx))
}

fn shape_elements_here(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str, attrs: &str, inner: &str, ctx: &Ctx) -> Vec<SlideElement> {
    let mut out = Vec::new();
    // A shape kept out of view ("none") or only printed isn't on screen.
    if ["draw:display", "drawooo:display"].iter().any(|k| matches!(extract_attr(attrs, k), Some("none" | "printer"))) {
        return out;
    }
    match name {
        "draw:frame" => out.extend(frame_elements(archive, attrs, inner, ctx)),
        "draw:g" => {
            let kids: Vec<SlideElement> = children(inner).into_iter().flat_map(|(n, a, i)| shape_elements(archive, n, a, i, ctx)).collect();
            if !kids.is_empty() {
                out.push(group_of(kids));
            }
        }
        // A shape inside a hyperlink: the shape (links have no place in this model).
        "draw:a" => {
            for (n, a, i) in children(inner) {
                out.extend(shape_elements(archive, n, a, i, ctx));
            }
        }
        "draw:line" | "draw:connector" | "draw:measure" => out.extend(line_element(name, attrs, ctx)),
        "draw:rect" | "draw:ellipse" | "draw:circle" | "draw:custom-shape" | "draw:polygon" | "draw:polyline" | "draw:path" | "draw:regular-polygon" | "draw:caption" => {
            out.extend(drawn_shape(archive, name, attrs, inner, ctx));
        }
        _ => {}
    }
    out
}

fn group_of(children: Vec<SlideElement>) -> SlideElement {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for c in &children {
        let t = c.transform();
        (x0, y0, x1, y1) = (x0.min(t.x), y0.min(t.y), x1.max(t.x + t.w), y1.max(t.y + t.h));
    }
    let transform = ElementTransform { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0), ..Default::default() };
    SlideElement::Group { id: new_id(), transform, children }
}

use pptx::new_id;

/// A colour attribute (`#rrggbb`) with an opacity percentage applied.
fn with_opacity(color: &str, opacity: Option<&str>) -> String {
    let a = opacity.and_then(|o| o.trim_end_matches('%').parse::<f64>().ok()).map_or(1.0, |p| (p / 100.0).clamp(0.0, 1.0));
    crate::storage::pptx_inherit::with_alpha(&color.to_lowercase(), a)
}

/// A colour with an opacity percentage, composed over white.
fn over_white(color: &str, opacity: Option<&str>) -> String {
    let a = opacity.and_then(|o| o.trim_end_matches('%').parse::<f64>().ok()).map_or(1.0, |p| (p / 100.0).clamp(0.0, 1.0));
    let Some(c) = color.trim_start_matches('#').get(..6).filter(|c| c.is_ascii() && a < 1.0) else {
        return color.to_lowercase();
    };
    let v = |i: usize| u8::from_str_radix(&c[i..i + 2], 16).map(f64::from).unwrap_or(255.0);
    let m = |x: f64| (x * a + 255.0 * (1.0 - a)).round() as u8;
    format!("#{:02x}{:02x}{:02x}", m(v(0)), m(v(2)), m(v(4)))
}

/// A shape's fill along its style chain: a colour (`#rrggbbaa` when see-through), a gradient (CSS),
/// a picture (`Err(name)`, the fill image), or none.
fn shape_fill(chain: &[&Style], ctx: &Ctx) -> Option<Result<String, String>> {
    let fill = prop(chain, "draw:fill").unwrap_or("solid");
    let opacity = prop(chain, "draw:opacity");
    // Filled with the slide's own background.
    if prop(chain, "loext:fill-use-slide-background") == Some("true") {
        return ctx.backdrop.clone().map(Ok);
    }
    match fill {
        "none" => None,
        "gradient" => {
            let g = prop(chain, "draw:fill-gradient-name").and_then(|n| ctx.styles.gradients.get(n))?;
            match gradient_of(g, opacity)? {
                SlideBackground::Gradient { kind, stops, angle_deg } => Some(Ok(pptx::gradient_css(&kind, &stops, angle_deg))),
                SlideBackground::Solid(c) => Some(Ok(c)),
                _ => None,
            }
        }
        "bitmap" => prop(chain, "draw:fill-image-name").map(|n| Err(n.to_string())),
        "hatch" => {
            // Lines over the background: a see-through tint of the hatch colour, or the
            // background colour under it when it states one.
            if prop(chain, "draw:fill-hatch-solid") == Some("true") {
                prop(chain, "draw:fill-color").map(|c| Ok(with_opacity(c, opacity)))
            } else {
                prop(chain, "draw:fill-hatch-name").and_then(|n| ctx.styles.hatches.get(n)).map(|c| Ok(with_opacity(c, Some("35%"))))
            }
        }
        _ => Some(Ok(with_opacity(prop(chain, "draw:fill-color").or_else(|| ctx.defaults("graphic").and_then(|d| d.props.get("draw:fill-color").map(String::as_str))).unwrap_or("#729fcf"), opacity))),
    }
}

/// A `<draw:gradient>`'s attributes as a gradient: ODF's styles (linear, axial, radial,
/// ellipsoid, square, rectangular), angle (tenths of a degree, or with a unit; counter-clockwise,
/// 0 = top to bottom) and border (the share held at the start colour).
fn gradient_of(attrs: &str, opacity: Option<&str>) -> Option<SlideBackground> {
    let color = |k: &str, ik: &str| {
        let c = extract_attr(attrs, k)?;
        let intensity = extract_attr(attrs, ik).and_then(|v| v.trim_end_matches('%').parse::<f64>().ok()).unwrap_or(100.0) / 100.0;
        let (r, g, b) = (u8::from_str_radix(c.get(1..3)?, 16).ok()?, u8::from_str_radix(c.get(3..5)?, 16).ok()?, u8::from_str_radix(c.get(5..7)?, 16).ok()?);
        let k = |v: u8| (v as f64 * intensity).round() as u8;
        Some(with_opacity(&format!("#{:02x}{:02x}{:02x}", k(r), k(g), k(b)), opacity))
    };
    let (start, end) = (color("draw:start-color", "draw:start-intensity")?, color("draw:end-color", "draw:end-intensity")?);
    let border = extract_attr(attrs, "draw:border").and_then(|v| v.trim_end_matches('%').parse::<f64>().ok()).unwrap_or(0.0) / 100.0;
    let angle = extract_attr(attrs, "draw:angle").map(|a| {
        if let Some(d) = a.strip_suffix("deg") { d.parse::<f64>().unwrap_or(0.0) }
        else if let Some(r) = a.strip_suffix("rad") { r.parse::<f64>().unwrap_or(0.0).to_degrees() }
        else if let Some(g) = a.strip_suffix("grad") { g.parse::<f64>().unwrap_or(0.0) * 0.9 }
        else { a.parse::<f64>().unwrap_or(0.0) / 10.0 }
    }).unwrap_or(0.0);
    let stop = |color: &String, offset: f64| GradientStop { color: color.clone(), offset };
    let style = extract_attr(attrs, "draw:style").unwrap_or("linear");
    match style {
        "linear" => Some(SlideBackground::Gradient { kind: "linear".into(), stops: vec![stop(&start, 0.0), stop(&start, border), stop(&end, 1.0)], angle_deg: Some((180.0 - angle).rem_euclid(360.0)) }),
        // Axial: the end colour in the middle, the start colour at both edges.
        "axial" => Some(SlideBackground::Gradient { kind: "linear".into(), stops: vec![stop(&start, 0.0), stop(&start, border / 2.0), stop(&end, 0.5), stop(&start, 1.0 - border / 2.0), stop(&start, 1.0)], angle_deg: Some((180.0 - angle).rem_euclid(360.0)) }),
        // Radial and the rest: the end colour at the centre, the start colour outside.
        _ => Some(SlideBackground::Gradient { kind: "radial".into(), stops: vec![stop(&end, 0.0), stop(&start, 1.0 - border), stop(&start, 1.0)], angle_deg: None }),
    }
}

/// A shape's outline along its style chain: colour (with opacity) and width in EMU, or none.
fn shape_stroke(chain: &[&Style]) -> Option<(String, f64)> {
    if prop(chain, "draw:stroke") == Some("none") {
        return None;
    }
    let color = prop(chain, "svg:stroke-color").unwrap_or("#000000");
    // Width 0 is a hairline: one device pixel, about 0.75pt.
    let w = prop(chain, "svg:stroke-width").and_then(length_emu).filter(|w| *w > 0.0).unwrap_or(9_525.0);
    Some((with_opacity(color, prop(chain, "svg:stroke-opacity")), w))
}

/// A line's width in canvas units (1080 to the canvas height), as path elements use.
fn canvas_stroke(width_emu: f64, geo: &Geo) -> f64 {
    pptx::geo_stroke(width_emu, geo)
}

/// The elements an evaluated geometry draws in `rect` (turned `rot`): a `Shape` per filled look,
/// a `Line` per outline-only part, grouped when there's more than one.
fn geometry_elements(geometry: &Geometry, fill: Option<&str>, stroke: Option<&(String, f64)>, arrows: (bool, bool), rect: [f64; 4], rot: f64, geo: &Geo) -> Vec<SlideElement> {
    let transform = geo.transform(rect, rot);
    let mut parts: Vec<(Option<String>, Option<String>, String)> = Vec::new();
    for p in &geometry.paths {
        let f = (p.fill != FillMode::None).then(|| fill.map(|f| pptx_geom::shade(f, p.fill))).flatten();
        let s = if p.stroke { stroke.map(|(c, _)| c.clone()) } else { None };
        if f.is_none() && s.is_none() {
            continue;
        }
        match parts.last_mut() {
            Some((pf, ps, d)) if *pf == f && *ps == s => {
                d.push(' ');
                d.push_str(&p.d);
            }
            _ => parts.push((f, s, p.d.clone())),
        }
    }
    let width = canvas_stroke(stroke.map_or(0.0, |(_, w)| *w), geo);
    let mut els: Vec<SlideElement> = parts.into_iter().map(|(f, s, d)| match f {
        Some(fill_color) => SlideElement::Shape { id: new_id(), transform: transform.clone(), shape_kind: "path".into(), fill_color, stroke_width: if s.is_some() { width } else { 0.0 }, stroke_color: s, path: Some(d) },
        None => SlideElement::Line { id: new_id(), transform: transform.clone(), line_kind: "curved".into(), color: s.unwrap_or_default(), stroke_width: width, start_arrow: arrows.0, end_arrow: arrows.1, path: Some(d) },
    }).collect();
    match els.len() {
        0 => Vec::new(),
        1 => els,
        _ => vec![SlideElement::Group { id: new_id(), transform: ElementTransform { rotation_deg: 0.0, ..transform }, children: std::mem::take(&mut els) }],
    }
}

/// A rectangle (with rounded corners of radius `r`, EMU) as geometry.
fn rect_geometry(w: f64, h: f64, r: f64) -> Geometry {
    if r <= 0.0 {
        return pptx_geom::preset("rect", "", w, h, (false, false)).unwrap_or_default();
    }
    let adj = (r / w.min(h).max(1.0) * 100_000.0).clamp(0.0, 50_000.0);
    pptx_geom::preset("roundRect", &format!(r#"<a:gd name="adj" fmla="val {adj}"/>"#), w, h, (false, false)).unwrap_or_default()
}

/// A drawn shape: its geometry painted with its style, a picture fill clipped to it, its text.
fn drawn_shape(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str, attrs: &str, inner: &str, ctx: &Ctx) -> Vec<SlideElement> {
    let Some(([x, y, w, h], rot)) = shape_box(attrs) else { return Vec::new() };
    let chain = ctx.shape_chain(attrs);
    let mut chain_d = chain.clone();
    chain_d.extend(ctx.defaults("graphic"));
    let mut out = Vec::new();

    // A file this app wrote: its own shape names, read back exactly.
    if ctx.own_file && name == "draw:custom-shape" {
        let kind = next_tag(inner, "draw:enhanced-geometry").and_then(|(a, _, _)| extract_attr(a, "draw:type")).map(map_custom_shape_type).unwrap_or_else(|| "rect".to_string());
        out.push(own_shape(&kind, attrs, &chain_d, ctx));
        out.extend(shape_text(inner, attrs, None, [x, y, w, h], rot, ctx));
        return out;
    }
    if ctx.own_file && matches!(name, "draw:polyline" | "draw:path") {
        let points = extract_attr(attrs, "draw:points").map_or(2, |p| p.split_whitespace().count());
        let kind = if name == "draw:path" { "curved" } else if points > 2 { "elbow" } else { "straight" };
        out.extend(own_line(kind, [x, y, w, h], &chain_d, ctx));
        return out;
    }
    if ctx.own_file && matches!(name, "draw:rect" | "draw:ellipse" | "draw:regular-polygon" | "draw:caption" | "draw:polygon") {
        let kind = match name {
            "draw:rect" => "rect",
            "draw:ellipse" => "ellipse",
            "draw:regular-polygon" => regular_polygon_kind(attrs),
            "draw:caption" => "callout",
            _ => "rect",
        };
        out.push(own_shape(kind, attrs, &chain_d, ctx));
        return out;
    }

    let view = || -> [f64; 4] {
        let v: Vec<f64> = extract_attr(attrs, "svg:viewBox").map(|v| v.split_whitespace().filter_map(|n| n.parse().ok()).collect()).unwrap_or_default();
        if v.len() == 4 && v[2] > 0.0 && v[3] > 0.0 { [v[0], v[1], v[2], v[3]] } else { [0.0, 0.0, (w / 360.0).max(1.0), (h / 360.0).max(1.0)] }
    };
    let geometry: Option<Geometry> = match name {
        "draw:rect" => Some(rect_geometry(w, h, extract_attr(attrs, "draw:corner-radius").and_then(length_emu).unwrap_or(0.0))),
        "draw:ellipse" | "draw:circle" => Some(ellipse_geometry(attrs, w, h)),
        "draw:custom-shape" => next_tag(inner, "draw:enhanced-geometry").and_then(|(ga, gi, _)| {
            // A preset named without its path (LibreOffice always writes one; others may not):
            // the matching DrawingML preset, drawn exactly.
            odp_geom::enhanced(ga, gi, w, h).or_else(|| extract_attr(ga, "draw:type").and_then(odf_preset).and_then(|p| pptx_geom::preset(p, "", w, h, (false, false))))
        }).or_else(|| Some(rect_geometry(w, h, 0.0))),
        "draw:polygon" | "draw:polyline" => extract_attr(attrs, "draw:points").and_then(|p| odp_geom::points_path(p, view(), name == "draw:polygon")).map(|(d, points)| path_geometry(d, points, name == "draw:polygon")),
        "draw:path" => extract_attr(attrs, "svg:d").and_then(|d| odp_geom::svg_path(&decode_xml_entities(d), view())).map(|(d, points, closed)| path_geometry(d, points, closed)),
        "draw:regular-polygon" => Some(regular_polygon_geometry(attrs)),
        "draw:caption" => Some(rect_geometry(w, h, extract_attr(attrs, "draw:corner-radius").and_then(length_emu).unwrap_or(0.0))),
        _ => None,
    };
    let Some(geometry) = geometry else { return out };

    let stroke = shape_stroke(&chain_d);
    let arrows = (prop(&chain_d, "draw:marker-start").is_some(), prop(&chain_d, "draw:marker-end").is_some());
    // A plain rectangle or ellipse stays this app's own shape (as PPTX's import keeps them).
    let plain = (name == "draw:rect" && extract_attr(attrs, "draw:corner-radius").and_then(length_emu).unwrap_or(0.0) <= 0.0)
        || (matches!(name, "draw:ellipse" | "draw:circle") && extract_attr(attrs, "draw:kind").is_none_or(|k| k == "full"));
    if plain {
        if let Some(fill) = match shape_fill(&chain_d, ctx) { Some(Ok(c)) => Some(Some(c)), None => Some(None), Some(Err(_)) => None } {
            let stroke_width = stroke.as_ref().map_or(0.0, |(_, sw)| pptx::emu_to_relative_stroke(*sw, w, h));
            let kind = if name == "draw:rect" { "rect" } else { "ellipse" };
            let tctx = ctx.over(fill.as_deref());
            out.push(SlideElement::Shape { id: new_id(), transform: ctx.geo.transform([x, y, w, h], rot), shape_kind: kind.into(), fill_color: fill.unwrap_or_else(|| "none".into()), stroke_color: stroke.map(|(c, _)| c), stroke_width, path: None });
            out.extend(shape_text(inner, attrs, None, [x, y, w, h], rot, &tctx));
            return out;
        }
    }
    let fill = if matches!(name, "draw:polyline") || (name == "draw:path" && geometry.paths.iter().all(|p| p.fill == FillMode::None)) { None } else { shape_fill(&chain_d, ctx) };
    match &fill {
        Some(Err(image)) => {
            out.extend(picture_fill(archive, image, &chain_d, &geometry, [x, y, w, h], rot, ctx));
            out.extend(geometry_elements(&geometry, None, stroke.as_ref(), arrows, [x, y, w, h], rot, &ctx.geo));
        }
        f => out.extend(geometry_elements(&geometry, f.as_ref().and_then(|f| f.as_ref().ok()).map(String::as_str), stroke.as_ref(), arrows, [x, y, w, h], rot, &ctx.geo)),
    }
    let backdrop = match &fill { Some(Ok(c)) => Some(c.as_str()), _ => None };
    out.extend(shape_text(inner, attrs, geometry.text_rect, [x, y, w, h], rot, &ctx.over(backdrop)));
    out
}

/// ODF's preset shape names (LibreOffice's gallery) with a DrawingML preset of the same shape.
fn odf_preset(t: &str) -> Option<&'static str> {
    Some(match t {
        "rectangle" | "square" | "flowchart-process" => "rect",
        "round-rectangle" | "round-square" | "flowchart-alternate-process" => "roundRect",
        "ellipse" | "circle" | "flowchart-connector" => "ellipse",
        "diamond" | "flowchart-decision" => "diamond",
        "isosceles-triangle" => "triangle",
        "right-triangle" => "rtTriangle",
        "parallelogram" | "flowchart-data" => "parallelogram",
        "trapezoid" => "trapezoid",
        "hexagon" | "flowchart-preparation" => "hexagon",
        "octagon" => "octagon",
        "pentagon" => "pentagon",
        "cross" => "plus",
        "ring" => "donut",
        "heart" => "heart",
        "lightning" => "lightningBolt",
        "sun" => "sun",
        "moon" => "moon",
        "smiley" => "smileyFace",
        "cloud" => "cloud",
        "can" => "can",
        "cube" => "cube",
        "star4" => "star4",
        "star5" => "star5",
        "star6" => "star6",
        "star8" => "star8",
        "star12" => "star12",
        "star24" => "star24",
        "right-arrow" => "rightArrow",
        "left-arrow" => "leftArrow",
        "up-arrow" => "upArrow",
        "down-arrow" => "downArrow",
        "left-right-arrow" => "leftRightArrow",
        "up-down-arrow" => "upDownArrow",
        "quad-arrow" => "quadArrow",
        "chevron" => "chevron",
        "pentagon-right" => "homePlate",
        "notched-right-arrow" => "notchedRightArrow",
        "striped-right-arrow" => "stripedRightArrow",
        "rectangular-callout" => "wedgeRectCallout",
        "round-rectangular-callout" => "wedgeRoundRectCallout",
        "round-callout" | "oval-callout" => "wedgeEllipseCallout",
        "cloud-callout" => "cloudCallout",
        "flowchart-merge" => "flowChartMerge",
        "flowchart-extract" => "flowChartExtract",
        "flowchart-manual-operation" => "flowChartManualOperation",
        _ => return None,
    })
}

fn path_geometry(d: String, points: Vec<Vec<(f64, f64)>>, closed: bool) -> Geometry {
    Geometry { paths: vec![pptx_geom::GeomPath { d, points, fill: if closed { FillMode::Norm } else { FillMode::None }, stroke: true }], text_rect: None }
}

/// An ellipse, or (`draw:kind`) the arc, section (pie) or cut (chord) of one between
/// `draw:start-angle` and `draw:end-angle` (degrees, counter-clockwise from 3 o'clock).
fn ellipse_geometry(attrs: &str, w: f64, h: f64) -> Geometry {
    let kind = extract_attr(attrs, "draw:kind").unwrap_or("full");
    if kind == "full" {
        return pptx_geom::preset("ellipse", "", w, h, (false, false)).unwrap_or_default();
    }
    let ang = |k: &str| extract_attr(attrs, k).map(|v| v.trim_end_matches("deg").parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0);
    let (a0, a1) = (ang("draw:start-angle"), ang("draw:end-angle"));
    let sweep = (a1 - a0).rem_euclid(360.0);
    let sweep = if sweep == 0.0 { 360.0 } else { sweep };
    // Counter-clockwise in ODF's y-up sense is decreasing angle on screen: sample the arc.
    let pt = |deg: f64| (50.0 + 50.0 * deg.to_radians().cos(), 50.0 - 50.0 * deg.to_radians().sin());
    let n = (sweep / 5.0).ceil().max(2.0) as usize;
    let pts: Vec<(f64, f64)> = (0..=n).map(|i| pt(a0 + sweep * i as f64 / n as f64)).collect();
    let mut d = format!("M {:.2} {:.2}", pts[0].0, pts[0].1);
    for p in &pts[1..] {
        d.push_str(&format!(" L {:.2} {:.2}", p.0, p.1));
    }
    let mut points = pts.clone();
    if kind == "section" {
        d.push_str(" L 50 50");
        points.push((50.0, 50.0));
    }
    if kind != "arc" {
        d.push_str(" Z");
    }
    Geometry { paths: vec![pptx_geom::GeomPath { d, points: vec![points], fill: if kind == "arc" { FillMode::None } else { FillMode::Norm }, stroke: true }], text_rect: None }
}

/// A regular polygon (`draw:corners`), point up; `draw:concave` makes a star whose inner corners
/// sit at `draw:sharpness` of the way in.
fn regular_polygon_geometry(attrs: &str) -> Geometry {
    let n = extract_attr(attrs, "draw:corners").and_then(|v| v.parse::<usize>().ok()).unwrap_or(3).clamp(3, 100);
    let concave = extract_attr(attrs, "draw:concave") == Some("true");
    let sharp = extract_attr(attrs, "draw:sharpness").and_then(|v| v.trim_end_matches('%').parse::<f64>().ok()).unwrap_or(0.0) / 100.0;
    let steps = if concave { 2 * n } else { n };
    let pts: Vec<(f64, f64)> = (0..steps).map(|i| {
        let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * i as f64 / steps as f64;
        let r = if concave && i % 2 == 1 { 50.0 * (1.0 - sharp) } else { 50.0 };
        (50.0 + r * a.cos(), 50.0 + r * a.sin())
    }).collect();
    let mut d = String::new();
    for (i, p) in pts.iter().enumerate() {
        d.push_str(&format!("{}{:.2} {:.2} ", if i == 0 { "M " } else { "L " }, p.0, p.1));
    }
    d.push('Z');
    Geometry { paths: vec![pptx_geom::GeomPath { d, points: vec![pts], fill: FillMode::Norm, stroke: true }], text_rect: None }
}

/// A shape from a file this app exported: its own `shape_kind`, colours as written.
fn own_shape(kind: &str, attrs: &str, chain: &[&Style], ctx: &Ctx) -> SlideElement {
    let ([x, y, w, h], rot) = shape_box(attrs).unwrap_or(([0.0; 4], 0.0));
    let fill = if prop(chain, "draw:fill") == Some("none") { None } else { prop(chain, "draw:fill-color").map(str::to_lowercase) };
    let stroke = shape_stroke(chain).filter(|_| prop(chain, "svg:stroke-color").is_some() || prop(chain, "svg:stroke-width").is_some());
    let rel = |emu: f64| pptx::emu_to_relative_stroke(emu, w, h);
    // A ring is written as an unfilled, thick-stroked outline in its fill colour (odp_export.rs).
    let (fill_color, stroke_color, stroke_width) = if kind == "ring" {
        (stroke.map(|(c, _)| c).unwrap_or_else(|| "none".into()), None, 0.0)
    } else {
        (fill.unwrap_or_else(|| "none".into()), stroke.as_ref().map(|(c, _)| c.clone()), stroke.as_ref().map_or(0.0, |(_, w)| rel(*w)))
    };
    SlideElement::Shape { id: new_id(), transform: ctx.geo.transform([x, y, w, h], rot), shape_kind: kind.to_string(), fill_color, stroke_color, stroke_width, path: None }
}

fn regular_polygon_kind(attrs: &str) -> &'static str {
    if extract_attr(attrs, "draw:concave") == Some("true") {
        return "star";
    }
    match extract_attr(attrs, "draw:corners").and_then(|s| s.parse::<u32>().ok()) {
        Some(3) => "triangle",
        Some(4) => "diamond",
        Some(5) => "pentagon",
        Some(6) => "hexagon",
        Some(8) => "octagon",
        _ => "rect",
    }
}

/// This app's own `shape_kind` for the ODF preset names its exporter writes (and the few ODF
/// names that mean the same shape), for reading back files this app wrote.
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
        "parallelogram" | "flowchart-data" => "parallelogram".to_string(),
        "hexagon" | "flowchart-preparation" => "hexagon".to_string(),
        "flowchart-merge" => "triangle-down".to_string(),
        other => other.to_string(),
    }
}

/// A line from a file this app wrote: its own `line_kind`, the width relative to its box as written.
fn own_line(kind: &str, rect: [f64; 4], chain: &[&Style], ctx: &Ctx) -> Option<SlideElement> {
    let color = prop(chain, "svg:stroke-color").unwrap_or("#000000").to_lowercase();
    let stroke_width = prop(chain, "svg:stroke-width").and_then(length_emu).map_or(3.0, |w| pptx::emu_to_relative_stroke(w, rect[2], rect[3]));
    Some(SlideElement::Line { id: new_id(), transform: ctx.geo.transform(rect, 0.0), line_kind: kind.into(), color, stroke_width, start_arrow: prop(chain, "draw:marker-start").is_some(), end_arrow: prop(chain, "draw:marker-end").is_some(), path: None })
}

/// A line, connector or measure line: from `svg:x1,y1` to `svg:x2,y2`, or (a connector LibreOffice
/// routed) along its `svg:d` in page coordinates (1/100 mm).
fn line_element(name: &str, attrs: &str, ctx: &Ctx) -> Option<SlideElement> {
    let mut chain = ctx.shape_chain(attrs);
    chain.extend(ctx.defaults("graphic"));
    let (color, width) = shape_stroke(&chain)?;
    let n = |k: &str| extract_attr(attrs, k).and_then(length_emu).unwrap_or(0.0);
    let (x1, y1, x2, y2) = (n("svg:x1"), n("svg:y1"), n("svg:x2"), n("svg:y2"));
    let arrows = (prop(&chain, "draw:marker-start").is_some(), prop(&chain, "draw:marker-end").is_some());
    let width = canvas_stroke(width, &ctx.geo);
    if name == "draw:connector" {
        if let Some(d) = extract_attr(attrs, "svg:d") {
            // In 1/100 mm on the page; placed in the box it spans -- the box of its points once
            // resolved (relative and h/v commands too), which a first pass at scale 1 gives.
            let d = decode_xml_entities(d);
            let (mut bx0, mut by0, mut bx1, mut by1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for &(x, y) in odp_geom::svg_path(&d, [0.0, 0.0, 100.0, 100.0]).map(|(_, pts, _)| pts).unwrap_or_default().iter().flatten() {
                (bx0, by0, bx1, by1) = (bx0.min(x), by0.min(y), bx1.max(x), by1.max(y));
            }
            if bx0 < bx1 || by0 < by1 {
                let view = [bx0, by0, (bx1 - bx0).max(1.0), (by1 - by0).max(1.0)];
                if let Some((path, _, _)) = odp_geom::svg_path(&d, view) {
                    let rect = [bx0 * 360.0, by0 * 360.0, view[2] * 360.0, view[3] * 360.0];
                    return Some(SlideElement::Line { id: new_id(), transform: ctx.geo.transform(rect, 0.0), line_kind: "curved".into(), color, stroke_width: width, start_arrow: arrows.0, end_arrow: arrows.1, path: Some(path) });
                }
            }
        }
    }
    let rect = [x1.min(x2), y1.min(y2), (x1 - x2).abs().max(1.0), (y1 - y2).abs().max(1.0)];
    if ctx.own_file {
        // A flat line still gets a box this app can select.
        let floor = |v: f64, page: f64| v.max(page * 0.01);
        let rect = [rect[0], rect[1], floor(rect[2], ctx.geo.canvas_w - 2.0 * ctx.geo.x_off), floor(rect[3], ctx.geo.canvas_h - 2.0 * ctx.geo.y_off)];
        return own_line("straight", rect, &chain, ctx);
    }
    let p = |x: f64, y: f64| (if rect[2] > 1.0 { (x - rect[0]) / rect[2] * 100.0 } else { 0.0 }, if rect[3] > 1.0 { (y - rect[1]) / rect[3] * 100.0 } else { 0.0 });
    let (a, b) = (p(x1, y1), p(x2, y2));
    let path = format!("M {:.2} {:.2} L {:.2} {:.2}", a.0, a.1, b.0, b.1);
    Some(SlideElement::Line { id: new_id(), transform: ctx.geo.transform(rect, 0.0), line_kind: "curved".into(), color, stroke_width: width, start_arrow: arrows.0, end_arrow: arrows.1, path: Some(path) })
}

// ---------------------------------------------------------------------------------------------
// Frames: text boxes, pictures, tables, embedded objects
// ---------------------------------------------------------------------------------------------

fn frame_elements(archive: &mut ZipArchive<Cursor<&[u8]>>, attrs: &str, inner: &str, ctx: &Ctx) -> Vec<SlideElement> {
    let Some(([x, y, w, h], rot)) = shape_box(attrs) else { return Vec::new() };
    let rect = [x, y, w, h];
    let mut out = Vec::new();
    let kids = children(inner);
    // A frame's own fill and outline (a filled text box, a framed picture); a table has none, its
    // cells drawing their own.
    let mut chain = ctx.shape_chain(attrs);
    chain.extend(ctx.defaults("graphic"));
    let table = kids.iter().any(|(n, _, _)| *n == "table:table");
    let fill = match prop(&chain, "draw:fill") {
        _ if table => None,
        Some("none") | None => None,
        _ => shape_fill(&chain, ctx).and_then(Result::ok),
    };
    let stroke = (!table && prop(&chain, "draw:stroke").is_some_and(|s| s != "none")).then(|| shape_stroke(&chain)).flatten();
    if fill.is_some() || stroke.is_some() {
        out.extend(geometry_elements(&rect_geometry(w, h, 0.0), fill.as_deref(), stroke.as_ref(), (false, false), rect, rot, &ctx.geo));
    }
    if let Some((_, _, ti)) = kids.iter().find(|(n, _, _)| *n == "draw:text-box") {
        out.extend(text_block(ti, attrs, rect, rot, &ctx.over(fill.as_deref()), false));
    } else if let Some((_, ta, ti)) = kids.iter().find(|(n, _, _)| *n == "table:table") {
        out.extend(table_element(archive, ti, ta, rect, ctx));
    } else if let Some(chart) = kids.iter().find(|(n, _, _)| *n == "draw:object").and_then(|(_, a, _)| extract_attr(a, "xlink:href"))
        .and_then(|h| read_zip_text(archive, &format!("{}/content.xml", h.trim_start_matches("./").trim_end_matches('/'))).ok())
        .and_then(|doc| crate::storage::odp_chart::to_drawingml(&doc))
        .and_then(|xml| pptx::chart_elements(&xml, rect, ctx.geo, ctx.media, ctx.media_dir))
    {
        // An embedded chart, drawn from its own data.
        out.push(chart);
    } else {
        // A picture: of several (an SVG with a PNG beside it, an object's replacement image), the
        // first this app can show.
        let images: Vec<&str> = kids.iter().filter(|(n, _, _)| *n == "draw:image").filter_map(|(_, a, _)| extract_attr(a, "xlink:href")).collect();
        out.extend(picture(archive, &images, &chain, rect, rot, ctx));
    }
    out
}

/// A picture frame: the first of `hrefs` that can be shown (a raster picture or SVG as an `Image`,
/// a WMF/EMF drawn as elements), cropped as the frame's style clips it.
fn picture(archive: &mut ZipArchive<Cursor<&[u8]>>, hrefs: &[&str], chain: &[&Style], rect: [f64; 4], rot: f64, ctx: &Ctx) -> Option<SlideElement> {
    for href in hrefs {
        let path = href.trim_start_matches("./");
        let Ok(bytes) = pptx::read_picture_bytes(archive, path) else { continue };
        if crate::storage::wmf::is_wmf(&bytes) || crate::storage::emf::is_emf(&bytes) {
            if let Some(el) = pptx::metafile_elements(&bytes, rect, rot, ctx.geo, ctx.media, ctx.media_dir) {
                return Some(el);
            }
            continue;
        }
        let Some(file_path) = ctx.store(path.to_string(), &bytes) else { continue };
        return Some(SlideElement::Image { id: new_id(), transform: ctx.geo.transform(rect, rot), file_path, crop: clip_of(chain, &bytes), mask_shape: None, alt_text: None, fit: Some("stretch".into()) });
    }
    None
}

/// `fo:clip="rect(top, right, bottom, left)"` (lengths of the picture's own size) as a crop.
fn clip_of(chain: &[&Style], bytes: &[u8]) -> Option<CropRect> {
    let clip = prop(chain, "fo:clip")?;
    let inner = clip.trim().strip_prefix("rect(")?.trim_end_matches(')');
    let v: Vec<f64> = inner.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(|s| length_emu(s).unwrap_or(0.0)).collect();
    if v.len() != 4 || v.iter().all(|x| *x == 0.0) {
        return None;
    }
    let img = image::load_from_memory(bytes).ok()?;
    let (dx, dy) = pptx::image_dpi(bytes).unwrap_or((96.0, 96.0));
    let (iw, ih) = (img.width() as f64 / dx * 914_400.0, img.height() as f64 / dy * 914_400.0);
    let (t, r, b, l) = (v[0] / ih, v[1] / iw, v[2] / ih, v[3] / iw);
    Some(CropRect { x: l, y: t, w: (1.0 - l - r).clamp(0.01, 1.0), h: (1.0 - t - b).clamp(0.01, 1.0) })
}

/// A bitmap fill: the picture over the shape's box (stretched, or tiled as `style:repeat` says),
/// clipped to its outline.
fn picture_fill(archive: &mut ZipArchive<Cursor<&[u8]>>, image: &str, chain: &[&Style], geometry: &Geometry, rect: [f64; 4], rot: f64, ctx: &Ctx) -> Option<SlideElement> {
    let href = ctx.styles.fill_images.get(image)?.trim_start_matches("./").to_string();
    let (file_path, _) = fill_picture(archive, &href, chain, [rect[2], rect[3]], [0.0, 0.0, rect[2], rect[3]], false, ctx)?;
    Some(SlideElement::Image { id: new_id(), transform: ctx.geo.transform(rect, rot), file_path, crop: None, mask_shape: pptx_geom::clip_polygon(geometry), alt_text: None, fit: Some("stretch".into()) })
}

/// A fill picture for a box `size` EMU big: as it is (stretched), or tiled (`style:repeat`
/// "repeat") at `draw:fill-image-width`/`-height` (else its own size), from the point
/// `draw:fill-image-ref-point` names within `area`.
/// Comes back with the opacity still to apply: none (1.0) once a page's fade is blended in, the
/// fade itself for a picture this can't decode.
fn fill_picture(archive: &mut ZipArchive<Cursor<&[u8]>>, href: &str, chain: &[&Style], size: [f64; 2], area: [f64; 4], paper: bool, ctx: &Ctx) -> Option<(String, f64)> {
    let repeat = prop(chain, "style:repeat").unwrap_or("repeat");
    let bytes = pptx::read_picture_bytes(archive, href).ok()?;
    // A page is drawn over white paper, so its see-through picture is blended onto white here
    // (the app's own background opacity isn't drawn by every view).
    let fade = if paper { prop(chain, "draw:opacity").and_then(|o| o.trim_end_matches('%').parse::<f64>().ok()).map_or(1.0, |p| (p / 100.0).clamp(0.0, 1.0)) } else { 1.0 };
    let on_paper = |mut img: image::RgbaImage| {
        for px in img.pixels_mut() {
            let a = px[3] as f64 / 255.0 * fade;
            let c = |v: u8| (v as f64 * a + 255.0 * (1.0 - a)).round() as u8;
            *px = image::Rgba([c(px[0]), c(px[1]), c(px[2]), 255]);
        }
        img
    };
    if repeat == "stretch" {
        if fade < 1.0 {
            if let Ok(src) = image::load_from_memory(&bytes) {
                return Some((ctx.store(format!("{href}#paper:{fade}"), &pptx::encode_picture(on_paper(src.to_rgba8()), pptx::is_jpeg(&bytes))?)?, 1.0));
            }
        }
        return Some((ctx.store(href.to_string(), &bytes)?, fade));
    }
    // Pages and shapes sharing a picture may tile it differently, so the key holds every setting.
    let settings: Vec<Option<&str>> = ["draw:fill-image-width", "draw:fill-image-height", "draw:fill-image-ref-point", "draw:fill-image-ref-point-x", "draw:fill-image-ref-point-y", "draw:tile-repeat-offset"]
        .iter().map(|k| prop(chain, k)).collect();
    let key = format!("{href}#tile:{}x{}:{:?}:{paper}:{fade}:{repeat}:{settings:?}", size[0].round(), size[1].round(), area);
    if let Some(done) = ctx.media.borrow().get(&key) {
        return Some((done.clone(), 1.0));
    }
    // A picture this can't decode (or a metafile) is shown as it is, its fade left to the viewer.
    let Ok(src) = image::load_from_memory(&bytes).map(|i| i.to_rgba8()) else { return Some((ctx.store(href.to_string(), &bytes)?, fade)) };
    let dpi = pptx::image_dpi(&bytes).unwrap_or((96.0, 96.0));
    let (nat_w, nat_h) = (src.width() as f64 / dpi.0 * 914_400.0, src.height() as f64 / dpi.1 * 914_400.0);
    // A tile size as a percentage is of the area filled (LibreOffice's "relative" size); 0 or
    // none is the picture's own size.
    let dim = |k: &str, nat: f64, area_len: f64| prop(chain, k).and_then(|v| match v.strip_suffix('%') { Some(p) => p.parse::<f64>().ok().map(|p| area_len * p / 100.0), None => length_emu(v) }).filter(|v| *v > 0.0).unwrap_or(nat);
    let (tw, th) = (dim("draw:fill-image-width", nat_w, area[2]), dim("draw:fill-image-height", nat_h, area[3]));
    let algn = match prop(chain, "draw:fill-image-ref-point").unwrap_or("top-left") {
        "top" => "t", "top-right" => "tr", "left" => "l", "center" => "ctr", "right" => "r", "bottom-left" => "bl", "bottom" => "b", "bottom-right" => "br", _ => "tl",
    };
    // The tiles start shifted by a share of their own size.
    let shift = |k: &str, len: f64| prop(chain, k).and_then(|v| v.trim_end_matches('%').parse::<f64>().ok()).map_or(0.0, |p| len * p / 100.0);
    let tile = format!(r#"sx="{}" sy="{}" algn="{algn}" tx="{}" ty="{}""#, tw / nat_w * 100_000.0, th / nat_h * 100_000.0, shift("draw:fill-image-ref-point-x", tw), shift("draw:fill-image-ref-point-y", th));
    let mut out = pptx::tile_picture(&src, Some(dpi), &tile, size, area, 1920.0 / ctx.geo.canvas_w);
    if repeat == "no-repeat" {
        // One copy only: everything outside the first tile is cleared.
        let scale = 1920.0 / ctx.geo.canvas_w;
        let (aw, ah) = (area[2], area[3]);
        let ax = match algn { "tr" | "r" | "br" => aw - tw, "t" | "ctr" | "b" => (aw - tw) / 2.0, _ => 0.0 };
        let ay = match algn { "bl" | "b" | "br" => ah - th, "l" | "ctr" | "r" => (ah - th) / 2.0, _ => 0.0 };
        // (The tile sits shifted by the reference-point offsets, as tile_picture draws it.)
        let (sx, sy) = (shift("draw:fill-image-ref-point-x", tw), shift("draw:fill-image-ref-point-y", th));
        let (x0, y0) = ((area[0] + ax + sx) * scale, (area[1] + ay + sy) * scale);
        let (x1, y1) = (x0 + tw * scale, y0 + th * scale);
        for (x, y, px) in out.enumerate_pixels_mut() {
            let (x, y) = (x as f64, y as f64);
            if x < x0 || x >= x1 || y < y0 || y >= y1 {
                // A page's background shows the paper (white) around it; a shape's, what's behind.
                *px = if paper { image::Rgba([255, 255, 255, 255]) } else { image::Rgba([0, 0, 0, 0]) };
            }
        }
    }
    let out = if fade < 1.0 { on_paper(out) } else { out };
    Some((ctx.store(key, &pptx::encode_picture(out, pptx::is_jpeg(&bytes))?)?, 1.0))
}

/// A page's background from its drawing-page style chain: solid, gradient or picture.
fn page_background(archive: &mut ZipArchive<Cursor<&[u8]>>, chain: &[&Style], ctx: &Ctx) -> Option<SlideBackground> {
    let opacity = prop(chain, "draw:opacity");
    match prop(chain, "draw:fill")? {
        // A see-through page shows the paper (white) behind it.
        "solid" => prop(chain, "draw:fill-color").map(|c| SlideBackground::Solid(over_white(c, opacity))),
        "gradient" => prop(chain, "draw:fill-gradient-name").and_then(|n| ctx.styles.gradients.get(n)).and_then(|g| gradient_of(g, opacity)),
        "bitmap" => {
            let href = prop(chain, "draw:fill-image-name").and_then(|n| ctx.styles.fill_images.get(n))?.trim_start_matches("./").to_string();
            let geo = ctx.geo;
            let area = [geo.x_off, geo.y_off, geo.canvas_w - 2.0 * geo.x_off, geo.canvas_h - 2.0 * geo.y_off];
            // Tiled over the whole canvas; a stretched picture is the slide's own size.
            let (file_path, opacity) = fill_picture(archive, &href, chain, [geo.canvas_w, geo.canvas_h], area, true, ctx)?;
            Some(SlideBackground::Image { file_path, opacity })
        }
        "hatch" => prop(chain, "draw:fill-color").map(|c| SlideBackground::Solid(c.to_lowercase())),
        _ => None,
    }
}

/// A table: in a file this app wrote, its own `Table`; otherwise drawn as a group of cells (fill
/// and border from each cell's style) with their text, columns and rows as wide and tall as the
/// table's styles say.
fn table_element(archive: &mut ZipArchive<Cursor<&[u8]>>, table_inner: &str, attrs: &str, rect: [f64; 4], ctx: &Ctx) -> Option<SlideElement> {
    let cols_xml: Vec<(&str, &str, &str)> = children(table_inner).into_iter().filter(|(n, _, _)| *n == "table:table-column").collect();
    let rows_xml: Vec<(&str, &str, &str)> = children(table_inner).into_iter().filter(|(n, _, _)| *n == "table:table-row").collect();
    let ncols: usize = cols_xml.iter().map(|(_, a, _)| extract_attr(a, "table:number-columns-repeated").and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).min(64)).sum::<usize>().max(1);
    if ctx.own_file {
        let mut cells: Vec<Vec<String>> = rows_xml.iter().map(|(_, _, r)| {
            let mut row: Vec<String> = children(r).into_iter().filter(|(n, _, _)| *n == "table:table-cell").map(|(_, _, c)| plain_paragraphs(c)).collect();
            row.resize(ncols, String::new());
            row
        }).collect();
        cells.truncate(256);
        if cells.is_empty() {
            return None;
        }
        return Some(SlideElement::Table { id: new_id(), transform: ctx.geo.transform(rect, 0.0), rows: cells.len(), cols: ncols, cells, text_color: None, border_color: None, font_size_pt: None });
    }
    // The table's template (its look by position: header row, banded rows...) under each cell's own style.
    let template = extract_attr(attrs, "table:template-name").and_then(|n| ctx.styles.templates.get(n));
    let uses = |k: &str| extract_attr(attrs, k) == Some("true");
    let nrows = rows_xml.len();
    // The first part that applies to the cell's position and that the template defines, else its body.
    let template_style = |r: usize, c: usize| -> Option<&str> {
        let t = template?;
        let parts = [
            (r == 0 && uses("table:use-first-row-styles"), "first-row"),
            (r + 1 == nrows && uses("table:use-last-row-styles"), "last-row"),
            (c == 0 && uses("table:use-first-column-styles"), "first-column"),
            (c + 1 == ncols && uses("table:use-last-column-styles"), "last-column"),
            (r % 2 == 1 && uses("table:use-banding-rows-styles"), "odd-rows"),
            (c % 2 == 1 && uses("table:use-banding-columns-styles"), "odd-columns"),
        ];
        parts.iter().filter(|(applies, _)| *applies).find_map(|(_, part)| t.get(*part)).or_else(|| t.get("body")).map(String::as_str)
    };
    let style = |family: &str, name: Option<&str>| ctx.styles.chain(ctx.scope, family, name);
    let mut widths: Vec<f64> = Vec::new();
    for (_, a, _) in &cols_xml {
        let w = prop(&style("table-column", extract_attr(a, "table:style-name")), "style:column-width").and_then(length_emu).unwrap_or(0.0);
        let n = extract_attr(a, "table:number-columns-repeated").and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).min(64);
        widths.extend(std::iter::repeat_n(w, n));
    }
    let total: f64 = widths.iter().sum();
    if total <= 0.0 {
        widths = vec![rect[2] / ncols as f64; ncols];
    }
    let col_defaults: Vec<Option<&str>> = cols_xml.iter().flat_map(|(_, a, _)| {
        let n = extract_attr(a, "table:number-columns-repeated").and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).min(64);
        std::iter::repeat_n(extract_attr(a, "table:default-cell-style-name"), n)
    }).collect();
    let mut heights: Vec<f64> = rows_xml.iter().map(|(_, a, _)| {
        let st = style("table-row", extract_attr(a, "table:style-name"));
        prop(&st, "style:row-height").or_else(|| prop(&st, "style:min-row-height")).and_then(length_emu).unwrap_or(rect[3] / rows_xml.len().max(1) as f64)
    }).collect();
    let col_x: Vec<f64> = std::iter::once(0.0).chain(widths.iter().scan(0.0, |acc, w| { *acc += w; Some(*acc) })).collect();
    // A row's height is its least: first each row grows to fit its cells' text, then they're drawn.
    let mut kids = Vec::new();
    // Borders go over every cell's fill (a neighbour's would hide a shared edge).
    let mut edges = Vec::new();
    let mut measured: HashMap<(usize, usize), ([f64; 4], Option<SlideElement>)> = HashMap::new();
    for measuring in [true, false] {
        let row_y: Vec<f64> = std::iter::once(0.0).chain(heights.iter().scan(0.0, |acc, h| { *acc += h; Some(*acc) })).collect();
        for (r, (_, row_attrs, row)) in rows_xml.iter().enumerate().take(256) {
            // Each cell, covered or not, takes one column (a spanning cell is followed by the covered
            // cells it spans); a repeated cell takes as many.
            let cells: Vec<(&str, &str, &str)> = children(row).into_iter().flat_map(|(n, a, i)| {
                let rep = extract_attr(a, "table:number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).clamp(1, 64);
                std::iter::repeat_n((n, a, i), rep)
            }).take(ncols * 2).collect();
            let mut c = 0;
            for (n, a, cell) in cells {
                if c >= ncols {
                    break;
                }
                if n == "table:covered-table-cell" {
                    c += 1;
                    continue;
                }
                if n != "table:table-cell" {
                    continue;
                }
                let span_c = extract_attr(a, "table:number-columns-spanned").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).max(1);
                let span_r = extract_attr(a, "table:number-rows-spanned").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).max(1);
                let (c2, r2) = ((c + span_c).min(ncols), (r + span_r).min(rows_xml.len()));
                let cell_rect = [rect[0] + col_x[c], rect[1] + row_y[r], col_x[c2] - col_x[c], row_y[r2] - row_y[r]];
                // A cell without a style of its own takes its row's default, else its column's.
                let cell_style = extract_attr(a, "table:style-name")
                    .or_else(|| extract_attr(row_attrs, "table:default-cell-style-name"))
                    .or_else(|| col_defaults.get(c).copied().flatten());
                let mut chain = style("table-cell", cell_style);
                chain.extend(style("table-cell", template_style(r, c)));
                // The text inside reads the same styles (its padding, its alignment, its colour); the
                // template's goes in an attribute of this importer's own.
                let mut cell_attrs = std::borrow::Cow::Borrowed(a);
                if let (None, Some(name)) = (extract_attr(a, "table:style-name"), cell_style) {
                    cell_attrs = std::borrow::Cow::Owned(format!(r#"{a} table:style-name="{name}""#));
                }
                if let Some(name) = template_style(r, c) {
                    cell_attrs = std::borrow::Cow::Owned(format!(r#"{cell_attrs} os:template-style="{name}""#));
                }
                chain.extend(ctx.defaults("table-cell"));
                let fill = match prop(&chain, "draw:fill") {
                    Some("solid") | None if prop(&chain, "draw:fill-color").is_some() => prop(&chain, "draw:fill-color").map(|c| with_opacity(c, prop(&chain, "draw:opacity"))),
                    Some("solid") => prop(&chain, "fo:background-color").filter(|c| *c != "transparent").map(str::to_lowercase),
                    None => prop(&chain, "fo:background-color").filter(|c| *c != "transparent").map(str::to_lowercase),
                    Some("gradient" | "hatch") => shape_fill(&chain, ctx).and_then(Result::ok),
                    _ => None,
                };
                // Each side's `fo:border-<side>`, else `fo:border`, from the nearest style stating
                // either: "0.03cm solid #000000".
                let sides = ["top", "right", "bottom", "left"].map(|side| {
                    let b = chain.iter().find_map(|st| st.props.get(&format!("fo:border-{side}")).or_else(|| st.props.get("fo:border")))?;
                    let parts: Vec<&str> = b.split_whitespace().collect();
                    if b.contains("none") || b.contains("hidden") {
                        return None;
                    }
                    let color = parts.iter().find(|p| p.starts_with('#'))?.to_lowercase();
                    let width = parts.iter().find_map(|p| length_emu(p)).unwrap_or(9_525.0);
                    Some((color, width))
                });
                let cell_fill = fill.clone();
                if measuring {
                    let text = text_block(cell, &cell_attrs, cell_rect, 0.0, &ctx.over(cell_fill.as_deref()), true);
                    if span_r == 1 {
                        let needed = text.iter().map(|el| match el {
                            SlideElement::TextBlock { block, transform, .. } => estimated_height(block, transform.w * ctx.geo.canvas_w, ctx) + cell_rect[3] - transform.h * ctx.geo.canvas_h,
                            _ => 0.0,
                        }).fold(0.0, f64::max);
                        heights[r] = heights[r].max(needed);
                    }
                    measured.insert((r, c), (cell_rect, text));
                    c += 1;
                    continue;
                }
                // A picture fill: the picture over the cell, under its border.
                if prop(&chain, "draw:fill") == Some("bitmap") {
                    if let Some(image) = prop(&chain, "draw:fill-image-name") {
                        kids.extend(picture_fill(archive, image, &chain, &rect_geometry(cell_rect[2], cell_rect[3], 0.0), cell_rect, 0.0, ctx));
                    }
                }
                if let Some(fill) = fill {
                    kids.push(SlideElement::Shape { id: new_id(), transform: ctx.geo.transform(cell_rect, 0.0), shape_kind: "path".into(), fill_color: fill, stroke_width: 0.0, stroke_color: None, path: Some("M 0 0 L 100 0 L 100 100 L 0 100 Z".into()) });
                }
                edges.extend(pptx::cell_edges(cell_rect, &sides, &ctx.geo));
                // The text from measuring, moved down with its row when rows above it grew; laid out
                // again only if its own cell changed height (anchoring and shrinking depend on it).
                kids.extend(match measured.remove(&(r, c)) {
                    Some((before, text)) if (before[3] - cell_rect[3]).abs() < 1.0 => text.map(|mut el| {
                        el.transform_mut().y += (cell_rect[1] - before[1]) / ctx.geo.canvas_h;
                        el
                    }),
                    _ => text_block(cell, &cell_attrs, cell_rect, 0.0, &ctx.over(cell_fill.as_deref()), true),
                });
                c += 1;
            }
        }
    }
    kids.extend(edges);
    (!kids.is_empty()).then(|| group_of(kids))
}

/// About how tall a text block wraps to at `width` EMU: each paragraph word-wrapped by its
/// runs' fonts, its lines `line_height` (1.2 when single) of its largest size apart.
fn estimated_height(block: &crate::core::models::TextBlock, width: f64, ctx: &Ctx) -> f64 {
    use crate::storage::text_metrics::{wrapped_lines, Span};
    let style = &block.paragraph_style;
    // Run sizes carry this file's font scale; paragraph gaps were scaled for the canvas.
    let emu = |pt: f64| pt / ctx.font_scale.max(1e-6) * 12_700.0;
    let gap_emu = |pt: f64| pt / ctx.geo.font_scale.max(1e-6) * 12_700.0;
    let mut paragraphs: Vec<Vec<Span>> = vec![Vec::new()];
    for run in &block.runs {
        for (i, piece) in run.text.split('\n').enumerate() {
            if i > 0 {
                paragraphs.push(Vec::new());
            }
            paragraphs.last_mut().expect("never empty").push(Span { text: piece, family: &run.font_family, bold: run.bold, size: emu(run.font_size_pt) });
        }
    }
    let fallback = block.runs.iter().map(|r| emu(r.font_size_pt)).fold(0.0, f64::max);
    paragraphs.iter().map(|spans| {
        let size = spans.iter().filter(|s| !s.text.is_empty()).map(|s| s.size).fold(0.0, f64::max);
        let size = if size > 0.0 { size } else { fallback };
        wrapped_lines(spans, width) as f64 * size * style.line_height.max(0.5) + gap_emu(style.space_before_pt + style.space_after_pt)
    }).sum()
}

fn plain_paragraphs(xml: &str) -> String {
    children(xml).into_iter().filter(|(n, _, _)| *n == "text:p" || *n == "text:h").map(|(_, _, p)| plain_text_of(p)).collect::<Vec<_>>().join("\n")
}

// ---------------------------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------------------------

/// Text inside a drawn shape: its paragraphs in the geometry's text area (else its box).
fn shape_text(inner: &str, attrs: &str, text_rect: Option<[f64; 4]>, rect: [f64; 4], rot: f64, ctx: &Ctx) -> Option<SlideElement> {
    let area = match text_rect {
        Some([l, t, r, b]) if r > l && b > t => {
            // The text box's centre, moved from the shape's and turned with it.
            let (w, h) = (r - l, b - t);
            let (dx, dy) = ((l + r) / 2.0 - rect[2] / 2.0, (t + b) / 2.0 - rect[3] / 2.0);
            let (sin, cos) = rot.to_radians().sin_cos();
            let (cx, cy) = (rect[0] + rect[2] / 2.0 + dx * cos - dy * sin, rect[1] + rect[3] / 2.0 + dx * sin + dy * cos);
            [cx - w / 2.0, cy - h / 2.0, w, h]
        }
        _ => rect,
    };
    text_block(inner, attrs, area, rot, ctx, true)
}

/// One text run's formatting, resolved.
#[derive(Clone, Default)]
struct RunLook {
    size_pt: f64,
    /// `None` when no style states one (a file this app wrote then keeps its own default).
    color: Option<String>,
    /// "Automatic" colour: black or white against what's behind it.
    auto_color: bool,
    font: Option<String>,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    baseline: &'static str,
    caps: bool,
}

/// A run's look along `chains` (nearest first): font size percentages scale what lies beneath.
fn run_look(chains: &[&Style], ctx: &Ctx) -> RunLook {
    let mut size = None;
    let mut factor = 1.0;
    for s in chains {
        match s.props.get("fo:font-size") {
            Some(v) if v.ends_with('%') => factor *= v.trim_end_matches('%').parse::<f64>().unwrap_or(100.0) / 100.0,
            Some(v) => {
                if let Some(pt) = length_pt(v) {
                    size = Some(pt);
                    break;
                }
            }
            None => {}
        }
    }
    let size_pt = size.unwrap_or(18.0) * factor;
    // A font name with no font-face declaration is the family itself.
    let font = prop(chains, "style:font-name").map(|n| ctx.styles.fonts.get(n).cloned().unwrap_or_else(|| n.to_string())).or_else(|| prop(chains, "fo:font-family").map(|f| f.trim_matches(|c| c == '\'' || c == '"').to_string()));
    let weight = prop(chains, "fo:font-weight").unwrap_or("normal");
    RunLook {
        size_pt,
        // The nearest style that says either a colour or "automatic" decides.
        color: chains.iter().find_map(|s| {
            if s.props.get("style:use-window-font-color").is_some_and(|v| v == "true") { Some(None) } else { s.props.get("fo:color").map(|c| Some(c.to_lowercase())) }
        }).flatten(),
        auto_color: chains.iter().find_map(|s| if s.props.get("style:use-window-font-color").is_some_and(|v| v == "true") { Some(true) } else { s.props.get("fo:color").map(|_| false) }).unwrap_or(false),
        font,
        bold: weight == "bold" || weight.parse::<u32>().is_ok_and(|w| w >= 600),
        italic: matches!(prop(chains, "fo:font-style"), Some("italic" | "oblique")),
        underline: prop(chains, "style:text-underline-style").is_some_and(|v| v != "none"),
        strike: prop(chains, "style:text-line-through-style").is_some_and(|v| v != "none"),
        baseline: match prop(chains, "style:text-position").and_then(|p| p.split_whitespace().next()) {
            Some("super") => "super",
            Some("sub") => "sub",
            Some(v) => match v.trim_end_matches('%').parse::<f64>() { Ok(x) if x > 0.0 => "super", Ok(x) if x < 0.0 => "sub", _ => "normal" },
            None => "normal",
        },
        caps: matches!(prop(chains, "fo:text-transform"), Some("uppercase")) || prop(chains, "fo:font-variant") == Some("small-caps"),
    }
}

/// A stored picture's average colour, as `#rrggbb`, sampled as LibreOffice's draft fill colour
/// does: every pixel on a grid of about 8 × 8 steps from the top-left corner.
fn average_color(path: &Path) -> Option<String> {
    Some(pptx::average_hex(&image::open(path).ok()?.to_rgb8(), 8))
}

/// Whether a colour (`#rrggbb[aa]`) is dark enough that automatic text on it is white, by
/// LibreOffice's rule: luminance at most 156 of 255, or 62 on its default shape blue (#729fcf).
fn is_dark(color: Option<&str>) -> bool {
    let Some(c) = color.and_then(|c| c.strip_prefix('#')).and_then(|c| c.get(..6)).filter(|c| c.is_ascii()) else { return false };
    let v = |i: usize| u8::from_str_radix(&c[i..i + 2], 16).map(u32::from).unwrap_or(255);
    let luminance = (v(4) * 29 + v(2) * 151 + v(0) * 76) >> 8;
    luminance <= if c[..6].eq_ignore_ascii_case("729fcf") { 62 } else { 156 }
}

fn run_of(text: String, look: &RunLook, ctx: &Ctx) -> TextRun {
    // A file this app wrote keeps its own font names and, where it wrote none, its own defaults;
    // anyone else's unstyled text is black in the default font, as LibreOffice shows it.
    let (color, font_family) = if ctx.own_file {
        (look.color.clone().unwrap_or_else(crate::core::models::default_text_color), look.font.clone().unwrap_or_else(crate::core::models::default_font_family))
    } else {
        let auto = || if is_dark(ctx.backdrop.as_deref()) { "#ffffff".to_string() } else { "#000000".to_string() };
        (look.color.clone().filter(|_| !look.auto_color).unwrap_or_else(auto), pptx::css_font_stack(look.font.as_deref().unwrap_or("Liberation Sans")))
    };
    TextRun {
        text: if look.caps { text.to_uppercase() } else { text },
        bold: look.bold,
        italic: look.italic,
        underline: look.underline,
        strike: look.strike,
        color,
        font_family,
        font_size_pt: (look.size_pt * ctx.font_scale * 10.0).round() / 10.0,
        baseline_shift: look.baseline.to_string(),
        ..Default::default()
    }
}

/// A paragraph's inline content as runs: text, spans (nested), spaces (`text:s`), tabs, line
/// breaks (kept inside a run), links, and fields (page number, footer).
fn inline_runs(xml: &str, chains: &[&Style], ctx: &Ctx, out: &mut Vec<TextRun>) {
    nested(|| inline_runs_here(xml, chains, ctx, out))
}

fn inline_runs_here(xml: &str, chains: &[&Style], ctx: &Ctx, out: &mut Vec<TextRun>) {
    // The text between elements, without any stray close tag or comment in it.
    let text_run = |between: &str, out: &mut Vec<TextRun>| {
        let mut plain = String::new();
        let mut inside = false;
        for c in between.chars() {
            match c {
                '<' => inside = true,
                '>' if inside => inside = false,
                c if !inside => plain.push(c),
                _ => {}
            }
        }
        let text = decode_xml_entities(&plain);
        if !text.is_empty() {
            out.push(run_of(text, &run_look(chains, ctx), ctx));
        }
    };
    // One pass over the elements (each a slice of `xml`), with the text before each.
    let base = xml.as_ptr() as usize;
    let mut pos = 0;
    for (name, attrs, inner) in children(xml) {
        // Where this element starts (its `<name`, just before its attributes) and ends (after its
        // close tag, or the self-closing open tag).
        let attrs_at = (attrs.as_ptr() as usize).saturating_sub(base).min(xml.len());
        let start = xml[..attrs_at].rfind('<').filter(|s| *s >= pos).unwrap_or(pos);
        text_run(&xml[pos..start], out);
        let open_end = xml[start..].find('>').map_or(xml.len(), |g| start + g + 1);
        let end = if inner.is_empty() && xml[..open_end].ends_with("/>") {
            open_end
        } else {
            let close = format!("</{name}>");
            let body_end = (inner.as_ptr() as usize).saturating_sub(base).min(xml.len()) + inner.len();
            xml[body_end.min(xml.len())..].find(&close).map_or(xml.len(), |p| body_end + p + close.len())
        };
        pos = end.max(pos).min(xml.len());
        let look = || run_look(chains, ctx);
        match name {
            "text:span" | "text:a" => {
                let mut c: Vec<&Style> = ctx.styles.chain(ctx.scope, "text", extract_attr(attrs, "text:style-name"));
                c.extend_from_slice(chains);
                inline_runs(inner, &c, ctx, out);
            }
            "text:s" => {
                let n = extract_attr(attrs, "text:c").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(200);
                out.push(run_of(" ".repeat(n), &look(), ctx));
            }
            "text:tab" => out.push(run_of("\t".into(), &look(), ctx)),
            // A line break stays inside the run before it (a run of exactly "\n" separates paragraphs).
            "text:line-break" => match out.last_mut() {
                Some(r) if r.text != "\n" => r.text.push('\n'),
                _ => out.push(run_of("\n\u{200b}".into(), &look(), ctx)),
            },
            "text:page-number" => out.push(run_of(ctx.page_number.to_string(), &look(), ctx)),
            "presentation:footer" => if let Some(f) = &ctx.footer { out.push(run_of(f.clone(), &look(), ctx)) },
            "presentation:date-time" | "presentation:header" => {}
            // Other fields show their current text.
            _ => inline_runs(inner, chains, ctx, out),
        }
    }
    text_run(&xml[pos..], out);
}

/// A list level's marker: a bullet character, a number format, or nothing; with its look.
struct Marker {
    bullet: Option<String>,
    number: Option<(String, String, String, usize)>,
    /// Indent of this level's text from the frame's left, in EMU (space-before + label width).
    indent: f64,
    size_pct: f64,
    color: Option<String>,
    font: Option<String>,
}

fn list_marker(list_inner: &str, level: usize) -> Option<Marker> {
    let lvl = children(list_inner).into_iter().find(|(_, a, _)| extract_attr(a, "text:level").and_then(|v| v.parse::<usize>().ok()) == Some(level))?;
    let (name, attrs, inner) = lvl;
    let props = next_tag(inner, "style:list-level-properties").map(|(a, _, _)| a).unwrap_or("");
    let n = |k: &str| extract_attr(props, k).and_then(length_emu).unwrap_or(0.0);
    let mut indent = n("text:space-before") + n("text:min-label-width");
    if let Some((al, _, _)) = next_tag(inner, "style:list-level-label-alignment") {
        indent = indent.max(extract_attr(al, "fo:margin-left").and_then(length_emu).unwrap_or(0.0));
    }
    let tp = next_tag(inner, "style:text-properties").map(|(a, _, _)| a).unwrap_or("");
    let size_pct = extract_attr(tp, "fo:font-size").and_then(|v| v.strip_suffix('%')?.parse::<f64>().ok()).unwrap_or(100.0) / 100.0;
    let color = (extract_attr(tp, "style:use-window-font-color") != Some("true")).then(|| extract_attr(tp, "fo:color").map(str::to_lowercase)).flatten();
    let font = extract_attr(tp, "fo:font-family").map(str::to_string);
    let mut m = Marker { bullet: None, number: None, indent, size_pct, color, font };
    match name {
        "text:list-level-style-bullet" => m.bullet = extract_attr(attrs, "text:bullet-char").map(decode_xml_entities).filter(|c| !c.trim().is_empty()),
        "text:list-level-style-number" => {
            let fmt = extract_attr(attrs, "style:num-format").unwrap_or("");
            if !fmt.is_empty() {
                let start = extract_attr(attrs, "text:start-value").and_then(|v| v.parse().ok()).unwrap_or(1);
                m.number = Some((fmt.to_string(), extract_attr(attrs, "style:num-prefix").unwrap_or("").to_string(), extract_attr(attrs, "style:num-suffix").unwrap_or("").to_string(), start));
            }
        }
        "text:list-level-style-image" => m.bullet = Some("•".into()),
        _ => {}
    }
    Some(m)
}

/// A number in an ODF number format ("1", "a", "A", "i", "I").
fn format_number(fmt: &str, n: usize) -> String {
    let kind = match fmt {
        "a" => "alphaLcPeriod",
        "A" => "alphaUcPeriod",
        "i" => "romanLcPeriod",
        "I" => "romanUcPeriod",
        _ => "arabicPeriod",
    };
    pptx_inherit_label(kind, n)
}

fn pptx_inherit_label(kind: &str, n: usize) -> String {
    // The DrawingML labels end with a period; ODF says its own suffix.
    crate::storage::pptx_inherit::autonum_label(kind, n).trim_end_matches('.').to_string()
}

/// A text body (a text box, a shape's or a table cell's paragraphs) as a `TextBlock` in `rect`
/// (EMU), its paragraphs styled along their chains down to `attrs`' frame style.
fn text_block(body: &str, attrs: &str, rect: [f64; 4], rot: f64, ctx: &Ctx, centred: bool) -> Option<SlideElement> {
    let frame = {
        let mut c = ctx.shape_chain(attrs);
        if attrs.contains("table:style-name") || attrs.contains("os:template-style") {
            c = ctx.styles.chain(ctx.scope, "table-cell", extract_attr(attrs, "table:style-name"));
            if let Some(t) = extract_attr(attrs, "os:template-style") {
                c.extend(ctx.styles.chain(ctx.scope, "table-cell", Some(t)));
            }
        }
        c
    };
    let text_style = ctx.styles.chain(ctx.scope, "paragraph", extract_attr(attrs, "draw:text-style-name"));
    let class = extract_attr(attrs, "presentation:class");
    let mut defaults: Vec<&Style> = Vec::new();
    defaults.extend(ctx.defaults("graphic"));
    defaults.extend(ctx.defaults("paragraph"));
    // The frame's presentation style's list, for paragraphs in a list that names none.
    let frame_list = frame.iter().find_map(|s| s.list.as_deref()).and_then(|n| ctx.styles.lists.get(n));

    // Flatten the body into (paragraph attrs, content, list depth, list style, numbered item index).
    let mut paras: Vec<(&str, &str, usize, Option<&str>, bool)> = Vec::new();
    // Paragraphs (by index) whose list says to go on counting where the last one stopped
    // (`text:continue-numbering`, `text:continue-list`).
    let mut continuing: std::collections::HashSet<usize> = std::collections::HashSet::new();
    fn walk<'a>(xml: &'a str, depth: usize, list: Option<&'a str>, header: bool, cont: bool, out: &mut Vec<(&'a str, &'a str, usize, Option<&'a str>, bool)>, continuing: &mut std::collections::HashSet<usize>) {
        nested(|| walk_here(xml, depth, list, header, cont, out, continuing))
    }
    fn walk_here<'a>(xml: &'a str, depth: usize, list: Option<&'a str>, header: bool, cont: bool, out: &mut Vec<(&'a str, &'a str, usize, Option<&'a str>, bool)>, continuing: &mut std::collections::HashSet<usize>) {
        for (n, a, i) in children(xml) {
            match n {
                "text:p" | "text:h" => {
                    if cont {
                        continuing.insert(out.len());
                    }
                    out.push((a, i, depth, list, header))
                }
                "text:list" => {
                    let cont = cont || extract_attr(a, "text:continue-numbering") == Some("true") || extract_attr(a, "text:continue-list").is_some();
                    walk(i, depth + 1, extract_attr(a, "text:style-name").or(list), false, cont, out, continuing)
                }
                "text:list-item" => walk(i, depth, list, false, cont, out, continuing),
                "text:list-header" => walk(i, depth, list, true, cont, out, continuing),
                _ => {}
            }
            if out.len() > 2_000 {
                return;
            }
        }
    }
    walk(body, 0, None, false, false, &mut paras, &mut continuing);

    let mut runs: Vec<TextRun> = Vec::new();
    let mut align_weight: HashMap<String, usize> = HashMap::new();
    let mut gaps: Vec<(f64, f64, usize)> = Vec::new();
    let mut counters = [0usize; 10];
    let mut numbering_broken = false;
    let mut any_text = false;
    let mut any_lead = false;
    let mut no_lead: Vec<usize> = Vec::new();
    let mut first: Option<(Vec<&Style>, f64)> = None;
    let base_indent = paras.iter().find(|p| p.2 > 0).and_then(|p| p.3.and_then(|n| ctx.styles.lists.get(n)).or(frame_list).and_then(|l| list_marker(l, 1))).map_or(0.0, |m| m.indent);
    for (pi, (pa, pc, depth, list, header)) in paras.iter().enumerate() {
        // The master's style for this outline level comes between the paragraph and the frame.
        let level_style: Vec<&Style> = if class == Some("outline") && *depth > 1 {
            ctx.styles.chain(Scope::Master, "presentation", Some(&format!("{}-outline{}", ctx.master, depth)))
        } else {
            Vec::new()
        };
        let mut chain: Vec<&Style> = ctx.styles.chain(ctx.scope, "paragraph", extract_attr(pa, "text:style-name"));
        chain.extend(text_style.iter().copied());
        chain.extend(level_style);
        chain.extend(frame.iter().copied());
        chain.extend(defaults.iter().copied());

        let mut para: Vec<TextRun> = Vec::new();
        inline_runs(pc, &chain, ctx, &mut para);
        if pi > 0 {
            let mut sep = para.first().cloned().unwrap_or_else(|| run_of(String::new(), &run_look(&chain, ctx), ctx));
            sep.text = "\n".into();
            runs.push(sep);
        }
        let has_text = para.iter().any(|r| !r.text.trim().is_empty());
        if !has_text {
            runs.extend(para);
            continue;
        }
        any_text = true;
        let look = run_look(&chain, ctx);
        if first.is_none() {
            first = Some((chain.clone(), look.size_pt));
        }
        let weight: usize = para.iter().map(|r| r.text.chars().count()).sum();
        let align = match prop(&chain, "fo:text-align") {
            Some("center") => "center",
            Some("end" | "right") => "right",
            Some("justify") => "justify",
            _ => "left",
        };
        *align_weight.entry(align.to_string()).or_default() += weight;
        let gap = |k: &str| prop(&chain, k).and_then(length_pt).unwrap_or(0.0) * ctx.geo.font_scale;
        gaps.push((gap("fo:margin-top"), gap("fo:margin-bottom"), weight));

        // The list marker and indent.
        let marker = if *depth > 0 && !header {
            list.and_then(|n| ctx.styles.lists.get(n)).or(frame_list).and_then(|l| list_marker(l, (*depth).min(10)))
        } else {
            // A paragraph outside any list (or a list header) ends the numbering: the next numbered
            // list starts again from its first number, as Impress counts -- unless it says to go on.
            numbering_broken = true;
            None
        };
        let label = marker.as_ref().and_then(|m| {
            let d = (*depth).min(10) - 1;
            if let Some((fmt, pre, suf, start)) = &m.number {
                if std::mem::take(&mut numbering_broken) && !continuing.contains(&pi) {
                    counters = [0; 10];
                }
                counters[d] += 1;
                counters.iter_mut().skip(d + 1).for_each(|c| *c = 0);
                Some(format!("{pre}{}{suf}", format_number(fmt, counters[d] + start - 1)))
            } else {
                counters.iter_mut().skip(d).for_each(|c| *c = 0);
                m.bullet.clone().map(|b| crate::storage::pptx_inherit::bullet_glyph(&b, m.font.as_deref()))
            }
        });
        let en = look.size_pt * 12_700.0 / 2.0;
        let indent = marker.as_ref().map_or(0.0, |m| (m.indent - base_indent).max(0.0)) + prop(&chain, "fo:margin-left").and_then(length_emu).unwrap_or(0.0).max(0.0);
        let spaces = if en > 0.0 && indent.is_finite() { (indent / en).round().clamp(0.0, 40.0) as usize } else { 0 };
        if label.is_some() || spaces > 0 {
            any_lead = true;
            let mut lead = para[0].clone();
            lead.text = format!("{}{}", "\u{2002}".repeat(spaces), label.map(|l| format!("{l} ")).unwrap_or_default());
            (lead.bold, lead.italic, lead.underline, lead.strike) = (false, false, false, false);
            if let Some(m) = &marker {
                if let Some(c) = &m.color {
                    lead.color = c.clone();
                }
                if let Some(f) = &m.font {
                    lead.font_family = pptx::css_font_stack(f);
                }
                lead.font_size_pt = ((lead.font_size_pt * m.size_pct) * 10.0).round() / 10.0;
            }
            runs.push(lead);
        } else {
            no_lead.push(runs.len());
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
    let (first_chain, first_size) = first.unwrap_or_default();
    let align = align_weight.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0))).map(|(a, _)| a).unwrap_or_else(|| "left".into());
    let (space_before_pt, space_after_pt) = pptx::paragraph_gaps(&gaps);
    // Line spacing: a percentage of single spacing (1.2 lines of the font), or a fixed height.
    let line_height = match prop(&first_chain, "fo:line-height") {
        Some(v) if v.ends_with('%') => v.trim_end_matches('%').parse::<f64>().unwrap_or(100.0) / 100.0 * 1.2,
        Some(v) => length_pt(v).filter(|_| first_size > 0.0).map_or(1.2, |pt| (pt / first_size).clamp(0.5, 5.0)),
        None => 1.2,
    };
    // Vertical anchor and insets from the frame (a shape's text is centred by default).
    let mut fchain = frame.clone();
    fchain.extend(ctx.defaults("graphic"));
    let vertical_align = match prop(&fchain, "draw:textarea-vertical-align").or_else(|| prop(&fchain, "style:vertical-align")) {
        Some("middle") => "middle",
        Some("bottom") => "bottom",
        Some("top") => "top",
        _ if centred => "middle",
        _ => "top",
    };
    // This app's own text boxes have no inner margin; anyone else's default to LibreOffice's.
    let pad = |k: &str, d: f64| prop(&fchain, k).or_else(|| prop(&fchain, "fo:padding")).and_then(length_emu).unwrap_or(if ctx.own_file { 0.0 } else { d });
    let (pl, pt, pr, pb) = (pad("fo:padding-left", 91_440.0), pad("fo:padding-top", 45_720.0), pad("fo:padding-right", 91_440.0), pad("fo:padding-bottom", 45_720.0));
    let [x, y, w, h] = rect;
    let inner = [x + pl, y + pt, (w - pl - pr).max(w * 0.2), (h - pt - pb).max(h * 0.2)];
    let mut block = TextBlock {
        runs,
        paragraph_style: TextParagraphStyle {
            align,
            line_height: (line_height * 100.0).round() / 100.0,
            space_before_pt,
            space_after_pt,
            vertical_align: Some(vertical_align.to_string()),
            hanging_indent: any_lead,
            ..Default::default()
        },
        effects: ElementEffects::default(),
        autofit: false,
    };
    // "Shrink on overflow": LibreOffice scales the text down until it fits the box (and stores no
    // scale), so the largest scale the estimate says fits is baked into the sizes here -- only
    // past the estimate's own margin of error, so text that fits isn't shrunk by mistake.
    if prop(&fchain, "style:shrink-to-fit") == Some("true") || prop(&fchain, "draw:fit-to-size") == Some("shrink-to-fit") {
        let height_at = |s: f64| {
            let mut b = block.clone();
            b.runs.iter_mut().for_each(|r| r.font_size_pt *= s);
            b.paragraph_style.space_before_pt *= s;
            b.paragraph_style.space_after_pt *= s;
            estimated_height(&b, inner[2], ctx)
        };
        let fits = |s: f64| height_at(s) <= inner[3];
        if height_at(1.0) > inner[3] * 1.15 {
            let (mut lo, mut hi) = (0.05, 1.0);
            for _ in 0..12 {
                let mid = (lo + hi) / 2.0;
                if fits(mid) { lo = mid } else { hi = mid }
            }
            block.runs.iter_mut().for_each(|r| r.font_size_pt = (r.font_size_pt * lo * 10.0).round() / 10.0);
            block.paragraph_style.space_before_pt = (block.paragraph_style.space_before_pt * lo * 10.0).round() / 10.0;
            block.paragraph_style.space_after_pt = (block.paragraph_style.space_after_pt * lo * 10.0).round() / 10.0;
        }
    }
    Some(SlideElement::TextBlock { id: new_id(), transform: ctx.geo.transform(inner, rot), block })
}

/// Plain text (paragraphs joined with "\n") from the notes block's own text frame
/// (`presentation:class="notes"`), ignoring the page-thumbnail sibling. `None` for an empty or
/// placeholder-only notes frame.
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

/// A paragraph's text without its markup.
fn plain_text_of(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    // Bounded: each pass consumes text up to a tag and the tag.
    while let Some(lt) = rest.find('<') {
        out.push_str(&decode_xml_entities(&rest[..lt]));
        let Some(gt) = rest[lt..].find('>') else { break };
        let tag = &rest[lt + 1..lt + gt];
        if tag.starts_with("text:s") && !tag.starts_with("text:span") {
            let n = extract_attr(tag, "text:c").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(200);
            out.push_str(&" ".repeat(n));
        } else if tag.starts_with("text:line-break") {
            out.push('\n');
        } else if tag.starts_with("text:tab") {
            out.push('\t');
        }
        rest = &rest[lt + gt + 1..];
    }
    out.push_str(&decode_xml_entities(rest));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import(content: &str) -> Presentation {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("mimetype", zip::write::FileOptions::<()>::default()).unwrap();
        std::io::Write::write_all(&mut zip, b"application/vnd.oasis.opendocument.presentation").unwrap();
        zip.start_file("content.xml", zip::write::FileOptions::<()>::default()).unwrap();
        std::io::Write::write_all(&mut zip, content.as_bytes()).unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let dir = tempfile::tempdir().unwrap();
        OdpImporter::import_odp_bytes(&bytes, "t", dir.path()).unwrap()
    }

    #[test]
    fn a_connector_sits_in_the_box_its_resolved_path_spans() {
        // LibreOffice writes elbow connectors with relative h/v commands: from (13900, 6400) left
        // 2000, down 3001, left 2000 -- x 9900..13900, y 6400..9401 (1/100 mm).
        let p = import(r#"<office:document-content><office:body><office:presentation><draw:page>
            <draw:connector svg:x1="13.9cm" svg:y1="6.4cm" svg:x2="9.9cm" svg:y2="9.401cm" svg:d="M13900 6400h-2000v3001h-2000"/>
          </draw:page></office:presentation></office:body></office:document-content>"#);
        let SlideElement::Line { transform: t, .. } = &p.slides[0].elements[0] else { panic!("{:?}", p.slides[0].elements) };
        // The page (no layout given) is 28 x 15.75 cm, the whole 16:9 canvas.
        let cm = |v: f64, page: f64| v * page;
        assert!((cm(t.x, 28.0) - 9.9).abs() < 0.01 && (cm(t.y, 15.75) - 6.4).abs() < 0.01, "{t:?}");
        assert!((cm(t.w, 28.0) - 4.0).abs() < 0.01 && (cm(t.h, 15.75) - 3.001).abs() < 0.01, "{t:?}");
    }

    #[test]
    fn numbering_starts_again_after_a_paragraph_outside_the_list_unless_continued() {
        let p = import(r#"<office:document-content><office:automatic-styles>
            <text:list-style style:name="L1"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."/></text:list-style>
          </office:automatic-styles><office:body><office:presentation><draw:page>
            <draw:frame svg:x="1cm" svg:y="1cm" svg:width="10cm" svg:height="8cm"><draw:text-box>
              <text:list text:style-name="L1"><text:list-item><text:p>a</text:p></text:list-item><text:list-item><text:p>b</text:p></text:list-item></text:list>
              <text:p>between</text:p>
              <text:list text:style-name="L1"><text:list-item><text:p>c</text:p></text:list-item></text:list>
            </draw:text-box></draw:frame>
          </draw:page></office:presentation></office:body></office:document-content>"#);
        let SlideElement::TextBlock { block, .. } = &p.slides[0].elements[0] else { panic!("{:?}", p.slides[0].elements) };
        let text: String = block.runs.iter().map(|r| r.text.as_str()).collect();
        assert!(text.contains("2. ") && !text.contains("3. "), "{text:?}");
        assert_eq!(text.matches("1. ").count(), 2, "{text:?}");
        // A list that says to continue counts on from 2.
        let p = import(r#"<office:document-content><office:automatic-styles>
            <text:list-style style:name="L1"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."/></text:list-style>
          </office:automatic-styles><office:body><office:presentation><draw:page>
            <draw:frame svg:x="1cm" svg:y="1cm" svg:width="10cm" svg:height="8cm"><draw:text-box>
              <text:list text:style-name="L1"><text:list-item><text:p>a</text:p></text:list-item><text:list-item><text:p>b</text:p></text:list-item></text:list>
              <text:p>between</text:p>
              <text:list text:style-name="L1" text:continue-numbering="true"><text:list-item><text:p>c</text:p></text:list-item></text:list>
            </draw:text-box></draw:frame>
          </draw:page></office:presentation></office:body></office:document-content>"#);
        let SlideElement::TextBlock { block, .. } = &p.slides[0].elements[0] else { panic!() };
        let text: String = block.runs.iter().map(|r| r.text.as_str()).collect();
        assert!(text.contains("3. "), "{text:?}");
    }
}
