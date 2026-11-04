//! OpenDocument charts (an embedded `Object N/content.xml` holding `<chart:chart>` and its own
//! data table) restated as the DrawingML chart part `pptx_chart` already draws: the same kinds
//! (bars, lines, areas, pies and rings, radar, scatter), stacking, series names and colours, value
//! labels, categories, title and legend. The values come from the chart's local table.

use crate::storage::pptx_inherit::children;
use crate::storage::xml_scan::{decode_xml_entities, encode_xml_text, extract_attr, next_tag};

/// A cell address ("$B$2", "B2") as (column, row), both from 0.
fn cell(addr: &str) -> Option<(usize, usize)> {
    let a = addr.rsplit('.').next()?.replace('$', "");
    let letters: String = a.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let row: usize = a[letters.len()..].parse().ok()?;
    let col = letters.to_ascii_uppercase().bytes().fold(0usize, |acc, b| acc * 26 + (b - b'A' + 1) as usize);
    (col >= 1 && row >= 1).then(|| (col - 1, row - 1))
}

/// A range ("local-table.$B$2:.$B$5") as the cells it covers, row by row (bounded).
fn range(r: &str) -> Vec<(usize, usize)> {
    let mut parts = r.split(':');
    let (Some(a), b) = (parts.next().and_then(cell), parts.next().and_then(cell)) else { return Vec::new() };
    let b = b.unwrap_or(a);
    let mut out = Vec::new();
    for row in a.1.min(b.1)..=a.1.max(b.1).min(a.1.min(b.1) + 1000) {
        for col in a.0.min(b.0)..=a.0.max(b.0).min(a.0.min(b.0) + 100) {
            out.push((col, row));
        }
    }
    out
}

/// The chart's local data table as rows of cell text (a number as written in `office:value`).
fn table(xml: &str) -> Vec<Vec<String>> {
    let Some((_, t, _)) = next_tag(xml, "table:table") else { return Vec::new() };
    let mut rows = Vec::new();
    let walk = |inner: &str, rows: &mut Vec<Vec<String>>| {
        for (n, _, r) in children(inner) {
            if n != "table:table-row" || rows.len() >= 1000 {
                continue;
            }
            let mut row = Vec::new();
            for (cn, ca, ci) in children(r) {
                if cn != "table:table-cell" && cn != "table:covered-table-cell" {
                    continue;
                }
                let v = extract_attr(ca, "office:value").map(str::to_string).unwrap_or_else(|| {
                    children(ci).into_iter().filter(|(n, _, _)| *n == "text:p").map(|(_, _, p)| decode_xml_entities(&strip_tags(p))).collect::<Vec<_>>().join(" ")
                });
                let n = extract_attr(ca, "table:number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(100);
                row.extend(std::iter::repeat_n(v, n));
                if row.len() > 200 {
                    break;
                }
            }
            rows.push(row);
        }
    };
    for (n, _, inner) in children(t) {
        match n {
            "table:table-header-rows" | "table:table-rows" => walk(inner, &mut rows),
            "table:table-row" => walk(&format!("<table:table-row>{inner}</table:table-row>"), &mut rows),
            _ => {}
        }
    }
    rows
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

/// A chart style's property (from any of its `style:*-properties`), by style name.
fn style_prop<'a>(doc: &'a str, name: Option<&str>, key: &str) -> Option<&'a str> {
    let name = name?;
    let (_, auto, _) = next_tag(doc, "office:automatic-styles")?;
    children(auto).into_iter().find(|(n, a, _)| *n == "style:style" && extract_attr(a, "style:name") == Some(name))
        .and_then(|(_, _, inner)| children(inner).into_iter().find_map(|(_, a, _)| extract_attr(a, key)))
}

