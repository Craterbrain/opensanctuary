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
    }
}

fn discover_web(web_plugins_dir: &Path, host_version: &str) -> Vec<Discovered> {
    // Ribbon-button icons are names of files in the app's icon set, `web/icons/crimson/` next to the plugins folder.
    let icons_dir = web_plugins_dir.parent().map(|w| w.join("icons").join("crimson"));
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(web_plugins_dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(d) = discover_sandbox(&path, host_version, icons_dir.as_deref()) {
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
            d.digest = artifact_digest(&files);
            Some(d)
        }
    }
}

/// Scans the web plugins folder.
pub fn discover(web_plugins_dir: &Path, host_version: &str) -> Vec<Discovered> {
    let mut out = Vec::new();
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
                signer: None, requested_json: "[]", error: None,
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
}
