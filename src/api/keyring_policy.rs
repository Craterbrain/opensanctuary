//! Who may touch the shared keyring namespaces (`OpenSanctuary:Shared:<name>`, the
//! `keyring.service` permission). One policy, used by the HTTP keyring routes and by the native
//! plugin host API, so the two can't drift apart.
//!
//! Ownership lives in `keyring_metadata.owner`; rows in a shared service are tagged
//! `shared:plugin:<id>` (a plugin created it) or `shared:host` (the console did).
//! * **read** (`get`, `has`): the plugin needs read or write access to that exact service *and* the
//!   row's owner must start with `shared:`. A row with any other owner (`host`, `plugin:x`, or none
//!   recorded) is reported as not found, so a secret stored under a shared-looking name before this
//!   existed is never exposed.
//! * **write** (`set`): needs `write` access, and the row must not exist or be the plugin's own, so a
//!   co-sharer can't swap another's key.
//! * **delete**: needs `write` access and the row must be the plugin's own.
//! * the console may read, set and delete every shared row (the operator's way to fix squatting).

use std::collections::HashSet;
use std::sync::Mutex;

use crate::core::plugin_permissions::{valid_shared_service, Access, PermissionSet, SHARED_SERVICE_PREFIX};
use crate::storage::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    Get,
    Has,
    Set,
    Delete,
}