fn cache(points: &[String], numeric: bool) -> String {
    let tag = if numeric { "c:numCache" } else { "c:strCache" };
    let pts: String = points.iter().enumerate().map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{}</c:v></c:pt>"#, encode_xml_text(v))).collect();
    format!(r#"<{tag}><c:ptCount val="{}"/>{pts}</{tag}>"#, points.len())
}

/// An ODF chart document (`Object N/content.xml`) as a DrawingML chart part, or `None` when it
/// isn't a chart this can draw.
pub fn to_drawingml(doc: &str) -> Option<String> {
    let (chart_attrs, chart, _) = next_tag(doc, "chart:chart")?;
    let class = extract_attr(chart_attrs, "chart:class").unwrap_or("chart:bar").trim_start_matches("chart:");
    let data = table(chart);
    let at = |(c, r): (usize, usize)| data.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default();
    let (_, plot_attrs_inner, _) = next_tag(chart, "chart:plot-area")?;
    let plot_attrs = next_tag(chart, "chart:plot-area").map(|(a, _, _)| a).unwrap_or("");
    let plot_style = extract_attr(plot_attrs, "chart:style-name");
    let flag = |k: &str| style_prop(doc, plot_style, k) == Some("true");

    // Categories: the x axis's range, else none (numbered).
    let cats: Vec<String> = children(plot_attrs_inner).into_iter()
        .find(|(n, a, _)| *n == "chart:axis" && extract_attr(a, "chart:dimension") == Some("x"))
        .and_then(|(_, _, ax)| next_tag(ax, "chart:categories"))
        .and_then(|(a, _, _)| extract_attr(a, "table:cell-range-address"))
        .map(|r| range(r).into_iter().map(at).collect())
        .unwrap_or_default();

    let kind_of = |c: &str| match c {
        "line" | "stock" => "c:lineChart",
        "area" => "c:areaChart",
        "circle" => "c:pieChart",
        "ring" => "c:doughnutChart",
        "scatter" | "bubble" => "c:scatterChart",
        "radar" | "filled-radar" => "c:radarChart",
        _ => "c:barChart",
    };
    let main_kind = kind_of(class);
    let mut series_xml = String::new();
    for (n, a, _) in children(plot_attrs_inner) {
        if n != "chart:series" {
            continue;
        }
        let values: Vec<String> = extract_attr(a, "chart:values-cell-range-address").map(|r| range(r).into_iter().map(at).collect()).unwrap_or_default();
        if values.is_empty() {
            continue;
        }
        let name = extract_attr(a, "chart:label-cell-address").and_then(cell).map(at).unwrap_or_default();
        let st = extract_attr(a, "chart:style-name");
        let color = style_prop(doc, st, "draw:fill-color").filter(|_| main_kind != "c:lineChart").or_else(|| style_prop(doc, st, "svg:stroke-color"));
        let sp = color.map(|c| format!(r#"<c:spPr><a:solidFill><a:srgbClr val="{}"/></a:solidFill></c:spPr>"#, c.trim_start_matches('#'))).unwrap_or_default();
        let show = style_prop(doc, st, "chart:data-label-number").is_some_and(|v| v != "none");
        let dl = if show { r#"<c:dLbls><c:showVal val="1"/></c:dLbls>"# } else { "" };
        series_xml.push_str(&format!(
            r#"<c:ser><c:tx><c:strRef>{name}</c:strRef></c:tx>{sp}{dl}<c:cat><c:strRef>{cat}</c:strRef></c:cat><c:val><c:numRef>{val}</c:numRef></c:val></c:ser>"#,
            name = cache(std::slice::from_ref(&name), false),
            cat = cache(&cats, false),
            val = cache(&values, true),
        ));
    }
    if series_xml.is_empty() {
        return None;
    }
    let grouping = if flag("chart:percentage") { "percentStacked" } else if flag("chart:stacked") { "stacked" } else if main_kind == "c:barChart" { "clustered" } else { "standard" };
    let body = match main_kind {
        "c:barChart" => format!(r#"<c:barDir val="{}"/><c:grouping val="{grouping}"/><c:varyColors val="0"/>{series_xml}<c:gapWidth val="100"/>"#, if flag("chart:vertical") { "bar" } else { "col" }),
        "c:pieChart" | "c:doughnutChart" => format!(r#"<c:varyColors val="1"/>{series_xml}<c:holeSize val="50"/>"#),
        _ => format!(r#"<c:grouping val="{grouping}"/><c:varyColors val="0"/>{series_xml}"#),
    };
    let axes = if matches!(main_kind, "c:pieChart" | "c:doughnutChart") { "" } else { "<c:catAx/><c:valAx><c:majorGridlines/></c:valAx>" };
    let title = next_tag(chart, "chart:title").map(|(_, t, _)| decode_xml_entities(&strip_tags(t))).filter(|t| !t.trim().is_empty());
    let title_xml = title.map(|t| format!(r#"<c:title><c:tx><c:rich><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx></c:title>"#, encode_xml_text(&t))).unwrap_or_else(|| r#"<c:autoTitleDeleted val="1"/>"#.into());
    let legend = next_tag(chart, "chart:legend").map(|(a, _, _)| {
        let pos = match extract_attr(a, "chart:legend-position") { Some("top") => "t", Some("bottom") => "b", Some("start") => "l", _ => "r" };
        format!(r#"<c:legend><c:legendPos val="{pos}"/></c:legend>"#)
    }).unwrap_or_default();
    Some(format!(r#"<c:chartSpace><c:style val="2"/><c:chart>{title_xml}<c:plotArea><{main_kind}>{body}</{main_kind}>{axes}</c:plotArea>{legend}</c:chart></c:chartSpace>"#))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bar_chart_restates_its_table_and_series() {
        let doc = r##"<office:document-content><office:automatic-styles>
            <style:style style:name="pa" style:family="chart"><style:chart-properties chart:stacked="true"/></style:style>
            <style:style style:name="s1" style:family="chart"><style:graphic-properties draw:fill-color="#ff0000"/></style:style>
          </office:automatic-styles><office:body><office:chart><chart:chart chart:class="chart:bar">
            <chart:title><text:p>Sales</text:p></chart:title><chart:legend chart:legend-position="bottom"/>
            <chart:plot-area chart:style-name="pa"><chart:axis chart:dimension="x"><chart:categories table:cell-range-address="local-table.$A$2:.$A$3"/></chart:axis>
              <chart:series chart:style-name="s1" chart:values-cell-range-address="local-table.$B$2:.$B$3" chart:label-cell-address="local-table.$B$1"/></chart:plot-area>
            <table:table table:name="local-table"><table:table-header-rows><table:table-row><table:table-cell><text:p/></table:table-cell><table:table-cell office:value-type="string"><text:p>2025</text:p></table:table-cell></table:table-row></table:table-header-rows>
              <table:table-rows><table:table-row><table:table-cell office:value-type="string"><text:p>Q1</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="3.5"><text:p>3.5</text:p></table:table-cell></table:table-row>
              <table:table-row><table:table-cell office:value-type="string"><text:p>Q2</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="4"><text:p>4</text:p></table:table-cell></table:table-row></table:table-rows></table:table>
          </chart:chart></office:chart></office:body></office:document-content>"##;
        let x = to_drawingml(doc).unwrap();
        assert!(x.contains(r#"<c:barChart><c:barDir val="col"/><c:grouping val="stacked"/>"#), "{x}");
        assert!(x.contains(r#"<c:v>2025</c:v>"#) && x.contains(r#"<c:v>Q2</c:v>"#) && x.contains(r#"<c:v>3.5</c:v>"#), "{x}");
        assert!(x.contains(r#"<a:srgbClr val="ff0000"/>"#) && x.contains("<a:t>Sales</a:t>") && x.contains(r#"<c:legendPos val="b"/>"#), "{x}");
    }
}
