//! Plugin manifests and the permission model. See docs/PLUGIN_PERMISSIONS.md.
//!
//! Honest scope: for a *native* plugin (an in-process library) the host can only
//! enforce the surface it mediates -- which hooks it calls and what it returns. The
//! code itself runs with the server's full privileges, so approving a native plugin
//! is a trust decision, and `native.code` is always listed as such.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

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
    ("library.read", Tier::Low, "enforced"),
    ("resources.provide", Tier::Medium, "enforced"),
    ("library.write", Tier::High, "enforced"),
    ("commands.send", Tier::Medium, "enforced"),
    ("commands.intercept", Tier::High, "enforced"),
    ("keyring.own", Tier::Medium, "enforced"),
    ("settings.read", Tier::Medium, "reserved"),
    ("settings.write", Tier::High, "reserved"),
    ("network.outbound", Tier::Medium, "enforced"),
    ("network.listen", Tier::High, "consent-only"),
    ("filesystem.plugin_data", Tier::Low, "enforced"),
    ("filesystem.read", Tier::High, "consent-only"),
    ("filesystem.write", Tier::Critical, "consent-only"),
    ("process.spawn", Tier::Critical, "consent-only"),
    ("process.run", Tier::Critical, "enforced"),
    ("ui.panel", Tier::Medium, "enforced"),
    ("ui.toolbar", Tier::Medium, "enforced"),
    ("notifications.post", Tier::Low, "enforced"),
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

    /// What the host actually polices for this permission, by plugin type. A native library
    /// shares the server's address space, so only the hooks the host mediates are enforced; a
    /// same-realm console script shares the page. A sandboxed page is enforced end to end.
    pub fn enforcement_for(&self, kind: &str) -> &'static str {
        let base = KNOWN.iter().find(|(id, _, _)| *id == self.id).map(|(_, _, e)| *e).unwrap_or("unknown");
        match kind {
            "native" => match self.id.as_str() {
                "show.observe" | "commands.intercept" | "resources.provide" => "enforced",
                "native.code" => "informational",
                _ => "consent-only",
            },
            "web" => match self.id.as_str() {
                "keyring.own" | "commands.send" => "enforced",
                "ui.dom" => "consent-only",
                _ => if base == "enforced" { "consent-only" } else { base },
            },
            _ => base,
        }
    }
}

/// What a plugin has actually been granted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet(pub Vec<PermissionRequest>);

impl PermissionSet {
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Everything the `OsPlugin` hooks can do -- granted to plugins registered programmatically
    /// in-process (host code and tests). Anything loaded from disk goes through the grant flow.
    pub fn in_process() -> Self {
        Self(in_process_requests())
    }

    pub fn has(&self, id: &str) -> bool {
        self.0.iter().any(|p| p.id == id)
    }

    pub fn reads(&self, category: &str) -> bool {
        self.0
            .iter()
            .any(|p| p.id == "library.read" && p.categories.iter().any(|c| c == "*" || c == category))
    }

    pub fn writes(&self, category: &str) -> bool {
        self.0
            .iter()
            .any(|p| p.id == "library.write" && p.categories.iter().any(|c| c == "*" || c == category))
    }

    /// The granted outbound hosts (`network.outbound`), across every entry.
    pub fn outbound_hosts(&self) -> Vec<String> {
        self.0.iter().filter(|p| p.id == "network.outbound").flat_map(|p| p.hosts.clone()).collect()
    }

    pub fn provides(&self, category: &str) -> bool {
        self.0
            .iter()
            .any(|p| p.id == "resources.provide" && p.categories.iter().any(|c| c == "*" || c == category))
    }

