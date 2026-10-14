//! Plugin manifests and the permission model. See docs/PLUGIN_PERMISSIONS.md.
//!
//! Honest scope: for a *native* plugin (an in-process library) the host can only
//! enforce the surface it mediates -- which hooks it calls and what it returns. The
//! code itself runs with the server's full privileges, so approving a native plugin
//! is a trust decision, and `native.code` is always listed as such.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

pub const MANIFEST_VERSION: u32 = 1;

/// Safety-critical commands no plugin may veto, matched on the parsed variant name.
pub const SAFETY_CRITICAL_COMMANDS: [&str; 4] = ["ToggleBlackout", "ToggleClearText", "ToggleLogo", "SetAlert"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Low,
    Medium,
    High,
    Critical,
}

/// `(id, base tier, enforcement)`; `enforcement` is shown in the console so the operator
/// can tell what the host actually polices.
const KNOWN: &[(&str, Tier, &str)] = &[
    ("native.code", Tier::Critical, "informational"),
    ("ui.dom", Tier::Critical, "consent-only"),
    ("show.observe", Tier::Low, "enforced"),
    ("library.read", Tier::Low, "informational"),
    ("resources.provide", Tier::Medium, "enforced"),
    ("library.write", Tier::High, "reserved"),
    ("commands.send", Tier::Medium, "reserved"),
    ("commands.intercept", Tier::High, "enforced"),
    ("keyring.own", Tier::Medium, "enforced"),
    ("settings.read", Tier::Medium, "reserved"),
    ("settings.write", Tier::High, "reserved"),
    ("network.outbound", Tier::Medium, "consent-only"),
    ("network.listen", Tier::High, "consent-only"),
    ("filesystem.plugin_data", Tier::Low, "consent-only"),
    ("filesystem.read", Tier::High, "consent-only"),
    ("filesystem.write", Tier::Critical, "consent-only"),
    ("process.spawn", Tier::Critical, "consent-only"),
    ("ui.panel", Tier::Medium, "reserved"),
    ("ui.toolbar", Tier::Medium, "reserved"),
];

/// Navigation-only commands: a `commands.send` limited to these stays Medium.
const NAVIGATION_COMMANDS: [&str; 8] =
    ["NextSlide", "PrevSlide", "NextItem", "PrevItem", "JumpSlide", "JumpSection", "GoLive", "StageItem"];

const RESOURCE_CATEGORIES: [&str; 5] = ["songs", "scriptures", "media", "presentations", "themes"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PermissionRequest {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The operator may decline an optional permission and still approve the plugin.
    #[serde(default)]
    pub optional: bool,
}

impl PermissionRequest {
    pub fn simple(id: &str) -> Self {
        Self { id: id.to_string(), ..Default::default() }
    }

    pub fn with_categories(id: &str, cats: &[&str]) -> Self {
        Self { id: id.to_string(), categories: cats.iter().map(|c| c.to_string()).collect(), ..Default::default() }
    }

    pub fn tier(&self) -> Tier {
        let base = KNOWN.iter().find(|(id, _, _)| *id == self.id).map(|(_, t, _)| *t).unwrap_or(Tier::Critical);
        if self.id == "commands.send" {
            let navigation_only = !self.commands.is_empty()
                && self.commands.iter().all(|c| NAVIGATION_COMMANDS.contains(&c.as_str()));
            return if navigation_only { Tier::Medium } else { Tier::High };
        }
        base
    }

    pub fn enforcement(&self) -> &'static str {
        KNOWN.iter().find(|(id, _, _)| *id == self.id).map(|(_, _, e)| *e).unwrap_or("unknown")
    }
}

/// What a plugin has actually been granted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet(pub Vec<PermissionRequest>);

impl PermissionSet {
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Everything the v1 plugin trait can do -- granted to plugins registered
    /// programmatically in-process and to grandfathered legacy libraries.
    pub fn legacy_full() -> Self {
        Self(legacy_requests())
    }

    pub fn has(&self, id: &str) -> bool {
        self.0.iter().any(|p| p.id == id)
    }

    pub fn provides(&self, category: &str) -> bool {
        self.0
            .iter()
            .any(|p| p.id == "resources.provide" && p.categories.iter().any(|c| c == "*" || c == category))
    }

