//! OpenSanctuary's own file formats, which keep everything the app holds for an item -- unlike `.ewsx`
//! and `.odp`, which squeeze it into someone else's schema:
//!
//! | extension     | holds                                                         |
//! |---------------|---------------------------------------------------------------|
//! | `.ossong`     | one or more songs                                             |
//! | `.ospres`     | one or more presentations                                     |
//! | `.osservice`  | a whole schedule (the order of service)                       |
//! | `.osbible`    | one Bible translation                                         |
//!
//! (Not `.osp`, `.osb` or `.osz`: OpenShot, osu! and both osu! and OpenLP already own those.)
//!
//! All four are the same zip container, so any unzip tool can open one:
//!
//! ```text
//! manifest.json          format name + version, kind, title, counts, and the media table
//! content.json           the app's own serde state (Songs / Presentations / a Schedule / chapters)
//! themes.json            definitions of the themes that content names (only if it names any)
//! media/<sha256>.<ext>   pictures, videos and music, stored once each however often they're used
//! ```
//!
//! Inside `content.json` a media reference (`/media/images/bg.jpg`, alone or inside CSS like
//! `url('...')`) is replaced by `bundle-media:<sha256>`, and put back as a path into the importing
//! library's own media folder. Themes are referenced by name; a bundled definition is only used when
//! the importing library has no theme of that name.
//!
//! A bundle is untrusted input. The reader only accepts entries from the fixed list above, caps
//! every read, checks each media file's hash and its real content type (the same sniffing the other
//! importers use) before it is written anywhere, and never uses an archive name as a path.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::core::models::{MediaItem, Presentation, Schedule, ScriptureItem, ScriptureVerse, Song, Theme};
use crate::storage::media_sniff::{sniff_audio_ext, sniff_media_ext};
use crate::storage::Database;

pub const FORMAT_NAME: &str = "opensanctuary-bundle";
pub const FORMAT_VERSION: u32 = 1;
const TOKEN_PREFIX: &str = "bundle-media:";

const MAX_ENTRIES: usize = 20_000;
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CONTENT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_THEMES_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MEDIA_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_MEDIA_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_SONGS: usize = 20_000;
const MAX_PRESENTATIONS: usize = 5_000;
const MAX_SERVICE_ITEMS: usize = 5_000;
const MAX_BIBLE_CHAPTERS: usize = 4_000;
const MAX_VERSES_PER_CHAPTER: usize = 400;
const MAX_VERSE_TEXT: usize = 16_000;
const MAX_NAME: usize = 100;
const IO_CHUNK: usize = 64 * 1024;
const BIBLE_INFO_PREFIX: &str = "bibleInfo_";

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("{0}")]
    Invalid(String),
    #[error("this file was made by a newer OpenSanctuary (format {found}); this version reads format {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("that is a {found} file, not a {expected} file")]
    WrongKind { found: &'static str, expected: &'static str },
    #[error("{0} is already installed; choose replace to overwrite it")]
    AlreadyInstalled(String),
    #[error("{0}")]
    NotFound(String),
    #[error("storage error: {0}")]
    Db(String),
    #[error("could not read or write the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a valid zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("malformed JSON in the bundle: {0}")]
    Json(#[from] serde_json::Error),
}

fn invalid(msg: impl Into<String>) -> BundleError {
    BundleError::Invalid(msg.into())
}

fn db_err(e: impl std::fmt::Display) -> BundleError {
    BundleError::Db(e.to_string())
}

// ------------------------------------------------------------------------------------------------
// Kinds
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BundleKind {
    Song,
    Bible,
    Presentation,
    Service,
}

