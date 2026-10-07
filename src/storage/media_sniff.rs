//! Content-based gatekeeping for files the importers copy into the
//! web-served media directory (`/media/...`).
//!
//! Importers used to preserve whatever extension the *source* named
//! (`foo.html`, `bar.svg`, an extensionless path to `/etc/passwd` defaulting
//! to `.jpg`), so a crafted PPTX/OpenLP/FreeShow/online-media payload could
//! plant active content on the app's own origin, or use a referenced local
//! path to copy an arbitrary file somewhere any LAN client could then fetch
//! it. Here the extension is derived from the file's magic bytes instead, and
//! anything that isn't a raster image or video container is refused. SVG is
//! deliberately not accepted: it can carry script.

use std::path::Path;

/// Largest single file the local-path copy helpers will read into memory.
const MAX_LOCAL_MEDIA_BYTES: u64 = 512 * 1024 * 1024;

/// Returns the canonical extension for `bytes` if they start with a known
/// raster-image or video signature, else `None`.
pub fn sniff_media_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"AVI " {
        Some("avi")
    } else if bytes.starts_with(b"BM") && bytes.len() > 14 {
        Some("bmp")
    } else if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        Some("webm") // EBML: webm/mkv
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        match &bytes[8..12] {
            b"avif" | b"avis" => Some("avif"),
            b"heic" | b"heix" | b"mif1" => Some("heic"),
            b"qt  " => Some("mov"),
            _ => Some("mp4"),
        }
    } else {
        None
    }
}

/// Writes `bytes` into `dir` as `<prefix>_<uuid>.<sniffed ext>` and returns
/// the file name, or `None` if the bytes aren't recognizable media or the
/// write fails.
pub fn write_sniffed_media(dir: &Path, prefix: &str, bytes: &[u8]) -> Option<String> {
    let ext = sniff_media_ext(bytes)?;
    std::fs::create_dir_all(dir).ok()?;
    let name = format!("{}_{}.{}", prefix, uuid::Uuid::new_v4(), ext);
    std::fs::write(dir.join(&name), bytes).ok()?;
    Some(name)
}

/// `write_sniffed_media` for a file already on disk (the source of a path
/// referenced by an imported document).
pub fn copy_sniffed_media(src: &Path, dir: &Path, prefix: &str) -> Option<String> {
    let meta = std::fs::metadata(src).ok()?;
    if !meta.is_file() || meta.len() > MAX_LOCAL_MEDIA_BYTES {
        return None;
    }
    let bytes = std::fs::read(src).ok()?;
    write_sniffed_media(dir, prefix, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_known_media_and_rejects_active_content() {
        assert_eq!(sniff_media_ext(b"\x89PNG\r\n\x1a\nxxxx"), Some("png"));
        assert_eq!(sniff_media_ext(&[0xFF, 0xD8, 0xFF, 0xE0, 0]), Some("jpg"));
        assert_eq!(sniff_media_ext(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff_media_ext(b"\0\0\0\x18ftypmp42\0\0\0\0"), Some("mp4"));
        assert_eq!(sniff_media_ext(b"<html><script>alert(1)</script>"), None);
        assert_eq!(sniff_media_ext(b"<svg xmlns='http://www.w3.org/2000/svg' onload='x()'/>"), None);
        assert_eq!(sniff_media_ext(b"root:x:0:0:root:/root:/bin/bash\n"), None);
        assert_eq!(sniff_media_ext(b""), None);
    }
}

/// Max bytes read from a single archive entry, by kind. Archives deflate
/// ~1000:1, so the request body limit does not bound what an entry expands to.
pub const MAX_ZIP_TEXT_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_ZIP_DB_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_ZIP_MEDIA_BYTES: u64 = 64 * 1024 * 1024;

/// Reads at most `limit` bytes; errors (rather than truncating) if more.
pub fn read_capped_bytes<R: std::io::Read>(r: R, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(r, limit + 1), &mut buf)?;
    if buf.len() as u64 > limit {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "archive entry exceeds size limit"));
    }
    Ok(buf)
}

pub fn read_capped_string<R: std::io::Read>(r: R, limit: u64) -> std::io::Result<String> {
    String::from_utf8(read_capped_bytes(r, limit)?)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod cap_tests {
    use super::*;
    #[test]
    fn capped_reads_reject_oversize() {
        assert_eq!(read_capped_bytes(&[0u8; 10][..], 10).unwrap().len(), 10);
        assert!(read_capped_bytes(&[0u8; 11][..], 10).is_err());
        assert!(read_capped_string(&b"abc"[..], 2).is_err());
    }
}
