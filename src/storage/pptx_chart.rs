//! Charts (`ppt/charts/chartN.xml`) drawn as a group of ordinary elements from the values the
//! chart caches with its data: bars (column or horizontal; clustered, stacked or percent-stacked;
//! 3-D bars drawn flat), lines, and pie or doughnut wedges, with gridlines, axis labels, value
//! labels, a title and a legend. Series take their own fill, else the theme's accent colours in
//! turn. A child of `pptx_import`, sharing its context and element helpers.

use super::*;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Bar { horizontal: bool },
    Line,
    Pie { hole: f64 },
}

struct Series {
    name: String,
    values: Vec<Option<f64>>,
    /// A scatter series' own x values (`<c:xVal>`), when they are numbers.
    x: Option<Vec<Option<f64>>>,
    color: Option<String>,
    /// Single points' own colours (`<c:dPt>`), by index.
    points: Vec<Option<String>>,
    labels: bool,
}

impl Series {
    fn point_color(&self, i: usize) -> Option<String> {
        self.points.get(i).cloned().flatten()
    }
}

struct Group {
    kind: Kind,
    /// "clustered", "stacked", "percentStacked" or "standard".
    grouping: String,
    gap: f64,
    vary_colors: bool,
    series: Vec<Series>,
}

/// The cached points of a `<c:strCache>`/`<c:numCache>`, in index order.
fn cache_points(xml: &str) -> Vec<Option<String>> {
    let cache = next_tag(xml, "c:strCache").or_else(|| next_tag(xml, "c:numCache")).map(|(_, c, _)| c).unwrap_or("");
    let count = next_tag(cache, "c:ptCount").and_then(|(a, _, _)| extract_attr(a, "val")?.parse::<usize>().ok()).unwrap_or(0).min(10_000);
    let mut out = vec![None; count];
    for (_, a, inner) in children(cache).into_iter().filter(|(n, _, _)| *n == "c:pt") {
        let Some(i) = extract_attr(a, "idx").and_then(|v| v.parse::<usize>().ok()).filter(|i| *i < 10_000) else { continue };
        if i >= out.len() {
            out.resize(i + 1, None);
        }
        out[i] = next_tag_content(inner, "c:v").map(|(v, _)| decode_xml_entities(v));
    }
    out
}

fn shows_values(xml: &str) -> Option<bool> {
    let (_, d, _) = next_tag(xml, "c:dLbls")?;
    let on = |t: &str| next_tag(d, t).and_then(|(a, _, _)| extract_attr(a, "val")).map(|v| v == "1");
    Some(on("c:showVal").unwrap_or(false) || on("c:showPercent").unwrap_or(false))
}

fn parse_groups(plot: &str, ctx: &Ctx) -> Vec<Group> {
    let mut groups = Vec::new();
    for (name, _, inner) in children(plot) {
        let kind = match name {
            "c:barChart" | "c:bar3DChart" => {
                let dir = next_tag(inner, "c:barDir").and_then(|(a, _, _)| extract_attr(a, "val")).unwrap_or("col");
                Kind::Bar { horizontal: dir == "bar" }
            }
            "c:lineChart" | "c:line3DChart" | "c:areaChart" | "c:area3DChart" | "c:radarChart" | "c:scatterChart" => Kind::Line,
            "c:pieChart" | "c:pie3DChart" | "c:ofPieChart" => Kind::Pie { hole: 0.0 },
            "c:doughnutChart" => Kind::Pie { hole: next_tag(inner, "c:holeSize").and_then(|(a, _, _)| extract_attr(a, "val")?.parse::<f64>().ok()).unwrap_or(50.0) / 100.0 },
            _ => continue,
        };
        let val = |t: &str| next_tag(inner, t).and_then(|(a, _, _)| extract_attr(a, "val").map(str::to_string));
        let chart_labels = children(inner).into_iter().find(|(n, _, _)| *n == "c:dLbls").and_then(|(_, _, d)| shows_values(&format!("<c:dLbls>{d}</c:dLbls>")));
        let series = children(inner).into_iter().filter(|(n, _, _)| *n == "c:ser").map(|(_, _, s)| {
            let name = next_tag(s, "c:tx").map(|(_, t, _)| cache_points(t).into_iter().flatten().collect::<String>())
                .or_else(|| next_tag(s, "c:tx").and_then(|(_, t, _)| next_tag_content(t, "c:v").map(|(v, _)| decode_xml_entities(v))))
                .unwrap_or_default();
            let values = next_tag(s, "c:val").or_else(|| next_tag(s, "c:yVal")).map(|(_, v, _)| cache_points(v)).unwrap_or_default()
                .into_iter().map(|v| v.and_then(|v| v.trim().parse::<f64>().ok())).collect();
            // The series' own fill (a bar's), else its line's colour (a line's).
            let sppr = children(s).into_iter().find(|(n, _, _)| *n == "c:spPr").map(|(_, _, i)| i).unwrap_or("");
            let color = children(sppr).into_iter().find(|(n, _, _)| matches!(*n, "a:solidFill" | "a:gradFill" | "a:pattFill")).and_then(|(n, _, i)| fill_color(n, i, ctx, None))
                .or_else(|| next_tag(sppr, "a:ln").and_then(|(_, l, _)| next_tag(l, "a:solidFill")).and_then(|(_, f, _)| ctx.palette().first_color(f, None)));
            let labels = shows_values(s).or(chart_labels).unwrap_or(false);
            let mut points: Vec<Option<String>> = Vec::new();
            for (_, _, dpt) in children(s).into_iter().filter(|(n, _, _)| *n == "c:dPt") {
                let Some(i) = next_tag(dpt, "c:idx").and_then(|(a, _, _)| extract_attr(a, "val")?.parse::<usize>().ok()).filter(|i| *i < 10_000) else { continue };
                let sppr = next_tag(dpt, "c:spPr").map(|(_, i, _)| i).unwrap_or("");
                let fill = children(sppr).into_iter().find(|(n, _, _)| matches!(*n, "a:solidFill" | "a:gradFill" | "a:pattFill")).and_then(|(n, _, i)| fill_color(n, i, ctx, None));
                if i >= points.len() {
                    points.resize(i + 1, None);
                }
                points[i] = fill;
            }
            let x = next_tag(s, "c:xVal").map(|(_, v, _)| cache_points(v).into_iter().map(|v| v.and_then(|v| v.trim().parse::<f64>().ok())).collect::<Vec<_>>())
                .filter(|xs| xs.iter().any(Option::is_some));
            Series { name, values, x, color, points, labels }
        }).collect::<Vec<_>>();
        if series.is_empty() {
            continue;
        }
        groups.push(Group {
            kind,
            grouping: val("c:grouping").unwrap_or_else(|| "clustered".into()),
            gap: val("c:gapWidth").and_then(|v| v.parse::<f64>().ok()).unwrap_or(150.0) / 100.0,
            vary_colors: val("c:varyColors").as_deref() == Some("1"),
            series,
        });
    }
    groups
}

