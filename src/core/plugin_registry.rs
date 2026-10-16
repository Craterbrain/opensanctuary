//! Finds plugins on disk (`web/plugins/`) and records what it saw in `plugin_grants`
//! (docs/PLUGIN_PERMISSIONS.md).
//!
//! A plugin is served/started only while it is `approved` *and* what is on disk still
//! hashes to what the operator reviewed; any change drops it to `changed` until it is
//! re-reviewed.

use std::path::Path;

use crate::core::plugin_permissions::{
    artifact_digest, sha256_file, sha256_hex, PermissionRequest, PermissionSet,
    PluginManifest,
};
use crate::storage::db::{ObservedPlugin, PluginGrantRow};
use crate::storage::Database;

#[derive(Debug, Clone)]
pub struct Discovered {
    pub id: String,
    /// `web` (a console script) or `sandbox`.
    pub kind: &'static str,
    pub name: String,
    pub version: Option<String>,
    pub source_path: String,
    pub manifest_sha256: Option<String>,
    pub digest: String,
    /// The publisher key whose signature over `plugin.json` verified (None = unsigned).
    pub signer: Option<String>,
    pub requested: Vec<PermissionRequest>,
    pub error: Option<String>,
    /// `(path, sha256)` of every file the digest covers (empty when the plugin failed to load).
    pub files: Vec<(String, String)>,
    /// What the signature, if any, covers: `unsigned`, `manifest` (only plugin.json) or `full` (every file).
    pub signature_scope: &'static str,
    /// The key this plugin's signer proved it replaces (a rotation statement signed by that old key), if any.
    pub rotation_from: Option<String>,
}

/// Checks the detached signature next to a manifest, if there is one. `Ok(None)` = unsigned.
/// A signature that is present but doesn't verify (or has no `publisher_key` to check against)
/// is an error: it must never silently degrade to "unsigned".
fn verify_signer(manifest_bytes: &[u8], manifest: &PluginManifest, sig_path: &Path) -> Result<Option<String>, String> {
    let sig_text = match std::fs::read_to_string(sig_path) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };
    let key = manifest
        .publisher_key
        .as_deref()
        .ok_or_else(|| "plugin.json.minisig is present but plugin.json has no publisher_key".to_string())?;
    crate::network::updater::verify_signature(manifest_bytes, &sig_text, key)
        .map_err(|e| format!("plugin.json signature: {e}"))?;
    Ok(Some(key.to_string()))
}

fn failed(id: String, kind: &'static str, source: &Path, msg: String) -> Discovered {
    Discovered {
        name: id.clone(),
        id,
        kind,
        version: None,
        source_path: source.to_string_lossy().to_string(),
        manifest_sha256: None,
        digest: sha256_hex(msg.as_bytes()),
        signer: None,
        requested: Vec::new(),
        error: Some(msg),
        files: Vec::new(),
        signature_scope: "unsigned",
        rotation_from: None,
    }
}

fn discover_web(web_plugins_dir: &Path, host_version: &str, icons_dir: Option<&Path>, folders_only: bool) -> Vec<Discovered> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(web_plugins_dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(d) = discover_sandbox(&path, host_version, icons_dir) {
                out.push(d);
            }
            continue;
        }
        if folders_only || !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("js") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string()) else { continue };
        let file_name = format!("{stem}.js");
        let file_sha = match sha256_file(&path) {
            Ok(h) => h,
            Err(_) => continue,
        };
        let manifest_path = web_plugins_dir.join(format!("{stem}.plugin.json"));
        if let Ok(bytes) = std::fs::read(&manifest_path) {
            let manifest_sha = sha256_hex(&bytes);
            match PluginManifest::parse(&bytes).and_then(|m| m.validate(host_version).map(|_| m)) {
                Err(e) => {
                    let mut d = failed(stem, "web", &path, e.to_string());
                    d.manifest_sha256 = Some(manifest_sha);
                    out.push(d);
                }
                Ok(m) => {
                    let (signer, sig_problem) = match verify_signer(&bytes, &m, &web_plugins_dir.join(format!("{stem}.plugin.json.minisig"))) {
                        Ok(s) => (s, None),
                        Err(e) => (None, Some(e)),
                    };
                    let problem = if let Some(e) = sig_problem {
                        Some(e)
                    } else if m.id != stem {
                        Some(format!("plugin id '{}' must match the file name '{}'", m.id, stem))
                    } else if m.web.as_ref().map(|w| w.entry.as_str()) != Some(file_name.as_str()) {
                        Some(format!("web.entry must be '{file_name}'"))
                    } else {
                        None
                    };
                    out.push(Discovered {
                        id: m.id.clone(),
                        kind: "web",
                        name: m.name.clone(),
                        version: Some(m.version.clone()).filter(|v| !v.is_empty()),
                        source_path: path.to_string_lossy().to_string(),
                        manifest_sha256: Some(manifest_sha),
                        digest: artifact_digest(&[(file_name.clone(), file_sha.clone())]),
                        signer: signer.clone(),
                        requested: m.permissions.clone(),
                        error: problem,
                        files: vec![(file_name.clone(), file_sha)],
                        // One script file: there are no other files, and `files` doesn't apply.
                        signature_scope: if signer.is_some() { "manifest" } else { "unsigned" },
                        rotation_from: None,
                    });
                }
            }
        } else {
            // A bare script with no manifest used to run with full console access. That is gone:
            // a web plugin needs a `<id>.plugin.json` (or a sandboxed folder).
            tracing::warn!("ignoring {path:?}: web plugins need a manifest ({stem}.plugin.json); loose scripts are no longer loaded");
        }
    }
    out
}

/// Most files / bytes a sandboxed plugin folder may hold (it is hashed on every request).
const SANDBOX_MAX_FILES: usize = 300;
const SANDBOX_MAX_BYTES: u64 = 25 * 1024 * 1024;

