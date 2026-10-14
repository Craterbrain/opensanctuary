//! Finds plugins on disk, records what it saw in `plugin_grants`, and decides which
//! native libraries are allowed to load (docs/PLUGIN_PERMISSIONS.md).
//!
//! A plugin is loaded/served only while it is `approved` *and* what is on disk still
//! hashes to what the operator reviewed; any change drops it to `changed` until it is
//! re-reviewed.

use std::path::{Path, PathBuf};

use crate::core::plugin_permissions::{
    artifact_digest, legacy_requests, platform_key, sha256_file, sha256_hex, PermissionRequest, PermissionSet,
    PluginManifest,
};
use crate::core::plugins::ApprovedNative;
use crate::storage::db::{ObservedPlugin, PluginGrantRow};
use crate::storage::Database;

/// Settings key set once the first post-upgrade scan has run, so libraries that were
/// already installed before permissions existed keep working exactly once.
const GRANDFATHER_KEY: &str = "plugins.grandfathered_v1";

pub const LEGACY_WEB_REQUESTS: [&str; 2] = ["ui.dom", "keyring.own"];

#[derive(Debug, Clone)]
pub struct Discovered {
    pub id: String,
    /// `native`, `web`, `legacy-native` or `legacy-web`.
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
    /// Native library to load, when this build's platform has one.
    pub library: Option<PathBuf>,
    /// Plugin ABI the manifest declares (legacy libraries are 1).
    pub abi: u32,
}

fn sanitize_legacy_id(stem: &str) -> String {
    let s: String = stem
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    format!("legacy.{s}")
}

#[cfg(unix)]
fn group_or_world_writable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).map(|m| m.permissions().mode() & 0o022 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn group_or_world_writable(_path: &Path) -> bool {
    false
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
        library: None,
        abi: 1,
    }
}

fn discover_native_dir(dir: &Path, host_version: &str) -> Option<Discovered> {
    let dir_name = dir.file_name()?.to_str()?.to_string();
    let manifest_path = dir.join("plugin.json");
    let bytes = match std::fs::read(&manifest_path) {
        Ok(b) => b,
        Err(_) => return None, // not a plugin directory
    };
    let manifest_sha = sha256_hex(&bytes);
    let manifest = match PluginManifest::parse(&bytes).and_then(|m| m.validate(host_version).map(|_| m)) {
        Ok(m) => m,
        Err(e) => {
            let mut d = failed(dir_name, "native", dir, e.to_string());
            d.manifest_sha256 = Some(manifest_sha);
            return Some(d);
        }
    };
    let signer = match verify_signer(&bytes, &manifest, &dir.join("plugin.json.minisig")) {
        Ok(s) => s,
        Err(e) => {
            let mut d = failed(dir_name, "native", dir, e);
            d.manifest_sha256 = Some(manifest_sha);
            return Some(d);
        }
    };
    let mut d = Discovered {
        id: manifest.id.clone(),
        kind: "native",
        name: manifest.name.clone(),
        version: Some(manifest.version.clone()).filter(|v| !v.is_empty()),
        source_path: dir.to_string_lossy().to_string(),
        manifest_sha256: Some(manifest_sha),
        digest: String::new(),
        signer,
        requested: manifest.permissions.clone(),
        error: None,
        library: None,
        abi: manifest.backend.as_ref().map(|b| b.abi.max(1)).unwrap_or(1),
    };
    let fail = |mut d: Discovered, msg: String| {
        d.digest = sha256_hex(format!("{}{}", d.manifest_sha256.clone().unwrap_or_default(), msg).as_bytes());
        d.error = Some(msg);
        d
    };
    if manifest.id != dir_name {
        return Some(fail(d, format!("plugin id '{}' must match its directory name '{}'", manifest.id, dir_name)));
    }
    let Some(backend) = manifest.backend.as_ref() else {
        return Some(fail(d, "this plugin has no native library; web plugins are installed in the application's plugins folder".to_string()));
    };
    let key = platform_key();
    let Some(entry) = backend.libraries.get(&key) else {
        return Some(fail(d, format!("no library declared for this platform ({key})")));
    };
    let lib_path = dir.join(&entry.path);
    // Symlinks could point the pinned path somewhere else after approval.
    match std::fs::symlink_metadata(&lib_path) {
        Ok(m) if m.is_file() => {}
        _ => return Some(fail(d, format!("library {} is missing or not a regular file", entry.path))),
    }
    if group_or_world_writable(dir) || group_or_world_writable(&lib_path) {
        return Some(fail(d, "plugin files must not be group- or world-writable".to_string()));
    }
    let actual = match sha256_file(&lib_path) {
        Ok(h) => h,
        Err(e) => return Some(fail(d, format!("could not read {}: {e}", entry.path))),
    };
    if !actual.eq_ignore_ascii_case(&entry.sha256) {
        return Some(fail(d, format!("{} does not match the sha256 in plugin.json", entry.path)));
    }
    d.digest = artifact_digest(&[(entry.path.clone(), actual)]);
    d.library = Some(lib_path);
    Some(d)
}

