//! Plugins: grants and scoped tokens, the host API each bundled plugin uses, sandboxed serving, install/remove, signing, trust and revocation.

use super::*;

const H: &str = "sig3_host";

struct Harness {
    base: String,
    c: reqwest::Client,
    web_dir: std::path::PathBuf,
    data_dir: std::path::PathBuf,
    db: os_next::storage::Database,
    _tmp: tempfile::TempDir,
}

fn write_revocation_list(dir: &std::path::Path, signer: &TestKey, sequence: u64, revoke: &[(&str, &str)]) {
    let list = serde_json::json!({ "sequence": sequence, "revoked": revoke.iter().map(|(k, r)| serde_json::json!({ "key": k, "reason": r, "since": "2026-10-14" })).collect::<Vec<_>>() }).to_string();
    std::fs::write(dir.join("revoked_keys.json"), &list).unwrap();
    std::fs::write(dir.join("revoked_keys.json.minisig"), sign_bytes(signer, list.as_bytes())).unwrap();
}

/// A plugin package signed by `key` (manifest lists every file), plus any extra entries (e.g. a rotation statement).
fn signed_zip(id: &str, version: &str, key: &TestKey, page: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let manifest = serde_json::json!({
        "manifest_version": 1, "id": id, "name": format!("Plugin {id}"), "version": version, "publisher": "Whoever",
        "publisher_key": key.b64, "web": {"entry": "index.html", "sandbox": true},
        "permissions": [{"id": "ui.panel", "reason": "x"}], "ui": {"placements": [{"slot": "dock"}]},
        "files": { "index.html": os_next::core::plugin_permissions::sha256_hex(page.as_bytes()) }
    }).to_string();
    let sig = sign_bytes(key, manifest.as_bytes());
    let mut entries: Vec<(&str, &[u8])> = vec![("plugin.json", manifest.as_bytes()), ("plugin.json.minisig", sig.as_bytes()), ("index.html", page.as_bytes())];
    entries.extend_from_slice(extra);
    make_zip(&entries)
}

fn sign_bytes(key: &TestKey, data: &[u8]) -> String {
    minisign::sign(None, &key.sk, data, None, None).unwrap().into_string()
}

fn signed_package(id: &str, publisher: &str, kp: &minisign::KeyPair) -> Vec<u8> {
    let manifest = serde_json::json!({
        "manifest_version": 1, "id": id, "name": format!("Plugin {id}"), "version": "1.0.0", "publisher": publisher,
        "publisher_key": kp.pk.to_base64(), "web": {"entry": "index.html", "sandbox": true},
        "permissions": [{"id": "ui.panel", "reason": "x"}], "ui": {"placements": [{"slot": "dock"}]}
    }).to_string();
    let sig = minisign::sign(None, &kp.sk, manifest.as_bytes(), None, None).unwrap().into_string();
    make_zip(&[("plugin.json", manifest.as_bytes()), ("plugin.json.minisig", sig.as_bytes()), ("index.html", b"<html></html>")])
}

fn plugin_manifest_json(id: &str) -> String {
    serde_json::json!({
        "manifest_version": 1, "id": id, "name": format!("Plugin {id}"), "version": "1.0.0",
        "web": {"entry": "index.html", "sandbox": true},
        "permissions": [{"id": "ui.panel", "reason": "x"}, {"id": "keyring.own", "reason": "x"}],
        "ui": {"placements": [{"slot": "dock"}]}
    }).to_string()
}

#[tokio::test]
async fn test_app_state_plugin_token_lifecycle() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_tokens.db");
    let db = Database::new(&db_path).expect("Database::new");
    let event_log = Arc::new(EventLog::new(100));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _) = tokio::sync::broadcast::channel(16);

    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));

    let app = os_next::api::ws::AppState {
        engine,
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        tx,
        host_session_token: "test_host_token_abc".to_string(),
        ..base_app_state(db)
    };

    // Issue tokens for different plugins
    let token_obs = app.issue_plugin_token("obs_remote").await;
    let token_pco = app.issue_plugin_token("planning_center").await;

    assert!(token_obs.starts_with("plg_"));
    assert!(token_pco.starts_with("plg_"));
    assert_ne!(token_obs, token_pco);

    // Resolve tokens
    let resolved_obs = app.resolve_plugin_token(&token_obs).await;
    assert_eq!(resolved_obs, Some("obs_remote".to_string()));

    let resolved_pco = app.resolve_plugin_token(&token_pco).await;
    assert_eq!(resolved_pco, Some("planning_center".to_string()));

    // Unknown token returns None
    let resolved_unknown = app.resolve_plugin_token("plg_nonexistent").await;
    assert_eq!(resolved_unknown, None);
}

/// POST /api/keyring/plugin/token used to mint a valid, unrestricted token for
/// any plugin_name a caller asserted — including one that was never actually
/// installed, e.g. "planning_center" — letting anyone read that plugin's
/// vault the moment any *real* plugin's secret was ever stored under that
/// name.
#[tokio::test]
async fn test_plugin_token_requires_a_real_installed_plugin() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_plugin_token.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(3000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        // The real repo web dir — "hello_world" is a genuinely installed
        // plugin there (web/plugins/hello_world.js), loaded by main.ts.
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        host_session_token: "test_host_session_plugin".to_string(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // Minting a plugin token is console-only: an unauthenticated LAN caller is refused
    // even for a real installed plugin (it would unlock that plugin's keyring secrets).
    let anon_resp = client.post(format!("http://{}/api/keyring/plugin/token", addr))
        .json(&serde_json::json!({ "plugin_name": "hello_world" }))
        .send().await.unwrap();
    assert_eq!(anon_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // A made-up plugin name that was never installed is rejected.
    let fake_resp = client.post(format!("http://{}/api/keyring/plugin/token", addr))
        .header("x-host-token", "test_host_session_plugin")
        .json(&serde_json::json!({ "plugin_name": "planning_center" }))
        .send().await.unwrap();
    assert_eq!(fake_resp.status(), reqwest::StatusCode::NOT_FOUND);

    // A real, installed plugin that nobody has approved yet gets no token...
    let unapproved = client.post(format!("http://{}/api/keyring/plugin/token", addr))
        .header("x-host-token", "test_host_session_plugin")
        .json(&serde_json::json!({ "plugin_name": "hello_world" }))
        .send().await.unwrap();
    assert_eq!(unapproved.status(), reqwest::StatusCode::FORBIDDEN);

    // ...and once approved with keyring access it does.
    let found = os_next::core::plugin_registry::discover(std::path::Path::new("web/plugins"), env!("CARGO_PKG_VERSION"));
    os_next::core::plugin_registry::sync(&db, &found);
    let row = db.get_plugin_grant("hello_world").unwrap().unwrap();
    db.set_plugin_decision(
        "hello_world", "approved", r#"[{"id":"keyring.own"}]"#,
        row.observed_manifest_sha256.as_deref(), Some(&row.observed_digest), None, "test",
    ).unwrap();
    let real_resp = client.post(format!("http://{}/api/keyring/plugin/token", addr))
        .header("x-host-token", "test_host_session_plugin")
        .json(&serde_json::json!({ "plugin_name": "hello_world" }))
        .send().await.unwrap();
    assert_eq!(real_resp.status(), reqwest::StatusCode::OK);
    let real_json: serde_json::Value = real_resp.json().await.unwrap();
    assert_eq!(real_json["success"], true);
    assert!(real_json["token"].as_str().unwrap().starts_with("plg_"));
}

/// The console's grant routes: who may call them, digest pinning, and that approval/
/// change/revoke actually gate what the server will serve under /plugins/.
#[tokio::test]
async fn test_plugin_grant_routes_and_web_plugin_serving() {
    let test_dir = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(test_dir.path().join("plugins.db").to_str().unwrap()).unwrap();
    let web_dir = test_dir.path().join("web");
    std::fs::create_dir_all(web_dir.join("plugins")).unwrap();
    std::fs::write(web_dir.join("plugins/demo_gate.js"), b"export default { init() {} };").unwrap();
    // A web plugin needs a manifest; a bare script with no manifest is not loaded at all.
    std::fs::write(
        web_dir.join("plugins/demo_gate.plugin.json"),
        serde_json::json!({
            "manifest_version": 1, "id": "demo_gate", "name": "Demo gate", "web": {"entry": "demo_gate.js"},
            "permissions": [{"id": "ui.dom"}, {"id": "keyring.own"}]
        }).to_string(),
    ).unwrap();
    std::fs::write(web_dir.join("plugins/loose.js"), b"export default {}").unwrap();

    let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph: Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media"))),
        web_dir: web_dir.clone(),
        media_dir: test_dir.path().join("media"),
        host_session_token: "plugin_test_host_token".to_string(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let app_for_plugin_token = app_state.clone();
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let base = format!("http://{}", addr);
    let client = reqwest::Client::new();
    let host = "plugin_test_host_token";

    // A paired remote, a persistent console credential and a plugin token.
    let remote_token = "dev_tok_remote_for_plugin_test";
    db.upsert_paired_device(&os_next::storage::PairedDevice {
        id: "remote_x".into(), name: "Remote".into(), platform: "mobile-remote".into(),
        paired_at: 0, last_seen: 0, token: remote_token.into(),
    }).unwrap();
    let console_cred = "console_tok_for_plugin_test";
    db.upsert_paired_device(&os_next::storage::PairedDevice {
        id: "console_x".into(), name: "Laptop".into(), platform: "console".into(),
        paired_at: 0, last_seen: 0, token: os_next::storage::db::console_credential_hash(console_cred),
    }).unwrap();
    let plugin_token = app_for_plugin_token.issue_plugin_token("demo_gate").await;

    // Only a console may list or decide: no credential, a remote's device token and a
    // plugin token are all refused.
    for headers in [vec![], vec![("x-device-token", remote_token)], vec![("x-host-token", remote_token)], vec![("x-plugin-token", plugin_token.as_str())]] {
        let mut r = client.get(format!("{base}/api/plugins"));
        for (k, v) in &headers { r = r.header(*k, *v); }
        assert_eq!(r.send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED, "{headers:?}");
    }
    let resp = client.post(format!("{base}/api/plugins/demo_gate/approve")).header("x-plugin-token", &plugin_token)
        .json(&serde_json::json!({ "digest": "x", "granted": [] })).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED, "a plugin must not approve itself");

    // Discovered as pending and NOT served.
    let list: serde_json::Value = client.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap().json().await.unwrap();
    let demo = list.as_array().unwrap().iter().find(|p| p["id"] == "demo_gate").unwrap().clone();
    assert_eq!(demo["state"], "pending");
    assert_eq!(demo["kind"], "web");
    assert!(list.as_array().unwrap().iter().all(|p| p["id"] != "loose"), "a script with no manifest isn't a plugin");
    let digest = demo["digest"].as_str().unwrap().to_string();
    let served = |path: &'static str| {
        let (client, base) = (client.clone(), base.clone());
        async move { client.get(format!("{base}{path}")).send().await.unwrap().status() }
    };
    assert_eq!(served("/plugins/demo_gate.js").await, reqwest::StatusCode::NOT_FOUND);

    // A stale digest is refused; a missing required permission is refused.
    let approve = |token: &'static str, digest: String, granted: Vec<&'static str>| {
        let (client, base) = (client.clone(), base.clone());
        async move {
            client.post(format!("{base}/api/plugins/demo_gate/approve")).header("x-host-token", token)
                .json(&serde_json::json!({ "digest": digest, "granted": granted })).send().await.unwrap()
        }
    };
    assert_eq!(approve(host, "stale".into(), vec!["ui.dom", "keyring.own"]).await.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(approve(host, digest.clone(), vec!["ui.dom"]).await.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(served("/plugins/demo_gate.js").await, reqwest::StatusCode::NOT_FOUND);

    // A console credential may approve; the audit trail says which console.
    let resp = approve(console_cred, digest.clone(), vec!["ui.dom", "keyring.own"]).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.json::<serde_json::Value>().await.unwrap()["state"], "active");
    assert_eq!(served("/plugins/demo_gate.js").await, reqwest::StatusCode::OK);
    // Nothing else under /plugins/ is reachable, manifests and unknown files included.
    assert_eq!(served("/plugins/other.js").await, reqwest::StatusCode::NOT_FOUND);
    assert_eq!(served("/plugins/loose.js").await, reqwest::StatusCode::NOT_FOUND);
    assert_eq!(served("/plugins/demo_gate.plugin.json").await, reqwest::StatusCode::NOT_FOUND);
    let audit: serde_json::Value = client.get(format!("{base}/api/plugins/audit")).header("x-host-token", host).send().await.unwrap().json().await.unwrap();
    assert!(audit.as_array().unwrap().iter().any(|a| a["action"] == "approved" && a["actor"] == "console:console_x"));

    // Editing the script after approval stops it being served until it is reviewed again.
    std::fs::write(web_dir.join("plugins/demo_gate.js"), b"export default { init() { /* swapped */ } };").unwrap();
    assert_eq!(served("/plugins/demo_gate.js").await, reqwest::StatusCode::NOT_FOUND);
    let list: serde_json::Value = client.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap().json().await.unwrap();
    let demo = list.as_array().unwrap().iter().find(|p| p["id"] == "demo_gate").unwrap().clone();
    assert_eq!(demo["state"], "changed");
    let new_digest = demo["digest"].as_str().unwrap().to_string();
    assert_ne!(new_digest, digest);
    assert_eq!(approve(host, new_digest, vec!["ui.dom", "keyring.own"]).await.status(), reqwest::StatusCode::OK);
    assert_eq!(served("/plugins/demo_gate.js").await, reqwest::StatusCode::OK);

    // Revoke stops it again.
    let resp = client.post(format!("{base}/api/plugins/demo_gate/revoke")).header("x-host-token", host).send().await.unwrap();
    assert_eq!(resp.json::<serde_json::Value>().await.unwrap()["state"], "revoked");
    assert_eq!(served("/plugins/demo_gate.js").await, reqwest::StatusCode::NOT_FOUND);
}

/// Scoped plugin tokens: minted only by a console for an approved plugin, limited to what it was
/// granted, checked against the live grant on every use, usable for commands and its keyring
/// only within scope, and dead the moment the plugin is revoked.
#[tokio::test]
async fn test_scoped_plugin_tokens_for_commands_and_keyring() {
    let test_dir = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(test_dir.path().join("scoped.db").to_str().unwrap()).unwrap();
    let web_dir = test_dir.path().join("web");
    std::fs::create_dir_all(web_dir.join("plugins")).unwrap();
    std::fs::write(web_dir.join("plugins/scoped_demo.js"), b"export default {}").unwrap();
    std::fs::write(
        web_dir.join("plugins/scoped_demo.plugin.json"),
        serde_json::json!({
            "manifest_version": 1, "id": "scoped_demo", "name": "Scoped", "web": {"entry": "scoped_demo.js"},
            "permissions": [
                {"id": "keyring.own"},
                {"id": "commands.send", "commands": ["NextSlide", "PrevSlide"], "optional": true}
            ]
        }).to_string(),
    ).unwrap();

    let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph: Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media"))),
        web_dir: web_dir.clone(),
        media_dir: test_dir.path().join("media"),
        host_session_token: "scoped_host".to_string(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{}", addr);
    let c = reqwest::Client::new();
    let host = "scoped_host";

    let json = |r: reqwest::Response| async move { r.json::<serde_json::Value>().await.unwrap() };
    let list = json(c.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap()).await;
    let digest = list.as_array().unwrap().iter().find(|p| p["id"] == "scoped_demo").unwrap()["digest"].as_str().unwrap().to_string();

    // Not approved yet: no token.
    let mint = |body: serde_json::Value, token: &'static str| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugins/scoped_demo/token")).header("x-host-token", token).json(&body).send().await.unwrap() }
    };
    assert_eq!(mint(serde_json::json!({}), host).await.status(), reqwest::StatusCode::FORBIDDEN);

    // Approved with keyring.own only (the command permission is optional and declined).
    let resp = c.post(format!("{base}/api/plugins/scoped_demo/approve")).header("x-host-token", host)
        .json(&serde_json::json!({ "digest": digest, "granted": ["keyring.own"] })).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Only a console can mint; a plugin token can't mint more.
    assert_eq!(mint(serde_json::json!({}), "nope").await.status(), reqwest::StatusCode::UNAUTHORIZED);
    // Asking for commands that weren't granted fails outright.
    let cmd_scope = serde_json::json!({ "scopes": [{ "id": "commands.send", "commands": ["NextSlide"] }] });
    assert_eq!(mint(cmd_scope.clone(), host).await.status(), reqwest::StatusCode::FORBIDDEN);
    // A permission that can't ride a token is a 400.
    assert_eq!(mint(serde_json::json!({ "scopes": [{ "id": "ui.dom" }] }), host).await.status(), reqwest::StatusCode::BAD_REQUEST);
    // The default token (keyring.own) works for the keyring but can't send commands.
    let body = json(mint(serde_json::json!({}), host).await).await;
    let kr_token = body["token"].as_str().unwrap().to_string();
    let set = c.post(format!("{base}/api/keyring/set")).header("x-plugin-token", &kr_token)
        .json(&serde_json::json!({ "service": "OpenSanctuary:Plugin:scoped_demo", "account": "a", "secret": "s" })).send().await.unwrap();
    assert!(set.status().is_success() || set.status().is_server_error(), "keyring may be unavailable in CI, but must not be an auth error: {}", set.status());
    assert_ne!(set.status(), reqwest::StatusCode::UNAUTHORIZED);
    let r = json(c.post(format!("{base}/api/command")).header("x-plugin-token", &kr_token).json(&serde_json::json!("NextSlide")).send().await.unwrap()).await;
    assert_eq!(r["success"], false, "a keyring-only token must not send commands");

    // Re-approve with the command permission, narrowed to NextSlide/PrevSlide.
    let resp = c.post(format!("{base}/api/plugins/scoped_demo/approve")).header("x-host-token", host)
        .json(&serde_json::json!({ "digest": digest, "granted": ["keyring.own", "commands.send"] })).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body = json(mint(cmd_scope, host).await).await;
    let cmd_token = body["token"].as_str().unwrap().to_string();

    // In scope: the command runs, with no console lock and no host token.
    let r = json(c.post(format!("{base}/api/command")).header("x-plugin-token", &cmd_token).json(&serde_json::json!("NextSlide")).send().await.unwrap()).await;
    assert_eq!(r["success"], true, "{r}");
    // Out of scope (granted to the plugin? no -- and not on the token): refused, and audited once.
    for _ in 0..3 {
        let r = json(c.post(format!("{base}/api/command")).header("x-plugin-token", &cmd_token).json(&serde_json::json!({ "NewSchedule": null })).send().await.unwrap()).await;
        assert_eq!(r["success"], false);
    }
    let audit = json(c.get(format!("{base}/api/plugins/audit")).header("x-host-token", host).send().await.unwrap()).await;
    let denied_new_schedule = audit.as_array().unwrap().iter()
        .filter(|a| a["action"] == "denied_call" && a["detail_json"].as_str().unwrap_or("").contains("NewSchedule"))
        .count();
    assert_eq!(denied_new_schedule, 1, "repeated denials of the same command are coalesced");

    // A token narrows with the live grant: shrink the grant and the token loses the command.
    let resp = c.post(format!("{base}/api/plugins/scoped_demo/approve")).header("x-host-token", host)
        .json(&serde_json::json!({ "digest": digest, "granted": ["keyring.own"] })).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let r = json(c.post(format!("{base}/api/command")).header("x-plugin-token", &cmd_token).json(&serde_json::json!("NextSlide")).send().await.unwrap()).await;
    assert_eq!(r["success"], false, "shrinking the grant must shrink tokens already issued");

    // Revoke kills every token at once.
    c.post(format!("{base}/api/plugins/scoped_demo/revoke")).header("x-host-token", host).send().await.unwrap();
    let r = json(c.post(format!("{base}/api/command")).header("x-plugin-token", &kr_token).json(&serde_json::json!("NextSlide")).send().await.unwrap()).await;
    assert_eq!(r["success"], false);
    let set = c.post(format!("{base}/api/keyring/get")).header("x-plugin-token", &kr_token)
        .json(&serde_json::json!({ "service": "OpenSanctuary:Plugin:scoped_demo", "account": "a" })).send().await.unwrap();
    assert_eq!(set.status(), reqwest::StatusCode::UNAUTHORIZED);
}

/// Sandboxed web plugins are served from /plugin-frame/<id>/ only while approved and unchanged,
/// with a CSP that allows nothing beyond what was granted, and nothing escapes the folder.
#[tokio::test]
async fn test_sandboxed_plugin_frame_serving_and_csp() {
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("sbx.db").to_str().unwrap()).unwrap();
    let web_dir = tmp.path().join("web");
    let pdir = web_dir.join("plugins/sbx_demo");
    std::fs::create_dir_all(pdir.join("ui")).unwrap();
    std::fs::write(pdir.join("plugin.json"), serde_json::json!({
        "manifest_version": 1, "id": "sbx_demo", "name": "Sandboxed",
        "web": {"entry": "index.html", "sandbox": true},
        "permissions": [
            {"id": "show.observe"},
            {"id": "network.outbound", "hosts": ["192.168.1.50:6454", "evil.example/\"; script-src *"], "optional": true}
        ]
    }).to_string()).unwrap();
    std::fs::write(pdir.join("index.html"), b"<script src=\"ui/app.js\"></script>").unwrap();
    std::fs::write(pdir.join("ui/app.js"), b"parent.postMessage('hi','*')").unwrap();
    std::fs::write(web_dir.join("plugins/secret.txt"), b"not part of any plugin").unwrap();

    let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph: Arc::new(os_next::media::AssetGraph::new(tmp.path().join("media"))),
        web_dir: web_dir.clone(),
        media_dir: tmp.path().join("media"),
        host_session_token: "sbx_host".to_string(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{}", addr);
    let c = reqwest::Client::new();
    let host = "sbx_host";
    let get = |path: &'static str| { let (c, base) = (c.clone(), base.clone()); async move { c.get(format!("{base}{path}")).send().await.unwrap() } };

    // Discovered as a sandboxed plugin; not served until approved.
    let list: serde_json::Value = c.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap().json().await.unwrap();
    let p = list.as_array().unwrap().iter().find(|p| p["id"] == "sbx_demo").unwrap().clone();
    assert_eq!(p["kind"], "sandbox");
    assert_eq!(p["sandboxed"], true);
    assert_eq!(p["entry"], "index.html");
    assert_eq!(p["state"], "pending");
    assert_eq!(get("/plugin-frame/sbx_demo/index.html").await.status(), reqwest::StatusCode::NOT_FOUND);

    // Approve observe only (the network permission is declined).
    let digest = p["digest"].as_str().unwrap();
    let r = c.post(format!("{base}/api/plugins/sbx_demo/approve")).header("x-host-token", host)
        .json(&serde_json::json!({ "digest": digest, "granted": ["show.observe"] })).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let r = get("/plugin-frame/sbx_demo/index.html").await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let csp = r.headers()["content-security-policy"].to_str().unwrap().to_string();
    assert!(csp.contains("connect-src 'none'"), "no network without a grant: {csp}");
    assert!(csp.contains("sandbox allow-scripts") && csp.contains("default-src 'none'"), "{csp}");
    assert_eq!(r.headers()["x-frame-options"], "SAMEORIGIN");
    assert_eq!(get("/plugin-frame/sbx_demo/ui/app.js").await.status(), reqwest::StatusCode::OK);

    // Nothing outside the folder, and the raw folder isn't served as an ordinary static path.
    for path in ["/plugin-frame/sbx_demo/../secret.txt", "/plugin-frame/sbx_demo/%2e%2e/secret.txt", "/plugin-frame/sbx_demo/..%2fsecret.txt",
                 "/plugin-frame/nope/index.html", "/plugin-frame/sbx_demo/plugin.json.bak",
                 "/plugins/sbx_demo/index.html", "/plugins/secret.txt"] {
        let status = c.get(format!("{base}{path}")).send().await.unwrap().status();
        assert_eq!(status, reqwest::StatusCode::NOT_FOUND, "{path}");
    }

    // Grant a network host: a well-formed one reaches the CSP, a malicious one is dropped.
    let r = c.post(format!("{base}/api/plugins/sbx_demo/approve")).header("x-host-token", host)
        .json(&serde_json::json!({ "digest": digest, "granted": ["show.observe", "network.outbound"] })).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let csp = get("/plugin-frame/sbx_demo/index.html").await.headers()["content-security-policy"].to_str().unwrap().to_string();
    assert!(csp.contains("https://192.168.1.50:6454"), "{csp}");
    assert!(!csp.contains("evil.example") && !csp.contains("script-src *"), "a hostile host must not reach the CSP: {csp}");

    // A symlink dropped into the folder after approval stops it being served at all (it could
    // point anywhere), as does editing a file: the folder no longer matches what was reviewed.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/hostname", pdir.join("ui/link.txt")).unwrap();
        assert_eq!(get("/plugin-frame/sbx_demo/index.html").await.status(), reqwest::StatusCode::NOT_FOUND);
        assert_eq!(get("/plugin-frame/sbx_demo/ui/link.txt").await.status(), reqwest::StatusCode::NOT_FOUND);
        std::fs::remove_file(pdir.join("ui/link.txt")).unwrap();
        assert_eq!(get("/plugin-frame/sbx_demo/index.html").await.status(), reqwest::StatusCode::OK);
    }
    std::fs::write(pdir.join("ui/app.js"), b"alert('swapped')").unwrap();
    assert_eq!(get("/plugin-frame/sbx_demo/index.html").await.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(get("/plugin-frame/sbx_demo/ui/app.js").await.status(), reqwest::StatusCode::NOT_FOUND);
}

/// REST and WebSocket run commands through one pipeline: the same shorthand expansion and the same
/// credential rules (including that a paired display can't send), and a plugin-origin command takes
/// that path too.
#[tokio::test]
async fn test_rest_ws_and_plugin_commands_share_one_pipeline() {
    use futures_util::{SinkExt, StreamExt};

    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("pipe.db").to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        tx,
        engine: engine.clone(),
        asset_graph: Arc::new(os_next::media::AssetGraph::new(tmp.path().join("media"))),
        web_dir: std::path::PathBuf::from("web"),
        media_dir: tmp.path().join("media"),
        host_session_token: "pipe_host".to_string(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let app_for_direct = app_state.clone();
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();

    db.upsert_paired_device(&os_next::storage::PairedDevice { id: "tv".into(), name: "TV".into(), platform: "android-tv".into(), paired_at: 0, last_seen: 0, token: "tv_tok".into() }).unwrap();
    db.upsert_paired_device(&os_next::storage::PairedDevice { id: "rem".into(), name: "R".into(), platform: "mobile-remote".into(), paired_at: 0, last_seen: 0, token: "rem_tok".into() }).unwrap();

    let blackout = || engine.snapshot().state.is_blackout;
    let rest = |headers: Vec<(&'static str, &'static str)>, body: serde_json::Value| {
        let (c, base) = (c.clone(), base.clone());
        async move {
            let mut r = c.post(format!("{base}/api/command"));
            for (k, v) in headers { r = r.header(k, v); }
            r.json(&body).send().await.unwrap().json::<serde_json::Value>().await.unwrap()
        }
    };
    let console = vec![("x-host-token", "pipe_host"), ("x-console-session-id", "s1")];

    // Credential rules, REST: a paired display can't send; a remote and the console can.
    let r = rest(vec![("x-device-token", "tv_tok")], serde_json::json!("ToggleBlackout")).await;
    assert_eq!((r["success"].as_bool(), r["error"].as_str()), (Some(false), Some("display_token_cannot_control")));
    assert!(!blackout());
    assert_eq!(rest(vec![("x-device-token", "nope")], serde_json::json!("ToggleBlackout")).await["success"], false);
    assert_eq!(rest(vec![("x-device-token", "rem_tok")], serde_json::json!("ToggleBlackout")).await["success"], true);
    assert!(blackout());
    assert_eq!(rest(vec![("x-host-token", "pipe_host")], serde_json::json!("ToggleBlackout")).await["error"], "Missing x-console-session-id");
    assert_eq!(rest(console.clone(), serde_json::json!("ToggleBlackout")).await["success"], true);
    assert!(!blackout());
    // A second console is locked out with its own reason.
    assert_eq!(rest(vec![("x-host-token", "pipe_host"), ("x-console-session-id", "s2")], serde_json::json!("ToggleBlackout")).await["error"], "console_locked");

    // The same command over WebSocket takes the same path (and the same plugin text).
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await.unwrap();
    let _ = ws.next().await;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "device_token": "rem_tok" }).to_string())).await.unwrap();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "device_token": "tv_tok" }).to_string())).await.unwrap(); // display: ignored
    for _ in 0..40 { if blackout() { break; } tokio::time::sleep(std::time::Duration::from_millis(25)).await; }
    assert!(blackout(), "a paired remote's WS command ran");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(blackout(), "the display's WS command must not have toggled it back");

    // A command from a plugin takes the same path (here the pipeline is driven directly).
    let origin = os_next::api::command_pipeline::CommandOrigin::Plugin { id: "recorder".into() };
    os_next::api::command_pipeline::run_command_pipeline(&app_for_direct, serde_json::json!("NextSlide"), &origin).unwrap();
    // And an unparseable payload is a rejection, not a crash.
    assert!(os_next::api::command_pipeline::run_command_pipeline(&app_for_direct, serde_json::json!({"Nope": 1}), &origin).is_err());
}