    /// Keeps only what `grant` also allows, narrowing parameter lists. A token's scopes are
    /// intersected with the plugin's *current* grant on every use, so shrinking or revoking the
    /// grant takes effect on tokens already in circulation.
    pub fn narrowed_by(&self, grant: &PermissionSet) -> PermissionSet {
        fn narrow(wanted: &[String], allowed: &[String]) -> Vec<String> {
            let allowed_all = allowed.iter().any(|a| a == "*");
            let wanted_all = wanted.iter().any(|w| w == "*");
            if allowed_all {
                wanted.to_vec()
            } else if wanted_all {
                allowed.to_vec()
            } else {
                wanted.iter().filter(|w| allowed.contains(w)).cloned().collect()
            }
        }
        let mut out = Vec::new();
        for want in &self.0 {
            let Some(have) = grant.0.iter().find(|g| g.id == want.id) else { continue };
            let mut p = want.clone();
            match want.id.as_str() {
                "commands.send" | "process.run" => {
                    p.commands = narrow(&want.commands, &have.commands);
                    if p.commands.is_empty() {
                        continue;
                    }
                }
                "network.outbound" => {
                    p.hosts = narrow(&want.hosts, &have.hosts);
                    if p.hosts.is_empty() {
                        continue;
                    }
                }
                "resources.provide" | "library.read" | "library.write" => {
                    p.categories = narrow(&want.categories, &have.categories);
                    if p.categories.is_empty() {
                        continue;
                    }
                }
                _ => {}
            }
            out.push(p);
        }
        PermissionSet(out)
    }

    /// Whether `process.run` covers the manifest command `id`.
    pub fn runs(&self, id: &str) -> bool {
        self.0.iter().any(|p| p.id == "process.run" && p.commands.iter().any(|c| c == "*" || c == id))
    }

    pub fn allows_command(&self, variant: &str) -> bool {
        self.0
            .iter()
            .any(|p| p.id == "commands.send" && p.commands.iter().any(|c| c == "*" || c == variant))
    }
}

/// Everything the `OsPlugin` hooks can do.
pub fn in_process_requests() -> Vec<PermissionRequest> {
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
    /// A same-realm plugin: file name of its ES module inside `web/plugins/`.
    /// A sandboxed plugin (`sandbox: true`): its HTML page inside `web/plugins/<id>/`.
    pub entry: String,
    /// Run in an iframe with an opaque origin and a per-plugin CSP, talking to the console only
    /// through a permission-checked message bridge. Needs no `ui.dom`.
    #[serde(default)]
    pub sandbox: bool,
}

/// Where a sandboxed plugin's page appears in the console.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Placement {
    /// `dock` (a small floating panel), `library-source` (an entry in a Resource Library tab's
    /// sidebar, shown in place of the list) or `video-picker` (the Videos tab of the background picker).
    pub slot: String,
    #[serde(default)]
    pub label: String,
    /// For `library-source`: `songs`, `scriptures`, `media`, `presentations` or `themes`.
    #[serde(default)]
    pub tab: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiSection {
    #[serde(default)]
    pub placements: Vec<Placement>,
    /// Buttons added to the console's top ribbon (`ui.toolbar`).
    #[serde(default)]
    pub toolbar: Vec<ToolbarItem>,
}

/// A ribbon button a sandboxed plugin declares. Everything here is shown to the operator at review
/// and can't change at runtime. `icon` is the *name* of an icon in the app's icon set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolbarItem {
    pub id: String,
    pub label: String,
    pub icon: String,
    #[serde(default)]
    pub tooltip: String,
}

/// Labels of the console's own ribbon buttons and their obvious synonyms: a plugin button can't
/// carry one (compared case-insensitively with whitespace collapsed).
const RESERVED_TOOLBAR_LABELS: &[&str] = &[
    "new", "open", "save", "import", "store", "web", "remote", "notices", "notifications", "go live", "live", "alert",
    "alerts", "logo", "black", "blackout", "clear", "clear text", "settings", "plugins",
];

/// Icons of the live-override cluster and the notification bell: not for plugin buttons.
const RESERVED_TOOLBAR_ICONS: &[&str] = &["golive", "blackout", "clear-lyrics", "logo", "live-status", "alert"];