impl BundleKind {
    pub fn extension(self) -> &'static str {
        match self {
            BundleKind::Song => "ossong",
            BundleKind::Bible => "osbible",
            BundleKind::Presentation => "ospres",
            BundleKind::Service => "osservice",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "ossong" => Some(BundleKind::Song),
            "osbible" => Some(BundleKind::Bible),
            "ospres" => Some(BundleKind::Presentation),
            "osservice" => Some(BundleKind::Service),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BundleKind::Song => "song",
            BundleKind::Bible => "Bible",
            BundleKind::Presentation => "presentation",
            BundleKind::Service => "service",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

impl MediaKind {
    /// The folder under the media directory (and the segment of its `/media/...` URL).
    fn dir(self) -> &'static str {
        match self {
            MediaKind::Image => "images",
            MediaKind::Video => "videos",
            MediaKind::Audio => "audio",
        }
    }

    /// The canonical extension for `head` if its bytes really are this kind of media.
    fn sniff(self, head: &[u8]) -> Option<&'static str> {
        const IMAGE_EXTS: [&str; 4] = ["png", "jpg", "gif", "webp"];
        match self {
            MediaKind::Audio => sniff_audio_ext(head),
            MediaKind::Image => sniff_media_ext(head).filter(|e| IMAGE_EXTS.contains(e)),
            MediaKind::Video => sniff_media_ext(head).filter(|e| !IMAGE_EXTS.contains(e)),
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Manifest and content
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaEntry {
    /// SHA-256 of the file, lowercase hex; also its name inside the archive (`media/<id>.<ext>`).
    pub id: String,
    pub kind: MediaKind,
    pub ext: String,
    /// The file's name in the library it came from (shown to people, never used as a path).
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Counts {
    #[serde(default)]
    pub songs: usize,
    #[serde(default)]
    pub presentations: usize,
    #[serde(default)]
    pub items: usize,
    #[serde(default)]
    pub chapters: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub kind: BundleKind,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub media: Vec<MediaEntry>,
    #[serde(default)]
    pub themes: Vec<String>,
    #[serde(default)]
    pub counts: Counts,
}

/// What a Bible bundle says about its translation. The app itself only knows a Bible by its
/// abbreviation, so the rest travels with the file (and survives a re-export).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BibleInfo {
    pub abbreviation: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub attribution: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BibleChapter {
    pub book: String,
    pub chapter: u32,
    pub verses: Vec<ScriptureVerse>,
}

// ------------------------------------------------------------------------------------------------
// Finding and moving media references
// ------------------------------------------------------------------------------------------------

const REF_EXTS: &[&str] = &[
    "jpeg", "jpg", "png", "gif", "webp", "bmp", "avif", "heic", "mp4", "m4v", "mov", "webm", "mkv", "avi", "ogv", "mp3", "wav", "flac", "opus", "ogg",
    "m4a", "aac",
];

#[derive(Debug, PartialEq)]
struct MediaRef {
    start: usize,
    end: usize,
    kind: MediaKind,
    file: String,
}

/// Every `/media/<images|videos|audio>/<file>` inside `s`, wherever it sits: a bare path, or inside
/// CSS such as `url('/media/images/a.jpg')` or a gradient with a `url(...)` layer. The file name ends
/// at the first known media extension followed by something that can't continue a name, so names with
/// spaces and parentheses (`Photo (1).jpg`, which the plugin host produces) are kept whole.
fn find_media_refs(s: &str) -> Vec<MediaRef> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = s[from..].find("/media/") {
        let after = from + rel + "/media/".len();
        let rest = &s[after..];
        let (kind, dir_len) = if rest.starts_with("images/") {
            (MediaKind::Image, 7)
        } else if rest.starts_with("videos/") {
            (MediaKind::Video, 7)
        } else if rest.starts_with("audio/") {
            (MediaKind::Audio, 6)
        } else {
            from = after;
            continue;
        };
        let name_start = after + dir_len;
        let region_end = s[name_start..].find(['\'', '"', '\n', '\r']).map(|i| name_start + i).unwrap_or(s.len());
        let region = &s[name_start..region_end];
        let lower = region.to_ascii_lowercase();
        let mut found = None;
        let mut search = 0;
        'dots: while let Some(dot) = lower[search..].find('.') {
            let d = search + dot;
            let tail = &lower[d + 1..];
            for ext in REF_EXTS {
                if tail.starts_with(ext) {
                    let end = d + 1 + ext.len();
                    let boundary = end == lower.len() || matches!(lower.as_bytes()[end], b')' | b';' | b',' | b' ' | b'\t' | b'?' | b'#');
                    if boundary {
                        found = Some(end);
                        break 'dots;
                    }
                }
            }
            search = d + 1;
        }
        match found {
            Some(end) => {
                out.push(MediaRef { start: from + rel, end: name_start + end, kind, file: region[..end].to_string() });
                from = name_start + end;
            }
            None => from = name_start,
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16)) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A bare file name: nothing that could climb out of its folder or name a hidden file.
fn safe_file_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 255 && !n.contains(['/', '\\', '\0']) && !n.starts_with('.')
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn walk_strings(v: &mut Value, f: &mut dyn FnMut(&mut String)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter_mut().for_each(|x| walk_strings(x, f)),
        Value::Object(m) => m.iter_mut().for_each(|(_, x)| walk_strings(x, f)),
        _ => {}
    }
}

fn collect_theme_names(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Array(a) => a.iter().for_each(|x| collect_theme_names(x, out)),
        Value::Object(m) => {
            for (k, x) in m {
                match x {
                    Value::String(s) if k == "theme_name" && !s.trim().is_empty() => {
                        out.insert(s.trim().to_string());
                    }
                    _ => collect_theme_names(x, out),
                }
            }
        }
        _ => {}
    }
}

/// Turns media paths into `bundle-media:<sha256>` tokens, remembering each distinct file once.
/// Used to write a bundle, and (without writing anything) to compare an item already in the library
/// with the copy a bundle carries.
struct Collector<'a> {
    media_dir: &'a Path,
    resolved: HashMap<(MediaKind, String), Option<String>>,
    entries: BTreeMap<String, (MediaEntry, PathBuf)>,
    missing: BTreeSet<String>,
}

struct Hashed {
    id: String,
    size: u64,
    ext: &'static str,
}

fn hash_media_file(kind: MediaKind, path: &Path) -> Option<Hashed> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    if len == 0 || len > MAX_MEDIA_FILE_BYTES {
        return None;
    }
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; IO_CHUNK];
    let mut head: Vec<u8> = Vec::new();
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        if head.len() < IO_CHUNK {
            head.extend_from_slice(&buf[..n.min(IO_CHUNK - head.len())]);
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    let ext = kind.sniff(&head)?;
    Some(Hashed { id: hex(&hasher.finalize()), size: total, ext })
}

impl<'a> Collector<'a> {
    fn new(media_dir: &'a Path) -> Self {
        Collector { media_dir, resolved: HashMap::new(), entries: BTreeMap::new(), missing: BTreeSet::new() }
    }

    fn resolve(&mut self, kind: MediaKind, file: &str) -> Option<String> {
        let key = (kind, file.to_string());
        if let Some(hit) = self.resolved.get(&key) {
            return hit.clone();
        }
        let decoded = percent_decode(file);
        let mut candidates = vec![file.to_string()];
        if decoded != file {
            candidates.push(decoded);
        }
        let mut result = None;
        for name in candidates {
            if !safe_file_name(&name) {
                continue;
            }
            let path = self.media_dir.join(kind.dir()).join(&name);
            if !path.is_file() {
                continue;
            }
            if let Some(h) = hash_media_file(kind, &path) {
                self.entries.entry(h.id.clone()).or_insert_with(|| {
                    (MediaEntry { id: h.id.clone(), kind, ext: h.ext.to_string(), name: name.clone(), size: h.size }, path.clone())
                });
                result = Some(h.id);
                break;
            }
        }
        if result.is_none() {
            self.missing.insert(format!("/media/{}/{}", kind.dir(), file));
        }
        self.resolved.insert(key, result.clone());
        result
    }

