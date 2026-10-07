use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use rusqlite::{params, Connection, Result};

use serde::{Deserialize, Serialize};

use crate::core::models::{ScriptureItem, ScriptureVerse};

use super::Database;

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

impl Database {
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
    pub(super) fn get_or_create_bible_conn(&self, version: &str) -> Result<Arc<Mutex<Connection>>> {
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
        let mut by_version: std::collections::HashMap<String, Vec<&ScriptureItem>> = std::collections::HashMap::new();
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
}
