use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use rusqlite::{params, Connection, Result};

use crate::core::models::{ScriptureItem, ScriptureVerse, Song, Theme};

use super::{load_and_synthesize_slides, serialize_slides_for_storage};

#[derive(Clone)]
pub struct Database {
    // Core presentation database (media, themes, presentations, settings, event_log)
    pub(super) main_conn: Arc<Mutex<Connection>>,

    // Path configuration
    pub(super) bibles_dir: Option<PathBuf>,
    pub(super) songs_dir: Option<PathBuf>,

    // Songs databases: split between public domain and copyrighted works
    pub(super) songs_pd_conn: Arc<Mutex<Connection>>,
    pub(super) songs_cr_conn: Arc<Mutex<Connection>>,

    // Bibles databases: one SQLite file per Bible translation in bibles/
    pub(super) bible_conns: Arc<Mutex<HashMap<String, Arc<Mutex<Connection>>>>>,

    // In-memory mode flag for fast, isolated testing
    pub(super) is_in_memory: bool,
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

    /// Flushes SQLite Write-Ahead Logs (WAL) for all open connections to disk.
    /// Used before backups, file relocations, and external copying.
    pub fn checkpoint(&self) -> Result<()> {
        if self.is_in_memory {
            return Ok(());
        }
        let _ = self.main_conn().execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        let _ = self.songs_pd_conn.lock().unwrap_or_else(|e| e.into_inner()).execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        let _ = self.songs_cr_conn.lock().unwrap_or_else(|e| e.into_inner()).execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        if let Ok(bible_map) = self.bible_conns.lock() {
            for conn_arc in bible_map.values() {
                if let Ok(c) = conn_arc.lock() {
                    let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
                }
            }
        }
        Ok(())
    }

    /// Checkpoints, then holds every connection's lock for the duration of
    /// `f` -- blocking every other read/write anywhere in the app for that
    /// time, since all of them go through `main_conn()`/`songs_pd_conn`/
    /// `songs_cr_conn`/`bible_conns` first. Used by `paths::move_data_dir` so
    /// the on-disk copy it makes is a consistent snapshot: without this, a
    /// write could land in the gap between `checkpoint()` returning and the
    /// copy loop reading each file, corrupting the copy, or (worse, combined
    /// with `delete_source`) landing on the original file *after* the copy
    /// was already taken and then being silently discarded when the original
    /// is deleted.
    pub fn with_all_connections_locked<R>(&self, f: impl FnOnce() -> R) -> R {
        let main = self.main_conn();
        if !self.is_in_memory {
            let _ = main.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
        let songs_pd = self.songs_pd_conn.lock().unwrap_or_else(|e| e.into_inner());
        if !self.is_in_memory {
            let _ = songs_pd.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
        let songs_cr = self.songs_cr_conn.lock().unwrap_or_else(|e| e.into_inner());
        if !self.is_in_memory {
            let _ = songs_cr.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
        let bible_map = self.bible_conns.lock().unwrap_or_else(|e| e.into_inner());
        let mut bible_guards = Vec::with_capacity(bible_map.len());
        for conn_arc in bible_map.values() {
            let g = conn_arc.lock().unwrap_or_else(|e| e.into_inner());
            if !self.is_in_memory {
                let _ = g.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            }
            bible_guards.push(g);
        }

        let result = f();

        drop(bible_guards);
        drop(bible_map);
        drop(songs_cr);
        drop(songs_pd);
        drop(main);
        result
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
        // Holds device tokens: owner-only on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(db_path.as_ref(), std::fs::Permissions::from_mode(0o600));
        }
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

    pub(super) fn init_scriptures_schema(conn: &Connection) -> Result<()> {
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

    pub fn execute_raw(&self, sql: &str) -> Result<usize> {
        let conn = self.conn();
        conn.execute(sql, [])
    }
}