    fn tokenize_string(&mut self, s: &str) -> Option<String> {
        let refs = find_media_refs(s);
        if refs.is_empty() {
            return None;
        }
        let mut out = s.to_string();
        let mut changed = false;
        for r in refs.into_iter().rev() {
            if let Some(id) = self.resolve(r.kind, &r.file) {
                out.replace_range(r.start..r.end, &format!("{TOKEN_PREFIX}{id}"));
                changed = true;
            }
        }
        changed.then_some(out)
    }

    fn tokenize(&mut self, v: &mut Value) {
        walk_strings(v, &mut |s| {
            if let Some(n) = self.tokenize_string(s) {
                *s = n;
            }
        });
    }
}

/// Puts the importing library's own paths where the tokens are.
fn detokenize(v: &mut Value, paths: &HashMap<String, String>) {
    walk_strings(v, &mut |s| {
        if !s.contains(TOKEN_PREFIX) {
            return;
        }
        let mut out = String::with_capacity(s.len());
        let mut rest = s.as_str();
        while let Some(at) = rest.find(TOKEN_PREFIX) {
            out.push_str(&rest[..at]);
            let after = &rest[at + TOKEN_PREFIX.len()..];
            let id = after.get(..64).filter(|id| is_sha256_hex(id));
            match id.and_then(|id| paths.get(id)) {
                Some(path) => {
                    out.push_str(path);
                    rest = &after[64..];
                }
                None => {
                    out.push_str(TOKEN_PREFIX);
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        *s = out;
    });
}

/// The name a restored file gets: readable, filesystem- and URL-safe, and the same every time the
/// same bytes are imported, so importing a bundle twice reuses the first copy.
fn restored_name(entry: &MediaEntry) -> String {
    let stem = Path::new(&entry.name).file_stem().and_then(|s| s.to_str()).unwrap_or("media");
    let mut clean = String::new();
    for c in stem.chars() {
        clean.push(if c.is_alphanumeric() || c == '-' { c } else { '_' });
    }
    let clean: String = clean.split('_').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("_");
    let clean: String = clean.chars().take(48).collect();
    let clean = if clean.is_empty() { "media".to_string() } else { clean };
    format!("{}-{}.{}", clean, &entry.id[..8], entry.ext)
}

// ------------------------------------------------------------------------------------------------
// Export
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize)]
pub struct ExportReport {
    pub title: String,
    pub media_count: usize,
    pub media_bytes: u64,
    /// References to media the library no longer has: left as they were in the bundle.
    pub missing_media: Vec<String>,
}

fn write_json_entry<W: Write + Seek>(zip: &mut ZipWriter<W>, name: &str, value: &impl Serialize) -> Result<(), BundleError> {
    zip.start_file(name, FileOptions::<()>::default().compression_method(CompressionMethod::Deflated))?;
    zip.write_all(&serde_json::to_vec(value)?)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_bundle<W: Write + Seek>(
    out: W,
    kind: BundleKind,
    title: String,
    counts: Counts,
    mut content: Value,
    themes: Vec<Theme>,
    mut collector: Collector<'_>,
) -> Result<ExportReport, BundleError> {
    collector.tokenize(&mut content);
    let mut theme_values = serde_json::to_value(&themes)?;
    collector.tokenize(&mut theme_values);

    let media: Vec<MediaEntry> = collector.entries.values().map(|(e, _)| e.clone()).collect();
    let media_bytes: u64 = media.iter().map(|m| m.size).sum();
    let manifest = Manifest {
        format: FORMAT_NAME.to_string(),
        format_version: FORMAT_VERSION,
        kind,
        created_by: format!("OpenSanctuary {}", env!("CARGO_PKG_VERSION")),
        created_at: chrono::Utc::now().to_rfc3339(),
        title: title.clone(),
        media: media.clone(),
        themes: themes.iter().map(|t| t.name.clone()).collect(),
        counts,
    };

    let mut zip = ZipWriter::new(out);
    write_json_entry(&mut zip, "manifest.json", &manifest)?;
    write_json_entry(&mut zip, "content.json", &content)?;
    if !themes.is_empty() {
        write_json_entry(&mut zip, "themes.json", &theme_values)?;
    }
    for entry in &media {
        let (_, path) = &collector.entries[&entry.id];
        // Already-compressed media is stored, not deflated a second time; zip64 only when needed.
        let opts = FileOptions::<()>::default().compression_method(CompressionMethod::Stored).large_file(entry.size >= 0xF000_0000);
        zip.start_file(format!("media/{}.{}", entry.id, entry.ext), opts)?;
        let mut f = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; IO_CHUNK];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            zip.write_all(&buf[..n])?;
        }
        if hex(&hasher.finalize()) != entry.id {
            return Err(invalid(format!("{} changed while it was being exported; try again", entry.name)));
        }
    }
    zip.finish()?;
    Ok(ExportReport { title, media_count: media.len(), media_bytes, missing_media: collector.missing.iter().cloned().collect() })
}

fn themes_named(db: &Database, content: &Value) -> Vec<Theme> {
    let mut names = BTreeSet::new();
    collect_theme_names(content, &mut names);
    names.iter().filter_map(|n| db.get_theme_by_name(n).ok().flatten()).collect()
}

fn count_title(single: Option<&str>, n: usize, noun: &str) -> String {
    match (single, n) {
        (Some(t), 1) => t.to_string(),
        _ => format!("{n} {noun}"),
    }
}

