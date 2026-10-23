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

    /// Registers a brand-new device id, atomically: `ON CONFLICT DO NOTHING` under one
    /// mutex-guarded call means there is no gap between "does this id exist" and "write it" for
    /// another concurrent call to land in. Returns `true` if this call actually created the row,
    /// `false` if `id` was already taken (by an earlier pairing, or by a racing call that won).
    ///
    /// This is what `authorize_pairing_handler` uses for the common case -- a caller with no host
    /// token pairing a *new* device. A plain check-then-`upsert_paired_device` (the previous
    /// shape) has a real window: `get_paired_device` and `upsert_paired_device` are two separate
    /// calls, each independently locking `main_conn`, so two concurrent `authorize` calls for the
    /// same new `device_id` could both pass the "does it exist" check before either writes --
    /// both would report success, each with its own token, and only one of those tokens ends up
    /// actually stored (last write wins). Whichever caller loses that race still generated and was
    /// told a token that matches nothing, and if a second, unrelated holder of a valid pairing
    /// session is racing a legitimate pairing for an id it guessed or observed, it ends up knowing
    /// whichever token *does* land in storage, including the real device's. This method closes
    /// that gap by letting SQLite's own uniqueness constraint resolve the race in a single
    /// statement, with nothing for a second caller to observe in between.
    pub fn insert_paired_device_if_new(&self, device: &PairedDevice) -> Result<bool> {
        let conn = self.conn();
        let rows = conn.execute(
            "INSERT INTO paired_devices (id, name, platform, paired_at, last_seen, token)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO NOTHING",
            params![
                device.id,
                device.name,
                device.platform,
                device.paired_at,
                device.last_seen,
                device.token,
            ],
        )?;
        Ok(rows == 1)
    }

    /// Unconditional create-or-overwrite -- used where the caller's own authorization (a valid
    /// host token) already makes "this id is already paired" irrelevant: an operator explicitly
    /// re-pairing a device, or the console's own re-pair path, every one of these race-free
    /// anyway (the host token itself already gates them to one trusted caller at a time in
    /// practice, and overwriting is the intended effect, not an accident to prevent).
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

#[cfg(test)]
mod race_tests {
    use super::*;
    use crate::storage::Database;

    fn device(id: &str, token: &str) -> PairedDevice {
        PairedDevice { id: id.to_string(), name: "Race TV".to_string(), platform: "android-tv".to_string(), paired_at: 0, last_seen: 0, token: token.to_string() }
    }

    /// Real OS threads, not tokio tasks: the race `insert_paired_device_if_new` exists to close
    /// is two threads actually running the check and the write at the same time (see its doc
    /// comment) -- `Database` is `Clone` over `Arc<Mutex<Connection>>`, so every thread here holds
    /// an independent handle to the exact same connection the real server would.
    #[test]
    fn concurrent_insert_for_the_same_new_id_lets_exactly_one_thread_win() {
        let db = Database::in_memory().expect("in-memory db");
        let n = 32;
        let handles: Vec<_> = (0..n)
            .map(|i| {
                let db = db.clone();
                std::thread::spawn(move || db.insert_paired_device_if_new(&device("race_tv_1", &format!("tok_{i}"))))
            })
            .collect();
        let results: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap().expect("no db error")).collect();
        let winners = results.iter().filter(|&&won| won).count();
        assert_eq!(winners, 1, "exactly one of {n} concurrent inserts for the same id should win, got {winners}");

        // And storage holds exactly one row for it, whichever one that was.
        let stored = db.get_paired_device("race_tv_1").unwrap().expect("the winning row should exist");
        assert!(stored.token.starts_with("tok_"));
    }

    /// Different ids never contend with each other -- same mutex, but no shared key to collide on.
    #[test]
    fn concurrent_insert_for_different_ids_all_win() {
        let db = Database::in_memory().expect("in-memory db");
        let n = 16;
        let handles: Vec<_> = (0..n)
            .map(|i| {
                let db = db.clone();
                std::thread::spawn(move || db.insert_paired_device_if_new(&device(&format!("tv_{i}"), &format!("tok_{i}"))))
            })
            .collect();
        let results: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap().expect("no db error")).collect();
        assert!(results.iter().all(|&won| won), "distinct ids should never conflict with each other: {results:?}");
        assert_eq!(db.get_paired_devices().unwrap().len(), n);
    }

    #[test]
    fn a_second_insert_for_an_already_taken_id_loses_without_touching_the_stored_row() {
        let db = Database::in_memory().expect("in-memory db");
        assert!(db.insert_paired_device_if_new(&device("tv_1", "tok_first")).unwrap());
        assert!(!db.insert_paired_device_if_new(&device("tv_1", "tok_second")).unwrap());
        assert_eq!(db.get_paired_device("tv_1").unwrap().unwrap().token, "tok_first");
    }
}
