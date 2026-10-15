use rusqlite::{params, Result};

use super::Database;

impl Database {
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

    /// Every secret on record that `owner` created, and forgets the records: for uninstalling a plugin.
    pub fn take_keyring_entries_owned_by(&self, owners: &[String]) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut found = Vec::new();
        for owner in owners {
            let mut stmt = conn.prepare("SELECT service, account FROM keyring_metadata WHERE owner = ?1")?;
            let rows = stmt.query_map(params![owner], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                found.push(row?);
            }
            conn.execute("DELETE FROM keyring_metadata WHERE owner = ?1", params![owner])?;
        }
        Ok(found)
    }

    /// The shared-namespace entries on record: `(service, account, owner)`, never the secrets.
    pub fn list_shared_keyring_entries(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT service, account, owner FROM keyring_metadata WHERE service LIKE 'OpenSanctuary:Shared:%' ORDER BY service, account",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
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
}