/// Exports `ids` (or every song when `None`) as an `.ossong` bundle.
pub fn export_songs<W: Write + Seek>(db: &Database, media_dir: &Path, ids: Option<&[String]>, out: W) -> Result<ExportReport, BundleError> {
    let songs: Vec<Song> = match ids {
        None => db.get_songs().map_err(db_err)?,
        Some(ids) => ids
            .iter()
            .map(|id| db.get_song_by_id(id).map_err(db_err)?.ok_or_else(|| BundleError::NotFound(format!("no song with id {id}"))))
            .collect::<Result<_, _>>()?,
    };
    if songs.is_empty() {
        return Err(invalid("there are no songs to export"));
    }
    let title = count_title(songs.first().map(|s| s.title.as_str()), songs.len(), "songs");
    let content = serde_json::json!({ "songs": songs });
    let themes = themes_named(db, &content);
    write_bundle(out, BundleKind::Song, title, Counts { songs: songs.len(), ..Default::default() }, content, themes, Collector::new(media_dir))
}

/// Exports `ids` (or every presentation when `None`) as an `.ospres` bundle.
pub fn export_presentations<W: Write + Seek>(db: &Database, media_dir: &Path, ids: Option<&[String]>, out: W) -> Result<ExportReport, BundleError> {
    let items: Vec<Presentation> = match ids {
        None => db.get_presentations().map_err(db_err)?,
        Some(ids) => ids
            .iter()
            .map(|id| db.get_presentation_by_id(id).map_err(db_err)?.ok_or_else(|| BundleError::NotFound(format!("no presentation with id {id}"))))
            .collect::<Result<_, _>>()?,
    };
    if items.is_empty() {
        return Err(invalid("there are no presentations to export"));
    }
    let title = count_title(items.first().map(|s| s.title.as_str()), items.len(), "presentations");
    let content = serde_json::json!({ "presentations": items });
    let themes = themes_named(db, &content);
    write_bundle(out, BundleKind::Presentation, title, Counts { presentations: items.len(), ..Default::default() }, content, themes, Collector::new(media_dir))
}

/// Exports a schedule as an `.osservice` bundle.
pub fn export_service<W: Write + Seek>(db: &Database, media_dir: &Path, schedule: &Schedule, title: Option<&str>, out: W) -> Result<ExportReport, BundleError> {
    if schedule.items.is_empty() {
        return Err(invalid("the schedule is empty, so there is nothing to export"));
    }
    // Licensed passages travel as references only; the opening side looks them up again.
    let mut schedule = schedule.without_licensed_text();
    schedule.is_modified = false;
    let title = title.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).unwrap_or_else(|| schedule.title.clone());
    schedule.title = title.clone();
    let content = serde_json::json!({ "schedule": schedule });
    let themes = themes_named(db, &content);
    write_bundle(out, BundleKind::Service, title, Counts { items: schedule.items.len(), ..Default::default() }, content, themes, Collector::new(media_dir))
}

fn bible_info_key(version: &str) -> String {
    format!("{BIBLE_INFO_PREFIX}{}", version.trim().to_uppercase())
}

fn read_bible_info(db: &Database, version: &str) -> BibleInfo {
    let mut info = db
        .get_setting(&bible_info_key(version))
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<BibleInfo>(&raw).ok())
        .unwrap_or_default();
    info.abbreviation = version.trim().to_uppercase();
    if info.name.trim().is_empty() {
        info.name = info.abbreviation.clone();
    }
    info
}

/// Exports one installed Bible translation as an `.osbible` bundle.
pub fn export_bible<W: Write + Seek>(db: &Database, version: &str, out: W) -> Result<ExportReport, BundleError> {
    let version = version.trim().to_uppercase();
    let installed = db.get_installed_bibles().map_err(db_err)?;
    if !installed.iter().any(|b| b.id.eq_ignore_ascii_case(&version)) {
        return Err(BundleError::NotFound(format!("{version} is not installed")));
    }
    let chapters: Vec<BibleChapter> = db
        .get_scriptures_for_version(&version)
        .map_err(db_err)?
        .into_iter()
        .map(|c| BibleChapter { book: c.book, chapter: c.chapter, verses: c.verses })
        .collect();
    if chapters.is_empty() {
        return Err(BundleError::NotFound(format!("{version} has no text to export")));
    }
    let info = read_bible_info(db, &version);
    let title = if info.name == info.abbreviation { info.abbreviation.clone() } else { format!("{} ({})", info.name, info.abbreviation) };
    let content = serde_json::json!({ "bible": info, "chapters": chapters });
    write_bundle(out, BundleKind::Bible, title, Counts { chapters: chapters.len(), ..Default::default() }, content, Vec::new(), Collector::new(Path::new("")))
}

// ------------------------------------------------------------------------------------------------
// Reading
// ------------------------------------------------------------------------------------------------

/// An opened, validated bundle. Nothing has been written anywhere yet.
pub struct Bundle<R: Read + Seek> {
    zip: ZipArchive<R>,
    pub manifest: Manifest,
    content: Value,
    themes: Vec<Theme>,
    /// media id -> index of its archive entry
    media_index: HashMap<String, usize>,
}

fn media_entry_name(id: &str, ext: &str) -> String {
    format!("media/{id}.{ext}")
}

