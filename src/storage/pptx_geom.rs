//! DrawingML shape geometry: the 187 preset shapes (`<a:prstGeom>`, defined in ECMA-376's
//! `presetShapeDefinitions.xml`, bundled compacted as `pptx_presets.xml`) and custom geometry
//! (`<a:custGeom>`), evaluated for one shape's size into SVG path data in its own 0..100 box.
//!
//! Both are the same language: adjust values (`avLst`, which a shape may override), guides
//! (`gdLst`, formulas over the shape's size and earlier guides), a text rectangle (`rect`) and
//! paths (`pathLst`) whose points name guides or numbers. Angles are in 60,000ths of a degree.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::storage::pptx_inherit::children;
use crate::storage::xml_scan::extract_attr;

static PRESETS_XML: &str = include_str!("pptx_presets.xml");

/// Preset name -> its definition's inner XML.
fn presets() -> &'static HashMap<&'static str, &'static str> {
    static P: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    P.get_or_init(|| {
        let root = children(PRESETS_XML).into_iter().find(|(n, _, _)| *n == "presetShapeDefinitons").map(|(_, _, i)| i).unwrap_or("");
        children(root).into_iter().map(|(n, _, i)| (n, i)).collect()
    })
}

pub fn is_preset(name: &str) -> bool {
    presets().contains_key(name)
}

/// A preset's adjust values' names, in the order its definition lists them (the order other
/// formats, such as ODF's `draw:modifiers`, give their values in).
pub fn adjust_names(name: &str) -> Vec<&'static str> {
    let Some(def) = presets().get(name) else { return Vec::new() };
    kid(def, "avLst").map(|(_, av)| children(av).into_iter().filter_map(|(n, a, _)| (local(n) == "gd").then(|| extract_attr(a, "name")).flatten()).collect()).unwrap_or_default()
}

