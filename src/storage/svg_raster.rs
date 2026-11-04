//! SVG pictures in imported slides, drawn to PNG: the app keeps no SVG (it can carry script), so
//! an SVG becomes a picture of itself as soon as an importer reads it -- before cropping,
//! recolouring or tiling, which all need pixels. resvg never runs script, and pictures the SVG
//! points at outside itself (files, URLs) are left out: only images embedded in it are drawn.

use resvg::{tiny_skia, usvg};
use std::io::Read;
use std::sync::{Arc, OnceLock};

/// The long side of the drawn picture, in pixels: sharp on a 1080p projector even full screen.
const LONG_SIDE: f32 = 2048.0;

/// The most a gzipped SVG (SVGZ) may inflate to.
const MAX_SVG_BYTES: u64 = 32 * 1024 * 1024;

/// How far into a document its `<svg` root may start (past a long prolog: DOCTYPE entities,
/// licence comments).
const ROOT_WITHIN: usize = 64 * 1024;

/// A picture as read from a package: an SVG (or SVGZ) drawn to PNG, anything else as it is.
pub fn as_raster(bytes: Vec<u8>) -> Vec<u8> {
    svg_document(&bytes).and_then(|svg| to_png(&svg)).unwrap_or(bytes)
}

/// The SVG document in `bytes` (inflated, for SVGZ), or `None` when it isn't one. Inflating stops
/// at a bound, so a small gzip bomb can't exhaust memory.
fn svg_document(bytes: &[u8]) -> Option<std::borrow::Cow<'_, [u8]>> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut inflated = Vec::new();
        flate2::read::GzDecoder::new(bytes).take(MAX_SVG_BYTES + 1).read_to_end(&mut inflated).ok()?;
        if inflated.len() as u64 > MAX_SVG_BYTES || !is_svg(&inflated) {
            return None;
        }
        return Some(inflated.into());
    }
    is_svg(bytes).then_some(bytes.into())
}

/// Whether `bytes` look like an SVG document: XML (or a bare `<svg`) with an `<svg` root early on.
fn is_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(ROOT_WITHIN)]);
    let t = head.trim_start_matches('\u{feff}').trim_start();
    (t.starts_with("<?xml") || t.starts_with("<!--") || t.starts_with("<!DOCTYPE") || t.starts_with("<svg")) && head.contains("<svg")
}

/// The system's fonts, loaded once, for SVGs with text in them.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

/// An SVG document drawn as a PNG, its long side 2048 pixels; `None` when it can't be parsed or
/// has no size. System fonts are only loaded for an SVG with text.
fn to_png(svg: &[u8]) -> Option<Vec<u8>> {
    let has_text = svg.windows(5).any(|w| w == b"<text") || svg.windows(9).any(|w| w == b":text");
    let mut opt = usvg::Options { resources_dir: None, ..Default::default() };
    if has_text {
        opt.fontdb = fonts();
    }
    // Embedded (data URL) pictures only: nothing outside the document is read.
    opt.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree = usvg::Tree::from_data(svg, &opt).ok()?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if !(w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()) {
        return None;
    }
    let scale = LONG_SIDE / w.max(h);
    let (pw, ph) = ((w * scale).round().max(1.0) as u32, (h * scale).round().max(1.0) as u32);
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(png: &[u8]) -> image::RgbaImage {
        image::load_from_memory(png).unwrap().to_rgba8()
    }

    #[test]
    fn draws_an_svg_at_2048_on_its_long_side() {
        let svg = br##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="#ff0000"/></svg>"##;
        let img = decoded(&as_raster(svg.to_vec()));
        assert_eq!(img.dimensions(), (2048, 1024));
        assert_eq!(img.get_pixel(1000, 500).0, [255, 0, 0, 255]);
        // A long prolog before the root is still an SVG.
        let long = format!("<?xml version=\"1.0\"?><!-- {} -->{}", "x".repeat(5000), std::str::from_utf8(&svg[21..]).unwrap());
        assert_eq!(decoded(&as_raster(long.into_bytes())).dimensions(), (2048, 1024));
    }

    #[test]
    fn draws_embedded_pictures_but_never_reads_files_outside() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 255, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let png = png.into_inner();
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("secret.png");
        std::fs::write(&outside, &png).unwrap();
        let image_svg = |href: &str| format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image xlink:href="{href}" width="10" height="10"/></svg>"#).into_bytes();
        use base64::Engine;
        let data_url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png));
        assert_eq!(decoded(&as_raster(image_svg(&data_url))).get_pixel(1024, 1024).0, [0, 0, 255, 255], "an embedded picture is drawn");
        for href in [outside.to_str().unwrap().to_string(), format!("file://{}", outside.display())] {
            assert!(decoded(&as_raster(image_svg(&href))).pixels().all(|p| p.0[3] == 0), "a real picture on disk ({href}) is not read");
        }
    }

    #[test]
    fn leaves_other_bytes_alone_and_bounds_svgz() {
        let not_svg = b"\x89PNG\r\n\x1a\nrest".to_vec();
        assert_eq!(as_raster(not_svg.clone()), not_svg);
        let broken = b"<svg xmlns='http://www.w3.org/2000/svg'".to_vec();
        assert_eq!(as_raster(broken.clone()), broken);
        // A gzip stream that inflates past the bound (or isn't SVG) is left as it is.
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &vec![b' '; (MAX_SVG_BYTES + 10) as usize]).unwrap();
        let bomb = gz.finish().unwrap();
        assert_eq!(as_raster(bomb.clone()), bomb);
        // A real SVGZ is drawn.
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4"/></svg>"#).unwrap();
        assert_eq!(decoded(&as_raster(gz.finish().unwrap())).dimensions(), (2048, 2048));
    }
}