/// The host API behind the Genius Lyrics plugin: the host fetches only from granted hosts (and
/// never a non-public address), and a plugin granted `library.write` can add songs but not choose
/// their ids or exceed limits.
#[tokio::test]
async fn test_plugin_host_fetch_and_add_song_for_the_genius_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("genius.db").to_str().unwrap()).unwrap();
    // Install the real bundled plugin into a throwaway web dir.
    let web_dir = tmp.path().join("web");
    let dest = web_dir.join("plugins/genius_lyrics");
    std::fs::create_dir_all(&dest).unwrap();
    for entry in std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/plugins/genius_lyrics")).unwrap().flatten() {
        std::fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
    }

    let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph: Arc::new(os_next::media::AssetGraph::new(tmp.path().join("media"))),
        web_dir: web_dir.clone(),
        media_dir: tmp.path().join("media"),
        host_session_token: "gen_host".to_string(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let host = "gen_host";
    let json = |r: reqwest::Response| async move { r.json::<serde_json::Value>().await.unwrap() };

    // Discovered as a pending sandboxed plugin that asks for exactly what it needs.
    let list = json(c.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap()).await;
    let p = list.as_array().unwrap().iter().find(|p| p["id"] == "genius_lyrics").unwrap().clone();
    assert_eq!((p["kind"].as_str(), p["state"].as_str(), p["error"].is_null()), (Some("sandbox"), Some("pending"), true), "{p}");
    let ids: Vec<&str> = p["requested"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["ui.panel", "network.outbound", "library.write", "commands.send"]);
    let hosts: Vec<&str> = p["requested"][1]["hosts"].as_array().unwrap().iter().map(|h| h.as_str().unwrap()).collect();
    assert_eq!(hosts, ["lrclib.net", "genius.com", "www.genius.com"], "network access is limited to LRCLIB and Genius");

    // No token, no service.
    let r = c.post(format!("{base}/api/plugin-host/songs")).json(&serde_json::json!({ "song": {} })).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);
    let r = c.post(format!("{base}/api/plugin-host/fetch")).json(&serde_json::json!({ "url": "https://genius.com/" })).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Approve with the network + write permissions but not the command permission.
    let r = c.post(format!("{base}/api/plugins/genius_lyrics/approve")).header("x-host-token", host)
        .json(&serde_json::json!({ "digest": p["digest"], "granted": ["ui.panel", "network.outbound", "library.write"] })).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);

    // A token can't ask for more than the grant: another host, or another category.
    let mint = |scopes: serde_json::Value| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugins/genius_lyrics/token")).header("x-host-token", "gen_host").json(&serde_json::json!({ "scopes": scopes })).send().await.unwrap() }
    };
    assert_eq!(mint(serde_json::json!([{ "id": "network.outbound", "hosts": ["evil.example"] }])).await.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(mint(serde_json::json!([{ "id": "library.write", "categories": ["themes"] }])).await.status(), reqwest::StatusCode::FORBIDDEN);
    let body = json(mint(serde_json::json!([
        { "id": "network.outbound", "hosts": ["genius.com", "www.genius.com"] },
        { "id": "library.write", "categories": ["songs"] }
    ])).await).await;
    let token = body["token"].as_str().unwrap().to_string();

    // The host refuses anything outside the grant -- without making a request.
    for url in ["https://example.com/", "http://genius.com/", "https://genius.com@evil.test/", "https://127.0.0.1/", "https://genius.com.evil.test/"] {
        let r = c.post(format!("{base}/api/plugin-host/fetch")).header("x-plugin-token", &token)
            .json(&serde_json::json!({ "url": url })).send().await.unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::FORBIDDEN, "{url}");
    }
    // Headers: this plugin declares none, so a token can't even be minted for an API-key header, an undeclared
    // header is refused at fetch time, and routing headers never pass.
    assert_eq!(
        mint(serde_json::json!([{ "id": "network.outbound", "hosts": ["genius.com"], "headers": ["X-Key"] }])).await.status(),
        reqwest::StatusCode::FORBIDDEN
    );
    for hdr in ["X-Key", "Cookie", "X-Host-Token"] {
        let r = c.post(format!("{base}/api/plugin-host/fetch")).header("x-plugin-token", &token)
            .json(&serde_json::json!({ "url": "https://genius.com/", "headers": { hdr: "v" } })).send().await.unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST, "{hdr}");
        assert!(r.text().await.unwrap().contains("can't be set"), "{hdr}");
    }
    // A granted *name* that resolves to a loopback address is still refused (DNS can't be a way in).
    let err = os_next::api::plugin_net::fetch(&["localhost:9".to_string()], "https://localhost:9/", &[]).await.err().unwrap();
    assert!(err.contains("public address"), "{err}");

    // Adding a song: the plugin's JSON shape is accepted; the host mints the id; limits apply.
    let song = serde_json::json!({
        "id": "attacker_chosen", "title": "Way Maker", "author": "Sinach", "copyright": "Genius Christian Lyrics", "ccli_number": "7115744",
        "slides": [{ "text": "Way maker\nMiracle worker", "header": "V1", "label": "Verse 1", "tag": "Verse 1" }],
        "theme_name": null, "alternate_title": null
    });
    let post_song = |song: serde_json::Value, token: String| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugin-host/songs")).header("x-plugin-token", token).json(&serde_json::json!({ "song": song })).send().await.unwrap() }
    };
    let r = post_song(song.clone(), token.clone()).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let added = json(r).await;
    let id = added["id"].as_str().unwrap();
    assert!(id.starts_with("song_") && id != "attacker_chosen", "the host mints the id: {id}");
    let stored = db.get_song_by_id(id).unwrap().expect("song stored");
    assert_eq!((stored.title.as_str(), stored.ccli_number.as_deref(), stored.slides.len()), ("Way Maker", Some("7115744"), 1));
    assert_eq!(stored.slides[0].header.as_deref(), Some("V1"));

    // No id at all is fine too (the plugin never has to invent one).
    let mut no_id = song.clone();
    no_id.as_object_mut().unwrap().remove("id");
    let r = post_song(no_id, token.clone()).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    assert!(json(r).await["id"].as_str().unwrap().starts_with("song_"));

    let mut empty = song.clone();
    empty["slides"] = serde_json::json!([]);
    assert_eq!(post_song(empty, token.clone()).await.status(), reqwest::StatusCode::BAD_REQUEST);
    let mut huge = song.clone();
    huge["slides"][0]["text"] = serde_json::json!("x".repeat(20_001));
    assert_eq!(post_song(huge, token.clone()).await.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(post_song(serde_json::json!({ "title": 5 }), token.clone()).await.status(), reqwest::StatusCode::BAD_REQUEST);

    // A token without library.write can't add; revoking the plugin kills the token.
    let net_only = json(mint(serde_json::json!([{ "id": "network.outbound", "hosts": ["genius.com"] }])).await).await;
    assert_eq!(post_song(song.clone(), net_only["token"].as_str().unwrap().to_string()).await.status(), reqwest::StatusCode::FORBIDDEN);
    c.post(format!("{base}/api/plugins/genius_lyrics/revoke")).header("x-host-token", host).send().await.unwrap();
    assert_eq!(post_song(song, token).await.status(), reqwest::StatusCode::UNAUTHORIZED);
}

