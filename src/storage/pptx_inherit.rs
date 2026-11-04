//! What a PowerPoint slide inherits: its layout's and master's placeholders (position, text
//! styles, body properties), the master's text styles, the theme's colours/fonts/background
//! styles, and the colour map that ties them together. Real decks leave almost everything to
//! this chain -- a title or body placeholder typically carries only its text, no position,
//! colour, size or alignment of its own -- so `pptx_import` resolves every text property and
//! background through here rather than reading only what the slide states outright.

use std::collections::HashMap;

use crate::storage::xml_scan::{extract_attr, next_tag};

/// One child element: `(name, attributes, inner XML)`. `inner` is empty for a self-closing tag.
pub type Child<'a> = (&'a str, &'a str, &'a str);

/// The direct children of an element's inner XML, in document order -- unlike `next_tag`, which
/// finds one named tag anywhere below, this walks one level only and copes with same-named
/// nesting (a `<p:grpSp>` inside a `<p:grpSp>`).
pub fn children(xml: &str) -> Vec<Child<'_>> {
    let mut out = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0;
    while let Some(rel) = xml[i..].find('<') {
        let start = i + rel;
        let rest = &xml[start + 1..];
        if rest.starts_with('/') || rest.starts_with('?') || rest.starts_with('!') {
            // A stray close tag, a processing instruction or a comment/CDATA: step over it.
            i = start + 1 + rest.find('>').unwrap_or(rest.len().saturating_sub(1)) + 1;
            continue;
        }
        let name_len = rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(rest.len());
        let name = &rest[..name_len];
        let Some(gt_rel) = rest.find('>') else { break };
        let gt = start + 1 + gt_rel;
        let self_closing = bytes.get(gt.wrapping_sub(1)) == Some(&b'/');
        let attrs_end = if self_closing { gt - 1 } else { gt };
        let attrs = &xml[start + 1 + name_len..attrs_end];
        if self_closing {
            out.push((name, attrs.trim(), ""));
            i = gt + 1;
            continue;
        }
        // Find the matching close tag, counting nested opens of the same name.
        let (open, close) = (format!("<{name}"), format!("</{name}>"));
        // `<a:p` must not count `<a:pPr` as another `<a:p>`: the next real opening tag at or after `from`.
        let real_open = |from: usize| {
            let mut at = from;
            loop {
                let p = at + xml[at..].find(&open)?;
                match xml.as_bytes().get(p + open.len()) {
                    Some(b' ') | Some(b'>') | Some(b'/') | Some(b'\n') | Some(b'\r') | Some(b'\t') => return Some(p),
                    _ => at = p + 1,
                }
            }
        };
        let mut depth = 1;
        let mut j = gt + 1;
        // Found once and reused until passed, so look-alike tags are never scanned twice.
        let mut cached_open: Option<Option<usize>> = None;
        let mut cached_close: Option<Option<usize>> = None;
        let end = loop {
            let next_close = match cached_close {
                Some(Some(c)) if c >= j => Some(c),
                Some(None) => None,
                _ => {
                    let c = xml[j..].find(&close).map(|p| j + p);
                    cached_close = Some(c);
                    c
                }
            };
            let next_open = match cached_open {
                Some(Some(o)) if o >= j => Some(o),
                Some(None) => None,
                _ => {
                    let o = real_open(j);
                    cached_open = Some(o);
                    o
                }
            };
            match (next_open, next_close) {
                (_, None) => break None,
                (Some(o), Some(c)) if o < c => {
                    // A self-closing nested open doesn't deepen.
                    let o_gt = o + xml[o..].find('>').unwrap_or(0);
                    if xml.as_bytes().get(o_gt.wrapping_sub(1)) != Some(&b'/') {
                        depth += 1;
                    }
                    j = o_gt + 1;
                }
                (_, Some(c)) => {
                    depth -= 1;
                    if depth == 0 {
                        break Some(c);
                    }
                    j = c + close.len();
                }
            }
        };
        let Some(end) = end else { break };
        out.push((name, attrs.trim(), &xml[gt + 1..end]));
        i = end + close.len();
    }
    out
}