fn discover_web(web_plugins_dir: &Path, host_version: &str) -> Vec<Discovered> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(web_plugins_dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(d) = discover_sandbox(&path, host_version) {
                out.push(d);
            }
            continue;
        }
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("js") {
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
                        digest: artifact_digest(&[(file_name.clone(), file_sha)]),
                        signer: signer.clone(),
                        requested: m.permissions.clone(),
                        error: problem,
                        library: None,
                        abi: 1,
                    });
                }
            }
        } else {
            // No manifest: runs in the console's own page, so it is treated as asking for
            // everything that implies.
            out.push(Discovered {
                id: sanitize_legacy_id(&stem).replacen("legacy.", "", 1),
                kind: "legacy-web",
                name: stem.clone(),
                version: None,
                source_path: path.to_string_lossy().to_string(),
                manifest_sha256: None,
                digest: artifact_digest(&[(file_name, file_sha)]),
                signer: None,
                requested: LEGACY_WEB_REQUESTS.iter().map(|id| PermissionRequest::simple(id)).collect(),
                error: None,
                library: None,
                abi: 1,
            });
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

fn discover_sandbox(dir: &Path, host_version: &str) -> Option<Discovered> {
    let dir_name = dir.file_name()?.to_str()?.to_string();
    let bytes = std::fs::read(dir.join("plugin.json")).ok()?; // not a plugin folder
    let manifest_sha = sha256_hex(&bytes);
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
        library: None,
        abi: 1,
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
    match sandbox_dir_files(dir) {
        Err(e) => Some(fail(d, e)),
        Ok(files) => {
            if !files.iter().any(|(rel, _)| *rel == web.entry) {
                return Some(fail(d, format!("entry page '{}' is missing", web.entry)));
            }
            d.digest = artifact_digest(&files);
            Some(d)
        }
    }
}

/// Scans the native plugin directory and the web plugins folder.
pub fn discover(plugin_dir: &Path, web_plugins_dir: &Path, host_version: &str) -> Vec<Discovered> {
    let mut out = Vec::new();
    if !plugin_dir.exists() {
        let _ = std::fs::create_dir_all(plugin_dir);
    }
    if let Ok(entries) = std::fs::read_dir(plugin_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(d) = discover_native_dir(&path, host_version) {
                    out.push(d);
                }
            } else if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "so" | "dll" | "dylib") {
                    let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()) else { continue };
                    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("plugin").to_string();
                    match sha256_file(&path) {
                        Ok(sha) => out.push(Discovered {
                            id: sanitize_legacy_id(&stem),
                            kind: "legacy-native",
                            name: file_name.clone(),
                            version: None,
                            source_path: path.to_string_lossy().to_string(),
                            manifest_sha256: None,
                            digest: artifact_digest(&[(file_name, sha)]),
                            signer: None,
                            requested: legacy_requests(),
                            error: None,
                            library: Some(path),
                            abi: 1,
                        }),
                        Err(e) => out.push(failed(sanitize_legacy_id(&stem), "legacy-native", &path, format!("unreadable: {e}"))),
                    }
                }
            }
        }
    }
    out.extend(discover_web(web_plugins_dir, host_version));
    // A duplicate id is ambiguous; neither copy may run.
    let mut counts = std::collections::HashMap::new();
    for d in &out {
        *counts.entry(d.id.clone()).or_insert(0usize) += 1;
    }
    for d in out.iter_mut() {
        if counts[&d.id] > 1 && d.error.is_none() {
            d.error = Some(format!("another plugin also uses the id '{}'", d.id));
        }
    }
    out
}