/// Every regular file in a sandboxed plugin folder as `(relative path, sha256)`. Symlinks and
/// anything over the caps are rejected: the digest must cover exactly what can be served.
pub fn sandbox_dir_files(dir: &Path) -> Result<Vec<(String, String)>, String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, String)>, total: &mut u64) -> Result<(), String> {
        for entry in std::fs::read_dir(dir).map_err(|e| format!("{dir:?}: {e}"))?.flatten() {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{path:?}: {e}"))?;
            if meta.file_type().is_symlink() {
                return Err(format!("{path:?} is a symlink; sandboxed plugin folders can't contain any"));
            }
            if meta.is_dir() {
                walk(base, &path, out, total)?;
            } else if meta.is_file() {
                *total += meta.len();
                if *total > SANDBOX_MAX_BYTES || out.len() >= SANDBOX_MAX_FILES {
                    return Err("plugin folder is too large".to_string());
                }
                let rel = path.strip_prefix(base).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
                out.push((rel, sha256_file(&path).map_err(|e| format!("{path:?}: {e}"))?));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    let mut total = 0u64;
    walk(dir, dir, &mut out, &mut total)?;
    Ok(out)
}

/// Files that are never listed in a manifest's `files`: the manifest itself (the signature covers its bytes)
/// and the signature.
pub const UNLISTED: [&str; 4] = ["plugin.json", "plugin.json.minisig", ROTATION_FILE, "plugin.rotation.json.minisig"];

/// A key rotation statement shipped next to the manifest: `{ "old_key": "...", "new_key": "..." }`, signed (detached,
/// `plugin.rotation.json.minisig`) by the OLD key. It says "the publisher who signed with old_key now signs with new_key".
pub const ROTATION_FILE: &str = "plugin.rotation.json";

#[derive(serde::Deserialize)]
struct RotationStatement {
    old_key: String,
    new_key: String,
}

/// The old key, if the folder carries a rotation statement for `new_key` that the old key really signed. Anything
/// wrong with it (missing, malformed, for another key, a bad signature) means no rotation: the plugin then shows the
/// ordinary "different publisher" warning, which is the safe reading.
fn verified_rotation(dir: &Path, new_key: &str) -> Option<String> {
    let bytes = std::fs::read(dir.join(ROTATION_FILE)).ok()?;
    if bytes.len() > 16 * 1024 {
        return None;
    }
    let sig = std::fs::read_to_string(dir.join("plugin.rotation.json.minisig")).ok()?;
    let st: RotationStatement = serde_json::from_slice(&bytes).ok()?;
    if st.new_key != new_key || st.old_key == new_key {
        return None;
    }
    crate::network::updater::verify_signature(&bytes, &sig, &st.old_key).ok()?;
    Some(st.old_key)
}

/// When a manifest lists its files, the folder must hold exactly those, with exactly those hashes: a
/// listed file that is missing, an unlisted file, a different hash, or a path that could escape the folder
/// is an error. (An unlisted file next to a signed manifest is how a signature gets laundered.)
pub fn check_listed_files(listed: Option<&std::collections::BTreeMap<String, String>>, actual: &[(String, String)]) -> Result<(), String> {
    let Some(listed) = listed else { return Ok(()) };
    let few = |v: Vec<&str>| {
        let mut s = v.iter().take(3).copied().collect::<Vec<_>>().join(", ");
        if v.len() > 3 {
            s.push_str(&format!(" and {} more", v.len() - 3));
        }
        s
    };
    for (path, hash) in listed {
        if !crate::core::plugin_permissions::relative_path_is_contained(path) || path.contains('\\') || UNLISTED.contains(&path.as_str()) {
            return Err(format!("'{path}' can't be listed in files"));
        }
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("the hash listed for '{path}' isn't a SHA-256"));
        }
    }
    let actual_map: std::collections::BTreeMap<&str, &str> =
        actual.iter().filter(|(p, _)| !UNLISTED.contains(&p.as_str())).map(|(p, h)| (p.as_str(), h.as_str())).collect();
    let missing: Vec<&str> = listed.keys().filter(|k| !actual_map.contains_key(k.as_str())).map(|k| k.as_str()).collect();
    if !missing.is_empty() {
        return Err(format!("listed but missing: {}", few(missing)));
    }
    let extra: Vec<&str> = actual_map.keys().filter(|k| !listed.contains_key(**k)).copied().collect();
    if !extra.is_empty() {
        return Err(format!("not listed in files: {}", few(extra)));
    }
    let different: Vec<&str> = listed.iter().filter(|(k, h)| !actual_map[k.as_str()].eq_ignore_ascii_case(h)).map(|(k, _)| k.as_str()).collect();
    if !different.is_empty() {
        return Err(format!("different from what was signed or listed: {}", few(different)));
    }
    Ok(())
}

/// Writes every file's SHA-256 into a plugin folder's `plugin.json` as `files` (and, if given, sets `publisher_key`),
/// so that one signature over the manifest covers the whole package. Returns how many files were listed. The caller signs
/// `plugin.json` afterwards; any existing signature is left in place, now stale.
pub fn write_file_list(folder: &Path, publisher_key: Option<&str>) -> Result<usize, String> {
    let manifest_path = folder.join("plugin.json");
    let bytes = std::fs::read(&manifest_path).map_err(|_| format!("{} has no plugin.json", folder.display()))?;
    let mut manifest = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(v) if v.is_object() => v,
        _ => return Err("plugin.json isn't a JSON object".to_string()),
    };
    let listed: std::collections::BTreeMap<String, String> =
        sandbox_dir_files(folder)?.into_iter().filter(|(p, _)| !UNLISTED.contains(&p.as_str())).collect();
    let count = listed.len();
    manifest["files"] = serde_json::json!(listed);
    if let Some(k) = publisher_key {
        manifest["publisher_key"] = serde_json::json!(k);
    }
    let mut text = serde_json::to_string_pretty(&manifest).unwrap_or_default();
    text.push('\n');
    std::fs::write(&manifest_path, text).map_err(|e| format!("couldn't write plugin.json: {e}"))?;
    Ok(count)
}

/// A manifest is a few KB; anything this big is hostile or broken, and is never parsed.
const MAX_MANIFEST_BYTES: usize = 256 * 1024;