/// The first direct child named `name`.
pub fn child<'a>(xml: &'a str, name: &str) -> Option<Child<'a>> {
    children(xml).into_iter().find(|(n, _, _)| *n == name)
}

fn num(attrs: &str, name: &str) -> Option<f64> {
    extract_attr(attrs, name).and_then(|s| s.parse::<f64>().ok())
}

fn flag(attrs: &str, name: &str) -> Option<bool> {
    extract_attr(attrs, name).map(|v| v == "1" || v == "true")
}

// ---------------------------------------------------------------------------------------------
// Theme and colours
// ---------------------------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct Theme {
    /// dk1, lt1, dk2, lt2, accent1..6, hlink, folHlink -> "#rrggbb".
    pub colors: HashMap<String, String>,
    pub major_font: Option<String>,
    pub minor_font: Option<String>,
    /// `<a:bgFillStyleLst>`'s fills, in order: `bgRef idx="1001"` is the first.
    pub bg_fills: Vec<String>,
    /// `<a:fillStyleLst>`'s fills: a shape's `fillRef idx="1"` is the first.
    pub fills: Vec<String>,
    /// The theme part's own path, for a background picture fill that lives in it.
    pub path: String,
}

impl Theme {
    pub fn parse(xml: &str, path: &str) -> Theme {
        let mut t = Theme { path: path.to_string(), ..Default::default() };
        if let Some((_, scheme, _)) = next_tag(xml, "a:clrScheme") {
            for (name, _, inner) in children(scheme) {
                let key = name.trim_start_matches("a:").to_string();
                if let Some((n, a, i)) = children(inner).into_iter().next() {
                    if let Some(c) = plain_color(n, a, i) {
                        t.colors.insert(key, c);
                    }
                }
            }
        }
        let font = |tag: &str| next_tag(xml, tag).and_then(|(_, inner, _)| next_tag(inner, "a:latin")).and_then(|(a, _, _)| extract_attr(a, "typeface").map(str::to_string)).filter(|f| !f.is_empty());
        t.major_font = font("a:majorFont");
        t.minor_font = font("a:minorFont");
        let fills = |tag: &str| next_tag(xml, tag).map(|(_, inner, _)| children(inner).into_iter().map(|(n, a, i)| format!("<{n} {a}>{i}</{n}>")).collect()).unwrap_or_default();
        t.bg_fills = fills("a:bgFillStyleLst");
        t.fills = fills("a:fillStyleLst");
        t
    }
}

/// A colour element that needs no theme: srgbClr, sysClr (its cached `lastClr`), prstClr, scrgbClr.
fn plain_color(name: &str, attrs: &str, _inner: &str) -> Option<String> {
    match name {
        "a:srgbClr" => extract_attr(attrs, "val").map(|v| format!("#{}", v.to_lowercase())),
        "a:sysClr" => extract_attr(attrs, "lastClr").map(|v| format!("#{}", v.to_lowercase()))
            .or_else(|| match extract_attr(attrs, "val") { Some("window") => Some("#ffffff".into()), Some(_) => Some("#000000".into()), None => None }),
        "a:prstClr" => preset_color(extract_attr(attrs, "val")?),
        "a:scrgbClr" => {
            let ch = |k: &str| num(attrs, k).map(|v| (linear_to_srgb(v / 100_000.0) * 255.0).round().clamp(0.0, 255.0) as u8);
            Some(format!("#{:02x}{:02x}{:02x}", ch("r")?, ch("g")?, ch("b")?))
        }
        _ => None,
    }
}