    pub fn allows_command(&self, variant: &str) -> bool {
        self.0
            .iter()
            .any(|p| p.id == "commands.send" && p.commands.iter().any(|c| c == "*" || c == variant))
    }
}

/// Permissions synthesized for a library with no manifest (everything the v1 hooks can do).
pub fn legacy_requests() -> Vec<PermissionRequest> {
    vec![
        PermissionRequest::simple("native.code"),
        PermissionRequest::simple("show.observe"),
        PermissionRequest::simple("commands.intercept"),
        PermissionRequest::with_categories("resources.provide", &["*"]),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackendSection {
    #[serde(default)]
    pub abi: u32,
    /// Keyed by `"{os}-{arch}"`, e.g. `linux-x86_64`, `windows-x86_64`, `macos-aarch64`.
    #[serde(default)]
    pub libraries: std::collections::BTreeMap<String, LibraryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSection {
    /// File name of the plugin's ES module inside `web/plugins/`.
    pub entry: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub description: String,
    /// semver requirement against the running OS-Next version, e.g. ">=0.2.0, <0.3.0".
    #[serde(default)]
    pub host_api: String,
    #[serde(default)]
    pub backend: Option<BackendSection>,
    #[serde(default)]
    pub web: Option<WebSection>,
    #[serde(default)]
    pub permissions: Vec<PermissionRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// Manifest asks for a newer manifest format than this build understands.
    Incompatible(String),
    Invalid(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Incompatible(m) | ManifestError::Invalid(m) => f.write_str(m),
        }
    }
}

pub fn valid_plugin_id(id: &str) -> bool {
    let b = id.as_bytes();
    (2..=64).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
}

/// A path from a manifest must stay inside the plugin's directory.
pub fn relative_path_is_contained(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && !path.is_absolute()
        && path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

impl PluginManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        // Check the format version first so a *newer* manifest that this build can't
        // fully parse is reported as incompatible, not as garbage.
        let raw: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| ManifestError::Invalid(format!("plugin.json is not valid JSON: {e}")))?;
        let ver = raw.get("manifest_version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        if ver > MANIFEST_VERSION {
            return Err(ManifestError::Incompatible(format!(
                "plugin.json uses manifest_version {ver}; this OS-Next understands up to {MANIFEST_VERSION}"
            )));
        }
        serde_json::from_value(raw).map_err(|e| ManifestError::Invalid(format!("plugin.json: {e}")))
    }

    pub fn validate(&self, host_version: &str) -> Result<(), ManifestError> {
        let bad = |m: String| Err(ManifestError::Invalid(m));
        if self.manifest_version != MANIFEST_VERSION {
            return bad(format!("manifest_version must be {MANIFEST_VERSION}"));
        }
        if !valid_plugin_id(&self.id) {
            return bad(format!("invalid plugin id '{}'", self.id));
        }
        if self.name.trim().is_empty() {
            return bad("name must not be empty".to_string());
        }
        if !self.host_api.trim().is_empty() {
            let req = semver::VersionReq::parse(self.host_api.trim())
                .map_err(|e| ManifestError::Invalid(format!("host_api '{}': {e}", self.host_api)))?;
            let host = semver::Version::parse(host_version)
                .map_err(|e| ManifestError::Invalid(format!("host version '{host_version}': {e}")))?;
            // Compare ignoring any pre-release tag, so a dev build satisfies ">=0.2.0".
            let host_release = semver::Version::new(host.major, host.minor, host.patch);
            if !req.matches(&host_release) {
                return Err(ManifestError::Incompatible(format!(
                    "plugin needs OS-Next {}, this is {}",
                    self.host_api, host_version
                )));
            }
        }
        if self.backend.is_none() && self.web.is_none() {
            return bad("a plugin needs a backend and/or a web section".to_string());
        }
        if let Some(b) = &self.backend {
            for (platform, lib) in &b.libraries {
                if !relative_path_is_contained(&lib.path) {
                    return bad(format!("library path for {platform} escapes the plugin directory: {}", lib.path));
                }
                if lib.sha256.len() != 64 || !lib.sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
                    return bad(format!("library {platform} needs a 64-hex-digit sha256"));
                }
            }
        }
        if let Some(w) = &self.web {
            let e = w.entry.as_str();
            if e.is_empty() || e.contains('/') || e.contains('\\') || !e.ends_with(".js") {
                return bad("web.entry must be a plain .js file name".to_string());
            }
        }
        let mut seen = std::collections::HashSet::new();
        for p in &self.permissions {
            if !KNOWN.iter().any(|(id, _, _)| *id == p.id) {
                // Fail closed: an unrecognised permission is never silently ignored.
                return bad(format!("unknown permission '{}'", p.id));
            }
            if !seen.insert(p.id.clone()) {
                return bad(format!("permission '{}' is listed twice", p.id));
            }
            if p.id == "resources.provide" || p.id == "library.read" || p.id == "library.write" {
                if p.categories.is_empty() {
                    return bad(format!("'{}' needs a categories list", p.id));
                }
                if let Some(c) = p.categories.iter().find(|c| *c != "*" && !RESOURCE_CATEGORIES.contains(&c.as_str())) {
                    return bad(format!("'{}': unknown category '{c}'", p.id));
                }
            }
            if p.id == "commands.send" && p.commands.is_empty() {
                return bad("'commands.send' needs a commands list".to_string());
            }
        }
        Ok(())
    }
}

