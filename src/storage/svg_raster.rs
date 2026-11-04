//! SVG pictures in imported slides, drawn to PNG: the app keeps no SVG (it can carry script), so
//! an SVG becomes a picture of itself. resvg never runs script, and pictures the SVG points at
//! outside itself (files, URLs) are left out -- only images embedded in it are drawn.

use resvg::{tiny_skia, usvg};
use std::sync::{Arc, OnceLock};

/// The long side of the drawn picture, in pixels: sharp on a 1080p projector even full screen.
const LONG_SIDE: f32 = 2048.0;

/// Whether `bytes` look like an SVG (or gzipped SVGZ) document.
pub fn is_svg(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return true;
    }
    let head = &bytes[..bytes.len().min(2048)];
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start_matches('\u{feff}').trim_start();
    (t.starts_with("<?xml") || t.starts_with("<!--") || t.starts_with("<!DOCTYPE") || t.starts_with("<svg")) && text.contains("<svg")
}

/// The system's fonts, loaded once, for text inside SVGs.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

/// An SVG (or SVGZ) drawn as a PNG, its long side 2048 pixels; `None` when it can't be parsed or
/// has no size.
pub fn to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut opt = usvg::Options { resources_dir: None, fontdb: fonts(), ..Default::default() };
    // Embedded (data URL) pictures only: nothing outside the document is read.
    opt.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree = usvg::Tree::from_data(bytes, &opt).ok()?;
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

    #[test]
    fn draws_an_svg_at_2048_on_its_long_side() {
        let svg = br##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="#ff0000"/></svg>"##;
        assert!(is_svg(svg));
        let png = to_png(svg).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (2048, 1024));
        assert_eq!(img.get_pixel(1000, 500).0, [255, 0, 0, 255]);
    }

    #[test]
    fn leaves_out_pictures_outside_the_document_and_rejects_non_svg() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image xlink:href="/etc/passwd" width="10" height="10"/></svg>"##;
        let img = image::load_from_memory(&to_png(svg).unwrap()).unwrap().to_rgba8();
        assert!(img.pixels().all(|p| p.0[3] == 0), "nothing drawn");
        assert!(!is_svg(b"\x89PNG\r\n\x1a\n"));
        assert!(to_png(b"<svg xmlns='http://www.w3.org/2000/svg'").is_none());
    }
}
