use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use rusqlite::{params, Connection, Result};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledBibleInfo {
    pub id: String,
    pub name: String,
    pub abbreviation: String,
    pub language: String,
    pub is_default: bool,
    pub is_licensed: bool,
    pub verse_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub paired_at: i64,
    pub last_seen: i64,
    pub token: String,
}

use crate::core::models::{MediaItem, Presentation, ScriptureItem, ScriptureVerse, Slide, Song, Theme};

/// Load slides from JSON and synthesize v2 elements for any legacy slides
pub fn load_and_synthesize_slides(slides_json: &str) -> Vec<Slide> {
    let mut slides: Vec<Slide> = serde_json::from_str(slides_json).unwrap_or_default();
    for slide in &mut slides {
        slide.synthesize_v2_if_needed();
    }
    slides
}

/// Serialize slides to JSON for storage, ensuring v2 elements and legacy text projection are up to date
pub fn serialize_slides_for_storage(slides: &[Slide]) -> String {
    let mut slides_to_save = slides.to_vec();
    for slide in &mut slides_to_save {
        slide.synthesize_v2_if_needed();
        slide.project_text_from_elements();
    }
    serde_json::to_string(&slides_to_save).unwrap_or_else(|_| "[]".to_string())
}


#[derive(Clone)]
pub struct Database {
    // Core presentation database (media, themes, presentations, settings, event_log)
    main_conn: Arc<Mutex<Connection>>,

    // Path configuration
    bibles_dir: Option<PathBuf>,
    songs_dir: Option<PathBuf>,

    // Songs databases: split between public domain and copyrighted works
    songs_pd_conn: Arc<Mutex<Connection>>,
    songs_cr_conn: Arc<Mutex<Connection>>,

    // Bibles databases: one SQLite file per Bible translation in bibles/
    bible_conns: Arc<Mutex<HashMap<String, Arc<Mutex<Connection>>>>>,

    // In-memory mode flag for fast, isolated testing
    is_in_memory: bool,
}