/// The host API behind the Online Bibles plugin: it can add chapters (ids and verse ranges are the
/// host's, limits apply), re-adding replaces rather than duplicates, and only a plugin granted
/// `library.write` on scriptures may.
#[tokio::test]
async fn test_online_bibles_plugin_host_api_adds_chapters() {
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new_with_dirs(
        tmp.path().join("bibles_plugin.db").to_str().unwrap(),
        &tmp.path().join("bibles"),
        &tmp.path().join("songs"),
    ).unwrap();
    let web_dir = tmp.path().join("web");
    let dest = web_dir.join("plugins/online_bibles");
    std::fs::create_dir_all(&dest).unwrap();
    for entry in std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/plugins/online_bibles")).unwrap().flatten() {
        std::fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
    }

    let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph: Arc::new(os_next::media::AssetGraph::new(tmp.path().join("media"))),
        web_dir: web_dir.clone(),
        media_dir: tmp.path().join("media"),
        host_session_token: "bib_host".to_string(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let host = "bib_host";
    let json = |r: reqwest::Response| async move { r.json::<serde_json::Value>().await.unwrap() };

    // The plugin asks for the project's own Bibles (GitHub), a GitHub repository, and bible-api.com: no Bolls.life, no ChurchApps.
    let list = json(c.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap()).await;
    let p = list.as_array().unwrap().iter().find(|p| p["id"] == "online_bibles").unwrap().clone();
    assert_eq!((p["kind"].as_str(), p["error"].is_null()), (Some("sandbox"), true), "{p}");
    let hosts: Vec<&str> = p["requested"][1]["hosts"].as_array().unwrap().iter().map(|h| h.as_str().unwrap()).collect();
    assert_eq!(hosts, ["bible-api.com", "api.github.com", "raw.githubusercontent.com"]);
    assert_eq!(p["requested"][2]["categories"], serde_json::json!(["scriptures"]));
    let r = c.post(format!("{base}/api/plugins/online_bibles/approve")).header("x-host-token", host)
        .json(&serde_json::json!({ "digest": p["digest"], "granted": ["ui.panel", "network.outbound", "library.write"] })).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);

    let mint = |scopes: serde_json::Value| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugins/online_bibles/token")).header("x-host-token", "bib_host").json(&serde_json::json!({ "scopes": scopes })).send().await.unwrap() }
    };
    // Songs weren't granted, only scriptures.
    assert_eq!(mint(serde_json::json!([{ "id": "library.write", "categories": ["songs"] }])).await.status(), reqwest::StatusCode::FORBIDDEN);
    let token = json(mint(serde_json::json!([{ "id": "library.write", "categories": ["scriptures"] }])).await).await["token"].as_str().unwrap().to_string();
    let post = |items: serde_json::Value, token: String| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugin-host/scriptures")).header("x-plugin-token", token).json(&serde_json::json!({ "items": items })).send().await.unwrap() }
    };
    assert_eq!(c.post(format!("{base}/api/plugin-host/scriptures")).json(&serde_json::json!({ "items": [] })).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);

    // Two chapters; verses arrive out of order and with attacker-chosen ids/ranges/reference.
    let chapter = |book: &str, ch: u32| serde_json::json!({
        "id": "attacker", "book": book, "chapter": ch, "verse_start": 99, "verse_end": 1, "version": "plugtest", "reference": "Evil 1:1",
        "verses": [{ "verse_number": 2, "text": "second" }, { "verse_number": 1, "text": "first" }]
    });
    let r = post(serde_json::json!([chapter("John", 3), chapter("Genesis", 1)]), token.clone()).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let body = json(r).await;
    assert_eq!((body["chapters"].as_u64(), body["verses"].as_u64()), (Some(2), Some(4)));
    let ids: Vec<String> = body["ids"].as_array().unwrap().iter().map(|i| i.as_str().unwrap().to_string()).collect();
    assert!(ids.iter().all(|i| i.starts_with("scrip_") && i != "attacker"));

    // Stored as the host derived it, under the upper-cased version, and listed as an installed Bible.
    let stored = db.get_scripture_by_id(&ids[0]).unwrap().expect("chapter stored");
    assert_eq!((stored.book.as_str(), stored.chapter, stored.version.as_str(), stored.verse_start, stored.verse_end, stored.reference.as_str()),
               ("John", 3, "PLUGTEST", 1, 2, "John 3"));
    assert_eq!(stored.verses.iter().map(|v| v.text.as_str()).collect::<Vec<_>>(), ["first", "second"]);
    assert!(db.get_installed_bibles().unwrap().iter().any(|b| b.id == "PLUGTEST"));

    // Sending the same chapter again replaces it (same id) instead of duplicating.
    let again = json(post(serde_json::json!([chapter("John", 3)]), token.clone()).await).await;
    assert_eq!(again["ids"][0], serde_json::Value::String(ids[0].clone()));
    assert_eq!(db.get_installed_bibles().unwrap().iter().find(|b| b.id == "PLUGTEST").unwrap().verse_count, 2); // (it counts stored chapters)

    // Limits.
    assert_eq!(post(serde_json::json!([]), token.clone()).await.status(), reqwest::StatusCode::BAD_REQUEST);
    let many: Vec<_> = (0..501).map(|i| chapter("John", 1 + (i % 20))).collect();
    assert_eq!(post(serde_json::json!(many), token.clone()).await.status(), reqwest::StatusCode::BAD_REQUEST);
    let mut no_verses = chapter("John", 3); no_verses["verses"] = serde_json::json!([]);
    assert_eq!(post(serde_json::json!([no_verses]), token.clone()).await.status(), reqwest::StatusCode::BAD_REQUEST);
    // A long verse is fine (the Greek Esther letters, Burmese text): over the cap is refused, not under it.
    let mut long_verse = chapter("Esther (Greek)", 8); long_verse["verses"][0]["text"] = serde_json::json!("x".repeat(10_500));
    assert_eq!(post(serde_json::json!([long_verse]), token.clone()).await.status(), reqwest::StatusCode::OK);
    let mut long_text = chapter("John", 3); long_text["verses"][0]["text"] = serde_json::json!("x".repeat(16_001));
    assert_eq!(post(serde_json::json!([long_text]), token.clone()).await.status(), reqwest::StatusCode::BAD_REQUEST);
    let mut bad_version = chapter("John", 3); bad_version["version"] = serde_json::json!("");
    assert_eq!(post(serde_json::json!([bad_version]), token.clone()).await.status(), reqwest::StatusCode::BAD_REQUEST);
    // A version name can never become a path: it is sanitised to a plain file name.
    let mut sneaky = chapter("John", 3); sneaky["version"] = serde_json::json!("../../etc/evil");
    assert_eq!(post(serde_json::json!([sneaky]), token.clone()).await.status(), reqwest::StatusCode::OK);
    assert!(!tmp.path().join("etc").exists());
    assert!(std::fs::read_dir(tmp.path().join("bibles")).unwrap().flatten().all(|e| e.path().parent() == Some(&tmp.path().join("bibles"))));

    // Revoke: the token is dead.
    c.post(format!("{base}/api/plugins/online_bibles/revoke")).header("x-host-token", host).send().await.unwrap();
    assert_eq!(post(serde_json::json!([chapter("John", 3)]), token).await.status(), reqwest::StatusCode::UNAUTHORIZED);
}

