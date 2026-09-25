use std::fs::File;
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;
use rusqlite::Connection;
use tempfile::NamedTempFile;
use zip::write::FileOptions;
use zip::{ZipArchive, ZipWriter};
use crate::core::models::{
    identity_arrangement, ArrangementEntry, ElementEffects, ElementTransform, Schedule,
    ScheduleItem, Slide, SlideBackground, SlideElement, TextBlock, TextParagraphStyle, TextRun,
};
use crate::storage::rtf;

/// EasyWorship type code to section prefix: 1 → V, 2 → C, 3 → B, 4 → E, 5 → P
pub fn ew_type_code_to_prefix(type_code: i64) -> &'static str {
    match type_code {
        1 => "V",
        2 => "C",
        3 => "B",
        4 => "E",
        5 => "P",
        _ => "S",
    }
}

/// EasyWorship type code to section label
pub fn ew_type_code_to_label(type_code: i64, number: i64) -> String {
    match type_code {
        1 => format!("Verse {}", number),
        2 => format!("Chorus {}", number),
        3 => format!("Bridge {}", number),
        4 => format!("Ending {}", number),
        5 => format!("Pre-Chorus {}", number),
        _ => format!("Section {}", number),
    }
}

/// Map section_id / tag / title to EasyWorship (Type, Number)
pub fn section_id_to_ew_type_and_number(section_id: &str, default_index: usize) -> (i64, i64) {
    let s = section_id.trim();
    if s.is_empty() {
        return (1, (default_index + 1) as i64);
    }
    let lower = s.to_lowercase();
    let (prefix_char, num_str) = if lower.starts_with("pre-chorus") || lower.starts_with("prechorus") {
        ('P', s.trim_start_matches(|c: char| !c.is_numeric()))
    } else if lower.starts_with("verse") {
        ('V', s.trim_start_matches(|c: char| !c.is_numeric()))
    } else if lower.starts_with("chorus") {
        ('C', s.trim_start_matches(|c: char| !c.is_numeric()))
    } else if lower.starts_with("bridge") {
        ('B', s.trim_start_matches(|c: char| !c.is_numeric()))
    } else if lower.starts_with("ending") {
        ('E', s.trim_start_matches(|c: char| !c.is_numeric()))
    } else {
        let first = s.chars().next().unwrap().to_ascii_uppercase();
        (first, &s[1..])
    };

    let type_code = match prefix_char {
        'V' => 1,
        'C' => 2,
        'B' => 3,
        'E' => 4,
        'P' => 5,
        _ => 1,
    };
    let number = num_str.trim().parse::<i64>().unwrap_or((default_index + 1) as i64);
    (type_code, number)
}

/// Builds a single TextBlock element from parsed RTF runs, or `None` when there's no
/// real formatting to represent — matches `Slide::synthesize_v2_if_needed`'s own
/// "only populate when needed" convention, so a plain-text slide still gets the
/// generic default element synthesized later rather than a redundant explicit one here.
fn text_block_from_rtf(doc: &rtf::RtfDocument) -> Option<SlideElement> {
    let runs = doc.to_text_runs();
    if runs.is_empty() {
        return None;
    }
    Some(SlideElement::TextBlock {
        id: format!("el_{}", uuid::Uuid::new_v4()),
        transform: ElementTransform::default(),
        block: TextBlock {
            runs,
            paragraph_style: TextParagraphStyle {
                align: doc.alignment.clone().unwrap_or_else(|| "center".to_string()),
                ..Default::default()
            },
            effects: ElementEffects::default(),
            autofit: true,
        },
    })
}

/// Determines the (table_name, file_path) to write a slide's background resource into,
/// preferring the structured `background_v2` and falling back to the legacy `background`
/// string (same Image-vs-Solid heuristic `Slide::synthesize_v2_if_needed` already uses).
/// Returns `None` for solid/gradient/pattern backgrounds — this schema only models media
/// (image/video) resources, matching what `parse_ewsx_sqlite`'s reader looks for.
fn slide_background_resource(slide: &Slide) -> Option<(&'static str, String)> {
    if let Some(bg) = &slide.background_v2 {
        return match bg {
            SlideBackground::Image { file_path, .. } => Some(("resource_image", file_path.clone())),
            SlideBackground::Video { file_path, .. } => Some(("resource_video", file_path.clone())),
            _ => None,
        };
    }
    let raw = slide.background.as_deref()?.trim();
    if raw.is_empty() || raw.starts_with('#') || raw.starts_with("rgb") {
        return None;
    }
    const VIDEO_EXTS: &[&str] = &[".mp4", ".mov", ".webm", ".avi", ".mkv", ".m4v"];
    let lower = raw.to_lowercase();
    if VIDEO_EXTS.iter().any(|ext| lower.ends_with(ext)) {
        Some(("resource_video", raw.to_string()))
    } else {
        Some(("resource_image", raw.to_string()))
    }
}

/// Builds the RTF to persist for a slide: the slide's real TextBlock formatting when
/// present (the inverse of `rtf::parse_rtf` + `text_block_from_rtf`), otherwise a minimal
/// single-run fallback wrapping the plain text.
fn rtf_for_slide(slide: &Slide) -> String {
    let text_block = slide.elements.iter().find_map(|el| match el {
        SlideElement::TextBlock { block, .. } => Some(block),
        _ => None,
    });
    match text_block {
        Some(block) if !block.runs.is_empty() => text_runs_to_rtf(&block.runs, &block.paragraph_style.align),
        _ => format!(
            "{{\\rtf1\\ansi\\deff0 {{\\fonttbl {{\\f0 Segoe UI;}}}}\\f0\\fs40 {}}}",
            slide.text.replace('\n', "\\par ")
        ),
    }
}