fn known_ext(ext: &str) -> bool {
    !ext.is_empty() && ext.len() <= 5 && ext.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str, cap: u64) -> Result<Option<Vec<u8>>, BundleError> {
    match zip.by_name(name) {
        Ok(f) => Ok(Some(crate::storage::media_sniff::read_capped_bytes(f, cap).map_err(|_| invalid(format!("{name} is too large")))?)),
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Opens `reader` as a bundle, refusing anything that isn't exactly the format described above.
pub fn open<R: Read + Seek>(reader: R, expect: Option<BundleKind>) -> Result<Bundle<R>, BundleError> {
    let mut zip = ZipArchive::new(reader).map_err(|_| invalid("that isn't an OpenSanctuary file (it isn't a zip archive)"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(invalid("the file holds too many entries"));
    }

    // Only the entries this format defines may exist; every other name is refused outright.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut media_entries: Vec<(String, usize, u64, String)> = Vec::new();
    for i in 0..zip.len() {
        let f = zip.by_index(i)?;
        let name = f.name().map_err(|_| invalid("the file has an entry whose name isn't valid text"))?.into_owned();
        if f.is_dir() && (name == "media/" || name == "themes/") {
            continue;
        }
        if !seen.insert(name.clone()) {
            return Err(invalid(format!("the file lists {name} twice")));
        }
        match name.as_str() {
            "manifest.json" | "content.json" | "themes.json" => {}
            _ => {
                let Some(stem) = name.strip_prefix("media/") else {
                    return Err(invalid(format!("the file contains an unexpected entry: {name}")));
                };
                let Some((id, ext)) = stem.rsplit_once('.') else {
                    return Err(invalid(format!("the file contains an unexpected entry: {name}")));
                };
                if !is_sha256_hex(id) || !known_ext(ext) {
                    return Err(invalid(format!("the file contains an unexpected entry: {name}")));
                }
                media_entries.push((id.to_string(), i, f.size(), name.clone()));
            }
        }
    }

    let manifest_bytes = read_entry(&mut zip, "manifest.json", MAX_MANIFEST_BYTES)?.ok_or_else(|| invalid("that isn't an OpenSanctuary file (no manifest)"))?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.format != FORMAT_NAME {
        return Err(invalid("that isn't an OpenSanctuary file"));
    }
    if manifest.format_version > FORMAT_VERSION {
        return Err(BundleError::UnsupportedVersion { found: manifest.format_version, supported: FORMAT_VERSION });
    }
    if let Some(want) = expect {
        if want != manifest.kind {
            return Err(BundleError::WrongKind { found: manifest.kind.label(), expected: want.label() });
        }
    }

    // The media table and the archive have to agree exactly: no listed file missing, none unlisted.
    let mut media_index = HashMap::new();
    let mut total = 0u64;
    for m in &manifest.media {
        if !is_sha256_hex(&m.id) || !known_ext(&m.ext) {
            return Err(invalid("the media list contains an invalid entry"));
        }
        let wanted = media_entry_name(&m.id, &m.ext);
        let Some((_, idx, size, _)) = media_entries.iter().find(|(id, _, _, entry_name)| *id == m.id && *entry_name == wanted) else {
            return Err(invalid(format!("the file lists {} but doesn't contain it", m.name)));
        };
        if *size > MAX_MEDIA_FILE_BYTES {
            return Err(invalid(format!("{} is larger than the {} GB limit", m.name, MAX_MEDIA_FILE_BYTES >> 30)));
        }
        total = total.saturating_add(*size);
        if media_index.insert(m.id.clone(), *idx).is_some() {
            return Err(invalid("the media list names the same file twice"));
        }
    }
    if media_index.len() != media_entries.len() {
        return Err(invalid("the file contains media that its list doesn't mention"));
    }
    if total > MAX_MEDIA_TOTAL_BYTES {
        return Err(invalid(format!("the media adds up to more than {} GB", MAX_MEDIA_TOTAL_BYTES >> 30)));
    }

    let content_bytes = read_entry(&mut zip, "content.json", MAX_CONTENT_BYTES)?.ok_or_else(|| invalid("the file has no content"))?;
    let content: Value = serde_json::from_slice(&content_bytes)?;
    let themes: Vec<Theme> = match read_entry(&mut zip, "themes.json", MAX_THEMES_BYTES)? {
        Some(bytes) => serde_json::from_slice(&bytes)?,
        None => Vec::new(),
    };
    Ok(Bundle { zip, manifest, content, themes, media_index })
}

fn content_array<'a>(content: &'a Value, key: &str, max: usize, what: &str) -> Result<&'a Vec<Value>, BundleError> {
    let arr = content.get(key).and_then(Value::as_array).ok_or_else(|| invalid(format!("the file has no {what}")))?;
    if arr.is_empty() {
        return Err(invalid(format!("the file has no {what}")));
    }
    if arr.len() > max {
        return Err(invalid(format!("the file has more than {max} {what}")));
    }
    Ok(arr)
}

// ------------------------------------------------------------------------------------------------
// Inspect: what a bundle holds, and how it relates to the library, without changing anything
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CopyrightNote {
    pub title: String,
    pub author: String,
    pub copyright: String,
    pub ccli_number: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LibraryStatus {
    /// Not in the library yet.
    pub new: usize,
    /// Already there, exactly as the bundle has it.
    pub identical: usize,
    /// There under the same id but different; imported as a separate copy, never over the original.
    pub different: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub kind: BundleKind,
    pub title: String,
    pub format_version: u32,
    pub created_by: String,
    pub created_at: String,
    pub counts: Counts,
    /// Titles of what's inside (songs, presentations or service items), for showing before importing.
    pub names: Vec<String>,
    pub media_count: usize,
    pub media_bytes: u64,
    pub themes: Vec<String>,
    /// Songs whose copyright or CCLI number says they aren't public domain: the import advisory.
    pub copyrighted: Vec<CopyrightNote>,
    pub library: Option<LibraryStatus>,
    pub bible: Option<BibleInfo>,
    /// For a Bible: whether that abbreviation is already installed (so importing needs "replace").
    pub bible_installed: bool,
}

fn song_note(s: &Song) -> Option<CopyrightNote> {
    let copyright = s.copyright.clone().unwrap_or_default();
    let ccli = s.ccli_number.clone().unwrap_or_default();
    (!Database::is_public_domain_song(s)).then(|| CopyrightNote { title: s.title.clone(), author: s.author.clone(), copyright, ccli_number: ccli })
}

/// A schedule item carries its copyright/CCLI only on its slides' CCLI metadata.
fn service_item_note(item: &crate::core::models::ScheduleItem) -> Option<CopyrightNote> {
    if item.item_type != "song" {
        return None;
    }
    let meta = item.slides.iter().find_map(|s| s.ccli_metadata.as_ref())?;
    let copyright = meta.copyright.clone().unwrap_or_default();
    let ccli = meta.ccli_number.clone().unwrap_or_default();
    if meta.public_domain == Some(true) || Database::is_public_domain_metadata(&copyright, &ccli) {
        return None;
    }
    Some(CopyrightNote { title: item.title.clone(), author: meta.author.clone().unwrap_or_else(|| item.author_or_ref.clone()), copyright, ccli_number: ccli })
}

fn existing_status(existing: Option<Value>, bundle_item: &Value, collector: &mut Collector<'_>, status: &mut LibraryStatus) {
    match existing {
        None => status.new += 1,
        Some(mut ex) => {
            collector.tokenize(&mut ex);
            if &ex == bundle_item {
                status.identical += 1;
            } else {
                status.different += 1;
            }
        }
    }
}

/// Describes `bundle` and compares it with the library. Writes nothing.
pub fn inspect<R: Read + Seek>(bundle: &Bundle<R>, db: &Database, media_dir: &Path) -> Result<Summary, BundleError> {
    let m = &bundle.manifest;
    let mut summary = Summary {
        kind: m.kind,
        title: m.title.clone(),
        format_version: m.format_version,
        created_by: m.created_by.clone(),
        created_at: m.created_at.clone(),
        counts: m.counts.clone(),
        names: Vec::new(),
        media_count: m.media.len(),
        media_bytes: m.media.iter().map(|x| x.size).sum(),
        themes: bundle.themes.iter().map(|t| t.name.clone()).collect(),
        copyrighted: Vec::new(),
        library: None,
        bible: None,
        bible_installed: false,
    };
    let mut collector = Collector::new(media_dir);
    match m.kind {
        BundleKind::Song => {
            let arr = content_array(&bundle.content, "songs", MAX_SONGS, "songs")?;
            let mut status = LibraryStatus::default();
            for v in arr {
                let song: Song = serde_json::from_value(v.clone())?;
                summary.names.push(song.title.clone());
                summary.copyrighted.extend(song_note(&song));
                let existing = db.get_song_by_id(&song.id).map_err(db_err)?.map(serde_json::to_value).transpose()?;
                existing_status(existing, v, &mut collector, &mut status);
            }
            summary.counts.songs = arr.len();
            summary.library = Some(status);
        }
        BundleKind::Presentation => {
            let arr = content_array(&bundle.content, "presentations", MAX_PRESENTATIONS, "presentations")?;
            let mut status = LibraryStatus::default();
            for v in arr {
                let p: Presentation = serde_json::from_value(v.clone())?;
                summary.names.push(p.title.clone());
                let existing = db.get_presentation_by_id(&p.id).map_err(db_err)?.map(serde_json::to_value).transpose()?;
                existing_status(existing, v, &mut collector, &mut status);
            }
            summary.counts.presentations = arr.len();
            summary.library = Some(status);
        }
        BundleKind::Service => {
            let schedule: Schedule = serde_json::from_value(bundle.content.get("schedule").ok_or_else(|| invalid("the file has no schedule"))?.clone())?;
            if schedule.items.len() > MAX_SERVICE_ITEMS {
                return Err(invalid(format!("the schedule has more than {MAX_SERVICE_ITEMS} items")));
            }
            summary.names = schedule.items.iter().filter(|i| !i.is_section_header).map(|i| i.title.clone()).collect();
            summary.copyrighted = schedule.items.iter().filter_map(service_item_note).collect();
            summary.counts.items = schedule.items.len();
        }
        BundleKind::Bible => {
            let info: BibleInfo = serde_json::from_value(bundle.content.get("bible").cloned().ok_or_else(|| invalid("the file has no Bible details"))?)?;
            let chapters = content_array(&bundle.content, "chapters", MAX_BIBLE_CHAPTERS, "chapters")?;
            summary.counts.chapters = chapters.len();
            let abbr = info.abbreviation.trim().to_uppercase();
            summary.bible_installed = db.get_installed_bibles().map_err(db_err)?.iter().any(|b| b.id.eq_ignore_ascii_case(&abbr));
            summary.bible = Some(info);
        }
    }
    Ok(summary)
}

// ------------------------------------------------------------------------------------------------
// Import
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// Replace a Bible that's already installed under the same abbreviation.
    pub replace_bible: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportResult {
    pub kind: Option<BundleKind>,
    pub added: usize,
    /// Left alone because the library already had exactly that.
    pub skipped_identical: usize,
    /// Same id as something already in the library but different: kept as a new item next to it.
    pub added_as_copies: usize,
    pub media_written: usize,
    pub media_reused: usize,
    /// Pictures newly listed in the Media library, so they can be picked as backgrounds elsewhere.
    #[serde(default)]
    pub backgrounds_added: usize,
    pub themes_added: Vec<String>,
    pub bible: Option<String>,
    /// A service is returned for the console to load, not stored.
    pub schedule: Option<Schedule>,
    pub warnings: Vec<String>,
}

/// Writes every media file the bundle carries into the library's media folders and returns where each
/// one now lives (`/media/<dir>/<name>`), keyed by its id. Each file is hashed and content-sniffed as it
/// streams out of the archive; one that doesn't match what the bundle claims is not kept.
fn restore_media<R: Read + Seek>(bundle: &mut Bundle<R>, media_dir: &Path, result: &mut ImportResult) -> Result<HashMap<String, String>, BundleError> {
    let mut paths = HashMap::new();
    let entries = bundle.manifest.media.clone();
    for entry in entries {
        let name = restored_name(&entry);
        let dir = media_dir.join(entry.kind.dir());
        let dest = dir.join(&name);
        let servable = format!("/media/{}/{}", entry.kind.dir(), name);
        if dest.is_file() && std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0) == entry.size {
            result.media_reused += 1;
            paths.insert(entry.id.clone(), servable);
            continue;
        }
        std::fs::create_dir_all(&dir)?;
        let idx = bundle.media_index[&entry.id];
        let mut file = bundle.zip.by_index(idx)?;
        let tmp = dir.join(format!(".import-{}.tmp", uuid::Uuid::new_v4()));
        let outcome = (|| -> Result<(), BundleError> {
            let mut out = std::fs::File::create(&tmp)?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; IO_CHUNK];
            let mut head: Vec<u8> = Vec::new();
            let mut total = 0u64;
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > MAX_MEDIA_FILE_BYTES {
                    return Err(invalid(format!("{} is larger than the {} GB limit", entry.name, MAX_MEDIA_FILE_BYTES >> 30)));
                }
                if head.len() < IO_CHUNK {
                    head.extend_from_slice(&buf[..n.min(IO_CHUNK - head.len())]);
                }
                hasher.update(&buf[..n]);
                out.write_all(&buf[..n])?;
            }
            out.flush()?;
            if hex(&hasher.finalize()) != entry.id {
                return Err(invalid(format!("{} is damaged (its contents don't match its checksum)", entry.name)));
            }
            if entry.kind.sniff(&head) != Some(entry.ext.as_str()) {
                return Err(invalid(format!("{} isn't the kind of {} file it says it is", entry.name, entry.kind.dir().trim_end_matches('s'))));
            }
            Ok(())
        })();
        match outcome {
            Ok(()) => {
                std::fs::rename(&tmp, &dest)?;
                result.media_written += 1;
                paths.insert(entry.id.clone(), servable);
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        }
    }
    Ok(paths)
}

