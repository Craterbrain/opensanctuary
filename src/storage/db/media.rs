use std::collections::HashSet;
use std::path::Path;
use rusqlite::{params, Connection, Result};

use crate::core::models::MediaItem;

use super::Database;

impl Database {
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
        let existing_paths: HashSet<String> = existing_items
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

        let existing_paths: HashSet<String> = self
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

                // Convert H.264 drop-ins to VP9 in the background (same
                // reasoning as ytdlp_import.rs) without blocking this scan,
                // which runs synchronously on every GET /api/media. Only
                // spawned when a Tokio runtime is actually active -- this fn
                // is also exercised from plain `#[test]`s with no runtime.
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    let db_clone = self.clone();
                    let disk_path = path.clone();
                    let mut item_for_transcode = item.clone();
                    handle.spawn(async move {
                        if crate::storage::transcode::needs_transcode(&disk_path).await {
                            match crate::storage::transcode::transcode_h264_to_vp9(&disk_path, |_pct| {}).await {
                                Ok(new_path) => {
                                    if let Some(filename) = new_path.file_name().and_then(|f| f.to_str()) {
                                        item_for_transcode.file_path = format!("/media/videos/{}", filename);
                                        if let Err(e) = db_clone.insert_media(&item_for_transcode) {
                                            tracing::warn!("transcode succeeded but failed to update DB record for {:?}: {} (transcoded file at {:?} is now orphaned)", disk_path, e, new_path);
                                        }
                                    }
                                }
                                Err(e) => tracing::warn!("background transcode skipped for {:?}: {}", disk_path, e),
                            }
                        }
                    });
                }
            }
        }

        if added_count > 0 {
            tracing::info!("Auto-discovered and registered {} new video media items from {:?}", added_count, media_videos_dir);
        }

        Ok(added_count)
    }
}