/// How a path is painted: DrawingML lets one shape shade some of its parts (a can's lid, a
/// bevel's sides) darker or lighter than its fill, or leave them unfilled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillMode {
    None,
    Norm,
    Lighten,
    LightenLess,
    Darken,
    DarkenLess,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeomPath {
    /// SVG path data in the shape's 0..100 box.
    pub d: String,
    /// The same outline flattened to points (curves and arcs sampled), one list per subpath.
    pub points: Vec<Vec<(f64, f64)>>,
    pub fill: FillMode,
    pub stroke: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Geometry {
    pub paths: Vec<GeomPath>,
    /// Where text goes, as l, t, r, b in the shape's own EMU (unflipped).
    pub text_rect: Option<[f64; 4]>,
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn kid<'a>(xml: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
    children(xml).into_iter().find(|(n, _, _)| local(n) == name).map(|(_, a, i)| (a, i))
}

/// Guide values by name, starting from the built-in ones a shape's size defines.
struct Guides(HashMap<String, f64>);

impl Guides {
    fn new(w: f64, h: f64) -> Guides {
        let (ss, ls) = (w.min(h), w.max(h));
        let mut m = HashMap::new();
        let mut set = |k: &str, v: f64| {
            m.insert(k.to_string(), v);
        };
        for (k, v) in [("w", w), ("h", h), ("l", 0.0), ("t", 0.0), ("r", w), ("b", h), ("hc", w / 2.0), ("vc", h / 2.0), ("ss", ss), ("ls", ls)] {
            set(k, v);
        }
        for d in [2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 16.0, 32.0] {
            set(&format!("wd{d}"), w / d);
            set(&format!("hd{d}"), h / d);
            set(&format!("ssd{d}"), ss / d);
        }
        for (k, v) in [("cd2", 10_800_000.0), ("cd4", 5_400_000.0), ("cd8", 2_700_000.0), ("3cd4", 16_200_000.0), ("3cd8", 8_100_000.0), ("5cd8", 13_500_000.0), ("7cd8", 18_900_000.0)] {
            set(k, v);
        }
        Guides(m)
    }

    fn get(&self, tok: &str) -> f64 {
        tok.parse::<f64>().ok().or_else(|| self.0.get(tok).copied()).unwrap_or(0.0)
    }

    fn eval(&self, fmla: &str) -> f64 {
        let mut it = fmla.split_whitespace();
        let op = it.next().unwrap_or("");
        let a: Vec<f64> = it.map(|t| self.get(t)).collect();
        let x = |i: usize| a.get(i).copied().unwrap_or(0.0);
        let rad = |v: f64| (v / 60_000.0).to_radians();
        let v = match op {
            "val" => x(0),
            "*/" => if x(2) == 0.0 { 0.0 } else { x(0) * x(1) / x(2) },
            "+-" => x(0) + x(1) - x(2),
            "+/" => if x(2) == 0.0 { 0.0 } else { (x(0) + x(1)) / x(2) },
            "?:" => if x(0) > 0.0 { x(1) } else { x(2) },
            "abs" => x(0).abs(),
            "at2" => x(1).atan2(x(0)).to_degrees() * 60_000.0,
            "cat2" => x(0) * x(2).atan2(x(1)).cos(),
            "sat2" => x(0) * x(2).atan2(x(1)).sin(),
            "cos" => x(0) * rad(x(1)).cos(),
            "sin" => x(0) * rad(x(1)).sin(),
            "tan" => x(0) * rad(x(1)).tan(),
            "max" => x(0).max(x(1)),
            "min" => x(0).min(x(1)),
            "mod" => (x(0) * x(0) + x(1) * x(1) + x(2) * x(2)).sqrt(),
            "pin" => if x(1) < x(0) { x(0) } else if x(1) > x(2) { x(2) } else { x(1) },
            "sqrt" => x(0).max(0.0).sqrt(),
            _ => 0.0,
        };
        if v.is_finite() { v } else { 0.0 }
    }

    fn define(&mut self, list: &str) {
        for (n, a, _) in children(list) {
            if local(n) != "gd" {
                continue;
            }
            if let (Some(name), Some(f)) = (extract_attr(a, "name"), extract_attr(a, "fmla")) {
                let v = self.eval(f);
                self.0.insert(name.to_string(), v);
            }
        }
    }
}

/// A preset's geometry for a `w` x `h` (EMU) shape; `av_overrides` is the shape's own `<a:avLst>`
/// contents. `None` for an unknown preset.
pub fn preset(name: &str, av_overrides: &str, w: f64, h: f64, flip: (bool, bool)) -> Option<Geometry> {
    let def = presets().get(name)?;
    Some(evaluate(def, Some(av_overrides), w, h, flip))
}

/// A `<a:custGeom>`'s geometry for a `w` x `h` (EMU) shape.
pub fn custom(cust: &str, w: f64, h: f64, flip: (bool, bool)) -> Geometry {
    evaluate(cust, None, w, h, flip)
}

fn evaluate(def: &str, av_overrides: Option<&str>, w: f64, h: f64, flip: (bool, bool)) -> Geometry {
    let mut g = Guides::new(w, h);
    if let Some((_, av)) = kid(def, "avLst") {
        g.define(av);
    }
    if let Some(over) = av_overrides {
        g.define(over);
    }
    if let Some((_, gd)) = kid(def, "gdLst") {
        g.define(gd);
    }
    let text_rect = kid(def, "rect").map(|(a, _)| {
        let v = |k: &str| extract_attr(a, k).map(|t| g.get(t)).unwrap_or(0.0);
        [v("l"), v("t"), v("r"), v("b")]
    });
    let paths = kid(def, "pathLst").map(|(_, lst)| {
        children(lst).into_iter().filter(|(n, _, _)| local(n) == "path").filter_map(|(_, a, i)| path(&g, a, i, w, h, flip)).collect()
    }).unwrap_or_default();
    Geometry { paths, text_rect }
}

/// Formats a coordinate the way the rest of the importer's path data does: at most 2 decimals.
fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

fn path(g: &Guides, attrs: &str, inner: &str, w: f64, h: f64, flip: (bool, bool)) -> Option<GeomPath> {
    // A path may have its own coordinate space (`w`/`h`), stretched over the shape.
    let space = |k: &str| extract_attr(attrs, k).and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0);
    let (sx, sy) = (space("w").map_or(1.0, |pw| w / pw), space("h").map_or(1.0, |ph| h / ph));
    let (bw, bh) = (if w > 0.0 { w } else { 1.0 }, if h > 0.0 { h } else { 1.0 });
    let unit = |(x, y): (f64, f64)| {
        let (mut x, mut y) = (x / bw * 100.0, y / bh * 100.0);
        if flip.0 { x = 100.0 - x }
        if flip.1 { y = 100.0 - y }
        (x, y)
    };
    let norm = |p: (f64, f64)| {
        let (x, y) = unit(p);
        format!("{} {}", num(x), num(y))
    };
    let mut points: Vec<Vec<(f64, f64)>> = Vec::new();
    // Every point a command reaches, flattened (bounded: a path is a few dozen commands).
    let mut add = |p: (f64, f64), new: bool| {
        if new || points.is_empty() {
            points.push(Vec::new());
        }
        if let Some(last) = points.last_mut() {
            last.push(unit(p));
        }
    };
    let pt = |a: &str| -> Option<(f64, f64)> { Some((g.get(extract_attr(a, "x")?) * sx, g.get(extract_attr(a, "y")?) * sy)) };
    let pts = |i: &str| -> Vec<(f64, f64)> { children(i).into_iter().filter(|(n, _, _)| local(n) == "pt").filter_map(|(_, a, _)| pt(a)).collect() };

    let mut d: Vec<String> = Vec::new();
    let mut cur = (0.0, 0.0);
    for (cmd, a, i) in children(inner) {
        match local(cmd) {
            c @ ("moveTo" | "lnTo" | "cubicBezTo" | "quadBezTo") => {
                let p = pts(i);
                let (letter, n) = match c { "moveTo" => ('M', 1), "lnTo" => ('L', 1), "cubicBezTo" => ('C', 3), _ => ('Q', 2) };
                if p.len() < n {
                    continue;
                }
                match c {
                    "moveTo" => add(p[0], true),
                    "lnTo" => add(p[0], false),
                    _ => {
                        // Sampled along the curve (Bézier in Bernstein form; a quadratic has one control point).
                        for k in 1..=8 {
                            let t = k as f64 / 8.0;
                            let u = 1.0 - t;
                            let q = if n == 3 {
                                let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                                (w[0] * cur.0 + w[1] * p[0].0 + w[2] * p[1].0 + w[3] * p[2].0, w[0] * cur.1 + w[1] * p[0].1 + w[2] * p[1].1 + w[3] * p[2].1)
                            } else {
                                let w = [u * u, 2.0 * u * t, t * t];
                                (w[0] * cur.0 + w[1] * p[0].0 + w[2] * p[1].0, w[0] * cur.1 + w[1] * p[0].1 + w[2] * p[1].1)
                            };
                            add(q, false);
                        }
                    }
                }
                cur = p[n - 1];
                d.push(format!("{letter} {}", p[..n].iter().map(|q| norm(*q)).collect::<Vec<_>>().join(" ")));
            }
            "arcTo" => {
                let v = |k: &str| extract_attr(a, k).map(|t| g.get(t)).unwrap_or(0.0);
                let (path_wr, path_hr) = (v("wR"), v("hR"));
                let (wr, hr) = (path_wr * sx, path_hr * sy);
                let (st, sw) = (v("stAng") / 60_000.0, v("swAng") / 60_000.0);
                if wr <= 0.0 || hr <= 0.0 || sw == 0.0 {
                    continue;
                }
                // DrawingML's angles are where a ray from the centre meets the ellipse, not the
                // ellipse's parameter: convert before placing points on it -- in the path's own
                // coordinates (the spec's), so a path stretched into its shape keeps its arcs.
                let on = |deg: f64| {
                    let r = deg.to_radians();
                    let t = (r.sin() * path_wr).atan2(r.cos() * path_hr);
                    (wr * t.cos(), hr * t.sin())
                };
                let s = on(st);
                let c = (cur.0 - s.0, cur.1 - s.1);
                // SVG can't draw a full turn as one arc: split into steps of at most 180 degrees.
                let steps = (sw.abs() / 180.0).ceil().max(1.0) as usize;
                let rx = num(wr / bw * 100.0);
                let ry = num(hr / bh * 100.0);
                let sweep = (sw > 0.0) != (flip.0 != flip.1);
                for k in 1..=(steps * 12) {
                    let e = on(st + sw * k as f64 / (steps * 12) as f64);
                    add((c.0 + e.0, c.1 + e.1), false);
                }
                for k in 1..=steps {
                    let e = on(st + sw * k as f64 / steps as f64);
                    let end = (c.0 + e.0, c.1 + e.1);
                    d.push(format!("A {rx} {ry} 0 0 {} {}", sweep as u8, norm(end)));
                    cur = end;
                }
            }
            "close" => d.push("Z".to_string()),
            _ => {}
        }
    }
    if d.is_empty() {
        return None;
    }
    let fill = match extract_attr(attrs, "fill") {
        Some("none") => FillMode::None,
        Some("lighten") => FillMode::Lighten,
        Some("lightenLess") => FillMode::LightenLess,
        Some("darken") => FillMode::Darken,
        Some("darkenLess") => FillMode::DarkenLess,
        _ => FillMode::Norm,
    };
    let stroke = !matches!(extract_attr(attrs, "stroke"), Some("0" | "false"));
    Some(GeomPath { d: d.join(" "), points, fill, stroke })
}