fn discover_sandbox(dir: &Path, host_version: &str, icons_dir: Option<&Path>) -> Option<Discovered> {
    let dir_name = dir.file_name()?.to_str()?.to_string();
    let bytes = std::fs::read(dir.join("plugin.json")).ok()?; // not a plugin folder
    let manifest_sha = sha256_hex(&bytes);
    if bytes.len() > MAX_MANIFEST_BYTES {
        let mut d = failed(dir_name, "sandbox", dir, format!("plugin.json is {} KB; the limit is {} KB", bytes.len() / 1024, MAX_MANIFEST_BYTES / 1024));
        d.manifest_sha256 = Some(manifest_sha);
        return Some(d);
    }
    let manifest = match PluginManifest::parse(&bytes).and_then(|m| m.validate(host_version).map(|_| m)) {
        Ok(m) => m,
        Err(e) => {
            let mut d = failed(dir_name, "sandbox", dir, e.to_string());
            d.manifest_sha256 = Some(manifest_sha);
            return Some(d);
        }
    };
    let mut d = Discovered {
        id: manifest.id.clone(),
        kind: "sandbox",
        name: manifest.name.clone(),
        version: Some(manifest.version.clone()).filter(|v| !v.is_empty()),
        source_path: dir.to_string_lossy().to_string(),
        manifest_sha256: Some(manifest_sha.clone()),
        digest: String::new(),
        signer: None,
        requested: manifest.permissions.clone(),
        error: None,
        files: Vec::new(),
        signature_scope: "unsigned",
        rotation_from: None,
    };
    let fail = |mut d: Discovered, msg: String| {
        d.digest = sha256_hex(format!("{manifest_sha}{msg}").as_bytes());
        d.error = Some(msg);
        d
    };
    let web = manifest.web.as_ref().filter(|w| w.sandbox);
    let Some(web) = web else {
        return Some(fail(d, "this folder's plugin.json isn't a sandboxed web plugin (web.sandbox must be true)".to_string()));
    };
    if manifest.id != dir_name {
        return Some(fail(d, format!("plugin id '{}' must match its folder name '{}'", manifest.id, dir_name)));
    }
    match verify_signer(&bytes, &manifest, &dir.join("plugin.json.minisig")) {
        Ok(s) => d.signer = s,
        Err(e) => return Some(fail(d, e)),
    }
    if let (Some(icons), Some(ui)) = (icons_dir, manifest.ui.as_ref()) {
        if let Some(item) = ui.toolbar.iter().find(|t| !icons.join(format!("{}.svg", t.icon)).is_file()) {
            return Some(fail(d, format!("toolbar button '{}' uses icon '{}', which isn't in the app's icon set", item.id, item.icon)));
        }
    }
    match sandbox_dir_files(dir) {
        Err(e) => Some(fail(d, e)),
        Ok(files) => {
            if !files.iter().any(|(rel, _)| *rel == web.entry) {
                return Some(fail(d, format!("entry page '{}' is missing", web.entry)));
            }
            if let Err(e) = check_listed_files(manifest.files.as_ref(), &files) {
                let why = if d.signer.is_some() { "this signed package doesn't match its manifest" } else { "the files don't match the hashes in plugin.json" };
                return Some(fail(d, format!("{why}: {e}")));
            }
            d.signature_scope = match (&d.signer, &manifest.files) {
                (None, _) => "unsigned",
                (Some(_), None) => "manifest",
                (Some(_), Some(_)) => "full",
            };
            d.rotation_from = d.signer.as_deref().and_then(|new_key| verified_rotation(dir, new_key));
            d.digest = artifact_digest(&files);
            d.files = files;
            Some(d)
        }
    }
}

/// Scans the web plugins folder.
/// Where plugins the operator installs live: under the data directory, never the (read-only) install.
pub fn user_plugins_dir(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("plugins")
}

/// Is `dir` one the operator installed (so it can be updated or removed), as opposed to one that
/// ships with the app?
pub fn is_installed_folder(data_dir: &Path, dir: &Path) -> bool {
    let (Ok(root), Ok(dir)) = (std::fs::canonicalize(user_plugins_dir(data_dir)), std::fs::canonicalize(dir)) else { return false };
    dir.parent() == Some(root.as_path())
}

/// Is this plugin's folder one that ships with the app (`web/plugins/`), rather than one the
/// operator installed? Decided by location, not by any signature (docs/PLUGIN_SIGNING.md
/// section 2's "Built-in" trust level: "the folder ships with the app").
pub fn is_builtin(data_dir: &Path, source_path: &Path) -> bool {
    !is_installed_folder(data_dir, source_path)
}

/// Whether `row` is actually usable right now: approved and unchanged (`effective_state() ==
/// "active"`) AND allowed to load under the console's signature policy
/// (docs/PLUGIN_SIGNING.md section 6) -- `require` stops a plugin that isn't built-in and isn't
/// signed, even if it was already approved and nothing about it has changed since.
pub fn is_usable(db: &Database, data_dir: &Path, row: &PluginGrantRow) -> bool {
    if row.effective_state() != "active" {
        return false;
    }
    let policy = crate::core::plugin_policy::Policy::parse(db.get_setting(crate::core::plugin_policy::SETTING_KEY).ok().flatten().as_deref());
    crate::core::plugin_policy::permits_load(policy, is_builtin(data_dir, Path::new(&row.source_path)), row.approved_signer.is_some())
}

/// The highest version ever approved for `plugin_id` under `signer` ("" for unsigned), if any.
/// Scoped per publisher key: an unrelated key's history says nothing about what to expect from
/// this one (docs/PLUGIN_SIGNING.md section 7).
pub fn version_floor(db: &Database, plugin_id: &str, signer: Option<&str>) -> Option<String> {
    db.version_floor(plugin_id, signer).ok().flatten()
}

/// Rolls `plugin_id`'s remembered highest-ever-approved version (scoped to `signer`) up to
/// `version` if it's a real, higher semver than what's recorded -- never down. Call this once a
/// plugin is actually approved; it's what keeps a downgrade detectable after a revoke/re-approve
/// or an uninstall/reinstall, not just while the plugin has stayed continuously approved
/// (docs/PLUGIN_SIGNING.md section 7). Deliberately never deleted when a plugin is uninstalled --
/// that would reopen exactly the hole this exists to close.
pub fn remember_approved_version(db: &Database, plugin_id: &str, signer: Option<&str>, version: Option<&str>) {
    let Some(version) = version else { return };
    let Ok(new) = semver::Version::parse(version) else { return };
    let should_raise = match version_floor(db, plugin_id, signer).as_deref().map(semver::Version::parse) {
        Some(Ok(old)) => new > old,
        _ => true,
    };
    if should_raise {
        let _ = db.raise_version_floor(plugin_id, signer, version);
    }
}

