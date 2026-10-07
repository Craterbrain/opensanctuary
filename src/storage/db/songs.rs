use rusqlite::{params, Connection, Result};
use std::sync::{Arc, Mutex};

use crate::core::models::Song;

use super::{load_and_synthesize_slides, serialize_slides_for_storage, Database};

impl Database {
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
}