/// Lists each picture the bundle brought in the Media library (as an image), so it shows in the background picker and can be used on
/// other slides, not only on the slides it came with. A picture already listed (same file) is left alone.
fn register_pictures<R: Read + Seek>(bundle: &Bundle<R>, db: &Database, paths: &HashMap<String, String>, result: &mut ImportResult) -> Result<(), BundleError> {
    let listed: BTreeSet<String> = db.get_media().map_err(db_err)?.into_iter().map(|m| m.file_path).collect();
    for entry in bundle.manifest.media.iter().filter(|e| e.kind == MediaKind::Image) {
        let Some(path) = paths.get(&entry.id) else { continue };
        if listed.contains(path) {
            continue;
        }
        let stem = Path::new(&entry.name).file_stem().and_then(|s| s.to_str()).unwrap_or("Picture");
        // `lc-02-giving-66cd0052.jpg` (a file this app restored earlier) reads as "lc 02 giving": the trailing hash is dropped.
        let mut words: Vec<String> = stem.chars().map(|c| if matches!(c, '-' | '_') { ' ' } else { c }).collect::<String>().split_whitespace().map(str::to_string).collect();
        if words.len() > 1 && words.last().is_some_and(|w| w.len() == 8 && w.chars().all(|c| c.is_ascii_hexdigit())) {
            words.pop();
        }
        let name = words.join(" ");
        let item = MediaItem {
            id: format!("bundle_{}", &entry.id[..16]),
            name: if name.is_empty() { "Picture".to_string() } else { name.chars().take(80).collect() },
            media_type: "image".to_string(),
            file_path: path.clone(),
            duration_seconds: None,
            thumbnail_path: None,
            loop_playback: false,
        };
        db.insert_media(&item).map_err(db_err)?;
        result.backgrounds_added += 1;
    }
    Ok(())
}

