use rusqlite::{params, Result};

use serde::{Deserialize, Serialize};

use super::Database;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub paired_at: i64,
    pub last_seen: i64,
    pub token: String,
}

/// `platform` value only `POST /api/pairing/remote-session` (host-token gated)
/// ever assigns -- the one role allowed to send show commands. The authorize
/// route refuses to hand it out, so pairing a display can't produce it.
pub const REMOTE_CONTROL_PLATFORM: &str = "mobile-remote";

/// `platform` of a persistent console credential (see `console_credential_hash`).
pub const CONSOLE_PLATFORM: &str = "console";

/// What is stored in `paired_devices.token` for a console credential: the SHA-256 of the
/// bearer secret, never the secret itself. The raw credential exists only in the
/// browser that was issued it, so a copy of `library.db` (or the Paired Devices list)
/// can't be turned into console access.
pub fn console_credential_hash(raw: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(raw.as_bytes());
    let mut out = String::from("sha256:");
    for b in digest.iter() {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

impl PairedDevice {
    /// Role check: paired *displays* (Android TV, Roku, ...) only receive
    /// state; only a paired remote may drive the show.
    pub fn can_control_show(&self) -> bool {
        self.platform == REMOTE_CONTROL_PLATFORM
    }
}

impl Database {
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
}