impl Op {
    pub fn name(self) -> &'static str {
        match self {
            Op::Get => "get",
            Op::Has => "has",
            Op::Set => "set",
            Op::Delete => "delete",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Go ahead. For a write, `owner` is the tag to record.
    Allow { owner: String },
    /// A read of something this plugin may not see: answer "not found", never "forbidden".
    NotFound,
    Deny(String),
}

/// Anything that looks like it is in the shared area, valid or not (so a typo'd name is refused as
/// a shared name rather than falling through to another rule).
pub fn in_shared_area(service: &str) -> bool {
    service.len() >= SHARED_SERVICE_PREFIX.len() && service[..SHARED_SERVICE_PREFIX.len()].eq_ignore_ascii_case(SHARED_SERVICE_PREFIX)
}

pub fn plugin_owner_tag(plugin_id: &str) -> String {
    format!("shared:plugin:{plugin_id}")
}

pub const HOST_OWNER_TAG: &str = "shared:host";

/// Decides one plugin operation on a shared service against the plugin's (live) grant.
pub fn check(db: &Database, plugin_id: &str, grant: &PermissionSet, op: Op, service: &str, account: &str) -> Verdict {
    if !valid_shared_service(service) {
        return match op {
            Op::Has | Op::Get => Verdict::NotFound,
            _ => Verdict::Deny(format!("'{service}' isn't a shareable keyring namespace")),
        };
    }
    let Some(access) = grant.shared_access(service) else {
        return match op {
            Op::Has | Op::Get => Verdict::NotFound,
            _ => Verdict::Deny("this plugin wasn't granted that shared keyring namespace".to_string()),
        };
    };
    let own = plugin_owner_tag(plugin_id);
    let owner = db.get_keyring_owner(service, account).unwrap_or(None);
    match op {
        Op::Get | Op::Has => {
            if owner.as_deref().is_some_and(|o| o.starts_with("shared:")) {
                Verdict::Allow { owner: own }
            } else {
                Verdict::NotFound
            }
        }
        Op::Set => {
            if access != Access::Write {
                return Verdict::Deny("this plugin only has read access to that shared keyring namespace".to_string());
            }
            match owner {
                None => Verdict::Allow { owner: own },
                Some(o) if o == own => Verdict::Allow { owner: own },
                Some(_) => Verdict::Deny("that secret is owned by another plugin or the console".to_string()),
            }
        }
        Op::Delete => {
            if access != Access::Write {
                return Verdict::Deny("this plugin only has read access to that shared keyring namespace".to_string());
            }
            if owner.as_deref() == Some(own.as_str()) {
                Verdict::Allow { owner: own }
            } else {
                Verdict::Deny("a plugin can only delete the shared secrets it created".to_string())
            }
        }
    }
}

static SEEN: Mutex<Option<HashSet<(String, String, String, &'static str, bool)>>> = Mutex::new(None);

/// Audits the first use (or first refusal) per plugin, service, account and operation per run, so a
/// busy plugin can't flood the log. Never records the secret.
pub fn audit_once(db: &Database, plugin_id: &str, service: &str, account: &str, op: Op, denied: bool) {
    let first = {
        let mut g = SEEN.lock().unwrap_or_else(|e| e.into_inner());
        let set = g.get_or_insert_with(Default::default);
        set.len() < 5000 && set.insert((plugin_id.to_string(), service.to_string(), account.to_string(), op.name(), denied))
    };
    if first {
        db.add_plugin_audit(
            plugin_id,
            &format!("plugin:{plugin_id}"),
            if denied { "denied_call" } else { "keyring_shared_access" },
            &serde_json::json!({ "service": service, "account": account, "op": op.name() }).to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::plugin_permissions::PermissionRequest;

    fn grant(services: &[&str], access: &str) -> PermissionSet {
        PermissionSet(vec![PermissionRequest {
            id: "keyring.service".into(),
            services: services.iter().map(|s| s.to_string()).collect(),
            access: Some(access.into()),
            ..Default::default()
        }])
    }

    fn db() -> (tempfile::TempDir, Database) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::new(tmp.path().join("k.db").to_str().unwrap()).unwrap();
        (tmp, db)
    }

    const S: &str = "OpenSanctuary:Shared:pexels";

    #[test]
    fn read_needs_a_grant_and_a_shared_owner() {
        let (_t, db) = db();
        let g = grant(&[S], "read");
        // Nothing stored: not found. A row owned by the host or another plugin's private tag: not found.
        assert_eq!(check(&db, "a", &g, Op::Get, S, "k"), Verdict::NotFound);
        db.record_keyring_owner(S, "k", "host").unwrap();
        assert_eq!(check(&db, "a", &g, Op::Get, S, "k"), Verdict::NotFound);
        db.record_keyring_owner(S, "k", "plugin:b").unwrap();
        assert_eq!(check(&db, "a", &g, Op::Has, S, "k"), Verdict::NotFound);
        // A shared row, from the console or a plugin, is readable.
        db.record_keyring_owner(S, "k", HOST_OWNER_TAG).unwrap();
        assert!(matches!(check(&db, "a", &g, Op::Get, S, "k"), Verdict::Allow { .. }));
        db.record_keyring_owner(S, "k", &plugin_owner_tag("b")).unwrap();
        assert!(matches!(check(&db, "a", &g, Op::Get, S, "k"), Verdict::Allow { .. }));
        // No grant for that service, or none at all: not found.
        assert_eq!(check(&db, "a", &grant(&["OpenSanctuary:Shared:other"], "read"), Op::Get, S, "k"), Verdict::NotFound);
        assert_eq!(check(&db, "a", &PermissionSet::empty(), Op::Get, S, "k"), Verdict::NotFound);
    }

    #[test]
    fn writes_need_write_access_and_never_touch_anothers_rows() {
        let (_t, db) = db();
        let r = grant(&[S], "read");
        let w = grant(&[S], "write");
        assert!(matches!(check(&db, "a", &r, Op::Set, S, "k"), Verdict::Deny(_)), "read can't set");
        assert!(matches!(check(&db, "a", &r, Op::Delete, S, "k"), Verdict::Deny(_)), "read can't delete");
        assert_eq!(check(&db, "a", &w, Op::Set, S, "k"), Verdict::Allow { owner: "shared:plugin:a".into() });
        db.record_keyring_owner(S, "k", &plugin_owner_tag("a")).unwrap();
        assert!(matches!(check(&db, "a", &w, Op::Set, S, "k"), Verdict::Allow { .. }), "its own row");
        assert!(matches!(check(&db, "a", &w, Op::Delete, S, "k"), Verdict::Allow { .. }));
        // Another sharer's row, the console's row, and a pre-existing private row: untouchable.
        for owner in [plugin_owner_tag("b"), HOST_OWNER_TAG.to_string(), "host".to_string(), "plugin:b".to_string()] {
            db.record_keyring_owner(S, "k", &owner).unwrap();
            assert!(matches!(check(&db, "a", &w, Op::Set, S, "k"), Verdict::Deny(_)), "set over {owner}");
            assert!(matches!(check(&db, "a", &w, Op::Delete, S, "k"), Verdict::Deny(_)), "delete {owner}");
        }
    }

    #[test]
    fn shapes_outside_the_shared_area_are_refused() {
        let (_t, db) = db();
        let g = grant(&[S], "write");
        for bad in ["OpenSanctuary:System", "OpenSanctuary:Plugin:a", "OpenSanctuary:CCLI", "opensanctuary:shared:pexels", "OpenSanctuary:Shared:", "OpenSanctuary:Shared:Pexels", "OpenSanctuary:Shared:a:b", "OpenSanctuary:Shared:.x", " OpenSanctuary:Shared:pexels"] {
            assert!(matches!(check(&db, "a", &g, Op::Set, bad, "k"), Verdict::Deny(_)), "{bad}");
            assert_eq!(check(&db, "a", &g, Op::Get, bad, "k"), Verdict::NotFound, "{bad}");
        }
        assert!(in_shared_area("opensanctuary:SHARED:x") && !in_shared_area("OpenSanctuary:Plugin:x"));
    }
}
