//! OpenDocument shape geometry (ODF 1.3, `draw:enhanced-geometry`, and SVG path data), evaluated
//! for one shape's size into the same path model as DrawingML's (`pptx_geom::Geometry`): SVG path
//! data in the shape's own 0..100 box, with each part's fill/stroke flags and the text area.
//!
//! An enhanced geometry is a coordinate space (`svg:viewBox`), adjust values (`draw:modifiers`,
//! `$0`, `$1`...), equations (`draw:equation draw:formula`, `?f0`...) and a path
//! (`draw:enhanced-path`). LibreOffice writes shapes that came from PowerPoint as
//! `draw:type="ooxml-<preset>"` with the preset's own adjust values: those are drawn by the exact
//! DrawingML preset instead (`pptx_geom::preset`).

use std::collections::HashMap;

use crate::storage::pptx_geom::{self, FillMode, GeomPath, Geometry};
use crate::storage::pptx_inherit::children;
use crate::storage::xml_scan::extract_attr;

/// Where formulas and path parameters get their values.
struct Env {
    modifiers: Vec<f64>,
    equations: Vec<String>,
    values: HashMap<usize, f64>,
    /// left, top, right, bottom, width, height of the view box; the shape's own size in 1/100 mm.
    view: [f64; 4],
    log: (f64, f64),
    depth: usize,
}

impl Env {
    fn keyword(&self, k: &str) -> Option<f64> {
        let [l, t, w, h] = self.view;
        Some(match k {
            "left" => l,
            "top" => t,
            "right" => l + w,
            "bottom" => t + h,
            "width" => w,
            "height" => h,
            "logwidth" => self.log.0,
            "logheight" => self.log.1,
            "pi" => std::f64::consts::PI,
            "hasstroke" | "hasfill" => 1.0,
            "xstretch" | "ystretch" => 0.0,
            _ => return None,
        })
    }

    fn equation(&mut self, i: usize) -> f64 {
        if let Some(v) = self.values.get(&i) {
            return *v;
        }
        // A formula naming itself (directly or round a loop) is 0, not a stack overflow.
        if self.depth > 64 || i >= self.equations.len() {
            return 0.0;
        }
        self.depth += 1;
        self.values.insert(i, 0.0);
        let f = self.equations[i].clone();
        let v = Parser { s: f.as_bytes(), at: 0, env: self }.expr();
        let v = if v.is_finite() { v } else { 0.0 };
        self.values.insert(i, v);
        self.depth -= 1;
        v
    }

    /// A path or text-area parameter: a number, `?fN`, `$N` or a keyword.
    fn param(&mut self, tok: &str) -> f64 {
        if let Some(n) = tok.strip_prefix("?f").and_then(|n| n.parse::<usize>().ok()) {
            self.equation(n)
        } else if let Some(n) = tok.strip_prefix('$').and_then(|n| n.parse::<usize>().ok()) {
            self.modifiers.get(n).copied().unwrap_or(0.0)
        } else {
            tok.parse::<f64>().ok().or_else(|| self.keyword(tok)).unwrap_or(0.0)
        }
    }
}

/// A recursive-descent reader of ODF formulas: + - * /, unary minus, parentheses, numbers,
/// `?fN`, `$N`, keywords, and the functions abs sqrt sin cos tan atan atan2 min max if.
struct Parser<'a, 'e> {
    s: &'a [u8],
    at: usize,
    env: &'e mut Env,
}