fn requested_json(d: &Discovered) -> String {
    serde_json::to_string(&d.requested).unwrap_or_else(|_| "[]".to_string())
}

/// Records the scan in the database. Libraries already installed when permissions first
/// shipped are approved once, with an audit entry; anything new is `pending`.
pub fn sync(db: &Database, discovered: &[Discovered]) {
    let grandfather = db.get_setting(GRANDFATHER_KEY).ok().flatten().is_none();
    let mut grandfathered = 0;
    for d in discovered {
        let existing = db.get_plugin_grant(&d.id).ok().flatten();
        let (status, granted) = match &existing {
            Some(_) => ("pending", "[]".to_string()), // ignored by the upsert for existing rows
            None if grandfather && d.kind == "legacy-native" && d.error.is_none() => {
                grandfathered += 1;
                ("approved", requested_json(d))
            }
            None => ("pending", "[]".to_string()),
        };
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
        };
        if db.upsert_observed_plugin(&observed, status, &granted).is_err() {
            continue;
        }
        match &existing {
            None if status == "approved" => db.add_plugin_audit(&d.id, "system", "grandfathered", "{}"),
            None => db.add_plugin_audit(&d.id, "system", "discovered", &serde_json::json!({ "kind": d.kind }).to_string()),
            Some(row) if row.observed_digest != d.digest && row.status == "approved" => {
                db.add_plugin_audit(&d.id, "system", "changed", &serde_json::json!({ "digest": d.digest }).to_string())
            }
            _ => {}
        }
    }
    if grandfather {
        let _ = db.set_setting(GRANDFATHER_KEY, "1");
        if grandfathered > 0 {
            tracing::info!("Approved {grandfathered} pre-existing native plugin(s) during the permissions upgrade; review them in Settings > Plugins");
        }
    }
}

pub fn parse_granted(row: &PluginGrantRow) -> PermissionSet {
    PermissionSet(serde_json::from_str::<Vec<PermissionRequest>>(&row.granted_json).unwrap_or_default())
}