pub const MAX_TOOLBAR_ITEMS: usize = 2;

/// Letters, digits, spaces and a little punctuation. No control, bidi-override, zero-width or emoji
/// characters, so a label can't imitate another or hide text.
fn toolbar_text_ok(s: &str, punctuation: &str) -> bool {
    s.chars().all(|c| c.is_alphanumeric() || c == ' ' || punctuation.contains(c))
}

impl ToolbarItem {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_short_id(&self.id) {
            return Err(format!("toolbar item id '{}' is not valid", self.id));
        }
        let label_chars = self.label.chars().count();
        if self.label != self.label.trim() || !(1..=16).contains(&label_chars) {
            return Err(format!("toolbar label '{}' must be 1 to 16 characters with no leading or trailing space", self.label));
        }
        if !toolbar_text_ok(&self.label, "-.&'()+") {
            return Err(format!("toolbar label '{}' may only use letters, digits, spaces and - . & ' ( ) +", self.label));
        }
        let collapsed = self.label.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        if RESERVED_TOOLBAR_LABELS.contains(&collapsed.as_str()) {
            return Err(format!("toolbar label '{}' is the name of a built-in button", self.label));
        }
        if !(!self.icon.is_empty() && self.icon.len() <= 40 && self.icon.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')) {
            return Err(format!("toolbar icon '{}' must be the name of an icon (letters, digits, dashes)", self.icon));
        }
        if RESERVED_TOOLBAR_ICONS.contains(&self.icon.as_str()) {
            return Err(format!("toolbar icon '{}' is used by a built-in button", self.icon));
        }
        if self.tooltip.chars().count() > 80 || !toolbar_text_ok(&self.tooltip, "-.&'()+,:;!?/") {
            return Err("toolbar tooltip must be at most 80 characters of plain text".to_string());
        }
        Ok(())
    }
}

/// A command-line tool the host keeps installed and current for a plugin, from a GitHub release
/// whose checksums file carries a detached OpenPGP signature. The signing key ships *inside the
/// plugin folder* (so the approval's digest pins it): it is never fetched from the same place as
/// the download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub id: String,
    /// "owner/repo" whose latest release provides the tool.
    pub github: String,
    /// Release asset to download, per `std::env::consts::OS` ("linux", "windows", "macos").
    pub assets: std::collections::BTreeMap<String, String>,
    /// Release asset listing "<sha256>  <asset name>" lines (e.g. SHA2-256SUMS).
    pub checksums: String,
    pub signature: ToolSignature,
    /// If set, a copy of the tool already on PATH under this name is used (and left alone) when
    /// the host hasn't installed its own.
    #[serde(default)]
    pub system_command: Option<String>,
    #[serde(default = "default_version_args")]
    pub version_args: Vec<String>,
}