/// The video-downloader plugin's host side, end to end with a stand-in `yt-dlp`: what the review screen
/// shows, who may run what, validated parameters, polling a job's output, moving the finished file into
/// the library only from the plugin's own job folder, and revoke.
#[cfg(unix)]
#[tokio::test]
async fn test_ytdlp_plugin_runs_only_its_declared_command_and_adds_media() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("ytdlp.db").to_str().unwrap()).unwrap();
    let web_dir = tmp.path().join("web");
    let dest = web_dir.join("plugins/ytdlp");
    std::fs::create_dir_all(&dest).unwrap();
    for entry in std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/plugins/ytdlp")).unwrap().flatten() {
        std::fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
    }
    std::fs::create_dir_all(web_dir.join("plugins/_shared")).unwrap();
    for f in ["sdk.js", "theme.css"] {
        std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/plugins/_shared").join(f), web_dir.join("plugins/_shared").join(f)).unwrap();
    }

    // The console's icons, as the plugin frame is allowed to load them.
    std::fs::create_dir_all(web_dir.join("icons/orange")).unwrap();
    std::fs::write(web_dir.join("icons/orange/check.svg"), "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();
    std::fs::write(web_dir.join("icons/emoji-map.json"), r#"[{"glyph":"\u2713","icon":"check"},{"glyph":"\u2702\ufe0f","icon":"scissors"},{"glyph":"\ud83d\udd0d","icon":null}]"#).unwrap();

    // A stand-in yt-dlp in the place the host installs the real one.
    let data_dir = tmp.path().join("data");
    let tool_dir = data_dir.join("tools/plugins/ytdlp");
    std::fs::create_dir_all(&tool_dir).unwrap();
    let fake = tool_dir.join("yt-dlp");
    std::fs::write(&fake, r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "2099.01.01"; exit 0; fi
out=""; audio=0
while [ $# -gt 0 ]; do
  case "$1" in -o) out="$2"; shift;; -x) audio=1;; --) shift; url="$1";; esac
  shift
done
dir=$(dirname "$out")
ext=webm; [ $audio = 1 ] && ext=mp3
echo "[download]   0.0% of 1.00MiB at Unknown ETA Unknown"
echo "[download]  50.0% of 1.00MiB at 2.00MiB/s ETA 00:01"
if [ $audio = 1 ]; then printf '\377\373\220\000fake mp3 bytes' > "$dir/Fake Title [abc123].$ext"; else printf '\032\105\337\243fake webm bytes' > "$dir/Fake Title [abc123].$ext"; fi
printf 'junk' > "$dir/Fake Title [abc123].f137.mp4.part"
echo "[download] 100% of 1.00MiB at 2.00MiB/s ETA 00:00"
echo "{\"id\":\"abc123\",\"title\":\"Fake Title\",\"duration\":12.5,\"thumbnail\":\"https://img.example/t.jpg\",\"_filename\":\"$dir/Fake Title [abc123].webm\",\"ext\":\"webm\",\"url\":\"$url\"}"
"#).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let media_dir = tmp.path().join("media");
    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph: Arc::new(os_next::media::AssetGraph::new(&media_dir)),
        web_dir: web_dir.clone(),
        media_dir: media_dir.clone(),
        data_dir: data_dir.clone(),
        host_session_token: "yt_host".to_string(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let host = "yt_host";
    let json = |r: reqwest::Response| async move { r.json::<serde_json::Value>().await.unwrap() };

    // What the review screen is shown: the exact command, the tool and its source, where it appears.
    let list = json(c.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap()).await;
    let p = list.as_array().unwrap().iter().find(|p| p["id"] == "ytdlp").unwrap().clone();
    assert_eq!((p["kind"].as_str(), p["error"].is_null()), (Some("sandbox"), true), "{p}");
    let line = p["commands"][0]["line"].as_str().unwrap();
    assert!(line.starts_with("yt-dlp --no-playlist --newline --progress --print-json -o {output_dir}/"), "{line}");
    assert!(line.contains("[if browser: --cookies-from-browser {browser}]") && line.ends_with("-- {url}"), "{line}");
    assert_eq!(p["tools"][0]["github"], "yt-dlp/yt-dlp");
    assert_eq!(p["tools"][0]["key_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(p["placements"].as_array().unwrap().len(), 2);
    let ids: Vec<&str> = p["requested"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["ui.panel", "ui.clipboard", "process.run", "library.write", "commands.send", "notifications.post"]);
    assert_eq!(p["requested"][2]["tier"], "critical");
    assert_eq!(p["requested"][2]["enforcement"], "enforced");

    // Nothing runs before approval (and a token can't be minted).
    let mint = |scopes: serde_json::Value| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugins/ytdlp/token")).header("x-host-token", "yt_host").json(&serde_json::json!({ "scopes": scopes })).send().await.unwrap() }
    };
    assert_eq!(mint(serde_json::json!([{ "id": "process.run", "commands": ["download"] }])).await.status(), reqwest::StatusCode::FORBIDDEN);
    let r = c.post(format!("{base}/api/plugins/ytdlp/approve")).header("x-host-token", host)
        .json(&serde_json::json!({ "digest": p["digest"], "granted": ["ui.panel", "process.run", "library.write"] })).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);

    // A token can't ask for a command the manifest doesn't declare, or more than was granted.
    assert_eq!(mint(serde_json::json!([{ "id": "process.run", "commands": ["rm-everything"] }])).await.status(), reqwest::StatusCode::FORBIDDEN);
    let token = json(mint(serde_json::json!([
        { "id": "process.run", "commands": ["download"] }, { "id": "library.write", "categories": ["media"] }])).await).await["token"].as_str().unwrap().to_string();
    let call = |path: &'static str, body: serde_json::Value, token: String| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugin-host/{path}")).header("x-plugin-token", token).json(&body).send().await.unwrap() }
    };
    assert_eq!(c.post(format!("{base}/api/plugin-host/process/start")).json(&serde_json::json!({ "command": "download" })).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);

    // The tool is reported installed (the stand-in answers --version).
    let st = json(call("tool/status", serde_json::json!({ "tool": "yt-dlp" }), token.clone()).await).await;
    assert_eq!((st["installed"].as_bool(), st["source"].as_str(), st["version"].as_str()), (Some(true), Some("managed"), Some("2099.01.01")));
    assert_eq!(call("tool/status", serde_json::json!({ "tool": "nope" }), token.clone()).await.status(), reqwest::StatusCode::NOT_FOUND);

    // Parameters are validated by the host; undeclared commands and parameters are refused.
    for bad in [
        serde_json::json!({ "command": "download", "params": { "url": "--exec touch /tmp/pwned" } }),
        serde_json::json!({ "command": "download", "params": { "url": "file:///etc/passwd" } }),
        serde_json::json!({ "command": "download", "params": { "url": "https://x.test/", "browser": "chrome; rm -rf /" } }),
        serde_json::json!({ "command": "download", "params": { "url": "https://x.test/", "exec": "x" } }),
        serde_json::json!({ "command": "download", "params": {} }),
    ] {
        assert_eq!(call("process/start", bad.clone(), token.clone()).await.status(), reqwest::StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(call("process/start", serde_json::json!({ "command": "other", "params": {} }), token.clone()).await.status(), reqwest::StatusCode::FORBIDDEN);

    // A real run: poll until it ends; progress lines and the JSON line arrive, and the file is listed.
    let started = json(call("process/start", serde_json::json!({ "command": "download", "params": { "url": "https://video.example/watch?v=abc123", "sponsorblock": true } }), token.clone()).await).await;
    let job = started["job"].as_str().unwrap().to_string();
    let mut since = 0;
    let mut lines: Vec<String> = Vec::new();
    let done = loop {
        let v = json(call("process/poll", serde_json::json!({ "job": job, "since": since }), token.clone()).await).await;
        since = v["next"].as_u64().unwrap();
        lines.extend(v["lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap().to_string()));
        if v["status"] != "running" { break v; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!((done["status"].as_str(), done["exit_code"].as_i64()), (Some("exited"), Some(0)), "{done}");
    assert!(lines.iter().any(|l| l.contains("50.0%")), "{lines:?}");
    assert!(lines.last().unwrap().starts_with('{') && lines.last().unwrap().contains("\"id\":\"abc123\""));
    let names: Vec<&str> = done["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Fake Title [abc123].webm"), "{names:?}");

    // Adding media: only a plain file name from THIS plugin's finished job, of the right type.
    let add = |file: &str, media_type: &str, job: &str, token: String| {
        let (body, call) = (serde_json::json!({ "job": job, "file": file, "media_type": media_type, "title": "Fake Title", "id": "ytdlp_abc123", "duration": 12.5, "thumbnail": "https://img.example/t.jpg" }), &call);
        call("media", body, token)
    };
    for bad in ["../../etc/passwd", "/etc/passwd", "sub/dir.webm", ".hidden.webm", "Fake Title [abc123].f137.mp4.part", "missing.webm"] {
        let r = add(bad, "video", &job, token.clone()).await;
        assert!(r.status() == reqwest::StatusCode::BAD_REQUEST || r.status() == reqwest::StatusCode::NOT_FOUND, "{bad}: {}", r.status());
    }
    assert_eq!(add("Fake Title [abc123].webm", "audio", &job, token.clone()).await.status(), reqwest::StatusCode::BAD_REQUEST, "an audio item must be an audio file");
    assert_eq!(add("Fake Title [abc123].webm", "video", "not-a-job", token.clone()).await.status(), reqwest::StatusCode::NOT_FOUND);
    let r = add("Fake Title [abc123].webm", "video", &job, token.clone()).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let added = json(r).await;
    assert_eq!(added["id"], "plugin_ytdlp_ytdlp_abc123");
    let item = db.get_media_by_id("plugin_ytdlp_ytdlp_abc123").unwrap().expect("media item stored");
    assert_eq!((item.name.as_str(), item.file_path.as_str(), item.duration_seconds), ("Fake Title", "/media/videos/Fake Title [abc123].webm", Some(12.5)));
    assert_eq!(std::fs::read(media_dir.join("videos/Fake Title [abc123].webm")).unwrap(), b"\x1a\x45\xdf\xa3fake webm bytes");
    assert!(!data_dir.join("plugin-jobs/ytdlp").join(&job).join("Fake Title [abc123].webm").exists(), "the file was moved, not copied");

    // The job folder is the host's to clean up.
    let r = json(call("process/release", serde_json::json!({ "job": job }), token.clone()).await).await;
    assert_eq!(r["success"], true);
    assert!(!data_dir.join("plugin-jobs/ytdlp").join(&job).exists());
    assert_eq!(call("process/poll", serde_json::json!({ "job": job }), token.clone()).await.status(), reqwest::StatusCode::NOT_FOUND);

    // The console's shared SDK and theme are served to an approved plugin, and nothing else is reachable
    // through the `__os/` prefix.
    let now = json(c.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap()).await;
    let now = now.as_array().unwrap().iter().find(|p| p["id"] == "ytdlp").unwrap().clone();
    assert_eq!(now["state"], "active", "{now}");
    assert_eq!(c.get(format!("{base}/plugin-frame/ytdlp/index.html")).send().await.unwrap().status().as_u16(), 200, "the plugin's own page");
    for (path, want) in [("sdk.js", 200), ("theme.css", 200), ("other.js", 404), ("..%2fplugin.json", 404), ("sdk.js/x", 404),
                         ("icons.js", 200), ("icons/orange/check.svg", 200), ("icons/orange/nope.svg", 404), ("icons/Orange/check.svg", 404),
                         ("icons/orange/check.svg/x", 404), ("icons/orange/..%2f..%2fplugin.json", 404), ("icons/../../etc/passwd", 404), ("icons/orange/check", 404)] {
        let r = c.get(format!("{base}/plugin-frame/ytdlp/__os/{path}")).send().await.unwrap();
        let csp = r.headers().get("content-security-policy").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let (st, body) = (r.status().as_u16(), r.text().await.unwrap_or_default());
        assert_eq!(st, want, "{path}: {}", body.chars().take(120).collect::<String>());
        if want == 200 {
            assert!(csp.contains("sandbox allow-scripts"), "{path}");
        }
    }

    // The emoji map reaches the frame as a script, with variation selectors stripped and glyphs without an icon left out.
    let icons_js = c.get(format!("{base}/plugin-frame/ytdlp/__os/icons.js")).send().await.unwrap().text().await.unwrap();
    assert!(icons_js.starts_with("window.__OS_EMOJI = {"), "{icons_js}");
    assert!(icons_js.contains("\"\u{2713}\":\"check\"") && icons_js.contains("\"\u{2702}\":\"scissors\""), "{icons_js}");
    assert!(!icons_js.contains('\u{fe0f}') && !icons_js.contains('\u{1f50d}'), "{icons_js}");

    // A token without library.write can't add media; and a revoke kills everything.
    let run_only = json(mint(serde_json::json!([{ "id": "process.run", "commands": ["download"] }])).await).await["token"].as_str().unwrap().to_string();
    assert_eq!(call("media", serde_json::json!({ "job": "x", "file": "a.webm", "media_type": "video", "title": "t" }), run_only).await.status(), reqwest::StatusCode::FORBIDDEN);
    c.post(format!("{base}/api/plugins/ytdlp/revoke")).header("x-host-token", host).send().await.unwrap();
    assert_eq!(call("process/start", serde_json::json!({ "command": "download", "params": { "url": "https://x.test/" } }), token).await.status(), reqwest::StatusCode::UNAUTHORIZED);

    // The shared SDK and theme are served to an approved plugin only (it was revoked just now).
    assert_eq!(c.get(format!("{base}/plugin-frame/ytdlp/__os/sdk.js")).send().await.unwrap().status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_notification_bus_plugin_posts_and_console_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("notify.db").to_str().unwrap()).unwrap();
    let web_dir = tmp.path().join("web");
    let dest = web_dir.join("plugins/ytdlp");
    std::fs::create_dir_all(&dest).unwrap();
    for entry in std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/plugins/ytdlp")).unwrap().flatten() {
        std::fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
    }
    let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph: Arc::new(os_next::media::AssetGraph::new(tmp.path().join("media"))),
        web_dir: web_dir.clone(),
        media_dir: tmp.path().join("media"),
        data_dir: tmp.path().to_path_buf(),
        host_session_token: "nt_host".to_string(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let bus = app_state.notifications.clone();
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let host = "nt_host";
    let json = |r: reqwest::Response| async move { r.json::<serde_json::Value>().await.unwrap() };

    // Reading and clearing are console-only; posting needs a plugin token.
    assert_eq!(c.get(format!("{base}/api/notifications")).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(c.post(format!("{base}/api/notifications/clear")).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(c.post(format!("{base}/api/plugin-host/notify")).json(&serde_json::json!({ "title": "x" })).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);

    // Approve the plugin WITHOUT the optional notification permission: its token can't post.
    let list = json(c.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap()).await;
    let p = list.as_array().unwrap().iter().find(|p| p["id"] == "ytdlp").unwrap().clone();
    assert!(p["requested"].as_array().unwrap().iter().any(|r| r["id"] == "notifications.post" && r["optional"] == true), "{p}");
    let approve = |granted: serde_json::Value| {
        let (c, base, digest) = (c.clone(), base.clone(), p["digest"].clone());
        async move { c.post(format!("{base}/api/plugins/ytdlp/approve")).header("x-host-token", "nt_host").json(&serde_json::json!({ "digest": digest, "granted": granted })).send().await.unwrap() }
    };
    assert_eq!(approve(serde_json::json!(["ui.panel", "process.run", "library.write"])).await.status(), reqwest::StatusCode::OK);
    let mint = |scopes: serde_json::Value| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugins/ytdlp/token")).header("x-host-token", "nt_host").json(&serde_json::json!({ "scopes": scopes })).send().await.unwrap() }
    };
    assert_eq!(mint(serde_json::json!([{ "id": "notifications.post" }])).await.status(), reqwest::StatusCode::FORBIDDEN, "can't mint what wasn't granted");

    // Granted: the plugin posts, the host stamps the source, text is clamped, bad levels refused.
    assert_eq!(approve(serde_json::json!(["ui.panel", "process.run", "library.write", "notifications.post"])).await.status(), reqwest::StatusCode::OK);
    let token = json(mint(serde_json::json!([{ "id": "notifications.post" }])).await).await["token"].as_str().unwrap().to_string();
    let post = |body: serde_json::Value| {
        let (c, base, token) = (c.clone(), base.clone(), token.clone());
        async move { c.post(format!("{base}/api/plugin-host/notify")).header("x-plugin-token", token).json(&body).send().await.unwrap() }
    };
    let r = post(serde_json::json!({ "title": "Download finished", "message": "Way Maker.mp4", "level": "success", "source": "host", "source_name": "OpenSanctuary" })).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    assert_eq!(post(serde_json::json!({ "title": "x", "level": "catastrophe" })).await.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(post(serde_json::json!({ "title": "  " })).await.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(post(serde_json::json!({ "title": "t".repeat(500), "message": "m".repeat(5000), "level": "error", "sticky": true, "key": "dl" })).await.status(), reqwest::StatusCode::OK);

    let got = json(c.get(format!("{base}/api/notifications?after=0")).header("x-host-token", host).send().await.unwrap()).await;
    let items = got["notifications"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{got}");
    assert_eq!((items[0]["source"].as_str(), items[0]["source_name"].as_str()), (Some("plugin:ytdlp"), Some("Video Downloader (yt-dlp)")), "the plugin can't pick its own source: {}", items[0]);
    assert_eq!((items[1]["title"].as_str().unwrap().len(), items[1]["body"].as_str().unwrap().len(), items[1]["sticky"].as_bool()), (80, 400, Some(true)));
    let first_id = items[0]["id"].as_u64().unwrap();

    // Long-poll: returns as soon as something new is posted.
    let waiter = {
        let (c, base) = (c.clone(), base.clone());
        let after = got["latest"].as_u64().unwrap();
        tokio::spawn(async move { c.get(format!("{base}/api/notifications?after={after}&wait=10")).header("x-host-token", "nt_host").send().await.unwrap().json::<serde_json::Value>().await.unwrap() })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    bus.post_host(os_next::api::notifications::Level::Warning, "Host notice", "from the host");
    let woke = tokio::time::timeout(std::time::Duration::from_secs(5), waiter).await.expect("long-poll should wake").unwrap();
    assert_eq!(woke["notifications"][0]["source"], "host");

    // Rate limit, then revoke: the token stops working.
    let mut limited = false;
    for _ in 0..20 {
        if post(serde_json::json!({ "title": "spam" })).await.status() == reqwest::StatusCode::TOO_MANY_REQUESTS { limited = true; break; }
    }
    assert!(limited, "a plugin can't flood the console");

    // Dismiss and clear.
    let r = c.post(format!("{base}/api/notifications/dismiss")).header("x-host-token", host).json(&serde_json::json!({ "id": first_id })).send().await.unwrap();
    assert_eq!(json(r).await["removed"], true);
    assert!(bus.since(0).iter().all(|n| n.id != first_id));
    c.post(format!("{base}/api/notifications/clear")).header("x-host-token", host).json(&serde_json::json!({ "source": "host" })).send().await.unwrap();
    assert!(bus.since(0).iter().all(|n| n.source != "host") && !bus.since(0).is_empty());
    c.post(format!("{base}/api/notifications/clear")).header("x-host-token", host).send().await.unwrap();
    assert!(bus.since(0).is_empty());

    assert_eq!(c.post(format!("{base}/api/plugins/ytdlp/revoke")).header("x-host-token", host).send().await.unwrap().status(), reqwest::StatusCode::OK);
    assert_eq!(post(serde_json::json!({ "title": "late" })).await.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_stock_media_plugin_host_api_limits() {
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("stock.db").to_str().unwrap()).unwrap();
    let web_dir = tmp.path().join("web");
    let dest = web_dir.join("plugins/stock_media");
    std::fs::create_dir_all(&dest).unwrap();
    for entry in std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/plugins/stock_media")).unwrap().flatten() {
        std::fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
    }
    let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph: Arc::new(os_next::media::AssetGraph::new(tmp.path().join("media"))),
        web_dir: web_dir.clone(),
        media_dir: tmp.path().join("media"),
        data_dir: tmp.path().to_path_buf(),
        host_session_token: "st_host".to_string(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let host = "st_host";
    let json = |r: reqwest::Response| async move { r.json::<serde_json::Value>().await.unwrap() };

    // The built-in providers and their routes are gone.
    for path in ["/api/media/online/search?q=sun", "/api/media/online/import"] {
        let r = c.get(format!("{base}{path}")).header("x-host-token", host).send().await.unwrap();
        assert!(r.status() == reqwest::StatusCode::NOT_FOUND || r.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED, "{path}: {}", r.status());
    }

    // What it asks for: its own two APIs and their image hosts, its own keys, media writes.
    let list = json(c.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap()).await;
    let p = list.as_array().unwrap().iter().find(|p| p["id"] == "stock_media").unwrap().clone();
    assert_eq!((p["kind"].as_str(), p["state"].as_str(), p["error"].is_null()), (Some("sandbox"), Some("pending"), true), "{p}");
    let ids: Vec<&str> = p["requested"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["ui.panel", "network.outbound", "keyring.own", "library.write", "notifications.post"]);
    let hosts: Vec<&str> = p["requested"][1]["hosts"].as_array().unwrap().iter().map(|h| h.as_str().unwrap()).collect();
    assert_eq!(hosts, ["api.pexels.com", "images.pexels.com", "pixabay.com", "cdn.pixabay.com"]);
    assert_eq!(p["requested"][3]["categories"], serde_json::json!(["media"]));

    let r = c.post(format!("{base}/api/plugins/stock_media/approve")).header("x-host-token", host)
        .json(&serde_json::json!({ "digest": p["digest"], "granted": ["ui.panel", "network.outbound", "keyring.own", "library.write"] })).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);

    // The frame may show pictures from exactly those hosts (and nowhere else), and connect to them.
    let r = c.get(format!("{base}/plugin-frame/stock_media/index.html")).header("x-host-token", host).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let csp = r.headers().get("content-security-policy").unwrap().to_str().unwrap().to_string();
    assert!(csp.contains("img-src 'self' data: blob: https://api.pexels.com https://images.pexels.com https://pixabay.com https://cdn.pixabay.com;"), "{csp}");
    assert!(csp.contains("sandbox allow-scripts") && !csp.contains("evil"), "{csp}");

    let token = json(c.post(format!("{base}/api/plugins/stock_media/token")).header("x-host-token", host)
        .json(&serde_json::json!({ "scopes": [
            { "id": "network.outbound", "hosts": ["api.pexels.com", "images.pexels.com", "pixabay.com", "cdn.pixabay.com"] },
            { "id": "library.write", "categories": ["media"] }, { "id": "keyring.own" } ] })).send().await.unwrap()).await["token"].as_str().unwrap().to_string();
    let media_url = |body: serde_json::Value, token: Option<String>| {
        let (c, base) = (c.clone(), base.clone());
        async move {
            let mut req = c.post(format!("{base}/api/plugin-host/media-url")).json(&body);
            if let Some(t) = token { req = req.header("x-plugin-token", t); }
            req.send().await.unwrap()
        }
    };
    let fetch = |body: serde_json::Value| {
        let (c, base, token) = (c.clone(), base.clone(), token.clone());
        async move { c.post(format!("{base}/api/plugin-host/fetch")).header("x-plugin-token", token).json(&body).send().await.unwrap() }
    };

    // Downloads need a token, a title, and a granted https host: nothing is requested otherwise.
    assert_eq!(media_url(serde_json::json!({ "url": "https://images.pexels.com/a.jpg", "title": "x" }), None).await.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(media_url(serde_json::json!({ "url": "https://images.pexels.com/a.jpg", "title": "  " }), Some(token.clone())).await.status(), reqwest::StatusCode::BAD_REQUEST);
    for url in ["https://example.com/a.jpg", "http://images.pexels.com/a.jpg", "https://images.pexels.com.evil.test/a.jpg", "https://127.0.0.1/a.jpg", "https://user@images.pexels.com/a.jpg", "file:///etc/passwd"] {
        let r = media_url(serde_json::json!({ "url": url, "title": "x" }), Some(token.clone())).await;
        assert_eq!(r.status(), reqwest::StatusCode::FORBIDDEN, "{url}");
    }
    assert!(db.get_media().unwrap().is_empty(), "nothing was added");

    // A plugin may send its key in Authorization, but not headers that steer the request.
    for h in [serde_json::json!({ "Host": "evil.test" }), serde_json::json!({ "Cookie": "a=b" }), serde_json::json!({ "X-Forwarded-For": "1.2.3.4" })] {
        let r = fetch(serde_json::json!({ "url": "https://api.pexels.com/v1/search?query=x", "headers": h })).await;
        assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST, "{h}");
    }
    // A refused fetch is audited without the query string (a Pixabay key rides in it).
    let r = fetch(serde_json::json!({ "url": "https://example.com/api?key=SECRETKEY123" })).await;
    assert_eq!(r.status(), reqwest::StatusCode::FORBIDDEN);
    let audit = json(c.get(format!("{base}/api/plugins/audit")).header("x-host-token", host).send().await.unwrap()).await.to_string();
    assert!(audit.contains("https://example.com/api") && !audit.contains("SECRETKEY123"), "{audit}");

    // A plugin without a media grant can't use the route at all.
    let narrow = json(c.post(format!("{base}/api/plugins/stock_media/token")).header("x-host-token", host)
        .json(&serde_json::json!({ "scopes": [{ "id": "keyring.own" }] })).send().await.unwrap()).await["token"].as_str().unwrap().to_string();
    assert_eq!(media_url(serde_json::json!({ "url": "https://images.pexels.com/a.jpg", "title": "x" }), Some(narrow)).await.status(), reqwest::StatusCode::FORBIDDEN);
}

/// The Advanced override: the operator grants exactly the permissions they choose, which may
/// differ from what the plugin asked for (extra, withheld, re-scoped), validated and audited.
#[tokio::test]
async fn test_advanced_permission_override() {
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("adv.db").to_str().unwrap()).unwrap();
    let web_dir = tmp.path().join("web");
    let dest = web_dir.join("plugins/stock_media");
    std::fs::create_dir_all(&dest).unwrap();
    for entry in std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/plugins/stock_media")).unwrap().flatten() {
        std::fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        web_dir: web_dir.clone(), media_dir: tmp.path().join("media"), data_dir: tmp.path().to_path_buf(),
        host_session_token: "adv_host".to_string(), server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let host = "adv_host";
    let json = |r: reqwest::Response| async move { r.json::<serde_json::Value>().await.unwrap() };

    // The catalog lists every permission the host knows, console only.
    assert_eq!(c.get(format!("{base}/api/plugins/permissions")).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);
    let catalog = json(c.get(format!("{base}/api/plugins/permissions")).header("x-host-token", host).send().await.unwrap()).await;
    let entry = |id: &str| catalog.as_array().unwrap().iter().find(|e| e["id"] == id).cloned().unwrap_or_else(|| panic!("{id} missing"));
    assert_eq!((entry("network.outbound")["scope"].as_str(), entry("library.write")["scope"].as_str(), entry("show.observe")["scope"].as_str()), (Some("hosts"), Some("categories"), Some("none")));
    assert_eq!(entry("process.run")["tier"], "critical");

    let list = json(c.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap()).await;
    let p = list.as_array().unwrap().iter().find(|p| p["id"] == "stock_media").unwrap().clone();
    let digest = p["digest"].clone();
    let approve = |body: serde_json::Value| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugins/stock_media/approve")).header("x-host-token", "adv_host").json(&body).send().await.unwrap() }
    };

    // The ordinary path still refuses anything not requested, and a required permission can't be dropped.
    assert_eq!(approve(serde_json::json!({ "digest": digest, "granted": ["ui.panel", "network.outbound", "keyring.own", "library.write", "show.observe"] })).await.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(approve(serde_json::json!({ "digest": digest, "granted": ["ui.panel"] })).await.status(), reqwest::StatusCode::BAD_REQUEST);

    // The override validates what the operator types: unknown ids, kinds that don't fit, bad scopes, duplicates.
    for (why, grants) in [
        ("unknown permission", serde_json::json!([{ "id": "made.up" }])),
        ("ui.dom has no meaning for a sandboxed page", serde_json::json!([{ "id": "ui.dom" }])),
        ("native.code no longer exists", serde_json::json!([{ "id": "native.code" }])),
        ("a host that isn't a host", serde_json::json!([{ "id": "network.outbound", "hosts": ["evil.test/path"] }])),
        ("a wildcard in the middle", serde_json::json!([{ "id": "network.outbound", "hosts": ["a.*.test"] }])),
        ("an unknown category", serde_json::json!([{ "id": "library.write", "categories": ["secrets"] }])),
        ("a scope the permission doesn't take", serde_json::json!([{ "id": "commands.send", "categories": ["media"], "commands": ["NextSlide"] }])),
        ("a duplicate", serde_json::json!([{ "id": "show.observe" }, { "id": "show.observe" }])),
        ("commands.modify no longer exists", serde_json::json!([{ "id": "commands.modify", "commands": ["GoLive"] }])),
        ("keyring.service with a namespace that isn't shareable", serde_json::json!([{ "id": "keyring.service", "services": ["OpenSanctuary:System"], "access": "read" }])),
    ] {
        let r = approve(serde_json::json!({ "digest": digest, "override": grants })).await;
        assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST, "{why}");
    }
    // A stale digest still wins over an override.
    assert_eq!(approve(serde_json::json!({ "digest": "stale", "override": [] })).await.status(), reqwest::StatusCode::CONFLICT);

    // A real override: drop required keyring.own and library.write, narrow the hosts, add two the plugin never asked for.
    let r = approve(serde_json::json!({ "digest": digest, "override": [
        { "id": "ui.panel" },
        { "id": "network.outbound", "hosts": ["api.pexels.com"] },
        { "id": "show.observe" },
        { "id": "commands.send", "commands": ["NextSlide", "PrevSlide"] },
    ] })).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v = json(r).await;
    assert_eq!(v["state"], "active");
    assert_eq!(v["overridden"], true);
    let diff = &v["override_diff"];
    assert_eq!(diff["added"], serde_json::json!(["show.observe", "commands.send"]));
    assert_eq!(diff["changed"], serde_json::json!(["network.outbound"]));
    let mut removed: Vec<&str> = diff["removed"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    removed.sort();
    assert_eq!(removed, ["keyring.own", "library.write", "notifications.post"]);

    // It is what is enforced: the frame may reach only the narrowed host, and tokens carry the extras but not what was withheld.
    let r = c.get(format!("{base}/plugin-frame/stock_media/index.html")).header("x-host-token", host).send().await.unwrap();
    let csp = r.headers().get("content-security-policy").unwrap().to_str().unwrap().to_string();
    assert!(csp.contains("https://api.pexels.com") && !csp.contains("pixabay"), "{csp}");
    let mint = |scopes: serde_json::Value| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugins/stock_media/token")).header("x-host-token", "adv_host").json(&serde_json::json!({ "scopes": scopes })).send().await.unwrap() }
    };
    assert_eq!(mint(serde_json::json!([{ "id": "commands.send", "commands": ["NextSlide"] }])).await.status(), reqwest::StatusCode::OK);
    assert_eq!(mint(serde_json::json!([{ "id": "commands.send", "commands": ["GoLive"] }])).await.status(), reqwest::StatusCode::FORBIDDEN, "only the commands the operator chose");
    assert_eq!(mint(serde_json::json!([{ "id": "library.write", "categories": ["media"] }])).await.status(), reqwest::StatusCode::FORBIDDEN, "withheld");
    assert_eq!(mint(serde_json::json!([{ "id": "network.outbound", "hosts": ["pixabay.com"] }])).await.status(), reqwest::StatusCode::FORBIDDEN, "narrowed");

    // The audit trail records the override.
    let audit = json(c.get(format!("{base}/api/plugins/audit")).header("x-host-token", host).send().await.unwrap()).await.to_string();
    assert!(audit.contains("override") && audit.contains("show.observe"), "{audit}");

    // Approving again the ordinary way (all requested, no extras) clears the override.
    let r = approve(serde_json::json!({ "digest": digest, "granted": ["ui.panel", "network.outbound", "keyring.own", "library.write"] })).await;
    let v = json(r).await;
    assert_eq!(v["overridden"], false);
}

/// `keyring.service`: two sandboxed plugins share a keyring namespace under the host's rules.
#[tokio::test]
async fn test_shared_keyring_namespaces_between_plugins() {
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("shared.db").to_str().unwrap()).unwrap();
    let web_dir = tmp.path().join("web");
    const S: &str = "OpenSanctuary:Shared:kp-test-pexels";
    let write_plugin = |id: &str, perms: serde_json::Value| {
        let dir = web_dir.join("plugins").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<html></html>").unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            serde_json::json!({ "manifest_version": 1, "id": id, "name": id, "web": { "entry": "index.html", "sandbox": true }, "permissions": perms }).to_string(),
        ).unwrap();
    };
    write_plugin("kp_a", serde_json::json!([{ "id": "keyring.service", "services": [S], "access": "write" }, { "id": "network.outbound", "hosts": ["example.com"] }]));
    write_plugin("kp_b", serde_json::json!([{ "id": "keyring.service", "services": [S], "access": "read" }, { "id": "keyring.own" }]));
    // Bad shapes are errors on the plugin, not silently narrowed.
    write_plugin("kp_bad1", serde_json::json!([{ "id": "keyring.service", "services": ["OpenSanctuary:System"], "access": "read" }]));
    write_plugin("kp_bad2", serde_json::json!([{ "id": "keyring.service", "services": ["OpenSanctuary:Plugin:kp_a"] }]));
    write_plugin("kp_bad3", serde_json::json!([{ "id": "keyring.service", "services": ["OpenSanctuary:Shared:Upper"] }]));
    write_plugin("kp_bad4", serde_json::json!([{ "id": "keyring.service", "services": [S], "access": "root" }]));
    write_plugin("kp_bad5", serde_json::json!([{ "id": "keyring.service" }]));
    write_plugin("kp_bad6", serde_json::json!([{ "id": "keyring.own", "services": [S] }]));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        web_dir: web_dir.clone(), media_dir: tmp.path().join("media"), data_dir: tmp.path().to_path_buf(),
        host_session_token: "kp_host".to_string(), server_port: addr.port(), ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let host = "kp_host";
    let json = |r: reqwest::Response| async move { r.json::<serde_json::Value>().await.unwrap() };
    let _ = os_next::storage::KeyringService::delete_secret(S, "api_key");

    let list = json(c.get(format!("{base}/api/plugins")).header("x-host-token", host).send().await.unwrap()).await;
    let find = |id: &str| list.as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap_or_else(|| panic!("{id} missing"));
    for bad in ["kp_bad1", "kp_bad2", "kp_bad3", "kp_bad4", "kp_bad5", "kp_bad6"] {
        assert!(find(bad)["error"].is_string(), "{bad} must be refused: {}", find(bad));
    }
    assert!(find("kp_a")["error"].is_null() && find("kp_b")["error"].is_null());
    assert_eq!(find("kp_a")["requested"][0]["tier"], "critical");
    // The review screen says who else shares the namespace.
    assert_eq!(find("kp_a")["shared_with"][0]["plugins"][0]["id"], "kp_b");
    assert_eq!(find("kp_b")["shared_with"][0]["plugins"][0]["access"], "write");

    let approve = |id: &'static str, granted: serde_json::Value| {
        let (c, base, list) = (c.clone(), base.clone(), list.clone());
        async move {
            let digest = list.as_array().unwrap().iter().find(|p| p["id"] == id).unwrap()["digest"].clone();
            c.post(format!("{base}/api/plugins/{id}/approve")).header("x-host-token", "kp_host").json(&serde_json::json!({ "digest": digest, "granted": granted })).send().await.unwrap()
        }
    };
    assert_eq!(approve("kp_a", serde_json::json!(["keyring.service", "network.outbound"])).await.status(), reqwest::StatusCode::OK);
    assert_eq!(approve("kp_b", serde_json::json!(["keyring.service", "keyring.own"])).await.status(), reqwest::StatusCode::OK);
    let mint = |id: &'static str, scopes: serde_json::Value| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugins/{id}/token")).header("x-host-token", "kp_host").json(&serde_json::json!({ "scopes": scopes })).send().await.unwrap() }
    };
    // A token can't carry more than the grant: another service, or write where only read was granted.
    assert_eq!(mint("kp_a", serde_json::json!([{ "id": "keyring.service", "services": ["OpenSanctuary:Shared:other"], "access": "read" }])).await.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(mint("kp_b", serde_json::json!([{ "id": "keyring.service", "services": [S], "access": "write" }])).await.status(), reqwest::StatusCode::FORBIDDEN, "read can't be widened to write");
    assert_eq!(mint("kp_a", serde_json::json!([{ "id": "keyring.service", "services": [S, "OpenSanctuary:Shared:other"], "access": "write" }])).await.status(), reqwest::StatusCode::FORBIDDEN, "no extra service");
    assert_eq!(mint("kp_a", serde_json::json!([{ "id": "keyring.own" }])).await.status(), reqwest::StatusCode::FORBIDDEN, "kp_a never got keyring.own");
    let tok_a = json(mint("kp_a", serde_json::json!([{ "id": "keyring.service", "services": [S], "access": "write" }])).await).await["token"].as_str().unwrap().to_string();
    let tok_b = json(mint("kp_b", serde_json::json!([{ "id": "keyring.service", "services": [S], "access": "read" }, { "id": "keyring.own" }])).await).await["token"].as_str().unwrap().to_string();

    let call = |op: &'static str, token: Option<String>, service: &str, account: &str, secret: Option<&str>| {
        let (c, base, service, account) = (c.clone(), base.clone(), service.to_string(), account.to_string());
        let secret = secret.map(|s| s.to_string());
        async move {
            let mut body = serde_json::json!({ "service": service, "account": account });
            if let Some(s) = secret { body["secret"] = s.into(); }
            let mut r = c.post(format!("{base}/api/keyring/{op}")).json(&body);
            r = match token { Some(t) => r.header("x-plugin-token", t), None => r.header("x-host-token", "kp_host") };
            let res = r.send().await.unwrap();
            (res.status(), res.json::<serde_json::Value>().await.unwrap_or_default())
        }
    };
    let ok = reqwest::StatusCode::OK;
    let forbidden = reqwest::StatusCode::FORBIDDEN;

    // A writes; the row is tagged as a shared row owned by A. B (read) sees it but can't change or delete it.
    assert_eq!(call("set", Some(tok_a.clone()), S, "api_key", Some("pexels-secret-1")).await.0, ok);
    assert_eq!(db.get_keyring_owner(S, "api_key").unwrap().as_deref(), Some("shared:plugin:kp_a"));
    let (st, v) = call("get", Some(tok_b.clone()), S, "api_key", None).await;
    assert_eq!((st, v["secret"].as_str()), (ok, Some("pexels-secret-1")));
    assert_eq!(call("has", Some(tok_b.clone()), S, "api_key", None).await.1["exists"], true);
    assert_eq!(call("set", Some(tok_b.clone()), S, "api_key", Some("swap")).await.0, forbidden, "read access can't set");
    assert_eq!(call("delete", Some(tok_b.clone()), S, "api_key", None).await.0, forbidden);

    // The console can read and replace any shared row; the row becomes the console's and A can no longer overwrite it.
    assert_eq!(call("set", None, S, "api_key", Some("console-value")).await.0, ok);
    assert_eq!(db.get_keyring_owner(S, "api_key").unwrap().as_deref(), Some("shared:host"));
    assert_eq!(call("set", Some(tok_a.clone()), S, "api_key", Some("grab")).await.0, forbidden, "a plugin can't take over a console-owned row");
    assert_eq!(call("get", Some(tok_b.clone()), S, "api_key", None).await.1["secret"], "console-value");
    assert_eq!(call("delete", Some(tok_a.clone()), S, "api_key", None).await.0, forbidden);

    // A row that exists under a shared-looking name but wasn't created through this feature is invisible.
    os_next::storage::KeyringService::set_secret(S, "legacy", "old-secret").unwrap();
    db.record_keyring_owner(S, "legacy", "host").unwrap();
    assert_eq!(call("get", Some(tok_b.clone()), S, "legacy", None).await.1["secret"], serde_json::Value::Null);
    assert_eq!(call("has", Some(tok_b.clone()), S, "legacy", None).await.1["exists"], false);
    assert_eq!(call("get", Some(tok_b.clone()), S, "never-stored", None).await.1["secret"], serde_json::Value::Null);

    // Nothing outside the shared area through the shared grant: not another plugin's namespace, not the host's services,
    // not the System namespace -- and A's token has no keyring.own, so not even its own.
    for service in ["OpenSanctuary:Plugin:kp_b", "OpenSanctuary:Plugin:kp_a", "OpenSanctuary:CCLI", "OpenSanctuary:Shared:other", "OpenSanctuary:System"] {
        assert_eq!(call("set", Some(tok_a.clone()), service, "x", Some("v")).await.0, forbidden, "{service}");
        assert_ne!(call("get", Some(tok_a.clone()), service, "x", None).await.1["secret"], serde_json::json!("v"), "{service}");
    }
    // kp_b has keyring.own too: its own namespace works as before, and doesn't leak to kp_a.
    assert_eq!(call("set", Some(tok_b.clone()), "OpenSanctuary:Plugin:kp_b", "own", Some("mine")).await.0, ok);
    assert_eq!(call("get", Some(tok_b.clone()), "OpenSanctuary:Plugin:kp_b", "own", None).await.1["secret"], "mine");
    assert_eq!(call("get", Some(tok_a.clone()), "OpenSanctuary:Plugin:kp_b", "own", None).await.0, forbidden);
    // The System namespace is blocked by prefix and in any case, even for the console.
    for service in ["OpenSanctuary:System", "opensanctuary:system", "OpenSanctuary:SystemTls", "OpenSanctuary:system:x"] {
        assert_eq!(call("get", None, service, "x", None).await.0, forbidden, "{service}");
    }

    // The console lists shared entries (never values); a plugin can't.
    let listing = c.get(format!("{base}/api/keyring/shared")).header("x-host-token", host).send().await.unwrap();
    let text = listing.text().await.unwrap();
    assert!(text.contains("api_key") && text.contains("shared:host") && !text.contains("console-value") && !text.contains("old-secret"), "{text}");
    assert_eq!(c.get(format!("{base}/api/keyring/shared")).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(c.get(format!("{base}/api/keyring/shared")).header("x-plugin-token", &tok_a).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);

    // The audit trail records first use and refusals, never the secret.
    let audit = json(c.get(format!("{base}/api/plugins/audit")).header("x-host-token", host).send().await.unwrap()).await.to_string();
    assert!(audit.contains("keyring_shared_access") && audit.contains("denied_call") && !audit.contains("pexels-secret-1") && !audit.contains("console-value"), "{audit}");

    // A's own write can be deleted by A (its own row), and revoking kills the token at once.
    assert_eq!(call("set", Some(tok_a.clone()), S, "mine", Some("a-secret")).await.0, ok);
    assert_eq!(call("delete", Some(tok_a.clone()), S, "mine", None).await.0, ok);
    assert_eq!(c.post(format!("{base}/api/plugins/kp_a/revoke")).header("x-host-token", host).send().await.unwrap().status(), ok);
    assert_eq!(call("get", Some(tok_a.clone()), S, "api_key", None).await.0, reqwest::StatusCode::UNAUTHORIZED);

    // Cleanup of real keyring entries this test created.
    for (svc, acct) in [(S, "api_key"), (S, "legacy"), (S, "mine"), ("OpenSanctuary:Plugin:kp_b", "own")] {
        let _ = os_next::storage::KeyringService::delete_secret(svc, acct);
    }
}

#[tokio::test]
async fn test_plugin_install_and_uninstall() {
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("inst.db").to_str().unwrap()).unwrap();
    let web_dir = tmp.path().join("web");
    std::fs::create_dir_all(web_dir.join("plugins/builtin_one")).unwrap();
    std::fs::write(web_dir.join("plugins/builtin_one/plugin.json"), plugin_manifest_json("builtin_one")).unwrap();
    std::fs::write(web_dir.join("plugins/builtin_one/index.html"), "<html></html>").unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        web_dir: web_dir.clone(), media_dir: tmp.path().join("media"), data_dir: data_dir.clone(),
        host_session_token: "inst_host".to_string(), server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let post = |query: &'static str, body: Vec<u8>| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.post(format!("{base}/api/plugins/install{query}")).header("x-host-token", "inst_host").body(body).send().await.unwrap() }
    };
    let ok_pkg = make_zip(&[("plugin.json", plugin_manifest_json("hello_pkg").as_bytes()), ("index.html", b"<html>v1</html>")]);

    // Console only.
    let anon = c.post(format!("{base}/api/plugins/install")).body(ok_pkg.clone()).send().await.unwrap();
    assert_eq!(anon.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Installing grants nothing: the plugin shows up as needing review, with files under the data dir.
    let r = post("", ok_pkg.clone()).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK, "{}", r.text().await.unwrap());
    let list: serde_json::Value = c.get(format!("{base}/api/plugins")).header("x-host-token", "inst_host").send().await.unwrap().json().await.unwrap();
    let p = list.as_array().unwrap().iter().find(|p| p["id"] == "hello_pkg").expect("installed plugin is listed").clone();
    assert_eq!((p["state"].as_str(), p["removable"].as_bool(), p["error"].is_null()), (Some("pending"), Some(true), true), "{p}");
    assert!(data_dir.join("plugins/hello_pkg/index.html").is_file());
    let builtin = list.as_array().unwrap().iter().find(|p| p["id"] == "builtin_one").unwrap();
    assert_eq!(builtin["removable"], false);
    // Not served until approved.
    assert_eq!(c.get(format!("{base}/plugin-frame/hello_pkg/index.html")).send().await.unwrap().status(), reqwest::StatusCode::NOT_FOUND);

    // Approve it, then it is served.
    let digest = p["digest"].clone();
    let a = c.post(format!("{base}/api/plugins/hello_pkg/approve")).header("x-host-token", "inst_host")
        .json(&serde_json::json!({ "digest": digest, "granted": ["ui.panel", "keyring.own"] })).send().await.unwrap();
    assert_eq!(a.status(), reqwest::StatusCode::OK);
    let served = c.get(format!("{base}/plugin-frame/hello_pkg/index.html")).send().await.unwrap();
    assert_eq!((served.status().as_u16(), served.text().await.unwrap().as_str()), (200, "<html>v1</html>"));

    // Same id again needs to say it is an update; an update drops the approval back to review.
    assert_eq!(post("", ok_pkg.clone()).await.status(), reqwest::StatusCode::CONFLICT);
    let v2 = make_zip(&[("hello_pkg/plugin.json", plugin_manifest_json("hello_pkg").as_bytes()), ("hello_pkg/index.html", b"<html>v2</html>")]);
    assert_eq!(post("?replace=true", v2).await.status(), reqwest::StatusCode::OK); // one wrapping folder is fine
    let after: serde_json::Value = c.get(format!("{base}/api/plugins")).header("x-host-token", "inst_host").send().await.unwrap().json().await.unwrap();
    let state = after.as_array().unwrap().iter().find(|p| p["id"] == "hello_pkg").unwrap()["state"].clone();
    assert_eq!(state, "changed");
    // The review says what changed: one file edited, no new permissions, same version number.
    let changes = after.as_array().unwrap().iter().find(|p| p["id"] == "hello_pkg").unwrap()["changes"].clone();
    assert_eq!(changes["files"]["modified"], serde_json::json!(["index.html"]), "{changes}");
    assert_eq!((changes["files"]["added"].as_array().map(Vec::len), changes["permissions"]["added"].as_array().map(Vec::len), changes["direction"].as_str()), (Some(0), Some(0), Some("same")));
    assert_eq!(c.get(format!("{base}/plugin-frame/hello_pkg/index.html")).send().await.unwrap().status(), reqwest::StatusCode::NOT_FOUND);

    // Hostile or broken packages are refused, and leave nothing behind.
    let big = vec![b'a'; 30 * 1024 * 1024];
    let bomb = make_zip(&[("plugin.json", plugin_manifest_json("bomb").as_bytes()), ("index.html", b"<html></html>"), ("big.bin", &big)]);
    let many: Vec<(String, Vec<u8>)> = (0..350).map(|i| (format!("f{i}.txt"), vec![b'x'])).collect();
    let mut many_entries: Vec<(&str, &[u8])> = many.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    many_entries.push(("plugin.json", b"{}"));
    let hostile: Vec<(&str, Vec<u8>)> = vec![
        ("not a zip", b"hello".to_vec()),
        ("path traversal", make_zip(&[("plugin.json", plugin_manifest_json("evil1").as_bytes()), ("../../escape.txt", b"x")])),
        ("absolute path", make_zip(&[("plugin.json", plugin_manifest_json("evil2").as_bytes()), ("/tmp/escape.txt", b"x")])),
        ("backslash path", make_zip(&[("plugin.json", plugin_manifest_json("evil3").as_bytes()), ("a\\..\\..\\x.txt", b"x")])),
        ("no manifest", make_zip(&[("index.html", b"<html></html>")])),
        ("bad manifest", make_zip(&[("plugin.json", b"{ nope"), ("index.html", b"<html></html>")])),
        ("native plugin", make_zip(&[("plugin.json", serde_json::json!({"manifest_version":1,"id":"nat","name":"n","backend":{"library":"x.so"}}).to_string().as_bytes()), ("index.html", b"x")])),
        ("clashes with a built-in", make_zip(&[("plugin.json", plugin_manifest_json("builtin_one").as_bytes()), ("index.html", b"<html>fake</html>")])),
        ("reserved id", make_zip(&[("plugin.json", plugin_manifest_json("__os").as_bytes()), ("index.html", b"x")])),
        ("entry missing", make_zip(&[("plugin.json", plugin_manifest_json("noentry").as_bytes())])),
        ("zip bomb", bomb),
        ("too many files", make_zip(&many_entries)),
    ];
    for (why, pkg) in hostile {
        let r = post("?replace=true", pkg).await;
        assert!(r.status().is_client_error(), "{why}: {}", r.status());
    }
    assert!(!data_dir.join("escape.txt").exists() && !tmp.path().join("escape.txt").exists() && !std::path::Path::new("/tmp/escape.txt").exists());
    assert_eq!(std::fs::read_to_string(web_dir.join("plugins/builtin_one/index.html")).unwrap(), "<html></html>", "a built-in is never overwritten");
    let names: Vec<String> = std::fs::read_dir(data_dir.join("plugins")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert_eq!(names, ["hello_pkg"], "failed installs leave nothing behind: {names:?}");
    let staging_left = std::fs::read_dir(data_dir.join("plugins-staging")).map(|d| d.count()).unwrap_or(0);
    assert_eq!(staging_left, 0, "staging is cleaned up");

    // Removal: built-ins refuse, an installed plugin goes completely, and its id is free again.
    let del = |id: &'static str| {
        let (c, base) = (c.clone(), base.clone());
        async move { c.delete(format!("{base}/api/plugins/{id}")).header("x-host-token", "inst_host").send().await.unwrap() }
    };
    assert_eq!(del("builtin_one").await.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(del("nope").await.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(c.delete(format!("{base}/api/plugins/hello_pkg")).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(del("hello_pkg").await.status(), reqwest::StatusCode::OK);
    assert!(!data_dir.join("plugins/hello_pkg").exists());
    assert!(db.get_plugin_grant("hello_pkg").unwrap().is_none());
    let audit = db.list_plugin_audit(50).unwrap();
    assert!(audit.iter().any(|a| a.plugin_id == "hello_pkg" && a.action == "uninstalled"));
    assert!(audit.iter().any(|a| a.plugin_id == "hello_pkg" && a.action == "installed"));
    assert_eq!(post("", ok_pkg).await.status(), reqwest::StatusCode::OK);
}

#[test]
fn test_plugin_author_workflow_hash_sign_check() {
    // The workflow docs/PLUGINS.md tells authors to follow: --hash-plugin, set publisher_key, sign with minisign, --check-plugin.
    let exe = env!("CARGO_BIN_EXE_os-next");
    let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("author_demo");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("index.html"), "<html>hi</html>").unwrap();
    std::fs::write(dir.join("lib/app.js"), "var x = 1;").unwrap();
    std::fs::write(dir.join("plugin.json"), serde_json::json!({
        "manifest_version": 1, "id": "author_demo", "name": "Author Demo", "version": "1.0.0",
        "web": {"entry": "index.html", "sandbox": true}, "permissions": [{"id": "ui.panel", "reason": "shows a panel"}]
    }).to_string()).unwrap();
    let run = |flag: &str| std::process::Command::new(exe).args([flag, dir.to_str().unwrap(), "--web-dir", web.to_str().unwrap()]).output().unwrap();
    let text = |o: &std::process::Output| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));

    // Unsigned to begin with.
    let o = run("--check-plugin");
    assert!(o.status.success() && text(&o).contains("signature: unsigned"), "{}", text(&o));

    // Hash the files: the manifest now lists them (and only them).
    let o = run("--hash-plugin");
    assert!(o.status.success() && text(&o).contains("listed 2 files"), "{}", text(&o));
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("plugin.json")).unwrap()).unwrap();
    let listed: Vec<&String> = manifest["files"].as_object().unwrap().keys().collect();
    assert_eq!(listed, ["index.html", "lib/app.js"]);
    assert_eq!(manifest["name"], "Author Demo", "other fields survive");

    // Set the key and sign the manifest as it is.
    let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let mut manifest = manifest;
    manifest["publisher_key"] = serde_json::json!(kp.pk.to_base64());
    let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    std::fs::write(dir.join("plugin.json"), &bytes).unwrap();
    std::fs::write(dir.join("plugin.json.minisig"), minisign::sign(None, &kp.sk, &bytes[..], None, None).unwrap().into_string()).unwrap();
    let o = run("--check-plugin");
    assert!(o.status.success() && text(&o).contains("fully signed"), "{}", text(&o));

    // Edit a file after signing: refused, with the file named.
    std::fs::write(dir.join("lib/app.js"), "steal();").unwrap();
    let o = run("--check-plugin");
    assert!(!o.status.success() && text(&o).contains("lib/app.js"), "{}", text(&o));

    // Running --hash-plugin again after an edit warns that the signature is stale.
    let o = run("--hash-plugin");
    assert!(o.status.success() && text(&o).contains("out of date"), "{}", text(&o));
    let o = run("--check-plugin");
    assert!(!o.status.success() && text(&o).contains("signature"), "a re-hashed manifest no longer matches its signature: {}", text(&o));
}

#[tokio::test]
async fn test_publisher_trust_levels_and_revocation() {
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("trust.db").to_str().unwrap()).unwrap();
    let web_dir = tmp.path().join("web");
    std::fs::create_dir_all(web_dir.join("plugins/builtin_one")).unwrap();
    std::fs::write(web_dir.join("plugins/builtin_one/plugin.json"), plugin_manifest_json("builtin_one")).unwrap();
    std::fs::write(web_dir.join("plugins/builtin_one/index.html"), "<html></html>").unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        web_dir: web_dir.clone(), media_dir: tmp.path().join("media"), data_dir: data_dir.clone(),
        host_session_token: "trust_host".to_string(), server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let h = "trust_host";
    let install = |pkg: Vec<u8>| { let (c, base) = (c.clone(), base.clone()); async move {
        let r = c.post(format!("{base}/api/plugins/install")).header("x-host-token", h).body(pkg).send().await.unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::OK);
        r.json::<serde_json::Value>().await.unwrap()
    } };
    let get = |id: &str| { let (c, base, id) = (c.clone(), base.clone(), id.to_string()); async move {
        let list: serde_json::Value = c.get(format!("{base}/api/plugins")).header("x-host-token", h).send().await.unwrap().json().await.unwrap();
        list.as_array().unwrap().iter().find(|p| p["id"] == id.as_str()).cloned().unwrap_or_else(|| panic!("{id} missing"))
    } };
    let approve = |id: &str, digest: serde_json::Value, extra: serde_json::Value| { let (c, base, id) = (c.clone(), base.clone(), id.to_string()); async move {
        let mut body = serde_json::json!({ "digest": digest, "granted": ["ui.panel"] });
        for (k, v) in extra.as_object().unwrap() { body[k] = v.clone(); }
        c.post(format!("{base}/api/plugins/{id}/approve")).header("x-host-token", h).json(&body).send().await.unwrap()
    } };
    let post = |path: String| { let (c, base) = (c.clone(), base.clone()); async move { c.post(format!("{base}{path}")).header("x-host-token", h).send().await.unwrap() } };
    let k1 = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let k2 = minisign::KeyPair::generate_unencrypted_keypair().unwrap();

    // Levels at first sight: built-in, unsigned, and a signer nobody has met.
    install(make_zip(&[("plugin.json", plugin_manifest_json("plain").as_bytes()), ("index.html", b"<html></html>")])).await;
    install(signed_package("a1", "Acme", &k1)).await;
    let (builtin, plain, a1) = (get("builtin_one").await, get("plain").await, get("a1").await);
    assert_eq!((builtin["trust"]["level"].as_str(), plain["trust"]["level"].as_str(), a1["trust"]["level"].as_str()), (Some("builtin"), Some("unsigned"), Some("new")), "{a1}");
    assert_eq!((a1["trust"]["can_trust"].as_bool(), a1["trust"]["publisher_name"].as_str(), a1["trust"]["name_is_claim"].as_bool()), (Some(true), Some("Acme"), Some(true)), "a self-declared name is a claim");
    assert_eq!(builtin["trust"]["can_trust"], false);
    assert_eq!(plain["trust"]["can_trust"], false);
    let key_id = a1["trust"]["key_id"].as_str().unwrap().to_string();

    // Approving with "trust this publisher" trusts the key; another plugin from it is then trusted, a different key isn't.
    assert_eq!(approve("plain", plain["digest"].clone(), serde_json::json!({ "trust_publisher": true })).await.status(), reqwest::StatusCode::BAD_REQUEST, "unsigned plugins have no publisher to trust");
    assert_eq!(approve("a1", a1["digest"].clone(), serde_json::json!({ "trust_publisher": true })).await.status(), reqwest::StatusCode::OK);
    install(signed_package("a2", "Acme", &k1)).await;
    install(signed_package("b1", "Other", &k2)).await;
    assert_eq!(get("a1").await["trust"]["level"], "trusted");
    assert_eq!(get("a2").await["trust"]["name_is_claim"], false, "the trusted name is the one the operator saw");
    assert_eq!(get("a2").await["trust"]["level"], "trusted");
    assert_eq!(get("b1").await["trust"]["level"], "new");
    let pubs: serde_json::Value = c.get(format!("{base}/api/plugins/publishers")).header("x-host-token", h).send().await.unwrap().json().await.unwrap();
    assert_eq!(pubs.as_array().unwrap().len(), 1);
    assert_eq!((pubs[0]["key_id"].as_str(), pubs[0]["trusted"].as_bool(), pubs[0]["name"].as_str()), (Some(key_id.as_str()), Some(true), Some("Acme")));
    assert_eq!(pubs[0]["plugins"], serde_json::json!(["a1", "a2"]));

    // Approving without trusting makes the key "known" (not trusted) for its other plugins.
    let b1 = get("b1").await;
    assert_eq!(approve("b1", b1["digest"].clone(), serde_json::json!({})).await.status(), reqwest::StatusCode::OK);
    install(signed_package("b2", "Other", &k2)).await;
    assert_eq!(get("b2").await["trust"]["level"], "known");
    // ... and the operator can then trust it by hand, and undo that.
    let b_id = get("b2").await["trust"]["key_id"].as_str().unwrap().to_string();
    assert_eq!(post(format!("/api/plugins/publishers/{b_id}/trust")).await.status(), reqwest::StatusCode::OK);
    assert_eq!(get("b2").await["trust"]["level"], "trusted");
    assert_eq!(post(format!("/api/plugins/publishers/{b_id}/untrust")).await.status(), reqwest::StatusCode::OK);
    assert_eq!(get("b2").await["trust"]["level"], "known");

    // Revoking the first publisher stops what it signed and was approved, and marks the rest.
    let r: serde_json::Value = post(format!("/api/plugins/publishers/{key_id}/revoke")).await.json().await.unwrap();
    assert_eq!(r["stopped"], serde_json::json!(["a1"]));
    assert_eq!(get("a1").await["state"], "revoked");
    let a2 = get("a2").await;
    assert_eq!(a2["trust"]["level"], "revoked");
    assert_eq!(get("builtin_one").await["trust"]["level"], "builtin", "built-ins are unaffected");
    // Approving something it signed needs an explicit confirmation, and "trust" can't be combined with it.
    assert_eq!(approve("a2", a2["digest"].clone(), serde_json::json!({})).await.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(approve("a2", a2["digest"].clone(), serde_json::json!({ "confirm_revoked_publisher": true, "trust_publisher": true })).await.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(post(format!("/api/plugins/publishers/{key_id}/trust")).await.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(approve("a2", a2["digest"].clone(), serde_json::json!({ "confirm_revoked_publisher": true })).await.status(), reqwest::StatusCode::OK);
    let audit = db.list_plugin_audit(100).unwrap();
    assert!(audit.iter().any(|a| a.plugin_id == "a2" && a.action == "approved" && a.detail_json.contains("\"revoked_publisher_accepted\":true")));
    for action in ["publisher_trusted", "publisher_untrusted", "publisher_revoked"] {
        assert!(audit.iter().any(|a| a.action == action), "{action} is audited");
    }

    // Everything here is console-only.
    for (method, path) in [("GET", "/api/plugins/publishers".to_string()), ("POST", format!("/api/plugins/publishers/{b_id}/trust")), ("POST", format!("/api/plugins/publishers/{b_id}/untrust")), ("POST", format!("/api/plugins/publishers/{b_id}/revoke"))] {
        let req = if method == "GET" { c.get(format!("{base}{path}")) } else { c.post(format!("{base}{path}")) };
        assert_eq!(req.send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED, "{path}");
    }
    assert_eq!(post("/api/plugins/publishers/0000000000000000/trust".to_string()).await.status(), reqwest::StatusCode::NOT_FOUND);
}

/// Plugin signing phase 4: the `pluginSignaturePolicy` setting (docs/PLUGIN_SIGNING.md section 6)
/// and the version floor that remembers a downgrade across revoke/re-approve (section 7).
#[tokio::test]
async fn test_plugin_signature_policy_and_version_floor() {
    let unsigned_manifest = |id: &str, version: &str, perms: serde_json::Value| {
        serde_json::json!({
            "manifest_version": 1, "id": id, "name": format!("Plugin {id}"), "version": version,
            "web": {"entry": "index.html", "sandbox": true}, "permissions": perms,
        }).to_string()
    };
    let pkg = |id: &str, version: &str, perms: serde_json::Value| {
        make_zip(&[("plugin.json", unsigned_manifest(id, version, perms).as_bytes()), ("index.html", b"<html></html>")])
    };
    let non_critical = || serde_json::json!([{ "id": "ui.panel", "reason": "x" }]);
    let critical = || serde_json::json!([{ "id": "keyring.service", "services": ["OpenSanctuary:Shared:policy-test"], "reason": "x" }]);

    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("policy.db").to_str().unwrap()).unwrap();
    let web_dir = tmp.path().join("web");
    // An unsigned built-in: `require` must never touch it.
    std::fs::create_dir_all(web_dir.join("plugins/builtin_u")).unwrap();
    std::fs::write(web_dir.join("plugins/builtin_u/plugin.json"), unsigned_manifest("builtin_u", "1.0.0", non_critical())).unwrap();
    std::fs::write(web_dir.join("plugins/builtin_u/index.html"), "<html></html>").unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        web_dir: web_dir.clone(), media_dir: tmp.path().join("media"), data_dir: data_dir.clone(),
        host_session_token: "policy_host".to_string(), server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let h = "policy_host";
    let install = |body: Vec<u8>| { let (c, base) = (c.clone(), base.clone()); async move {
        let r = c.post(format!("{base}/api/plugins/install")).header("x-host-token", h).body(body).send().await.unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::OK, "{}", r.text().await.unwrap_or_default());
    } };
    let get = |id: &str| { let (c, base, id) = (c.clone(), base.clone(), id.to_string()); async move {
        let list: serde_json::Value = c.get(format!("{base}/api/plugins")).header("x-host-token", h).send().await.unwrap().json().await.unwrap();
        list.as_array().unwrap().iter().find(|p| p["id"] == id.as_str()).cloned().unwrap_or_else(|| panic!("{id} missing"))
    } };
    let approve = |id: &str, granted: &[&str], digest: serde_json::Value, extra: serde_json::Value| { let (c, base, id, granted) = (c.clone(), base.clone(), id.to_string(), granted.iter().map(|s| s.to_string()).collect::<Vec<_>>()); async move {
        let mut body = serde_json::json!({ "digest": digest, "granted": granted });
        for (k, v) in extra.as_object().unwrap() { body[k] = v.clone(); }
        c.post(format!("{base}/api/plugins/{id}/approve")).header("x-host-token", h).json(&body).send().await.unwrap()
    } };
    let revoke = |id: &str| { let (c, base, id) = (c.clone(), base.clone(), id.to_string()); async move {
        c.post(format!("{base}/api/plugins/{id}/revoke")).header("x-host-token", h).send().await.unwrap()
    } };
    let uninstall = |id: &str| { let (c, base, id) = (c.clone(), base.clone(), id.to_string()); async move {
        c.delete(format!("{base}/api/plugins/{id}")).header("x-host-token", h).send().await.unwrap()
    } };

    // Default policy (nothing set yet) is `allow`: an unsigned, non-critical plugin needs no confirmation.
    install(pkg("u1", "1.0.0", non_critical())).await;
    assert_eq!(approve("u1", &["ui.panel"], get("u1").await["digest"].clone(), serde_json::json!({})).await.status(), reqwest::StatusCode::OK);

    // Independent of policy: an unsigned plugin granted a Critical permission always needs confirmation.
    install(pkg("u2", "1.0.0", critical())).await;
    let u2_digest = get("u2").await["digest"].clone();
    assert_eq!(approve("u2", &["keyring.service"], u2_digest.clone(), serde_json::json!({})).await.status(), reqwest::StatusCode::CONFLICT, "still under allow, but Critical+unsigned always asks");
    assert_eq!(approve("u2", &["keyring.service"], u2_digest, serde_json::json!({ "confirm_unsigned": true })).await.status(), reqwest::StatusCode::OK);

    // `warn`: now ANY unsigned plugin needs confirmation, critical or not.
    db.set_setting("pluginSignaturePolicy", "warn").unwrap();
    install(pkg("u3", "1.0.0", non_critical())).await;
    let u3_digest = get("u3").await["digest"].clone();
    assert_eq!(approve("u3", &["ui.panel"], u3_digest.clone(), serde_json::json!({})).await.status(), reqwest::StatusCode::CONFLICT, "warn asks for every unsigned plugin");
    assert_eq!(approve("u3", &["ui.panel"], u3_digest, serde_json::json!({ "confirm_unsigned": true })).await.status(), reqwest::StatusCode::OK);

    // `require`: a confirmation no longer helps a non-built-in unsigned plugin -- it's refused outright.
    db.set_setting("pluginSignaturePolicy", "require").unwrap();
    install(pkg("u4", "1.0.0", non_critical())).await;
    let u4_digest = get("u4").await["digest"].clone();
    assert_eq!(approve("u4", &["ui.panel"], u4_digest.clone(), serde_json::json!({})).await.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(approve("u4", &["ui.panel"], u4_digest, serde_json::json!({ "confirm_unsigned": true })).await.status(), reqwest::StatusCode::FORBIDDEN, "no confirmation unlocks `require`");
    // But a built-in is never touched by any policy value, signed or not.
    assert_eq!(approve("builtin_u", &["ui.panel"], get("builtin_u").await["digest"].clone(), serde_json::json!({})).await.status(), reqwest::StatusCode::OK);
    assert_eq!(get("builtin_u").await["state"], "active");
    // u1, approved earlier under `allow`, is still approved -- but `require` now stops it from
    // actually loading, a distinct honest state rather than silently claiming "active".
    assert_eq!(get("u1").await["state"], "policy_blocked");
    // Loosen it back: u1 recovers without having to be re-approved.
    db.set_setting("pluginSignaturePolicy", "allow").unwrap();
    assert_eq!(get("u1").await["state"], "active");

    // Version floor (section 7): u1 is approved at 1.0.0. Bump it to 2.0.0 and approve that too.
    std::fs::write(data_dir.join("plugins/u1/plugin.json"), unsigned_manifest("u1", "2.0.0", non_critical())).unwrap();
    let u1_v2 = get("u1").await;
    assert_eq!(u1_v2["state"], "changed");
    assert_eq!(approve("u1", &["ui.panel"], u1_v2["digest"].clone(), serde_json::json!({})).await.status(), reqwest::StatusCode::OK);
    assert_eq!(get("u1").await["version_floor"], "2.0.0");

    // Revoke it, then bring back 1.0.0 -- an older, previously-superseded version signed by the
    // same (lack of a) key. The floor remembers 2.0.0 regardless of the revoke, and flags this as
    // a downgrade even before it's re-approved.
    assert_eq!(revoke("u1").await.status(), reqwest::StatusCode::OK);
    std::fs::write(data_dir.join("plugins/u1/plugin.json"), unsigned_manifest("u1", "1.0.0", non_critical())).unwrap();
    let u1_v1_again = get("u1").await;
    assert_eq!(u1_v1_again["version"], "1.0.0");
    assert_eq!(u1_v1_again["version_floor"], "2.0.0", "the floor survives the revoke");
    assert_eq!(u1_v1_again["is_downgrade_from_floor"], true);
    // Re-approving the downgrade is allowed (flagged, not blocked) -- and must not lower the floor.
    assert_eq!(approve("u1", &["ui.panel"], u1_v1_again["digest"].clone(), serde_json::json!({})).await.status(), reqwest::StatusCode::OK);
    assert_eq!(get("u1").await["version_floor"], "2.0.0", "approving an older version never lowers the remembered floor");

    // The floor also has to survive a full uninstall/reinstall, not just a revoke -- otherwise an
    // attacker could erase the memory simply by removing and reinstalling an old, vulnerable
    // version instead of going through revoke/re-approve. Uninstalling drops the plugin_grants
    // row entirely (a fresh id, fresh first_seen on reinstall); the floor must not be keyed to it.
    assert_eq!(uninstall("u1").await.status(), reqwest::StatusCode::OK);
    install(pkg("u1", "0.5.0", non_critical())).await;
    let u1_reinstalled = get("u1").await;
    assert_eq!(u1_reinstalled["state"], "pending", "a fresh install starts over, same as any new plugin");
    assert_eq!(u1_reinstalled["version_floor"], "2.0.0", "the floor survives uninstall/reinstall, not just revoke");
    assert_eq!(u1_reinstalled["is_downgrade_from_floor"], true);
    assert_eq!(approve("u1", &["ui.panel"], u1_reinstalled["digest"].clone(), serde_json::json!({})).await.status(), reqwest::StatusCode::OK, "a downgrade is flagged, not blocked, even after a clean reinstall");
    assert_eq!(get("u1").await["version_floor"], "2.0.0");
}

impl Harness {
    async fn start(release: &TestKey) -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let db = os_next::storage::Database::new(tmp.path().join("sig3.db").to_str().unwrap()).unwrap();
        let web_dir = tmp.path().join("web");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(web_dir.join("plugins")).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app_state = os_next::api::ws::AppState {
            web_dir: web_dir.clone(), media_dir: tmp.path().join("media"), data_dir: data_dir.clone(),
            host_session_token: H.to_string(), server_port: addr.port(), plugin_release_key: release.b64.clone(),
            ..base_app_state(db.clone())
        };
        let router = os_next::api::create_router(app_state);
        tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
        Harness { base: format!("http://{addr}"), c: reqwest::Client::new(), web_dir, data_dir, db, _tmp: tmp }
    }
    async fn install(&self, pkg: Vec<u8>, replace: bool) -> reqwest::Response {
        self.c.post(format!("{}/api/plugins/install{}", self.base, if replace { "?replace=true" } else { "" })).header("x-host-token", H).body(pkg).send().await.unwrap()
    }
    async fn plugin(&self, id: &str) -> serde_json::Value {
        let list: serde_json::Value = self.c.get(format!("{}/api/plugins", self.base)).header("x-host-token", H).send().await.unwrap().json().await.unwrap();
        list.as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap_or_else(|| panic!("{id} missing"))
    }
    async fn approve(&self, id: &str, extra: serde_json::Value) -> reqwest::StatusCode {
        let p = self.plugin(id).await;
        let mut body = serde_json::json!({ "digest": p["digest"], "granted": ["ui.panel"] });
        for (k, v) in extra.as_object().unwrap() { body[k] = v.clone(); }
        self.c.post(format!("{}/api/plugins/{id}/approve", self.base)).header("x-host-token", H).json(&body).send().await.unwrap().status()
    }
    async fn post(&self, path: &str) -> reqwest::Response {
        self.c.post(format!("{}{path}", self.base)).header("x-host-token", H).send().await.unwrap()
    }
    async fn revocations(&self) -> serde_json::Value {
        self.c.get(format!("{}/api/plugins/revocations", self.base)).header("x-host-token", H).send().await.unwrap().json().await.unwrap()
    }
}

