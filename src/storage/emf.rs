//! Enhanced Metafile (`.emf`) pictures: what PowerPoint embeds for pasted Excel ranges and charts,
//! Visio drawings, equations and the previews of embedded objects. A browser can't show one, so
//! its records are read into the same vector items as a WMF (`wmf::WmfItem`), in device units,
//! for `pptx_import`'s `metafile_group` to turn into slide elements.
//!
//! Covers the records such pictures use (MS-EMF): the header's frame, mapping modes, window and
//! viewport origin/extent, world transforms, saved device contexts, pens, brushes, fonts (and the
//! stock ones), path brackets (begin/end/close/fill/stroke), lines, polylines, polygons and
//! Béziers in 32- and 16-bit forms, poly-polygons, rectangles, rounded rectangles, ellipses, arcs,
//! chords, pies, `ExtTextOut` (wide and ANSI) and device-independent bitmaps (`StretchDIBits`,
//! `BitBlt`, `StretchBlt`, including a pattern-filled rectangle). EMF+ records (in GDI comments)
//! are skipped: Office writes GDI records alongside them. Clipping and raster operations are
//! skipped too.

use std::collections::HashMap;

use crate::storage::wmf::{dib_to_bmp, Wmf, WmfItem};

/// A runaway or hostile file stops adding items here rather than flooding a slide.
const MAX_ITEMS: usize = 20_000;

pub fn is_emf(bytes: &[u8]) -> bool {
    u32le(bytes, 0) == Some(1) && bytes.get(40..44) == Some(b" EMF")
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn i32le(b: &[u8], at: usize) -> Option<f64> {
    u32le(b, at).map(|v| v as i32 as f64)
}

fn i16le(b: &[u8], at: usize) -> Option<f64> {
    Some(i16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as f64)
}

fn f32le(b: &[u8], at: usize) -> Option<f64> {
    Some(f32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as f64).filter(|v| v.is_finite())
}

fn colorref(b: &[u8], at: usize) -> Option<String> {
    let c = b.get(at..at + 3)?;
    Some(format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
}

#[derive(Clone)]
enum Obj {
    Pen { color: String, width: f64, null: bool },
    Brush { color: String, null: bool },
    Font { height: f64, weight: u32, italic: bool, face: String, escapement: f64 },
    Other,
}

/// Logical (page) coordinates to device ones: the world transform, then the window/viewport mapping.
#[derive(Clone)]
struct Xf {
    /// eM11, eM12, eM21, eM22, eDx, eDy.
    world: [f64; 6],
    map_mode: u32,
    win_org: (f64, f64),
    win_ext: (f64, f64),
    vp_org: (f64, f64),
    vp_ext: (f64, f64),
    /// Device units per millimetre (from the header), for the metric mapping modes.
    px_per_mm: (f64, f64),
}

impl Xf {
    /// The page-to-device scale: anisotropic/isotropic from the extents, metric modes from their
    /// unit size (y up), text mode 1:1.
    fn page_scale(&self) -> (f64, f64) {
        let metric = |mm: f64| (mm * self.px_per_mm.0, -mm * self.px_per_mm.1);
        match self.map_mode {
            2 => metric(0.1),
            3 => metric(0.01),
            4 => metric(0.254),
            5 => metric(0.0254),
            6 => metric(25.4 / 1440.0),
            7 | 8 if self.win_ext.0 != 0.0 && self.win_ext.1 != 0.0 => {
                let (sx, sy) = (self.vp_ext.0 / self.win_ext.0, self.vp_ext.1 / self.win_ext.1);
                if self.map_mode == 7 {
                    // Isotropic: one scale for both axes, the smaller, keeping each axis's direction.
                    let s = sx.abs().min(sy.abs());
                    (s * sx.signum(), s * sy.signum())
                } else {
                    (sx, sy)
                }
            }
            _ => (1.0, 1.0),
        }
    }

    fn dev(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let w = &self.world;
        let (px, py) = (w[0] * x + w[2] * y + w[4], w[1] * x + w[3] * y + w[5]);
        let (sx, sy) = self.page_scale();
        ((px - self.win_org.0) * sx + self.vp_org.0, (py - self.win_org.1) * sy + self.vp_org.1)
    }

    /// How long a logical length is on the device (a pen width, a font height).
    fn len(&self, v: f64) -> f64 {
        let (sx, sy) = self.page_scale();
        let w = &self.world;
        let k = ((w[0] * w[3] - w[1] * w[2]).abs()).sqrt() * (sx * sy).abs().sqrt();
        v.abs() * if k > 0.0 { k } else { 1.0 }
    }

    /// Whether the mapping turns or shears (axis-aligned shapes then become polygons).
    fn turns(&self) -> bool {
        self.world[1].abs() > 1e-9 || self.world[2].abs() > 1e-9
    }
}

/// `a` then `b`, as XFORMs compose.
fn compose(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Clone)]
struct Dc {
    /// Colour and width in logical units.
    pen: Option<(String, f64)>,
    brush: Option<String>,
    font: (f64, u32, bool, String, f64),
    text_color: String,
    text_align: u32,
    /// The current position, in logical units.
    pos: (f64, f64),
    clockwise: bool,
    xf: Xf,
}

/// The figures of an open path bracket, in device units, and whether each was closed.
#[derive(Default)]
struct PathBuild {
    figures: Vec<(Vec<(f64, f64)>, bool)>,
}

impl PathBuild {
    fn start(&mut self, p: (f64, f64)) {
        self.figures.push((vec![p], false));
    }
    fn extend(&mut self, pts: impl IntoIterator<Item = (f64, f64)>, from: (f64, f64)) {
        if self.figures.last().is_none_or(|(f, closed)| f.is_empty() || *closed) {
            self.start(from);
        }
        if let Some((f, _)) = self.figures.last_mut() {
            f.extend(pts);
        }
    }
    fn closed_figure(&mut self, pts: Vec<(f64, f64)>) {
        if pts.len() >= 2 {
            self.figures.push((pts, true));
        }
    }
}

/// Points along a cubic Bézier after `p0` (not including it).
fn bezier(p0: (f64, f64), c1: (f64, f64), c2: (f64, f64), p3: (f64, f64)) -> impl Iterator<Item = (f64, f64)> {
    (1..=16).map(move |k| {
        let t = k as f64 / 16.0;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        (a * p0.0 + b * c1.0 + c * c2.0 + d * p3.0, a * p0.1 + b * c1.1 + c * c2.1 + d * p3.1)
    })
}

/// A pattern brush record's (EMR_CREATEMONOBRUSH / EMR_CREATEDIBPATTERNBRUSHPT) picture, as its
/// average colour: ihBrush, iUsage, then the bitmap header's and bits' offsets and sizes.
fn pattern_color(r: &[u8]) -> Option<String> {
    let (off_bmi, cb_bmi, off_bits, cb_bits) = (u32le(r, 16)? as usize, u32le(r, 20)? as usize, u32le(r, 24)? as usize, u32le(r, 28)? as usize);
    let mut dib = r.get(off_bmi..off_bmi.checked_add(cb_bmi)?)?.to_vec();
    dib.extend_from_slice(r.get(off_bits..off_bits.checked_add(cb_bits)?)?);
    let bmp = crate::storage::wmf::dib_to_bmp(&dib)?;
    let img = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp).ok()?.to_rgb8();
    Some(crate::storage::pptx_import::average_hex(&img, 1))
}

