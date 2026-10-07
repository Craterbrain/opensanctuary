use std::path::{Path, PathBuf};
use rusqlite::Connection;
use crate::core::models::{GradientStop, Slide, SlideBackground, Schedule, ScheduleItem, ScheduleItemType, ScriptureItem, ScriptureVerse, Song};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OpenLpImportSummary {
    pub success: bool,
    pub songs_count: usize,
    pub bibles_count: usize,
    pub bible_versions: Vec<String>,
    pub message: String,
}

pub struct OpenLPImporter;

impl OpenLPImporter {
    /// Import songs from an OpenLP SQLite songs database (`songs.sqlite` / `songs.db`)
    pub fn import_songs_db<P: AsRef<Path>>(path: P) -> Result<Vec<Song>, Box<dyn std::error::Error>> {
        Self::import_songs_db_with_media(path, None)
    }

    /// Import songs from an OpenLP SQLite songs database with optional media directory for theme backgrounds
    pub fn import_songs_db_with_media<P: AsRef<Path>>(
        path: P,
        media_dir: Option<&Path>,
    ) -> Result<Vec<Song>, Box<dyn std::error::Error>> {
        let conn = Connection::open(path.as_ref())?;

        // Determine if authors and authors_songs tables exist
        let has_authors: bool = conn.query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='authors';",
            [],
            |r| r.get(0),
        ).unwrap_or(false);

        let has_theme_name: bool = {
            let mut stmt = conn.prepare("PRAGMA table_info(songs);")?;
            let mut rows = stmt.query([])?;
            let mut found = false;
            while let Some(row) = rows.next()? {
                let col: String = row.get(1)?;
                if col == "theme_name" {
                    found = true;
                    break;
                }
            }
            found
        };

        let theme_col = if has_theme_name { "s.theme_name" } else { "NULL" };

        let query = if has_authors {
            format!(
                "SELECT s.id, s.title, s.alternate_title, s.lyrics, s.copyright, s.ccli_number, s.comments,
                        GROUP_CONCAT(COALESCE(NULLIF(a.display_name, ''), TRIM(COALESCE(a.first_name, '') || ' ' || COALESCE(a.last_name, ''))), ', ') AS author_names,
                        {} AS theme_name
                 FROM songs s
                 LEFT JOIN authors_songs as_rel ON s.id = as_rel.song_id
                 LEFT JOIN authors a ON as_rel.author_id = a.id
                 GROUP BY s.id
                 ORDER BY s.title ASC;",
                theme_col
            )
        } else {
            format!(
                "SELECT s.id, s.title, s.alternate_title, s.lyrics, s.copyright, s.ccli_number, s.comments,
                        NULL AS author_names,
                        {} AS theme_name
                 FROM songs s
                 ORDER BY s.title ASC;",
                theme_col
            )
        };