/// Native libraries that may load right now: approved, unchanged on disk, valid.
pub fn approved_natives(db: &Database, discovered: &[Discovered]) -> Vec<ApprovedNative> {
    discovered
        .iter()
        .filter(|d| d.error.is_none())
        .filter_map(|d| {
            let lib = d.library.clone()?;
            let row = db.get_plugin_grant(&d.id).ok().flatten()?;
            (row.effective_state() == "active").then(|| ApprovedNative { id: d.id.clone(), library: lib, grants: parse_granted(&row), abi: d.abi })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(dir: &Path) -> Database {
        Database::new(dir.join("t.db").to_str().unwrap()).unwrap()
    }

    #[test]
    fn grandfathers_existing_legacy_libraries_exactly_once() {
        let tmp = tempfile::tempdir().unwrap();
        let plugins = tmp.path().join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("Old Plugin.so"), b"not really a library").unwrap();
        let db = temp_db(tmp.path());

        let first = discover(&plugins, &tmp.path().join("web"), "0.2.0");
        sync(&db, &first);
        let row = db.get_plugin_grant("legacy.old_plugin").unwrap().unwrap();
        assert_eq!(row.effective_state(), "active");
        assert_eq!(row.decided_by.as_deref(), Some("system:grandfathered"));
        assert_eq!(approved_natives(&db, &first).len(), 1);

        // A library that shows up after the upgrade must be reviewed first.
        std::fs::write(plugins.join("new.so"), b"another").unwrap();
        let second = discover(&plugins, &tmp.path().join("web"), "0.2.0");
        sync(&db, &second);
        assert_eq!(db.get_plugin_grant("legacy.new").unwrap().unwrap().effective_state(), "pending");
        assert_eq!(approved_natives(&db, &second).len(), 1);
    }

    #[test]
    fn a_changed_binary_stops_loading_until_reapproved() {
        let tmp = tempfile::tempdir().unwrap();
        let plugins = tmp.path().join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        let lib = plugins.join("thing.so");
        std::fs::write(&lib, b"v1").unwrap();
        let db = temp_db(tmp.path());
        sync(&db, &discover(&plugins, &tmp.path().join("web"), "0.2.0"));
        assert_eq!(db.get_plugin_grant("legacy.thing").unwrap().unwrap().effective_state(), "active");

        std::fs::write(&lib, b"v2 -- swapped").unwrap();
        let after = discover(&plugins, &tmp.path().join("web"), "0.2.0");
        sync(&db, &after);
        assert_eq!(db.get_plugin_grant("legacy.thing").unwrap().unwrap().effective_state(), "changed");
        assert!(approved_natives(&db, &after).is_empty());
        let audit = db.list_plugin_audit(50).unwrap();
        assert!(audit.iter().any(|a| a.plugin_id == "legacy.thing" && a.action == "changed"));
    }

    #[test]
    fn manifest_plugin_must_match_its_pinned_library() {
        let tmp = tempfile::tempdir().unwrap();
        let plugins = tmp.path().join("plugins");
        let dir = plugins.join("org.example.good");
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("lib/x.so"), b"binary").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::set_permissions(dir.join("lib/x.so"), std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let manifest = |sha: &str| {
            serde_json::json!({
                "manifest_version": 1, "id": "org.example.good", "name": "Good", "version": "1.0.0",
                "backend": {"abi": 1, "libraries": { platform_key(): {"path": "lib/x.so", "sha256": sha}}},
                "permissions": [{"id": "show.observe"}]
            })
            .to_string()
        };
        std::fs::write(dir.join("plugin.json"), manifest(&sha256_hex(b"binary"))).unwrap();
        let found = discover(&plugins, &tmp.path().join("web"), "0.2.0");
        let d = found.iter().find(|d| d.id == "org.example.good").unwrap();
        assert!(d.error.is_none(), "{:?}", d.error);
        assert!(d.library.is_some());

        std::fs::write(dir.join("plugin.json"), manifest(&sha256_hex(b"something else"))).unwrap();
        let found = discover(&plugins, &tmp.path().join("web"), "0.2.0");
        assert!(found.iter().find(|d| d.id == "org.example.good").unwrap().error.is_some());
    }

    #[test]
    fn web_plugins_without_a_manifest_are_treated_as_full_console_access() {
        let tmp = tempfile::tempdir().unwrap();
        let web = tmp.path().join("web_plugins");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("demo.js"), b"export default {}").unwrap();
        let found = discover(&tmp.path().join("plugins"), &web, "0.2.0");
        let d = found.iter().find(|d| d.id == "demo").unwrap();
        assert_eq!(d.kind, "legacy-web");
        assert!(d.requested.iter().any(|p| p.id == "ui.dom"));
        let db = temp_db(tmp.path());
        sync(&db, &found);
        // Never grandfathered: it runs in the console's page.
        assert_eq!(db.get_plugin_grant("demo").unwrap().unwrap().effective_state(), "pending");
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
        let found = discover(&tmp.path().join("plugins"), &web, "0.2.0");
        let d = found.iter().find(|d| d.id == "signed_demo").unwrap();
        assert!(d.error.is_none(), "{:?}", d.error);
        assert_eq!(d.signer.as_deref(), Some(pk.as_str()));

        // Tamper with the manifest after signing: it must not degrade to "unsigned".
        write_signed_web_plugin(&web, |m| m.replace("keyring.own", "ui.dom"));
        let found = discover(&tmp.path().join("plugins"), &web, "0.2.0");
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
        let found = discover(&tmp.path().join("plugins"), &web, "0.2.0");
        assert!(found.iter().find(|d| d.id == "p").unwrap().error.is_some());
    }
}