/// Logical points of an elliptical arc in box `(l, t, r, b)`, from the radial through `start`,
/// `sweep` radians (negative: counter-clockwise on screen), as `n` steps.
fn arc_points((l, t, r, b): (f64, f64, f64, f64), start: f64, sweep: f64, n: usize) -> Vec<(f64, f64)> {
    let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + b) / 2.0, (r - l).abs() / 2.0, (b - t).abs() / 2.0);
    (0..=n).map(|k| {
        let a = start + sweep * k as f64 / n as f64;
        (cx + rx * a.cos(), cy + ry * a.sin())
    }).collect()
}

/// Reads an enhanced metafile. `None` when it isn't one, or draws nothing this understands.
pub fn parse(bytes: &[u8]) -> Option<Wmf> {
    if !is_emf(bytes) {
        return None;
    }
    // Header: bounds (device units), frame (0.01 mm), ... device size in pixels and millimetres.
    let rect = |at: usize| Some((i32le(bytes, at)?, i32le(bytes, at + 4)?, i32le(bytes, at + 8)?, i32le(bytes, at + 12)?));
    let bounds = rect(8)?;
    let frame_mm = rect(24)?;
    let (dev_px, dev_mm) = ((i32le(bytes, 72)?, i32le(bytes, 76)?), (i32le(bytes, 80)?, i32le(bytes, 84)?));
    let px_per_mm = if dev_px.0 > 0.0 && dev_mm.0 > 0.0 && dev_px.1 > 0.0 && dev_mm.1 > 0.0 { (dev_px.0 / dev_mm.0, dev_px.1 / dev_mm.1) } else { (96.0 / 25.4, 96.0 / 25.4) };

    let xf = Xf { world: IDENTITY, map_mode: 1, win_org: (0.0, 0.0), win_ext: (1.0, 1.0), vp_org: (0.0, 0.0), vp_ext: (1.0, 1.0), px_per_mm };
    let mut dc = Dc { pen: Some(("#000000".into(), 0.0)), brush: Some("#ffffff".into()), font: (12.0, 400, false, "Arial".into(), 0.0), text_color: "#000000".into(), text_align: 0, pos: (0.0, 0.0), clockwise: false, xf };
    let mut saved: Vec<Dc> = Vec::new();
    let mut objects: HashMap<u32, Obj> = HashMap::new();
    let mut items: Vec<WmfItem> = Vec::new();
    let mut path: Option<PathBuild> = None;
    let mut ended: Option<PathBuild> = None;

    let pen_of = |dc: &Dc| dc.pen.as_ref().map(|(c, w)| (c.clone(), dc.xf.len(*w).max(1.0)));

    let mut at = 0usize;
    while at + 8 <= bytes.len() && items.len() < MAX_ITEMS {
        let (Some(kind), Some(size)) = (u32le(bytes, at), u32le(bytes, at + 4)) else { break };
        let size = size as usize;
        if size < 8 || size % 4 != 0 || at + size > bytes.len() {
            break;
        }
        let r = &bytes[at..at + size];
        let pt = |o: usize| Some((i32le(r, o)?, i32le(r, o + 4)?));
        let pt16 = |o: usize| Some((i16le(r, o)?, i16le(r, o + 2)?));
        match kind {
            14 => break, // EOF
            9 => dc.xf.win_ext = pt(8).unwrap_or((1.0, 1.0)),
            10 => dc.xf.win_org = pt(8).unwrap_or((0.0, 0.0)),
            11 => dc.xf.vp_ext = pt(8).unwrap_or((1.0, 1.0)),
            12 => dc.xf.vp_org = pt(8).unwrap_or((0.0, 0.0)),
            17 => dc.xf.map_mode = u32le(r, 8).unwrap_or(1),
            35 | 36 => {
                let m: Option<Vec<f64>> = (0..6).map(|k| f32le(r, 8 + 4 * k)).collect();
                if let Some(m) = m {
                    let m = [m[0], m[1], m[2], m[3], m[4], m[5]];
                    dc.xf.world = match (kind, u32le(r, 32).unwrap_or(4)) {
                        (35, _) | (_, 4) => m,
                        (_, 1) => IDENTITY,
                        (_, 2) => compose(m, dc.xf.world),
                        (_, 3) => compose(dc.xf.world, m),
                        _ => dc.xf.world,
                    };
                }
            }
            // (Bounded: a file of nothing but SaveDC records can't grow this without limit.)
            33 if saved.len() < 1024 => saved.push(dc.clone()),
            33 => {}
            34 => {
                let n = i32le(r, 8).unwrap_or(-1.0).abs().max(1.0) as usize;
                for _ in 0..n.min(saved.len()) {
                    if let Some(d) = saved.pop() {
                        dc = d;
                    }
                }
            }
            38 | 95 | 39 | 82 | 93 | 94 => {
                let Some(ih) = u32le(r, 8) else { at += size; continue };
                let obj = match kind {
                    38 => Obj::Pen { null: u32le(r, 12).is_some_and(|s| s & 0xF == 5), width: i32le(r, 16).unwrap_or(0.0), color: colorref(r, 24).unwrap_or_else(|| "#000000".into()) },
                    95 => Obj::Pen { null: u32le(r, 28).is_some_and(|s| s & 0xF == 5) || u32le(r, 36) == Some(1), width: u32le(r, 32).unwrap_or(0) as f64, color: colorref(r, 40).unwrap_or_else(|| "#000000".into()) },
                    // A hatched brush (style 2) draws thin lines of its colour over what's behind: a
                    // see-through tint of it, as the ODP importer shows a hatch.
                    39 => {
                        let color = colorref(r, 16).unwrap_or_else(|| "#000000".into());
                        let color = if u32le(r, 12) == Some(2) { crate::storage::pptx_inherit::with_alpha(&color, 0.35) } else { color };
                        Obj::Brush { null: u32le(r, 12) == Some(1), color }
                    }
                    // A pattern brush (mono or DIB): its picture's average colour.
                    93 | 94 => Obj::Brush { null: false, color: pattern_color(r).unwrap_or_else(|| "#808080".into()) },
                    82 => {
                        let face: String = char::decode_utf16((0..32).map_while(|k| r.get(40 + 2 * k..42 + 2 * k).map(|b| u16::from_le_bytes([b[0], b[1]]))).take_while(|c| *c != 0))
                            .map(|c| c.unwrap_or('?')).collect();
                        Obj::Font { height: i32le(r, 12).unwrap_or(12.0), escapement: i32le(r, 20).unwrap_or(0.0) / 10.0, weight: u32le(r, 28).unwrap_or(400), italic: r.get(32).is_some_and(|&b| b != 0), face }
                    }
                    _ => Obj::Other,
                };
                objects.insert(ih, obj);
            }
            37 => {
                let Some(ih) = u32le(r, 8) else { at += size; continue };
                if ih & 0x8000_0000 != 0 {
                    match ih & 0x7FFF_FFFF {
                        0 => dc.brush = Some("#ffffff".into()),
                        1 => dc.brush = Some("#c0c0c0".into()),
                        2 => dc.brush = Some("#808080".into()),
                        3 => dc.brush = Some("#404040".into()),
                        4 => dc.brush = Some("#000000".into()),
                        5 => dc.brush = None,
                        6 => dc.pen = Some(("#ffffff".into(), 0.0)),
                        7 => dc.pen = Some(("#000000".into(), 0.0)),
                        8 => dc.pen = None,
                        _ => {}
                    }
                } else if let Some(obj) = objects.get(&ih) {
                    match obj.clone() {
                        Obj::Pen { color, width, null } => dc.pen = (!null).then_some((color, width)),
                        Obj::Brush { color, null } => dc.brush = (!null).then_some(color),
                        Obj::Font { height, weight, italic, face, escapement } => dc.font = (height, weight, italic, face, escapement),
                        Obj::Other => {}
                    }
                }
            }
            40 => {
                if let Some(ih) = u32le(r, 8) {
                    objects.remove(&ih);
                }
            }
            24 => dc.text_color = colorref(r, 8).unwrap_or_else(|| "#000000".into()),
            22 => dc.text_align = u32le(r, 8).unwrap_or(0),
            57 => dc.clockwise = u32le(r, 8) == Some(2),
            // BeginPath (59) opens a bracket; EndPath (60) closes it, keeping the path for a later
            // Fill/Stroke (drawing after it goes on the page again). FlattenPath (65) and WidenPath
            // (66) leave it as it is (curves are already points here). SelectClipPath (67) uses it
            // up as a clip, which isn't drawn (clipping isn't supported); AbortPath (68) drops it.
            59 => {
                path = Some(PathBuild::default());
                ended = None;
            }
            60 => ended = path.take(),
            67 | 68 => {
                path = None;
                ended = None;
            }
            61 => {
                if let Some((_, closed)) = path.as_mut().and_then(|p| p.figures.last_mut()) {
                    *closed = true;
                }
            }
            62..=64 => {
                if let Some(p) = path.take().or_else(|| ended.take()) {
                    let fill = if kind == 64 { None } else { dc.brush.clone() };
                    let stroke = if kind == 62 { None } else { pen_of(&dc) };
                    let subpaths: Vec<Vec<(f64, f64)>> = p.figures.iter().filter(|(f, _)| f.len() >= 2).map(|(f, _)| f.clone()).collect();
                    let closed = kind != 64 || p.figures.iter().all(|(_, c)| *c);
                    if !subpaths.is_empty() && (fill.is_some() || stroke.is_some()) {
                        items.push(WmfItem::Path { subpaths, closed, fill: if closed { fill } else { None }, stroke });
                    }
                }
            }
            27 => {
                let to = pt(8).unwrap_or((0.0, 0.0));
                dc.pos = to;
                if let Some(p) = path.as_mut() {
                    p.start(dc.xf.dev(to));
                }
            }
            54 => {
                let to = pt(8).unwrap_or((0.0, 0.0));
                let (from, end) = (dc.xf.dev(dc.pos), dc.xf.dev(to));
                match path.as_mut() {
                    Some(p) => p.extend([end], from),
                    None => if let Some(stroke) = pen_of(&dc) {
                        items.push(WmfItem::Path { subpaths: vec![vec![from, end]], closed: false, fill: None, stroke: Some(stroke) });
                    },
                }
                dc.pos = to;
            }
            // Polylines/polygons/Béziers, 32-bit (2-6) and 16-bit (85-89): bounds, count, points.
            2..=6 | 85..=89 => {
                let wide = kind < 85;
                let base = if wide { kind } else { kind - 83 };
                let n = u32le(r, 24).unwrap_or(0) as usize;
                let step = if wide { 8 } else { 4 };
                let pts: Vec<(f64, f64)> = (0..n).map_while(|k| if wide { pt(28 + step * k) } else { pt16(28 + step * k) }).collect();
                let to_line = base == 5 || base == 6;
                let mut dev: Vec<(f64, f64)> = Vec::new();
                let start = if to_line { Some(dc.pos) } else { pts.first().copied() };
                if base == 2 || base == 5 {
                    // Béziers: a start point (or the current position), then threes.
                    let (mut cur, rest) = match start {
                        Some(s) if to_line => (s, &pts[..]),
                        Some(s) => (s, &pts[1.min(pts.len())..]),
                        None => (dc.pos, &pts[..]),
                    };
                    let first = cur;
                    dev.push(dc.xf.dev(first));
                    for c in rest.chunks_exact(3) {
                        dev.extend(bezier(cur, c[0], c[1], c[2]).map(|p| dc.xf.dev(p)));
                        cur = c[2];
                    }
                    if to_line {
                        dc.pos = cur;
                    }
                } else {
                    if let (true, Some(s)) = (to_line, start) {
                        dev.push(dc.xf.dev(s));
                    }
                    dev.extend(pts.iter().map(|p| dc.xf.dev(*p)));
                    if to_line {
                        if let Some(l) = pts.last() {
                            dc.pos = *l;
                        }
                    }
                }
                if dev.len() >= 2 {
                    let closed = base == 3;
                    match path.as_mut() {
                        Some(p) if to_line => p.extend(dev.into_iter().skip(1), dc.xf.dev(start.unwrap_or(dc.pos))),
                        Some(p) if closed => p.closed_figure(dev),
                        Some(p) => p.figures.push((dev, false)),
                        None => items.push(WmfItem::Path { subpaths: vec![dev], closed, fill: if closed { dc.brush.clone() } else { None }, stroke: pen_of(&dc) }),
                    }
                }
            }
            // PolyPolyline/PolyPolygon, 32-bit (7, 8) and 16-bit (90, 91): bounds, polys, total, counts, points.
            7 | 8 | 90 | 91 => {
                let wide = kind < 90;
                let polys = (u32le(r, 24).unwrap_or(0) as usize).min(size / 4);
                let counts: Vec<usize> = (0..polys).map_while(|k| u32le(r, 32 + 4 * k).map(|c| c as usize)).collect();
                let mut o = 32 + 4 * polys;
                let mut subpaths = Vec::new();
                for c in counts {
                    let pts: Vec<(f64, f64)> = (0..c).map_while(|k| if wide { pt(o + 8 * k) } else { pt16(o + 4 * k) }).map(|p| dc.xf.dev(p)).collect();
                    let short = pts.len() < c;
                    o += c * if wide { 8 } else { 4 };
                    if pts.len() >= 2 {
                        subpaths.push(pts);
                    }
                    if short {
                        break;
                    }
                }
                let closed = kind == 8 || kind == 91;
                match path.as_mut() {
                    Some(p) => subpaths.into_iter().for_each(|s| p.figures.push((s, closed))),
                    None if !subpaths.is_empty() => items.push(WmfItem::Path { subpaths, closed, fill: if closed { dc.brush.clone() } else { None }, stroke: pen_of(&dc) }),
                    None => {}
                }
            }
            43 | 44 | 42 => {
                let Some((l, t, rr, b)) = (|| Some((i32le(r, 8)?, i32le(r, 12)?, i32le(r, 16)?, i32le(r, 20)?)))() else { at += size; continue };
                let outline: Vec<(f64, f64)> = if kind == 42 {
                    arc_points((l, t, rr, b), 0.0, std::f64::consts::TAU, 64)
                } else {
                    vec![(l, t), (rr, t), (rr, b), (l, b)]
                };
                let dev: Vec<(f64, f64)> = outline.iter().map(|p| dc.xf.dev(*p)).collect();
                match path.as_mut() {
                    Some(p) => p.closed_figure(dev),
                    None if kind == 42 && !dc.xf.turns() => {
                        let (c, e) = (dc.xf.dev(((l + rr) / 2.0, (t + b) / 2.0)), dc.xf.dev((rr, b)));
                        let (rx, ry) = ((e.0 - c.0).abs(), (e.1 - c.1).abs());
                        let from = (c.0 + rx, c.1);
                        items.push(WmfItem::Arc { center: c, rx, ry, from, to: from, large: true, closed: true, pie: false, fill: dc.brush.clone(), stroke: pen_of(&dc) });
                    }
                    None => items.push(WmfItem::Path { subpaths: vec![dev], closed: true, fill: dc.brush.clone(), stroke: pen_of(&dc) }),
                }
            }
            // Arc, Chord, Pie, ArcTo: box, then the radials the arc starts and ends on.
            45 | 46 | 47 | 55 => {
                let Some(((l, t, rr, b), s, e)) = (|| Some(((i32le(r, 8)?, i32le(r, 12)?, i32le(r, 16)?, i32le(r, 20)?), pt(24)?, pt(32)?)))() else { at += size; continue };
                let (cx, cy) = ((l + rr) / 2.0, (t + b) / 2.0);
                // Where each radial meets the ellipse, as its parametric angle (x = cx + rx cos t,
                // y = cy + ry sin t): on a box that isn't square that differs from the radial's angle.
                let (rx, ry) = (((rr - l) / 2.0).abs().max(f64::EPSILON), ((b - t) / 2.0).abs().max(f64::EPSILON));
                let param = |p: (f64, f64)| ((p.1 - cy) * rx).atan2((p.0 - cx) * ry);
                let (a0, a1) = (param(s), param(e));
                // Counter-clockwise on screen (y down) is decreasing angle; clockwise increasing.
                let tau = std::f64::consts::TAU;
                let mut sweep = if dc.clockwise { (a1 - a0).rem_euclid(tau) } else { -(a0 - a1).rem_euclid(tau) };
                if sweep == 0.0 {
                    sweep = if dc.clockwise { tau } else { -tau };
                }
                let mut pts: Vec<(f64, f64)> = arc_points((l, t, rr, b), a0, sweep, 48).into_iter().map(|p| dc.xf.dev(p)).collect();
                if kind == 47 {
                    pts.push(dc.xf.dev((cx, cy)));
                }
                let closed = kind == 46 || kind == 47;
                match path.as_mut() {
                    Some(p) if kind == 55 => p.extend(pts.clone(), dc.xf.dev(dc.pos)),
                    Some(p) if closed => p.closed_figure(pts.clone()),
                    Some(p) => p.figures.push((pts.clone(), false)),
                    None => {
                        let mut subpath = pts.clone();
                        if kind == 55 {
                            subpath.insert(0, dc.xf.dev(dc.pos));
                        }
                        items.push(WmfItem::Path { subpaths: vec![subpath], closed, fill: if closed { dc.brush.clone() } else { None }, stroke: pen_of(&dc) });
                    }
                }
                if kind == 55 {
                    if let Some(last) = arc_points((l, t, rr, b), a0, sweep, 1).last() {
                        dc.pos = *last;
                    }
                }
            }
            // ExtTextOutA/W: bounds, graphics mode, scales, then EMRTEXT (reference, count, string offset, options).
            83 | 84 => {
                let Some(reference) = pt(36) else { at += size; continue };
                let (n, off, options) = (u32le(r, 44).unwrap_or(0) as usize, u32le(r, 48).unwrap_or(0) as usize, u32le(r, 52).unwrap_or(0));
                // ETO_GLYPH_INDEX: the string is glyph numbers in some font, not text.
                if options & 0x10 != 0 || n == 0 {
                    at += size;
                    continue;
                }
                let text: String = if kind == 84 {
                    char::decode_utf16((0..n).map_while(|k| r.get(off + 2 * k..off + 2 * k + 2).map(|b| u16::from_le_bytes([b[0], b[1]])))).map(|c| c.unwrap_or('?')).collect()
                } else {
                    r.get(off..off + n).map(|b| b.iter().map(|&c| c as char).collect()).unwrap_or_default()
                };
                let text = text.trim_end_matches('\0').to_string();
                if !text.trim().is_empty() {
                    let anchor = if dc.text_align & 1 != 0 { dc.pos } else { reference };
                    let (x, y) = dc.xf.dev(anchor);
                    let (height, weight, italic, face, angle) = dc.font.clone();
                    let a = dc.text_align;
                    items.push(WmfItem::Text {
                        x,
                        y,
                        text,
                        size: dc.xf.len(if height != 0.0 { height } else { 12.0 }),
                        color: dc.text_color.clone(),
                        font: if face.is_empty() { "Arial".into() } else { face },
                        bold: weight >= 600,
                        italic,
                        align: match a & 0x0006 { 6 => "center", 2 => "right", _ => "left" },
                        baseline: a & 0x0018 == 0x0018,
                        angle_deg: angle,
                    });
                }
            }
            // StretchDIBits / BitBlt / StretchBlt: a destination box and a packed bitmap.
            81 | 76 | 77 => {
                let (dest, bmi, bits) = if kind == 81 {
                    (pt(24).zip(pt(72)), (u32le(r, 48), u32le(r, 52)), (u32le(r, 56), u32le(r, 60)))
                } else {
                    (pt(24).zip(pt(32)), (u32le(r, 84), u32le(r, 88)), (u32le(r, 92), u32le(r, 96)))
                };
                let Some(((x, y), (w, h))) = dest else { at += size; continue };
                let corners = [dc.xf.dev((x, y)), dc.xf.dev((x + w, y + h))];
                let (x0, y0) = (corners[0].0.min(corners[1].0), corners[0].1.min(corners[1].1));
                let (dw, dh) = ((corners[1].0 - corners[0].0).abs(), (corners[1].1 - corners[0].1).abs());
                match (bmi, bits) {
                    ((Some(ob), Some(cb)), (Some(od), Some(cd))) if cb > 0 => {
                        let (ob, cb, od, cd) = (ob as usize, cb as usize, od as usize, cd as usize);
                        if let (Some(info), Some(px)) = (r.get(ob..ob.saturating_add(cb)), r.get(od..od.saturating_add(cd))) {
                            let mut dib = info.to_vec();
                            dib.extend_from_slice(px);
                            if let Some(bmp) = dib_to_bmp(&dib) {
                                items.push(WmfItem::Bitmap { x: x0, y: y0, w: dw, h: dh, bmp });
                            }
                        }
                    }
                    // No bitmap: a rectangle painted with the brush (PATCOPY).
                    _ if kind != 81 => if let Some(fill) = dc.brush.clone() {
                        items.push(WmfItem::Path { subpaths: vec![vec![(x0, y0), (x0 + dw, y0), (x0 + dw, y0 + dh), (x0, y0 + dh)]], closed: true, fill: Some(fill), stroke: None });
                    },
                    _ => {}
                }
            }
            _ => {}
        }
        at += size;
    }

    if items.is_empty() {
        return None;
    }
    // The picture's frame, in device units: its 0.01 mm frame at the device's resolution, else its bounds.
    let (l, t, r, b) = frame_mm;
    let frame = if r > l && b > t {
        (l / 100.0 * px_per_mm.0, t / 100.0 * px_per_mm.1, (r - l) / 100.0 * px_per_mm.0, (b - t) / 100.0 * px_per_mm.1)
    } else {
        let (l, t, r, b) = bounds;
        (l, t, (r - l + 1.0).max(1.0), (b - t + 1.0).max(1.0))
    };
    Some(Wmf { items, frame })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An EMF: header (96 px/in, frame 0..1000 x 0..500 0.01mm) plus the given records and EOF.
    fn emf(records: &[Vec<u8>]) -> Vec<u8> {
        let le = |v: i32| v.to_le_bytes().to_vec();
        let mut h = Vec::new();
        h.extend(le(1));
        h.extend(le(88));
        for v in [0, 0, 37, 18, 0, 0, 1000, 500] {
            h.extend(le(v));
        }
        h.extend(b" EMF");
        h.extend(le(0x10000));
        // Bytes, records, handles + reserved, description length and offset, palette entries.
        for _ in 0..6 {
            h.extend(le(0));
        }
        for v in [960, 960, 254, 254] {
            h.extend(le(v));
        }
        let mut out = h;
        for r in records {
            out.extend(r);
        }
        out.extend(le(14));
        out.extend(le(20));
        out.extend([0u8; 12]);
        let total = out.len() as i32;
        out[48..52].copy_from_slice(&total.to_le_bytes());
        out
    }

    fn rec(kind: u32, body: &[i32]) -> Vec<u8> {
        let mut r = kind.to_le_bytes().to_vec();
        r.extend(((8 + body.len() * 4) as u32).to_le_bytes());
        body.iter().for_each(|v| r.extend(v.to_le_bytes()));
        r
    }

    #[test]
    fn shapes_fills_and_mapping_come_through_in_device_units() {
        let bytes = emf(&[
            // A red brush (handle 1) selected, then a rectangle in a window of 0..100 mapped to 0..10 px.
            rec(39, &[1, 0, 0x0000FF, 0]),
            rec(37, &[1]),
            rec(17, &[8]),
            rec(9, &[100, 100]),
            rec(11, &[10, 10]),
            rec(43, &[0, 0, 50, 100]),
            // A path bracket: a stroked open triangle.
            rec(59, &[]),
            rec(27, &[0, 0]),
            rec(54, &[100, 0]),
            rec(54, &[100, 100]),
            rec(60, &[]),
            rec(64, &[]),
        ]);
        assert!(is_emf(&bytes));
        let wmf = parse(&bytes).unwrap();
        // Frame: 10mm x 5mm at 3.78 px/mm.
        assert!((wmf.frame.2 - 37.8).abs() < 0.1 && (wmf.frame.3 - 18.9).abs() < 0.1, "{:?}", wmf.frame);
        assert_eq!(wmf.items[0], WmfItem::Path { subpaths: vec![vec![(0.0, 0.0), (5.0, 0.0), (5.0, 10.0), (0.0, 10.0)]], closed: true, fill: Some("#ff0000".into()), stroke: Some(("#000000".into(), 1.0)) });
        let WmfItem::Path { subpaths, closed, fill, .. } = &wmf.items[1] else { panic!("{:?}", wmf.items[1]) };
        assert_eq!((subpaths[0].len(), *closed, fill.is_none()), (3, false, true));
    }

    #[test]
    fn hostile_counts_stop_at_the_record_end() {
        // A polygon claiming a billion points, and a record whose size runs past the file.
        let mut bad = rec(3, &[0, 0, 10, 10, 1_000_000_000, 1, 1, 2, 2]);
        bad.extend(rec(43, &[0, 0, 1, 1]));
        let mut bytes = emf(&[bad]);
        let n = bytes.len();
        bytes[n - 16..n - 12].copy_from_slice(&0x7FFF_FFF0u32.to_le_bytes());
        let wmf = parse(&bytes).unwrap();
        assert!(matches!(&wmf.items[0], WmfItem::Path { subpaths, .. } if subpaths[0].len() == 2));
    }

    #[test]
    fn drawing_after_a_clip_path_bracket_is_drawn() {
        // Office's pattern: a clip rectangle in a path bracket, then the real drawing.
        let bytes = emf(&[
            rec(59, &[]),
            rec(43, &[0, 0, 100, 100]),
            rec(60, &[]),
            rec(67, &[]),
            rec(43, &[10, 10, 20, 20]),
            rec(54, &[30, 30]),
        ]);
        let wmf = parse(&bytes).unwrap();
        assert_eq!(wmf.items.len(), 2, "the rectangle and the line after the clip: {:?}", wmf.items);
    }

    #[test]
    fn a_flattened_path_is_still_stroked_and_a_clip_path_is_not_drawn() {
        let bytes = emf(&[
            // Begin, a line, End, FlattenPath, StrokePath: the line is drawn.
            rec(59, &[]),
            rec(27, &[0, 0]),
            rec(54, &[50, 50]),
            rec(60, &[]),
            rec(65, &[]),
            rec(64, &[]),
            // A clip bracket: SelectClipPath uses the path up, so a Fill after it draws nothing.
            rec(59, &[]),
            rec(43, &[0, 0, 100, 100]),
            rec(60, &[]),
            rec(67, &[]),
            rec(62, &[]),
        ]);
        let wmf = parse(&bytes).unwrap();
        assert_eq!(wmf.items.len(), 1, "{:?}", wmf.items);
        let WmfItem::Path { closed, .. } = &wmf.items[0] else { panic!() };
        assert!(!closed, "the stroked line");
    }

    #[test]
    fn a_pie_on_a_wide_box_starts_where_its_radial_meets_the_ellipse() {
        // Box 0..200 x 0..50 (centre 100, 25); start radial at 45 degrees towards (200, 125), end straight down.
        let bytes = emf(&[rec(47, &[0, 0, 200, 50, 200, 125, 100, 200])]);
        let wmf = parse(&bytes).unwrap();
        let WmfItem::Path { subpaths, .. } = &wmf.items[0] else { panic!("{:?}", wmf.items) };
        // The ray at 45 degrees meets the ellipse x = 100 + 100 cos t, y = 25 + 25 sin t where
        // 25 sin t = 100 cos t: t = atan(4), at about (124.3, 49.3).
        let (x, y) = subpaths[0][0];
        assert!((x - 124.25).abs() < 0.5 && (y - 49.25).abs() < 0.5, "{x} {y}");
    }
}
