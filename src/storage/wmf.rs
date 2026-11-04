//! Windows Metafile (`.wmf`) pictures, as old PowerPoint decks embed clip art and drawn figures.
//! A browser can't show one, so its drawing records are read into plain vector items (filled or
//! stroked paths, text, embedded bitmaps) that the importers turn into ordinary slide elements.
//!
//! Covers the records real clip art and figures use (MS-WMF): window origin/extent, pens, brushes,
//! fonts, object selection and deletion, saved device contexts, lines, polylines, polygons,
//! poly-polygons, rectangles, rounded rectangles, ellipses, arcs, pies, chords, `TextOut`,
//! `ExtTextOut`, and device-independent bitmaps (`StretchDIB`, `DIBStretchBlt`). Anything else
//! (clipping, raster operations, escapes) is skipped.

/// One drawn thing, in the metafile's own logical coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum WmfItem {
    /// A path in logical coordinates: subpaths of points, closed or open.
    Path { subpaths: Vec<Vec<(f64, f64)>>, closed: bool, fill: Option<String>, stroke: Option<(String, f64)> },
    /// An elliptical arc drawn counter-clockwise from `from` to `to` (a whole ellipse when they
    /// meet); `closed` joins the ends (a chord), or runs through the centre when `pie`.
    Arc { center: (f64, f64), rx: f64, ry: f64, from: (f64, f64), to: (f64, f64), large: bool, closed: bool, pie: bool, fill: Option<String>, stroke: Option<(String, f64)> },
    /// Text whose anchor point is `(x, y)`; `size` is the character height in logical units.
    Text { x: f64, y: f64, text: String, size: f64, color: String, font: String, bold: bool, italic: bool, align: &'static str, baseline: bool, angle_deg: f64 },
    /// A bitmap as a BMP file, drawn into `(x, y, w, h)`.
    Bitmap { x: f64, y: f64, w: f64, h: f64, bmp: Vec<u8> },
}

/// A parsed metafile: what it draws, and the logical rectangle it draws into (`x, y, w, h`).
#[derive(Debug)]
pub struct Wmf {
    pub items: Vec<WmfItem>,
    pub frame: (f64, f64, f64, f64),
}