#[tokio::test]
async fn test_revocation_list_is_signed_ordered_and_cannot_touch_the_release_key() {
    let (release, acme, rival) = (test_key("release"), test_key("acme"), test_key("rival"));
    let h = Harness::start(&release).await;
    assert_eq!(h.install(signed_zip("acme_one", "1.0.0", &acme, "<html>a</html>", &[]), false).await.status(), reqwest::StatusCode::OK);
    assert_eq!(h.install(signed_zip("rival_one", "1.0.0", &rival, "<html>r</html>", &[]), false).await.status(), reqwest::StatusCode::OK);
    assert_eq!(h.approve("acme_one", serde_json::json!({ "trust_publisher": true })).await, reqwest::StatusCode::OK);
    assert_eq!(h.approve("rival_one", serde_json::json!({})).await, reqwest::StatusCode::OK);
    assert_eq!(h.plugin("rival_one").await["state"], "active");

    // A list signed by the wrong key (a publisher, not the release key) does nothing, and says why.
    write_revocation_list(&h.web_dir, &acme, 1, &[(&rival.b64, "forged")]);
    assert_eq!(h.plugin("rival_one").await["state"], "active");
    let st = h.revocations().await;
    assert_eq!((st["applied_sequence"].as_u64(), st["newest_valid_sequence"].is_null()), (Some(0), true), "{st}");
    assert!(st["problems"][0].as_str().unwrap().contains("revoked_keys.json"), "{st}");

    // A list edited after it was signed is no better.
    write_revocation_list(&h.web_dir, &release, 1, &[(&rival.b64, "real")]);
    let text = std::fs::read_to_string(h.web_dir.join("revoked_keys.json")).unwrap().replace(&rival.b64, &acme.b64);
    std::fs::write(h.web_dir.join("revoked_keys.json"), text).unwrap();
    assert_eq!(h.plugin("acme_one").await["state"], "active");
    assert_eq!(h.plugin("acme_one").await["trust"]["level"], "trusted");

    // A genuine list: the key is revoked and what it signed stops; the other publisher is untouched.
    write_revocation_list(&h.web_dir, &release, 1, &[(&rival.b64, "key leaked"), (&release.b64, "try to revoke the release key"), ("not a key", "junk")]);
    assert_eq!(h.plugin("rival_one").await["state"], "revoked");
    assert_eq!(h.plugin("rival_one").await["trust"]["level"], "revoked");
    assert_eq!(h.plugin("acme_one").await["state"], "active");
    assert_eq!(h.revocations().await["applied_sequence"], 1);
    let publishers = h.db.list_publishers().unwrap();
    let rival_row = publishers.iter().find(|p| p.key == rival.b64).expect("rival is on record as revoked");
    assert!(rival_row.revoked && rival_row.note.contains("revocation list #1: key leaked"), "{rival_row:?}");
    assert!(publishers.iter().all(|p| p.key != release.b64 && p.key != "not a key"), "the release key and junk are skipped");
    let audit = h.db.list_plugin_audit(100).unwrap();
    assert!(audit.iter().any(|a| a.plugin_id == "rival_one" && a.action == "revoked" && a.actor == "revocation list"));

    // Approving it again needs the explicit confirmation, exactly as for a manual revoke.
    assert_eq!(h.approve("rival_one", serde_json::json!({})).await, reqwest::StatusCode::CONFLICT);

    // A newer list in the data directory wins over the shipped one.
    write_revocation_list(&h.data_dir, &release, 5, &[(&acme.b64, "later news")]);
    assert_eq!(h.plugin("acme_one").await["state"], "revoked");
    assert_eq!(h.revocations().await["applied_sequence"], 5);

    // An older (validly signed) list can't undo anything: sequence only goes up.
    std::fs::remove_file(h.data_dir.join("revoked_keys.json")).unwrap();
    std::fs::remove_file(h.data_dir.join("revoked_keys.json.minisig")).unwrap();
    write_revocation_list(&h.web_dir, &release, 3, &[]);
    assert_eq!(h.plugin("acme_one").await["state"], "revoked");
    assert_eq!(h.revocations().await["applied_sequence"], 5);
    assert!(h.db.get_publisher(&acme.b64).unwrap().unwrap().revoked);

    // The release key can't be revoked by hand either.
    let id = os_next::core::plugin_trust::key_id(&release.b64);
    h.install(signed_zip("rel_one", "1.0.0", &release, "<html>rel</html>", &[]), false).await;
    assert_eq!(h.post(&format!("/api/plugins/publishers/{id}/revoke")).await.status(), reqwest::StatusCode::CONFLICT);
    // Console-only.
    assert_eq!(h.c.get(format!("{}/api/plugins/revocations", h.base)).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_release_signed_plugins_are_the_apps_own_and_names_cannot_be_forged() {
    let (release, acme) = (test_key("release"), test_key("acme"));
    let h = Harness::start(&release).await;
    // A bundled plugin signed by the release key, and one that is bundled but unsigned (a dev build).
    for (id, key) in [("bundled_signed", Some(&release)), ("bundled_dev", None)] {
        let dir = h.web_dir.join("plugins").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<html></html>").unwrap();
        std::fs::write(dir.join("plugin.json"), serde_json::json!({
            "manifest_version": 1, "id": id, "name": id, "version": "1.0.0", "publisher": "OpenSanctuary",
            "web": {"entry": "index.html", "sandbox": true}, "permissions": [{"id": "ui.panel", "reason": "x"}]
        }).to_string()).unwrap();
        if let Some(k) = key {
            os_next::core::plugin_registry::write_file_list(&dir, Some(&k.b64)).unwrap();
            let bytes = std::fs::read(dir.join("plugin.json")).unwrap();
            std::fs::write(dir.join("plugin.json.minisig"), sign_bytes(k, &bytes)).unwrap();
        }
    }
    let (signed, dev) = (h.plugin("bundled_signed").await, h.plugin("bundled_dev").await);
    assert_eq!((signed["trust"]["level"].as_str(), signed["trust"]["release_signed"].as_bool(), signed["signature_scope"].as_str()), (Some("builtin"), Some(true), Some("full")), "{signed}");
    assert_eq!((dev["trust"]["level"].as_str(), dev["trust"]["release_signed"].as_bool()), (Some("builtin"), Some(false)));
    assert_eq!(signed["trust"]["publisher_name"], "OpenSanctuary");

    // An installed plugin signed by the release key is the app's own: trusted, name vouched for, nothing to choose.
    assert_eq!(h.install(signed_zip("official_extra", "1.0.0", &release, "<html>x</html>", &[]), false).await.status(), reqwest::StatusCode::OK);
    let official = h.plugin("official_extra").await;
    assert_eq!((official["trust"]["level"].as_str(), official["trust"]["name_is_claim"].as_bool(), official["trust"]["can_trust"].as_bool(), official["trust"]["publisher_name"].as_str()), (Some("trusted"), Some(false), Some(false), Some("OpenSanctuary")), "{official}");

    // Anyone else who writes "OpenSanctuary" in their manifest is only claiming it.
    let mut fake = signed_zip("fake_official", "1.0.0", &acme, "<html>x</html>", &[]);
    let _ = &mut fake;
    h.install(fake, false).await;
    let fake = h.plugin("fake_official").await;
    assert_eq!((fake["trust"]["level"].as_str(), fake["trust"]["name_is_claim"].as_bool(), fake["trust"]["release_signed"].as_bool()), (Some("new"), Some(true), Some(false)));
}

#[tokio::test]
async fn test_key_rotation_needs_the_old_keys_signature() {
    let (release, acme, rival) = (test_key("release"), test_key("acme"), test_key("rival"));
    let statement = |old: &TestKey, new: &TestKey| serde_json::json!({ "old_key": old.b64, "new_key": new.b64 }).to_string();
    let rotation = |old: &TestKey, new: &TestKey, signed_by: &TestKey| {
        let st = statement(old, new);
        let sig = sign_bytes(signed_by, st.as_bytes());
        (st, sig)
    };

    let h = Harness::start(&release).await;
    // v1 is signed by `acme`, approved, and `acme` is trusted.
    assert_eq!(h.install(signed_zip("moving", "1.0.0", &acme, "<html>v1</html>", &[]), false).await.status(), reqwest::StatusCode::OK);
    assert_eq!(h.approve("moving", serde_json::json!({ "trust_publisher": true })).await, reqwest::StatusCode::OK);

    // v2 is signed by a different key with no statement: a different publisher, loudly.
    h.install(signed_zip("moving", "2.0.0", &rival, "<html>v2</html>", &[]), true).await;
    let p = h.plugin("moving").await;
    assert_eq!((p["signer_changed"].as_bool(), p["trust"]["level"].as_str(), p["trust"]["rotated_from_key_id"].is_null()), (Some(true), Some("new"), true), "{p}");

    // A statement signed by the NEW key itself proves nothing (anyone can say "I am the old publisher").
    let (st, sig) = rotation(&acme, &rival, &rival);
    h.install(signed_zip("moving", "2.0.0", &rival, "<html>v2</html>", &[("plugin.rotation.json", st.as_bytes()), ("plugin.rotation.json.minisig", sig.as_bytes())]), true).await;
    let p = h.plugin("moving").await;
    assert_eq!((p["signer_changed"].as_bool(), p["trust"]["level"].as_str()), (Some(true), Some("new")), "forged: {p}");

    // A statement for some other new key doesn't apply either.
    let (st, sig) = rotation(&acme, &release, &acme);
    h.install(signed_zip("moving", "2.0.0", &rival, "<html>v2</html>", &[("plugin.rotation.json", st.as_bytes()), ("plugin.rotation.json.minisig", sig.as_bytes())]), true).await;
    assert_eq!(h.plugin("moving").await["signer_changed"], true);

    // The real thing: the old key signed "new_key is me". No warning, the old key's standing carries over,
    // and approving moves the trust to the new key.
    let (st, sig) = rotation(&acme, &rival, &acme);
    let good = signed_zip("moving", "2.0.0", &rival, "<html>v2</html>", &[("plugin.rotation.json", st.as_bytes()), ("plugin.rotation.json.minisig", sig.as_bytes())]);
    assert_eq!(h.install(good.clone(), true).await.status(), reqwest::StatusCode::OK, "the statement files are allowed next to a fully signed manifest");
    let p = h.plugin("moving").await;
    assert_eq!((p["signer_changed"].as_bool(), p["trust"]["level"].as_str(), p["state"].as_str()), (Some(false), Some("trusted"), Some("changed")), "{p}");
    assert_eq!(p["trust"]["rotated_from_key_id"], os_next::core::plugin_trust::key_id(&acme.b64));
    assert_eq!(p["trust"]["can_trust"], true, "the new key isn't trusted in its own right yet");
    assert_eq!(h.approve("moving", serde_json::json!({})).await, reqwest::StatusCode::OK);
    assert!(h.db.get_publisher(&rival.b64).unwrap().is_some_and(|p| p.trusted), "trust moved to the new key");
    assert!(h.db.list_plugin_audit(100).unwrap().iter().any(|a| a.action == "key_rotated"));
    assert_eq!(h.plugin("moving").await["trust"]["can_trust"], false);

    // If the old key was revoked, a rotation from it is not honored.
    let h2 = Harness::start(&release).await;
    h2.install(signed_zip("moving", "1.0.0", &acme, "<html>v1</html>", &[]), false).await;
    assert_eq!(h2.approve("moving", serde_json::json!({})).await, reqwest::StatusCode::OK);
    write_revocation_list(&h2.web_dir, &release, 1, &[(&acme.b64, "leaked")]);
    h2.install(good, true).await;
    let p = h2.plugin("moving").await;
    assert_eq!((p["signer_changed"].as_bool(), p["trust"]["level"].as_str()), (Some(true), Some("new")), "{p}");
}

// ---------------------------------------------------------------------------------------------
// library.addMedia (a file from a plugin's own process job, as opposed to addMediaUrl's download):
// does the host trust the claimed extension, or check what the bytes actually are?
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn test_add_media_from_a_process_job_is_checked_against_its_actual_bytes_not_just_its_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("media_sniff.db").to_str().unwrap()).unwrap();
    let web_dir = tmp.path().join("web");
    let data_dir = tmp.path().join("data");
    let plugin_id = "fake_tool_plugin";
    let dir = web_dir.join("plugins").join(plugin_id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<html></html>").unwrap();
    std::fs::write(dir.join("plugin.json"), serde_json::json!({
        "manifest_version": 1, "id": plugin_id, "name": "Fake Tool", "web": {"entry": "index.html", "sandbox": true},
        "permissions": [{"id": "process.run", "commands": ["make"], "reason": "x"}, {"id": "library.write", "categories": ["media"], "reason": "x"}],
        "commands": [{"id": "make", "tool": "faketool", "args": ["{output_dir}", "{name}"], "params": {"name": {"type": "string"}}}],
        "tools": [{"id": "faketool", "github": "nobody/nothing", "assets": {"linux": "a", "windows": "a.exe", "macos": "a"}, "checksums": "SHA2-256SUMS", "signature": {"asset": "SHA2-256SUMS.sig", "key_file": "k.pub"}}]
    }).to_string()).unwrap();

    // The host's "managed" copy of the tool: a tiny real script. Whatever bytes it writes is exactly
    // what a compromised or dishonest plugin's own declared tool could write -- the host only ever
    // runs *this exact, operator-reviewed command line*, never anything else.
    let tools_dir = data_dir.join("tools");
    let tool_dir = tools_dir.join("plugins").join(plugin_id);
    std::fs::create_dir_all(&tool_dir).unwrap();
    let tool_path = tool_dir.join(format!("faketool{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&tool_path, "#!/bin/sh\nprintf '<html><script>alert(document.domain)</script></html>' > \"$1/$2\"\n").unwrap();
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&tool_path, std::fs::Permissions::from_mode(0o755)).unwrap(); }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        web_dir: web_dir.clone(), media_dir: tmp.path().join("media"), data_dir: data_dir.clone(),
        host_session_token: "sniff_host".to_string(), server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let h = "sniff_host";

    let list: serde_json::Value = c.get(format!("{base}/api/plugins")).header("x-host-token", h).send().await.unwrap().json().await.unwrap();
    let p = list.as_array().unwrap().iter().find(|p| p["id"] == plugin_id).unwrap().clone();
    assert_eq!(p["error"], serde_json::Value::Null, "{p}");
    let a = c.post(format!("{base}/api/plugins/{plugin_id}/approve")).header("x-host-token", h)
        .json(&serde_json::json!({ "digest": p["digest"], "granted": ["process.run", "library.write"] })).send().await.unwrap();
    assert_eq!(a.status(), reqwest::StatusCode::OK, "{}", a.text().await.unwrap());

    let token: serde_json::Value = c.post(format!("{base}/api/plugins/{plugin_id}/token")).header("x-host-token", h)
        .json(&serde_json::json!({ "scopes": [{"id": "process.run", "commands": ["make"]}, {"id": "library.write", "categories": ["media"]}] }))
        .send().await.unwrap().json().await.unwrap();
    let pt = token["token"].as_str().unwrap_or_else(|| panic!("{token}"));

    // Run the "tool": it writes HTML+script to a file it (truthfully, as far as the host checks) calls "evil.mp4".
    let started: serde_json::Value = c.post(format!("{base}/api/plugin-host/process/start")).header("x-plugin-token", pt)
        .json(&serde_json::json!({ "command": "make", "params": { "name": "evil.mp4" } })).send().await.unwrap().json().await.unwrap();
    assert_eq!(started["success"], true, "{started}");
    let job = started["job"].as_str().unwrap().to_string();
    let mut tries = 0;
    loop {
        let poll: serde_json::Value = c.post(format!("{base}/api/plugin-host/process/poll")).header("x-plugin-token", pt).json(&serde_json::json!({ "job": job })).send().await.unwrap().json().await.unwrap();
        if poll["status"] != "running" { assert_eq!(poll["status"], "exited", "{poll}"); break; }
        tries += 1;
        assert!(tries < 100, "job never finished");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Sanity: the file really is HTML, not a video, sitting right where addMedia will look for it.
    let job_dir = data_dir.join("plugin-jobs").join(plugin_id).join(&job);
    let written = std::fs::read_to_string(job_dir.join("evil.mp4")).unwrap();
    assert!(written.contains("<script>"), "{written}");

    let add = c.post(format!("{base}/api/plugin-host/media")).header("x-plugin-token", pt)
        .json(&serde_json::json!({ "job": job, "file": "evil.mp4", "media_type": "video", "title": "Evil" }))
        .send().await.unwrap();
    let status = add.status();
    let body: serde_json::Value = add.json().await.unwrap_or(serde_json::json!({}));
    // The real content must be checked, exactly as library.addMediaUrl already does with sniff_media_ext:
    // an extension on a file name is not evidence of what the file contains.
    assert_ne!(status, reqwest::StatusCode::OK, "HTML masquerading as a .mp4 (right extension, wrong bytes) must be refused: {body}");
    assert!(body["error"].as_str().unwrap_or("").contains("evil.mp4"), "{body}");
    assert!(!tmp.path().join("media/videos/evil.mp4").exists(), "the impostor file must not have been moved into the library");

    // A genuine video, named honestly, still goes through -- this isn't a ban on the extension, it's a
    // check that the bytes back it up.
    std::fs::write(job_dir.join("real.webm"), [0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0, b'x', b'x', b'x', b'x']).unwrap();
    let add2 = c.post(format!("{base}/api/plugin-host/media")).header("x-plugin-token", pt)
        .json(&serde_json::json!({ "job": job, "file": "real.webm", "media_type": "video", "title": "Real" }))
        .send().await.unwrap();
    assert_eq!(add2.status(), reqwest::StatusCode::OK);
    let body2: serde_json::Value = add2.json().await.unwrap();
    assert_eq!(body2["item"]["file_path"], "/media/videos/real.webm");
    assert!(tmp.path().join("media/videos/real.webm").exists());

    // The same for audio: a 4-byte MPEG sync pattern in front of HTML is not a stream (nothing real
    // follows it), but a genuine one -- here, an actual encoder's output, not hand-built bytes -- passes.
    std::fs::write(job_dir.join("evil.mp3"), [0xFF, 0xFB, 0x90, 0x00].iter().chain(b"<html><script>alert(1)</script>").copied().collect::<Vec<u8>>()).unwrap();
    let bad_audio = c.post(format!("{base}/api/plugin-host/media")).header("x-plugin-token", pt)
        .json(&serde_json::json!({ "job": job, "file": "evil.mp3", "media_type": "audio", "title": "Evil" }))
        .send().await.unwrap();
    assert_ne!(bad_audio.status(), reqwest::StatusCode::OK, "a sync pattern in front of HTML, with nothing real following, must be refused");
    assert!(!tmp.path().join("media/audio/evil.mp3").exists());

    let real_mp3 = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/media/bare_real.mp3")).unwrap();
    std::fs::write(job_dir.join("real.mp3"), &real_mp3).unwrap();
    let add3 = c.post(format!("{base}/api/plugin-host/media")).header("x-plugin-token", pt)
        .json(&serde_json::json!({ "job": job, "file": "real.mp3", "media_type": "audio", "title": "Real Audio" }))
        .send().await.unwrap();
    assert_eq!(add3.status(), reqwest::StatusCode::OK, "{}", add3.text().await.unwrap());
    assert!(tmp.path().join("media/audio/real.mp3").exists());
}