/// Serializes rich TextRuns into a minimal valid RTF document — the write-side inverse
/// of `rtf::parse_rtf`. Builds a font table and color table by first-use order (so
/// `\f0`/`\cf1` etc. line up with the tables actually emitted), then one paired
/// on/off-formatted run per TextRun.
fn text_runs_to_rtf(runs: &[TextRun], align: &str) -> String {
    let mut fonts: Vec<String> = Vec::new();
    let mut colors: Vec<String> = Vec::new();

    let mut font_index_of = |name: &str| -> usize {
        match fonts.iter().position(|f| f == name) {
            Some(idx) => idx,
            None => {
                fonts.push(name.to_string());
                fonts.len() - 1
            }
        }
    };
    let mut color_index_of = |hex: &str| -> usize {
        match colors.iter().position(|c| c == hex) {
            Some(idx) => idx + 1, // \cf0 is reserved for "auto" — real colors start at 1
            None => {
                colors.push(hex.to_string());
                colors.len()
            }
        }
    };

    let align_code = match align {
        "left" => "\\ql",
        "right" => "\\qr",
        "justify" => "\\qj",
        _ => "\\qc",
    };
    let mut body = format!("\\pard{} ", align_code);

    for run in runs {
        let f_idx = font_index_of(&run.font_family);
        let c_idx = color_index_of(&run.color);
        let half_pt = (run.font_size_pt * 2.0).round().max(2.0) as i64;
        body.push_str(&format!("\\f{}\\fs{}\\cf{}", f_idx, half_pt, c_idx));
        if run.bold {
            body.push_str("\\b");
        }
        if run.italic {
            body.push_str("\\i");
        }
        if run.underline {
            body.push_str("\\ul");
        }
        if run.strike {
            body.push_str("\\strike");
        }
        body.push(' ');
        for ch in run.text.chars() {
            match ch {
                '\\' => body.push_str("\\\\"),
                '{' => body.push_str("\\{"),
                '}' => body.push_str("\\}"),
                '\n' => body.push_str("\\par "),
                '\t' => body.push_str("\\tab "),
                c if c.is_ascii() => body.push(c),
                c => body.push_str(&format!("\\u{}?", c as u32)),
            }
        }
        // Paired on/off toggles, matching the style `rtf::parse_rtf` was tested against.
        // No trailing delimiter space needed here: every one of these words is followed
        // either by nothing (end of the run list) or by the next run's own `\f...`
        // control word, and a backslash unambiguously ends any control word regardless
        // of whether it has a numeric parameter — unlike the space right before the run's
        // literal text above, which IS required (text starting with a letter/digit would
        // otherwise be read as continuing the preceding word/parameter).
        if run.bold {
            body.push_str("\\b0");
        }
        if run.italic {
            body.push_str("\\i0");
        }
        if run.underline {
            body.push_str("\\ulnone");
        }
        if run.strike {
            body.push_str("\\strike0");
        }
    }

    let mut header = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl");
    for (idx, f) in fonts.iter().enumerate() {
        header.push_str(&format!("{{\\f{} {};}}", idx, f));
    }
    header.push('}');
    header.push_str("{\\colortbl;");
    for c in &colors {
        let (r, g, b) = hex_to_rgb(c);
        header.push_str(&format!("\\red{}\\green{}\\blue{};", r, g, b));
    }
    header.push('}');

    format!("{}{}}}", header, body)
}

fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim_start_matches('#');
    if h.len() == 6 {
        let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(255);
        let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(255);
        (r, g, b)
    } else {
        (255, 255, 255)
    }
}

pub struct EwsxManager;

impl EwsxManager {
    /// Save a Schedule struct into a standard .ewsx zip archive with main.db, manifest.json, and Schedule.xml
    pub fn save_schedule_to_ewsx(schedule: &Schedule, path: impl AsRef<Path>) -> Result<(), String> {
        let file = File::create(path.as_ref()).map_err(|e| format!("Failed to create file: {}", e))?;
        Self::write_ewsx_zip(schedule, file)?;
        Ok(())
    }

    /// Same real EWSX package (zip containing a native-compatible `main.db`
    /// SQLite database, `Schedule.xml`, and `manifest.json`) as
    /// `save_schedule_to_ewsx`, but returned as bytes instead of written to a
    /// server-local path — for a save flow that has to hand the file to a
    /// browser download rather than a filesystem location (see
    /// `POST /api/schedule/export` in `src/api/routes.rs`, which this backs).
    pub fn save_schedule_to_ewsx_bytes(schedule: &Schedule) -> Result<Vec<u8>, String> {
        let cursor = Self::write_ewsx_zip(schedule, Cursor::new(Vec::new()))?;
        Ok(cursor.into_inner())
    }

    fn write_ewsx_zip<W: Write + Seek>(schedule: &Schedule, writer: W) -> Result<W, String> {
        let mut zip_writer = ZipWriter::new(writer);
        let options = FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o755);