pub fn is_wmf(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) || (bytes.len() >= 18 && matches!(u16le(bytes, 0), Some(1 | 2)) && u16le(bytes, 2) == Some(9))
}

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn i16le(b: &[u8], at: usize) -> Option<i16> {
    u16le(b, at).map(|v| v as i16)
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

fn colorref(b: &[u8], at: usize) -> Option<String> {
    Some(format!("#{:02x}{:02x}{:02x}", b.get(at)?, b.get(at + 1)?, b.get(at + 2)?))
}

#[derive(Clone)]
enum Obj {
    Pen { color: String, width: f64, null: bool },
    Brush { color: String, null: bool },
    Font { height: f64, weight: u16, italic: bool, face: String, escapement: f64 },
    Other,
}

#[derive(Clone)]
struct Dc {
    pen: Option<(String, f64)>,
    brush: Option<String>,
    font: (f64, u16, bool, String, f64),
    text_color: String,
    text_align: u16,
    pos: (f64, f64),
    win_org: (f64, f64),
    win_ext: Option<(f64, f64)>,
}

/// Windows-1252 bytes to text (what `TextOut` strings in Western clip art are).
fn cp1252(bytes: &[u8]) -> String {
    const HIGH: [char; 32] = ['€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ'];
    bytes.iter().map(|&b| if (0x80..0xA0).contains(&b) { HIGH[(b - 0x80) as usize] } else { b as char }).collect()
}

/// Reads a metafile. `None` when it isn't one, or draws nothing this understands.
pub fn parse(bytes: &[u8]) -> Option<Wmf> {
    let mut at = 0usize;
    let mut placeable: Option<(f64, f64, f64, f64)> = None;
    if bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
        let (l, t, r, b) = (i16le(bytes, 6)? as f64, i16le(bytes, 8)? as f64, i16le(bytes, 10)? as f64, i16le(bytes, 12)? as f64);
        placeable = Some((l.min(r), t.min(b), (r - l).abs(), (b - t).abs()));
        at = 22;
    }
    let header_words = u16le(bytes, at + 2)? as usize;
    if header_words != 9 {
        return None;
    }
    at += header_words * 2;

    let mut objects: Vec<Option<Obj>> = Vec::new();
    let mut dc = Dc { pen: Some(("#000000".into(), 1.0)), brush: Some("#ffffff".into()), font: (12.0, 400, false, "Arial".into(), 0.0), text_color: "#000000".into(), text_align: 0, pos: (0.0, 0.0), win_org: (0.0, 0.0), win_ext: None };
    let mut saved: Vec<Dc> = Vec::new();
    let mut items = Vec::new();
    let mut first_window: Option<((f64, f64), (f64, f64))> = None;

    while at + 6 <= bytes.len() {
        let size_words = u32le(bytes, at)? as usize;
        let func = u16le(bytes, at + 4)?;
        if size_words < 3 || func == 0 {
            break;
        }
        let end = (at + size_words * 2).min(bytes.len());
        let p = &bytes[at + 6..end];
        let i = |k: usize| i16le(p, k * 2).map(|v| v as f64);
        match func {
            0x020B => dc.win_org = (i(1).unwrap_or(0.0), i(0).unwrap_or(0.0)),
            0x020C => {
                dc.win_ext = Some((i(1).unwrap_or(1.0), i(0).unwrap_or(1.0)));
                if first_window.is_none() {
                    first_window = Some((dc.win_org, dc.win_ext.unwrap()));
                }
            }
            0x02FA | 0x02FC | 0x02FB | 0x00F7 | 0x01F9 | 0x06FF | 0x0142 | 0x01F8 => {
                let obj = match func {
                    0x02FA => Obj::Pen { null: u16le(p, 0) == Some(5), width: i(1).unwrap_or(0.0).abs(), color: colorref(p, 6).unwrap_or_else(|| "#000000".into()) },
                    0x02FC => Obj::Brush { null: u16le(p, 0) == Some(1), color: colorref(p, 2).unwrap_or_else(|| "#000000".into()) },
                    0x02FB => {
                        let face_bytes = p.get(18..p.len().min(18 + 32)).unwrap_or(&[]);
                        let face = cp1252(face_bytes.split(|&b| b == 0).next().unwrap_or(&[]));
                        Obj::Font { height: i(0).unwrap_or(12.0).abs(), escapement: i(2).unwrap_or(0.0) / 10.0, weight: u16le(p, 8).unwrap_or(400), italic: p.get(10).is_some_and(|&b| b != 0), face }
                    }
                    _ => Obj::Other,
                };
                match objects.iter().position(Option::is_none) {
                    Some(slot) => objects[slot] = Some(obj),
                    None => objects.push(Some(obj)),
                }
            }
            0x012D => {
                if let Some(Some(obj)) = u16le(p, 0).and_then(|k| objects.get(k as usize)) {
                    match obj.clone() {
                        Obj::Pen { color, width, null } => dc.pen = (!null).then_some((color, width)),
                        Obj::Brush { color, null } => dc.brush = (!null).then_some(color),
                        Obj::Font { height, weight, italic, face, escapement } => dc.font = (height, weight, italic, face, escapement),
                        Obj::Other => {}
                    }
                }
            }
            0x01F0 => {
                if let Some(slot) = u16le(p, 0).and_then(|k| objects.get_mut(k as usize)) {
                    *slot = None;
                }
            }
            0x001E => saved.push(dc.clone()),
            0x0127 => {
                // RestoreDC(-n) pops n levels.
                let n = i(0).unwrap_or(-1.0).abs().max(1.0) as usize;
                for _ in 0..n {
                    if let Some(d) = saved.pop() {
                        dc = d;
                    }
                }
            }
            0x0209 => dc.text_color = colorref(p, 0).unwrap_or_else(|| "#000000".into()),
            0x012E => dc.text_align = u16le(p, 0).unwrap_or(0),
            0x0214 => dc.pos = (i(1).unwrap_or(0.0), i(0).unwrap_or(0.0)),
            0x0213 => {
                let to = (i(1).unwrap_or(0.0), i(0).unwrap_or(0.0));
                if dc.pen.is_some() {
                    items.push(WmfItem::Path { subpaths: vec![vec![dc.pos, to]], closed: false, fill: None, stroke: dc.pen.clone() });
                }
                dc.pos = to;
            }
            0x0325 | 0x0324 => {
                let n = u16le(p, 0).unwrap_or(0) as usize;
                let pts: Vec<(f64, f64)> = (0..n).map_while(|k| Some((i(1 + 2 * k)?, i(2 + 2 * k)?))).collect();
                if pts.len() >= 2 {
                    let closed = func == 0x0324;
                    items.push(WmfItem::Path { subpaths: vec![pts], closed, fill: if closed { dc.brush.clone() } else { None }, stroke: dc.pen.clone() });
                }
            }
            0x0538 => {
                let polys = u16le(p, 0).unwrap_or(0) as usize;
                // Every loop stops at the record's end, whatever counts it declares.
                let counts: Vec<usize> = (0..polys).map_while(|k| u16le(p, 2 + 2 * k).map(|c| c as usize)).collect();
                let mut k = 1 + polys;
                let mut subpaths = Vec::new();
                for c in counts {
                    let pts: Vec<(f64, f64)> = (0..c).map_while(|j| Some((i(k + 2 * j)?, i(k + 2 * j + 1)?))).collect();
                    let short = pts.len() < c;
                    k += 2 * c;
                    if pts.len() >= 2 {
                        subpaths.push(pts);
                    }
                    if short {
                        break;
                    }
                }
                if !subpaths.is_empty() {
                    items.push(WmfItem::Path { subpaths, closed: true, fill: dc.brush.clone(), stroke: dc.pen.clone() });
                }
            }
            0x041B | 0x0418 | 0x061C => {
                // Rectangle/Ellipse: bottom, right, top, left. RoundRect: height, width first.
                let o = if func == 0x061C { 2 } else { 0 };
                let (b, r, t, l) = (i(o).unwrap_or(0.0), i(o + 1).unwrap_or(0.0), i(o + 2).unwrap_or(0.0), i(o + 3).unwrap_or(0.0));
                let (fill, stroke) = (dc.brush.clone(), dc.pen.clone());
                if func == 0x0418 {
                    let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + b) / 2.0, (r - l).abs() / 2.0, (b - t).abs() / 2.0);
                    items.push(WmfItem::Arc { center: (cx, cy), rx, ry, from: (cx + rx, cy), to: (cx + rx, cy), large: true, closed: true, pie: false, fill, stroke });
                } else {
                    items.push(WmfItem::Path { subpaths: vec![vec![(l, t), (r, t), (r, b), (l, b)]], closed: true, fill, stroke });
                }
            }
            0x0817 | 0x081A | 0x0830 => {
                // Arc/Pie/Chord: yEnd, xEnd, yStart, xStart, bottom, right, top, left; counter-clockwise.
                let (ye, xe, ys, xs, b, r, t, l) = (i(0)?, i(1)?, i(2)?, i(3)?, i(4)?, i(5)?, i(6)?, i(7)?);
                let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + b) / 2.0, (r - l).abs() / 2.0, (b - t).abs() / 2.0);
                let on = |x: f64, y: f64| {
                    let a = (y - cy).atan2(x - cx);
                    ((cx + rx * a.cos(), cy + ry * a.sin()), a)
                };
                let ((from, a0), (to, a1)) = (on(xs, ys), on(xe, ye));
                // Counter-clockwise on screen (y down) is decreasing angle.
                let sweep = (a0 - a1).rem_euclid(std::f64::consts::TAU);
                let closed = func != 0x0817;
                items.push(WmfItem::Arc {
                    center: (cx, cy),
                    rx,
                    ry,
                    from,
                    to,
                    large: sweep > std::f64::consts::PI,
                    closed,
                    pie: func == 0x081A,
                    fill: if closed { dc.brush.clone() } else { None },
                    stroke: dc.pen.clone(),
                });
            }
            0x0521 => {
                let n = u16le(p, 0).unwrap_or(0) as usize;
                let text = cp1252(p.get(2..2 + n).unwrap_or(&[]));
                let k = (2 + n).div_ceil(2);
                if let (Some(y), Some(x)) = (i(k), i(k + 1)) {
                    push_text(&mut items, &dc, x, y, text);
                }
            }
            0x0A32 => {
                let (y, x, n, opts) = (i(0).unwrap_or(0.0), i(1).unwrap_or(0.0), u16le(p, 4).unwrap_or(0) as usize, u16le(p, 6).unwrap_or(0));
                let start = if opts & 0x0006 != 0 { 16 } else { 8 };
                let text = cp1252(p.get(start..start + n).unwrap_or(&[]));
                push_text(&mut items, &dc, x, y, text);
            }
            0x0B41 | 0x0F43 => {
                // DIBStretchBlt: rop(4), srcH, srcW, ySrc, xSrc, destH, destW, yDest, xDest, DIB.
                // StretchDIB: rop(4), colorUsage, then the same.
                let o = if func == 0x0F43 { 6 } else { 4 };
                let g = |k: usize| i16le(p, o + 2 * k).map(|v| v as f64);
                if let (Some(h), Some(w), Some(y), Some(x)) = (g(4), g(5), g(6), g(7)) {
                    if let Some(bmp) = p.get(o + 16..).and_then(dib_to_bmp) {
                        items.push(WmfItem::Bitmap { x: x.min(x + w), y: y.min(y + h), w: w.abs(), h: h.abs(), bmp });
                    }
                }
            }
            _ => {}
        }
        at += size_words * 2;
    }

    if items.is_empty() {
        return None;
    }
    let frame = match (first_window, placeable) {
        (Some(((ox, oy), (ex, ey))), _) if ex != 0.0 && ey != 0.0 => (ox.min(ox + ex), oy.min(oy + ey), ex.abs(), ey.abs()),
        (_, Some(b)) if b.2 > 0.0 && b.3 > 0.0 => b,
        _ => return None,
    };
    // A negative window extent flips the drawing: mirror the items into the frame's direction.
    let (flip_x, flip_y) = first_window.map(|(_, (ex, ey))| (ex < 0.0, ey < 0.0)).unwrap_or((false, false));
    if flip_x || flip_y {
        let (fx, fy, fw, fh) = frame;
        let m = |(x, y): (f64, f64)| (if flip_x { 2.0 * fx + fw - x } else { x }, if flip_y { 2.0 * fy + fh - y } else { y });
        for it in items.iter_mut() {
            match it {
                WmfItem::Path { subpaths, .. } => subpaths.iter_mut().flatten().for_each(|p| *p = m(*p)),
                WmfItem::Arc { from, to, center, .. } => {
                    *from = m(*from);
                    *to = m(*to);
                    *center = m(*center);
                }
                WmfItem::Text { x, y, .. } => (*x, *y) = m((*x, *y)),
                WmfItem::Bitmap { x, y, w, h, .. } => {
                    let (nx, ny) = m((*x + if flip_x { *w } else { 0.0 }, *y + if flip_y { *h } else { 0.0 }));
                    (*x, *y) = (nx, ny);
                }
            }
        }
    }
    Some(Wmf { items, frame })
}

