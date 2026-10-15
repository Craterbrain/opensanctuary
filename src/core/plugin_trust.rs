//! How much the operator has reason to trust a plugin's publisher (docs/PLUGIN_SIGNING.md, section 2).
//!
//! This is display and policy only. A trust level never widens what a plugin may do, never skips the
//! review screen, and never approves anything.

use std::collections::HashSet;

use crate::storage::db::{PluginGrantRow, PublisherRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    /// Ships with the app.
    Builtin,
    /// Signed by a key the operator chose to trust.
    Trusted,
    /// Signed by a key the operator has approved a plugin from before, but not explicitly trusted.
    Known,
    /// Signed by a key never seen before.
    New,
    Unsigned,
    /// A signature is present and doesn't verify, or the package doesn't match its signed manifest.
    Invalid,
    /// Signed by a key the operator revoked.
    Revoked,
}

impl TrustLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            TrustLevel::Builtin => "builtin",
            TrustLevel::Trusted => "trusted",
            TrustLevel::Known => "known",
            TrustLevel::New => "new",
            TrustLevel::Unsigned => "unsigned",
            TrustLevel::Invalid => "invalid",
            TrustLevel::Revoked => "revoked",
        }
    }
}

/// Did discovery refuse this plugin because of its signature (as opposed to some other manifest problem)?
pub fn is_signature_error(message: &str) -> bool {
    message.starts_with("plugin.json signature")
        || message.starts_with("plugin.json.minisig is present")
        || message.starts_with("this signed package doesn't match its manifest")
}

/// `known_keys` are the publisher keys of plugins the operator has approved (any plugin).
pub fn trust_level(row: &PluginGrantRow, builtin: bool, publisher: Option<&PublisherRow>, known_keys: &HashSet<String>) -> TrustLevel {
    if row.error.as_deref().is_some_and(is_signature_error) {
        return TrustLevel::Invalid;
    }
    if builtin {
        return TrustLevel::Builtin;
    }
    let Some(key) = row.observed_signer.as_deref() else { return TrustLevel::Unsigned };
    match publisher {
        Some(p) if p.revoked => TrustLevel::Revoked,
        Some(p) if p.trusted => TrustLevel::Trusted,
        _ if known_keys.contains(key) => TrustLevel::Known,
        _ => TrustLevel::New,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(signer: Option<&str>, error: Option<&str>) -> PluginGrantRow {
        PluginGrantRow {
            plugin_id: "p".into(), kind: "sandbox".into(), name: "P".into(), version: None, source_path: "x".into(),
            observed_manifest_sha256: None, observed_digest: "d".into(), observed_signer: signer.map(String::from), approved_signer: None,
            requested_json: "[]".into(), error: error.map(String::from), approved_manifest_sha256: None, approved_digest: None,
            status: "pending".into(), granted_json: "[]".into(), decided_at: None, decided_by: None, first_seen: 0, last_seen: 0,
            observed_files_json: None, approved_files_json: None, approved_version: None, approved_requested_json: None,
            observed_signature_scope: None, approved_signature_scope: None,
        }
    }
    fn publisher(trusted: bool, revoked: bool) -> PublisherRow {
        PublisherRow { key: "K".into(), name: "N".into(), trusted, revoked, added_at: 0, added_by: "t".into() }
    }

    #[test]
    fn levels_follow_the_order_invalid_builtin_revoked_trusted_known_new_unsigned() {
        let none = HashSet::new();
        let known: HashSet<String> = ["K".to_string()].into();
        assert_eq!(trust_level(&row(None, None), false, None, &none), TrustLevel::Unsigned);
        assert_eq!(trust_level(&row(Some("K"), None), false, None, &none), TrustLevel::New);
        assert_eq!(trust_level(&row(Some("K"), None), false, None, &known), TrustLevel::Known);
        assert_eq!(trust_level(&row(Some("K"), None), false, Some(&publisher(true, false)), &known), TrustLevel::Trusted);
        // Revoked beats trusted and known.
        assert_eq!(trust_level(&row(Some("K"), None), false, Some(&publisher(true, true)), &known), TrustLevel::Revoked);
        // The app's own plugins are built-in whatever the table says, but a bad signature is never hidden.
        assert_eq!(trust_level(&row(None, None), true, None, &none), TrustLevel::Builtin);
        assert_eq!(trust_level(&row(Some("K"), Some("plugin.json signature: bad")), true, None, &none), TrustLevel::Invalid);
        // An unrelated load error isn't a signature problem.
        assert_eq!(trust_level(&row(None, Some("entry page 'x' is missing")), false, None, &none), TrustLevel::Unsigned);
    }

    #[test]
    fn signature_errors_are_recognised() {
        for e in ["plugin.json signature: Signature verification failed", "plugin.json.minisig is present but plugin.json has no publisher_key", "this signed package doesn't match its manifest: not listed in files: x"] {
            assert!(is_signature_error(e), "{e}");
        }
        assert!(!is_signature_error("the files don't match the hashes in plugin.json: x"));
    }
}