fn srgb_to_linear(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Applies `f` to each channel in linear light and converts back.
fn linear_map((r, g, b): (f64, f64, f64), f: impl Fn(f64) -> f64) -> (f64, f64, f64) {
    let m = |c: f64| linear_to_srgb(f(srgb_to_linear(c)).clamp(0.0, 1.0));
    (m(r), m(g), m(b))
}

fn linear_to_srgb(v: f64) -> f64 {
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

fn preset_color(name: &str) -> Option<String> {
    Some(match name {
        "black" => "#000000", "white" => "#ffffff", "red" => "#ff0000", "green" => "#008000", "blue" => "#0000ff",
        "yellow" => "#ffff00", "gray" | "grey" => "#808080", "darkGray" | "dkGray" => "#a9a9a9", "lightGray" | "ltGray" => "#d3d3d3",
        "orange" => "#ffa500", "purple" => "#800080", "navy" => "#000080", "maroon" => "#800000", "teal" => "#008080",
        _ => return None,
    }.to_string())
}

/// The colour map: which theme slot `bg1`, `tx1`, `bg2`, `tx2` (and the rest) stand for.
#[derive(Clone)]
pub struct ClrMap(pub HashMap<String, String>);

impl Default for ClrMap {
    fn default() -> Self {
        let pairs = [("bg1", "lt1"), ("tx1", "dk1"), ("bg2", "lt2"), ("tx2", "dk2")];
        ClrMap(pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect())
    }
}

impl ClrMap {
    pub fn from_attrs(attrs: &str) -> ClrMap {
        let mut m = ClrMap::default();
        for k in ["bg1", "tx1", "bg2", "tx2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"] {
            if let Some(v) = extract_attr(attrs, k) {
                m.0.insert(k.to_string(), v.to_string());
            }
        }
        m
    }

    /// A slide's or layout's `<p:clrMapOvr>`: an `<a:overrideClrMapping>` replaces the map.
    pub fn overridden(&self, part_xml: &str) -> ClrMap {
        next_tag(part_xml, "p:clrMapOvr")
            .and_then(|(_, inner, _)| next_tag(inner, "a:overrideClrMapping"))
            .map(|(attrs, _, _)| ClrMap::from_attrs(attrs))
            .unwrap_or_else(|| self.clone())
    }
}

/// Everything colour resolution needs.
pub struct Palette<'a> {
    pub theme: &'a Theme,
    pub map: &'a ClrMap,
}

impl Palette<'_> {
    /// A colour element (`<a:schemeClr>`, `<a:srgbClr>`, ...) with its modifiers applied.
    /// `ph` stands in for `phClr` (the colour a style reference passes into a theme fill).
    pub fn color(&self, name: &str, attrs: &str, inner: &str, ph: Option<&str>) -> Option<String> {
        let base = if name == "a:schemeClr" {
            let val = extract_attr(attrs, "val")?;
            if val == "phClr" {
                ph?.to_string()
            } else {
                let slot = self.map.0.get(val).map(String::as_str).unwrap_or(val);
                self.theme.colors.get(slot).cloned()?
            }
        } else {
            plain_color(name, attrs, inner)?
        };
        Some(apply_modifiers(&base, inner))
    }

    /// The colour inside a fill-ish element's content (`<a:solidFill>`, `<a:buClr>`, a style
    /// reference): its first colour child.
    pub fn first_color(&self, inner: &str, ph: Option<&str>) -> Option<String> {
        children(inner).into_iter().find_map(|(n, a, i)| self.color(n, a, i, ph))
    }
}

fn hex_rgb(c: &str) -> (f64, f64, f64) {
    let c = c.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(c.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f64 / 255.0;
    (p(0), p(2), p(4))
}

fn rgb_hex(r: f64, g: f64, b: f64) -> String {
    let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(r), q(g), q(b))
}

fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r { (g - b) / d + if g < b { 6.0 } else { 0.0 } } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 { t += 1.0 }
        if t > 1.0 { t -= 1.0 }
        if t < 1.0 / 6.0 { p + (q - p) * 6.0 * t } else if t < 0.5 { q } else if t < 2.0 / 3.0 { p + (q - p) * (2.0 / 3.0 - t) * 6.0 } else { p }
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// The modifiers real decks use on a theme colour: lumMod/lumOff (PowerPoint's "Lighter 40%"
/// etc.), tint (towards white), shade (towards black). Others (alpha, satMod, hue...) are ignored.
fn apply_modifiers(base: &str, inner: &str) -> String {
    let (mut r, mut g, mut b) = hex_rgb(base);
    for (name, attrs, _) in children(inner) {
        let v = num(attrs, "val").unwrap_or(100_000.0) / 100_000.0;
        match name {
            "a:lumMod" | "a:lumOff" => {
                let (h, s, l) = rgb_to_hsl(r, g, b);
                let l = if name == "a:lumMod" { l * v } else { l + v };
                (r, g, b) = hsl_to_rgb(h, s, l.clamp(0.0, 1.0));
            }
            // Tint and shade work in linear light (as Office and LibreOffice do), so a tinted navy
            // washes out to a near-neutral grey rather than staying visibly blue.
            "a:tint" => (r, g, b) = linear_map((r, g, b), |c| c + (1.0 - c) * (1.0 - v)),
            "a:shade" => (r, g, b) = linear_map((r, g, b), |c| c * v),
            _ => {}
        }
    }
    rgb_hex(r, g, b)
}