        let mut stmt = conn.prepare(&query)?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })?;

        // Collect search directories for resolving theme backgrounds
        let mut search_dirs = Vec::new();
        if let Some(parent) = path.as_ref().parent() {
            search_dirs.push(parent.to_path_buf());
            search_dirs.push(parent.join("themes"));
            if let Some(grandparent) = parent.parent() {
                search_dirs.push(grandparent.to_path_buf());
                search_dirs.push(grandparent.join("themes"));
            }
        }
        for local_dir in Self::find_local_openlp_dirs() {
            search_dirs.push(local_dir.clone());
            search_dirs.push(local_dir.join("themes"));
        }

        let mut songs = Vec::new();
        for r in rows {
            let (_, title, alt_title, lyrics_xml_opt, copyright, ccli, _, author_opt, theme_name_opt) = r?;
            let author = author_opt
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .unwrap_or_else(|| "Unknown".to_string());

            let mut song = Song::new(&title, &author);
            song.id = format!("song_{}", uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, format!("{}:{}", title, author).as_bytes()));
            song.alternate_title = alt_title;
            song.copyright = copyright;
            song.ccli_number = ccli;
            song.theme_name = theme_name_opt.clone();

            if let Some(lyrics_xml) = lyrics_xml_opt {
                let mut verse_idx = 1;
                let mut chorus_idx = 1;
                let mut bridge_idx = 1;
                let mut prechorus_idx = 1;
                let mut intro_idx = 1;
                let mut ending_idx = 1;
                let mut other_idx = 1;

                for part in lyrics_xml.split("<verse") {
                    if let Some(tag_end) = part.find('>') {
                        let header_attr = &part[..tag_end];
                        let content_part = &part[tag_end + 1..];
                        if let Some(close_tag) = content_part.find("</verse>") {
                            let raw_text = &content_part[..close_tag];
                            let clean_text = raw_text
                                .replace("<![CDATA[", "")
                                .replace("]]>", "")
                                .replace("<lines>", "")
                                .replace("</lines>", "\n")
                                .replace("<br/>", "\n")
                                .replace("<br />", "\n")
                                .replace("<br>", "\n")
                                .replace("\r\n", "\n")
                                .replace('\r', "\n");

                            let clean_trimmed = clean_text.trim().to_string();
                            if clean_trimmed.is_empty() {
                                continue;
                            }

                            // Extract explicit verse number from header if present (e.g. label="2" or name="v2")
                            let num_from_attr = extract_number_from_attr(header_attr);

                            let lower_header = header_attr.to_lowercase();
                            let is_chorus = lower_header.contains("type=\"c") || lower_header.contains("type='c") || lower_header.contains("name=\"c") || lower_header.contains("name='c");
                            let is_bridge = lower_header.contains("type=\"b") || lower_header.contains("type='b") || lower_header.contains("name=\"b") || lower_header.contains("name='b");
                            let is_prechorus = lower_header.contains("type=\"p") || lower_header.contains("type='p") || lower_header.contains("name=\"p") || lower_header.contains("name='p");
                            let is_intro = lower_header.contains("type=\"i") || lower_header.contains("type='i") || lower_header.contains("name=\"i") || lower_header.contains("name='i");
                            let is_ending = lower_header.contains("type=\"e") || lower_header.contains("type='e") || lower_header.contains("name=\"e") || lower_header.contains("name='e");
                            let is_other = lower_header.contains("type=\"o") || lower_header.contains("type='o") || lower_header.contains("name=\"o") || lower_header.contains("name='o");

                            let (tag, label) = if is_chorus {
                                let idx = num_from_attr.unwrap_or(chorus_idx);
                                chorus_idx = idx + 1;
                                (format!("C{}", idx), format!("Chorus {}", idx))
                            } else if is_bridge {
                                let idx = num_from_attr.unwrap_or(bridge_idx);
                                bridge_idx = idx + 1;
                                (format!("B{}", idx), format!("Bridge {}", idx))
                            } else if is_prechorus {
                                let idx = num_from_attr.unwrap_or(prechorus_idx);
                                prechorus_idx = idx + 1;
                                (format!("P{}", idx), format!("Pre-Chorus {}", idx))
                            } else if is_intro {
                                let idx = num_from_attr.unwrap_or(intro_idx);
                                intro_idx = idx + 1;
                                (format!("I{}", idx), format!("Intro {}", idx))
                            } else if is_ending {
                                let idx = num_from_attr.unwrap_or(ending_idx);
                                ending_idx = idx + 1;
                                (format!("E{}", idx), format!("Ending {}", idx))
                            } else if is_other {
                                let idx = num_from_attr.unwrap_or(other_idx);
                                other_idx = idx + 1;
                                (format!("O{}", idx), format!("Other {}", idx))
                            } else {
                                let idx = num_from_attr.unwrap_or(verse_idx);
                                verse_idx = idx + 1;
                                (format!("V{}", idx), format!("Verse {}", idx))
                            };

                            song.add_slide(label, tag, clean_trimmed);
                        }
                    }
                }
            }

            if song.slides.is_empty() {
                let title = song.title.clone();
                song.add_slide("Verse 1", "V1", title);
            }

            // A2: Resolve theme background and apply to every slide of the song
            if let Some(ref tname) = theme_name_opt {
                if let Some(bg) = Self::resolve_theme_background(tname, &search_dirs, media_dir) {
                    let legacy_bg = slide_background_to_legacy(&bg);
                    for slide in &mut song.slides {
                        slide.background = Some(legacy_bg.clone());
                        slide.background_v2 = Some(bg.clone());
                    }
                }
            }

            songs.push(song);
        }

        Ok(songs)
    }

    /// Import Bibles from an OpenLP SQLite bible database (`KJV.sqlite`, `ASV.sqlite`, etc.)
    pub fn import_bible_db<P: AsRef<Path>>(path: P) -> Result<Vec<ScriptureItem>, Box<dyn std::error::Error>> {
        let conn = Connection::open(path.as_ref())?;

        let version_name = conn.query_row(
            "SELECT value FROM metadata WHERE LOWER(key) = 'version' OR LOWER(key) = 'name' LIMIT 1;",
            [],
            |r| r.get::<_, String>(0),
        ).unwrap_or_else(|_| {
            path.as_ref().file_stem().and_then(|s| s.to_str()).unwrap_or("KJV").to_string()
        });

        let mut stmt = conn.prepare(
            "SELECT b.name, v.chapter, v.verse, v.text
             FROM verse v
             JOIN book b ON v.book_id = b.id
             ORDER BY b.id ASC, v.chapter ASC, v.verse ASC;",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;

        use std::collections::BTreeMap;
        let mut chapter_map: BTreeMap<(String, u32), Vec<ScriptureVerse>> = BTreeMap::new();

        for r in rows {
            let (book_name, chapter, verse_num, raw_text) = r?;
            // Clean OpenLP Bible formatting tags like {su}...{/su}
            let clean_text = clean_bible_verse_text(&raw_text);

            let entry = chapter_map.entry((book_name, chapter)).or_default();
            entry.push(ScriptureVerse {
                verse_number: verse_num,
                text: clean_text,
            });
        }

        let mut scriptures = Vec::new();
        for ((book, chapter), verses) in chapter_map {
            if verses.is_empty() {
                continue;
            }
            let first_verse = verses.first().map(|v| v.verse_number).unwrap_or(1);
            let last_verse = verses.last().map(|v| v.verse_number).unwrap_or(1);

            let mut item = ScriptureItem::new(&book, chapter, first_verse, last_verse, &version_name);
            item.id = format!("scrip_{}", uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, format!("{}:{}:{}-{}:{}", book, chapter, first_verse, last_verse, version_name).as_bytes()));
            item.verses = verses;
            scriptures.push(item);
        }

        Ok(scriptures)
    }

    /// Import OpenLP Service JSON / `.osj` data into a Schedule struct
    pub fn import_openlp_service_json(json_str: &str) -> Result<Schedule, Box<dyn std::error::Error>> {
        Self::import_openlp_service_json_with_media(json_str, None)
    }

    /// Import OpenLP Service JSON / `.osj` data into a Schedule struct with optional media directory
    pub fn import_openlp_service_json_with_media(
        json_str: &str,
        media_dir: Option<&Path>,
    ) -> Result<Schedule, Box<dyn std::error::Error>> {
        let value: serde_json::Value = serde_json::from_str(json_str)?;
        let mut schedule = Schedule {
            id: format!("sched_{}", uuid::Uuid::new_v4()),
            title: "OpenLP Service".to_string(),
            items: Vec::new(),
            selected_item_index: None,
            is_modified: false,
            schedule_version: 0,
        };

        let entries = if let Some(array) = value.as_array() {
            array.clone()
        } else if let Some(service) = value.get("service").and_then(|s| s.as_array()) {
            service.clone()
        } else {
            vec![value]
        };

        for entry in entries {
            // Check for service-level metadata (e.g. openlp_core.service-theme)
            if let Some(core) = entry.get("openlp_core") {
                if let Some(theme) = core.get("service-theme").and_then(|t| t.as_str()) {
                    if !theme.is_empty() && schedule.title == "OpenLP Service" {
                        schedule.title = format!("OpenLP Service ({})", theme);
                    }
                }
                continue;
            }

            let service_item = entry.get("serviceitem").unwrap_or(&entry);
            if let Some(header) = service_item.get("header") {
                let title = header
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Untitled Service Item");

                let plugin = header
                    .get("name")
                    .or_else(|| header.get("plugin"))
                    .and_then(|p| p.as_str())
                    .unwrap_or("songs");

                let item_type = match plugin {
                    "bibles" | "bible" => ScheduleItemType::Scripture,
                    "media" => ScheduleItemType::Media,
                    "presentations" | "presentation" => ScheduleItemType::Presentation,
                    _ => ScheduleItemType::Song,
                };

                let mut item = ScheduleItem::new(item_type, title);
                if let Some(notes) = header.get("notes").and_then(|n| n.as_str()).filter(|n| !n.is_empty()) {
                    item.notes = Some(notes.to_string());
                    item.subtitle = Some(notes.to_string());
                }

                if let Some(data_array) = service_item.get("data").and_then(|d| d.as_array()) {
                    for (idx, slide_val) in data_array.iter().enumerate() {
                        let text = slide_val
                            .get("raw_slide")
                            .or_else(|| slide_val.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");

                        let clean_text = clean_slide_text(text);
                        let verse_tag = slide_val.get("verseTag").and_then(|t| t.as_str());
                        let slide_title = slide_val.get("title").and_then(|t| t.as_str());

                        let tag_str = verse_tag
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| format!("V{}", idx + 1));

                        let label = verse_tag
                            .map(|t| t.to_string())
                            .or_else(|| slide_title.map(|s| s.to_string()))
                            .unwrap_or_else(|| format!("Slide {}", idx + 1));

                        item.slides.push(Slide {
                            label: Some(label),
                            header: Some(tag_str.clone()),
                            text: clean_text,
                            notes: None,
                            background: None,
                            tag: Some(tag_str),
                            ..Default::default()
                        });
                    }
                }

                // A3: Check for header theme and resolve background
                if let Some(theme) = header.get("theme").and_then(|t| t.as_str()).filter(|t| !t.trim().is_empty()) {
                    let mut dirs = Self::find_local_openlp_dirs();
                    if let Some(md) = media_dir {
                        dirs.push(md.to_path_buf());
                        if let Some(p) = md.parent() {
                            dirs.push(p.to_path_buf());
                            dirs.push(p.join("themes"));
                        }
                    }
                    if let Some(bg) = Self::resolve_theme_background(theme, &dirs, media_dir) {
                        let legacy_bg = slide_background_to_legacy(&bg);
                        item.background = Some(legacy_bg.clone());
                        for slide in &mut item.slides {
                            slide.background = Some(legacy_bg.clone());
                            slide.background_v2 = Some(bg.clone());
                        }
                    }
                }

                if !item.slides.is_empty() {
                    schedule.items.push(item);
                }
            }
        }

        if !schedule.items.is_empty() {
            schedule.selected_item_index = Some(0);
        }

        Ok(schedule)
    }

    /// Discover standard OpenLP data directories across platforms (Linux, Windows, macOS)
    pub fn find_local_openlp_dirs() -> Vec<PathBuf> {
        let mut candidates = Vec::new();

        // 1. Linux/Unix XDG paths
        if let Ok(home) = std::env::var("HOME") {
            let h = PathBuf::from(&home);
            candidates.push(h.join(".local/share/openlp"));
            candidates.push(h.join(".local/share/openlp/data"));
            candidates.push(h.join("Library/Application Support/openlp/data")); // macOS
        }

        // 2. Windows APPDATA paths
        if let Ok(appdata) = std::env::var("APPDATA") {
            let a = PathBuf::from(&appdata);
            candidates.push(a.join("openlp/data"));
            candidates.push(a.join("OpenLP/data"));
            candidates.push(a.join("openlp"));
            candidates.push(a.join("OpenLP"));
        }

        // Return only existing directories
        candidates.into_iter().filter(|p| p.is_dir()).collect()
    }

    /// Automatically detect and import songs & Bibles from a local OpenLP installation
    pub fn detect_and_import_local(db: &crate::storage::db::Database, media_dir: Option<&Path>) -> Result<OpenLpImportSummary, Box<dyn std::error::Error>> {
        let dirs = Self::find_local_openlp_dirs();
        if dirs.is_empty() {
            return Ok(OpenLpImportSummary {
                success: false,
                songs_count: 0,
                bibles_count: 0,
                bible_versions: Vec::new(),
                message: "No OpenLP data directory found in standard local paths.".to_string(),
            });
        }

        let mut total_songs = 0;
        let mut total_bibles = 0;
        let mut versions = Vec::new();

        for base_dir in dirs {
            // A. Check songs database: songs/songs.sqlite, songs/songs.db, or songs.sqlite
            let song_candidates = [
                base_dir.join("songs/songs.sqlite"),
                base_dir.join("songs/songs.db"),
                base_dir.join("songs.sqlite"),
                base_dir.join("songs.db"),
            ];

            for song_path in song_candidates {
                if song_path.is_file() {
                    if let Ok(songs) = Self::import_songs_db_with_media(&song_path, media_dir) {
                        for song in &songs {
                            if let Ok(()) = db.insert_song(song) {
                                total_songs += 1;
                            }
                        }
                    }
                    break;
                }
            }

            // B. Check bibles directory: bibles/*.sqlite
            let bibles_dir = base_dir.join("bibles");
            if bibles_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(bibles_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|s| s.to_str()).map(|ext| ext == "sqlite" || ext == "db").unwrap_or(false) {
                            if let Ok(scriptures) = Self::import_bible_db(&path) {
                                if !scriptures.is_empty() {
                                    let ver = scriptures[0].version.clone();
                                    if let Ok(()) = db.insert_scriptures_batch(&scriptures) {
                                        total_bibles += scriptures.len();
                                        if !versions.contains(&ver) {
                                            versions.push(ver);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let message = if total_songs > 0 || total_bibles > 0 {
            format!("Successfully imported {} songs and {} Bible chapters from OpenLP!", total_songs, total_bibles)
        } else {
            "OpenLP directory found, but no songs or Bible databases were available to import.".to_string()
        };

        Ok(OpenLpImportSummary {
            success: total_songs > 0 || total_bibles > 0,
            songs_count: total_songs,
            bibles_count: total_bibles,
            bible_versions: versions,
            message,
        })
    }

    /// Resolve an OpenLP theme name to a SlideBackground by scanning candidate directories.
    pub fn resolve_theme_background(
        theme_name: &str,
        search_dirs: &[PathBuf],
        media_dir: Option<&Path>,
    ) -> Option<SlideBackground> {
        let clean = theme_name.trim();
        if clean.is_empty() {
            return None;
        }

        let variant_underscore = clean.replace(' ', "_");
        let variant_space = clean.replace('_', " ");
        let variants = [clean, &variant_underscore, &variant_space];

        for dir in search_dirs {
            for &variant in &variants {
                // 1. Direct JSON file candidates — the format a real, modern OpenLP
                // install actually writes (`<data dir>/themes/<Name>/<Name>.json`,
                // confirmed against real files; see `parse_openlp_theme_json`). Tried
                // before XML/OTZ since this is what's actually on disk today.
                let json_candidates = [
                    dir.join(format!("{}.json", variant)),
                    dir.join(variant).join(format!("{}.json", variant)),
                    dir.join("themes").join(format!("{}.json", variant)),
                    dir.join("themes").join(variant).join(format!("{}.json", variant)),
                ];

                for json_path in &json_candidates {
                    if json_path.is_file() {
                        if let Ok(content) = std::fs::read_to_string(json_path) {
                            if let Some(bg) = Self::parse_openlp_theme_json(&content, json_path.parent(), media_dir) {
                                return Some(bg);
                            }
                        }
                    }
                }

                // 2. Direct XML file candidates — older/exported theme format.
                let candidates = [
                    dir.join(format!("{}.xml", variant)),
                    dir.join(variant).join(format!("{}.xml", variant)),
                    dir.join("themes").join(format!("{}.xml", variant)),
                    dir.join("themes").join(variant).join(format!("{}.xml", variant)),
                ];

                for xml_path in &candidates {
                    if xml_path.is_file() {
                        if let Ok(content) = std::fs::read_to_string(xml_path) {
                            if let Some(bg) = Self::parse_openlp_theme_xml(&content, xml_path.parent(), media_dir) {
                                return Some(bg);
                            }
                        }
                    }
                }

                // 3. OTZ package candidates
                let otz_candidates = [
                    dir.join(format!("{}.otz", variant)),
                    dir.join("themes").join(format!("{}.otz", variant)),
                ];

                for otz_path in &otz_candidates {
                    if otz_path.is_file() {
                        if let Ok(file) = std::fs::File::open(otz_path) {
                            if let Some(bg) = Self::parse_openlp_otz(file, media_dir) {
                                return Some(bg);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Parse a real OpenLP theme **JSON** file — the format modern OpenLP installs
    /// actually write to `<data dir>/themes/<Theme Name>/<Theme Name>.json`. Confirmed
    /// directly against 5 real theme files on a local OpenLP install (Blue Burst,
    /// Clouds, Geo Purple, Moss on tree, Wheat) rather than assumed: a flat object
    /// always carrying every field regardless of `background_type`, e.g.
    /// `{"background_type": "image", "background_color": "#000000",
    /// "background_filename": {"parts": ["Moss on tree", "climbing-moss.jpeg"], ...},
    /// "background_start_color": ..., "background_end_color": ...,
    /// "background_direction": "vertical", ...}`. `parse_openlp_theme_xml` below
    /// targets a different, older export shape (`.otz` theme packages) — both are
    /// tried by `resolve_theme_background` since either can be present depending on
    /// OpenLP version or whether a theme was hand-exported; this JSON form is the one
    /// that actually exists on a live install today, so it's tried first.
    ///
    /// All 5 real samples checked happened to be `background_type: "image"`; `"solid"`
    /// and `"gradient"` are handled from the flat `background_color` /
    /// `background_start_color`/`background_end_color`/`background_direction` fields
    /// that are present (just unused) on every sample regardless of type, but those
    /// two branches are unverified against a real non-image sample — if either turns
    /// out wrong, it degrades to `None` rather than a wrong-but-confident guess (the
    /// image branch already only returns `Some` once the referenced file has actually
    /// been located and copied, never a fabricated path).
    pub fn parse_openlp_theme_json(
        json_content: &str,
        theme_dir: Option<&Path>,
        media_dir: Option<&Path>,
    ) -> Option<SlideBackground> {
        let v: serde_json::Value = serde_json::from_str(json_content).ok()?;
        let bg_type = v.get("background_type")?.as_str()?.to_lowercase();

        match bg_type.as_str() {
            "solid" => {
                let color = v.get("background_color").and_then(|c| c.as_str())?;
                Some(SlideBackground::Solid(color.to_string()))
            }
            "gradient" => {
                let start = v.get("background_start_color").and_then(|c| c.as_str())?.to_string();
                let end = v.get("background_end_color").and_then(|c| c.as_str()).unwrap_or("#333333").to_string();
                let direction = v.get("background_direction").and_then(|d| d.as_str()).unwrap_or("vertical").to_lowercase();
                let (kind, angle_deg) = if direction.contains("circular") {
                    ("radial".to_string(), None)
                } else if direction.contains("horizontal") {
                    ("linear".to_string(), Some(90.0))
                } else {
                    ("linear".to_string(), Some(180.0))
                };
                Some(SlideBackground::Gradient {
                    kind,
                    stops: vec![
                        GradientStop { color: start, offset: 0.0 },
                        GradientStop { color: end, offset: 1.0 },
                    ],
                    angle_deg,
                })
            }
            "image" => {
                // `parts` is the theme file's own directory-relative path segments —
                // confirmed on disk: parts == ["Moss on tree", "climbing-moss.jpeg"]
                // and the real file sits at "themes/Moss on tree/climbing-moss.jpeg",
                // i.e. directly inside the same directory the .json file itself is in.
                let filename = v.get("background_filename")?
                    .get("parts")?
                    .as_array()?
                    .last()?
                    .as_str()?
                    .trim();
                if filename.is_empty() {
                    return None;
                }

                let src_path = theme_dir.map(|d| d.join(filename)).filter(|p| p.is_file())?;
                let mdir = media_dir?;
                let new_filename = crate::storage::media_sniff::copy_sniffed_media(&src_path, mdir, "openlp")?;
                // Only ever return a background once the file has actually been found
                // and copied — a path we can't resolve is worse than no background.
                Some(SlideBackground::Image {
                    file_path: format!("/media/images/{}", new_filename),
                    opacity: 1.0,
                })
            }
            _ => None,
        }
    }

    /// Parse OpenLP theme XML content into a SlideBackground.
    ///
    /// Supports:
    /// - `<background type="solid"><color>#hex</color></background>`
    /// - `<background type="gradient"><start_color>#hex</start_color><end_color>#hex</end_color><direction>vertical|horizontal</direction></background>`
    /// - `<background type="image"><filename>file.ext</filename></background>`
    pub fn parse_openlp_theme_xml(
        xml_content: &str,
        theme_dir: Option<&Path>,
        media_dir: Option<&Path>,
    ) -> Option<SlideBackground> {
        let bg_start = xml_content.find("<background")?;
        let after_bg_start = &xml_content[bg_start..];
        let tag_close = after_bg_start.find('>')?;
        let bg_open_tag = &after_bg_start[..tag_close];

        let bg_end = after_bg_start[tag_close + 1..].find("</background>")? + tag_close + 1;
        let bg_body = &after_bg_start[tag_close + 1..bg_end];

        // Determine background type from attribute type="..." or child <type>...</type>
        let mut bg_type = String::new();
        if let Some(pos) = bg_open_tag.find("type=\"") {
            let after = &bg_open_tag[pos + 6..];
            if let Some(end) = after.find('"') {
                bg_type = after[..end].trim().to_lowercase();
            }
        } else if let Some(pos) = bg_open_tag.find("type='") {
            let after = &bg_open_tag[pos + 6..];
            if let Some(end) = after.find('\'') {
                bg_type = after[..end].trim().to_lowercase();
            }
        }
        if bg_type.is_empty() {
            if let Some(s) = bg_body.find("<type>") {
                let after = &bg_body[s + 6..];
                if let Some(e) = after.find("</type>") {
                    bg_type = after[..e].trim().to_lowercase();
                }
            }
        }

        match bg_type.as_str() {
            "solid" => {
                let color = extract_xml_tag_content(bg_body, "color")
                    .unwrap_or_else(|| "#000000".to_string());
                Some(SlideBackground::Solid(color))
            }
            "gradient" => {
                let start_color = extract_xml_tag_content(bg_body, "start_color")
                    .or_else(|| extract_xml_tag_content(bg_body, "startColor"))
                    .or_else(|| extract_xml_tag_content(bg_body, "color"))
                    .unwrap_or_else(|| "#000000".to_string());
                let end_color = extract_xml_tag_content(bg_body, "end_color")
                    .or_else(|| extract_xml_tag_content(bg_body, "endColor"))
                    .unwrap_or_else(|| "#333333".to_string());
                let direction = extract_xml_tag_content(bg_body, "direction")
                    .unwrap_or_else(|| "vertical".to_string())
                    .to_lowercase();
                let angle_deg = if direction.contains("horizontal") {
                    Some(90.0)
                } else if direction.contains("circular") {
                    None
                } else {
                    Some(180.0)
                };
                Some(SlideBackground::Gradient {
                    kind: if direction.contains("circular") {
                        "radial".to_string()
                    } else {
                        "linear".to_string()
                    },
                    stops: vec![
                        GradientStop {
                            color: start_color,
                            offset: 0.0,
                        },
                        GradientStop {
                            color: end_color,
                            offset: 1.0,
                        },
                    ],
                    angle_deg,
                })
            }
            "image" => {
                let filename = extract_xml_tag_content(bg_body, "filename")?;
                let trimmed_filename = filename.trim();
                if trimmed_filename.is_empty() {
                    return None;
                }

                // Attempt to resolve file on disk
                let resolved_path = if let Some(dir) = theme_dir {
                    let c1 = dir.join(trimmed_filename);
                    if c1.exists() {
                        Some(c1)
                    } else if let Some(parent) = dir.parent() {
                        let c2 = parent.join(trimmed_filename);
                        if c2.exists() {
                            Some(c2)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                // Only ever return a background once the referenced file has actually
                // been found AND copied to a servable location — a local filesystem
                // path (unservable by the web frontend) or a bare unresolved filename
                // (guaranteed not to exist anywhere) are both worse than no background.
                let src_path = resolved_path?;
                let mdir = media_dir?;
                let new_filename = crate::storage::media_sniff::copy_sniffed_media(&src_path, mdir, "openlp")?;
                Some(SlideBackground::Image {
                    file_path: format!("/media/images/{}", new_filename),
                    opacity: 1.0,
                })
            }
            _ => None,
        }
    }

    /// Parse an OpenLP `.otz` theme package (zip archive containing XML and media).
    pub fn parse_openlp_otz<R: std::io::Read + std::io::Seek>(
        reader: R,
        media_dir: Option<&Path>,
    ) -> Option<SlideBackground> {
        let mut archive = zip::ZipArchive::new(reader).ok()?;
        // 1. Locate and read the theme XML inside the zip
        let mut xml_content = String::new();
        for i in 0..archive.len() {
            if let Ok(mut f) = archive.by_index(i) {
                if f.name().to_lowercase().ends_with(".xml") {
                    xml_content = crate::storage::media_sniff::read_capped_string(&mut f, crate::storage::media_sniff::MAX_ZIP_TEXT_BYTES).unwrap_or_default();
                    break;
                }
            }
        }
        if xml_content.is_empty() {
            return None;
        }

        // Check if XML has solid or gradient
        if let Some(bg) = Self::parse_openlp_theme_xml(&xml_content, None, None) {
            match bg {
                SlideBackground::Solid(_) | SlideBackground::Gradient { .. } => return Some(bg),
                _ => {}
            }
        }

        // If it's an image background, extract filename and find it in zip
        let bg_body_start = xml_content.find("<background")?;
        let bg_body_end = xml_content.find("</background>")?;
        let bg_body = &xml_content[bg_body_start..bg_body_end];
        let img_name = extract_xml_tag_content(bg_body, "filename")?;
        let trimmed_name = img_name.trim();

        // Find file in archive matching trimmed_name. Only ever return a background
        // once its bytes have actually been extracted AND written to a servable
        // location — a bare filename with no directory (guaranteed unservable) is
        // worse than no background, same as the plain-XML "image" branch above.
        let mdir = media_dir?;
        for i in 0..archive.len() {
            if let Ok(mut f) = archive.by_index(i) {
                let name = f.name().to_string();
                if name.ends_with(trimmed_name) || name.rsplit('/').next() == Some(trimmed_name) {
                    if let Ok(img_bytes) = crate::storage::media_sniff::read_capped_bytes(&mut f, crate::storage::media_sniff::MAX_ZIP_MEDIA_BYTES) {
                        let new_filename = crate::storage::media_sniff::write_sniffed_media(mdir, "openlp", &img_bytes)?;
                        return Some(SlideBackground::Image {
                            file_path: format!("/media/images/{}", new_filename),
                            opacity: 1.0,
                        });
                    }
                }
            }
        }

        None
    }
}

/// Extract verse number from attribute string, e.g. label="2" or name="v2" -> Some(2)
fn extract_number_from_attr(header: &str) -> Option<u32> {
    for part in header.split_whitespace() {
        if part.starts_with("label=") || part.starts_with("name=") {
            let clean = part
                .replace("label=", "")
                .replace("name=", "")
                .replace('"', "")
                .replace('\'', "");
            let digits: String = clean.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u32>() {
                return Some(n);
            }
        }
    }
    None
}

/// Clean OpenLP Bible text (superscript verse markers `{su}2:1{/su}` and HTML tags)
fn clean_bible_verse_text(raw: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;

    for ch in raw.chars() {
        if ch == '{' || ch == '<' {
            in_tag = true;
        } else if ch == '}' || ch == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }

    out.replace("&nbsp;", " ").trim().to_string()
}

/// Clean raw slide text from OpenLP service JSON
fn clean_slide_text(raw: &str) -> String {
    let unescaped = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("<br>", "\n");

    clean_bible_verse_text(&unescaped)
}

/// Helper to convert a SlideBackground into a legacy background CSS string
pub fn slide_background_to_legacy(bg: &SlideBackground) -> String {
    match bg {
        SlideBackground::Solid(c) => c.clone(),
        SlideBackground::Image { file_path, .. } => file_path.clone(),
        SlideBackground::Gradient { stops, .. } => {
            if let (Some(s1), Some(s2)) = (stops.first(), stops.last()) {
                format!("linear-gradient({}, {})", s1.color, s2.color)
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Extract content between `<tag>` and `</tag>` in simple XML
pub fn extract_xml_tag_content(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)?;
    let after = &xml[start + open.len()..];
    let end = after.find(&close)?;
    Some(after[..end].trim().to_string())
}