fn add_missing_themes(bundle: &Bundle<impl Read + Seek>, db: &Database, paths: &HashMap<String, String>, result: &mut ImportResult) -> Result<(), BundleError> {
    for theme in &bundle.themes {
        if theme.name.trim().is_empty() || db.get_theme_by_name(&theme.name).map_err(db_err)?.is_some() {
            continue;
        }
        let mut v = serde_json::to_value(theme)?;
        detokenize(&mut v, paths);
        let mut t: Theme = serde_json::from_value(v)?;
        t.id = None;
        t.is_default = false;
        db.insert_theme(&t).map_err(db_err)?;
        result.themes_added.push(t.name);
    }
    Ok(())
}

enum Fate {
    Add,
    Identical,
    Copy,
}

fn decide(existing: Option<Value>, bundle_item: &Value, collector: &mut Collector<'_>) -> Fate {
    match existing {
        None => Fate::Add,
        Some(mut ex) => {
            collector.tokenize(&mut ex);
            if &ex == bundle_item {
                Fate::Identical
            } else {
                Fate::Copy
            }
        }
    }
}

fn valid_version(v: &str) -> bool {
    let v = v.trim();
    !v.is_empty() && v.len() <= 40 && v.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
}

/// Imports `bundle` into the library. For a service nothing is stored: the schedule comes back in the
/// result for the console to load, with its media restored and themes added.
pub fn import<R: Read + Seek>(bundle: &mut Bundle<R>, db: &Database, media_dir: &Path, opts: &ImportOptions) -> Result<ImportResult, BundleError> {
    let kind = bundle.manifest.kind;
    let mut result = ImportResult { kind: Some(kind), ..Default::default() };

    // Everything that can fail on its own is checked before the first write to the library.
    let bible = if kind == BundleKind::Bible {
        let info: BibleInfo = serde_json::from_value(bundle.content.get("bible").cloned().ok_or_else(|| invalid("the file has no Bible details"))?)?;
        if !valid_version(&info.abbreviation) {
            return Err(invalid("the Bible's abbreviation is missing or not valid"));
        }
        let abbr = info.abbreviation.trim().to_uppercase();
        if !opts.replace_bible && db.get_installed_bibles().map_err(db_err)?.iter().any(|b| b.id.eq_ignore_ascii_case(&abbr)) {
            return Err(BundleError::AlreadyInstalled(abbr));
        }
        Some((info, abbr, bible_items(&bundle.content)?))
    } else {
        None
    };

    let paths = restore_media(bundle, media_dir, &mut result)?;
    register_pictures(bundle, db, &paths, &mut result)?;
    add_missing_themes(bundle, db, &paths, &mut result)?;
    let mut content = bundle.content.clone();
    let mut collector = Collector::new(media_dir);

    match kind {
        BundleKind::Song => {
            for v in content_array(&content, "songs", MAX_SONGS, "songs")?.clone() {
                let id = v.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                let existing = if id.is_empty() { None } else { db.get_song_by_id(&id).map_err(db_err)?.map(serde_json::to_value).transpose()? };
                let mut item = v.clone();
                match decide(existing, &v, &mut collector) {
                    Fate::Identical => result.skipped_identical += 1,
                    fate => {
                        if matches!(fate, Fate::Copy) || id.is_empty() {
                            item["id"] = Value::String(format!("song_{}", uuid::Uuid::new_v4()));
                            result.added_as_copies += 1;
                        } else {
                            result.added += 1;
                        }
                        detokenize(&mut item, &paths);
                        let song: Song = serde_json::from_value(item)?;
                        db.insert_song(&song).map_err(db_err)?;
                    }
                }
            }
        }
        BundleKind::Presentation => {
            for v in content_array(&content, "presentations", MAX_PRESENTATIONS, "presentations")?.clone() {
                let id = v.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                let existing = if id.is_empty() { None } else { db.get_presentation_by_id(&id).map_err(db_err)?.map(serde_json::to_value).transpose()? };
                let mut item = v.clone();
                match decide(existing, &v, &mut collector) {
                    Fate::Identical => result.skipped_identical += 1,
                    fate => {
                        if matches!(fate, Fate::Copy) || id.is_empty() {
                            item["id"] = Value::String(format!("pres_{}", uuid::Uuid::new_v4()));
                            result.added_as_copies += 1;
                        } else {
                            result.added += 1;
                        }
                        detokenize(&mut item, &paths);
                        let p: Presentation = serde_json::from_value(item)?;
                        db.insert_presentation(&p).map_err(db_err)?;
                    }
                }
            }
        }
        BundleKind::Service => {
            let sv = content.get_mut("schedule").ok_or_else(|| invalid("the file has no schedule"))?;
            detokenize(sv, &paths);
            let schedule: Schedule = serde_json::from_value(sv.clone())?;
            if schedule.items.len() > MAX_SERVICE_ITEMS {
                return Err(invalid(format!("the schedule has more than {MAX_SERVICE_ITEMS} items")));
            }
            result.added = schedule.items.len();
            result.schedule = Some(schedule);
        }
        BundleKind::Bible => {
            let (info, abbr, items) = bible.expect("a Bible bundle always parsed its chapters above");
            if opts.replace_bible {
                db.delete_installed_bible(&abbr).map_err(db_err)?;
            }
            db.insert_scriptures_batch(&items).map_err(db_err)?;
            let stored = BibleInfo { abbreviation: abbr.clone(), ..info };
            db.set_setting(&bible_info_key(&abbr), &serde_json::to_string(&stored)?).map_err(db_err)?;
            result.added = items.len();
            result.bible = Some(abbr);
        }
    }
    Ok(result)
}