// ---------------------------------------------------------------------------------------------
// Text styles
// ---------------------------------------------------------------------------------------------

/// A colour as written (a fill's inner XML), resolved only once the palette for the slide is known.
pub type ColorXml = String;

#[derive(Default, Clone, Debug)]
pub struct RunProps {
    pub size_pt: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<ColorXml>,
    pub font: Option<String>,
    /// `baseline` in thousandths of a percent: >0 superscript, <0 subscript.
    pub baseline: Option<f64>,
}

impl RunProps {
    pub fn parse(attrs: &str, inner: &str) -> RunProps {
        let strike = extract_attr(attrs, "strike").map(|v| v != "noStrike");
        let mut p = RunProps {
            size_pt: num(attrs, "sz").map(|v| v / 100.0),
            bold: flag(attrs, "b"),
            italic: flag(attrs, "i"),
            underline: extract_attr(attrs, "u").map(|v| v != "none"),
            strike,
            baseline: num(attrs, "baseline"),
            ..Default::default()
        };
        for (name, a, i) in children(inner) {
            match name {
                "a:solidFill" => p.color = Some(i.to_string()),
                "a:latin" => p.font = extract_attr(a, "typeface").filter(|f| !f.is_empty()).map(str::to_string),
                _ => {}
            }
        }
        p
    }

