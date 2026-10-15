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

/// Returns the canonical extension for `bytes` if they start with a known audio container/stream
/// signature, else `None`. Separate from `sniff_media_ext` because an importer or a plugin's
/// `library.addMedia` call picks one or the other depending on what it claims to be adding, never both.
pub fn sniff_audio_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"ID3") {
        Some("mp3")
    } else if mpeg_audio_frame_len(bytes).is_some() {
        // A bare .mp3/.aac stream with no ID3 tag: no container magic exists for this, so the check
        // is the MPEG audio frame header itself -- see mpeg_audio_frame_len below.
        Some("mp3")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        Some("wav")
    } else if bytes.starts_with(b"fLaC") {
        Some("flac")
    } else if bytes.starts_with(b"OggS") {
        if bytes.windows(9).any(|w| w == b"OpusHead\0" || w == b"OpusHead ") || bytes.windows(8).any(|w| w == b"OpusHead") {
            Some("opus")
        } else {
            Some("ogg")
        }
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && matches!(&bytes[8..12], b"M4A " | b"M4B ") {
        Some("m4a")
    } else {
        None
    }
}

/// MPEG-1/2/2.5 Audio bitrate tables in kbps, by layer, indexed 1..=14 (index 0 is "free" bitrate --
/// legal in the format but its frame length can't be computed without decoding, so this module
/// can't confirm a second frame follows it; treated as not recognized). Reserved rows for layers
/// that don't apply to a given version are unused. ISO/IEC 11172-3, table B.1/B.2.
const BITRATE_V1_L1: [u16; 15] = [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448];
const BITRATE_V1_L2: [u16; 15] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384];
const BITRATE_V1_L3: [u16; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
const BITRATE_V2_L1: [u16; 15] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256];
const BITRATE_V2_L23: [u16; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

/// Sample rates in Hz, indexed 0..=2 by the header's 2-bit sampling-rate field, per MPEG version.
const SAMPLE_RATE_V1: [u32; 3] = [44100, 48000, 32000];
const SAMPLE_RATE_V2: [u32; 3] = [22050, 24000, 16000];
const SAMPLE_RATE_V25: [u32; 3] = [11025, 12000, 8000];

/// Decodes one 4-byte MPEG audio frame header at the start of `bytes` and returns its frame length
/// in bytes, or `None` if the header is structurally invalid (bad sync, a reserved field, or a
/// "free" bitrate whose length isn't derivable from the header alone). Does not decode any audio.
fn decode_mpeg_header(bytes: &[u8]) -> Option<(usize, u8, u8, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || (bytes[1] & 0xE0) != 0xE0 {
        return None;
    }
    let version_bits = (bytes[1] >> 3) & 0x3;
    let layer_bits = (bytes[1] >> 1) & 0x3;
    if version_bits == 0b01 || layer_bits == 0b00 {
        return None; // reserved
    }
    let bitrate_idx = (bytes[2] >> 4) & 0xF;
    let samplerate_idx = (bytes[2] >> 2) & 0x3;
    if bitrate_idx == 0 || bitrate_idx == 0xF || samplerate_idx == 0b11 {
        return None; // free (unknowable length), bad, or reserved
    }
    let padding = u32::from((bytes[2] >> 1) & 0x1);
    let sample_rate = match version_bits {
        0b11 => SAMPLE_RATE_V1[samplerate_idx as usize],
        0b10 => SAMPLE_RATE_V2[samplerate_idx as usize],
        _ => SAMPLE_RATE_V25[samplerate_idx as usize], // 0b00
    };
    let bitrate_kbps = u32::from(match (version_bits, layer_bits) {
        (0b11, 0b11) => BITRATE_V1_L1[bitrate_idx as usize],
        (0b11, 0b10) => BITRATE_V1_L2[bitrate_idx as usize],
        (0b11, 0b01) => BITRATE_V1_L3[bitrate_idx as usize],
        (_, 0b11) => BITRATE_V2_L1[bitrate_idx as usize],
        _ => BITRATE_V2_L23[bitrate_idx as usize],
    });
    let len = if layer_bits == 0b11 {
        (12 * bitrate_kbps * 1000 / sample_rate + padding) * 4 // Layer I
    } else {
        144 * bitrate_kbps * 1000 / sample_rate + padding // Layer II/III
    };
    // Smallest real frame is well over this; guards the division results above from being used as
    // a 0 or absurd length if a future table edit introduces a typo.
    if !(16..=8192).contains(&len) {
        return None;
    }
    Some((len as usize, version_bits, layer_bits, sample_rate))
}

/// Whether `bytes` begins with a real MPEG audio bitstream: a structurally valid frame header whose
/// declared length lands on a second frame header agreeing on version, layer and sample rate (a
/// plain two-byte sync-pattern match, which is all the header-only check above gives by itself,
/// costs an attacker nothing -- this is what actually distinguishes a genuine stream from a crafted
/// prefix in front of arbitrary bytes). Bitrate may differ frame to frame (that's normal for VBR).
pub fn mpeg_audio_frame_len(bytes: &[u8]) -> Option<usize> {
    let (len, version, layer, rate) = decode_mpeg_header(bytes)?;
    let next = bytes.get(len..)?;
    let (_, version2, layer2, rate2) = decode_mpeg_header(next)?;
    (version == version2 && layer == layer2 && rate == rate2).then_some(len)
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

    /// Builds one structurally valid MPEG-1 Layer III frame: a real header plus exactly enough
    /// filler payload bytes to reach its declared length, so the next frame lands correctly.
    fn build_frame(bitrate_idx: u8, samplerate_idx: u8, fill: u8) -> Vec<u8> {
        const BITRATE_V1_L3: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
        const SR_V1: [u32; 3] = [44100, 48000, 32000];
        let b1 = 0xE0 | (0b11 << 3) | (0b01 << 1) | 1; // MPEG1, Layer III, no CRC
        let b2 = (bitrate_idx << 4) | (samplerate_idx << 2);
        let mut frame = vec![0xFF, b1, b2, 0x00];
        let len = 144 * BITRATE_V1_L3[bitrate_idx as usize] * 1000 / SR_V1[samplerate_idx as usize];
        frame.resize(len as usize, fill);
        frame
    }

    #[test]
    fn mpeg_frame_chaining_tells_a_real_stream_from_a_crafted_prefix() {
        // Two real, valid frames (a bitrate change, as in a VBR stream) chain and are accepted.
        let first = build_frame(5, 0, b'a'); // 64kbps/44100
        let first_len = first.len();
        let mut vbr = first;
        vbr.extend(build_frame(9, 0, b'b')); // 128kbps/44100, same version/layer/samplerate
        assert_eq!(mpeg_audio_frame_len(&vbr), Some(first_len));
        assert_eq!(sniff_audio_ext(&vbr), Some("mp3"));

        // The genuine encoder-produced fixture (testdata/media/bare_real.mp3, no ID3 tag) passes too.
        let real = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/media/bare_real.mp3")).unwrap();
        assert!(mpeg_audio_frame_len(&real).is_some(), "a real encoder's bare MP3 stream must be recognized");
        assert_eq!(sniff_audio_ext(&real), Some("mp3"));
        let synthetic = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/media/synthetic_vbr.bin")).unwrap();
        assert!(mpeg_audio_frame_len(&synthetic).is_some());

        // A single valid-looking header with no real second frame following -- exactly what a two-byte
        // sync-pattern attack can produce for free -- is refused: nothing says what follows is a frame.
        let mut lone = build_frame(5, 0, b'a');
        lone.truncate(lone.len() - 4); // cut short: the next "header" is garbage/EOF, not a real frame
        lone.extend_from_slice(b"\xFF\xFB\x90\x00<html><script>alert(document.domain)</script>");
        assert_eq!(mpeg_audio_frame_len(&lone), None);
        assert_eq!(sniff_audio_ext(&lone), None);

        // The classic attack this replaces: 2 bytes of sync pattern, then arbitrary script.
        let mut prefixed = vec![0xFF, 0xFB];
        prefixed.extend_from_slice(b"<html><script>alert(document.domain)</script></html>");
        assert_eq!(sniff_audio_ext(&prefixed), None, "a bare sync pattern in front of HTML must not pass as mp3");

        // Reserved/invalid header fields are refused outright (no frame-2 lookup needed).
        assert_eq!(decode_mpeg_header(&[0xFF, 0xEB, 0x00, 0x00]), None, "version 01 is reserved");
        assert_eq!(decode_mpeg_header(&[0xFF, 0xF9, 0x00, 0x00]), None, "layer 00 is reserved");
        assert_eq!(decode_mpeg_header(&[0xFF, 0xFB, 0xF0, 0x00]), None, "bitrate index 15 (0xF) is bad");
        assert_eq!(decode_mpeg_header(&[0xFF, 0xFB, 0x00, 0x00]), None, "bitrate index 0 (free) can't be length-checked");
        assert_eq!(decode_mpeg_header(&[0xFF, 0xFB, 0x0C, 0x00]), None, "sample-rate index 3 is reserved");
        assert_eq!(decode_mpeg_header(b"<htm"), None);
        assert_eq!(decode_mpeg_header(&[0xFF]), None, "too short");
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