/// Is `candidate` an older version than the remembered floor? `false` whenever either side isn't
/// a parseable semver -- an unknown relationship is not reported as a downgrade.
pub fn is_downgrade_from_floor(floor: Option<&str>, candidate: Option<&str>) -> bool {
    match (floor.and_then(|v| semver::Version::parse(v).ok()), candidate.and_then(|v| semver::Version::parse(v).ok())) {
        (Some(floor), Some(candidate)) => candidate < floor,
        _ => false,
    }
}

pub fn discover(web_plugins_dir: &Path, host_version: &str) -> Vec<Discovered> {
    discover_all(web_plugins_dir, None, host_version)
}

fn flag_duplicates(out: &mut [Discovered]) {
    // A duplicate id is ambiguous; neither copy may run.
    let mut counts = std::collections::HashMap::new();
    for d in out.iter() {
        *counts.entry(d.id.clone()).or_insert(0usize) += 1;
    }
    for d in out.iter_mut() {
        if counts[&d.id] > 1 && d.error.is_none() {
            d.error = Some(format!("another plugin also uses the id '{}'", d.id));
        }
    }
}

/// The plugins that ship with the app (`web/plugins/`) plus those the operator installed
/// (`<data dir>/plugins/`, sandboxed folders only). A built-in plugin always wins an id clash: an
/// installed folder can never shadow, replace or break one that shipped.
pub fn discover_all(web_plugins_dir: &Path, user_plugins_dir: Option<&Path>, host_version: &str) -> Vec<Discovered> {
    // Ribbon-button icons are names of files in the app's icon set, `web/icons/crimson/` next to the plugins folder.
    let icons_dir = web_plugins_dir.parent().map(|w| w.join("icons").join("crimson"));
    let mut out = discover_web(web_plugins_dir, host_version, icons_dir.as_deref(), false);
    flag_duplicates(&mut out);
    if let Some(user) = user_plugins_dir {
        let mut installed = discover_web(user, host_version, icons_dir.as_deref(), true);
        flag_duplicates(&mut installed);
        for d in installed {
            if out.iter().any(|b| b.id == d.id) {
                tracing::warn!("ignoring installed plugin folder {:?}: '{}' is the id of a built-in plugin", d.source_path, d.id);
                continue;
            }
            out.push(d);
        }
    }
    out
}

/// Checks one extracted plugin folder the way discovery would, returning the reason it can't be
/// installed. The folder's name must be the plugin's id.
pub fn check_plugin_folder(dir: &Path, host_version: &str, icons_dir: Option<&Path>) -> Result<Discovered, String> {
    match discover_sandbox(dir, host_version, icons_dir) {
        None => Err("there is no plugin.json in the package".to_string()),
        Some(d) => match &d.error {
            Some(e) => Err(e.clone()),
            None => Ok(d),
        },
    }
}

fn requested_json(d: &Discovered) -> String {
    serde_json::to_string(&d.requested).unwrap_or_else(|_| "[]".to_string())
}

/// Records the scan in the database. Anything new is `pending` until the operator approves it.
pub fn sync(db: &Database, discovered: &[Discovered]) {
    // Rows for the removed kinds (native libraries, pre-manifest files) would otherwise sit in the list forever.
    let _ = db.purge_removed_plugin_kinds();
    for d in discovered {
        let existing = db.get_plugin_grant(&d.id).ok().flatten();
        let observed = ObservedPlugin {
            plugin_id: &d.id,
            kind: d.kind,
            name: &d.name,
            version: d.version.as_deref(),
            source_path: &d.source_path,
            manifest_sha256: d.manifest_sha256.as_deref(),
            digest: &d.digest,
            signer: d.signer.as_deref(),
            requested_json: &requested_json(d),
            error: d.error.as_deref(),
            files_json: &serde_json::to_string(&d.files).unwrap_or_else(|_| "[]".to_string()),
            signature_scope: d.signature_scope,
            rotation_from: d.rotation_from.as_deref(),
        };
        if db.upsert_observed_plugin(&observed).is_err() {
            continue;
        }
        match &existing {
            None => db.add_plugin_audit(&d.id, "system", "discovered", &serde_json::json!({ "kind": d.kind }).to_string()),
            Some(row) if row.observed_digest != d.digest && row.status == "approved" => {
                db.add_plugin_audit(&d.id, "system", "changed", &serde_json::json!({ "digest": d.digest }).to_string())
            }
            _ => {}
        }
    }
}

/// How an approved plugin's files now differ from what the operator reviewed -- the "what changed" shown when
/// it stops at `changed`. `None` unless it was approved and has since changed. Permission differences are
/// measured against what the plugin *asked for* last time, so an optional permission you declined is not
/// reported as new every update.
pub fn describe_update(row: &PluginGrantRow) -> Option<serde_json::Value> {
    if row.status != "approved" || row.error.is_some() || row.approved_digest.as_deref() == Some(row.observed_digest.as_str()) {
        return None;
    }
    row.approved_digest.as_ref()?;
    let files = |json: &Option<String>| -> Option<std::collections::BTreeMap<String, String>> {
        serde_json::from_str::<Vec<(String, String)>>(json.as_deref()?).ok().map(|v| v.into_iter().collect())
    };
    let file_diff = match (files(&row.approved_files_json), files(&row.observed_files_json)) {
        (Some(old), Some(new)) => {
            let added: Vec<&String> = new.keys().filter(|k| !old.contains_key(*k)).collect();
            let removed: Vec<&String> = old.keys().filter(|k| !new.contains_key(*k)).collect();
            let modified: Vec<&String> = new.iter().filter(|(k, v)| old.get(*k).is_some_and(|o| o != *v)).map(|(k, _)| k).collect();
            Some(serde_json::json!({ "added": added, "removed": removed, "modified": modified }))
        }
        _ => None, // approved before file lists were recorded
    };
    let direction = match (
        row.approved_version.as_deref().and_then(|v| semver::Version::parse(v).ok()),
        row.version.as_deref().and_then(|v| semver::Version::parse(v).ok()),
    ) {
        (Some(a), Some(b)) => match b.cmp(&a) {
            std::cmp::Ordering::Greater => "upgrade",
            std::cmp::Ordering::Less => "downgrade",
            std::cmp::Ordering::Equal => "same",
        },
        _ => "unknown",
    };
    let parse = |json: &str| serde_json::from_str::<Vec<PermissionRequest>>(json).unwrap_or_default();
    // Rows approved before this was recorded fall back to what was granted.
    let before = parse(row.approved_requested_json.as_deref().unwrap_or(&row.granted_json));
    let now = parse(&row.requested_json);
    let mut permissions = crate::core::plugin_permissions::permission_diff(&before, &now);
    // New asks come with their risk, so the review can say "this update wants something Critical".
    permissions["added_tiers"] = serde_json::json!(
        now.iter().filter(|p| !before.iter().any(|b| b.id == p.id)).map(|p| serde_json::json!({ "id": p.id, "tier": format!("{:?}", p.tier()).to_lowercase() })).collect::<Vec<_>>()
    );
    Some(serde_json::json!({
        "from_version": row.approved_version, "to_version": row.version, "direction": direction,
        "files": file_diff, "permissions": permissions,
    }))
}

