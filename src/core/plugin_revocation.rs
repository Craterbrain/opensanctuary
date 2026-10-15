//! The plugin revocation list (docs/PLUGIN_SIGNING.md, section 5): a small file, signed by the release key,
//! that names publisher keys the app must no longer trust. It ships with the app (`web/revoked_keys.json` and
//! its `.minisig`) and a newer one can be dropped in the data directory.
//!
//! A list only counts if its signature verifies against the release key. A higher `sequence` replaces a lower
//! one and a lower one is never applied after a higher one was (so an old list can't un-revoke anything).
//! The release key itself can't be revoked by a list.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::storage::Database;

/// Setting that remembers the highest sequence ever applied.
const SEQUENCE_SETTING: &str = "plugin_revocation_sequence";

#[derive(Debug, Clone, Deserialize)]
pub struct RevokedKey {
    pub key: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub since: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RevocationList {
    pub sequence: u64,
    #[serde(default)]
    pub revoked: Vec<RevokedKey>,
}

/// What applying a list did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Applied {
    pub sequence: u64,
    /// Keys newly marked revoked.
    pub revoked_keys: Vec<String>,
    /// Plugins that were running and were stopped.
    pub stopped: Vec<String>,
}

/// Where lists are looked for: the one shipped with the app, and a newer one in the data directory.
pub fn list_paths(web_dir: &Path, data_dir: &Path) -> Vec<PathBuf> {
    vec![web_dir.join("revoked_keys.json"), data_dir.join("revoked_keys.json")]
}

/// Reads one list and verifies its detached signature (`<path>.minisig`) against `release_key`.
pub fn load_verified(path: &Path, release_key: &str) -> Result<RevocationList, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() > 256 * 1024 {
        return Err(format!("{} is too large", path.display()));
    }
    let sig_path = PathBuf::from(format!("{}.minisig", path.display()));
    let sig = std::fs::read_to_string(&sig_path).map_err(|_| format!("{} has no signature ({})", path.display(), sig_path.display()))?;
    crate::network::updater::verify_signature(&bytes, &sig, release_key).map_err(|e| format!("{}: {e}", path.display()))?;
    let list: RevocationList = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if list.revoked.len() > 5000 {
        return Err(format!("{} lists too many keys", path.display()));
    }
    Ok(list)
}

/// The valid list with the highest sequence among `paths` (missing files are simply absent), plus a note for every
/// file that exists but can't be used.
pub fn newest_valid(paths: &[PathBuf], release_key: &str) -> (Option<RevocationList>, Vec<String>) {
    let mut best: Option<RevocationList> = None;
    let mut problems = Vec::new();
    for p in paths.iter().filter(|p| p.is_file()) {
        match load_verified(p, release_key) {
            Ok(l) if best.as_ref().is_none_or(|b| l.sequence > b.sequence) => best = Some(l),
            Ok(_) => {}
            Err(e) => problems.push(e),
        }
    }
    (best, problems)
}

/// Applies the newest valid list if it is newer than anything applied before: revokes each key and stops the
/// approved plugins it signed. Idempotent, and a no-op when nothing valid or newer exists.
pub fn apply(db: &Database, release_key: &str, paths: &[PathBuf]) -> Option<Applied> {
    let (list, problems) = newest_valid(paths, release_key);
    for p in &problems {
        tracing::warn!("plugin revocation list ignored: {p}");
    }
    let list = list?;
    let applied_before: u64 = db.get_setting(SEQUENCE_SETTING).ok().flatten().and_then(|s| s.parse().ok()).unwrap_or(0);
    if list.sequence <= applied_before {
        return None; // already applied, or an older list (rollback): nothing to do
    }
    let mut out = Applied { sequence: list.sequence, ..Default::default() };
    for entry in &list.revoked {
        if entry.key == release_key || crate::core::plugin_trust::parse_public_key(&entry.key).is_none() {
            continue; // the release key can't be revoked by a list, and junk isn't a key
        }
        let note = format!("revocation list #{}: {}", list.sequence, entry.reason);
        let was_revoked = db.get_publisher(&entry.key).ok().flatten().is_some_and(|p| p.revoked);
        if db.revoke_publisher(&entry.key, "", "revocation list", &note).is_err() {
            continue;
        }
        if !was_revoked {
            out.revoked_keys.push(entry.key.clone());
        }
        for id in db.plugins_signed_by(&entry.key).unwrap_or_default() {
            let Some(row) = db.get_plugin_grant(&id).ok().flatten() else { continue };
            if row.status == "approved" && row.approved_signer.as_deref() == Some(entry.key.as_str()) {
                let _ = db.set_plugin_decision(&id, "revoked", "[]", None, None, None, "revocation list");
                db.add_plugin_audit(&id, "revocation list", "revoked", &serde_json::json!({ "reason": note, "publisher_key_id": crate::core::plugin_trust::key_id(&entry.key) }).to_string());
                out.stopped.push(id);
            }
        }
        db.add_plugin_audit("*", "revocation list", "publisher_revoked", &serde_json::json!({ "publisher_key_id": crate::core::plugin_trust::key_id(&entry.key), "reason": note }).to_string());
    }
    let _ = db.set_setting(SEQUENCE_SETTING, &list.sequence.to_string());
    Some(out)
}

/// The sequence applied so far (0 = none).
pub fn applied_sequence(db: &Database) -> u64 {
    db.get_setting(SEQUENCE_SETTING).ok().flatten().and_then(|s| s.parse().ok()).unwrap_or(0)
}