/// The chapters of a Bible bundle as the library stores them, validated the way the plugin host
/// validates a Bible it is handed.
fn bible_items(content: &Value) -> Result<Vec<ScriptureItem>, BundleError> {
    let abbr = content
        .get("bible")
        .and_then(|b| b.get("abbreviation"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_uppercase();
    let chapters: Vec<BibleChapter> = serde_json::from_value(Value::Array(content_array(content, "chapters", MAX_BIBLE_CHAPTERS, "chapters")?.clone()))?;
    let mut items = Vec::with_capacity(chapters.len());
    for c in chapters {
        let book = c.book.trim().to_string();
        if book.is_empty() || book.len() > MAX_NAME {
            return Err(invalid(format!("a chapter needs a book name of up to {MAX_NAME} characters")));
        }
        if c.verses.is_empty() || c.verses.len() > MAX_VERSES_PER_CHAPTER || c.verses.iter().any(|v| v.text.len() > MAX_VERSE_TEXT) {
            return Err(invalid(format!("{book} {} must have 1-{MAX_VERSES_PER_CHAPTER} verses of up to {MAX_VERSE_TEXT} bytes each", c.chapter)));
        }
        let verse_start = c.verses.first().map(|v| v.verse_number).unwrap_or(1);
        let verse_end = c.verses.last().map(|v| v.verse_number).unwrap_or(verse_start);
        // The same id the plugin host gives a chapter, so a Bible installed either way is the same rows.
        let id = format!("scrip_{}", uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, format!("{}:{}:{}-{}:{}", book, c.chapter, verse_start, verse_end, abbr).as_bytes()));
        items.push(ScriptureItem {
            id,
            reference: format!("{} {}", book, c.chapter),
            book,
            chapter: c.chapter,
            verse_start,
            verse_end,
            version: abbr.clone(),
            verses: c.verses,
            theme_name: None,
        });
    }
    Ok(items)
}

#[cfg(test)]
mod tests;