pub fn parse_granted(row: &PluginGrantRow) -> PermissionSet {
    PermissionSet(serde_json::from_str::<Vec<PermissionRequest>>(&row.granted_json).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(dir: &Path) -> Database {
        Database::new(dir.join("t.db").to_str().unwrap()).unwrap()
    }

    /// A sandboxed plugin folder `<id>` under `plugins` with the given entry page.
    fn write_sandbox_plugin(plugins: &Path, id: &str, page: &str) -> std::path::PathBuf {
        let dir = plugins.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), page).unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            serde_json::json!({
                "manifest_version": 1, "id": id, "name": "Thing", "version": "1.0.0",
                "web": {"entry": "index.html", "sandbox": true},
                "permissions": [{"id": "show.observe"}]
            }).to_string(),
        ).unwrap();
        dir
    }

    #[test]
    fn removed_plugin_forms_are_ignored_or_refused_and_old_rows_are_purged() {
        let tmp = tempfile::tempdir().unwrap();
        let web = tmp.path().join("web_plugins");
        std::fs::create_dir_all(&web).unwrap();
        // A bare script with no manifest is not a plugin.
        std::fs::write(web.join("demo.js"), b"export default {}").unwrap();
        // A manifest that asks for a native library is refused with an explanation, not loaded.
        let dir = web.join("native_try");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plugin.json"), serde_json::json!({
            "manifest_version": 1, "id": "native_try", "name": "N",
            "backend": {"abi": 2, "libraries": {}}, "permissions": [{"id": "show.observe"}]
        }).to_string()).unwrap();
        let found = discover(&web, "0.2.0");
        assert!(found.iter().all(|d| d.id != "demo"), "a bare script isn't a plugin");
        let n = found.iter().find(|d| d.id == "native_try").unwrap();
        assert!(n.error.as_deref().unwrap_or("").contains("native"), "{:?}", n.error);

        // Rows an older version recorded for the removed kinds are dropped; real plugins are not.
        let db = temp_db(tmp.path());
        for (id, kind) in [("legacy.old_plugin", "legacy-native"), ("demo", "legacy-web"), ("org.example.observer", "native")] {
            let observed = ObservedPlugin {
                plugin_id: id, kind, name: id, version: None, source_path: "x", manifest_sha256: None, digest: "d",
                signer: None, requested_json: "[]", error: None, files_json: "[]", signature_scope: "unsigned", rotation_from: None,
            };
            db.upsert_observed_plugin(&observed).unwrap();
        }
        write_sandbox_plugin(&web, "kept", "<html></html>");
        sync(&db, &discover(&web, "0.2.0"));
        for gone in ["legacy.old_plugin", "demo", "org.example.observer"] {
            assert!(db.get_plugin_grant(gone).unwrap().is_none(), "{gone}");
        }
        assert_eq!(db.get_plugin_grant("kept").unwrap().unwrap().effective_state(), "pending");
    }

    #[test]
    fn nothing_is_approved_automatically_and_a_changed_plugin_stops_until_reapproved() {
        let tmp = tempfile::tempdir().unwrap();
        let web = tmp.path().join("web_plugins");
        let db = temp_db(tmp.path());
        let dir = write_sandbox_plugin(&web, "thing", "<html>v1</html>");
        let first = discover(&web, "0.2.0");
        sync(&db, &first);
        assert_eq!(db.get_plugin_grant("thing").unwrap().unwrap().effective_state(), "pending", "new plugins always wait for the operator");

        // Approve what was seen.
        let d = first.iter().find(|d| d.id == "thing").unwrap();
        db.set_plugin_decision("thing", "approved", &requested_json(d), d.manifest_sha256.as_deref(), Some(&d.digest), None, "test").unwrap();
        assert_eq!(db.get_plugin_grant("thing").unwrap().unwrap().effective_state(), "active");

        // Change a file: a different digest, so it is `changed` and stops until it is reviewed again.
        std::fs::write(dir.join("index.html"), "<html>v2 -- swapped</html>").unwrap();
        sync(&db, &discover(&web, "0.2.0"));
        assert_eq!(db.get_plugin_grant("thing").unwrap().unwrap().effective_state(), "changed");
        let audit = db.list_plugin_audit(50).unwrap();
        assert!(audit.iter().any(|a| a.plugin_id == "thing" && a.action == "changed"));
    }

    fn write_versioned(web: &Path, id: &str, version: &str, perms: &str, html: &str, extra: &[(&str, &str)]) {
        let dir = web.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            format!(r#"{{"manifest_version":1,"id":"{id}","name":"V","version":"{version}","web":{{"entry":"index.html","sandbox":true}},"permissions":[{perms}]}}"#),
        ).unwrap();
        std::fs::write(dir.join("index.html"), html).unwrap();
        for (name, body) in extra {
            std::fs::write(dir.join(name), body).unwrap();
        }
    }

    #[test]
    fn an_update_is_described_as_a_difference_from_what_was_approved() {
        let tmp = tempfile::tempdir().unwrap();
        let web = tmp.path().join("plugins");
        let db = temp_db(tmp.path());
        let ui = r#"{"id":"ui.panel","reason":"x"}"#;
        write_versioned(&web, "up", "1.0.0", ui, "<html>v1</html>", &[("old.js", "1")]);
        sync(&db, &discover(&web, "0.2.0"));
        let row = db.get_plugin_grant("up").unwrap().unwrap();
        assert!(describe_update(&row).is_none(), "nothing to compare before approval");
        db.set_plugin_decision("up", "approved", &row.requested_json, row.observed_manifest_sha256.as_deref(), Some(&row.observed_digest), None, "t").unwrap();
        assert!(describe_update(&db.get_plugin_grant("up").unwrap().unwrap()).is_none(), "approved and unchanged");

        // v1.1.0: one file edited, one removed, one added.
        std::fs::remove_file(web.join("up/old.js")).unwrap();
        write_versioned(&web, "up", "1.1.0", ui, "<html>v2</html>", &[("new.js", "2")]);
        sync(&db, &discover(&web, "0.2.0"));
        let d = describe_update(&db.get_plugin_grant("up").unwrap().unwrap()).expect("changed plugins are described");
        assert_eq!((d["from_version"].as_str(), d["to_version"].as_str(), d["direction"].as_str()), (Some("1.0.0"), Some("1.1.0"), Some("upgrade")), "{d}");
        assert_eq!(d["files"]["modified"], serde_json::json!(["index.html", "plugin.json"]), "the manifest counts too: its version changed: {d}");
        assert_eq!(d["files"]["added"], serde_json::json!(["new.js"]));
        assert_eq!(d["files"]["removed"], serde_json::json!(["old.js"]));
        assert_eq!(d["permissions"]["added"], serde_json::json!([]));

        // Rolling back to an older version is flagged as a downgrade.
        write_versioned(&web, "up", "0.9.0", ui, "<html>v0</html>", &[]);
        sync(&db, &discover(&web, "0.2.0"));
        assert_eq!(describe_update(&db.get_plugin_grant("up").unwrap().unwrap()).unwrap()["direction"], "downgrade");

        // A version that isn't semver is refused outright.
        write_versioned(&web, "up", "latest", ui, "<html>v0</html>", &[]);
        let found = discover(&web, "0.2.0");
        assert!(found.iter().find(|d| d.id == "up").unwrap().error.as_deref().unwrap_or("").contains("semantic version"));
    }

    #[test]
    fn a_new_permission_in_an_update_is_reported_with_its_risk_and_declined_optionals_are_not() {
        let tmp = tempfile::tempdir().unwrap();
        let web = tmp.path().join("plugins");
        let db = temp_db(tmp.path());
        let base = r#"{"id":"ui.panel","reason":"x"},{"id":"notifications.post","optional":true,"reason":"x"}"#;
        write_versioned(&web, "np", "1.0.0", base, "<html>v1</html>", &[]);
        sync(&db, &discover(&web, "0.2.0"));
        let row = db.get_plugin_grant("np").unwrap().unwrap();
        // The operator approves only ui.panel, declining the optional one.
        db.set_plugin_decision("np", "approved", r#"[{"id":"ui.panel"}]"#, None, Some(&row.observed_digest), None, "t").unwrap();
        write_versioned(&web, "np", "1.0.1", &format!(r#"{base},{{"id":"keyring.own","reason":"y"}}"#), "<html>v2</html>", &[]);
        sync(&db, &discover(&web, "0.2.0"));
        let d = describe_update(&db.get_plugin_grant("np").unwrap().unwrap()).unwrap();
        assert_eq!(d["permissions"]["added"], serde_json::json!(["keyring.own"]), "{d}");
        assert_eq!(d["permissions"]["added_tiers"], serde_json::json!([{"id": "keyring.own", "tier": "medium"}]));
    }

    /// A sandboxed plugin folder, optionally listing its files in the manifest and signing the manifest.
    /// Returns the folder. `after_signing` runs last, to tamper.
    fn write_package(web: &Path, list_files: bool, sign: bool, after_signing: impl Fn(&Path)) -> std::path::PathBuf {
        let dir = web.join("pkg");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("index.html"), "<html>page</html>").unwrap();
        std::fs::write(dir.join("sub/app.js"), "var a = 1;").unwrap();
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let mut manifest = serde_json::json!({
            "manifest_version": 1, "id": "pkg", "name": "Pkg", "version": "1.0.0",
            "web": {"entry": "index.html", "sandbox": true}, "permissions": [{"id": "ui.panel"}]
        });
        if sign {
            manifest["publisher_key"] = serde_json::json!(kp.pk.to_base64());
        }
        if list_files {
            let hash = |name: &str| sha256_file(&dir.join(name)).unwrap();
            manifest["files"] = serde_json::json!({ "index.html": hash("index.html"), "sub/app.js": hash("sub/app.js") });
        }
        let text = manifest.to_string();
        std::fs::write(dir.join("plugin.json"), &text).unwrap();
        if sign {
            let sig = minisign::sign(None, &kp.sk, text.as_bytes(), None, None).unwrap().into_string();
            std::fs::write(dir.join("plugin.json.minisig"), sig).unwrap();
        }
        after_signing(&dir);
        dir
    }

    fn scope_and_error(web: &Path) -> (&'static str, Option<String>) {
        let found = discover(web, "0.2.0");
        let d = found.iter().find(|d| d.id == "pkg").unwrap();
        (d.signature_scope, d.error.clone())
    }

    #[test]
    fn what_a_signature_covers_is_unsigned_manifest_only_or_full() {
        for (list, sign, expected) in [(false, false, "unsigned"), (false, true, "manifest"), (true, true, "full"), (true, false, "unsigned")] {
            let tmp = tempfile::tempdir().unwrap();
            let web = tmp.path().join("plugins");
            write_package(&web, list, sign, |_| {});
            assert_eq!(scope_and_error(&web), (expected, None), "files listed: {list}, signed: {sign}");
        }
    }

    #[test]
    fn a_fully_signed_package_that_was_tampered_with_is_refused_not_downgraded() {
        let cases: Vec<(&str, Box<dyn Fn(&Path)>, &str)> = vec![
            ("a file swapped after signing", Box::new(|d| std::fs::write(d.join("sub/app.js"), "steal();").unwrap()), "different from what was signed"),
            ("an extra file dropped in", Box::new(|d| std::fs::write(d.join("evil.js"), "x").unwrap()), "not listed in files: evil.js"),
            ("a listed file deleted", Box::new(|d| std::fs::remove_file(d.join("sub/app.js")).unwrap()), "listed but missing: sub/app.js"),
            ("a file in a new subfolder", Box::new(|d| { std::fs::create_dir_all(d.join("x")).unwrap(); std::fs::write(d.join("x/y.js"), "x").unwrap(); }), "not listed in files: x/y.js"),
        ];
        for (why, tamper, expect) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let web = tmp.path().join("plugins");
            write_package(&web, true, true, tamper);
            let (_, err) = scope_and_error(&web);
            let err = err.unwrap_or_else(|| panic!("{why}: must be refused"));
            assert!(err.contains("doesn't match its manifest") && err.contains(expect), "{why}: {err}");
        }
    }

    #[test]
    fn the_signature_and_manifest_files_are_never_listed_and_listed_paths_are_contained() {
        let h = "a".repeat(64);
        let actual = vec![("index.html".to_string(), h.clone()), ("plugin.json".to_string(), "x".into()), ("plugin.json.minisig".to_string(), "y".into())];
        let listed = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<std::collections::BTreeMap<_, _>>();
        // The manifest and its signature are outside the list, so a correct list passes.
        assert!(check_listed_files(Some(&listed(&[("index.html", &h)])), &actual).is_ok());
        assert!(check_listed_files(None, &actual).is_ok(), "no list, nothing to check");
        for bad in ["../x", "/etc/passwd", "a\\b", "plugin.json", "plugin.json.minisig", ""] {
            let err = check_listed_files(Some(&listed(&[("index.html", &h), (bad, &h)])), &actual).unwrap_err();
            assert!(err.contains("can't be listed"), "{bad}: {err}");
        }
        assert!(check_listed_files(Some(&listed(&[("index.html", "nothex")])), &actual).unwrap_err().contains("isn't a SHA-256"));
        // Hash case doesn't matter.
        assert!(check_listed_files(Some(&listed(&[("index.html", &h.to_uppercase())])), &actual).is_ok());
    }

    #[test]
    fn listed_hashes_are_checked_even_without_a_signature() {
        let tmp = tempfile::tempdir().unwrap();
        let web = tmp.path().join("plugins");
        write_package(&web, true, false, |d| std::fs::write(d.join("index.html"), "<html>corrupted</html>").unwrap());
        let (_, err) = scope_and_error(&web);
        assert!(err.unwrap().contains("don't match the hashes"));
    }

    #[test]
    fn a_signature_is_only_as_strong_as_its_scope_when_it_comes_to_the_review_screen() {
        // Approved fully signed, then re-released as manifest-only signed: flagged as weaker.
        let tmp = tempfile::tempdir().unwrap();
        let web = tmp.path().join("plugins");
        let db = temp_db(tmp.path());
        write_package(&web, true, true, |_| {});
        sync(&db, &discover(&web, "0.2.0"));
        let row = db.get_plugin_grant("pkg").unwrap().unwrap();
        assert_eq!(row.observed_signature_scope.as_deref(), Some("full"));
        db.set_plugin_decision("pkg", "approved", &row.requested_json, row.observed_manifest_sha256.as_deref(), Some(&row.observed_digest), row.observed_signer.as_deref(), "t").unwrap();
        assert_eq!(db.get_plugin_grant("pkg").unwrap().unwrap().approved_signature_scope.as_deref(), Some("full"));
        std::fs::remove_dir_all(web.join("pkg")).unwrap();
        write_package(&web, false, true, |_| {});
        sync(&db, &discover(&web, "0.2.0"));
        let row = db.get_plugin_grant("pkg").unwrap().unwrap();
        assert_eq!((row.approved_signature_scope.as_deref(), row.observed_signature_scope.as_deref()), (Some("full"), Some("manifest")));
    }

    #[test]
    fn an_oversized_manifest_is_rejected_unparsed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("plugins").join("big");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<html></html>").unwrap();
        let manifest = serde_json::json!({
            "manifest_version": 1, "id": "big", "name": "Big", "web": {"entry": "index.html", "sandbox": true},
            "permissions": [{"id": "ui.panel"}], "description": "x".repeat(MAX_MANIFEST_BYTES + 1)
        });
        std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
        let found = discover(&tmp.path().join("plugins"), "0.2.0");
        let err = found.iter().find(|d| d.id == "big").unwrap().error.clone().unwrap_or_default();
        assert!(err.contains("the limit is"), "{err}");
    }

    #[test]
    fn a_ribbon_button_icon_must_exist_in_the_icon_set() {
        let tmp = tempfile::tempdir().unwrap();
        let web = tmp.path().join("web");
        let plugin_dir = web.join("plugins").join("tb");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::create_dir_all(web.join("icons").join("crimson")).unwrap();
        std::fs::write(web.join("icons/crimson/refresh.svg"), "<svg/>").unwrap();
        std::fs::write(plugin_dir.join("index.html"), "<html></html>").unwrap();
        let write = |icon: &str| {
            std::fs::write(
                plugin_dir.join("plugin.json"),
                serde_json::json!({
                    "manifest_version": 1, "id": "tb", "name": "Tb", "web": {"entry": "index.html", "sandbox": true},
                    "permissions": [{"id": "ui.toolbar"}],
                    "ui": {"toolbar": [{"id": "go", "label": "Sync", "icon": icon}]}
                }).to_string(),
            ).unwrap();
        };
        write("refresh");
        let found = discover(&web.join("plugins"), "0.2.0");
        assert!(found.iter().find(|d| d.id == "tb").unwrap().error.is_none());
        write("not-an-icon");
        let found = discover(&web.join("plugins"), "0.2.0");
        let err = found.iter().find(|d| d.id == "tb").unwrap().error.clone().unwrap_or_default();
        assert!(err.contains("isn't in the app's icon set"), "{err}");
    }

    fn write_signed_web_plugin(web: &Path, manifest_edit: impl Fn(String) -> String) -> String {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let pk = kp.pk.to_base64();
        let manifest = serde_json::json!({
            "manifest_version": 1, "id": "signed_demo", "name": "Signed", "publisher_key": pk,
            "web": {"entry": "signed_demo.js"}, "permissions": [{"id": "keyring.own"}]
        })
        .to_string();
        std::fs::write(web.join("signed_demo.js"), b"export default {}").unwrap();
        let sig = minisign::sign(None, &kp.sk, manifest.as_bytes(), None, None).unwrap().into_string();
        std::fs::write(web.join("signed_demo.plugin.json.minisig"), sig).unwrap();
        // Written after signing, so an edit makes the signature stale.
        std::fs::write(web.join("signed_demo.plugin.json"), manifest_edit(manifest)).unwrap();
        pk
    }

    #[test]
    fn a_valid_signature_is_recorded_and_a_stale_one_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let web = tmp.path().join("web_plugins");
        std::fs::create_dir_all(&web).unwrap();
        let pk = write_signed_web_plugin(&web, |m| m);
        let found = discover(&web, "0.2.0");
        let d = found.iter().find(|d| d.id == "signed_demo").unwrap();
        assert!(d.error.is_none(), "{:?}", d.error);
        assert_eq!(d.signer.as_deref(), Some(pk.as_str()));

        // Tamper with the manifest after signing: it must not degrade to "unsigned".
        write_signed_web_plugin(&web, |m| m.replace("keyring.own", "ui.dom"));
        let found = discover(&web, "0.2.0");
        let d = found.iter().find(|d| d.id == "signed_demo").unwrap();
        assert!(d.error.as_deref().unwrap_or("").contains("signature"), "{:?}", d.error);
    }

    #[test]
    fn a_signature_without_a_publisher_key_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let web = tmp.path().join("web_plugins");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("p.js"), b"x").unwrap();
        std::fs::write(
            web.join("p.plugin.json"),
            r#"{"manifest_version":1,"id":"p","name":"P","web":{"entry":"p.js"},"permissions":[]}"#,
        )
        .unwrap();
        std::fs::write(web.join("p.plugin.json.minisig"), "untrusted comment: x\nAAAA\n").unwrap();
        let found = discover(&web, "0.2.0");
        assert!(found.iter().find(|d| d.id == "p").unwrap().error.is_some());
    }

    #[test]
    fn is_builtin_is_decided_by_folder_location_not_by_anything_about_the_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let installed = user_plugins_dir(&data_dir).join("from_operator");
        std::fs::create_dir_all(&installed).unwrap();
        let shipped = tmp.path().join("web").join("plugins").join("shipped");
        std::fs::create_dir_all(&shipped).unwrap();

        assert!(!is_builtin(&data_dir, &installed));
        assert!(is_installed_folder(&data_dir, &installed));
        assert!(is_builtin(&data_dir, &shipped), "anywhere outside <data dir>/plugins/* counts as built-in");
        assert!(!is_installed_folder(&data_dir, &shipped));
        // A folder that doesn't exist (e.g. a stale row after uninstall) can't canonicalize --
        // the safe reading is "not installed", i.e. built-in, never the reverse.
        assert!(is_builtin(&data_dir, &data_dir.join("plugins").join("gone")));
    }

    #[test]
    fn version_floor_only_ever_rises_and_is_scoped_per_signer() {
        let tmp = tempfile::tempdir().unwrap();
        let db = temp_db(tmp.path());

        assert_eq!(version_floor(&db, "p", Some("KEY_A")), None);
        remember_approved_version(&db, "p", Some("KEY_A"), Some("2.0.0"));
        assert_eq!(version_floor(&db, "p", Some("KEY_A")).as_deref(), Some("2.0.0"));

        // Re-approving a lower version (e.g. after a revoke) must not lower the floor.
        remember_approved_version(&db, "p", Some("KEY_A"), Some("1.0.0"));
        assert_eq!(version_floor(&db, "p", Some("KEY_A")).as_deref(), Some("2.0.0"), "the floor only ever rises");

        // A genuine upgrade raises it.
        remember_approved_version(&db, "p", Some("KEY_A"), Some("2.1.0"));
        assert_eq!(version_floor(&db, "p", Some("KEY_A")).as_deref(), Some("2.1.0"));

        // A different signer (or unsigned) starts its own floor: an unrelated key's releases
        // say nothing about what to expect from this one.
        assert_eq!(version_floor(&db, "p", Some("KEY_B")), None);
        assert_eq!(version_floor(&db, "p", None), None);
        remember_approved_version(&db, "p", None, Some("0.1.0"));
        assert_eq!(version_floor(&db, "p", None).as_deref(), Some("0.1.0"));
        assert_eq!(version_floor(&db, "p", Some("KEY_A")).as_deref(), Some("2.1.0"), "unaffected by the unsigned floor");

        // Garbage versions are ignored rather than parsed as "anything, so always lower".
        remember_approved_version(&db, "p", Some("KEY_A"), Some("not-a-version"));
        assert_eq!(version_floor(&db, "p", Some("KEY_A")).as_deref(), Some("2.1.0"));
        remember_approved_version(&db, "p", Some("KEY_A"), None);
        assert_eq!(version_floor(&db, "p", Some("KEY_A")).as_deref(), Some("2.1.0"));
    }

    #[test]
    fn downgrade_from_floor_is_only_reported_for_two_real_comparable_versions() {
        assert!(is_downgrade_from_floor(Some("2.0.0"), Some("1.0.0")));
        assert!(!is_downgrade_from_floor(Some("2.0.0"), Some("2.0.0")));
        assert!(!is_downgrade_from_floor(Some("2.0.0"), Some("3.0.0")));
        assert!(!is_downgrade_from_floor(None, Some("1.0.0")), "nothing remembered yet is not a downgrade");
        assert!(!is_downgrade_from_floor(Some("2.0.0"), None));
        assert!(!is_downgrade_from_floor(Some("not-semver"), Some("1.0.0")));
    }
}