/// The `"{os}-{arch}"` key this build looks up in `backend.libraries`.
pub fn platform_key() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    d.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    Ok(sha256_hex(&std::fs::read(path)?))
}

/// SHA-256 over the sorted `"<relpath>\0<sha256>\n"` lines of every pinned file, so
/// changing, adding or removing any one of them changes the digest.
pub fn artifact_digest(files: &[(String, String)]) -> String {
    let mut lines: Vec<String> = files.iter().map(|(rel, sha)| format!("{rel}\0{sha}\n")).collect();
    lines.sort();
    sha256_hex(lines.concat().as_bytes())
}

/// The command's variant name from any of the shapes it travels in: a bare
/// `"NextSlide"` string, `{"GoLive": {...}}`, or a `{"cmd": ...}` envelope; or, for raw
/// non-JSON text, the text itself if it is a plain identifier.
pub fn command_variant(text: &str) -> Option<String> {
    let trimmed = text.trim();
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(v) => variant_of_value(&v),
        Err(_) => {
            let ident = !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            ident.then(|| trimmed.to_string())
        }
    }
}

fn variant_of_value(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(map) => {
            if let Some(inner) = map.get("cmd") {
                return variant_of_value(inner);
            }
            if map.len() == 1 {
                return map.keys().next().cloned();
            }
            None
        }
        _ => None,
    }
}

/// True for the emergency commands. Decided on the parsed variant; only when the text
/// can't be parsed at all does it fall back to the old substring match, so an
/// unparseable payload can never turn a real emergency command into a vetoable one.
pub fn is_safety_critical(text: &str) -> bool {
    match command_variant(text) {
        Some(v) => SAFETY_CRITICAL_COMMANDS.contains(&v.as_str()),
        None => SAFETY_CRITICAL_COMMANDS.iter().any(|c| text.contains(c)),
    }
}