fn push_text(items: &mut Vec<WmfItem>, dc: &Dc, x: f64, y: f64, text: String) {
    let text = text.trim_end_matches('\0').to_string();
    if text.trim().is_empty() {
        return;
    }
    let (size, weight, italic, face, angle) = dc.font.clone();
    let a = dc.text_align;
    items.push(WmfItem::Text {
        x,
        y,
        text,
        size: if size > 0.0 { size } else { 12.0 },
        color: dc.text_color.clone(),
        font: if face.is_empty() { "Arial".into() } else { face },
        bold: weight >= 600,
        italic,
        align: match a & 0x0006 { 6 => "center", 2 => "right", _ => "left" },
        baseline: a & 0x0018 == 0x0018,
        angle_deg: angle,
    });
}

/// A packed DIB (BITMAPINFOHEADER, palette, pixels) as a BMP file.
fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let header = u32le(dib, 0)? as usize;
    if !(12..=124).contains(&header) || dib.len() <= header {
        return None;
    }
    let bit_count = u16le(dib, 14)? as usize;
    let compression = u32le(dib, 16).unwrap_or(0);
    let mut colors = u32le(dib, 32).unwrap_or(0) as usize;
    if colors == 0 && bit_count <= 8 {
        colors = 1 << bit_count;
    }
    let masks = if compression == 3 && header == 40 { 12 } else { 0 };
    let offset = 14 + header + masks + colors * 4;
    let total = 14 + dib.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(func: u16, params: &[u8]) -> Vec<u8> {
        let mut params = params.to_vec();
        if !params.len().is_multiple_of(2) {
            params.push(0);
        }
        let mut r = ((3 + params.len() / 2) as u32).to_le_bytes().to_vec();
        r.extend_from_slice(&func.to_le_bytes());
        r.extend_from_slice(&params);
        r
    }

    fn record(func: u16, params: &[i16]) -> Vec<u8> {
        raw(func, &params.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>())
    }

    fn metafile(records: &[Vec<u8>]) -> Vec<u8> {
        let mut b = vec![1, 0, 9, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        records.iter().for_each(|r| b.extend_from_slice(r));
        b.extend_from_slice(&record(0, &[]));
        b
    }

    #[test]
    fn reads_window_brushes_pens_shapes_and_text() {
        let red_brush = raw(0x02FC, &[0, 0, 0xff, 0, 0, 0, 0, 0]);
        let null_pen = raw(0x02FA, &[5, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let text = raw(0x0521, &[2, 0, b'H', b'i', 50, 0, 10, 0]);
        let wmf = parse(&metafile(&[
            record(0x020B, &[0, 0]),
            record(0x020C, &[200, 400]),
            red_brush,
            null_pen,
            record(0x012D, &[0]),
            record(0x012D, &[1]),
            record(0x0324, &[3, 0, 0, 100, 0, 50, 80]),
            text,
        ])).unwrap();
        assert_eq!(wmf.frame, (0.0, 0.0, 400.0, 200.0));
        assert_eq!(wmf.items[0], WmfItem::Path { subpaths: vec![vec![(0.0, 0.0), (100.0, 0.0), (50.0, 80.0)]], closed: true, fill: Some("#ff0000".into()), stroke: None });
        assert!(matches!(&wmf.items[1], WmfItem::Text { text, x, y, .. } if text == "Hi" && *x == 10.0 && *y == 50.0));
    }

    #[test]
    fn truncated_or_corrupted_metafiles_never_panic() {
        let mut good = metafile(&[
            record(0x020C, &[200, 400]),
            raw(0x02FC, &[0, 0, 0xff, 0, 0, 0, 0, 0]),
            record(0x012D, &[0]),
            record(0x0538, &[2, 3, 3, 0, 0, 9, 0, 5, 5, 1, 1, 8, 1, 4, 4]),
            record(0x0817, &[0, 10, 10, 0, 20, 20, 0, 0]),
            raw(0x0A32, &[5, 0, 6, 0, 3, 0, 6, 0, 0, 0, 0, 0, 9, 0, 9, 0, b'a', b'b', b'c']),
            raw(0x0B41, &[0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0, 0, 40, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 24, 0]),
        ]);
        assert!(parse(&good).is_some());
        for len in 0..good.len() {
            let _ = parse(&good[..len]);
        }
        // A deterministic scramble of every byte position, many times over.
        let mut seed: u32 = 0x9e37_79b9;
        for _ in 0..20_000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let at = seed as usize % good.len();
            let old = good[at];
            good[at] = (seed >> 8) as u8;
            let _ = parse(&good);
            good[at] = old;
        }
    }

    #[test]
    fn declared_counts_beyond_the_record_cost_nothing() {
        // 65535 polygons of 65535 points each, in a record holding six points.
        let start = std::time::Instant::now();
        let mut params = vec![-1i16];
        params.extend(std::iter::repeat(-1i16).take(3));
        params.extend([0, 0, 5, 0, 5, 5]);
        let _ = parse(&metafile(&[record(0x020C, &[10, 10]), record(0x0538, &params)]));
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn garbage_is_not_a_metafile() {
        assert!(parse(b"not a metafile at all").is_none());
        assert!(parse(&[0xD7, 0xCD, 0xC6, 0x9A, 0, 0]).is_none());
        assert!(!is_wmf(b"\x89PNG"));
    }
}