impl Database {
    fn main_conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.main_conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Backward-compatible accessor for callers expecting direct connection access
    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.main_conn()
    }

    pub fn bibles_dir(&self) -> Option<&Path> {
        self.bibles_dir.as_deref()
    }

    pub fn songs_dir(&self) -> Option<&Path> {
        self.songs_dir.as_deref()
    }

    pub fn new(db_path: impl AsRef<Path>) -> Result<Self> {
        let path = db_path.as_ref();
        let base_dir = path
            .parent()
            .map(|p| if p.as_os_str().is_empty() { Path::new(".") } else { p })
            .unwrap_or_else(|| Path::new("."));
        let bibles_dir = base_dir.join("bibles");
        let songs_dir = base_dir.join("songs");
        Self::new_with_dirs(path, bibles_dir, songs_dir)
    }

    pub fn new_with_dirs(
        db_path: impl AsRef<Path>,
        bibles_dir: impl AsRef<Path>,
        songs_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let b_dir = bibles_dir.as_ref().to_path_buf();
        let s_dir = songs_dir.as_ref().to_path_buf();

        // Ensure expected folders exist - create them if missing
        if !b_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&b_dir) {
                tracing::warn!("Failed to create bibles directory {:?}: {}", b_dir, e);
            } else {
                tracing::info!("Created missing bibles directory: {:?}", b_dir);
            }
        }
        if !s_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&s_dir) {
                tracing::warn!("Failed to create songs directory {:?}: {}", s_dir, e);
            } else {
                tracing::info!("Created missing songs directory: {:?}", s_dir);
            }
        }

        // Open main core database
        let main_conn = Connection::open(db_path.as_ref())?;
        main_conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;

        // Open split song databases
        let songs_pd_path = s_dir.join("public_domain.db");
        let songs_cr_path = s_dir.join("copyrighted.db");

        let songs_pd_conn = Connection::open(&songs_pd_path)?;
        songs_pd_conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        Self::init_songs_schema(&songs_pd_conn)?;

        let songs_cr_conn = Connection::open(&songs_cr_path)?;
        songs_cr_conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        Self::init_songs_schema(&songs_cr_conn)?;

        let db = Self {
            main_conn: Arc::new(Mutex::new(main_conn)),
            bibles_dir: Some(b_dir),
            songs_dir: Some(s_dir),
            songs_pd_conn: Arc::new(Mutex::new(songs_pd_conn)),
            songs_cr_conn: Arc::new(Mutex::new(songs_cr_conn)),
            bible_conns: Arc::new(Mutex::new(HashMap::new())),
            is_in_memory: false,
        };

        db.init_main_schema()?;
        db.migrate_legacy_data_if_needed()?;
        db.seed_default_themes_if_empty()?;
        db.seed_master_layouts_if_empty()?;

        Ok(db)
    }

    pub fn in_memory() -> Result<Self> {
        let main_conn = Connection::open_in_memory()?;
        main_conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;

        let songs_pd_conn = Connection::open_in_memory()?;
        Self::init_songs_schema(&songs_pd_conn)?;

        let songs_cr_conn = Connection::open_in_memory()?;
        Self::init_songs_schema(&songs_cr_conn)?;

        let db = Self {
            main_conn: Arc::new(Mutex::new(main_conn)),
            bibles_dir: None,
            songs_dir: None,
            songs_pd_conn: Arc::new(Mutex::new(songs_pd_conn)),
            songs_cr_conn: Arc::new(Mutex::new(songs_cr_conn)),
            bible_conns: Arc::new(Mutex::new(HashMap::new())),
            is_in_memory: true,
        };

        db.init_main_schema()?;
        Ok(db)
    }

    /// Some existing on-disk databases (predating the current flat-CSS `Theme` model)
    /// have a `themes` table from an older, richer per-field schema
    /// (`background_value`/`bold`/`italic`/`shadow`/`shadow_color`/`vertical_alignment`,
    /// no `background`/`text_shadow`/`line_height`/`letter_spacing`/`opacity` columns).
    /// `CREATE TABLE IF NOT EXISTS` never fixes this, so migrate it once: read whatever
    /// rows exist under the old shape, translate them into the current `Theme` model,
    /// rename the old table out of the way (kept, not dropped, in case translation
    /// missed something worth recovering by hand), and create a fresh current-schema
    /// table for `insert_theme`/`get_themes` to use going forward.
    fn migrate_legacy_themes_table_if_needed(&self, conn: &Connection) -> Result<()> {
        let mut has_background_value = false;
        let mut has_background = false;
        if let Ok(mut pragma) = conn.prepare("PRAGMA table_info(themes)") {
            let cols = pragma.query_map([], |r| r.get::<_, String>(1))?;
            for col in cols.flatten() {
                if col == "background_value" { has_background_value = true; }
                if col == "background" { has_background = true; }
            }
        }
        if !has_background_value || has_background {
            return Ok(());
        }

        tracing::info!("Migrating legacy 'themes' table (pre-flat-CSS schema) to the current Theme model...");

        let legacy_themes: Vec<Theme> = {
            let mut stmt = conn.prepare(
                "SELECT name, background_value, font_family, font_size, font_color, alignment,
                        shadow, shadow_color, margin_top, margin_bottom, margin_left, margin_right,
                        vertical_alignment
                 FROM themes"
            )?;
            let rows = stmt.query_map([], |row| {
                let font_size_raw: rusqlite::types::Value = row.get(3)?;
                let font_size = match font_size_raw {
                    rusqlite::types::Value::Integer(n) => format!("{}px", n),
                    rusqlite::types::Value::Text(s) => s,
                    _ => "48px".to_string(),
                };
                let parse_dim = |val: rusqlite::types::Value| -> String {
                    match val {
                        rusqlite::types::Value::Integer(n) => format!("{}px", n),
                        rusqlite::types::Value::Text(s) => s,
                        _ => "5%".to_string(),
                    }
                };
                let m_top: rusqlite::types::Value = row.get(8)?;
                let m_bottom: rusqlite::types::Value = row.get(9)?;
                let m_left: rusqlite::types::Value = row.get(10)?;
                let m_right: rusqlite::types::Value = row.get(11)?;
                let has_shadow: i64 = row.get(6).unwrap_or(0);
                let shadow_color: String = row.get(7).unwrap_or_else(|_| "#000000".to_string());
                let text_shadow = if has_shadow != 0 {
                    format!("2px 2px 8px {}", shadow_color)
                } else {
                    "none".to_string()
                };
                Ok(Theme {
                    id: None,
                    name: row.get(0)?,
                    background: row.get(1)?,
                    font_family: row.get(2)?,
                    font_size,
                    font_color: row.get(4)?,
                    alignment: row.get(5)?,
                    text_shadow,
                    margin_top: parse_dim(m_top),
                    margin_bottom: parse_dim(m_bottom),
                    margin_left: parse_dim(m_left),
                    margin_right: parse_dim(m_right),
                    vertical_align: row.get::<_, Option<String>>(12)?.unwrap_or_else(|| "center".to_string()),
                    ..Default::default()
                })
            })?;
            let mut out = Vec::new();
            for t in rows {
                out.push(t?);
            }
            out
        };

        conn.execute("ALTER TABLE themes RENAME TO themes_legacy_backup", [])?;
        conn.execute_batch(
            "CREATE TABLE themes (
                name TEXT PRIMARY KEY,
                background TEXT NOT NULL,
                font_family TEXT NOT NULL,
                font_size TEXT NOT NULL,
                font_color TEXT NOT NULL,
                alignment TEXT NOT NULL,
                text_shadow TEXT NOT NULL,
                line_height TEXT NOT NULL,
                letter_spacing TEXT NOT NULL,
                opacity TEXT NOT NULL,
                margin_top TEXT NOT NULL,
                margin_bottom TEXT NOT NULL,
                margin_left TEXT NOT NULL,
                margin_right TEXT NOT NULL,
                vertical_align TEXT NOT NULL,
                category TEXT NOT NULL DEFAULT 'song',
                is_default INTEGER NOT NULL DEFAULT 0,
                reference_position TEXT NOT NULL DEFAULT 'inline',
                chroma_key_enabled INTEGER NOT NULL DEFAULT 0,
                chroma_key_color TEXT NOT NULL DEFAULT '#00ff00',
                safe_area_percent INTEGER NOT NULL DEFAULT 0
            );",
        )?;
        for theme in &legacy_themes {
            // Insert directly against the already-held `conn` guard rather than via
            // `self.insert_theme` (which would re-lock the same non-reentrant mutex
            // this function's caller is already holding, and deadlock).
            conn.execute(
                "INSERT OR REPLACE INTO themes (
                    name, background, font_family, font_size, font_color, alignment,
                    text_shadow, line_height, letter_spacing, opacity,
                    margin_top, margin_bottom, margin_left, margin_right, vertical_align,
                    category, is_default, reference_position, chroma_key_enabled,
                    chroma_key_color, safe_area_percent
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
                params![
                    theme.name, theme.background, theme.font_family, theme.font_size, theme.font_color, theme.alignment,
                    theme.text_shadow, theme.line_height, theme.letter_spacing, theme.opacity,
                    theme.margin_top, theme.margin_bottom, theme.margin_left, theme.margin_right, theme.vertical_align,
                    theme.category, theme.is_default as i64, theme.reference_position, theme.chroma_key_enabled as i64,
                    theme.chroma_key_color, theme.safe_area_percent as i64
                ],
            )?;
        }
        tracing::info!("Migrated {} theme(s) from the legacy schema; original table kept as 'themes_legacy_backup'.", legacy_themes.len());

        Ok(())
    }

    fn init_main_schema(&self) -> Result<()> {
        let conn = self.main_conn();
        self.migrate_legacy_themes_table_if_needed(&conn)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS media (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                media_type TEXT NOT NULL,
                file_path TEXT NOT NULL,
                duration_seconds REAL,
                thumbnail_path TEXT,
                loop_playback INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS themes (
                name TEXT PRIMARY KEY,
                background TEXT NOT NULL,
                font_family TEXT NOT NULL,
                font_size TEXT NOT NULL,
                font_color TEXT NOT NULL,
                alignment TEXT NOT NULL,
                text_shadow TEXT NOT NULL,
                line_height TEXT NOT NULL,
                letter_spacing TEXT NOT NULL,
                opacity TEXT NOT NULL,
                margin_top TEXT NOT NULL,
                margin_bottom TEXT NOT NULL,
                margin_left TEXT NOT NULL,
                margin_right TEXT NOT NULL,
                vertical_align TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS presentations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                author TEXT NOT NULL,
                slides_json TEXT NOT NULL,
                theme_name TEXT
            );

            CREATE TABLE IF NOT EXISTS slide_templates (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                category TEXT,
                elements_json TEXT NOT NULL,
                background_json TEXT,
                thumbnail_data_url TEXT
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_media_name ON media(name);
            CREATE INDEX IF NOT EXISTS idx_presentations_title ON presentations(title);

            CREATE TABLE IF NOT EXISTS event_log (
                sequence_number INTEGER PRIMARY KEY,
                event_type TEXT NOT NULL,
                payload JSON NOT NULL,
                timestamp_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS keyring_metadata (
                service TEXT NOT NULL,
                account TEXT NOT NULL,
                owner TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (service, account)
            );

            CREATE TABLE IF NOT EXISTS paired_devices (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                platform TEXT NOT NULL,
                paired_at INTEGER NOT NULL,
                last_seen INTEGER NOT NULL,
                token TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_paired_devices_token ON paired_devices(token);",
        )?;

        // Ensure loop_playback exists on older databases
        let _ = conn.execute("ALTER TABLE media ADD COLUMN loop_playback INTEGER DEFAULT 0", []);

        // Ensure the theme-category/default/lower-third columns exist on older databases
        let _ = conn.execute("ALTER TABLE themes ADD COLUMN category TEXT NOT NULL DEFAULT 'song'", []);
        let _ = conn.execute("ALTER TABLE themes ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE themes ADD COLUMN reference_position TEXT NOT NULL DEFAULT 'inline'", []);
        let _ = conn.execute("ALTER TABLE themes ADD COLUMN chroma_key_enabled INTEGER NOT NULL DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE themes ADD COLUMN chroma_key_color TEXT NOT NULL DEFAULT '#00ff00'", []);
        let _ = conn.execute("ALTER TABLE themes ADD COLUMN safe_area_percent INTEGER NOT NULL DEFAULT 0", []);

        Ok(())
    }

    fn init_songs_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS songs (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                author TEXT NOT NULL,
                copyright TEXT,
                ccli_number TEXT,
                slides_json TEXT NOT NULL,
                theme_name TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_songs_title ON songs(title);
            CREATE INDEX IF NOT EXISTS idx_songs_author ON songs(author);",
        )
    }

    fn init_scriptures_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS scriptures (
                id TEXT PRIMARY KEY,
                book TEXT NOT NULL,
                chapter INTEGER NOT NULL,
                verse_start INTEGER NOT NULL,
                verse_end INTEGER NOT NULL,
                version TEXT NOT NULL,
                reference TEXT NOT NULL,
                verses_json TEXT NOT NULL,
                theme_name TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_scriptures_reference ON scriptures(reference);
            CREATE INDEX IF NOT EXISTS idx_scriptures_book ON scriptures(book);",
        )
    }

    /// Automatically migrates existing songs and scriptures out of a single monolithic library.db
    /// into the split songs/ and bibles/ folders. Drops the legacy tables once migrated.
    fn migrate_legacy_data_if_needed(&self) -> Result<()> {
        let main_conn = self.main_conn();

        // 1. Check legacy songs table in main_conn
        let has_songs_table: bool = main_conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='songs')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);

        if has_songs_table {
            let song_count: usize = main_conn
                .query_row("SELECT COUNT(*) FROM songs", [], |r| r.get(0))
                .unwrap_or(0);

            if song_count > 0 {
                tracing::info!(
                    "Migrating {} existing songs from legacy monolithic database to songs/ folder...",
                    song_count
                );
                let mut stmt = main_conn.prepare(
                    "SELECT id, title, author, copyright, ccli_number, slides_json, theme_name FROM songs",
                )?;
                let songs: Vec<Song> = stmt
                    .query_map([], |row| {
                        let slides_json: String = row.get(5)?;
                        let slides = load_and_synthesize_slides(&slides_json);
                        Ok(Song {
                            id: row.get(0)?,
                            title: row.get(1)?,
                            author: row.get(2)?,
                            copyright: row.get(3)?,
                            ccli_number: row.get(4)?,
                            slides,
                            theme_name: row.get(6)?,
                            alternate_title: None,
                        })
                    })?
                    .filter_map(|s| s.ok())
                    .collect();
                drop(stmt);

                for song in &songs {
                    let is_pd = Self::is_public_domain_song(song);
                    let target_arc = if is_pd {
                        &self.songs_pd_conn
                    } else {
                        &self.songs_cr_conn
                    };
                    let slides_json = serialize_slides_for_storage(&song.slides);
                    let conn = target_arc.lock().unwrap_or_else(|e| e.into_inner());
                    let _ = conn.execute(
                        "INSERT OR REPLACE INTO songs (id, title, author, copyright, ccli_number, slides_json, theme_name)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            song.id,
                            song.title,
                            song.author,
                            song.copyright,
                            song.ccli_number,
                            slides_json,
                            song.theme_name
                        ],
                    );
                }

                let _ = main_conn.execute("DROP TABLE IF EXISTS songs", []);
                tracing::info!(
                    "Migration of {} songs into songs/ complete. Legacy table dropped.",
                    songs.len()
                );
            }
        }

        // 2. Check legacy scriptures table in main_conn
        let has_scriptures_table: bool = main_conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='scriptures')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);

        if has_scriptures_table {
            let scrip_count: usize = main_conn
                .query_row("SELECT COUNT(*) FROM scriptures", [], |r| r.get(0))
                .unwrap_or(0);

            if scrip_count > 0 {
                tracing::info!(
                    "Migrating {} existing scriptures from legacy monolithic database to bibles/ folder...",
                    scrip_count
                );
                let mut stmt = main_conn.prepare(
                    "SELECT id, book, chapter, verse_start, verse_end, version, reference, verses_json, theme_name FROM scriptures",
                )?;
                let items: Vec<ScriptureItem> = stmt
                    .query_map([], |row| {
                        let verses_json: String = row.get(7)?;
                        let verses: Vec<ScriptureVerse> = serde_json::from_str(&verses_json).unwrap_or_default();
                        Ok(ScriptureItem {
                            id: row.get(0)?,
                            book: row.get(1)?,
                            chapter: row.get(2)?,
                            verse_start: row.get(3)?,
                            verse_end: row.get(4)?,
                            version: row.get(5)?,
                            reference: row.get(6)?,
                            verses,
                            theme_name: row.get(8)?,
                        })
                    })?
                    .filter_map(|s| s.ok())
                    .collect();
                drop(stmt);

                let mut by_ver: HashMap<String, Vec<ScriptureItem>> = HashMap::new();
                for it in items {
                    by_ver.entry(it.version.clone()).or_default().push(it);
                }

                for (ver, ver_items) in by_ver {
                    if let Ok(conn_arc) = self.get_or_create_bible_conn(&ver) {
                        let mut conn = conn_arc.lock().unwrap_or_else(|e| e.into_inner());
                        if let Ok(tx) = conn.transaction() {
                            {
                                let insert_stmt = tx
                                    .prepare(
                                        "INSERT OR REPLACE INTO scriptures (id, book, chapter, verse_start, verse_end, version, reference, verses_json, theme_name)
                                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                                    )
                                    .ok();
                                if let Some(mut s) = insert_stmt {
                                    for it in &ver_items {
                                        let verses_json = serde_json::to_string(&it.verses)
                                            .unwrap_or_else(|_| "[]".to_string());
                                        let _ = s.execute(params![
                                            it.id,
                                            it.book,
                                            it.chapter,
                                            it.verse_start,
                                            it.verse_end,
                                            it.version,
                                            it.reference,
                                            verses_json,
                                            it.theme_name
                                        ]);
                                    }
                                }
                            }
                            let _ = tx.commit();
                        }
                        tracing::info!(
                            "Migrated {} scriptures to bibles/{}.db",
                            ver_items.len(),
                            Self::sanitize_bible_filename(&ver)
                        );
                    }
                }

                let _ = main_conn.execute("DROP TABLE IF EXISTS scriptures", []);
                tracing::info!("Migration of scriptures into bibles/ complete. Legacy table dropped.");
            }
        }

        // 3. Migrate legacy app_metadata to settings if present
        let has_app_metadata: bool = main_conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='app_metadata')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);

        if has_app_metadata {
            let _ = main_conn.execute(
                "INSERT OR IGNORE INTO settings (key, value) SELECT key, value FROM app_metadata",
                [],
            );
            let _ = main_conn.execute("DROP TABLE IF EXISTS app_metadata", []);
            tracing::info!("Migrated legacy app_metadata into settings table and dropped app_metadata.");
        }

        Ok(())
    }

    /// Sanitizes Bible version names for safe cross-platform file naming
    pub fn sanitize_bible_filename(version: &str) -> String {
        let safe: String = version
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let trimmed = safe.trim_matches('_');
        if trimmed.is_empty() {
            "bible".to_string()
        } else {
            trimmed.to_string()
        }
    }

    /// Retrieves an existing cached connection or opens/creates a per-Bible SQLite file
    fn get_or_create_bible_conn(&self, version: &str) -> Result<Arc<Mutex<Connection>>> {
        let canon = version.trim().to_uppercase();
        let key = if canon.is_empty() {
            "DEFAULT".to_string()
        } else {
            canon
        };

        let mut conns = self.bible_conns.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(conn) = conns.get(&key) {
            return Ok(conn.clone());
        }

        if self.is_in_memory {
            let conn = Connection::open_in_memory()?;
            conn.execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA busy_timeout = 5000;",
            )?;
            Self::init_scriptures_schema(&conn)?;
            let conn_arc = Arc::new(Mutex::new(conn));
            conns.insert(key, conn_arc.clone());
            Ok(conn_arc)
        } else {
            let b_dir = self
                .bibles_dir
                .as_ref()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("bibles"));
            if !b_dir.exists() {
                let _ = std::fs::create_dir_all(&b_dir);
            }
            let safe_name = Self::sanitize_bible_filename(&key);
            let file_path = b_dir.join(format!("{}.db", safe_name));
            let conn = Connection::open(&file_path)?;
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA busy_timeout = 5000;",
            )?;
            Self::init_scriptures_schema(&conn)?;
            let conn_arc = Arc::new(Mutex::new(conn));
            conns.insert(key, conn_arc.clone());
            Ok(conn_arc)
        }
    }

    /// Discovers and returns connections to all Bible databases in bibles/
    fn get_all_bible_conns(&self) -> Result<Vec<Arc<Mutex<Connection>>>> {
        if self.is_in_memory {
            let conns = self.bible_conns.lock().unwrap_or_else(|e| e.into_inner());
            Ok(conns.values().cloned().collect())
        } else {
            let b_dir = self
                .bibles_dir
                .as_ref()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("bibles"));
            if !b_dir.exists() {
                let _ = std::fs::create_dir_all(&b_dir);
                return Ok(Vec::new());
            }

            // Discover any *.db or *.sqlite files in bibles/
            let entries = match std::fs::read_dir(&b_dir) {
                Ok(rd) => rd,
                Err(_) => return Ok(Vec::new()),
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext == "db" || ext == "sqlite" {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            let key = stem.trim().to_uppercase();
                            let mut conns = self.bible_conns.lock().unwrap_or_else(|e| e.into_inner());
                            if let std::collections::hash_map::Entry::Vacant(e) = conns.entry(key) {
                                if let Ok(conn) = Connection::open(&path) {
                                    let _ = conn.execute_batch(
                                        "PRAGMA journal_mode = WAL;
                                         PRAGMA synchronous = NORMAL;
                                         PRAGMA busy_timeout = 5000;",
                                    );
                                    let _ = Self::init_scriptures_schema(&conn);
                                    e.insert(Arc::new(Mutex::new(conn)));
                                }
                            }
                        }
                    }
                }
            }

            let conns = self.bible_conns.lock().unwrap_or_else(|e| e.into_inner());
            Ok(conns.values().cloned().collect())
        }
    }

    // --- Song CRUD & Copyright Segmentation ---

    /// Categorizes songs into public domain vs. copyrighted works
    pub fn is_public_domain_song(song: &Song) -> bool {
        let copyright = song.copyright.as_deref().unwrap_or("").trim().to_lowercase();
        let ccli = song.ccli_number.as_deref().unwrap_or("").trim();

        if copyright.contains("public domain") || copyright == "pd" {
            return true;
        }
        // Traditional hymns or songs with no copyright/CCLI metadata default to public domain
        if copyright.is_empty() && ccli.is_empty() {
            return true;
        }
        false
    }

    pub fn get_songs(&self) -> Result<Vec<Song>> {
        let mut all_songs = Vec::new();

        let read_songs = |conn_arc: &Arc<Mutex<Connection>>| -> Result<Vec<Song>> {
            let conn = conn_arc.lock().unwrap_or_else(|e| e.into_inner());
            let mut stmt = conn.prepare_cached(
                "SELECT id, title, author, copyright, ccli_number, slides_json, theme_name FROM songs ORDER BY title ASC",
            )?;
            let song_iter = stmt.query_map([], |row| {
                let slides_json: String = row.get(5)?;
                let slides = load_and_synthesize_slides(&slides_json);
                Ok(Song {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    author: row.get(2)?,
                    copyright: row.get(3)?,
                    ccli_number: row.get(4)?,
                    slides,
                    theme_name: row.get(6)?,
                    alternate_title: None,
                })
            })?;
            let mut list = Vec::new();
            for s in song_iter {
                list.push(s?);
            }
            Ok(list)
        };

        if let Ok(pd_songs) = read_songs(&self.songs_pd_conn) {
            all_songs.extend(pd_songs);
        }
        if let Ok(cr_songs) = read_songs(&self.songs_cr_conn) {
            all_songs.extend(cr_songs);
        }

        all_songs.sort_by_key(|a| a.title.to_lowercase());
        all_songs.dedup_by(|a, b| a.id == b.id);
        Ok(all_songs)
    }

    pub fn get_song_by_id(&self, id: &str) -> Result<Option<Song>> {
        // 1. Check public domain DB
        {
            let conn = self.songs_pd_conn.lock().unwrap_or_else(|e| e.into_inner());
            let mut stmt = conn.prepare_cached(
                "SELECT id, title, author, copyright, ccli_number, slides_json, theme_name FROM songs WHERE id = ?1",
            )?;
            let mut rows = stmt.query(params![id])?;
            if let Some(row) = rows.next()? {
                let slides_json: String = row.get(5)?;
                let slides = load_and_synthesize_slides(&slides_json);
                return Ok(Some(Song {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    author: row.get(2)?,
                    copyright: row.get(3)?,
                    ccli_number: row.get(4)?,
                    slides,
                    theme_name: row.get(6)?,
                    alternate_title: None,
                }));
            }
        }

        // 2. Check copyrighted DB
        {
            let conn = self.songs_cr_conn.lock().unwrap_or_else(|e| e.into_inner());
            let mut stmt = conn.prepare_cached(
                "SELECT id, title, author, copyright, ccli_number, slides_json, theme_name FROM songs WHERE id = ?1",
            )?;
            let mut rows = stmt.query(params![id])?;
            if let Some(row) = rows.next()? {
                let slides_json: String = row.get(5)?;
                let slides = load_and_synthesize_slides(&slides_json);
                return Ok(Some(Song {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    author: row.get(2)?,
                    copyright: row.get(3)?,
                    ccli_number: row.get(4)?,
                    slides,
                    theme_name: row.get(6)?,
                    alternate_title: None,
                }));
            }
        }

        Ok(None)
    }

    pub fn insert_song(&self, song: &Song) -> Result<()> {
        let is_pd = Self::is_public_domain_song(song);
        let (target_arc, other_arc) = if is_pd {
            (&self.songs_pd_conn, &self.songs_cr_conn)
        } else {
            (&self.songs_cr_conn, &self.songs_pd_conn)
        };

        let slides_json = serialize_slides_for_storage(&song.slides);
        {
            let conn = target_arc.lock().unwrap_or_else(|e| e.into_inner());
            conn.execute(
                "INSERT OR REPLACE INTO songs (id, title, author, copyright, ccli_number, slides_json, theme_name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    song.id,
                    song.title,
                    song.author,
                    song.copyright,
                    song.ccli_number,
                    slides_json,
                    song.theme_name
                ],
            )?;
        }

        // Ensure clean separation by deleting from other DB if previously stored there
        {
            let other = other_arc.lock().unwrap_or_else(|e| e.into_inner());
            let _ = other.execute("DELETE FROM songs WHERE id = ?1", params![song.id]);
        }

        Ok(())
    }

    pub fn delete_song(&self, id: &str) -> Result<()> {
        {
            let conn = self.songs_pd_conn.lock().unwrap_or_else(|e| e.into_inner());
            let _ = conn.execute("DELETE FROM songs WHERE id = ?1", params![id]);
        }
        {
            let conn = self.songs_cr_conn.lock().unwrap_or_else(|e| e.into_inner());
            let _ = conn.execute("DELETE FROM songs WHERE id = ?1", params![id]);
        }
        Ok(())
    }

    pub fn search_songs(&self, query: &str) -> Result<Vec<Song>> {
        let q = format!("%{}%", query);
        let mut results = Vec::new();

        let search_conn = |conn_arc: &Arc<Mutex<Connection>>| -> Result<Vec<Song>> {
            let conn = conn_arc.lock().unwrap_or_else(|e| e.into_inner());
            let mut stmt = conn.prepare_cached(
                "SELECT id, title, author, copyright, ccli_number, slides_json, theme_name 
                 FROM songs 
                 WHERE title LIKE ?1 OR author LIKE ?1 OR slides_json LIKE ?1
                 ORDER BY title ASC LIMIT 50",
            )?;
            let song_iter = stmt.query_map(params![q], |row| {
                let slides_json: String = row.get(5)?;
                let slides = load_and_synthesize_slides(&slides_json);
                Ok(Song {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    author: row.get(2)?,
                    copyright: row.get(3)?,
                    ccli_number: row.get(4)?,
                    slides,
                    theme_name: row.get(6)?,
                    alternate_title: None,
                })
            })?;
            let mut list = Vec::new();
            for s in song_iter {
                list.push(s?);
            }
            Ok(list)
        };

        if let Ok(pd) = search_conn(&self.songs_pd_conn) {
            results.extend(pd);
        }
        if let Ok(cr) = search_conn(&self.songs_cr_conn) {
            results.extend(cr);
        }

        results.sort_by_key(|a| a.title.to_lowercase());
        results.dedup_by(|a, b| a.id == b.id);
        results.truncate(50);
        Ok(results)
    }

    // --- Scripture & Per-Bible Operations ---

    pub fn get_scriptures(&self) -> Result<Vec<ScriptureItem>> {
        let b_conns = self.get_all_bible_conns()?;
        let mut all_items = Vec::new();

        for conn_arc in b_conns {
            let conn = conn_arc.lock().unwrap_or_else(|e| e.into_inner());
            let mut stmt = match conn.prepare_cached(
                "SELECT id, book, chapter, verse_start, verse_end, version, reference, verses_json, theme_name 
                 FROM scriptures ORDER BY reference ASC",
            ) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let scrip_iter = stmt.query_map([], |row| {
                let verses_json: String = row.get(7)?;
                let verses: Vec<ScriptureVerse> = serde_json::from_str(&verses_json).unwrap_or_default();
                Ok(ScriptureItem {
                    id: row.get(0)?,
                    book: row.get(1)?,
                    chapter: row.get(2)?,
                    verse_start: row.get(3)?,
                    verse_end: row.get(4)?,
                    version: row.get(5)?,
                    reference: row.get(6)?,
                    verses,
                    theme_name: row.get(8)?,
                })
            });

            if let Ok(iter) = scrip_iter {
                for item in iter.flatten() {
                    all_items.push(item);
                }
            }
        }

        all_items.sort_by(|a, b| a.reference.cmp(&b.reference));
        Ok(all_items)
    }

    pub fn get_scripture_by_id(&self, id: &str) -> Result<Option<ScriptureItem>> {
        let b_conns = self.get_all_bible_conns()?;
        for conn_arc in b_conns {
            let conn = conn_arc.lock().unwrap_or_else(|e| e.into_inner());
            let mut stmt = match conn.prepare_cached(
                "SELECT id, book, chapter, verse_start, verse_end, version, reference, verses_json, theme_name 
                 FROM scriptures WHERE id = ?1",
            ) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let mut rows = stmt.query(params![id])?;
            if let Some(row) = rows.next()? {
                let verses_json: String = row.get(7)?;
                let verses: Vec<ScriptureVerse> = serde_json::from_str(&verses_json).unwrap_or_default();
                return Ok(Some(ScriptureItem {
                    id: row.get(0)?,
                    book: row.get(1)?,
                    chapter: row.get(2)?,
                    verse_start: row.get(3)?,
                    verse_end: row.get(4)?,
                    version: row.get(5)?,
                    reference: row.get(6)?,
                    verses,
                    theme_name: row.get(8)?,
                }));
            }
        }
        Ok(None)
    }

    pub fn insert_scripture(&self, scrip: &ScriptureItem) -> Result<()> {
        let conn_arc = self.get_or_create_bible_conn(&scrip.version)?;
        let conn = conn_arc.lock().unwrap_or_else(|e| e.into_inner());
        let verses_json = serde_json::to_string(&scrip.verses).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT OR REPLACE INTO scriptures (id, book, chapter, verse_start, verse_end, version, reference, verses_json, theme_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                scrip.id,
                scrip.book,
                scrip.chapter,
                scrip.verse_start,
                scrip.verse_end,
                scrip.version,
                scrip.reference,
                verses_json,
                scrip.theme_name
            ],
        )?;
        Ok(())
    }

    pub fn insert_scriptures_batch(&self, items: &[ScriptureItem]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }

        // Group by version and batch insert into each Bible's database
        let mut by_version: HashMap<String, Vec<&ScriptureItem>> = HashMap::new();
        for item in items {
            by_version.entry(item.version.clone()).or_default().push(item);
        }

        for (version, v_items) in by_version {
            let conn_arc = self.get_or_create_bible_conn(&version)?;
            let mut conn = conn_arc.lock().unwrap_or_else(|e| e.into_inner());
            let tx = conn.transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT OR REPLACE INTO scriptures (id, book, chapter, verse_start, verse_end, version, reference, verses_json, theme_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )?;
                for scrip in v_items {
                    let verses_json = serde_json::to_string(&scrip.verses).unwrap_or_else(|_| "[]".to_string());
                    stmt.execute(params![
                        scrip.id,
                        scrip.book,
                        scrip.chapter,
                        scrip.verse_start,
                        scrip.verse_end,
                        scrip.version,
                        scrip.reference,
                        verses_json,
                        scrip.theme_name
                    ])?;
                }
            }
            tx.commit()?;
        }

        Ok(())
    }

    pub fn delete_scripture(&self, id: &str) -> Result<()> {
        let b_conns = self.get_all_bible_conns()?;
        for conn_arc in b_conns {
            let conn = conn_arc.lock().unwrap_or_else(|e| e.into_inner());
            let _ = conn.execute("DELETE FROM scriptures WHERE id = ?1", params![id]);
        }
        Ok(())
    }

    pub fn search_scriptures(&self, query: &str) -> Result<Vec<ScriptureItem>> {
        let b_conns = self.get_all_bible_conns()?;
        let q = format!("%{}%", query);
        let mut results = Vec::new();

        for conn_arc in b_conns {
            let conn = conn_arc.lock().unwrap_or_else(|e| e.into_inner());
            let mut stmt = match conn.prepare_cached(
                "SELECT id, book, chapter, verse_start, verse_end, version, reference, verses_json, theme_name 
                 FROM scriptures 
                 WHERE reference LIKE ?1 OR verses_json LIKE ?1
                 ORDER BY reference ASC LIMIT 50",
            ) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let scrip_iter = stmt.query_map(params![q], |row| {
                let verses_json: String = row.get(7)?;
                let verses: Vec<ScriptureVerse> = serde_json::from_str(&verses_json).unwrap_or_default();
                Ok(ScriptureItem {
                    id: row.get(0)?,
                    book: row.get(1)?,
                    chapter: row.get(2)?,
                    verse_start: row.get(3)?,
                    verse_end: row.get(4)?,
                    version: row.get(5)?,
                    reference: row.get(6)?,
                    verses,
                    theme_name: row.get(8)?,
                })
            });

            if let Ok(iter) = scrip_iter {
                for item in iter.flatten() {
                    results.push(item);
                }
            }
        }

        results.sort_by(|a, b| a.reference.cmp(&b.reference));
        results.truncate(50);
        Ok(results)
    }

    /// Queries a passage (chapter and verses) for a specific Bible translation.
    /// Matches book name case-insensitively and selects the chapter row with the widest verse coverage.
    pub fn get_passage(&self, version: &str, book: &str, chapter: u32) -> Result<Option<ScriptureItem>> {
        let conn_arc = self.get_or_create_bible_conn(version)?;
        let conn = conn_arc.lock().unwrap_or_else(|e| e.into_inner());

        let mut stmt = conn.prepare_cached(
            "SELECT id, book, chapter, verse_start, verse_end, version, reference, verses_json, theme_name 
             FROM scriptures 
             WHERE LOWER(book) = LOWER(?1) AND chapter = ?2
             ORDER BY (verse_end - verse_start) DESC LIMIT 1;",
        )?;

        let mut rows = stmt.query(params![book, chapter])?;
        if let Some(row) = rows.next()? {
            let verses_json: String = row.get(7)?;
            let verses: Vec<ScriptureVerse> = serde_json::from_str(&verses_json).unwrap_or_default();
            Ok(Some(ScriptureItem {
                id: row.get(0)?,
                book: row.get(1)?,
                chapter: row.get(2)?,
                verse_start: row.get(3)?,
                verse_end: row.get(4)?,
                version: row.get(5)?,
                reference: row.get(6)?,
                verses,
                theme_name: row.get(8)?,
            }))
        } else {
            Ok(None)
        }
    }

    // --- Media CRUD ---
    pub fn get_media_by_id(&self, id: &str) -> Result<Option<MediaItem>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT id, name, media_type, file_path, duration_seconds, thumbnail_path FROM media WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(MediaItem {
                id: row.get(0)?,
                name: row.get(1)?,
                media_type: row.get(2)?,
                file_path: row.get(3)?,
                duration_seconds: row.get(4)?,
                thumbnail_path: row.get(5)?,
                loop_playback: false,
            }))
        } else {
            Ok(None)
        }
    }

    /// Some `media` tables predate `loop_playback` and lack the column entirely;
    /// others (this one included) have it as `NOT NULL` with no default. Detect
    /// which shape we're dealing with rather than assuming either — the same
    /// approach `get_themes` uses below for its own schema drift.
    fn media_has_loop_playback_column(conn: &Connection) -> bool {
        if let Ok(mut pragma) = conn.prepare_cached("PRAGMA table_info(media);") {
            if let Ok(rows) = pragma.query_map([], |r| r.get::<_, String>(1)) {
                return rows.flatten().any(|col| col == "loop_playback");
            }
        }
        false
    }

    pub fn get_media(&self) -> Result<Vec<MediaItem>> {
        let conn = self.conn();
        let has_loop = Self::media_has_loop_playback_column(&conn);
        let sql = if has_loop {
            "SELECT id, name, media_type, file_path, duration_seconds, thumbnail_path, loop_playback FROM media ORDER BY name ASC"
        } else {
            "SELECT id, name, media_type, file_path, duration_seconds, thumbnail_path FROM media ORDER BY name ASC"
        };
        let mut stmt = conn.prepare_cached(sql)?;
        let media_iter = stmt.query_map([], |row| {
            Ok(MediaItem {
                id: row.get(0)?,
                name: row.get(1)?,
                media_type: row.get(2)?,
                file_path: row.get(3)?,
                duration_seconds: row.get(4)?,
                thumbnail_path: row.get(5)?,
                loop_playback: if has_loop { row.get(6)? } else { false },
            })
        })?;

        let mut items = Vec::new();
        for m in media_iter {
            items.push(m?);
        }
        Ok(items)
    }

    pub fn insert_media(&self, media: &MediaItem) -> Result<()> {
        let conn = self.conn();
        if Self::media_has_loop_playback_column(&conn) {
            conn.execute(
                "INSERT OR REPLACE INTO media (id, name, media_type, file_path, duration_seconds, thumbnail_path, loop_playback)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![media.id, media.name, media.media_type, media.file_path, media.duration_seconds, media.thumbnail_path, media.loop_playback],
            )?;
        } else {
            conn.execute(
                "INSERT OR REPLACE INTO media (id, name, media_type, file_path, duration_seconds, thumbnail_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![media.id, media.name, media.media_type, media.file_path, media.duration_seconds, media.thumbnail_path],
            )?;
        }

        Ok(())
    }

    pub fn delete_media(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM media WHERE id = ?1", params![id])?;
        
        Ok(())
    }

    /// Automatically discovers and registers image files from a local media directory
    /// into the SQLite `media` table if not already indexed.
    pub fn sync_media_folder(&self, media_images_dir: &Path) -> Result<usize> {
        if !media_images_dir.exists() || !media_images_dir.is_dir() {
            return Ok(0);
        }

        let existing_items = self.get_media()?;
        let existing_paths: std::collections::HashSet<String> = existing_items
            .into_iter()
            .map(|m| m.file_path.to_lowercase())
            .collect();

        let mut added_count = 0;
        let entries = match std::fs::read_dir(media_images_dir) {
            Ok(e) => e,
            Err(_) => return Ok(0),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "svg") {
                continue;
            }

            let filename = match path.file_name().and_then(|s| s.to_str()) {
                Some(f) => f,
                None => continue,
            };

            let servable_path = format!("/media/images/{}", filename);
            if existing_paths.contains(&servable_path.to_lowercase()) {
                continue;
            }

            // Derive a clean, human-readable display name from filename
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(filename);
            let display_name = stem
                .replace(['-', '_'], " ")
                .split_whitespace()
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");

            let item = MediaItem {
                id: format!("media-auto-{}", uuid::Uuid::new_v4()),
                name: display_name,
                media_type: "Image".to_string(),
                file_path: servable_path.clone(),
                duration_seconds: None,
                thumbnail_path: Some(servable_path),
                loop_playback: false,
            };

            if self.insert_media(&item).is_ok() {
                added_count += 1;
            }
        }

        if added_count > 0 {
            tracing::info!("Auto-discovered and registered {} new media items from {:?}", added_count, media_images_dir);
        }

        Ok(added_count)
    }

    /// Auto-discovers video files from the provided `media_videos_dir` (e.g. `web/media/videos`),
    /// generates human-readable titles, and registers them into the `media` table if not already present.
    pub fn sync_media_videos_folder(&self, media_videos_dir: &Path) -> Result<usize> {
        if !media_videos_dir.exists() || !media_videos_dir.is_dir() {
            return Ok(0);
        }

        let existing_paths: std::collections::HashSet<String> = self
            .get_media()?
            .into_iter()
            .map(|m| m.file_path.to_lowercase())
            .collect();

        let mut added_count = 0;
        let entries = match std::fs::read_dir(media_videos_dir) {
            Ok(e) => e,
            Err(_) => return Ok(0),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !matches!(ext.as_str(), "mp4" | "webm" | "mov" | "mkv" | "m4v" | "avi") {
                continue;
            }

            let filename = match path.file_name().and_then(|s| s.to_str()) {
                Some(f) => f,
                None => continue,
            };

            let servable_path = format!("/media/videos/{}", filename);
            if existing_paths.contains(&servable_path.to_lowercase()) {
                continue;
            }

            // Derive a clean, human-readable display name from filename
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(filename);
            let display_name = stem
                .replace(['-', '_'], " ")
                .split_whitespace()
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");

            let item = MediaItem {
                id: format!("media-auto-vid-{}", uuid::Uuid::new_v4()),
                name: display_name,
                media_type: "Video".to_string(),
                file_path: servable_path.clone(),
                duration_seconds: None,
                thumbnail_path: None,
                loop_playback: true,
            };

            if self.insert_media(&item).is_ok() {
                added_count += 1;
            }
        }

        if added_count > 0 {
            tracing::info!("Auto-discovered and registered {} new video media items from {:?}", added_count, media_videos_dir);
        }

        Ok(added_count)
    }

    // --- Themes CRUD (Resilient to legacy & modern schema) ---
    pub fn get_theme_by_name(&self, name: &str) -> Result<Option<Theme>> {
        let themes = self.get_themes()?;
        Ok(themes.into_iter().find(|t| t.name.eq_ignore_ascii_case(name)))
    }

    pub fn get_themes(&self) -> Result<Vec<Theme>> {
        let conn = self.conn();

        let mut stmt = conn.prepare_cached(
            "SELECT name, background, font_family, font_size, font_color, alignment,
                    text_shadow, line_height, letter_spacing, opacity,
                    margin_top, margin_bottom, margin_left, margin_right, vertical_align,
                    category, is_default, reference_position, chroma_key_enabled,
                    chroma_key_color, safe_area_percent
             FROM themes ORDER BY name ASC"
        )?;
        let theme_iter = stmt.query_map([], |row| {
            Ok(Theme {
                id: None,
                name: row.get(0)?,
                background: row.get(1)?,
                font_family: row.get(2)?,
                font_size: row.get(3)?,
                font_color: row.get(4)?,
                alignment: row.get(5)?,
                text_shadow: row.get(6)?,
                line_height: row.get(7)?,
                letter_spacing: row.get(8)?,
                opacity: row.get(9)?,
                margin_top: row.get(10)?,
                margin_bottom: row.get(11)?,
                margin_left: row.get(12)?,
                margin_right: row.get(13)?,
                vertical_align: row.get(14)?,
                category: row.get(15)?,
                is_default: row.get::<_, i64>(16)? != 0,
                reference_position: row.get(17)?,
                chroma_key_enabled: row.get::<_, i64>(18)? != 0,
                chroma_key_color: row.get(19)?,
                safe_area_percent: row.get::<_, i64>(20)? as u8,
            })
        })?;

        let mut themes = Vec::new();
        for t in theme_iter {
            themes.push(t?);
        }
        Ok(themes)
    }

    pub fn insert_theme(&self, theme: &Theme) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO themes (
                name, background, font_family, font_size, font_color, alignment,
                text_shadow, line_height, letter_spacing, opacity,
                margin_top, margin_bottom, margin_left, margin_right, vertical_align,
                category, is_default, reference_position, chroma_key_enabled,
                chroma_key_color, safe_area_percent
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
            params![
                theme.name, theme.background, theme.font_family, theme.font_size, theme.font_color, theme.alignment,
                theme.text_shadow, theme.line_height, theme.letter_spacing, theme.opacity,
                theme.margin_top, theme.margin_bottom, theme.margin_left, theme.margin_right, theme.vertical_align,
                theme.category, theme.is_default as i64, theme.reference_position, theme.chroma_key_enabled as i64,
                theme.chroma_key_color, theme.safe_area_percent as i64
            ],
        )?;

        Ok(())
    }

    pub fn delete_theme(&self, name: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM themes WHERE name = ?1", params![name])?;

        Ok(())
    }

    /// Clears `is_default` for every theme in `category`, then sets it on `name` —
    /// enforces exactly one default theme per category. Newly added songs/scriptures/
    /// presentations auto-apply their category's default (see `AddScheduleItem`-style
    /// commands in engine.rs).
    pub fn set_default_theme(&self, category: &str, name: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("UPDATE themes SET is_default = 0 WHERE category = ?1", params![category])?;
        conn.execute(
            "UPDATE themes SET is_default = 1 WHERE category = ?1 AND name = ?2",
            params![category, name],
        )?;
        Ok(())
    }

    /// Looks up the one `is_default` theme for a category (song/scripture/presentation),
    /// used to auto-apply a look when a schedule item is added without an explicit
    /// theme — see `apply_default_theme_if_unset` in src/api/routes.rs.
    pub fn get_default_theme_for_category(&self, category: &str) -> Result<Option<Theme>> {
        Ok(self.get_themes()?.into_iter().find(|t| t.category == category && t.is_default))
    }

    /// Seeds a starter theme catalog on first run only (no-op once any theme exists),
    /// so a fresh install has usable, editable Song/Scripture/Presentation themes
    /// instead of relying on the frontend's display-only DEFAULT_THEMES fallback.
    pub fn seed_default_themes_if_empty(&self) -> Result<()> {
        if !self.get_themes()?.is_empty() {
            return Ok(());
        }

        let song_gradients: &[(&str, &str, &str)] = &[
            ("Midnight Ocean", "linear-gradient(135deg, #0f2027, #203a43, #2c5364)", "Segoe UI"),
            ("Golden Sanctuary", "linear-gradient(135deg, #1f1c18, #473e34, #1f1c18)", "Georgia"),
            ("Deep Worship", "linear-gradient(135deg, #200122, #6f0000)", "Segoe UI"),
            ("Celestial Blue", "linear-gradient(135deg, #000428, #004e92)", "Arial"),
            ("Forest Grace", "linear-gradient(135deg, #134e5e, #71b280)", "Verdana"),
            ("Solid Dark", "#1a1a1a", "Segoe UI"),
            ("Ember Fire", "linear-gradient(135deg, #1c100e, #36150b, #1c100e)", "Segoe UI"),
        ];
        for (i, (name, bg, font)) in song_gradients.iter().enumerate() {
            self.insert_theme(&Theme {
                id: None,
                name: name.to_string(),
                background: bg.to_string(),
                font_family: font.to_string(),
                category: "song".to_string(),
                is_default: i == 0,
                ..Default::default()
            })?;
        }

        self.insert_theme(&Theme {
            id: None,
            name: "Scripture Classic".to_string(),
            background: "linear-gradient(135deg, #1a1a2e, #16213e)".to_string(),
            font_family: "Georgia".to_string(),
            category: "scripture".to_string(),
            is_default: true,
            reference_position: "top".to_string(),
            ..Default::default()
        })?;
        self.insert_theme(&Theme {
            id: None,
            name: "Scripture Inline".to_string(),
            background: "linear-gradient(135deg, #232526, #414345)".to_string(),
            font_family: "Georgia".to_string(),
            category: "scripture".to_string(),
            is_default: false,
            reference_position: "inline".to_string(),
            ..Default::default()
        })?;

        self.insert_theme(&Theme {
            id: None,
            name: "Presentation Light".to_string(),
            background: "#ffffff".to_string(),
            font_family: "Inter, sans-serif".to_string(),
            font_color: "#1a1a1a".to_string(),
            category: "presentation".to_string(),
            is_default: true,
            ..Default::default()
        })?;
        self.insert_theme(&Theme {
            id: None,
            name: "Presentation Dark".to_string(),
            background: "#0f172a".to_string(),
            font_family: "Inter, sans-serif".to_string(),
            category: "presentation".to_string(),
            is_default: false,
            ..Default::default()
        })?;

        Ok(())
    }

    // --- Presentation CRUD ---
    pub fn get_presentation_by_id(&self, id: &str) -> Result<Option<Presentation>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT id, title, author, slides_json, theme_name FROM presentations WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            let slides_json: String = row.get(3)?;
            let slides = load_and_synthesize_slides(&slides_json);
            Ok(Some(Presentation {
                id: row.get(0)?,
                title: row.get(1)?,
                author: row.get(2)?,
                slides,
                theme_name: row.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_presentations(&self) -> Result<Vec<Presentation>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT id, title, author, slides_json, theme_name FROM presentations ORDER BY title ASC")?;
        let pres_iter = stmt.query_map([], |row| {
            let slides_json: String = row.get(3)?;
            let slides = load_and_synthesize_slides(&slides_json);
            Ok(Presentation {
                id: row.get(0)?,
                title: row.get(1)?,
                author: row.get(2)?,
                slides,
                theme_name: row.get(4)?,
            })
        })?;

        let mut items = Vec::new();
        for p in pres_iter {
            items.push(p?);
        }
        Ok(items)
    }

    pub fn insert_presentation(&self, pres: &Presentation) -> Result<()> {
        let conn = self.conn();
        let slides_json = serialize_slides_for_storage(&pres.slides);
        conn.execute(
            "INSERT OR REPLACE INTO presentations (id, title, author, slides_json, theme_name)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![pres.id, pres.title, pres.author, slides_json, pres.theme_name],
        )?;

        Ok(())
    }

    pub fn delete_presentation(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM presentations WHERE id = ?1", params![id])?;

        Ok(())
    }

    // --- Slide Template CRUD ---
    pub fn get_slide_templates(&self) -> Result<Vec<crate::core::models::SlideTemplate>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, category, elements_json, background_json, thumbnail_data_url FROM slide_templates ORDER BY name ASC"
        )?;
        let iter = stmt.query_map([], |row| {
            let elements_json: String = row.get(3)?;
            let background_json: Option<String> = row.get(4)?;
            let elements = serde_json::from_str(&elements_json).unwrap_or_default();
            let background = background_json.and_then(|j| serde_json::from_str(&j).ok());
            Ok(crate::core::models::SlideTemplate {
                id: row.get(0)?,
                name: row.get(1)?,
                category: row.get(2)?,
                elements,
                background,
                thumbnail_data_url: row.get(5)?,
            })
        })?;

        let mut items = Vec::new();
        for t in iter {
            items.push(t?);
        }
        Ok(items)
    }

    pub fn insert_slide_template(&self, template: &crate::core::models::SlideTemplate) -> Result<()> {
        let conn = self.conn();
        let elements_json = serde_json::to_string(&template.elements).unwrap_or_else(|_| "[]".to_string());
        let background_json = template.background.as_ref().map(|b| serde_json::to_string(b).unwrap_or_default());
        conn.execute(
            "INSERT OR REPLACE INTO slide_templates (id, name, category, elements_json, background_json, thumbnail_data_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![template.id, template.name, template.category, elements_json, background_json, template.thumbnail_data_url],
        )?;

        Ok(())
    }

    /// Seeds the 4 EasyWorship-style master slide layouts on first run only (no-op
    /// once any slide template exists) — these structure a Presentation item's slides;
    /// the active Presentation-category `Theme` supplies the look on top of whichever
    /// one the operator applies via the editor's existing "Apply Template" dropdown.
    pub fn seed_master_layouts_if_empty(&self) -> Result<()> {
        use crate::core::models::{
            ElementEffects, ElementTransform, SlideElement, SlideTemplate, TextBlock,
            TextParagraphStyle, TextRun,
        };

        if !self.get_slide_templates()?.is_empty() {
            return Ok(());
        }

        fn text_element(x: f64, y: f64, w: f64, h: f64, text: &str, align: &str, font_size_pt: f64) -> SlideElement {
            SlideElement::TextBlock {
                id: format!("el_{}", uuid::Uuid::new_v4()),
                transform: ElementTransform { x, y, w, h, rotation_deg: 0.0, z_index: 0, locked: false, opacity: 1.0 },
                block: TextBlock {
                    runs: vec![TextRun { text: text.to_string(), font_size_pt, ..Default::default() }],
                    paragraph_style: TextParagraphStyle { align: align.to_string(), ..Default::default() },
                    effects: ElementEffects::default(),
                    autofit: true,
                },
            }
        }

        let layouts: Vec<(&str, Vec<SlideElement>)> = vec![
            ("Title Slide", vec![text_element(0.1, 0.35, 0.8, 0.3, "Title", "center", 54.0)]),
            (
                "Title & Content",
                vec![
                    text_element(0.06, 0.06, 0.88, 0.18, "Title", "left", 40.0),
                    text_element(0.06, 0.28, 0.88, 0.64, "Content", "left", 28.0),
                ],
            ),
            ("Content Only", vec![text_element(0.06, 0.08, 0.88, 0.84, "Content", "left", 28.0)]),
            ("Blank", vec![]),
        ];

        for (name, elements) in layouts {
            self.insert_slide_template(&SlideTemplate {
                id: format!("master_layout_{}", name.to_lowercase().replace(' ', "_").replace('&', "and")),
                name: format!("Master: {}", name),
                category: Some("Master Layout".to_string()),
                elements,
                background: None,
                thumbnail_data_url: None,
            })?;
        }

        Ok(())
    }

    pub fn delete_slide_template(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM slide_templates WHERE id = ?1", params![id])?;

        Ok(())
    }

    // --- Settings Key-Value ---
    pub fn get_settings(&self) -> Result<HashMap<String, String>> {
        let conn = self.conn();
        let mut map = HashMap::new();

        // Check if legacy app_metadata exists as fallback
        if let Ok(mut stmt) = conn.prepare_cached("SELECT key, value FROM app_metadata") {
            if let Ok(iter) = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))) {
                for item in iter.flatten() {
                    map.insert(item.0, item.1);
                }
            }
        }

        // Canonical settings table overrides legacy metadata
        if let Ok(mut stmt) = conn.prepare_cached("SELECT key, value FROM settings") {
            if let Ok(iter) = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))) {
                for item in iter.flatten() {
                    map.insert(item.0, item.1);
                }
            }
        }

        // Set defaults if empty
        if !map.contains_key("churchName") {
            map.insert("churchName".to_string(), "Grace Sanctuary Media Ministry".to_string());
        }
        if !map.contains_key("aspectRatio") {
            map.insert("aspectRatio".to_string(), "16:9".to_string());
        }

        Ok(map)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn();
        if let Ok(mut stmt) = conn.prepare_cached("SELECT value FROM settings WHERE key = ?1") {
            if let Ok(mut rows) = stmt.query(params![key]) {
                if let Ok(Some(row)) = rows.next() {
                    return Ok(Some(row.get(0)?));
                }
            }
        }
        // Fallback to legacy app_metadata if not found in settings
        if let Ok(mut stmt) = conn.prepare_cached("SELECT value FROM app_metadata WHERE key = ?1") {
            if let Ok(mut rows) = stmt.query(params![key]) {
                if let Ok(Some(row)) = rows.next() {
                    return Ok(Some(row.get(0)?));
                }
            }
        }
        Ok(None)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;

        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        Ok(())
    }

    // --- Keyring Secret Ownership Tracking ---

    /// Records or updates ownership of a keyring secret.
    /// `owner` is either "host" or "plugin:<plugin_name>".
    pub fn record_keyring_owner(&self, service: &str, account: &str, owner: &str) -> Result<()> {
        let conn = self.conn();
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO keyring_metadata (service, account, owner, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(service, account) DO UPDATE SET owner = excluded.owner, created_at = excluded.created_at",
            params![service, account, owner, now],
        )?;
        Ok(())
    }

    /// Queries the owner of a keyring secret. Returns None if unrecorded.
    pub fn get_keyring_owner(&self, service: &str, account: &str) -> Result<Option<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT owner FROM keyring_metadata WHERE service = ?1 AND account = ?2",
        )?;
        let mut rows = stmt.query(params![service, account])?;
        if let Some(row) = rows.next()? {
            let owner: String = row.get(0)?;
            Ok(Some(owner))
        } else {
            Ok(None)
        }
    }

    /// Removes ownership metadata when a secret is deleted.
    pub fn delete_keyring_metadata(&self, service: &str, account: &str) -> Result<bool> {
        let conn = self.conn();
        let count = conn.execute(
            "DELETE FROM keyring_metadata WHERE service = ?1 AND account = ?2",
            params![service, account],
        )?;
        Ok(count > 0)
    }

    pub fn save_event(&self, envelope: &crate::core::events::EventEnvelope) -> Result<()> {
        let conn = self.conn();
        let payload = serde_json::to_string(&envelope.event).unwrap_or_default();
        let event_type = match &envelope.event {
            crate::core::events::ShowEvent::ItemWentLive { .. } => "ItemWentLive",
            crate::core::events::ShowEvent::LiveSlideChanged { .. } => "LiveSlideChanged",
            crate::core::events::ShowEvent::ItemStaged { .. } => "ItemStaged",
            crate::core::events::ShowEvent::StagedSlideChanged { .. } => "StagedSlideChanged",
            crate::core::events::ShowEvent::BlackoutToggled { .. } => "BlackoutToggled",
            crate::core::events::ShowEvent::ClearTextToggled { .. } => "ClearTextToggled",
            crate::core::events::ShowEvent::LogoToggled { .. } => "LogoToggled",
            crate::core::events::ShowEvent::AlertChanged { .. } => "AlertChanged",
            crate::core::events::ShowEvent::TransitionChanged { .. } => "TransitionChanged",
            crate::core::events::ShowEvent::ScheduleItemAdded { .. } => "ScheduleItemAdded",
            crate::core::events::ShowEvent::ScheduleItemRemoved { .. } => "ScheduleItemRemoved",
            crate::core::events::ShowEvent::ScheduleReordered { .. } => "ScheduleReordered",
            crate::core::events::ShowEvent::ScheduleSlidesReordered { .. } => "ScheduleSlidesReordered",
            crate::core::events::ShowEvent::ScheduleSlideDuplicated { .. } => "ScheduleSlideDuplicated",
            crate::core::events::ShowEvent::ScheduleSlideRemoved { .. } => "ScheduleSlideRemoved",
            crate::core::events::ShowEvent::ScheduleItemExpandToggled { .. } => "ScheduleItemExpandToggled",
            crate::core::events::ShowEvent::ScheduleItemThemeSet { .. } => "ScheduleItemThemeSet",
            crate::core::events::ShowEvent::SlideBackgroundSet { .. } => "SlideBackgroundSet",
            crate::core::events::ShowEvent::GlobalThemeSet { .. } => "GlobalThemeSet",
            crate::core::events::ShowEvent::ScheduleLoaded { .. } => "ScheduleLoaded",
            crate::core::events::ShowEvent::ScheduleCleared => "ScheduleCleared",
            crate::core::events::ShowEvent::ScheduleTitleSet { .. } => "ScheduleTitleSet",
            _ => "MediaControlEvent",
        };
        conn.execute(
            "INSERT INTO event_log (sequence_number, event_type, payload, timestamp_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(sequence_number) DO NOTHING",
            (envelope.sequence_number, event_type, payload, envelope.timestamp_ms as i64),
        )?;
        Ok(())
    }

    pub fn get_max_event_sequence(&self) -> Result<u64> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT COALESCE(MAX(sequence_number), 0) FROM event_log")?;
        let max_seq: i64 = stmt.query_row([], |row| row.get(0))?;
        Ok(max_seq.max(0) as u64)
    }

    pub fn load_events(&self) -> Result<Vec<crate::core::events::EventEnvelope>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT sequence_number, payload, timestamp_ms FROM event_log ORDER BY sequence_number ASC")?;
        let event_iter = stmt.query_map([], |row| {
            let seq: u64 = row.get(0)?;
            let payload_str: String = row.get(1)?;
            let ts: i64 = row.get(2)?;
            match serde_json::from_str::<crate::core::events::ShowEvent>(&payload_str) {
                Ok(event) => Ok(Some(crate::core::events::EventEnvelope {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    sequence_number: seq,
                    event,
                    timestamp_ms: ts as u64,
                })),
                Err(e) => {
                    tracing::warn!("Skipping unparseable event #{} from event_log: {}", seq, e);
                    Ok(None)
                }
            }
        })?;
        
        let mut events = Vec::new();
        for env in event_iter.flatten().flatten() {
            events.push(env);
        }
        Ok(events)
    }

    pub fn get_installed_bibles(&self) -> Result<Vec<InstalledBibleInfo>> {
        let b_conns = self.get_all_bible_conns()?;
        let mut result = Vec::new();

        let settings = self.get_settings().unwrap_or_default();
        let default_version = settings
            .get("defaultBibleVersion")
            .or_else(|| settings.get("default_bible"))
            .cloned()
            .unwrap_or_default();

        for conn_arc in b_conns {
            let conn = conn_arc.lock().unwrap_or_else(|e| e.into_inner());
            let mut stmt = match conn.prepare_cached(
                "SELECT version, COUNT(*) FROM scriptures 
                 WHERE version IS NOT NULL AND TRIM(version) != '' AND LOWER(TRIM(version)) != 'tests' 
                 GROUP BY version 
                 HAVING COUNT(*) > 0
                 ORDER BY version ASC;",
            ) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let rows = stmt.query_map([], |row| {
                let ver: String = row.get(0)?;
                let count: usize = row.get(1)?;
                Ok((ver, count))
            });

            if let Ok(iter) = rows {
                for (ver, count) in iter.flatten() {
                    let is_default = !default_version.is_empty()
                        && ver.eq_ignore_ascii_case(&default_version);
                    result.push(InstalledBibleInfo {
                        id: ver.clone(),
                        name: ver.clone(),
                        abbreviation: ver.clone(),
                        language: "ANY".to_string(),
                        is_default,
                        is_licensed: false,
                        verse_count: count,
                    });
                }
            }
        }

        result.sort_by(|a, b| a.name.cmp(&b.name));
        result.dedup_by(|a, b| a.id.to_uppercase() == b.id.to_uppercase());

        // If no translation matched default_version and we have at least one installed Bible:
        if !result.is_empty() {
            if default_version.is_empty() {
                result[0].is_default = true;
            } else if !result.iter().any(|b| b.is_default) {
                for b in &mut result {
                    if b.id.to_uppercase() == default_version.to_uppercase() {
                        b.is_default = true;
                        break;
                    }
                }
            }
        }

        Ok(result)
    }

    /// Uninstalls an installed Bible database, closing its connection and removing file artifacts.
    pub fn delete_installed_bible(&self, version: &str) -> Result<bool> {
        let key = version.trim().to_uppercase();
        let safe_name = Self::sanitize_bible_filename(&key);

        // Remove from connection cache
        {
            let mut conns = self.bible_conns.lock().unwrap_or_else(|e| e.into_inner());
            conns.remove(&key);
            conns.remove(&safe_name);
        }

        if self.is_in_memory {
            return Ok(true);
        }

        let b_dir = self
            .bibles_dir
            .as_ref()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("bibles"));
        let db_path = b_dir.join(format!("{}.db", safe_name));
        let wal_path = b_dir.join(format!("{}.db-wal", safe_name));
        let shm_path = b_dir.join(format!("{}.db-shm", safe_name));

        let mut removed = false;
        if db_path.exists() {
            let _ = std::fs::remove_file(&db_path);
            removed = true;
        }
        if wal_path.exists() {
            let _ = std::fs::remove_file(&wal_path);
        }
        if shm_path.exists() {
            let _ = std::fs::remove_file(&shm_path);
        }

        Ok(removed)
    }

    // --- Paired Client Displays ---

    pub fn get_paired_devices(&self) -> Result<Vec<PairedDevice>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, platform, paired_at, last_seen, token FROM paired_devices ORDER BY paired_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PairedDevice {
                id: row.get(0)?,
                name: row.get(1)?,
                platform: row.get(2)?,
                paired_at: row.get(3)?,
                last_seen: row.get(4)?,
                token: row.get(5)?,
            })
        })?;
        let mut list = Vec::new();
        for item in rows.flatten() {
            list.push(item);
        }
        Ok(list)
    }

    pub fn get_paired_device(&self, id: &str) -> Result<Option<PairedDevice>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, platform, paired_at, last_seen, token FROM paired_devices WHERE id = ?1"
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(PairedDevice {
                id: row.get(0)?,
                name: row.get(1)?,
                platform: row.get(2)?,
                paired_at: row.get(3)?,
                last_seen: row.get(4)?,
                token: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_paired_device_by_token(&self, token: &str) -> Result<Option<PairedDevice>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, platform, paired_at, last_seen, token FROM paired_devices WHERE token = ?1"
        )?;
        let mut rows = stmt.query(params![token])?;
        if let Some(row) = rows.next()? {
            Ok(Some(PairedDevice {
                id: row.get(0)?,
                name: row.get(1)?,
                platform: row.get(2)?,
                paired_at: row.get(3)?,
                last_seen: row.get(4)?,
                token: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn upsert_paired_device(&self, device: &PairedDevice) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO paired_devices (id, name, platform, paired_at, last_seen, token)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 platform = excluded.platform,
                 last_seen = excluded.last_seen,
                 token = excluded.token",
            params![
                device.id,
                device.name,
                device.platform,
                device.paired_at,
                device.last_seen,
                device.token,
            ],
        )?;
        Ok(())
    }

    pub fn touch_paired_device(&self, id: &str) -> Result<bool> {
        let conn = self.conn();
        let now = chrono::Utc::now().timestamp_millis();
        let count = conn.execute(
            "UPDATE paired_devices SET last_seen = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(count > 0)
    }

    pub fn delete_paired_device(&self, id: &str) -> Result<bool> {
        let conn = self.conn();
        let count = conn.execute(
            "DELETE FROM paired_devices WHERE id = ?1",
            params![id],
        )?;
        Ok(count > 0)
    }

    pub fn execute_raw(&self, sql: &str) -> Result<usize> {
        let conn = self.conn();
        conn.execute(sql, [])
    }
}