pub fn contained_join(base: &Path, rel: &str) -> Option<PathBuf> {
    relative_path_is_contained(rel).then(|| base.join(rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> Result<PluginManifest, ManifestError> {
        let m = PluginManifest::parse(json.as_bytes())?;
        m.validate("0.2.0")?;
        Ok(m)
    }

    const GOOD: &str = r#"{
        "manifest_version": 1, "id": "org.example.dmx", "name": "DMX", "version": "1.0.0",
        "host_api": ">=0.1.0, <1.0.0",
        "backend": {"abi": 1, "libraries": {"linux-x86_64": {"path": "lib/x.so", "sha256": "0000000000000000000000000000000000000000000000000000000000000000"}}},
        "permissions": [
            {"id": "show.observe", "reason": "follow blackout"},
            {"id": "commands.send", "commands": ["NextSlide"], "optional": true},
            {"id": "resources.provide", "categories": ["songs"]}
        ]
    }"#;

    #[test]
    fn accepts_a_good_manifest() {
        let m = manifest(GOOD).unwrap();
        assert_eq!(m.permissions.len(), 3);
        assert_eq!(m.permissions[1].tier(), Tier::Medium); // navigation-only
    }

    #[test]
    fn rejects_bad_ids_unknown_permissions_and_escaping_paths() {
        assert!(manifest(&GOOD.replace("org.example.dmx", "Bad ID!")).is_err());
        assert!(manifest(&GOOD.replace("show.observe", "show.everything")).is_err());
        assert!(manifest(&GOOD.replace("lib/x.so", "../x.so")).is_err());
        assert!(manifest(&GOOD.replace("lib/x.so", "/etc/x.so")).is_err());
        assert!(manifest(&GOOD.replace("0000000000000000000000000000000000000000000000000000000000000000", "abc")).is_err());
        // Duplicate permission ids and missing parameters.
        assert!(manifest(&GOOD.replace(r#"{"id": "show.observe", "reason": "follow blackout"}"#, r#"{"id": "resources.provide", "categories": ["media"]}"#)).is_err());
        assert!(manifest(&GOOD.replace(r#""categories": ["songs"]"#, r#""categories": []"#)).is_err());
        assert!(manifest(&GOOD.replace(r#""commands": ["NextSlide"]"#, r#""commands": []"#)).is_err());
    }

    #[test]
    fn newer_manifest_versions_and_host_mismatch_are_incompatible() {
        let newer = GOOD.replace(r#""manifest_version": 1"#, r#""manifest_version": 2"#);
        assert!(matches!(PluginManifest::parse(newer.as_bytes()), Err(ManifestError::Incompatible(_))));
        let too_new_host = GOOD.replace(">=0.1.0, <1.0.0", ">=5.0.0");
        assert!(matches!(manifest(&too_new_host), Err(ManifestError::Incompatible(_))));
    }

    #[test]
    fn digest_is_order_independent_and_sensitive_to_every_file() {
        let a = vec![("lib/a.so".to_string(), "11".to_string()), ("web/b.js".to_string(), "22".to_string())];
        let b = vec![a[1].clone(), a[0].clone()];
        assert_eq!(artifact_digest(&a), artifact_digest(&b));
        let c = vec![("lib/a.so".to_string(), "11".to_string()), ("web/b.js".to_string(), "23".to_string())];
        assert_ne!(artifact_digest(&a), artifact_digest(&c));
        assert_ne!(artifact_digest(&a), artifact_digest(&a[..1]));
    }

    #[test]
    fn permission_set_checks() {
        let set = PermissionSet(vec![
            PermissionRequest { id: "commands.send".into(), commands: vec!["NextSlide".into()], ..Default::default() },
            PermissionRequest::with_categories("resources.provide", &["songs"]),
        ]);
        assert!(set.allows_command("NextSlide"));
        assert!(!set.allows_command("NewSchedule"));
        assert!(set.provides("songs"));
        assert!(!set.provides("themes"));
        assert!(!set.has("show.observe"));
        assert!(PermissionSet::legacy_full().provides("themes"));
        let wild = PermissionSet(vec![PermissionRequest { id: "commands.send".into(), commands: vec!["*".into()], ..Default::default() }]);
        assert_eq!(wild.0[0].tier(), Tier::High);
        assert!(wild.allows_command("Anything"));
    }

    #[test]
    fn command_variant_handles_every_shape() {
        assert_eq!(command_variant(r#""NextSlide""#).as_deref(), Some("NextSlide"));
        assert_eq!(command_variant("NextSlide").as_deref(), Some("NextSlide"));
        assert_eq!(command_variant(r#"{"GoLive":{"item_index":1}}"#).as_deref(), Some("GoLive"));
        assert_eq!(command_variant(r#"{"cmd":{"SetAlert":{"text":"x"}},"device_token":"t"}"#).as_deref(), Some("SetAlert"));
        assert_eq!(command_variant("{not json"), None);
    }

    #[test]
    fn safety_critical_is_decided_on_the_variant_not_a_substring() {
        assert!(is_safety_critical(r#""ToggleBlackout""#));
        assert!(is_safety_critical("ToggleClearText"));
        assert!(is_safety_critical(r#"{"SetAlert":{"text":"fire"}}"#));
        // A song that merely *mentions* an emergency command is an ordinary command.
        assert!(!is_safety_critical(r#"{"AddToSchedule":{"title":"SetAlert ToggleBlackout"}}"#));
        // Unparseable text keeps the old, conservative behaviour.
        assert!(is_safety_critical("{broken ToggleLogo"));
    }
}