fn default_version_args() -> Vec<String> {
    vec!["--version".to_string()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSignature {
    /// Release asset holding the detached signature of the checksums asset.
    pub asset: String,
    /// ASCII-armored OpenPGP public key, relative to the plugin folder.
    pub key_file: String,
}

/// One entry of a command template: a plain argument, or a group included depending on a parameter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ArgItem {
    Literal(String),
    Conditional {
        /// A parameter name (a bool is true / any other kind is present), or the built-in `ffmpeg_dir`.
        #[serde(rename = "if")]
        if_param: String,
        args: Vec<String>,
        #[serde(default, rename = "else")]
        else_args: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamSpec {
    /// `url`, `enum`, `bool` or `string`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub max_len: Option<usize>,
}

/// A fixed command line a plugin may run: the tool, the argument template, and the typed
/// parameters that may be substituted. The host runs nothing else, never through a shell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSpec {
    pub id: String,
    pub tool: String,
    pub args: Vec<ArgItem>,
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, ParamSpec>,
}

/// Values the host itself supplies to a template.
pub const BUILTIN_PLACEHOLDERS: [&str; 2] = ["output_dir", "ffmpeg_dir"];

pub fn valid_short_id(id: &str) -> bool {
    let b = id.as_bytes();
    (1..=40).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
}

/// `{name}` placeholders in an argument string.
pub fn placeholders(arg: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = arg;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                out.push(&after[..end]);
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

impl CommandSpec {
    fn all_args(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for a in &self.args {
            match a {
                ArgItem::Literal(s) => out.push(s.as_str()),
                ArgItem::Conditional { args, else_args, .. } => {
                    out.extend(args.iter().map(|s| s.as_str()));
                    out.extend(else_args.iter().map(|s| s.as_str()));
                }
            }
        }
        out
    }

    /// The command line as the operator reads it at review: placeholders left visible, groups shown
    /// with their condition.
    pub fn display(&self) -> String {
        let mut parts: Vec<String> = vec![self.tool.clone()];
        for a in &self.args {
            match a {
                ArgItem::Literal(s) => parts.push(s.clone()),
                ArgItem::Conditional { if_param, args, else_args } => {
                    let mut g = format!("[if {if_param}: {}", args.join(" "));
                    if !else_args.is_empty() {
                        g.push_str(&format!(" | else: {}", else_args.join(" ")));
                    }
                    g.push(']');
                    parts.push(g);
                }
            }
        }
        parts.join(" ")
    }
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
    /// The publisher's minisign public key (base64). Only meaningful together with a detached
    /// `plugin.json.minisig` over the manifest bytes; the manifest pins every artifact hash, so
    /// that one signature covers the binaries too. Pinned on first approval (trust on first use).
    #[serde(default)]
    pub publisher_key: Option<String>,
    /// semver requirement against the running OS-Next version, e.g. ">=0.2.0, <0.3.0".
    #[serde(default)]
    pub host_api: String,
    #[serde(default)]
    pub backend: Option<BackendSection>,
    #[serde(default)]
    pub web: Option<WebSection>,
    #[serde(default)]
    pub permissions: Vec<PermissionRequest>,
    /// Where the plugin's page appears (default: a floating dock panel).
    #[serde(default)]
    pub ui: Option<UiSection>,
    /// Command-line tools the host manages for this plugin (see `ToolSpec`).
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    /// The only commands the plugin may run (see `CommandSpec`); granted per id via `process.run`.
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
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
            if b.abi != 2 {
                return bad("backend.abi must be 2 (the original ABI 1 is no longer supported; rebuild the plugin against this version)".to_string());
            }
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
            if w.sandbox {
                if !relative_path_is_contained(e) || e.contains('\\') || !e.ends_with(".html") {
                    return bad("a sandboxed plugin's web.entry must be a relative .html path inside its folder".to_string());
                }
                if self.permissions.iter().any(|p| p.id == "ui.dom") {
                    return bad("a sandboxed plugin can't request ui.dom; that is what the sandbox replaces".to_string());
                }
                if self.backend.is_some() {
                    return bad("a sandboxed web plugin can't also ship a native library".to_string());
                }
            } else if e.is_empty() || e.contains('/') || e.contains('\\') || !e.ends_with(".js") {
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
            if p.id == "process.run" {
                if p.commands.is_empty() {
                    return bad("'process.run' needs a commands list".to_string());
                }
                for c in &p.commands {
                    if !self.commands.iter().any(|d| &d.id == c) {
                        return bad(format!("'process.run' lists '{c}', which this plugin doesn't declare in \"commands\""));
                    }
                }
            }
        }
        self.validate_ui()?;
        self.validate_tools_and_commands()
    }

    fn validate_ui(&self) -> Result<(), ManifestError> {
        let wants_toolbar = self.permissions.iter().any(|p| p.id == "ui.toolbar");
        let Some(ui) = &self.ui else {
            return if wants_toolbar { Err(ManifestError::Invalid("'ui.toolbar' needs at least one item under ui.toolbar".to_string())) } else { Ok(()) };
        };
        let bad = |m: String| Err(ManifestError::Invalid(m));
        if !ui.toolbar.is_empty() || wants_toolbar {
            if !self.web.as_ref().is_some_and(|w| w.sandbox) {
                return bad("ribbon buttons (ui.toolbar) are only for sandboxed web plugins".to_string());
            }
            if !wants_toolbar {
                return bad("ribbon buttons need the 'ui.toolbar' permission".to_string());
            }
            if ui.toolbar.is_empty() {
                return bad("'ui.toolbar' needs at least one item under ui.toolbar".to_string());
            }
            if ui.toolbar.len() > MAX_TOOLBAR_ITEMS {
                return bad(format!("at most {MAX_TOOLBAR_ITEMS} toolbar buttons"));
            }
            let mut ids = std::collections::HashSet::new();
            for item in &ui.toolbar {
                item.validate().map_err(ManifestError::Invalid)?;
                if !ids.insert(item.id.clone()) {
                    return bad(format!("toolbar item id '{}' is listed twice", item.id));
                }
            }
        }
        if !ui.placements.is_empty() && !self.web.as_ref().is_some_and(|w| w.sandbox) {
            return bad("ui placements are only for sandboxed web plugins".to_string());
        }
        if !ui.placements.is_empty() && !self.permissions.iter().any(|p| p.id == "ui.panel") {
            return bad("showing UI in the console needs the 'ui.panel' permission".to_string());
        }
        if ui.placements.len() > 4 {
            return bad("at most 4 placements".to_string());
        }
        for p in &ui.placements {
            match p.slot.as_str() {
                "dock" | "video-picker" => {}
                "library-source" => {
                    if p.label.trim().is_empty() || p.label.chars().count() > 40 {
                        return bad("a 'library-source' placement needs a label of up to 40 characters".to_string());
                    }
                    if !RESOURCE_CATEGORIES.contains(&p.tab.as_str()) {
                        return bad("a 'library-source' placement needs a tab: songs, scriptures, media, presentations or themes".to_string());
                    }
                }
                other => return bad(format!("unknown placement slot '{other}'")),
            }
        }
        Ok(())
    }

    fn validate_tools_and_commands(&self) -> Result<(), ManifestError> {
        let bad = |m: String| Err(ManifestError::Invalid(m));
        if (!self.tools.is_empty() || !self.commands.is_empty()) && !self.web.as_ref().is_some_and(|w| w.sandbox) {
            return bad("tools and commands are only supported for sandboxed web plugins".to_string());
        }
        let mut tool_ids = std::collections::HashSet::new();
        for t in &self.tools {
            if !valid_short_id(&t.id) || !tool_ids.insert(t.id.clone()) {
                return bad(format!("bad or duplicate tool id '{}'", t.id));
            }
            let repo_ok = t.github.split('/').count() == 2
                && t.github.split('/').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) && !p.chars().all(|c| c == '.'));
            if !repo_ok {
                return bad(format!("tool '{}': github must be \"owner/repo\"", t.id));
            }
            let plain = |n: &str| !n.is_empty() && n.len() <= 120 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
            if t.assets.is_empty() || !t.assets.iter().all(|(os, a)| matches!(os.as_str(), "linux" | "windows" | "macos") && plain(a)) {
                return bad(format!("tool '{}': assets must map linux/windows/macos to plain release asset names", t.id));
            }
            if !plain(&t.checksums) || !plain(&t.signature.asset) {
                return bad(format!("tool '{}': checksums and signature must be plain release asset names", t.id));
            }
            if !relative_path_is_contained(&t.signature.key_file) {
                return bad(format!("tool '{}': signature.key_file must stay inside the plugin folder", t.id));
            }
            if let Some(c) = &t.system_command {
                if !plain(c) {
                    return bad(format!("tool '{}': bad system_command", t.id));
                }
            }
        }
        let mut cmd_ids = std::collections::HashSet::new();
        for c in &self.commands {
            if !valid_short_id(&c.id) || !cmd_ids.insert(c.id.clone()) {
                return bad(format!("bad or duplicate command id '{}'", c.id));
            }
            if !tool_ids.contains(&c.tool) {
                return bad(format!("command '{}' uses undeclared tool '{}'", c.id, c.tool));
            }
            for (name, p) in &c.params {
                if !valid_short_id(name) || BUILTIN_PLACEHOLDERS.contains(&name.as_str()) {
                    return bad(format!("command '{}': bad parameter name '{name}'", c.id));
                }
                match p.kind.as_str() {
                    "url" | "bool" | "string" => {}
                    "enum" if !p.values.is_empty() && p.values.iter().all(|v| !v.is_empty() && !v.starts_with('-')) => {}
                    other => return bad(format!("command '{}': parameter '{name}' has an invalid type/values ({other})", c.id)),
                }
            }
            let known = |n: &str| c.params.contains_key(n) || BUILTIN_PLACEHOLDERS.contains(&n);
            for a in &c.args {
                if let ArgItem::Conditional { if_param, .. } = a {
                    if !known(if_param) {
                        return bad(format!("command '{}': 'if' refers to unknown parameter '{if_param}'", c.id));
                    }
                }
            }
            for arg in c.all_args() {
                for ph in placeholders(arg) {
                    if !known(ph) {
                        return bad(format!("command '{}': unknown placeholder {{{ph}}}", c.id));
                    }
                }
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
        "backend": {"abi": 2, "libraries": {"linux-x86_64": {"path": "lib/x.so", "sha256": "0000000000000000000000000000000000000000000000000000000000000000"}}},
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
    fn sandboxed_web_plugins_have_their_own_rules() {
        let sandboxed = |extra: &str| {
            format!(
                r#"{{"manifest_version":1,"id":"sbx","name":"S","web":{{"entry":"index.html","sandbox":true}},"permissions":[{{"id":"show.observe"}}{extra}]}}"#
            )
        };
        assert!(manifest(&sandboxed("")).is_ok());
        assert!(manifest(&sandboxed(r#",{"id":"ui.dom"}"#)).is_err(), "the sandbox replaces ui.dom");
        assert!(manifest(&sandboxed("").replace("index.html", "../index.html")).is_err());
        assert!(manifest(&sandboxed("").replace("index.html", "main.js")).is_err());
        assert!(manifest(&sandboxed("").replace("index.html", "ui/index.html")).is_ok());
    }

    #[test]
    fn ribbon_buttons_are_validated_hard() {
        let with = |item: &str, perm: bool, sandbox: bool| {
            format!(
                r#"{{"manifest_version":1,"id":"tb","name":"T","web":{{"entry":"index.html","sandbox":{sandbox}}},"permissions":[{perm}],"ui":{{"toolbar":[{item}]}}}}"#,
                perm = if perm { r#"{"id":"ui.toolbar"}"# } else { r#"{"id":"show.observe"}"# },
            )
        };
        let good = r#"{"id":"sync","label":"Sync","icon":"refresh","tooltip":"Pull this week's plan"}"#;
        assert!(manifest(&with(good, true, true)).is_ok());
        assert!(manifest(&with(good, false, true)).is_err(), "needs the permission");
        assert!(manifest(&with(good, true, false)).is_err(), "sandboxed plugins only");
        // The permission without any item is meaningless.
        assert!(manifest(&format!(r#"{{"manifest_version":1,"id":"tb","name":"T","web":{{"entry":"index.html","sandbox":true}},"permissions":[{{"id":"ui.toolbar"}}]}}"#)).is_err());
        assert!(manifest(&with("", true, true)).is_err());
        let item = |id: &str, label: &str, icon: &str| format!(r#"{{"id":"{id}","label":"{label}","icon":"{icon}"}}"#);
        for (why, bad) in [
            ("a built-in label", item("a", "Go Live", "refresh")),
            ("a built-in label with odd spacing and case", item("a", "go  live", "refresh")),
            ("another built-in label", item("a", "Blackout", "refresh")),
            ("a bidi override", item("a", "Sy\\u202enc", "refresh")),
            ("a zero-width space", item("a", "Sy\\u200bnc", "refresh")),
            ("an emoji", item("a", "Sync \\ud83d\\udd04", "refresh")),
            ("markup", item("a", "<b>x</b>", "refresh")),
            ("leading space", item("a", " Sync", "refresh")),
            ("too long", item("a", "Seventeen chars!!", "refresh")),
            ("empty", item("a", "", "refresh")),
            ("a reserved icon", item("a", "Sync", "golive")),
            ("an icon that is a path", item("a", "Sync", "../x")),
            ("a bad id", item("A B", "Sync", "refresh")),
        ] {
            assert!(manifest(&with(&bad, true, true)).is_err(), "{why}");
        }
        assert!(manifest(&with(r#"{"id":"a","label":"Sync","icon":"refresh","extra":1}"#, true, true)).is_err(), "unknown keys");
        let long_tip = format!(r#"{{"id":"a","label":"Sync","icon":"refresh","tooltip":"{}"}}"#, "x".repeat(81));
        assert!(manifest(&with(&long_tip, true, true)).is_err());
        let three = format!("{},{},{}", item("a", "One", "refresh"), item("b", "Two", "refresh"), item("c", "Three", "refresh"));
        assert!(manifest(&with(&three, true, true)).is_err(), "at most two");
        let dup = format!("{},{}", item("a", "One", "refresh"), item("a", "Two", "refresh"));
        assert!(manifest(&with(&dup, true, true)).is_err(), "duplicate id");
        assert!(manifest(&with(&format!("{},{}", item("a", "One", "refresh"), item("b", "Two", "plus")), true, true)).is_ok());
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
        assert!(PermissionSet::in_process().provides("themes"));
        let wild = PermissionSet(vec![PermissionRequest { id: "commands.send".into(), commands: vec!["*".into()], ..Default::default() }]);
        assert_eq!(wild.0[0].tier(), Tier::High);
        assert!(wild.allows_command("Anything"));
    }

    #[test]
    fn narrowing_never_widens_and_follows_the_live_grant() {
        let cmds = |c: &[&str]| PermissionRequest { id: "commands.send".into(), commands: c.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let grant = PermissionSet(vec![cmds(&["NextSlide", "PrevSlide"]), PermissionRequest::simple("keyring.own")]);
        // Asking for more than granted is cut down to the intersection...
        let tok = PermissionSet(vec![cmds(&["NextSlide", "NewSchedule"]), PermissionRequest::simple("keyring.own"), PermissionRequest::simple("show.observe")]);
        let eff = tok.narrowed_by(&grant);
        assert!(eff.allows_command("NextSlide"));
        assert!(!eff.allows_command("NewSchedule"));
        assert!(!eff.has("show.observe"), "an ungranted permission is dropped");
        assert!(eff.has("keyring.own"));
        // ...a wildcard token under a narrow grant gets only the grant...
        let wild = PermissionSet(vec![cmds(&["*"])]).narrowed_by(&grant);
        assert!(wild.allows_command("PrevSlide") && !wild.allows_command("GoLive"));
        // ...and a shrunk or revoked grant shrinks the token with it.
        assert!(!tok.narrowed_by(&PermissionSet::empty()).has("keyring.own"));
        assert!(!PermissionSet(vec![cmds(&["Zzz"])]).narrowed_by(&grant).has("commands.send"));
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