impl Parser<'_, '_> {
    fn ws(&mut self) {
        while self.s.get(self.at).is_some_and(|c| c.is_ascii_whitespace()) {
            self.at += 1;
        }
    }
    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.s.get(self.at) == Some(&c) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> f64 {
        let mut v = self.term();
        loop {
            if self.eat(b'+') {
                v += self.term();
            } else if self.eat(b'-') {
                v -= self.term();
            } else {
                return v;
            }
        }
    }
    fn term(&mut self) -> f64 {
        let mut v = self.unary();
        loop {
            if self.eat(b'*') {
                v *= self.unary();
            } else if self.eat(b'/') {
                let d = self.unary();
                v = if d == 0.0 { 0.0 } else { v / d };
            } else {
                return v;
            }
        }
    }
    fn unary(&mut self) -> f64 {
        if self.eat(b'-') {
            -self.unary()
        } else if self.eat(b'+') {
            self.unary()
        } else {
            self.atom()
        }
    }
    fn args(&mut self) -> Vec<f64> {
        let mut out = Vec::new();
        if !self.eat(b'(') {
            return out;
        }
        if self.eat(b')') {
            return out;
        }
        loop {
            out.push(self.expr());
            if self.eat(b',') {
                continue;
            }
            self.eat(b')');
            return out;
        }
    }
    fn atom(&mut self) -> f64 {
        self.ws();
        if self.eat(b'(') {
            let v = self.expr();
            self.eat(b')');
            return v;
        }
        let start = self.at;
        while self.s.get(self.at).is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'?' | b'$' | b'_')) {
            self.at += 1;
        }
        let tok = std::str::from_utf8(&self.s[start..self.at]).unwrap_or("");
        if tok.is_empty() {
            // Not something this reads: skip a byte so a malformed formula always ends.
            self.at += 1;
            return 0.0;
        }
        let a = |v: &Vec<f64>, i: usize| v.get(i).copied().unwrap_or(0.0);
        match tok {
            "abs" | "sqrt" | "sin" | "cos" | "tan" | "atan" | "atan2" | "min" | "max" | "if" => {
                let v = self.args();
                match tok {
                    "abs" => a(&v, 0).abs(),
                    "sqrt" => a(&v, 0).max(0.0).sqrt(),
                    "sin" => a(&v, 0).sin(),
                    "cos" => a(&v, 0).cos(),
                    "tan" => a(&v, 0).tan(),
                    "atan" => a(&v, 0).atan(),
                    "atan2" => a(&v, 0).atan2(a(&v, 1)),
                    "min" => a(&v, 0).min(a(&v, 1)),
                    "max" => a(&v, 0).max(a(&v, 1)),
                    _ => if a(&v, 0) > 0.0 { a(&v, 1) } else { a(&v, 2) },
                }
            }
            _ => self.env.param(tok),
        }
    }
}