    pub fn over(&self, base: &RunProps) -> RunProps {
        RunProps {
            size_pt: self.size_pt.or(base.size_pt),
            bold: self.bold.or(base.bold),
            italic: self.italic.or(base.italic),
            underline: self.underline.or(base.underline),
            strike: self.strike.or(base.strike),
            color: self.color.clone().or_else(|| base.color.clone()),
            font: self.font.clone().or_else(|| base.font.clone()),
            baseline: self.baseline.or(base.baseline),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Bullet {
    None,
    Char(String),
    AutoNum(String),
}

/// One paragraph level's properties (`<a:lvl1pPr>`, or a paragraph's own `<a:pPr>`).
#[derive(Default, Clone, Debug)]
pub struct Level {
    pub align: Option<String>,
    pub mar_l: Option<f64>,
    pub bullet: Option<Bullet>,
    pub bullet_font: Option<String>,
    pub bullet_color: Option<ColorXml>,
    /// Line spacing as a fraction of single spacing (`<a:lnSpc><a:spcPct>`).
    pub line_spacing: Option<f64>,
    pub run: RunProps,
}

impl Level {
    pub fn parse(attrs: &str, inner: &str) -> Level {
        let mut l = Level { align: extract_attr(attrs, "algn").map(str::to_string), mar_l: num(attrs, "marL"), ..Default::default() };
        for (name, a, i) in children(inner) {
            match name {
                "a:buNone" => l.bullet = Some(Bullet::None),
                "a:buChar" => l.bullet = extract_attr(a, "char").map(|c| Bullet::Char(crate::storage::xml_scan::decode_xml_entities(c))),
                "a:buAutoNum" => l.bullet = Some(Bullet::AutoNum(extract_attr(a, "type").unwrap_or("arabicPeriod").to_string())),
                "a:buFont" => l.bullet_font = extract_attr(a, "typeface").map(str::to_string),
                "a:buClr" => l.bullet_color = Some(i.to_string()),
                "a:lnSpc" => l.line_spacing = child(i, "a:spcPct").and_then(|(_, a, _)| num(a, "val")).map(|v| v / 100_000.0),
                "a:defRPr" => l.run = RunProps::parse(a, i),
                _ => {}
            }
        }
        l
    }

    pub fn over(&self, base: &Level) -> Level {
        Level {
            align: self.align.clone().or_else(|| base.align.clone()),
            mar_l: self.mar_l.or(base.mar_l),
            bullet: self.bullet.clone().or_else(|| base.bullet.clone()),
            bullet_font: self.bullet_font.clone().or_else(|| base.bullet_font.clone()),
            bullet_color: self.bullet_color.clone().or_else(|| base.bullet_color.clone()),
            line_spacing: self.line_spacing.or(base.line_spacing),
            run: self.run.over(&base.run),
        }
    }
}

/// Nine paragraph levels (`<a:lstStyle>`, `<p:titleStyle>`, `<p:defaultTextStyle>`...).
#[derive(Default, Clone, Debug)]
pub struct ListStyle(pub [Level; 9]);

impl ListStyle {
    pub fn parse(inner: &str) -> ListStyle {
        let mut s = ListStyle::default();
        let mut def: Option<Level> = None;
        for (name, a, i) in children(inner) {
            if name == "a:defPPr" {
                def = Some(Level::parse(a, i));
            } else if let Some(n) = name.strip_prefix("a:lvl").and_then(|r| r.strip_suffix("pPr")).and_then(|d| d.parse::<usize>().ok()) {
                if (1..=9).contains(&n) {
                    s.0[n - 1] = Level::parse(a, i);
                }
            }
        }
        if let Some(d) = def {
            for l in s.0.iter_mut() {
                *l = l.over(&d);
            }
        }
        s
    }

    pub fn over(&self, base: &ListStyle) -> ListStyle {
        let mut s = base.clone();
        for (i, l) in s.0.iter_mut().enumerate() {
            *l = self.0[i].over(l);
        }
        s
    }
}

/// A text box's `<a:bodyPr>`: vertical anchor, insets, and the shrink-on-overflow scale.
#[derive(Default, Clone, Debug)]
pub struct BodyProps {
    pub anchor: Option<String>,
    /// left, top, right, bottom insets in EMU.
    pub insets: [Option<f64>; 4],
    pub font_scale: Option<f64>,
    pub line_reduction: Option<f64>,
}

impl BodyProps {
    pub fn parse(attrs: &str, inner: &str) -> BodyProps {
        let mut b = BodyProps {
            anchor: extract_attr(attrs, "anchor").map(str::to_string),
            insets: [num(attrs, "lIns"), num(attrs, "tIns"), num(attrs, "rIns"), num(attrs, "bIns")],
            ..Default::default()
        };
        if let Some((_, a, _)) = child(inner, "a:normAutofit") {
            b.font_scale = Some(num(a, "fontScale").unwrap_or(100_000.0) / 100_000.0);
            b.line_reduction = Some(num(a, "lnSpcReduction").unwrap_or(0.0) / 100_000.0);
        }
        b
    }

    pub fn over(&self, base: &BodyProps) -> BodyProps {
        let mut insets = base.insets;
        for (i, v) in self.insets.iter().enumerate() {
            if v.is_some() {
                insets[i] = *v;
            }
        }
        BodyProps {
            anchor: self.anchor.clone().or_else(|| base.anchor.clone()),
            insets,
            font_scale: self.font_scale.or(base.font_scale),
            line_reduction: self.line_reduction.or(base.line_reduction),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Placeholders and the parts that hold them
// ---------------------------------------------------------------------------------------------

/// A shape's `<p:ph>`: its type (`title`, `body`, `ctrTitle`, `dt`...; none means `obj`) and index.
#[derive(Clone, Debug, Default)]
pub struct Ph {
    pub kind: String,
    pub idx: Option<String>,
}

impl Ph {
    pub fn of_shape(shape_inner: &str) -> Option<Ph> {
        let (_, nv, _) = next_tag(shape_inner, "p:nvSpPr").or_else(|| next_tag(shape_inner, "p:nvPicPr"))?;
        let (attrs, _, _) = next_tag(nv, "p:ph")?;
        Some(Ph { kind: extract_attr(attrs, "type").unwrap_or("obj").to_string(), idx: extract_attr(attrs, "idx").map(str::to_string) })
    }

    /// The family a master matches by, and whose master text style applies.
    pub fn family(&self) -> &'static str {
        match self.kind.as_str() {
            "title" | "ctrTitle" => "title",
            "dt" => "dt",
            "ftr" => "ftr",
            "sldNum" => "sldNum",
            "hdr" => "hdr",
            _ => "body",
        }
    }
}

/// A placeholder as a layout or master defines it.
#[derive(Clone, Debug, Default)]
pub struct PartPh {
    pub ph: Ph,
    /// off.x, off.y, ext.cx, ext.cy in EMU.
    pub xfrm: Option<[f64; 4]>,
    pub list: ListStyle,
    pub body: BodyProps,
}

pub fn shape_xfrm(shape_inner: &str) -> Option<[f64; 4]> {
    let (_, sppr, _) = next_tag(shape_inner, "p:spPr")?;
    let (_, xfrm, _) = next_tag(sppr, "a:xfrm")?;
    let (off, _, _) = next_tag(xfrm, "a:off")?;
    let (ext, _, _) = next_tag(xfrm, "a:ext")?;
    Some([num(off, "x")?, num(off, "y")?, num(ext, "cx")?, num(ext, "cy")?])
}

pub fn shape_text_styles(shape_inner: &str) -> (ListStyle, BodyProps) {
    let Some((_, body, _)) = next_tag(shape_inner, "p:txBody") else { return Default::default() };
    let list = child(body, "a:lstStyle").map(|(_, _, i)| ListStyle::parse(i)).unwrap_or_default();
    let props = child(body, "a:bodyPr").map(|(_, a, i)| BodyProps::parse(a, i)).unwrap_or_default();
    (list, props)
}

/// The placeholders in a layout's or master's shape tree.
pub fn part_placeholders(part_xml: &str) -> Vec<PartPh> {
    let Some((_, tree, _)) = next_tag(part_xml, "p:spTree") else { return Vec::new() };
    children(tree)
        .into_iter()
        .filter(|(n, _, _)| *n == "p:sp")
        .filter_map(|(_, _, inner)| {
            let ph = Ph::of_shape(inner)?;
            let (list, body) = shape_text_styles(inner);
            Some(PartPh { ph, xfrm: shape_xfrm(inner), list, body })
        })
        .collect()
}

/// What every slide built on one layout inherits from it and its master.
#[derive(Clone, Default)]
pub struct Inherited {
    pub layout_phs: Vec<PartPh>,
    pub master_phs: Vec<PartPh>,
    pub title_style: ListStyle,
    pub body_style: ListStyle,
    pub other_style: ListStyle,
    /// `<p:defaultTextStyle>` from presentation.xml: text boxes that aren't placeholders.
    pub default_style: ListStyle,
}

impl Inherited {
    fn layout_ph(&self, ph: &Ph) -> Option<&PartPh> {
        let by_idx = ph.idx.as_ref().and_then(|idx| self.layout_phs.iter().find(|p| p.ph.idx.as_ref() == Some(idx)));
        by_idx.or_else(|| self.layout_phs.iter().find(|p| p.ph.kind == ph.kind))
            .or_else(|| self.layout_phs.iter().find(|p| p.ph.family() == ph.family() && ph.family() != "body"))
    }

    fn master_ph(&self, ph: &Ph) -> Option<&PartPh> {
        self.master_phs.iter().find(|p| p.ph.family() == ph.family())
    }

    /// The position, paragraph levels and body properties a shape inherits, before its own.
    pub fn for_shape(&self, ph: Option<&Ph>) -> (Option<[f64; 4]>, ListStyle, BodyProps) {
        let Some(ph) = ph else { return (None, self.default_style.clone(), BodyProps::default()) };
        let base = match ph.family() {
            "title" => &self.title_style,
            "body" => &self.body_style,
            _ => &self.other_style,
        };
        let (master, layout) = (self.master_ph(ph), self.layout_ph(ph));
        let mut list = base.clone();
        let mut body = BodyProps::default();
        let mut xfrm = None;
        for p in [master, layout].into_iter().flatten() {
            list = p.list.over(&list);
            body = p.body.over(&body);
            xfrm = p.xfrm.or(xfrm);
        }
        (xfrm, list, body)
    }
}

/// Reads a `<p:txStyles>` entry from a master.
pub fn master_text_style(master_xml: &str, tag: &str) -> ListStyle {
    next_tag(master_xml, "p:txStyles").and_then(|(_, tx, _)| child(tx, tag)).map(|(_, _, i)| ListStyle::parse(i)).unwrap_or_default()
}

/// The visible text of an automatic bullet number: `type` per ECMA-376 `ST_TextAutonumberScheme`.
pub fn autonum_label(kind: &str, n: usize) -> String {
    fn roman(mut n: usize) -> String {
        let table = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
        let mut s = String::new();
        for (v, r) in table {
            while n >= v {
                s.push_str(r);
                n -= v;
            }
        }
        s
    }
    let alpha = |n: usize| ((b'a' + ((n - 1) % 26) as u8) as char).to_string();
    let body = if kind.starts_with("alphaLc") {
        alpha(n)
    } else if kind.starts_with("alphaUc") {
        alpha(n).to_uppercase()
    } else if kind.starts_with("romanLc") {
        roman(n)
    } else if kind.starts_with("romanUc") {
        roman(n).to_uppercase()
    } else {
        n.to_string()
    };
    if kind.ends_with("ParenBoth") {
        format!("({body})")
    } else if kind.ends_with("ParenR") {
        format!("{body})")
    } else if kind.ends_with("Plain") {
        body
    } else {
        format!("{body}.")
    }
}

/// A bullet character as it should look: symbol fonts (Wingdings, Symbol) put their glyphs on
/// ordinary letters, so `§` in Wingdings is a square, not a section sign.
pub fn bullet_glyph(ch: &str, font: Option<&str>) -> String {
    let font = font.unwrap_or("").to_ascii_lowercase();
    if font.contains("wingdings") {
        return match ch {
            "§" | "n" => "▪", "l" => "●", "Ø" => "➢", "ü" => "✓", "q" | "o" => "❑", "v" => "❖", "ð" | "è" => "➔", "Ÿ" => "•", "w" => "◆",
            _ => "•",
        }.to_string();
    }
    if font.contains("symbol") {
        return match ch { "·" | "\u{f0b7}" => "•", "-" => "–", _ => "•" }.to_string();
    }
    match ch {
        "" => "•".to_string(),
        c if c.chars().all(|c| ('\u{f000}'..='\u{f0ff}').contains(&c)) => "•".to_string(), // private-use symbol-font code points
        c => c.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_walks_one_level_and_copes_with_nesting_and_prefix_names() {
        let xml = r#"<a:p><a:pPr lvl="1"/><a:r><a:t>x</a:t></a:r></a:p><a:br/><p:grpSp><p:grpSp>in</p:grpSp></p:grpSp>"#;
        let names: Vec<&str> = children(xml).iter().map(|c| c.0).collect();
        assert_eq!(names, ["a:p", "a:br", "p:grpSp"]);
        assert_eq!(children(xml)[2].2, "<p:grpSp>in</p:grpSp>");
        assert_eq!(children(children(xml)[0].2).iter().map(|c| c.0).collect::<Vec<_>>(), ["a:pPr", "a:r"]);
    }

    #[test]
    fn many_look_alike_tags_stay_linear() {
        let xml = format!("<a:p>{}</a:p>", "<a:pPr/></a:p><a:p>".repeat(20_000));
        let start = std::time::Instant::now();
        assert_eq!(children(&xml).len(), 20_001);
        assert!(start.elapsed() < std::time::Duration::from_secs(2), "{:?}", start.elapsed());
        // Deep same-name nesting: many opens before the first close.
        let deep = format!("{}{}", "<p:grpSp>".repeat(20_000), "</p:grpSp>".repeat(20_000));
        let start = std::time::Instant::now();
        assert_eq!(children(&deep).len(), 1);
        assert!(start.elapsed() < std::time::Duration::from_secs(2), "{:?}", start.elapsed());
    }

    #[test]
    fn scheme_colours_go_through_the_colour_map_and_modifiers() {
        let theme = Theme::parse(r#"<a:clrScheme><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:accent1><a:srgbClr val="4F81BD"/></a:accent1></a:clrScheme>"#, "t");
        let map = ClrMap::default();
        let pal = Palette { theme: &theme, map: &map };
        assert_eq!(pal.color("a:schemeClr", r#"val="tx1""#, "", None).as_deref(), Some("#000000"));
        assert_eq!(pal.color("a:schemeClr", r#"val="bg1""#, "", None).as_deref(), Some("#ffffff"));
        // "Darker 50%" of white is mid grey; tint 50% of black is mid grey too.
        assert_eq!(pal.color("a:schemeClr", r#"val="bg1""#, r#"<a:lumMod val="50000"/>"#, None).as_deref(), Some("#808080"));
        // Tint works in linear light: half-way to white from black is a light grey, not #808080.
        assert_eq!(pal.color("a:srgbClr", r#"val="000000""#, r#"<a:tint val="50000"/>"#, None).as_deref(), Some("#bcbcbc"));
        let dark = ClrMap::from_attrs(r#"bg1="dk1" tx1="lt1""#);
        assert_eq!(Palette { theme: &theme, map: &dark }.color("a:schemeClr", r#"val="tx1""#, "", None).as_deref(), Some("#ffffff"));
    }

    #[test]
    fn list_styles_cascade_level_by_level() {
        let master = ListStyle::parse(r#"<a:lvl1pPr algn="l"><a:buChar char="•"/><a:defRPr sz="3200"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:defRPr></a:lvl1pPr><a:lvl2pPr><a:defRPr sz="2800"/></a:lvl2pPr>"#);
        let layout = ListStyle::parse(r#"<a:lvl1pPr><a:buNone/><a:defRPr sz="2400"/></a:lvl1pPr>"#);
        let s = layout.over(&master);
        assert_eq!(s.0[0].run.size_pt, Some(24.0));
        assert_eq!(s.0[0].align.as_deref(), Some("l"));
        assert_eq!(s.0[0].bullet, Some(Bullet::None));
        assert!(s.0[0].run.color.is_some());
        assert_eq!(s.0[1].run.size_pt, Some(28.0));
    }

    #[test]
    fn placeholders_match_by_index_then_type() {
        let ph = |kind: &str, idx: Option<&str>| Ph { kind: kind.into(), idx: idx.map(str::to_string) };
        let part = |kind: &str, idx: Option<&str>, x: f64| PartPh { ph: ph(kind, idx), xfrm: Some([x, 0.0, 1.0, 1.0]), ..Default::default() };
        let inh = Inherited {
            layout_phs: vec![part("ctrTitle", None, 1.0), part("subTitle", Some("1"), 2.0)],
            master_phs: vec![part("title", None, 10.0), part("body", Some("1"), 20.0)],
            ..Default::default()
        };
        assert_eq!(inh.for_shape(Some(&ph("ctrTitle", None))).0.unwrap()[0], 1.0);
        assert_eq!(inh.for_shape(Some(&ph("subTitle", Some("1")))).0.unwrap()[0], 2.0);
        // Not on the layout: the master's placeholder of the same family.
        assert_eq!(inh.for_shape(Some(&ph("body", Some("7")))).0.unwrap()[0], 20.0);
    }

    #[test]
    fn bullets_and_numbers_look_right() {
        assert_eq!(bullet_glyph("§", Some("Wingdings")), "▪");
        assert_eq!(bullet_glyph("–", Some("Arial")), "–");
        assert_eq!(autonum_label("arabicPeriod", 3), "3.");
        assert_eq!(autonum_label("alphaLcParenR", 2), "b)");
        assert_eq!(autonum_label("romanUcPeriod", 4), "IV.");
    }
}
