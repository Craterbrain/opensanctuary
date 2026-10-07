use std::collections::HashMap;
use rusqlite::{params, Result};

use super::Database;

impl Database {
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
}