/// The colours a chart's series (or a pie's slices) get unless they set their own, from the chart
/// style (`<c:style>`, 1-48, cycling every eight): greyscale, the six accents in turn (the default),
/// or one accent shaded from dark to light.
fn series_colors(ctx: &Ctx, style: u32, n: usize) -> Vec<String> {
    let pal = ctx.palette();
    let theme = |slot: &str, fallback: &str| ctx.theme.colors.get(slot).cloned().unwrap_or_else(|| fallback.to_string());
    let fallbacks = ["#4f81bd", "#c0504d", "#9bbb59", "#8064a2", "#4bacc6", "#f79646"];
    let variant = (style.max(1) - 1) % 8;
    let n = n.max(1);
    let ramp = |base: String| -> Vec<String> {
        // Darkest first, as Office orders a single-colour chart's series.
        (0..n).map(|i| {
            let t = if n == 1 { 0.5 } else { i as f64 / (n - 1) as f64 };
            let hex = base.trim_start_matches('#').to_string();
            let modifier = if t < 0.5 { format!(r#"<a:shade val="{}"/>"#, ((0.5 + t) * 100_000.0) as i64) } else { format!(r#"<a:tint val="{}"/>"#, ((1.5 - t) * 100_000.0) as i64) };
            pal.color("a:srgbClr", &format!(r#"val="{hex}""#), &modifier, None).unwrap_or(base.clone())
        }).collect()
    };
    match variant {
        0 => ramp("#808080".into()),
        1 => (0..n).map(|i| {
            let base = theme(&format!("accent{}", i % 6 + 1), fallbacks[i % 6]);
            // A second round of the six is drawn darker.
            if i >= 6 { pal.color("a:srgbClr", &format!(r#"val="{}""#, base.trim_start_matches('#')), r#"<a:lumMod val="60000"/>"#, None).unwrap_or(base) } else { base }
        }).collect(),
        v => ramp(theme(&format!("accent{}", v - 1), fallbacks[(v as usize - 2) % 6])),
    }
}

/// A round axis step for about `n` divisions of `range`.
fn nice_step(range: f64, n: f64) -> f64 {
    if range <= 0.0 || !range.is_finite() {
        return 1.0;
    }
    let raw = range / n;
    let mag = 10f64.powf(raw.log10().floor());
    let f = raw / mag;
    let nice = if f <= 1.0 { 1.0 } else if f <= 2.0 { 2.0 } else if f <= 2.5 { 2.5 } else if f <= 5.0 { 5.0 } else { 10.0 };
    nice * mag
}

fn number(v: f64, percent: bool) -> String {
    let v = if percent { v * 100.0 } else { v };
    let s = if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{:.1}", v).trim_end_matches('0').trim_end_matches('.').to_string() };
    if percent { format!("{s}%") } else { s }
}

struct Draw<'a, 'b> {
    ctx: &'a Ctx<'b>,
    out: Vec<SlideElement>,
    size_pt: f64,
    colors: Vec<String>,
}

impl Draw<'_, '_> {
    fn color(&self, i: usize) -> String {
        self.colors[i % self.colors.len()].clone()
    }
}

impl Draw<'_, '_> {
    fn id() -> String {
        new_id()
    }

    fn rect(&mut self, r: [f64; 4], fill: &str, stroke: Option<&str>) {
        if r[2] <= 0.0 || r[3] <= 0.0 {
            return;
        }
        self.out.push(SlideElement::Shape { id: Self::id(), transform: self.ctx.geo.transform(r, 0.0), shape_kind: "rect".into(), fill_color: fill.into(), stroke_color: stroke.map(str::to_string), stroke_width: if stroke.is_some() { 1.0 } else { 0.0 }, path: None });
    }

    fn line(&mut self, (x1, y1): (f64, f64), (x2, y2): (f64, f64), color: &str, width_emu: f64) {
        let (x, y, w, h) = (x1.min(x2), y1.min(y2), (x2 - x1).abs(), (y2 - y1).abs());
        let p = |px: f64, py: f64| (if w > 0.0 { (px - x) / w * 100.0 } else { 0.0 }, if h > 0.0 { (py - y) / h * 100.0 } else { 0.0 });
        let (a, b) = (p(x1, y1), p(x2, y2));
        let d = format!("M {} {} L {} {}", PathData::n(a.0), PathData::n(a.1), PathData::n(b.0), PathData::n(b.1));
        self.out.push(SlideElement::Line { id: Self::id(), transform: self.ctx.geo.transform([x, y, w, h], 0.0), line_kind: "straight".into(), color: color.into(), stroke_width: canvas_stroke(width_emu, self.ctx), start_arrow: false, end_arrow: false, path: Some(d) });
    }

    fn path(&mut self, box_: [f64; 4], d: String, fill: Option<&str>, stroke: Option<(&str, f64)>) {
        let transform = self.ctx.geo.transform(box_, 0.0);
        match fill {
            Some(f) => self.out.push(SlideElement::Shape { id: Self::id(), transform, shape_kind: "path".into(), fill_color: f.into(), stroke_color: stroke.map(|s| s.0.to_string()), stroke_width: stroke.map_or(0.0, |s| canvas_stroke(s.1, self.ctx)), path: Some(d) }),
            None => if let Some((c, w)) = stroke {
                self.out.push(SlideElement::Line { id: Self::id(), transform, line_kind: "curved".into(), color: c.into(), stroke_width: canvas_stroke(w, self.ctx), start_arrow: false, end_arrow: false, path: Some(d) });
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, r: [f64; 4], text: &str, scale: f64, color: &str, align: &str, valign: &str, bold: bool) {
        if text.trim().is_empty() || r[2] <= 0.0 {
            return;
        }
        let run = crate::core::models::TextRun {
            text: text.to_string(),
            bold,
            color: color.to_string(),
            font_family: self.ctx.font(None),
            font_size_pt: ((self.size_pt * scale * self.ctx.geo.font_scale) * 10.0).round() / 10.0,
            ..Default::default()
        };
        self.out.push(SlideElement::TextBlock {
            id: Self::id(),
            transform: self.ctx.geo.transform(r, 0.0),
            block: crate::core::models::TextBlock {
                runs: vec![run],
                paragraph_style: crate::core::models::TextParagraphStyle { align: align.into(), line_height: 1.1, vertical_align: Some(valign.into()), ..Default::default() },
                effects: Default::default(),
                autofit: false,
            },
        });
    }
}

/// Draws the chart in `chart_xml` into the slide rectangle `rect` (EMU). `None` if it has no data.
pub(super) fn chart_group(chart_xml: &str, rect: [f64; 4], ctx: &Ctx) -> Option<SlideElement> {
    let (_, chart, _) = next_tag(chart_xml, "c:chart")?;
    let (_, plot, _) = next_tag(chart, "c:plotArea")?;
    let groups = parse_groups(plot, ctx);
    if groups.is_empty() {
        return None;
    }
    let first_ser = children(plot).into_iter().find_map(|(n, _, i)| if n.ends_with("Chart") { children(i).into_iter().find(|(n, _, _)| *n == "c:ser").map(|(_, _, s)| s) } else { None }).unwrap_or("");
    let cats: Vec<String> = next_tag(first_ser, "c:cat").or_else(|| next_tag(first_ser, "c:xVal")).map(|(_, c, _)| cache_points(c).into_iter().map(Option::unwrap_or_default).collect()).unwrap_or_default();
    let ncat = groups.iter().flat_map(|g| g.series.iter().map(|s| s.values.len())).max().unwrap_or(0).max(cats.len()).max(1);

    // Text size: the chart's own default, else 10pt; the title a size up.
    let size_pt = next_tag(chart_xml, "c:txPr").and_then(|(_, t, _)| next_tag(t, "a:defRPr")).and_then(|(a, _, _)| extract_attr(a, "sz")?.parse::<f64>().ok()).map(|v| v / 100.0).unwrap_or(10.0);
    let style = next_tag(chart_xml, "c:style").and_then(|(a, _, _)| extract_attr(a, "val")?.parse::<u32>().ok()).unwrap_or(2);
    let needed = groups.iter().map(|g| g.series.len()).sum::<usize>().max(ncat);
    let mut d = Draw { ctx, out: Vec::new(), size_pt, colors: series_colors(ctx, style, needed) };
    let [x, y, w, h] = rect;
    let em = size_pt * 12_700.0;

    // Title.
    let deleted = next_tag(chart, "c:autoTitleDeleted").and_then(|(a, _, _)| extract_attr(a, "val")) == Some("1");
    // Its own text, else (an automatic title) the single series' name.
    let title = next_tag(chart, "c:title").map(|(_, t, _)| {
        let mut s = String::new();
        let mut rest = t;
        while let Some((c, r)) = next_tag_content(rest, "a:t") {
            s.push_str(&decode_xml_entities(c));
            rest = r;
        }
        s
    }).filter(|s| !s.trim().is_empty())
      .or_else(|| (!deleted && next_tag(chart, "c:title").is_some() && groups.iter().map(|g| g.series.len()).sum::<usize>() == 1).then(|| groups[0].series[0].name.clone()));
    let mut top = y + em * 0.5;
    if let Some(t) = &title {
        d.text([x, top, w, em * 1.6], t, 1.4, "#404040", "center", "top", true);
        top += em * 2.0;
    }

    // Legend.
    let legend_pos = next_tag(chart, "c:legend").map(|(_, l, _)| next_tag(l, "c:legendPos").and_then(|(a, _, _)| extract_attr(a, "val").map(str::to_string)).unwrap_or_else(|| "r".into()));
    let pie = matches!(groups[0].kind, Kind::Pie { .. });
    let entries: Vec<(String, String)> = if pie || (groups[0].vary_colors && groups[0].series.len() == 1) {
        (0..ncat).map(|i| (cats.get(i).cloned().unwrap_or_default(), groups[0].series[0].point_color(i).unwrap_or_else(|| d.color(i)))).collect()
    } else {
        let mut k = 0;
        groups.iter().flat_map(|g| g.series.iter()).map(|s| { k += 1; (s.name.clone(), s.color.clone().unwrap_or_else(|| d.color(k - 1))) }).collect()
    };
    let (mut px, mut py, mut pw, mut ph) = (x + em * 0.5, top, w - em, y + h - top - em * 0.5);
    if let Some(pos) = &legend_pos {
        let sq = em * 0.7;
        match pos.as_str() {
            "b" | "t" => {
                let item_w = (w * 0.9 / entries.len().max(1) as f64).min(em * 12.0);
                let total = item_w * entries.len() as f64;
                let ly = if pos == "b" { y + h - em * 1.6 } else { top };
                for (i, (name, color)) in entries.iter().enumerate() {
                    let lx = x + (w - total) / 2.0 + i as f64 * item_w;
                    d.rect([lx, ly + (em * 1.2 - sq) / 2.0, sq, sq], color, None);
                    d.text([lx + sq * 1.4, ly, item_w - sq * 1.6, em * 1.4], name, 1.0, "#404040", "left", "top", false);
                }
                if pos == "b" { ph -= em * 2.0 } else { py += em * 2.0; ph -= em * 2.0 }
            }
            _ => {
                let lw = (w * 0.28).min(em * 14.0);
                let lx = if pos == "l" { x + em * 0.3 } else { x + w - lw };
                let total = entries.len() as f64 * em * 1.5;
                for (i, (name, color)) in entries.iter().enumerate() {
                    let ly = y + (h - total) / 2.0 + i as f64 * em * 1.5;
                    d.rect([lx, ly + (em * 1.2 - sq) / 2.0, sq, sq], color, None);
                    d.text([lx + sq * 1.4, ly, lw - sq * 1.6, em * 1.4], name, 1.0, "#404040", "left", "top", false);
                }
                pw -= lw + em * 0.3;
                if pos == "l" { px += lw + em * 0.3 }
            }
        }
    }
    // A manual plot layout (fractions of the chart) wins over the computed one.
    if let Some((_, ml, _)) = next_tag(plot, "c:manualLayout") {
        let v = |t: &str| next_tag(ml, t).and_then(|(a, _, _)| extract_attr(a, "val")?.parse::<f64>().ok());
        if let (Some(fx), Some(fy), Some(fw), Some(fh)) = (v("c:x"), v("c:y"), v("c:w"), v("c:h")) {
            (px, py, pw, ph) = (x + fx * w, y + fy * h, fw * w, fh * h);
        }
    }

    if let Kind::Pie { hole } = groups[0].kind {
        draw_pie(&mut d, &groups[0], [px, py, pw, ph], hole);
    } else {
        draw_axes_chart(&mut d, plot, &groups, &cats, ncat, [px, py, pw, ph]);
    }
    if d.out.is_empty() {
        return None;
    }
    for (z, el) in d.out.iter_mut().enumerate() {
        el.transform_mut().z_index = z as i32;
    }
    Some(SlideElement::Group { id: Draw::id(), transform: ctx.geo.transform(rect, 0.0), children: d.out })
}

fn draw_pie(d: &mut Draw, g: &Group, [px, py, pw, ph]: [f64; 4], hole: f64) {
    let s = &g.series[0];
    let total: f64 = s.values.iter().flatten().filter(|v| **v > 0.0).sum();
    if total <= 0.0 {
        return;
    }
    let r = pw.min(ph) / 2.0;
    let (cx, cy) = (px + pw / 2.0, py + ph / 2.0);
    let box_ = [cx - r, cy - r, 2.0 * r, 2.0 * r];
    let at = |a: f64, rr: f64| (50.0 + 50.0 * rr * a.cos(), 50.0 + 50.0 * rr * a.sin());
    let mut a0 = -std::f64::consts::FRAC_PI_2;
    for (i, v) in s.values.iter().enumerate() {
        let Some(v) = v.filter(|v| *v > 0.0) else { continue };
        let sweep = v / total * std::f64::consts::TAU;
        let a1 = a0 + sweep;
        let large = sweep > std::f64::consts::PI;
        let color = s.point_color(i).unwrap_or_else(|| d.color(i));
        let mut p = PathData::default();
        if sweep >= std::f64::consts::TAU - 1e-9 {
            // One slice is the whole pie: two half arcs.
            p.cmd('M', &[at(a0, 1.0)]);
            p.arc(50.0, 50.0, false, true, at(a0 + std::f64::consts::PI, 1.0));
            p.arc(50.0, 50.0, false, true, at(a0, 1.0));
        } else if hole > 0.0 {
            p.cmd('M', &[at(a0, 1.0)]);
            p.arc(50.0, 50.0, large, true, at(a1, 1.0));
            p.cmd('L', &[at(a1, hole)]);
            p.arc(50.0 * hole, 50.0 * hole, large, false, at(a0, hole));
        } else {
            p.cmd('M', &[(50.0, 50.0), at(a0, 1.0)]);
            p.arc(50.0, 50.0, large, true, at(a1, 1.0));
        }
        p.close();
        d.path(box_, p.0, Some(&color), Some(("#ffffff", 9_525.0)));
        if s.labels {
            let mid = a0 + sweep / 2.0;
            let rr = if hole > 0.0 { (1.0 + hole) / 2.0 } else { 0.65 };
            let (lx, ly) = (cx + r * rr * mid.cos(), cy + r * rr * mid.sin());
            let em = d.size_pt * 12_700.0;
            d.text([lx - em * 3.0, ly - em * 0.7, em * 6.0, em * 1.4], &number(*s.values.get(i).and_then(|v| v.as_ref()).unwrap_or(&0.0), false), 1.0, "#ffffff", "center", "middle", true);
        }
        a0 = a1;
    }
}

fn draw_axes_chart(d: &mut Draw, plot: &str, groups: &[Group], cats: &[String], ncat: usize, [mut px, py, mut pw, mut ph]: [f64; 4]) {
    let em = d.size_pt * 12_700.0;
    let horizontal = matches!(groups[0].kind, Kind::Bar { horizontal: true });
    let percent_stacked = groups.iter().any(|g| g.grouping == "percentStacked");
    let fmt_percent = percent_stacked || next_tag(plot, "c:valAx").and_then(|(_, v, _)| next_tag(v, "c:numFmt")).and_then(|(a, _, _)| extract_attr(a, "formatCode")).is_some_and(|f| f.contains('%'));
    let deleted = |ax: &str| next_tag(plot, ax).and_then(|(_, v, _)| next_tag(v, "c:delete")).and_then(|(a, _, _)| extract_attr(a, "val")) == Some("1");
    let gridlines = next_tag(plot, "c:valAx").is_some_and(|(_, v, _)| next_tag(v, "c:majorGridlines").is_some());

    // Each group's per-category values as drawn (stacked groups add up; percent ones to 1).
    let stacked = |g: &Group| g.grouping == "stacked" || g.grouping == "percentStacked";
    let mut lo: f64 = 0.0;
    let mut hi: f64 = 0.0;
    for g in groups {
        for c in 0..ncat {
            let vals: Vec<f64> = g.series.iter().map(|s| s.values.get(c).copied().flatten().unwrap_or(0.0)).collect();
            if g.grouping == "percentStacked" {
                hi = hi.max(1.0);
            } else if stacked(g) {
                hi = hi.max(vals.iter().filter(|v| **v > 0.0).sum());
                lo = lo.min(vals.iter().filter(|v| **v < 0.0).sum());
            } else {
                for v in vals {
                    hi = hi.max(v);
                    lo = lo.min(v);
                }
            }
        }
    }
    // A scaling the chart states (`c:min`/`c:max`) wins.
    let scale = |t: &str| next_tag(plot, "c:valAx").and_then(|(_, v, _)| next_tag(v, t)).and_then(|(a, _, _)| extract_attr(a, "val")?.parse::<f64>().ok());
    let step = nice_step(hi - lo, 5.0);
    let lo = scale("c:min").unwrap_or(if lo < 0.0 { (lo / step).floor() * step } else { 0.0 });
    let hi = scale("c:max").unwrap_or(if percent_stacked { 1.0 } else { ((hi / step).ceil() * step).max(lo + step) });
    let step = nice_step(hi - lo, 5.0);

    // Room for the value labels and the category labels.
    let label_w = em * if deleted("c:valAx") { 0.5 } else { 3.5 };
    let label_h = em * if deleted("c:catAx") { 0.5 } else { 2.6 };
    if horizontal {
        let cat_w = (pw * 0.22).min(em * 10.0);
        px += cat_w;
        pw -= cat_w;
        ph -= label_h * 0.7;
    } else {
        px += label_w;
        pw -= label_w;
        ph -= label_h;
    }
    if pw <= 0.0 || ph <= 0.0 {
        return;
    }
    // Value -> position along the value axis.
    let vpos = |v: f64| {
        let t = (v - lo) / (hi - lo);
        if horizontal { px + t * pw } else { py + ph - t * ph }
    };

    // Gridlines and value labels.
    let mut v = lo;
    let mut guard = 0;
    while v <= hi + step * 1e-6 && guard < 50 {
        let p = vpos(v);
        if horizontal {
            if gridlines { d.line((p, py), (p, py + ph), "#d9d9d9", 9_525.0) }
            if !deleted("c:valAx") { d.text([p - em * 2.5, py + ph + em * 0.2, em * 5.0, em * 1.4], &number(v, fmt_percent), 0.9, "#595959", "center", "top", false) }
        } else {
            if gridlines { d.line((px, p), (px + pw, p), "#d9d9d9", 9_525.0) }
            if !deleted("c:valAx") { d.text([px - label_w, p - em * 0.7, label_w - em * 0.4, em * 1.4], &number(v, fmt_percent), 0.9, "#595959", "right", "middle", false) }
        }
        v += step;
        guard += 1;
    }
    // The category axis line at zero.
    let zero = vpos(0.0f64.clamp(lo, hi));
    if horizontal { d.line((zero, py), (zero, py + ph), "#868686", 9_525.0) } else { d.line((px, zero), (px + pw, zero), "#868686", 9_525.0) }

    // Scatter series with numeric x values go along an x value axis instead of categories.
    let xs: Vec<f64> = groups.iter().flat_map(|g| g.series.iter()).filter_map(|s| s.x.as_ref()).flatten().flatten().copied().collect();
    let x_axis = (!horizontal && !xs.is_empty()).then(|| {
        let (lo, hi) = xs.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        let step = nice_step((hi - lo).max(f64::EPSILON), 5.0);
        let lo = if lo >= 0.0 && lo < step * 2.0 { 0.0 } else { (lo / step).floor() * step };
        (lo, ((hi / step).ceil() * step).max(lo + step), step)
    });
    let xpos = |v: f64| x_axis.map_or(px, |(lo, hi, _)| px + (v - lo) / (hi - lo) * pw);
    // A scatter's x axis is its horizontal value axis (axPos b or t), and its `c:delete` hides the labels.
    let x_deleted = children(plot).into_iter()
        .filter(|(n, _, _)| *n == "c:valAx")
        .find(|(_, _, ax)| next_tag(ax, "c:axPos").and_then(|(a, _, _)| extract_attr(a, "val")).is_some_and(|p| p == "b" || p == "t"))
        .and_then(|(_, _, ax)| next_tag(ax, "c:delete").and_then(|(a, _, _)| extract_attr(a, "val")))
        == Some("1");
    if let Some((xlo, xhi, xstep)) = x_axis {
        let mut v = xlo;
        let mut guard = 0;
        while v <= xhi + xstep * 1e-6 && guard < 50 {
            if !x_deleted {
                d.text([xpos(v) - em * 2.5, py + ph + em * 0.2, em * 5.0, em * 1.4], &number(v, false), 0.9, "#595959", "center", "top", false);
            }
            v += xstep;
            guard += 1;
        }
    }

    // Category labels.
    let band = if horizontal { ph } else { pw } / ncat as f64;
    if !deleted("c:catAx") && x_axis.is_none() {
        for (i, c) in cats.iter().enumerate().take(ncat) {
            if horizontal {
                // First category at the bottom, as Office draws horizontal bars.
                let by = py + ph - (i as f64 + 1.0) * band;
                d.text([px - (pw * 0.22).min(em * 10.0), by, (pw * 0.22).min(em * 10.0) - em * 0.4, band], c, 0.9, "#595959", "right", "middle", false);
            } else {
                d.text([px + i as f64 * band, py + ph + em * 0.2, band, label_h - em * 0.2], c, 0.9, "#595959", "center", "top", false);
            }
        }
    }

    let mut color_k = 0;
    for g in groups {
        let n = g.series.len();
        match g.kind {
            Kind::Bar { .. } => {
                let slots = if stacked(g) { 1.0 } else { n as f64 };
                let bw = band / (slots + g.gap);
                for c in 0..ncat {
                    let total: f64 = g.series.iter().map(|s| s.values.get(c).copied().flatten().unwrap_or(0.0).abs()).sum();
                    let (mut pos_acc, mut neg_acc) = (0.0, 0.0);
                    for (k, s) in g.series.iter().enumerate() {
                        let Some(raw) = s.values.get(c).copied().flatten() else { continue };
                        let val = if g.grouping == "percentStacked" && total > 0.0 { raw / total } else { raw };
                        let (start, end) = if stacked(g) {
                            let acc = if val >= 0.0 { &mut pos_acc } else { &mut neg_acc };
                            let s0 = *acc;
                            *acc += val;
                            (s0, *acc)
                        } else {
                            (0.0, val)
                        };
                        let slot = if stacked(g) { 0.0 } else { k as f64 };
                        let offset = (if horizontal { py + ph - (c as f64 + 1.0) * band } else { px + c as f64 * band }) + bw * g.gap / 2.0 + slot * bw;
                        let (a, b) = (vpos(start.clamp(lo, hi)), vpos(end.clamp(lo, hi)));
                        let r = if horizontal { [a.min(b), offset, (b - a).abs(), bw] } else { [offset, a.min(b), bw, (b - a).abs()] };
                        let color = s.point_color(c).unwrap_or_else(|| if g.vary_colors && n == 1 { d.color(c) } else { s.color.clone().unwrap_or_else(|| d.color(color_k + k)) });
                        d.rect(r, &color, None);
                        if s.labels {
                            let label = number(raw, fmt_percent && !percent_stacked);
                            if stacked(g) {
                                d.text(r, &label, 0.9, "#ffffff", "center", "middle", false);
                            } else if horizontal {
                                d.text([r[0] + r[2] + em * 0.2, r[1], em * 4.0, r[3]], &label, 0.9, "#404040", "left", "middle", false);
                            } else {
                                d.text([r[0] - em, r[1] - em * 1.4, r[2] + 2.0 * em, em * 1.4], &label, 0.9, "#404040", "center", "bottom", false);
                            }
                        }
                    }
                }
            }
            Kind::Line => {
                for (k, s) in g.series.iter().enumerate() {
                    // Each drawn point with the index of its value (a gap leaves one out).
                    let at_x = |c: usize| match (&s.x, x_axis) {
                        (Some(xs), Some(_)) => xs.get(c).copied().flatten().map(xpos),
                        _ => Some(px + (c as f64 + 0.5) * band),
                    };
                    let pts: Vec<(usize, f64, f64)> = s.values.iter().enumerate().filter_map(|(c, v)| Some((c, at_x(c)?, vpos((*v)?.clamp(lo, hi))))).collect();
                    if pts.len() < 2 {
                        continue;
                    }
                    let local = |&(_, x, y): &(usize, f64, f64)| ((x - px) / pw * 100.0, (y - py) / ph * 100.0);
                    let mut p = PathData::default();
                    p.cmd('M', &[local(&pts[0])]);
                    p.cmd('L', &pts[1..].iter().map(local).collect::<Vec<_>>());
                    let color = s.color.clone().unwrap_or_else(|| d.color(color_k + k));
                    d.path([px, py, pw, ph], p.0, None, Some((&color, 28_575.0)));
                    if s.labels {
                        for &(c, lx, ly) in &pts {
                            let label = number(s.values.get(c).copied().flatten().unwrap_or(0.0), fmt_percent);
                            d.text([lx - em * 2.0, ly - em * 1.6, em * 4.0, em * 1.4], &label, 0.9, "#404040", "center", "bottom", false);
                        }
                    }
                }
            }
            Kind::Pie { .. } => {}
        }
        color_k += n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_ctx<T>(f: impl FnOnce(&Ctx) -> T) -> T {
        let theme = Theme::parse(r#"<a:clrScheme><a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:accent1><a:srgbClr val="4F81BD"/></a:accent1><a:accent2><a:srgbClr val="C0504D"/></a:accent2></a:clrScheme>"#, "t");
        let inh = Inherited::default();
        let media = RefCell::new(HashMap::new());
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx { geo: Geo::new(12_192_000.0, 6_858_000.0), theme: &theme, map: ClrMap::default(), inh: &inh, media: &media, media_dir: dir.path(), table_styles: "", bg_fill: None };
        f(&ctx)
    }

    fn chart(grouping: &str, style: u32) -> String {
        let ser = |i: u32, name: &str, vals: [f64; 3]| format!(
            r#"<c:ser><c:idx val="{i}"/><c:tx><c:strRef><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{name}</c:v></c:pt></c:strCache></c:strRef></c:tx>
               <c:cat><c:strRef><c:strCache><c:ptCount val="3"/><c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt><c:pt idx="2"><c:v>C</c:v></c:pt></c:strCache></c:strRef></c:cat>
               <c:val><c:numRef><c:numCache><c:ptCount val="3"/><c:pt idx="0"><c:v>{}</c:v></c:pt><c:pt idx="1"><c:v>{}</c:v></c:pt><c:pt idx="2"><c:v>{}</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#, vals[0], vals[1], vals[2]);
        format!(r#"<c:chartSpace><c:style val="{style}"/><c:chart><c:autoTitleDeleted val="1"/><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="{grouping}"/>{}{}<c:gapWidth val="100"/></c:barChart><c:catAx/><c:valAx/></c:plotArea><c:legend><c:legendPos val="b"/></c:legend></c:chart></c:chartSpace>"#,
            ser(0, "First", [10.0, 20.0, 40.0]), ser(1, "Second", [30.0, 20.0, 0.0]))
    }

    fn bars(g: &SlideElement) -> Vec<(f64, f64, f64, String)> {
        let SlideElement::Group { children, .. } = g else { panic!("a group") };
        // The bars: rect shapes taller than a legend swatch.
        children.iter().filter_map(|c| match c {
            SlideElement::Shape { shape_kind, transform: t, fill_color, .. } if shape_kind == "rect" && t.h > 0.02 => Some((t.x, t.y, t.h, fill_color.clone())),
            _ => None,
        }).collect()
    }

    #[test]
    fn clustered_bars_are_as_tall_as_their_values_in_the_theme_accents() {
        with_ctx(|ctx| {
            let g = chart_group(&chart("clustered", 2), [0.0, 0.0, 9_144_000.0, 5_000_000.0], ctx).expect("a chart");
            let b = bars(&g);
            assert_eq!(b.len(), 5, "six values, one of them zero");
            let h = |i: usize| b[i].2;
            // Category A: 10 and 30; category C: 40.
            assert!((h(1) / h(0) - 3.0).abs() < 1e-6, "{b:?}");
            assert!((b.iter().map(|x| x.2).fold(0.0, f64::max) / h(0) - 4.0).abs() < 1e-6);
            assert_eq!((b[0].3.as_str(), b[1].3.as_str()), ("#4f81bd", "#c0504d"));
            let SlideElement::Group { children, .. } = &g else { unreachable!() };
            let texts: Vec<String> = children.iter().filter_map(|c| match c { SlideElement::TextBlock { block, .. } => Some(block.runs[0].text.clone()), _ => None }).collect();
            for t in ["A", "B", "C", "First", "Second"] {
                assert!(texts.contains(&t.to_string()), "{t} in {texts:?}");
            }
        });
    }

    #[test]
    fn percent_stacked_bars_fill_the_axis_and_a_single_accent_style_shades_one_colour() {
        with_ctx(|ctx| {
            // Style 4: shades of accent2.
            let g = chart_group(&chart("percentStacked", 4), [0.0, 0.0, 9_144_000.0, 5_000_000.0], ctx).expect("a chart");
            let b = bars(&g);
            // Category A (10 + 30) and B (20 + 20) both reach 100%: their stacks are equally tall.
            let stack = |x: f64| b.iter().filter(|s| (s.0 - x).abs() < 1e-9).map(|s| s.2).sum::<f64>();
            assert!((stack(b[0].0) - stack(b[2].0)).abs() < 1e-6, "{b:?}");
            assert_ne!(b[0].3, b[1].3, "two shades");
            assert!(b.iter().all(|s| s.3 != "#4f81bd"), "not the multicolour palette");
        });
    }

    #[test]
    fn axis_steps_and_labels_are_round() {
        assert_eq!(nice_step(100.0, 5.0), 20.0);
        assert_eq!(nice_step(7.0, 5.0), 2.0);
        assert_eq!(number(0.25, true), "25%");
        assert_eq!(number(12.0, false), "12");
        assert_eq!(number(3.14159, false), "3.1");
    }

    fn texts(g: &SlideElement) -> Vec<(String, f64, f64)> {
        let SlideElement::Group { children, .. } = g else { panic!("a group") };
        children.iter().filter_map(|c| match c { SlideElement::TextBlock { block, transform: t, .. } => Some((block.runs[0].text.clone(), t.x + t.w / 2.0, t.y + t.h)), _ => None }).collect()
    }

    #[test]
    fn line_labels_stay_on_their_own_points_across_a_gap() {
        with_ctx(|ctx| {
            let xml = r#"<c:chartSpace><c:chart><c:plotArea><c:lineChart><c:grouping val="standard"/><c:ser><c:dLbls><c:showVal val="1"/></c:dLbls>
                <c:val><c:numRef><c:numCache><c:ptCount val="4"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="2"><c:v>3</c:v></c:pt><c:pt idx="3"><c:v>4</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:lineChart><c:catAx/><c:valAx/></c:plotArea></c:chart></c:chartSpace>"#;
            let g = chart_group(xml, [0.0, 0.0, 9_144_000.0, 5_000_000.0], ctx).expect("a chart");
            let labels: Vec<String> = texts(&g).into_iter().map(|t| t.0).filter(|t| ["0", "1", "3", "4"].contains(&t.as_str())).collect();
            // The value-axis labels include "0"/"1"/"3"/"4" too; the point labels are the last three.
            assert_eq!(labels[labels.len() - 3..], ["1", "3", "4"], "{labels:?}");
        });
    }

    #[test]
    fn a_scatter_places_points_by_their_x_values() {
        with_ctx(|ctx| {
            let pts = |tag: &str, v: [f64; 3]| format!(r#"<{tag}><c:numRef><c:numCache><c:ptCount val="3"/><c:pt idx="0"><c:v>{}</c:v></c:pt><c:pt idx="1"><c:v>{}</c:v></c:pt><c:pt idx="2"><c:v>{}</c:v></c:pt></c:numCache></c:numRef></{tag}>"#, v[0], v[1], v[2]);
            let xml = format!(r#"<c:chartSpace><c:chart><c:plotArea><c:scatterChart><c:ser><c:dLbls><c:showVal val="1"/></c:dLbls>{}{}</c:ser></c:scatterChart><c:valAx/><c:valAx/></c:plotArea></c:chart></c:chartSpace>"#,
                pts("c:xVal", [0.0, 1.0, 10.0]), pts("c:yVal", [5.0, 6.0, 7.0]));
            let g = chart_group(&xml, [0.0, 0.0, 9_144_000.0, 5_000_000.0], ctx).expect("a chart");
            let t = texts(&g);
            let at = |label: &str| t.iter().rev().find(|x| x.0 == label).map(|x| x.1).unwrap();
            // x = 0, 1, 10: the second point sits a tenth of the way from the first to the third.
            let (a, b, c) = (at("5"), at("6"), at("7"));
            assert!(((b - a) / (c - a) - 0.1).abs() < 0.02, "{a} {b} {c}");
        });
    }
}