/// The geometry's outline as a CSS `polygon(...)` in percent of its box, when it is one filled
/// outline (a donut's hole, or several pieces, can't be one polygon).
pub fn clip_polygon(g: &Geometry) -> Option<String> {
    let mut filled = g.paths.iter().filter(|p| p.fill != FillMode::None);
    let path = filled.next()?;
    if filled.next().is_some() || path.points.len() != 1 || path.points[0].len() < 3 {
        return None;
    }
    let pts: Vec<String> = path.points[0].iter().map(|(x, y)| format!("{}% {}%", num(*x), num(*y))).collect();
    Some(format!("polygon({})", pts.join(", ")))
}

/// A `#rrggbb` (or `#rrggbbaa`) fill shaded the way a path's fill mode asks; a gradient is left as is.
pub fn shade(color: &str, mode: FillMode) -> String {
    let (toward, k) = match mode {
        FillMode::Darken => (0.0, 0.4),
        FillMode::DarkenLess => (0.0, 0.2),
        FillMode::Lighten => (255.0, 0.4),
        FillMode::LightenLess => (255.0, 0.2),
        _ => return color.to_string(),
    };
    let hex = color.trim_start_matches('#');
    // Hex digits only: the colour can come straight from a file, and slicing a non-ASCII string by bytes
    // (`#ééé` is six bytes) would split a character and panic.
    if !color.starts_with('#') || (hex.len() != 6 && hex.len() != 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return color.to_string();
    }
    let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map(|v| v as f64).unwrap_or(0.0);
    let m = |v: f64| (v + (toward - v) * k).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}{}", m(c(0)), m(c(2)), m(c(4)), &hex[6..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_evaluates_to_drawable_paths() {
        assert_eq!(presets().len(), 187);
        for name in presets().keys() {
            let g = preset(name, "", 1_000_000.0, 600_000.0, (false, false)).unwrap();
            assert!(!g.paths.is_empty(), "{name} has no paths");
            for p in &g.paths {
                assert!(!p.d.contains("NaN") && !p.d.contains("inf"), "{name}: {}", p.d);
            }
        }
    }

    #[test]
    fn guides_and_adjust_overrides_shape_a_rounded_rectangle() {
        // roundRect's corner is adj (default 16667) of the shorter side.
        let g = preset("roundRect", "", 1_000.0, 500.0, (false, false)).unwrap();
        assert!(g.paths[0].d.starts_with("M 0 16.67"), "{}", g.paths[0].d);
        let g = preset("roundRect", r#"<a:gd name="adj" fmla="val 50000"/>"#, 1_000.0, 500.0, (false, false)).unwrap();
        assert!(g.paths[0].d.starts_with("M 0 50"), "{}", g.paths[0].d);
    }

    #[test]
    fn arcs_use_visual_angles_and_full_turns_split() {
        // An ellipse is four quarter arcs (or a full one split) closing on its start.
        let g = preset("ellipse", "", 200.0, 100.0, (false, false)).unwrap();
        let d = &g.paths[0].d;
        assert!(d.starts_with("M 0 50"), "{d}");
        assert!(d.contains("A 50 50 0 0 1"), "{d}");
        // The text box of an ellipse is inset to the inscribed rectangle.
        let r = g.text_rect.unwrap();
        assert!((r[0] - 29.29).abs() < 0.1 && (r[2] - 170.71).abs() < 0.1, "{r:?}");
    }

    #[test]
    fn shaded_parts_keep_their_own_fill_modes() {
        let g = preset("can", "", 1_000.0, 1_000.0, (false, false)).unwrap();
        let modes: Vec<_> = g.paths.iter().map(|p| (p.fill, p.stroke)).collect();
        assert!(modes.contains(&(FillMode::Lighten, false)), "{modes:?}");
        assert!(modes.contains(&(FillMode::None, true)), "{modes:?}");
        assert_eq!(shade("#808080", FillMode::Darken), "#4d4d4d");
        // Not a colour this can shade: given back as it came, never a panic.
        assert_eq!(shade("#ééé", FillMode::Darken), "#ééé");
        assert_eq!(shade("#zzzzzz", FillMode::Lighten), "#zzzzzz");
    }
}