        // 1. Build SQLite main.db for 100% native EasyWorship 7 compatibility
        let tmp_db_file = NamedTempFile::new().map_err(|e| e.to_string())?;
        let mut conn = Connection::open(tmp_db_file.path()).map_err(|e| e.to_string())?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS presentation (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT,
                author TEXT,
                copyright TEXT,
                reference_number TEXT,
                presentation_type INTEGER
            );
            CREATE TABLE IF NOT EXISTS slide (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                presentation_id INTEGER,
                title TEXT,
                order_index INTEGER
            );
            CREATE TABLE IF NOT EXISTS element (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                slide_id INTEGER,
                foreground_resource_id INTEGER,
                background_resource_id INTEGER,
                element_type INTEGER,
                element_style_type INTEGER
            );
            CREATE TABLE IF NOT EXISTS resource_text (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                resource_id INTEGER,
                rtf TEXT
            );
            CREATE TABLE IF NOT EXISTS resource_image (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                resource_id INTEGER,
                file_path TEXT
            );
            CREATE TABLE IF NOT EXISTS resource_video (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                resource_id INTEGER,
                file_path TEXT
            );
            CREATE TABLE IF NOT EXISTS Verses (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                presentation_id INTEGER,
                Type INTEGER,
                Number INTEGER,
                Words TEXT
            );
            CREATE TABLE IF NOT EXISTS PlayOrder (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                presentation_id INTEGER,
                POrder INTEGER,
                Type INTEGER,
                Number INTEGER
            );"
        ).map_err(|e| e.to_string())?;

        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut res_id = 1;
        let mut bg_res_id = 1;
        for (p_idx, item) in schedule.items.iter().enumerate() {
            let p_type = if item.item_type == "song" { 6 } else { 1 };
            tx.execute(
                "INSERT INTO presentation (rowid, title, author, copyright, reference_number, presentation_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6);",
                rusqlite::params![
                    p_idx + 1,
                    item.title,
                    item.author_or_ref,
                    "",
                    "",
                    p_type
                ],
            ).map_err(|e| e.to_string())?;

            for (s_idx, slide) in item.slides.iter().enumerate() {
                let (type_code, num) = section_id_to_ew_type_and_number(
                    slide.tag.as_deref().or(slide.label.as_deref()).unwrap_or(""),
                    s_idx,
                );

                // Prefer the slide's real rich-text formatting (bold/italic/color/font/etc.)
                // when present; otherwise fall back to a minimal single-run RTF wrapping the
                // plain text. Written into BOTH `Verses.Words` and `resource_text.rtf` so
                // formatting survives regardless of which table a reader consults first.
                let rtf_content = rtf_for_slide(slide);

                tx.execute(
                    "INSERT INTO Verses (presentation_id, Type, Number, Words) VALUES (?1, ?2, ?3, ?4);",
                    rusqlite::params![p_idx + 1, type_code, num, rtf_content],
                ).map_err(|e| e.to_string())?;

                let slide_rowid = (p_idx * 100) + s_idx + 1;
                tx.execute(
                    "INSERT INTO slide (rowid, presentation_id, title, order_index)
                     VALUES (?1, ?2, ?3, ?4);",
                    rusqlite::params![slide_rowid, p_idx + 1, slide.label.as_deref().unwrap_or(""), s_idx],
                ).map_err(|e| e.to_string())?;

                tx.execute(
                    "INSERT INTO resource_text (resource_id, rtf) VALUES (?1, ?2);",
                    rusqlite::params![res_id, rtf_content],
                ).map_err(|e| e.to_string())?;

                // Background image/video: written into its own resource table + linked via
                // element.background_resource_id, mirroring the linkage this same module's
                // reader (parse_ewsx_sqlite) looks for.
                let background_resource_id: Option<i64> = match slide_background_resource(slide) {
                    Some((table, file_path)) => {
                        tx.execute(
                            &format!("INSERT INTO {} (resource_id, file_path) VALUES (?1, ?2);", table),
                            rusqlite::params![bg_res_id, file_path],
                        ).map_err(|e| e.to_string())?;
                        let id = bg_res_id;
                        bg_res_id += 1;
                        Some(id)
                    }
                    None => None,
                };

                tx.execute(
                    "INSERT INTO element (slide_id, foreground_resource_id, background_resource_id, element_type, element_style_type)
                     VALUES (?1, ?2, ?3, ?4, ?5);",
                    rusqlite::params![slide_rowid, res_id, background_resource_id, 6, 4],
                ).map_err(|e| e.to_string())?;

                res_id += 1;
            }

            for (p_order, entry) in item.arrangement.iter().enumerate() {
                let (type_code, num) = section_id_to_ew_type_and_number(&entry.section_id, entry.source_slide_index);
                tx.execute(
                    "INSERT INTO PlayOrder (presentation_id, POrder, Type, Number) VALUES (?1, ?2, ?3, ?4);",
                    rusqlite::params![p_idx + 1, (p_order + 1) as i64, type_code, num],
                ).map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        drop(conn);

        // Read SQLite db bytes and write into zip
        let mut db_bytes = Vec::new();
        File::open(tmp_db_file.path()).map_err(|e| e.to_string())?.read_to_end(&mut db_bytes).map_err(|e| e.to_string())?;

        zip_writer.start_file("main.db", options).map_err(|e| e.to_string())?;
        zip_writer.write_all(&db_bytes).map_err(|e| e.to_string())?;

        // 2. Write Schedule.xml
        zip_writer.start_file("Schedule.xml", options).map_err(|e| e.to_string())?;
        let xml_content = Self::schedule_to_xml(schedule);
        zip_writer.write_all(xml_content.as_bytes()).map_err(|e| e.to_string())?;

        // 3. Write Manifest JSON
        zip_writer.start_file("manifest.json", options).map_err(|e| e.to_string())?;
        let json_manifest = serde_json::to_string_pretty(schedule).map_err(|e| e.to_string())?;
        zip_writer.write_all(json_manifest.as_bytes()).map_err(|e| e.to_string())?;

        let writer = zip_writer.finish().map_err(|e| format!("Zip finish error: {}", e))?;
        Ok(writer)
    }

    /// Load from a filesystem path, extracting any embedded background media (e.g. a
    /// FreeShow show's inline base64 image) into the default `web/media/images`
    /// directory relative to the process's current working directory. Prefer
    /// `load_schedule_from_ewsx_with_media_dir` in any real server context — the
    /// default here is only correct when the process happens to be launched from the
    /// web root, which a real deployment cannot assume (see the `_with_media_dir`
    /// doc comment below for why this matters).
    pub fn load_schedule_from_ewsx(path: impl AsRef<Path>) -> Result<Schedule, String> {
        Self::load_schedule_from_ewsx_with_media_dir(path, Path::new("web/media/images"))
    }

    /// Same as `load_schedule_from_ewsx`, but writes any extracted background media
    /// into `media_dir` instead of assuming a CWD-relative default. Callers with a
    /// real configured web directory (i.e. any HTTP route) must use this, not the
    /// CWD-relative convenience wrapper above.
    pub fn load_schedule_from_ewsx_with_media_dir(path: impl AsRef<Path>, media_dir: &Path) -> Result<Schedule, String> {
        let bytes = std::fs::read(path.as_ref()).map_err(|e| format!("Failed to read file: {}", e))?;
        let stem = path.as_ref().file_stem().and_then(|s| s.to_str()).unwrap_or("Schedule");
        Self::load_schedule_from_bytes_with_media_dir(&bytes, stem, media_dir)
    }

    /// Load from raw binary bytes (base64 decoded or direct file upload), extracting
    /// any embedded background media into the default `web/media/images` directory
    /// relative to the process's current working directory. This default is only
    /// correct for tests and ad hoc CLI use where CWD == the web root — a real
    /// server process can be launched from anywhere (see `--web-dir`), so any HTTP
    /// route handler must call `load_schedule_from_bytes_with_media_dir` with the
    /// app's actual configured web directory instead of this convenience wrapper.
    pub fn load_schedule_from_bytes(bytes: &[u8], title_hint: &str) -> Result<Schedule, String> {
        Self::load_schedule_from_bytes_with_media_dir(bytes, title_hint, Path::new("web/media/images"))
    }

    /// Same as `load_schedule_from_bytes`, but writes any extracted background media
    /// (currently: a FreeShow show's inline base64 image backgrounds) into
    /// `media_dir` instead of a hardcoded, CWD-relative path. Without this, a
    /// resolved `/media/images/<file>` URL in the returned Schedule can point at a
    /// file written somewhere the web server never serves from — confirmed live: a
    /// server started with an explicit `--web-dir` from a different CWD wrote the
    /// file next to the CWD instead, and the returned image URL 404'd.
    pub fn load_schedule_from_bytes_with_media_dir(bytes: &[u8], title_hint: &str, media_dir: &Path) -> Result<Schedule, String> {
        // 1. Try EasyWorship legacy .ews binary format (Version 5/6)
        if bytes.starts_with(b"EasyWorship Schedule File") {
            return Self::parse_ews_binary(bytes, title_hint);
        }

        // 2. Try raw SQLite DB (starts with "SQLite format 3\0")
        if bytes.starts_with(b"SQLite format 3\0") {
            return Self::parse_ewsx_sqlite(bytes, title_hint);
        }

        // 3. Try ZipArchive (EWSX / OSZ / ZIP / EWPX)
        let cursor = Cursor::new(bytes);
        if let Ok(mut archive) = ZipArchive::new(cursor) {
            // A. Check for manifest.json / schedule.json
            for json_name in &["manifest.json", "schedule.json"] {
                if let Ok(mut manifest_file) = archive.by_name(json_name) {
                    let mut content = String::new();
                    if manifest_file.read_to_string(&mut content).is_ok() {
                        if let Ok(sched) = serde_json::from_str::<Schedule>(&content) {
                            return Ok(sched);
                        }
                    }
                }
            }

            // B. Check for native EasyWorship main.db (or any .db database inside zip)
            for db_name in &["main.db", "Songs.db", "songs.db", "schedule.db", "presentation.db"] {
                if let Ok(mut db_file) = archive.by_name(db_name) {
                    let mut db_bytes = Vec::new();
                    if db_file.read_to_end(&mut db_bytes).is_ok() {
                        if let Ok(sched) = Self::parse_ewsx_sqlite(&db_bytes, title_hint) {
                            return Ok(sched);
                        }
                    }
                }
            }

            // Also search all files in archive ending with .db or .sqlite
            for i in 0..archive.len() {
                if let Ok(mut file) = archive.by_index(i) {
                    let name = file.name().to_lowercase();
                    if name.ends_with(".db") || name.ends_with(".sqlite") {
                        let mut db_bytes = Vec::new();
                        if file.read_to_end(&mut db_bytes).is_ok() {
                            if let Ok(sched) = Self::parse_ewsx_sqlite(&db_bytes, title_hint) {
                                return Ok(sched);
                            }
                        }
                    }
                }
            }

            // C. Check for Schedule.xml
            if let Ok(mut xml_file) = archive.by_name("Schedule.xml") {
                let mut xml_str = String::new();
                if xml_file.read_to_string(&mut xml_str).is_ok() {
                    if let Ok(sched) = Self::parse_schedule_xml(&xml_str) {
                        return Ok(sched);
                    }
                }
            }

            // D. Check for OpenLP service files (*.osj or service_data.json)
            for i in 0..archive.len() {
                if let Ok(mut file) = archive.by_index(i) {
                    let name = file.name().to_lowercase();
                    if name.ends_with(".osj") || name == "service_data.json" || name.ends_with("/service_data.json") {
                        let mut osj_str = String::new();
                        if file.read_to_string(&mut osj_str).is_ok() {
                            if let Ok(sched) = crate::storage::OpenLPImporter::import_openlp_service_json(&osj_str) {
                                if !sched.items.is_empty() {
                                    return Ok(sched);
                                }
                            }
                        }
                    }
                }
            }

            // E. Check for FreeShow show files (*.show)
            for i in 0..archive.len() {
                if let Ok(mut file) = archive.by_index(i) {
                    let name = file.name().to_lowercase();
                    if name.ends_with(".show") || name.ends_with(".project") {
                        let mut show_str = String::new();
                        if file.read_to_string(&mut show_str).is_ok() {
                            if let Ok(sched) = crate::storage::FreeShowShowImporter::import_show_json_str(&show_str, media_dir) {
                                if !sched.items.is_empty() {
                                    return Ok(sched);
                                }
                            }
                        }
                    }
                }
            }
        }

        // 4. Try UTF-8 Plain Text (JSON, XML, or Text)
        if let Ok(text) = std::str::from_utf8(bytes) {
            let trimmed = text.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                if let Ok(sched) = serde_json::from_str::<Schedule>(trimmed) {
                    return Ok(sched);
                }
                if let Ok(sched) = crate::storage::FreeShowShowImporter::import_show_json_str(trimmed, media_dir) {
                    if !sched.items.is_empty() {
                        return Ok(sched);
                    }
                }
                if let Ok(sched) = crate::storage::OpenLPImporter::import_openlp_service_json(trimmed) {
                    if !sched.items.is_empty() {
                        return Ok(sched);
                    }
                }
            }
            if trimmed.starts_with('<') {
                if let Ok(sched) = Self::parse_schedule_xml(trimmed) {
                    return Ok(sched);
                }
            }
        }

        Err("No valid schedule data found in uploaded file".to_string())
    }

    /// Parse EasyWorship main.db SQLite database
    fn parse_ewsx_sqlite(db_bytes: &[u8], title_hint: &str) -> Result<Schedule, String> {
        let mut tmp_file = NamedTempFile::new().map_err(|e| e.to_string())?;
        tmp_file.write_all(db_bytes).map_err(|e| e.to_string())?;

        let conn = Connection::open(tmp_file.path()).map_err(|e| format!("SQLite open error: {}", e))?;

        let mut schedule = Schedule {
            id: format!("sched_{}", uuid::Uuid::new_v4()),
            title: title_hint.to_string(),
            items: Vec::new(),
            selected_item_index: None,
            is_modified: false,
            schedule_version: 0,
        };

        // Query available table names
        let mut table_stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table';")
            .map_err(|e| e.to_string())?;
        let table_names: Vec<String> = table_stmt.query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();

        let find_table = |name: &str| -> Option<String> {
            table_names.iter().find(|t| t.eq_ignore_ascii_case(name)).cloned()
        };

        let get_cols = |tbl: &str| -> Vec<String> {
            if let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info(\"{}\");", tbl)) {
                if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) {
                    return rows.filter_map(Result::ok).collect();
                }
            }
            Vec::new()
        };

        // Resolves `element.background_resource_id` to an actual image/video file path.
        // This app's own writer (`save_schedule_to_ewsx` below) uses `resource_image` /
        // `resource_video` tables, but a genuine third-party EasyWorship export's exact
        // table/column names for media resources aren't confirmed against a real sample,
        // so several plausible candidates are tried — the same flexible discovery already
        // used for Verses/PlayOrder above rather than assuming one fixed schema.
        const BG_IMAGE_TABLES: &[&str] = &["resource_image", "resource_media_image", "image"];
        const BG_VIDEO_TABLES: &[&str] = &["resource_video", "resource_media_video", "video"];
        const BG_PATH_COLUMNS: &[&str] = &["file_path", "path", "filename", "file_name", "url", "src", "location"];
        let resolve_background = |resource_id: i64| -> Option<SlideBackground> {
            let try_tables = |tables: &[&str]| -> Option<String> {
                for t in tables {
                    let tbl = match find_table(*t) {
                        Some(tbl) => tbl,
                        None => continue,
                    };
                    let cols = get_cols(&tbl);
                    let col = match BG_PATH_COLUMNS.iter().find(|c| cols.iter().any(|tc| tc.eq_ignore_ascii_case(*c))) {
                        Some(col) => col,
                        None => continue,
                    };
                    let sql = format!("SELECT \"{}\" FROM \"{}\" WHERE resource_id = ?1 OR rowid = ?1 LIMIT 1;", col, tbl);
                    if let Ok(path) = conn.query_row(&sql, rusqlite::params![resource_id], |r| r.get::<_, String>(0)) {
                        if !path.trim().is_empty() {
                            return Some(path.trim().to_string());
                        }
                    }
                }
                None
            };
            if let Some(path) = try_tables(BG_IMAGE_TABLES) {
                return Some(SlideBackground::Image { file_path: path, opacity: 1.0 });
            }
            if let Some(path) = try_tables(BG_VIDEO_TABLES) {
                return Some(SlideBackground::Video { file_path: path, loop_playback: true, is_muted: false });
            }
            None
        };
        let apply_background = |slide: &mut Slide, bg: SlideBackground| {
            slide.background = Some(match &bg {
                SlideBackground::Image { file_path, .. } => file_path.clone(),
                SlideBackground::Video { file_path, .. } => file_path.clone(),
                _ => return,
            });
            slide.background_v2 = Some(bg);
        };
        // `Verses` (the common path for song lyrics) has no column linking a row back to
        // an `element`/`background_resource_id` — that linkage only exists via `slide`.
        // Positionally matching `slide.order_index` to the Nth slide `load_verses` produced
        // is a heuristic (this app's own writer below keeps them in sync 1:1; a genuine
        // third-party export's ordering guarantee is unconfirmed) but it's the only bridge
        // available without a real per-slide foreign key between the two tables.
        let load_slide_backgrounds_by_order = |presentation_id: i64| -> Vec<(i64, i64)> {
            if find_table("slide").is_none() || find_table("element").is_none() {
                return Vec::new();
            }
            let sql = "SELECT s.order_index, e.background_resource_id
                       FROM slide s
                       JOIN element e ON e.slide_id = s.rowid
                       WHERE s.presentation_id = ?1 AND e.background_resource_id IS NOT NULL
                       ORDER BY s.order_index ASC;";
            let mut pairs = Vec::new();
            if let Ok(mut stmt) = conn.prepare(sql) {
                if let Ok(rows) = stmt.query_map(rusqlite::params![presentation_id], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                }) {
                    pairs.extend(rows.flatten());
                }
            }
            pairs
        };
        let apply_backgrounds_to_slides = |slides: &mut [Slide], presentation_id: i64| {
            for (order_idx, bg_id) in load_slide_backgrounds_by_order(presentation_id) {
                if let Some(slide) = slides.get_mut(order_idx as usize) {
                    if let Some(bg) = resolve_background(bg_id) {
                        apply_background(slide, bg);
                    }
                }
            }
        };

        let verses_tbl = find_table("Verses").or_else(|| find_table("song_words"));
        let playorder_tbl = find_table("PlayOrder").or_else(|| find_table("play_order"));
        let pres_tbl = find_table("presentation");
        let song_tbl = find_table("song").or_else(|| find_table("songs"));

        let load_verses = |tbl: &str, id_filter: Option<(&str, i64)>| -> Vec<Slide> {
            let cols = get_cols(tbl);
            let type_col = cols.iter().find(|c| c.eq_ignore_ascii_case("Type") || c.eq_ignore_ascii_case("verse_type")).map(|s| s.as_str()).unwrap_or("Type");
            let num_col = cols.iter().find(|c| c.eq_ignore_ascii_case("Number") || c.eq_ignore_ascii_case("verse_number")).map(|s| s.as_str()).unwrap_or("Number");
            let words_col = cols.iter().find(|c| {
                c.eq_ignore_ascii_case("Words") || c.eq_ignore_ascii_case("text") || c.eq_ignore_ascii_case("rtf") || c.eq_ignore_ascii_case("content") || c.eq_ignore_ascii_case("lyrics")
            }).map(|s| s.as_str()).unwrap_or("Words");

            let sql = match id_filter {
                Some((col, _)) => format!("SELECT {}, {}, {} FROM \"{}\" WHERE {} = ?1 ORDER BY rowid ASC;", type_col, num_col, words_col, tbl, col),
                None => format!("SELECT {}, {}, {} FROM \"{}\" ORDER BY rowid ASC;", type_col, num_col, words_col, tbl),
            };

            let mut slides = Vec::new();
            if let Ok(mut stmt) = conn.prepare(&sql) {
                let mapper = |r: &rusqlite::Row| -> rusqlite::Result<(i64, i64, String)> {
                    Ok((
                        r.get::<_, Option<i64>>(0).unwrap_or(Some(1)).unwrap_or(1),
                        r.get::<_, Option<i64>>(1).unwrap_or(Some(1)).unwrap_or(1),
                        r.get::<_, Option<String>>(2).unwrap_or_default().unwrap_or_default(),
                    ))
                };
                let rows_res = if let Some((_, id_val)) = id_filter {
                    stmt.query_map(rusqlite::params![id_val], mapper)
                } else {
                    stmt.query_map([], mapper)
                };

                if let Ok(rows) = rows_res {
                    for r in rows.flatten() {
                        let (t_code, num, words) = r;
                        let (clean, elements) = if words.starts_with("{\\rtf") {
                            let doc = rtf::parse_rtf(&words);
                            (doc.plain_text(), text_block_from_rtf(&doc))
                        } else {
                            (words.trim().to_string(), None)
                        };
                        if clean.is_empty() {
                            continue;
                        }
                        let section_id = format!("{}{}", ew_type_code_to_prefix(t_code), num);
                        let label = ew_type_code_to_label(t_code, num);
                        let has_elements = elements.is_some();
                        slides.push(Slide {
                            text: clean,
                            header: Some(label.clone()),
                            label: Some(label),
                            tag: Some(section_id),
                            background: None,
                            notes: None,
                            elements: elements.into_iter().collect(),
                            slide_document_version: if has_elements { 1 } else { 0 },
                            ..Default::default()
                        });
                    }
                }
            }
            slides
        };

        let load_playorder = |tbl: &str, id_filter: Option<(&str, i64)>, slides: &[Slide]| -> Vec<ArrangementEntry> {
            let cols = get_cols(tbl);
            let porder_col = cols.iter().find(|c| c.eq_ignore_ascii_case("POrder") || c.eq_ignore_ascii_case("order_index") || c.eq_ignore_ascii_case("play_order")).map(|s| s.as_str()).unwrap_or("POrder");
            let type_col = cols.iter().find(|c| c.eq_ignore_ascii_case("Type") || c.eq_ignore_ascii_case("verse_type")).map(|s| s.as_str()).unwrap_or("Type");
            let num_col = cols.iter().find(|c| c.eq_ignore_ascii_case("Number") || c.eq_ignore_ascii_case("verse_number")).map(|s| s.as_str()).unwrap_or("Number");

            let sql = match id_filter {
                Some((col, _)) => format!("SELECT {}, {} FROM \"{}\" WHERE {} = ?1 ORDER BY {} ASC;", type_col, num_col, tbl, col, porder_col),
                None => format!("SELECT {}, {} FROM \"{}\" ORDER BY {} ASC;", type_col, num_col, tbl, porder_col),
            };

            let mut entries = Vec::new();
            if let Ok(mut stmt) = conn.prepare(&sql) {
                let po_mapper = |r: &rusqlite::Row| -> rusqlite::Result<(i64, i64)> {
                    Ok((
                        r.get::<_, Option<i64>>(0).unwrap_or(Some(1)).unwrap_or(1),
                        r.get::<_, Option<i64>>(1).unwrap_or(Some(1)).unwrap_or(1),
                    ))
                };
                let rows_res = if let Some((_, id_val)) = id_filter {
                    stmt.query_map(rusqlite::params![id_val], po_mapper)
                } else {
                    stmt.query_map([], po_mapper)
                };

                if let Ok(rows) = rows_res {
                    for r in rows.flatten() {
                        let (t_code, num) = r;
                        let section_id = format!("{}{}", ew_type_code_to_prefix(t_code), num);
                        let source_idx = slides.iter().position(|s| {
                            s.tag.as_deref() == Some(&section_id)
                                || s.label.as_deref() == Some(&section_id)
                                || s.tag.as_deref().map(|t| t.eq_ignore_ascii_case(&section_id)).unwrap_or(false)
                                || s.label.as_deref().map(|l| l.eq_ignore_ascii_case(&ew_type_code_to_label(t_code, num))).unwrap_or(false)
                        }).unwrap_or_else(|| {
                            if (num as usize) >= 1 && (num as usize) <= slides.len() {
                                (num as usize) - 1
                            } else {
                                0
                            }
                        });

                        entries.push(ArrangementEntry {
                            section_id,
                            source_slide_index: source_idx.min(slides.len().saturating_sub(1)),
                            background_override: None,
                        });
                    }
                }
            }
            entries
        };

        if let Some(ref p_tbl) = pres_tbl {
            let sql = format!("SELECT rowid, title, author, presentation_type FROM \"{}\" ORDER BY rowid ASC;", p_tbl);
            if let Ok(mut pres_stmt) = conn.prepare(&sql) {
                let pres_rows = pres_stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i32>>(3)?,
                    ))
                }).map_err(|e| e.to_string())?;

                for p_res in pres_rows {
                    let (rowid, title_opt, author_opt, p_type) = p_res.map_err(|e| e.to_string())?;
                    let title = title_opt.unwrap_or_else(|| "Untitled Item".to_string());
                    let author = author_opt.unwrap_or_default();
                    let item_type = if p_type == Some(6) { "song".to_string() } else { "presentation".to_string() };

                    let mut item = ScheduleItem {
                        id: format!("item_{}", uuid::Uuid::new_v4()),
                        title: title.clone(),
                        item_type,
                        author_or_ref: author.clone(),
                        slides: Vec::new(),
                        background: None,
                        theme_name: None,
                        is_expanded: false,
                        is_section_header: false,
                        subtitle: if author.is_empty() { None } else { Some(author.clone()) },
                        notes: None,
                        arrangement: Vec::new(),
                        default_slide_duration_seconds: None,
                        slideshow_loop: false,
                    };

                    // Check if Verses table exists and has rows for this presentation
                    if let Some(ref v_tbl) = verses_tbl {
                        let v_cols = get_cols(v_tbl);
                        let id_col = v_cols.iter().find(|c| c.eq_ignore_ascii_case("presentation_id") || c.eq_ignore_ascii_case("song_id"));
                        let id_filter = id_col.map(|c| (c.as_str(), rowid));
                        item.slides = load_verses(v_tbl, id_filter);
                        apply_backgrounds_to_slides(&mut item.slides, rowid);
                    }

                    // If no slides loaded from Verses table, try slide + element + resource_text.
                    // For real songs (presentation_type = 6), EasyWorship marks the actual lyric
                    // text element with element_style_type = 4, distinct from other text elements
                    // on the same slide (e.g. secondary captions) — matching OpenLP's importer.
                    // Other presentation types don't follow that convention (this sample file's
                    // announcement slides use style types 6/3, never 4), so the extra filter is
                    // only applied for songs.
                    if item.slides.is_empty() && find_table("slide").is_some() {
                        let style_filter = if item.item_type == "song" { " AND e.element_style_type = 4" } else { "" };
                        let sql = format!(
                            "SELECT rt.rtf, s.title, s.order_index, e.background_resource_id
                             FROM slide as s
                             LEFT JOIN element as e ON e.slide_id = s.rowid AND e.element_type = 6{style_filter}
                             LEFT JOIN resource_text as rt ON rt.resource_id = e.foreground_resource_id
                             WHERE s.presentation_id = ?1
                             ORDER BY s.order_index ASC, s.rowid ASC;"
                        );
                        let mut slide_stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;

                        let slide_rows = slide_stmt.query_map([rowid], |srow| {
                            Ok((
                                srow.get::<_, Option<String>>(0)?,
                                srow.get::<_, Option<String>>(1)?,
                                srow.get::<_, Option<i32>>(2)?,
                                srow.get::<_, Option<i64>>(3)?,
                            ))
                        }).map_err(|e| e.to_string())?;

                        let mut seen_texts = std::collections::HashSet::new();
                        let mut slide_count = 0;

                        for s_res in slide_rows {
                            let (rtf_opt, slide_title_opt, _, bg_resource_id) = s_res.map_err(|e| e.to_string())?;
                            if let Some(rtf_text) = rtf_opt {
                                let doc = rtf::parse_rtf(&rtf_text);
                                let clean = doc.plain_text();
                                if !clean.is_empty() && !seen_texts.contains(&clean) {
                                    seen_texts.insert(clean.clone());
                                    slide_count += 1;

                                    let raw_title = slide_title_opt.as_deref().unwrap_or("");
                                    let (t_code, num) = if !raw_title.trim().is_empty() {
                                        section_id_to_ew_type_and_number(raw_title, slide_count - 1)
                                    } else if clean.to_lowercase().starts_with("chorus") {
                                        (2, 1)
                                    } else if clean.to_lowercase().starts_with("bridge") {
                                        (3, 1)
                                    } else if clean.to_lowercase().starts_with("ending") {
                                        (4, 1)
                                    } else {
                                        (1, slide_count as i64)
                                    };

                                    let section_id = format!("{}{}", ew_type_code_to_prefix(t_code), num);
                                    let label = if !raw_title.trim().is_empty() {
                                        raw_title.to_string()
                                    } else {
                                        ew_type_code_to_label(t_code, num)
                                    };

                                    let elements = text_block_from_rtf(&doc);
                                    let has_elements = elements.is_some();
                                    let mut slide = Slide {
                                        text: clean,
                                        header: Some(label.clone()),
                                        label: Some(label),
                                        background: None,
                                        notes: None,
                                        tag: Some(section_id),
                                        elements: elements.into_iter().collect(),
                                        slide_document_version: if has_elements { 1 } else { 0 },
                                        ..Default::default()
                                    };
                                    if let Some(id) = bg_resource_id {
                                        if let Some(bg) = resolve_background(id) {
                                            apply_background(&mut slide, bg);
                                        }
                                    }
                                    item.slides.push(slide);
                                }
                            }
                        }
                    }

                    if item.slides.is_empty() {
                        item.slides.push(Slide {
                            text: title.clone(),
                            header: Some("Slide 1".to_string()),
                            label: Some("V1".to_string()),
                            background: None,
                            notes: None,
                            tag: Some("V1".to_string()),
                            ..Default::default()
                        });
                    }

                    // Check PlayOrder table
                    if let Some(ref po_tbl) = playorder_tbl {
                        let po_cols = get_cols(po_tbl);
                        let id_col = po_cols.iter().find(|c| c.eq_ignore_ascii_case("presentation_id") || c.eq_ignore_ascii_case("song_id"));
                        let id_filter = id_col.map(|c| (c.as_str(), rowid));
                        item.arrangement = load_playorder(po_tbl, id_filter, &item.slides);
                    }

                    if item.arrangement.is_empty() {
                        item.arrangement = identity_arrangement(&item.slides);
                    }

                    schedule.items.push(item);
                }
            }
        } else if let Some(ref s_tbl) = song_tbl {
            let sql = format!("SELECT rowid, title, author FROM \"{}\" ORDER BY rowid ASC;", s_tbl);
            if let Ok(mut song_stmt) = conn.prepare(&sql) {
                let song_rows = song_stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                }).map_err(|e| e.to_string())?;

                for s_res in song_rows {
                    let (rowid, title_opt, author_opt, _) = match s_res {
                        Ok((r, t, a)) => (r, t, a, ()),
                        Err(_) => continue,
                    };
                    let title = title_opt.unwrap_or_else(|| "Untitled Song".to_string());
                    let author = author_opt.unwrap_or_default();

                    let mut item = ScheduleItem {
                        id: format!("item_{}", uuid::Uuid::new_v4()),
                        title: title.clone(),
                        item_type: "song".to_string(),
                        author_or_ref: author.clone(),
                        slides: Vec::new(),
                        background: None,
                        theme_name: None,
                        is_expanded: false,
                        is_section_header: false,
                        subtitle: if author.is_empty() { None } else { Some(author.clone()) },
                        notes: None,
                        arrangement: Vec::new(),
                        default_slide_duration_seconds: None,
                        slideshow_loop: false,
                    };

                    if let Some(ref v_tbl) = verses_tbl {
                        let v_cols = get_cols(v_tbl);
                        let id_col = v_cols.iter().find(|c| c.eq_ignore_ascii_case("song_id") || c.eq_ignore_ascii_case("presentation_id"));
                        let id_filter = id_col.map(|c| (c.as_str(), rowid));
                        item.slides = load_verses(v_tbl, id_filter);
                        apply_backgrounds_to_slides(&mut item.slides, rowid);
                    }

                    if item.slides.is_empty() {
                        item.slides.push(Slide {
                            text: title.clone(),
                            header: Some("Verse 1".to_string()),
                            label: Some("V1".to_string()),
                            background: None,
                            notes: None,
                            tag: Some("V1".to_string()),
                            ..Default::default()
                        });
                    }

                    if let Some(ref po_tbl) = playorder_tbl {
                        let po_cols = get_cols(po_tbl);
                        let id_col = po_cols.iter().find(|c| c.eq_ignore_ascii_case("song_id") || c.eq_ignore_ascii_case("presentation_id"));
                        let id_filter = id_col.map(|c| (c.as_str(), rowid));
                        item.arrangement = load_playorder(po_tbl, id_filter, &item.slides);
                    }

                    if item.arrangement.is_empty() {
                        item.arrangement = identity_arrangement(&item.slides);
                    }

                    schedule.items.push(item);
                }
            }
        } else if verses_tbl.is_some() || playorder_tbl.is_some() {
            let mut item = ScheduleItem {
                id: format!("item_{}", uuid::Uuid::new_v4()),
                title: title_hint.to_string(),
                item_type: "song".to_string(),
                author_or_ref: String::new(),
                slides: Vec::new(),
                background: None,
                theme_name: None,
                is_expanded: false,
                is_section_header: false,
                subtitle: None,
                notes: None,
                arrangement: Vec::new(),
                default_slide_duration_seconds: None,
                slideshow_loop: false,
            };

            if let Some(ref v_tbl) = verses_tbl {
                item.slides = load_verses(v_tbl, None);
            }

            if item.slides.is_empty() {
                item.slides.push(Slide {
                    text: title_hint.to_string(),
                    header: Some("Verse 1".to_string()),
                    label: Some("V1".to_string()),
                    background: None,
                    notes: None,
                    tag: Some("V1".to_string()),
                    ..Default::default()
                });
            }

            if let Some(ref po_tbl) = playorder_tbl {
                item.arrangement = load_playorder(po_tbl, None, &item.slides);
            }

            if item.arrangement.is_empty() {
                item.arrangement = identity_arrangement(&item.slides);
            }

            schedule.items.push(item);
        }

        if !schedule.items.is_empty() {
            schedule.selected_item_index = Some(0);
        }

        Ok(schedule)
    }

    /// Parse legacy EasyWorship .ews (Version 5/6 binary stream)
    fn parse_ews_binary(bytes: &[u8], title_hint: &str) -> Result<Schedule, String> {
        let mut schedule = Schedule {
            id: format!("sched_{}", uuid::Uuid::new_v4()),
            title: title_hint.to_string(),
            items: Vec::new(),
            selected_item_index: None,
            is_modified: false,
            schedule_version: 0,
        };

        // Scan for strings / RTF blocks in binary file
        let mut text_blocks: Vec<(String, Option<SlideElement>)> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i..].starts_with(b"{\\rtf") {
                // Find closing brace
                let mut depth = 0;
                let start = i;
                while i < bytes.len() {
                    if bytes[i] == b'{' { depth += 1; }
                    else if bytes[i] == b'}' {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    i += 1;
                }
                if let Ok(rtf_str) = std::str::from_utf8(&bytes[start..i]) {
                    let doc = rtf::parse_rtf(rtf_str);
                    let clean = doc.plain_text();
                    if !clean.is_empty() {
                        text_blocks.push((clean, text_block_from_rtf(&doc)));
                    }
                }
            } else {
                i += 1;
            }
        }

        if !text_blocks.is_empty() {
            let mut slides = Vec::new();
            for (idx, (txt, element)) in text_blocks.into_iter().enumerate() {
                let has_element = element.is_some();
                slides.push(Slide {
                    text: txt,
                    header: Some(format!("Verse {}", idx + 1)),
                    label: Some(format!("V{}", idx + 1)),
                    background: None,
                    notes: None,
                    tag: Some(format!("V{}", idx + 1)),
                    elements: element.into_iter().collect(),
                    slide_document_version: if has_element { 1 } else { 0 },
                    ..Default::default()
                });
            }
            schedule.items.push(ScheduleItem {
                id: format!("item_{}", uuid::Uuid::new_v4()),
                title: title_hint.to_string(),
                item_type: "song".to_string(),
                author_or_ref: String::new(),
                arrangement: crate::core::models::identity_arrangement(&slides),
                slides,
                background: None,
                theme_name: None,
                is_expanded: false,
                is_section_header: false,
                subtitle: None,
                notes: None,
                default_slide_duration_seconds: None,
                slideshow_loop: false,
            });
            schedule.selected_item_index = Some(0);
            return Ok(schedule);
        }

        // Fallback: extract ASCII title from header
        let title_bytes = if bytes.len() > 60 { &bytes[38..80] } else { b"Sample Schedule" };
        let title = extract_cstring(title_bytes);

        let slides = vec![Slide {
                text: title_hint.to_string(),
                header: Some("Slide 1".to_string()),
                label: Some("V1".to_string()),
                background: None,
                notes: None,
                tag: Some("V1".to_string()),
                ..Default::default()
            }];
            let arrangement = crate::core::models::identity_arrangement(&slides);

            schedule.items.push(ScheduleItem {
                id: format!("item_{}", uuid::Uuid::new_v4()),
                title: if title.is_empty() { title_hint.to_string() } else { title },
                item_type: "song".to_string(),
                author_or_ref: String::new(),
                slides,
                background: None,
                theme_name: None,
                is_expanded: false,
                is_section_header: false,
                subtitle: None,
                notes: None,
                arrangement,
                default_slide_duration_seconds: None,
                slideshow_loop: false,
            });
        schedule.selected_item_index = Some(0);

        Ok(schedule)
    }

    fn schedule_to_xml(schedule: &Schedule) -> String {
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        xml.push_str("<Schedule Version=\"1.0\">\n");
        xml.push_str(&format!("  <Title>{}</Title>\n", escape_xml(&schedule.title)));
        xml.push_str("  <Items>\n");

        for item in &schedule.items {
            xml.push_str("    <Item>\n");
            xml.push_str(&format!("      <Id>{}</Id>\n", escape_xml(&item.id)));
            xml.push_str(&format!("      <Title>{}</Title>\n", escape_xml(&item.title)));
            xml.push_str(&format!("      <Type>{}</Type>\n", escape_xml(&item.item_type)));
            xml.push_str(&format!("      <AuthorOrRef>{}</AuthorOrRef>\n", escape_xml(&item.author_or_ref)));
            xml.push_str(&format!("      <IsSectionHeader>{}</IsSectionHeader>\n", item.is_section_header));
            xml.push_str("      <Slides>\n");

            for slide in &item.slides {
                xml.push_str("        <Slide>\n");
                xml.push_str(&format!("          <Text>{}</Text>\n", escape_xml(&slide.text)));
                if let Some(ref h) = slide.header {
                    xml.push_str(&format!("          <Header>{}</Header>\n", escape_xml(h)));
                }
                if let Some(ref l) = slide.label {
                    xml.push_str(&format!("          <Label>{}</Label>\n", escape_xml(l)));
                }
                if let Some(ref t) = slide.tag {
                    xml.push_str(&format!("          <Tag>{}</Tag>\n", escape_xml(t)));
                }
                if let Some(ref bg) = slide.background {
                    xml.push_str(&format!("          <Background>{}</Background>\n", escape_xml(bg)));
                }
                xml.push_str("        </Slide>\n");
            }

            xml.push_str("      </Slides>\n");

            if !item.arrangement.is_empty() {
                xml.push_str("      <Arrangement>\n");
                for entry in &item.arrangement {
                    xml.push_str("        <Entry>\n");
                    xml.push_str(&format!("          <SectionId>{}</SectionId>\n", escape_xml(&entry.section_id)));
                    xml.push_str(&format!("          <SourceSlideIndex>{}</SourceSlideIndex>\n", entry.source_slide_index));
                    if let Some(ref bg) = entry.background_override {
                        xml.push_str(&format!("          <BackgroundOverride>{}</BackgroundOverride>\n", escape_xml(bg)));
                    }
                    xml.push_str("        </Entry>\n");
                }
                xml.push_str("      </Arrangement>\n");
            }

            xml.push_str("    </Item>\n");
        }

        xml.push_str("  </Items>\n");
        xml.push_str("</Schedule>\n");
        xml
    }

    fn parse_schedule_xml(xml: &str) -> Result<Schedule, String> {
        let mut schedule = Schedule::default();
        
        if let Some(title) = extract_tag(xml, "Title") {
            schedule.title = title;
        }

        let item_blocks: Vec<&str> = xml.split("<Item>").skip(1).collect();
        for block in item_blocks {
            let item_chunk = block.split("</Item>").next().unwrap_or("");
            let title = extract_tag(item_chunk, "Title").unwrap_or_else(|| "Untitled Item".to_string());
            let item_type = extract_tag(item_chunk, "Type").unwrap_or_else(|| "song".to_string());
            let author_or_ref = extract_tag(item_chunk, "AuthorOrRef").unwrap_or_default();
            let is_section_header = extract_tag(item_chunk, "IsSectionHeader")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false);

            let mut slides = Vec::new();
            let slide_blocks: Vec<&str> = item_chunk.split("<Slide>").skip(1).collect();
            for s_block in slide_blocks {
                let s_chunk = s_block.split("</Slide>").next().unwrap_or("");
                let text = extract_tag(s_chunk, "Text").unwrap_or_default();
                let header = extract_tag(s_chunk, "Header");
                let label = extract_tag(s_chunk, "Label");
                let tag = extract_tag(s_chunk, "Tag").or_else(|| label.clone());
                let background = extract_tag(s_chunk, "Background");

                slides.push(Slide {
                    text: unescape_xml(&text),
                    header: header.map(|h| unescape_xml(&h)),
                    label: label.map(|l| unescape_xml(&l)),
                    background: background.map(|b| unescape_xml(&b)),
                    notes: None,
                    tag: tag.map(|t| unescape_xml(&t)),
                    ..Default::default()
                });
            }

            let mut arrangement = Vec::new();
            if let Some(arr_block) = extract_tag(item_chunk, "Arrangement") {
                let entry_blocks: Vec<&str> = arr_block.split("<Entry>").skip(1).collect();
                for e_block in entry_blocks {
                    let e_chunk = e_block.split("</Entry>").next().unwrap_or("");
                    let section_id = extract_tag(e_chunk, "SectionId").unwrap_or_else(|| "V1".to_string());
                    let source_slide_index = extract_tag(e_chunk, "SourceSlideIndex")
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(0);
                    let background_override = extract_tag(e_chunk, "BackgroundOverride")
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| unescape_xml(&s));
                    arrangement.push(ArrangementEntry {
                        section_id: unescape_xml(&section_id),
                        source_slide_index,
                        background_override,
                    });
                }
            }
            if arrangement.is_empty() {
                arrangement = identity_arrangement(&slides);
            }

            schedule.items.push(ScheduleItem {
                id: format!("item_{}", uuid::Uuid::new_v4()),
                title: unescape_xml(&title),
                item_type,
                author_or_ref: unescape_xml(&author_or_ref),
                arrangement,
                slides,
                background: None,
                theme_name: None,
                is_expanded: false,
                is_section_header,
                notes: None,
                subtitle: None,
                default_slide_duration_seconds: None,
                slideshow_loop: false,
            });
        }

        Ok(schedule)
    }
}


fn extract_cstring(bytes: &[u8]) -> String {
    let null_pos = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let slice = &bytes[..null_pos];
    String::from_utf8(slice.to_vec())
        .unwrap_or_else(|_| slice.iter().map(|&b| b as char).collect())
        .trim()
        .to_string()
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
     .replace('\'', "&apos;")
}

fn unescape_xml(s: &str) -> String {
    s.replace("&amp;", "&")
     .replace("&lt;", "<")
     .replace("&gt;", ">")
     .replace("&quot;", "\"")
     .replace("&apos;", "'")
}

fn extract_tag(source: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = source.find(&open)? + open.len();
    let end = source[start..].find(&close)?;
    Some(source[start..start + end].trim().to_string())
}