/// The path parameters as tokens: commands are single letters, everything else a value.
fn tokens(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in path.chars() {
        if c.is_whitespace() || c == ',' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else if c.is_ascii_alphabetic() && cur.is_empty() {
            // A command letter on its own, or the start of a keyword ("left", "logwidth").
            cur.push(c);
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn is_command(t: &str) -> bool {
    t.len() == 1 && "MLCZNFSTUABWVXYQG".contains(t)
}

/// Formats a coordinate as the importers' path data does: at most 2 decimals.
fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

/// Builds one path's data in the 0..100 box from points in shape units (`scale` maps them).
struct Builder {
    d: Vec<String>,
    points: Vec<Vec<(f64, f64)>>,
    cur: (f64, f64),
    sx: f64,
    sy: f64,
    flip: (bool, bool),
}

impl Builder {
    fn unit(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let (mut x, mut y) = (x * self.sx, y * self.sy);
        if self.flip.0 { x = 100.0 - x }
        if self.flip.1 { y = 100.0 - y }
        (x, y)
    }
    fn fmt(&self, p: (f64, f64)) -> String {
        let (x, y) = self.unit(p);
        format!("{} {}", num(x), num(y))
    }
    fn mark(&mut self, p: (f64, f64), new: bool) {
        let u = self.unit(p);
        if new || self.points.is_empty() {
            self.points.push(Vec::new());
        }
        if let Some(last) = self.points.last_mut() {
            last.push(u);
        }
    }
    fn move_to(&mut self, p: (f64, f64)) {
        self.d.push(format!("M {}", self.fmt(p)));
        self.mark(p, true);
        self.cur = p;
    }
    fn line_to(&mut self, p: (f64, f64)) {
        if self.d.is_empty() {
            return self.move_to(p);
        }
        self.d.push(format!("L {}", self.fmt(p)));
        self.mark(p, false);
        self.cur = p;
    }
    fn cubic(&mut self, c1: (f64, f64), c2: (f64, f64), p: (f64, f64)) {
        if self.d.is_empty() {
            self.move_to(self.cur);
        }
        let p0 = self.cur;
        self.d.push(format!("C {} {} {}", self.fmt(c1), self.fmt(c2), self.fmt(p)));
        for k in 1..=8 {
            let t = k as f64 / 8.0;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            self.mark((w[0] * p0.0 + w[1] * c1.0 + w[2] * c2.0 + w[3] * p.0, w[0] * p0.1 + w[1] * c1.1 + w[2] * c2.1 + w[3] * p.1), false);
        }
        self.cur = p;
    }
    /// An elliptical arc around `c` with radii `r`, from parametric angle `a0` through `sweep`
    /// radians (positive: clockwise on screen), as cubic Béziers (exact enough, and flips cleanly).
    fn arc(&mut self, c: (f64, f64), r: (f64, f64), a0: f64, sweep: f64, connect: bool) {
        let at = |a: f64| (c.0 + r.0 * a.cos(), c.1 + r.1 * a.sin());
        let start = at(a0);
        if connect && !self.d.is_empty() {
            self.line_to(start);
        } else {
            self.move_to(start);
        }
        let n = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
        let step = sweep / n as f64;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        for i in 0..n {
            let (s, e) = (a0 + step * i as f64, a0 + step * (i + 1) as f64);
            let (p0, p3) = (at(s), at(e));
            let c1 = (p0.0 - k * r.0 * s.sin(), p0.1 + k * r.1 * s.cos());
            let c2 = (p3.0 + k * r.0 * e.sin(), p3.1 - k * r.1 * e.cos());
            self.cubic(c1, c2, p3);
        }
    }
    fn close(&mut self) {
        if !self.d.is_empty() {
            self.d.push("Z".into());
        }
    }
}

/// The parametric angle (radians) on an ellipse of radii (wr, hr) for a visual angle in degrees,
/// as ODF (and DrawingML) measure angles: where a ray from the centre at that angle meets it.
fn visual_to_param(wr: f64, hr: f64, deg: f64) -> f64 {
    let r = deg.to_radians();
    (r.sin() * wr).atan2(r.cos() * hr)
}

/// An enhanced geometry (`<draw:enhanced-geometry>`'s attributes and contents) for a shape `w` x
/// `h` (EMU). `None` when it states no path (nor a preset this reads).
pub fn enhanced(attrs: &str, inner: &str, w: f64, h: f64) -> Option<Geometry> {
    let flip = (extract_attr(attrs, "draw:mirror-horizontal") == Some("true"), extract_attr(attrs, "draw:mirror-vertical") == Some("true"));
    let modifiers: Vec<f64> = extract_attr(attrs, "draw:modifiers").map(|m| m.split_whitespace().filter_map(|v| v.parse().ok()).collect()).unwrap_or_default();
    // A PowerPoint shape LibreOffice carried over: its preset, exactly, with its adjust values.
    if let Some(prst) = extract_attr(attrs, "draw:type").and_then(|t| t.strip_prefix("ooxml-")) {
        if pptx_geom::is_preset(prst) {
            let av: String = pptx_geom::adjust_names(prst).iter().zip(&modifiers).map(|(n, v)| format!(r#"<a:gd name="{n}" fmla="val {v}"/>"#)).collect();
            return pptx_geom::preset(prst, &av, w, h, flip);
        }
    }
    let path = extract_attr(attrs, "drawooo:enhanced-path").or_else(|| extract_attr(attrs, "draw:enhanced-path"))?;
    let vb: Vec<f64> = extract_attr(attrs, "svg:viewBox").map(|v| v.split_whitespace().filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
    // Shape size in 1/100 mm (EMU / 360).
    let log = (w / 360.0, h / 360.0);
    let sub: Vec<f64> = extract_attr(attrs, "drawooo:sub-view-size").map(|v| v.split_whitespace().filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
    let view = match (vb.as_slice(), sub.as_slice()) {
        ([l, t, vw, vh], _) if *vw > 0.0 && *vh > 0.0 => [*l, *t, *vw, *vh],
        // A PowerPoint freeform: its path's own coordinate size.
        (_, [sw, sh]) if *sw > 0.0 && *sh > 0.0 => [0.0, 0.0, *sw, *sh],
        // No view box (LibreOffice's other OOXML shapes): coordinates are the shape's own 1/100 mm.
        _ => [0.0, 0.0, log.0.max(1.0), log.1.max(1.0)],
    };
    let equations: Vec<String> = children(inner).into_iter().filter(|(n, _, _)| *n == "draw:equation").map(|(_, a, _)| crate::storage::xml_scan::decode_xml_entities(extract_attr(a, "draw:formula").unwrap_or("0"))).collect();
    let mut env = Env { modifiers, equations, values: HashMap::new(), view, log, depth: 0 };
    let (sx, sy) = (100.0 / view[2], 100.0 / view[3]);
    let origin = (view[0], view[1]);

    let toks = tokens(&crate::storage::xml_scan::decode_xml_entities(path));
    let mut paths = Vec::new();
    let mut b = Builder { d: Vec::new(), points: Vec::new(), cur: (0.0, 0.0), sx, sy, flip };
    let (mut fill, mut stroke) = (true, true);
    let mut finish = |b: &mut Builder, fill: &mut bool, stroke: &mut bool| {
        if !b.d.is_empty() {
            paths.push(GeomPath { d: b.d.join(" "), points: std::mem::take(&mut b.points), fill: if *fill { FillMode::Norm } else { FillMode::None }, stroke: *stroke });
        }
        b.d.clear();
        b.points.clear();
        (*fill, *stroke) = (true, true);
    };
    let mut i = 0;
    let mut cmd = "M".to_string();
    let mut quadrant_x = true;
    // Bounded: every iteration consumes at least one token.
    while i < toks.len() {
        if is_command(&toks[i]) {
            cmd = toks[i].clone();
            i += 1;
            match cmd.as_str() {
                "Z" => { b.close(); continue }
                "N" => { finish(&mut b, &mut fill, &mut stroke); continue }
                "F" => { fill = false; continue }
                "S" => { stroke = false; continue }
                "X" => quadrant_x = true,
                "Y" => quadrant_x = false,
                _ => {}
            }
        }
        let need = match cmd.as_str() {
            "M" | "L" | "X" | "Y" => 2,
            "Q" | "G" => 4,
            "C" | "T" | "U" => 6,
            "A" | "B" | "W" | "V" => 8,
            _ => { i += 1; continue }
        };
        if i + need > toks.len() || toks[i..i + need].iter().any(|t| is_command(t)) {
            i += 1;
            continue;
        }
        let v: Vec<f64> = toks[i..i + need].iter().map(|t| env.param(t)).collect();
        i += need;
        let p = |k: usize| (v[k] - origin.0, v[k + 1] - origin.1);
        match cmd.as_str() {
            "M" => { b.move_to(p(0)); cmd = "L".into() }
            "L" => b.line_to(p(0)),
            "C" => b.cubic(p(0), p(2), p(4)),
            "Q" => {
                let (q, e) = (p(0), p(2));
                let c = b.cur;
                b.cubic((c.0 + 2.0 / 3.0 * (q.0 - c.0), c.1 + 2.0 / 3.0 * (q.1 - c.1)), (e.0 + 2.0 / 3.0 * (q.0 - e.0), e.1 + 2.0 / 3.0 * (q.1 - e.1)), e);
            }
            "T" | "U" => {
                // Centre, radii, start and end angle (degrees): clockwise on screen.
                let (c, (wr, hr)) = (p(0), (v[2].abs(), v[3].abs()));
                let (st, en) = (v[4], v[5]);
                let full = ((en - st).abs() - 360.0).abs() < 1e-9;
                let a0 = visual_to_param(wr, hr, st);
                let mut sweep = if full { std::f64::consts::TAU } else { (visual_to_param(wr, hr, en) - a0).rem_euclid(std::f64::consts::TAU) };
                if sweep == 0.0 && !full { sweep = 0.0001 }
                b.arc(c, (wr, hr), a0, sweep, cmd == "T");
            }
            "A" | "B" | "W" | "V" => {
                // Bounding box corners, then the radials through the start and end points;
                // A/B counter-clockwise on screen, W/V clockwise; B/V start a new subpath.
                let (r1, r2) = (p(0), p(2));
                let c = ((r1.0 + r2.0) / 2.0, (r1.1 + r2.1) / 2.0);
                let r = ((r2.0 - r1.0).abs() / 2.0, (r2.1 - r1.1).abs() / 2.0);
                if r.0 > 0.0 && r.1 > 0.0 {
                    let ang = |q: (f64, f64)| ((q.1 - c.1) / r.1).atan2((q.0 - c.0) / r.0);
                    let (a0, a1) = (ang(p(4)), ang(p(6)));
                    let tau = std::f64::consts::TAU;
                    let mut sweep = if matches!(cmd.as_str(), "W" | "V") { (a1 - a0).rem_euclid(tau) } else { -(a0 - a1).rem_euclid(tau) };
                    if sweep == 0.0 { sweep = if matches!(cmd.as_str(), "W" | "V") { tau } else { -tau } }
                    b.arc(c, r, a0, sweep, matches!(cmd.as_str(), "A" | "W"));
                }
            }
            "G" => {
                // DrawingML's arcTo: radii, start and swing angle (degrees), from the current point.
                let (wr, hr, st, sw) = (v[0].abs(), v[1].abs(), v[2], v[3].clamp(-360.0, 360.0));
                if wr > 0.0 && hr > 0.0 {
                    let a0 = visual_to_param(wr, hr, st);
                    let a1 = visual_to_param(wr, hr, st + sw);
                    let mut sweep = a1 - a0;
                    if sw > 0.0 && sweep <= 0.0 { sweep += std::f64::consts::TAU }
                    if sw < 0.0 && sweep >= 0.0 { sweep -= std::f64::consts::TAU }
                    let c = (b.cur.0 - wr * a0.cos(), b.cur.1 - hr * a0.sin());
                    b.arc(c, (wr, hr), a0, sweep, true);
                }
            }
            "X" | "Y" => {
                // A quarter ellipse to the point, leaving horizontally (X) or vertically (Y);
                // repeated pairs alternate.
                let (s, e) = (b.cur, p(0));
                const K: f64 = 0.552_284_75;
                if quadrant_x {
                    b.cubic((s.0 + K * (e.0 - s.0), s.1), (e.0, e.1 + K * (s.1 - e.1)), e);
                } else {
                    b.cubic((s.0, s.1 + K * (e.1 - s.1)), (e.0 + K * (s.0 - e.0), e.1), e);
                }
                quadrant_x = !quadrant_x;
            }
            _ => {}
        }
    }
    finish(&mut b, &mut fill, &mut stroke);

    // The text area: the first of `draw:text-areas` (left top right bottom), in shape units.
    let text_rect = extract_attr(attrs, "draw:text-areas").and_then(|t| {
        let v: Vec<f64> = tokens(t).iter().take(4).map(|x| env.param(x)).collect();
        (v.len() == 4).then(|| {
            let ex = |x: f64| (x - origin.0) / view[2] * w;
            let ey = |y: f64| (y - origin.1) / view[3] * h;
            [ex(v[0]), ey(v[1]), ex(v[2]), ey(v[3])]
        })
    });
    (!paths.is_empty()).then_some(Geometry { paths, text_rect })
}

/// SVG path data (`svg:d`, in the coordinates of `view` = x, y, w, h) as absolute M/L/C/Q/A/Z
/// commands in the 0..100 box: relative commands made absolute, H/V/S/T expanded. Returns the
/// data, the flattened points per subpath, and whether any subpath is closed.
pub fn svg_path(d: &str, view: [f64; 4]) -> Option<(String, Vec<Vec<(f64, f64)>>, bool)> {
    if view[2] <= 0.0 || view[3] <= 0.0 {
        return None;
    }
    let mut b = Builder { d: Vec::new(), points: Vec::new(), cur: (0.0, 0.0), sx: 100.0 / view[2], sy: 100.0 / view[3], flip: (false, false) };
    let o = (view[0], view[1]);
    // Tokens: command letters and numbers (with exponents, signs and run-together decimals).
    let mut toks: Vec<Result<char, f64>> = Vec::new();
    let s = d.as_bytes();
    let mut i = 0;
    while i < s.len() {
        let c = s[i] as char;
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            toks.push(Ok(c));
            i += 1;
        } else if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' {
            let start = i;
            i += 1;
            let mut dot = c == '.';
            while i < s.len() {
                let ch = s[i] as char;
                if ch.is_ascii_digit() || (ch == '.' && !dot) {
                    dot |= ch == '.';
                    i += 1;
                } else if (ch == 'e' || ch == 'E') && i + 1 < s.len() {
                    i += 1;
                    if matches!(s[i] as char, '-' | '+') { i += 1 }
                } else {
                    break;
                }
            }
            if let Ok(v) = d[start..i].parse::<f64>() {
                toks.push(Err(v));
            }
        } else {
            i += 1;
        }
    }
    let mut closed = false;
    let (mut cmd, mut k) = ('M', 0usize);
    let mut start = (0.0, 0.0);
    let mut last_ctrl: Option<(f64, f64)> = None;
    let mut last_q: Option<(f64, f64)> = None;
    let mut cur = (0.0, 0.0);
    // Bounded: every pass consumes at least one token.
    while k < toks.len() {
        if let Ok(c) = toks[k] {
            cmd = c;
            k += 1;
            if c == 'Z' || c == 'z' {
                b.close();
                closed = true;
                cur = start;
                b.cur = (start.0 - o.0, start.1 - o.1);
                continue;
            }
        }
        let n = match cmd.to_ascii_uppercase() { 'M' | 'L' | 'T' => 2, 'H' | 'V' => 1, 'C' => 6, 'S' | 'Q' => 4, 'A' => 7, _ => { k += 1; continue } };
        let vals: Vec<f64> = toks[k..].iter().take(n).map_while(|t| t.err()).collect();
        if vals.len() < n {
            k += vals.len().max(1);
            continue;
        }
        k += n;
        let rel = cmd.is_ascii_lowercase();
        let abs = |x: f64, y: f64| if rel { (cur.0 + x, cur.1 + y) } else { (x, y) };
        let local = |p: (f64, f64)| (p.0 - o.0, p.1 - o.1);
        let mut ctrl = None;
        let mut qctrl = None;
        match cmd.to_ascii_uppercase() {
            'M' => {
                cur = abs(vals[0], vals[1]);
                start = cur;
                b.move_to(local(cur));
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' => { cur = abs(vals[0], vals[1]); b.line_to(local(cur)) }
            'H' => { cur = (if rel { cur.0 + vals[0] } else { vals[0] }, cur.1); b.line_to(local(cur)) }
            'V' => { cur = (cur.0, if rel { cur.1 + vals[0] } else { vals[0] }); b.line_to(local(cur)) }
            'C' | 'S' => {
                let (c1, c2, e) = if cmd.to_ascii_uppercase() == 'C' {
                    (abs(vals[0], vals[1]), abs(vals[2], vals[3]), abs(vals[4], vals[5]))
                } else {
                    let c1 = last_ctrl.map_or(cur, |lc| (2.0 * cur.0 - lc.0, 2.0 * cur.1 - lc.1));
                    (c1, abs(vals[0], vals[1]), abs(vals[2], vals[3]))
                };
                b.cubic(local(c1), local(c2), local(e));
                ctrl = Some(c2);
                cur = e;
            }
            'Q' | 'T' => {
                let (q, e) = if cmd.to_ascii_uppercase() == 'Q' {
                    (abs(vals[0], vals[1]), abs(vals[2], vals[3]))
                } else {
                    (last_q.map_or(cur, |lq| (2.0 * cur.0 - lq.0, 2.0 * cur.1 - lq.1)), abs(vals[0], vals[1]))
                };
                let c = cur;
                b.cubic(local((c.0 + 2.0 / 3.0 * (q.0 - c.0), c.1 + 2.0 / 3.0 * (q.1 - c.1))), local((e.0 + 2.0 / 3.0 * (q.0 - e.0), e.1 + 2.0 / 3.0 * (q.1 - e.1))), local(e));
                qctrl = Some(q);
                cur = e;
            }
            'A' => {
                // Endpoint arc -> centre form (SVG implementation notes F.6.5), then Béziers.
                let e = abs(vals[5], vals[6]);
                let (mut rx, mut ry) = (vals[0].abs(), vals[1].abs());
                let phi = vals[2].to_radians();
                let (large, sweep) = (vals[3] != 0.0, vals[4] != 0.0);
                if rx == 0.0 || ry == 0.0 || (cur.0 - e.0).abs() + (cur.1 - e.1).abs() < 1e-12 {
                    b.line_to(local(e));
                } else {
                    let (cp, sp) = (phi.cos(), phi.sin());
                    let (dx, dy) = ((cur.0 - e.0) / 2.0, (cur.1 - e.1) / 2.0);
                    let (x1, y1) = (cp * dx + sp * dy, -sp * dx + cp * dy);
                    let lam = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
                    if lam > 1.0 { rx *= lam.sqrt(); ry *= lam.sqrt() }
                    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
                    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
                    let mut f = if den > 0.0 { (num / den).sqrt() } else { 0.0 };
                    if large == sweep { f = -f }
                    let (cx1, cy1) = (f * rx * y1 / ry, -f * ry * x1 / rx);
                    let (cx, cy) = (cp * cx1 - sp * cy1 + (cur.0 + e.0) / 2.0, sp * cx1 + cp * cy1 + (cur.1 + e.1) / 2.0);
                    let ang = |ux: f64, uy: f64| uy.atan2(ux);
                    let a0 = ang((x1 - cx1) / rx, (y1 - cy1) / ry);
                    let a1 = ang((-x1 - cx1) / rx, (-y1 - cy1) / ry);
                    let tau = std::f64::consts::TAU;
                    let mut d = a1 - a0;
                    if sweep && d < 0.0 { d += tau }
                    if !sweep && d > 0.0 { d -= tau }
                    if phi.abs() < 1e-9 {
                        b.arc(local((cx, cy)), (rx, ry), a0, d, true);
                    } else {
                        // A turned ellipse: sampled.
                        for j in 1..=24 {
                            let a = a0 + d * j as f64 / 24.0;
                            let (px, py) = (rx * a.cos(), ry * a.sin());
                            b.line_to(local((cp * px - sp * py + cx, sp * px + cp * py + cy)));
                        }
                    }
                }
                cur = e;
            }
            _ => {}
        }
        b.cur = local(cur);
        last_ctrl = ctrl;
        last_q = qctrl;
    }
    (!b.d.is_empty()).then(|| (b.d.join(" "), b.points, closed))
}

/// Points (`draw:points="x,y x,y ..."`, in `view` coordinates) as path data in the 0..100 box.
pub fn points_path(points: &str, view: [f64; 4], close: bool) -> Option<(String, Vec<Vec<(f64, f64)>>)> {
    let nums: Vec<f64> = points.split(|c: char| c.is_whitespace() || c == ',').filter_map(|v| v.parse().ok()).collect();
    let mut d = String::new();
    for (k, p) in nums.chunks_exact(2).enumerate() {
        d.push_str(&format!("{}{} {} ", if k == 0 { "M" } else { "L" }, p[0], p[1]));
    }
    if close {
        d.push('Z');
    }
    svg_path(&d, view).map(|(d, pts, _)| (d, pts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas_read_like_libreoffice_writes_them() {
        let mut env = Env { modifiers: vec![5400.0], equations: vec!["if(0-$0 ,0,if(21599999-$0 ,$0 ,21599999))".into(), "?f0 *sin(pi*(90)/180)+logwidth/2".into(), "max(min(3, -2), abs(-7))/(1+1)".into()], values: HashMap::new(), view: [0.0, 0.0, 21600.0, 21600.0], log: (1000.0, 500.0), depth: 0 };
        assert_eq!(env.equation(0), 5400.0);
        assert!((env.equation(1) - 5900.0).abs() < 1e-6);
        assert_eq!(env.equation(2), 3.5);
        // A formula that refers to itself is 0, not a hang.
        let mut env = Env { modifiers: vec![], equations: vec!["?f0 +1".into()], values: HashMap::new(), view: [0.0, 0.0, 1.0, 1.0], log: (1.0, 1.0), depth: 0 };
        assert_eq!(env.equation(0), 1.0);
    }

    #[test]
    fn enhanced_paths_draw_in_the_view_box() {
        // LibreOffice's own rectangle-with-a-diagonal: two parts, the second unfilled.
        let g = enhanced(r#"svg:viewBox="0 0 21600 21600" draw:enhanced-path="M 0 0 L 21600 0 21600 21600 0 21600 Z N M 0 0 L ?f0 21600 F N""#, r#"<draw:equation draw:formula="width"/>"#, 2000.0, 1000.0).unwrap();
        assert_eq!(g.paths.len(), 2);
        assert_eq!(g.paths[0].d, "M 0 0 L 100 0 L 100 100 L 0 100 Z");
        assert_eq!((g.paths[1].d.as_str(), g.paths[1].fill), ("M 0 0 L 100 100", FillMode::None));
        // A whole ellipse by angle-ellipse (U): four quarter curves back to its start.
        let g = enhanced(r#"svg:viewBox="0 0 21600 21600" draw:enhanced-path="U 10800 10800 10800 10800 0 360 Z N""#, "", 1000.0, 1000.0).unwrap();
        assert!(g.paths[0].d.starts_with("M 100 50 C"), "{}", g.paths[0].d);
        assert_eq!(g.paths[0].d.matches('C').count(), 4);
    }

    #[test]
    fn powerpoint_shapes_use_their_exact_preset() {
        let g = enhanced(r#"draw:type="ooxml-roundRect" draw:modifiers="50000" svg:viewBox="0 0 0 0" draw:enhanced-path="M 0 0 Z""#, "", 1000.0, 500.0).unwrap();
        assert!(g.paths[0].d.starts_with("M 0 50"), "{}", g.paths[0].d);
    }

    #[test]
    fn svg_paths_become_absolute_in_the_box() {
        let (d, _, closed) = svg_path("m100 100 h 100 v100 H100 z", [100.0, 100.0, 200.0, 200.0]).unwrap();
        assert_eq!(d, "M 0 0 L 50 0 L 50 50 L 0 50 Z");
        assert!(closed);
        let (d, _, _) = svg_path("M0 0 A 50 50 0 0 1 100 0", [0.0, -50.0, 100.0, 100.0]).unwrap();
        assert!(d.starts_with("M 0 50 L 0 50 C") || d.starts_with("M 0 50 C"), "{d}");
        let (d, _) = points_path("0,0 10,0 10,10", [0.0, 0.0, 10.0, 10.0], true).unwrap();
        assert_eq!(d, "M 0 0 L 100 0 L 100 100 Z");
    }
}
