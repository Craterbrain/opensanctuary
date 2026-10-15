//! Persistence for plugin permission grants and their audit trail
//! (docs/PLUGIN_PERMISSIONS.md).

use rusqlite::{params, Result};
use serde::Serialize;

use super::Database;

#[derive(Debug, Clone, Serialize)]
pub struct PluginGrantRow {
    pub plugin_id: String,
    pub kind: String,
    pub name: String,
    pub version: Option<String>,
    pub source_path: String,
    pub observed_manifest_sha256: Option<String>,
    pub observed_digest: String,
    pub observed_signer: Option<String>,
    pub approved_signer: Option<String>,
    pub requested_json: String,
    pub error: Option<String>,
    pub approved_manifest_sha256: Option<String>,
    pub approved_digest: Option<String>,
    pub status: String,
    pub granted_json: String,
    pub decided_at: Option<i64>,
    pub decided_by: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
    /// `[[path, sha256], ...]` for the files on disk now / when approved (None for rows approved before this was recorded).
    pub observed_files_json: Option<String>,
    pub approved_files_json: Option<String>,
    pub approved_version: Option<String>,
    /// The permissions the plugin asked for when it was approved (granted_json is only what was granted).
    pub approved_requested_json: Option<String>,
}

impl PluginGrantRow {
    /// `active` only while approved and the files on disk still match what was reviewed.
    pub fn effective_state(&self) -> &'static str {
        match self.status.as_str() {
            "approved" => {
                if self.error.is_some() {
                    "incompatible"
                } else if self.approved_digest.as_deref() == Some(self.observed_digest.as_str()) {
                    "active"
                } else {
                    "changed"
                }
            }
            "denied" => "denied",
            "revoked" => "revoked",
            _ => {
                if self.error.is_some() {
                    "incompatible"
                } else {
                    "pending"
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginAuditRow {
    pub id: i64,
    pub ts_ms: i64,
    pub plugin_id: String,
    pub actor: String,
    pub action: String,
    pub detail_json: String,
}

const GRANT_COLS: &str = "plugin_id, kind, name, version, source_path, observed_manifest_sha256, observed_digest, \
     observed_signer, approved_signer, requested_json, error, approved_manifest_sha256, approved_digest, status, granted_json, decided_at, decided_by, \
     first_seen, last_seen, observed_files_json, approved_files_json, approved_version, approved_requested_json";

fn grant_from_row(row: &rusqlite::Row<'_>) -> Result<PluginGrantRow> {
    Ok(PluginGrantRow {
        plugin_id: row.get(0)?,
        kind: row.get(1)?,
        name: row.get(2)?,
        version: row.get(3)?,
        source_path: row.get(4)?,
        observed_manifest_sha256: row.get(5)?,
        observed_digest: row.get(6)?,
        observed_signer: row.get(7)?,
        approved_signer: row.get(8)?,
        requested_json: row.get(9)?,
        error: row.get(10)?,
        approved_manifest_sha256: row.get(11)?,
        approved_digest: row.get(12)?,
        status: row.get(13)?,
        granted_json: row.get(14)?,
        decided_at: row.get(15)?,
        decided_by: row.get(16)?,
        first_seen: row.get(17)?,
        last_seen: row.get(18)?,
        observed_files_json: row.get(19)?,
        approved_files_json: row.get(20)?,
        approved_version: row.get(21)?,
        approved_requested_json: row.get(22)?,
    })
}

/// What a discovery pass saw on disk for one plugin.
pub struct ObservedPlugin<'a> {
    pub plugin_id: &'a str,
    pub kind: &'a str,
    pub name: &'a str,
    pub version: Option<&'a str>,
    pub source_path: &'a str,
    pub manifest_sha256: Option<&'a str>,
    pub digest: &'a str,
    pub signer: Option<&'a str>,
    pub requested_json: &'a str,
    pub error: Option<&'a str>,
    pub files_json: &'a str,
}

impl Database {
    /// Drops rows for plugin kinds that no longer exist (`native`, `legacy-native`, `legacy-web`).
    /// The audit log is history, so it keeps its entries.
    pub fn purge_removed_plugin_kinds(&self) -> Result<usize> {
        let conn = self.conn();
        Ok(conn.execute("DELETE FROM plugin_grants WHERE kind IN ('native', 'legacy-native', 'legacy-web')", [])?)
    }

    /// Forgets a plugin that was uninstalled. The audit log is history, so it keeps its entries.
    pub fn delete_plugin_grant(&self, plugin_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM plugin_grants WHERE plugin_id = ?1", params![plugin_id])?;
        Ok(())
    }

    pub fn get_plugin_grant(&self, plugin_id: &str) -> Result<Option<PluginGrantRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!("SELECT {GRANT_COLS} FROM plugin_grants WHERE plugin_id = ?1"))?;
        let mut rows = stmt.query(params![plugin_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(grant_from_row(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_plugin_grants(&self) -> Result<Vec<PluginGrantRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!("SELECT {GRANT_COLS} FROM plugin_grants ORDER BY name COLLATE NOCASE"))?;
        let rows = stmt.query_map([], grant_from_row)?;
        rows.collect()
    }

    /// Records what is on disk. A brand-new plugin is inserted as `pending` with nothing granted
    /// (nothing is ever approved automatically); an existing one keeps its decision and only its
    /// `observed_*` fields move.
    pub fn upsert_observed_plugin(&self, o: &ObservedPlugin<'_>) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO plugin_grants (plugin_id, kind, name, version, source_path, observed_manifest_sha256, \
                 observed_digest, requested_json, error, approved_manifest_sha256, approved_digest, status, \
                 granted_json, decided_at, decided_by, first_seen, last_seen, observed_signer, approved_signer, observed_files_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9, NULL, NULL, 'pending', '[]', NULL, NULL, ?10, ?10, ?11, NULL, ?12)
             ON CONFLICT(plugin_id) DO UPDATE SET
                 kind = excluded.kind, name = excluded.name, version = excluded.version,
                 source_path = excluded.source_path,
                 observed_manifest_sha256 = excluded.observed_manifest_sha256,
                 observed_digest = excluded.observed_digest,
                 observed_signer = excluded.observed_signer,
                 requested_json = excluded.requested_json,
                 error = excluded.error,
                 observed_files_json = excluded.observed_files_json,
                 last_seen = excluded.last_seen",
            params![
                o.plugin_id,
                o.kind,
                o.name,
                o.version,
                o.source_path,
                o.manifest_sha256,
                o.digest,
                o.requested_json,
                o.error,
                now,
                o.signer,
                o.files_json,
            ],
        )?;
        Ok(())
    }

    /// Stores the operator's decision. `granted_json` is the list of permissions granted
    /// (empty unless approving); approving pins the manifest and digest that were reviewed.
    pub fn set_plugin_decision(
        &self,
        plugin_id: &str,
        status: &str,
        granted_json: &str,
        pin_manifest_sha256: Option<&str>,
        pin_digest: Option<&str>,
        pin_signer: Option<&str>,
        decided_by: &str,
    ) -> Result<bool> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn();
        let changed = if status == "approved" {
            conn.execute(
                "UPDATE plugin_grants SET status = ?2, granted_json = ?3, approved_manifest_sha256 = ?4, \
                     approved_digest = ?5, decided_at = ?6, decided_by = ?7, approved_signer = ?8, \
                     approved_files_json = observed_files_json, approved_version = version, approved_requested_json = requested_json \
                     WHERE plugin_id = ?1",
                params![plugin_id, status, granted_json, pin_manifest_sha256, pin_digest, now, decided_by, pin_signer],
            )?
        } else {
            conn.execute(
                "UPDATE plugin_grants SET status = ?2, granted_json = '[]', decided_at = ?3, decided_by = ?4 \
                 WHERE plugin_id = ?1",
                params![plugin_id, status, now, decided_by],
            )?
        };
        Ok(changed > 0)
    }

    pub fn add_plugin_audit(&self, plugin_id: &str, actor: &str, action: &str, detail_json: &str) {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn();
        let _ = conn.execute(
            "INSERT INTO plugin_audit (ts_ms, plugin_id, actor, action, detail_json) VALUES (?1,?2,?3,?4,?5)",
            params![now, plugin_id, actor, action, detail_json],
        );
    }

    pub fn list_plugin_audit(&self, limit: i64) -> Result<Vec<PluginAuditRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, ts_ms, plugin_id, actor, action, detail_json FROM plugin_audit ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.clamp(1, 1000)], |row| {
            Ok(PluginAuditRow {
                id: row.get(0)?,
                ts_ms: row.get(1)?,
                plugin_id: row.get(2)?,
                actor: row.get(3)?,
                action: row.get(4)?,
                detail_json: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    /// Keeps the audit table bounded; called at startup.
    pub fn prune_plugin_audit(&self, keep: i64) {
        let conn = self.conn();
        let _ = conn.execute(
            "DELETE FROM plugin_audit WHERE id <= (SELECT COALESCE(MAX(id), 0) FROM plugin_audit) - ?1",
            params![keep],
        );
    }
}
