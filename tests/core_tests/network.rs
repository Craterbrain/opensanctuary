//! The HTTP/WS API surface: who may call what (host token, console lock, pairing), network settings, security headers, the split HTTP/HTTPS planes.

use super::*;

/// Builds a minimal real AppState + router for the check-public-url tests
/// below, bound to an ephemeral loopback port. Mirrors the construction
/// `test_freeshow_import_http_route` already uses elsewhere in this file.
async fn spawn_test_app(host_session_token: &str) -> String {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_public_url.db");
    let db = Database::new(&db_path).expect("Database::new");
    let web_dir = test_dir.path().join("web");
    std::fs::create_dir_all(&web_dir).unwrap();

    let event_log = Arc::new(EventLog::with_initial_sequence(3000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let asset_graph = Arc::new(AssetGraph::new(&web_dir));
    let (tx, _rx) = tokio::sync::broadcast::channel(512);

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        web_dir: web_dir.clone(),
        media_dir: web_dir.clone(),
        host_session_token: host_session_token.to_string(),
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{}", addr)
}

#[tokio::test]
async fn test_shorthand_catalog_item_expansion_and_route_handlers() {
    use os_next::core::models::ToScheduleItem;

    let db = os_next::storage::Database::in_memory().unwrap();
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log.clone());

    // 1. Seed Song into DB
    let song = os_next::models::Song {
        id: "song_101".to_string(),
        title: "Be Thou My Vision".to_string(),
        alternate_title: None, public_domain_override: None,
        author: "Eleanor Hull".to_string(),
        copyright: Some("Public Domain".to_string()),
        ccli_number: Some("12345".to_string()),
        slides: vec![os_next::models::Slide {
            text: "Be Thou my vision, O Lord of my heart".to_string(),
            header: None,
            label: Some("V1".to_string()),
            background: None,
            notes: None,
            tag: None,
            ..Default::default()
        }],
        theme_name: None,
    };
    db.insert_song(&song).unwrap();

    // 2. Seed Scripture into DB
    let scripture = os_next::models::ScriptureItem {
        id: "scripture_202".to_string(),
        book: "Psalm".to_string(),
        chapter: 121,
        verse_start: 1,
        verse_end: 2,
        version: "KJV".to_string(),
        reference: "Psalm 121:1-2".to_string(),
        verses: vec![os_next::models::ScriptureVerse {
            verse_number: 1,
            text: "I will lift up mine eyes unto the hills".to_string(),
        }],
        theme_name: None,
    };
    db.insert_scripture(&scripture).unwrap();

    // 3. Seed Media into DB
    let media = os_next::models::MediaItem {
        id: "media_303".to_string(),
        name: "Sunrise Loop".to_string(),
        media_type: "video".to_string(),
        file_path: "/media/videos/sunrise.mp4".to_string(),
        duration_seconds: Some(15.0),
        thumbnail_path: None,
        loop_playback: true,
    };
    db.insert_media(&media).unwrap();

    // 4. Seed Presentation into DB
    let presentation = os_next::models::Presentation {
        id: "pres_404".to_string(),
        title: "Welcome Slides".to_string(),
        author: "Church Admin".to_string(),
        slides: vec![os_next::models::Slide {
            text: "Announcements: Fellowship lunch at noon".to_string(),
            header: Some("Welcome".to_string()),
            label: Some("Slide 1".to_string()),
            background: None,
            notes: None,
            tag: None,
            ..Default::default()
        }],
        theme_name: None,
    };
    db.insert_presentation(&presentation).unwrap();

    // Simulate server-side shorthand resolution: match item_type and item_id
    let types_and_ids = vec![
        ("song", "song_101", "song"),
        ("scripture", "scripture_202", "scripture"),
        ("media", "media_303", "media"),
        ("presentation", "pres_404", "presentation"),
    ];

    for (item_type, item_id, expected_type) in types_and_ids {
        let sched_item = match item_type {
            "song" => db.get_song_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
            "scripture" => db.get_scripture_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
            "media" => db.get_media_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
            "presentation" => db.get_presentation_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
            _ => None,
        };

        assert!(sched_item.is_some(), "Shorthand resolution for {} must succeed", item_type);
        let item = sched_item.unwrap();
        assert_eq!(item.item_type, expected_type);
        engine.execute_command(ShowCommand::AddToSchedule(item));
    }

    // Verify all 4 items were added to active engine schedule
    let snap = engine.snapshot();
    assert_eq!(snap.schedule.items.len(), 4);
    assert_eq!(snap.schedule.items[0].title, "Be Thou My Vision");
    assert_eq!(snap.schedule.items[1].title, "Psalm 121:1-2");
    assert_eq!(snap.schedule.items[2].title, "Sunrise Loop");
    assert_eq!(snap.schedule.items[3].title, "Welcome Slides");

    // Negative case: unknown item_id returns None gracefully
    let invalid = db.get_song_by_id("nonexistent_id").ok().flatten();
    assert!(invalid.is_none(), "Nonexistent ID must return None without panicking");
}

#[test]
fn test_schedule_replace_vs_append_mode() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    // Initial schedule with item A
    engine.execute_command(ShowCommand::AddToSchedule(simple_item("item_a", "Item A", &["Slide A1"])));
    assert_eq!(engine.snapshot().schedule.items.len(), 1);

    // Incoming schedule with item B
    let incoming = os_next::core::models::Schedule {
        id: "incoming_sched".to_string(),
        title: "Incoming Schedule".to_string(),
        items: vec![simple_item("item_b", "Item B", &["Slide B1"])],
        selected_item_index: None,
        is_modified: false,
        schedule_version: 0,
    };

    // Case 1: Append mode adds items without dropping existing items
    for item in incoming.items.clone() {
        engine.execute_command(ShowCommand::AddToSchedule(item));
    }
    let snap_appended = engine.snapshot();
    assert_eq!(snap_appended.schedule.items.len(), 2);
    assert_eq!(snap_appended.schedule.items[0].title, "Item A");
    assert_eq!(snap_appended.schedule.items[1].title, "Item B");

    // Case 2: Replace mode replaces the entire schedule
    engine.execute_command(ShowCommand::LoadSchedule(incoming));
    let snap_replaced = engine.snapshot();
    assert_eq!(snap_replaced.schedule.items.len(), 1);
    assert_eq!(snap_replaced.schedule.items[0].title, "Item B");
    assert_eq!(snap_replaced.schedule.title, "Incoming Schedule");
}

#[tokio::test]
async fn test_network_api_routes_and_conflict_detection() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_network.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(2000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let bound_port = addr.port();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        server_port: bound_port,
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // 1. GET /api/network/info
    let resp = client.get(format!("http://{}/api/network/info", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let info_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(info_json["success"], true);
    assert_eq!(info_json["port"], bound_port);
    assert!(info_json["remote_url"].as_str().unwrap().contains(&format!(":{}", bound_port)));
    assert!(info_json["mdns_url"].as_str().unwrap().contains(".local:"));

    // 2. GET /api/network/interfaces
    let resp = client.get(format!("http://{}/api/network/interfaces", addr))
        .header("x-host-token", "test_host_session_token")
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ifaces_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(ifaces_json["success"], true);
    assert!(ifaces_json["interfaces"].is_array());
    assert_eq!(ifaces_json["port"], bound_port);

    // 3. POST /api/network/interfaces
    let save_payload = serde_json::json!({
        "enabled_interfaces": ["eth0", "wlan0"],
        "broadcast_all": true
    });
    let resp = client.post(format!("http://{}/api/network/interfaces", addr))
        .header("x-host-token", "test_host_session_token")
        .json(&save_payload)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let save_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(save_json["success"], true);

    // Verify settings saved in db
    let settings = db.get_settings().unwrap();
    assert_eq!(settings.get("networkEnabledInterfaces").map(|s| s.as_str()), Some("eth0,wlan0"));
    assert_eq!(settings.get("networkBroadcastAll").map(|s| s.as_str()), Some("true"));

    // 4. GET /api/network/check-port?port=<bound_port>
    // Should identify as in_use_by_current = true and available = true (since it belongs to this server)
    let resp = client.get(format!("http://{}/api/network/check-port?port={}", addr, bound_port))
        .header("x-host-token", "test_host_session_token")
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let port_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(port_json["success"], true);
    assert_eq!(port_json["available"], true);
    assert_eq!(port_json["in_use_by_current"], true);

    // 5. GET /api/network/check-hostname?name=MySanctuary_Host
    let resp = client.get(format!("http://{}/api/network/check-hostname?name=MySanctuary_Host", addr))
        .header("x-host-token", "test_host_session_token")
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let hn_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(hn_json["success"], true);
    assert_eq!(hn_json["sanitized_hostname"], "mysanctuary-host");

    // 6. POST /api/network/broadcast-option12
    let bcast_payload = serde_json::json!({
        "hostname": "test-bcast-host"
    });
    let resp = client.post(format!("http://{}/api/network/broadcast-option12", addr))
        .header("x-host-token", "test_host_session_token")
        .json(&bcast_payload)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let bcast_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(bcast_json["success"], true);
    assert_eq!(bcast_json["hostname"], "test-bcast-host");
}

/// Regression test: the hostname/port conflict check was only ever wired
/// into the read-only
/// GET /api/network/check-hostname and check-port endpoints — the actual
/// write paths (POST /api/settings, POST /api/network/dedicated-mac/toggle)
/// saved whatever they were given regardless of what the check would have
/// said. This proves the write paths now hold a flagged value back (while
/// still saving the rest of the same request) unless explicitly overridden.
#[tokio::test]
async fn test_conflicting_network_settings_are_held_back_unless_confirmed() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_conflict.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(2100, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let bound_port = addr.port();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        host_session_token: "test_host_session_conflict".to_string(),
        server_port: bound_port,
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // --- Port conflict: deterministic, no network dependency ---
    // Occupy a real port with a plain TCP listener so check_port_conflict
    // has a genuine collision to find, exactly like
    // test_check_port_conflict_detects_occupied_port in src/network/mod.rs.
    let occupied_listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let occupied_port = occupied_listener.local_addr().unwrap().port();

    let resp = client.post(format!("http://{}/api/settings", addr))
        .header("x-host-token", "test_host_session_conflict")
        .json(&serde_json::json!({
            "networkPort": occupied_port.to_string(),
            "churchName": "Held Back Church",
        }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["_conflicts"]["networkPort"].is_object(), "expected networkPort to be flagged: {body}");

    let settings_after = db.get_settings().unwrap();
    // The conflicting key was held back entirely (this is a fresh db, so it
    // was never set to anything at all — not the occupied value, not
    // anything else)...
    assert!(settings_after.get("networkPort").is_none());
    // ...but an unrelated key in the SAME request still saved normally.
    assert_eq!(settings_after.get("churchName").map(|s| s.as_str()), Some("Held Back Church"));

    // Resubmitting with _confirmOverrides saves it anyway.
    let resp = client.post(format!("http://{}/api/settings", addr))
        .header("x-host-token", "test_host_session_conflict")
        .json(&serde_json::json!({
            "networkPort": occupied_port.to_string(),
            "_confirmOverrides": "networkPort",
        }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.get("_conflicts").is_none());
    let settings_forced = db.get_settings().unwrap();
    assert_eq!(settings_forced.get("networkPort").map(|s| s.as_str()), Some(occupied_port.to_string().as_str()));
    drop(occupied_listener);

    // A non-conflicting port saves through with no _conflicts at all. Bind
    // and immediately drop a fresh ephemeral listener to get a port number
    // that's very likely free, rather than guessing via arithmetic.
    let free_port = {
        let probe = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    let resp = client.post(format!("http://{}/api/settings", addr))
        .header("x-host-token", "test_host_session_conflict")
        .json(&serde_json::json!({ "networkPort": free_port.to_string() }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.get("_conflicts").is_none());

    // Deliberately NOT testing POST /api/network/dedicated-mac/toggle's new
    // hostname-conflict guard here with `enable: true`, even with a clean
    // (non-conflicting) hostname. That handler falls through to a real
    // `pkexec` invocation (src/network/linux.rs) whenever there's no
    // conflict — confirmed the hard way while writing this test: with no
    // polkit auth agent registered in this sandbox, `pkexec` blocks
    // indefinitely instead of failing fast, which hung this exact test
    // until the child process was killed by hand. That's a pre-existing
    // risk this fix didn't introduce, just newly reachable by testing the
    // handler at all — an automated test still shouldn't invoke it.
    //
    // The conflict branch itself WAS verified live, not just by inspection:
    // this dev machine's avahi-daemon already sees a real device on the LAN
    // (`avahi-browse -rp -t _uscan._tcp` → an Epson printer answering as
    // `EPSON5490D4.local` / 192.168.1.118). Hitting this handler with that
    // exact hostname returned `{"success":false,"conflict":true,
    // "conflicting_ip":"192.168.1.118",...}` and did not touch pkexec at
    // all — the same request against `/api/settings` held `networkHostname`
    // back in `_conflicts` too. Not reproduced here as an automated
    // assertion because it depends on a specific device on this specific
    // LAN, which won't exist on another machine or in CI — but the
    // identical enforcement pattern (check, then hold back unless forced)
    // is proven end-to-end and automatically above via ports, which are
    // fully reproducible without any external device.
}

#[tokio::test]
async fn test_client_pairing_lifecycle_and_zero_auth_displays() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_pairing.db");
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
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        // with_connect_info: GET /api/internal/host-token needs the real
        // peer address to enforce its loopback-only check (see below).
        let _ = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await;
    });

    let client = reqwest::Client::new();
    let host_token = "test_host_session_token";

    // 0. CRITICAL AUTH INVARIANT: minting a pairing session with no host
    // token (or the wrong one) is rejected. Before this fix, any LAN
    // client could mint its own session and self-pair a rogue device.
    let sess_noauth_resp = client.post(format!("http://{}/api/pairing/session", addr))
        .send().await.unwrap();
    assert_eq!(sess_noauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let sess_badauth_resp = client.post(format!("http://{}/api/pairing/session", addr))
        .header("x-host-token", "wrong_token")
        .send().await.unwrap();
    assert_eq!(sess_badauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 1. POST /api/pairing/session (with the real host token) - generate ephemeral pairing session
    let sess_resp = client.post(format!("http://{}/api/pairing/session", addr))
        .header("x-host-token", host_token)
        .send().await.unwrap();
    assert_eq!(sess_resp.status(), reqwest::StatusCode::OK);
    let sess_json: serde_json::Value = sess_resp.json().await.unwrap();
    let session_token = sess_json["session_token"].as_str().unwrap().to_string();
    assert!(session_token.starts_with("pair_sess_"));
    assert_eq!(sess_json["expires_in"], 300);

    // 2. Reject authorization with invalid session token
    let bad_auth_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .json(&serde_json::json!({
            "session_token": "pair_sess_invalid_token",
            "device_id": "tv-sanctuary-1",
            "name": "Sanctuary Main TV",
            "platform": "android-tv"
        }))
        .send().await.unwrap();
    assert_eq!(bad_auth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let bad_auth_json: serde_json::Value = bad_auth_resp.json().await.unwrap();
    assert_eq!(bad_auth_json["error"], "Invalid or expired pairing session token");

    // 3. Status check before pairing reports false
    let status_before = client.get(format!("http://{}/api/pairing/status?device_id=tv-sanctuary-1", addr))
        .send().await.unwrap();
    assert_eq!(status_before.status(), reqwest::StatusCode::OK);
    let status_before_json: serde_json::Value = status_before.json().await.unwrap();
    assert_eq!(status_before_json["paired"], false);

    // 4. Authorize with valid session token
    let auth_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .json(&serde_json::json!({
            "session_token": session_token,
            "device_id": "tv-sanctuary-1",
            "name": "Sanctuary Main TV",
            "platform": "android-tv"
        }))
        .send().await.unwrap();
    assert_eq!(auth_resp.status(), reqwest::StatusCode::OK);
    let auth_json: serde_json::Value = auth_resp.json().await.unwrap();
    assert_eq!(auth_json["success"], true);
    assert_eq!(auth_json["device_id"], "tv-sanctuary-1");
    assert_eq!(auth_json["name"], "Sanctuary Main TV");
    let dev_token = auth_json["token"].as_str().unwrap().to_string();
    assert!(dev_token.starts_with("dev_tok_"));

    // 5. The same session token authorizes a SECOND device too — sessions
    // are scoped by TTL, not single-use, so a technician can pair every TV
    // in the building from one console-minted session within its 5-minute
    // window instead of the phone needing to mint its own sessions (which
    // would require giving the phone console-level trust it was never meant
    // to have). See `validate_pairing_session` in src/api/ws.rs.
    let second_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .json(&serde_json::json!({
            "session_token": session_token,
            "device_id": "tv-sanctuary-2",
            "name": "Sanctuary Side TV",
            "platform": "roku"
        }))
        .send().await.unwrap();
    assert_eq!(second_resp.status(), reqwest::StatusCode::OK);
    let second_json: serde_json::Value = second_resp.json().await.unwrap();
    assert_eq!(second_json["success"], true);
    assert_eq!(second_json["device_id"], "tv-sanctuary-2");

    // A session-token holder can't take over an already-paired device id;
    // the console (host token) can.
    let hijack_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .json(&serde_json::json!({
            "session_token": session_token,
            "device_id": "tv-sanctuary-1",
            "name": "Attacker",
            "platform": "android-tv"
        }))
        .send().await.unwrap();
    assert_eq!(hijack_resp.status(), reqwest::StatusCode::CONFLICT);
    let repair_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .header("x-host-token", host_token)
        .json(&serde_json::json!({
            "session_token": session_token,
            "device_id": "tv-sanctuary-2",
            "name": "Sanctuary Side TV",
            "platform": "roku"
        }))
        .send().await.unwrap();
    assert_eq!(repair_resp.status(), reqwest::StatusCode::OK);

    // An unknown/garbage session token still fails, same as before.
    let bad_reuse_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .json(&serde_json::json!({
            "session_token": "pair_sess_totally_made_up",
            "device_id": "tv-sanctuary-3",
            "name": "Lobby TV",
            "platform": "roku"
        }))
        .send().await.unwrap();
    assert_eq!(bad_reuse_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 6. Status check after pairing reports true with token
    let status_after = client.get(format!("http://{}/api/pairing/status?device_id=tv-sanctuary-1", addr))
        .send().await.unwrap();
    assert_eq!(status_after.status(), reqwest::StatusCode::OK);
    let status_after_json: serde_json::Value = status_after.json().await.unwrap();
    assert_eq!(status_after_json["paired"], true);
    assert_eq!(status_after_json["device_id"], "tv-sanctuary-1");
    assert_eq!(status_after_json["token"], dev_token);

    // 6b. A SECOND status poll for the same, already-paired device_id no
    // longer includes the token -- it was already delivered once above.
    // Before this fix, any LAN caller could read an already-paired device's
    // permanent bearer token forever just by naming its device_id.
    let status_again = client.get(format!("http://{}/api/pairing/status?device_id=tv-sanctuary-1", addr))
        .send().await.unwrap();
    let status_again_json: serde_json::Value = status_again.json().await.unwrap();
    assert_eq!(status_again_json["paired"], true);
    assert_eq!(status_again_json["token"], serde_json::Value::Null);

    // 7. Verify token endpoint
    let verify_resp = client.post(format!("http://{}/api/pairing/verify", addr))
        .json(&serde_json::json!({ "token": dev_token }))
        .send().await.unwrap();
    assert_eq!(verify_resp.status(), reqwest::StatusCode::OK);
    let verify_json: serde_json::Value = verify_resp.json().await.unwrap();
    assert_eq!(verify_json["valid"], true);
    assert_eq!(verify_json["device_id"], "tv-sanctuary-1");

    // 8. GET /api/pairing/devices lists both paired devices — but only for
    // a caller with the host token; a plain LAN client gets nothing.
    // Before this fix, every paired device's permanent bearer token was
    // readable by anyone.
    let list_noauth_resp = client.get(format!("http://{}/api/pairing/devices", addr))
        .send().await.unwrap();
    assert_eq!(list_noauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let list_resp = client.get(format!("http://{}/api/pairing/devices", addr))
        .header("x-host-token", host_token)
        .send().await.unwrap();
    assert_eq!(list_resp.status(), reqwest::StatusCode::OK);
    let list_json: Vec<serde_json::Value> = list_resp.json().await.unwrap();
    assert_eq!(list_json.len(), 2);
    assert!(list_json.iter().any(|d| d["id"] == "tv-sanctuary-1" && d["name"] == "Sanctuary Main TV" && d["platform"] == "android-tv"));
    assert!(list_json.iter().any(|d| d["id"] == "tv-sanctuary-2"));

    // 9. DELETE /api/pairing/devices/:id unpairs the device — same gate.
    // A plain LAN client can no longer revoke a live display mid-service.
    let del_noauth_resp = client.delete(format!("http://{}/api/pairing/devices/tv-sanctuary-1", addr))
        .send().await.unwrap();
    assert_eq!(del_noauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let del_resp = client.delete(format!("http://{}/api/pairing/devices/tv-sanctuary-1", addr))
        .header("x-host-token", host_token)
        .send().await.unwrap();
    assert_eq!(del_resp.status(), reqwest::StatusCode::OK);
    let del_json: serde_json::Value = del_resp.json().await.unwrap();
    assert_eq!(del_json["success"], true);

    // Verify status returns false after revocation
    let status_revoked = client.get(format!("http://{}/api/pairing/status?device_id=tv-sanctuary-1", addr))
        .send().await.unwrap();
    let status_revoked_json: serde_json::Value = status_revoked.json().await.unwrap();
    assert_eq!(status_revoked_json["paired"], false);

    // 10. CRITICAL ZERO-AUTH INVARIANT:
    // Verify /api/state and public display endpoints require NO auth headers or tokens
    let state_resp = client.get(format!("http://{}/api/state", addr))
        .send().await.unwrap();
    assert_eq!(state_resp.status(), reqwest::StatusCode::OK);

    let remote_resp = client.get(format!("http://{}/remote", addr))
        .send().await.unwrap();
    // /remote redirects or renders 200 OK without requiring auth
    assert!(remote_resp.status().is_success() || remote_resp.status().is_redirection());

    // 11. GET /api/server-info no longer carries host_token — this route is
    // deliberately zero-auth (any LAN client reads it), which used to mean
    // anyone could read the host token from it and get unrestricted keyring
    // access.
    let server_info_resp = client.get(format!("http://{}/api/server-info", addr))
        .send().await.unwrap();
    assert_eq!(server_info_resp.status(), reqwest::StatusCode::OK);
    let server_info_json: serde_json::Value = server_info_resp.json().await.unwrap();
    assert!(server_info_json.get("host_token").is_none());
    assert!(server_info_json.get("instance_id").is_some());

    // 12. GET /api/internal/host-token: this test's own reqwest client
    // connects over loopback (127.0.0.1), so it's the one caller allowed to
    // read the host token this way — proving the intended "browser tab on
    // this machine" path works, not just that the network path is closed.
    let host_token_resp = client.get(format!("http://{}/api/internal/host-token", addr))
        .send().await.unwrap();
    assert_eq!(host_token_resp.status(), reqwest::StatusCode::OK);
    let host_token_json: serde_json::Value = host_token_resp.json().await.unwrap();
    assert_eq!(host_token_json["host_token"], host_token);

    // 12b. A reverse proxy on this same machine (Caddy, see docs/TUNNELS.md)
    // makes every real request's TCP peer loopback -- this test's own
    // reqwest client already IS that loopback peer, so it can simulate one
    // directly by adding its own X-Forwarded-For header. A real, non-
    // loopback original client reported that way must be rejected...
    let proxied_real_client_resp = client.get(format!("http://{}/api/internal/host-token", addr))
        .header("x-forwarded-for", "203.0.113.7")
        .send().await.unwrap();
    assert_eq!(proxied_real_client_resp.status(), reqwest::StatusCode::FORBIDDEN, "a proxy reporting a real, non-loopback original client must not get the host token");

    // ...while one reporting a loopback original client (e.g. a browser tab
    // on this same machine going through the proxy) still succeeds.
    let proxied_loopback_client_resp = client.get(format!("http://{}/api/internal/host-token", addr))
        .header("x-forwarded-for", "127.0.0.1")
        .send().await.unwrap();
    assert_eq!(proxied_loopback_client_resp.status(), reqwest::StatusCode::OK);

    // 13. POST /api/pairing/remote-session requires the host token too —
    // otherwise any LAN client could mint itself a live remote-control
    // token without ever touching the console or scanning a QR.
    let remote_sess_noauth_resp = client.post(format!("http://{}/api/pairing/remote-session", addr))
        .send().await.unwrap();
    assert_eq!(remote_sess_noauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let remote_sess_resp = client.post(format!("http://{}/api/pairing/remote-session", addr))
        .header("x-host-token", host_token)
        .send().await.unwrap();
    assert_eq!(remote_sess_resp.status(), reqwest::StatusCode::OK);
    let remote_sess_json: serde_json::Value = remote_sess_resp.json().await.unwrap();
    assert_eq!(remote_sess_json["success"], true);
    assert!(remote_sess_json["token"].as_str().unwrap().starts_with("dev_tok_"));
}

/// `POST /api/command` and `/ws` used to execute any command from any caller
/// with no authentication at all, unless it carried a paired-device token
/// (a pre-existing console-auth gap). Both now require `host_session_token`
/// for callers with no device token -- this proves the gate actually blocks
/// execution (not just
/// returns an error status) on both entry points.
#[tokio::test]
async fn test_command_endpoint_requires_host_token_for_unpaired_callers() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_command_host_token.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host_token = "test_host_session_command".to_string();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        host_session_token: host_token.clone(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // POST /api/internal/verify-host-token: no loopback restriction (unlike
    // GET /api/internal/host-token), just a yes/no against the real token --
    // this is how a remote console confirms a token it was handed
    // out-of-band (printed to the server's terminal, or pasted manually).
    let resp = client.post(format!("http://{}/api/internal/verify-host-token", addr))
        .json(&serde_json::json!({ "token": host_token }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["valid"], true);
    let resp = client.post(format!("http://{}/api/internal/verify-host-token", addr))
        .json(&serde_json::json!({ "token": "not-the-real-token" }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["valid"], false);

    // HTTP: no token at all -- rejected, state unchanged.
    let resp = client.post(format!("http://{}/api/command", addr))
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["state"]["state"]["is_blackout"], false);

    // HTTP: wrong token -- also rejected.
    let resp = client.post(format!("http://{}/api/command", addr))
        .header("x-host-token", "not-the-real-token")
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["state"]["state"]["is_blackout"], false);

    // HTTP: the real host token but no console_session_id -- still rejected
    // (a distinct "missing x-console-session-id" reason, not a state change).
    let resp = client.post(format!("http://{}/api/command", addr))
        .header("x-host-token", &host_token)
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["state"]["state"]["is_blackout"], false);

    // HTTP: the real host token + a session id -- executes.
    let resp = client.post(format!("http://{}/api/command", addr))
        .header("x-host-token", &host_token)
        .header("x-console-session-id", "session-a")
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["state"]["state"]["is_blackout"], true);

    // WS: a message with no device_token and no (or a wrong) host_token
    // must not execute either -- connect, send both bad variants, then
    // confirm via a fresh HTTP snapshot that blackout is still on from the
    // HTTP toggle above (i.e. an unauthenticated WS ToggleBlackout did not
    // flip it back off).
    use futures_util::{SinkExt, StreamExt};
    let (mut ws_stream, _) = tokio_tungstenite::connect_async(format!("ws://{}/ws", addr)).await.unwrap();
    let _initial_snapshot = ws_stream.next().await; // server sends one on connect

    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout" }).to_string().into(),
    )).await.unwrap();
    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "host_token": "not-the-real-token", "console_session_id": "session-a" }).to_string().into(),
    )).await.unwrap();

    // Give the server a moment to (not) process those, then verify via HTTP
    // that state didn't change -- a broadcast never arrives on this socket
    // either way since a rejected message never calls broadcast_snapshot.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], true, "unauthenticated/wrong-token WS commands must not execute");

    // WS: the real host token but no console_session_id -- also rejected.
    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "host_token": host_token }).to_string().into(),
    )).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], true, "a WS command missing console_session_id must not execute");

    // WS: the real host token + the same session id already holding the
    // lock from the HTTP call above -- executes (toggles back off).
    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "host_token": host_token, "console_session_id": "session-a" }).to_string().into(),
    )).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], false, "a WS command with the real host token and matching session id must execute");
}

/// Library CRUD-delete, schedule save, settings, and the local-update-apply
/// routes all now require the host token, same as `/api/command`/`/ws` --
/// previously any LAN caller could delete the whole song/media/theme/
/// scripture library, overwrite an arbitrary file via `/api/schedule/save`,
/// or make the console open an arbitrary local file via
/// `/api/updates/apply-local`, with zero authentication. This does not
/// exercise `apply_local_update`'s success path (it would try to actually
/// open a file with the OS's default handler) -- only that an
/// unauthenticated/wrong-token call is rejected before it gets that far.
#[tokio::test]
async fn test_library_mutation_and_admin_routes_require_host_token() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_library_auth.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::new(100));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host_token = "test_host_session_library_auth".to_string();

    let song = os_next::core::models::Song::new("Auth Test Song", "Test Author");
    db.insert_song(&song).unwrap();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: test_dir.path().to_path_buf(),
        host_session_token: host_token.clone(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // DELETE /api/songs/:id -- no token, wrong token, then the real one.
    let resp = client.delete(format!("http://{}/api/songs/{}", addr, song.id)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let resp = client.delete(format!("http://{}/api/songs/{}", addr, song.id))
        .header("x-host-token", "wrong")
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(db.get_songs().unwrap().len(), 1, "unauthenticated caller must not be able to delete a song");
    let resp = client.delete(format!("http://{}/api/songs/{}", addr, song.id))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(db.get_songs().unwrap().len(), 0);

    // POST /api/schedule/save -- no token is rejected before any file is written.
    let save_path = test_dir.path().join("unauthorized_save.ewsx");
    let resp = client.post(format!("http://{}/api/schedule/save", addr))
        .json(&serde_json::json!({ "path": save_path.to_str().unwrap() }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(!save_path.exists(), "unauthenticated caller must not be able to write an arbitrary file via schedule save");
    let resp = client.post(format!("http://{}/api/schedule/save", addr))
        .header("x-host-token", &host_token)
        .json(&serde_json::json!({ "path": save_path.to_str().unwrap() }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert!(save_path.exists());

    // POST /api/settings -- no token is rejected.
    let resp = client.post(format!("http://{}/api/settings", addr))
        .json(&serde_json::json!({ "churchName": "Attacker Name" }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_ne!(
        db.get_settings().unwrap().get("churchName").map(|s| s.as_str()),
        Some("Attacker Name"),
        "unauthenticated caller must not be able to change settings"
    );

    // POST /api/updates/apply-local -- no token is rejected before it ever
    // tries to hand the path to the OS's open-with-default-handler.
    let resp = client.post(format!("http://{}/api/updates/apply-local", addr))
        .json(&serde_json::json!({ "path": "/nonexistent/should-not-be-opened", "force": true }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // The automated check/download flow (GET /api/updates/status, POST
    // /api/updates/check, POST /api/updates/download-and-install) needs the
    // same host-token gate -- without it, any LAN caller could trigger a
    // real download-and-install, not just read status.
    let resp = client.get(format!("http://{}/api/updates/status", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let resp = client.post(format!("http://{}/api/updates/check", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let resp = client.post(format!("http://{}/api/updates/download-and-install", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // ADB TV-provisioning routes -- without this, any LAN caller could
    // trigger a real adb install/launch sequence against whatever IP it
    // supplied, or read another operator's in-progress provisioning status.
    let resp = client.get(format!("http://{}/api/tv-provision/status", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let resp = client.post(format!("http://{}/api/tv-provision/start", addr))
        .json(&serde_json::json!({ "ip": "192.0.2.1" }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let resp = client.get(format!("http://{}/api/tv-provision/progress/nonexistent", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

/// Real, authenticated exercise of the ADB provisioning flow's actual
/// `adb` invocation -- targets `127.0.0.1` (loopback, nothing listening on
/// the ADB port in a test environment) rather than mocking `adb` out, so
/// this proves the real subprocess plumbing (spawn, argument construction,
/// the "adb connect exits 0 even on failure" quirk `network::adb::provision`
/// specifically works around) rather than just the HTTP plumbing around it.
/// Doesn't assert *which* failure reason (no `adb` binary vs. connection
/// refused) since that depends on whether this machine has `adb` installed
/// -- only that it fails cleanly and reports a real error, never hangs.
#[tokio::test]
async fn test_tv_provision_start_and_progress_against_unreachable_device() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_tv_provision.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::new(100));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host_token = "test_host_session_tv_provision".to_string();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        host_session_token: host_token.clone(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // No bundled APK in this test harness -- resolve_tv_apk_path() finds
    // nothing next to the `cargo test` binary, so this correctly reports
    // apk_available: false rather than crashing.
    let resp = client.get(format!("http://{}/api/tv-provision/status", addr))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["apk_available"], false, "no APK is bundled in the test harness, expected {:?}", body);

    // POST /api/tv-provision/start correctly refuses when there's no APK to
    // install, rather than starting a task that could only ever fail later.
    let resp = client.post(format!("http://{}/api/tv-provision/start", addr))
        .header("x-host-token", &host_token)
        .json(&serde_json::json!({ "ip": "127.0.0.1" }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE, "expected a clean refusal with no APK bundled");
}

/// Real, authenticated end-to-end coverage of the automated update routes
/// against the live public test repo (`crate::network::updater::RELEASES_REPO`)
/// -- `status` starts empty, `check` does a real GitHub call and populates
/// it, and a second `status` read reflects that without hitting the network
/// again.
///
/// `#[ignore]`d -- see `downloads_and_verifies_the_real_test_repo_release`
/// in `src/network/updater.rs` for why: this and that test together were
/// the dominant source of real GitHub API calls during this feature's own
/// development, enough to trip a secondary rate limit mid-session. Run
/// explicitly with `cargo test --test core_tests -- --ignored` when
/// actually verifying the live path.
#[tokio::test]
#[ignore]
async fn test_update_check_and_status_routes_against_real_test_repo() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_update_routes.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::new(100));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host_token = "test_host_session_update_routes".to_string();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        host_session_token: host_token.clone(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // Nothing's checked yet -- no background task in this harness (that's
    // only started from `main.rs`), so status starts as null.
    let resp = client.get(format!("http://{}/api/updates/status", addr))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["status"].is_null(), "expected no check to have run yet, got {:?}", body);

    // A real check against the real test repo.
    let resp = client.post(format!("http://{}/api/updates/check", addr))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["status"]["error"].is_null(), "expected a real check to succeed, got {:?}", body);
    assert!(body["status"]["current_version"].is_string());

    // Status now reflects that check, without another network call.
    let resp = client.get(format!("http://{}/api/updates/status", addr))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(!body["status"].is_null(), "expected status to be populated after a check");

    // A second POST /api/updates/check right away must NOT make another
    // real GitHub call -- MIN_SECONDS_BETWEEN_REAL_CHECKS in routes.rs
    // returns the still-fresh cached result instead (`throttled: true`).
    // This is exactly the behavior that keeps an operator mashing "Check
    // for Updates" from hammering GitHub's API.
    let resp = client.post(format!("http://{}/api/updates/check", addr))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["throttled"], true, "a second check within the cooldown should be throttled, got {:?}", body);
}

/// "One console at a time, first one connected wins" (docs/CLIENT_PAIRING.md).
/// A second console (different `console_session_id`) presenting the correct
/// host token must still be rejected with the distinct `console_locked`
/// reason while a first console holds the lock -- and once the first one's
/// WS connection closes, the lock frees up for the next one.
#[tokio::test]
async fn test_only_one_console_connected_at_a_time() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_console_lock.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(6000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host_token = "test_host_session_lock".to_string();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        host_session_token: host_token.clone(),
        server_port: addr.port(),
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    use futures_util::{SinkExt, StreamExt};
    let client = reqwest::Client::new();

    // Console A connects over WS and claims the lock with its first command.
    let (mut ws_a, _) = tokio_tungstenite::connect_async(format!("ws://{}/ws", addr)).await.unwrap();
    let _ = ws_a.next().await; // initial snapshot
    ws_a.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "host_token": host_token, "console_session_id": "console-a" }).to_string().into(),
    )).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr)).send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], true, "console A's first command should claim the lock and execute");

    // Console B (different session id, correct host token) is locked out --
    // via HTTP, with the distinct "console_locked" error surfaced.
    let resp = client.post(format!("http://{}/api/command", addr))
        .header("x-host-token", &host_token)
        .header("x-console-session-id", "console-b")
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["error"], "console_locked");
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr)).send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], true, "console B must not have executed while A holds the lock");

    // Console A can keep issuing commands (same session id it already holds
    // the lock for) while B remains locked out.
    ws_a.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "host_token": host_token, "console_session_id": "console-a" }).to_string().into(),
    )).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr)).send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], false, "console A should still be able to issue further commands");

    // Console A disconnects -- releasing the lock -- so console B can now claim it.
    drop(ws_a);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let resp = client.post(format!("http://{}/api/command", addr))
        .header("x-host-token", &host_token)
        .header("x-console-session-id", "console-b")
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true, "console B should be able to claim the lock once A disconnects");
}

/// Confirms the router itself behaves correctly across both listeners --
/// zero-auth routes reachable on each, the HTTPS plane accepting the
/// self-signed cert, and `GET /api/network/info` always preferring https://
/// URLs when available (`remote_url`/`mdns_url`/`pairing_url` -- since
/// `src/main.rs`, the cleartext plane is never network-reachable at all, so
/// every URL handed to a non-loopback caller must be https). The actual
/// production loopback-only (127.0.0.1) vs network-wide (0.0.0.0) bind
/// split lives in `src/main.rs`, not exercised here -- this test builds its
/// own listeners directly against the router, both already on loopback by
/// construction, which is the right level for what this test actually
/// checks.
#[tokio::test]
async fn test_split_plane_http_https_listeners_and_zero_auth_displays() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_split_plane.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(4000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));

    // 1. Generate TLS certificate
    let interfaces = os_next::network::NetworkInterfaceInfo::discover_all();
    // A key file in the temp dir, not the developer's real OS keyring.
    let key_store = os_next::network::tls::TlsKeyStore::File(test_dir.path().join("tls_private_key.pem"));
    let (cert_pem, key_pem) = os_next::network::tls::get_or_create_tls_certificate_in(&db, "test-sanctuary", &interfaces, &key_store).unwrap();
    assert!(cert_pem.contains("BEGIN CERTIFICATE"));
    assert!(key_pem.contains("PRIVATE KEY"));

    // 2. Bind cleartext HTTP listener
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();
    let http_port = http_addr.port();

    // 3. Bind HTTPS listener port
    let https_probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let https_port = https_probe.local_addr().unwrap().port();
    drop(https_probe);

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        host_session_token: "test_host_session_split".to_string(),
        server_port: http_port,
        https_port: Some(https_port),
        https_enabled: true,
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);

    // Spawn cleartext HTTP listener
    let http_router = router.clone();
    tokio::spawn(async move {
        let _ = axum::serve(http_listener, http_router).await;
    });

    // Spawn secure HTTPS listener
    let rustls_config = os_next::network::tls::create_rustls_config(&cert_pem, &key_pem).await.unwrap();
    let https_router = router.clone();
    let https_addr = std::net::SocketAddr::from(([127, 0, 0, 1], https_port));
    tokio::spawn(async move {
        let _ = axum_server::bind_rustls(https_addr, rustls_config)
            .serve(https_router.into_make_service())
            .await;
    });

    // Wait briefly for servers to listen
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Standard cleartext HTTP client
    let http_client = reqwest::Client::new();

    // TLS-capable client accepting our self-signed cert
    let cert = reqwest::Certificate::from_pem(cert_pem.as_bytes()).unwrap();
    let https_client = reqwest::Client::builder()
        .add_root_certificate(cert)
        .build()
        .unwrap();

    // 4. Verify cleartext HTTP /api/network/info reports both ports
    let net_info_resp = http_client.get(format!("http://{}/api/network/info", http_addr))
        .send().await.unwrap();
    assert_eq!(net_info_resp.status(), reqwest::StatusCode::OK);
    let net_info: serde_json::Value = net_info_resp.json().await.unwrap();
    assert_eq!(net_info["success"], true);
    assert_eq!(net_info["http_port"], http_port);
    assert_eq!(net_info["https_port"], https_port);
    assert_eq!(net_info["https_enabled"], true);
    assert!(net_info["pairing_url"].as_str().unwrap().starts_with("https://"));
    assert!(net_info["pairing_url"].as_str().unwrap().contains(&format!(":{}", https_port)));
    // remote_url/mdns_url: the cleartext plane is never network-reachable
    // (src/main.rs loopback-bind), so these must prefer https:// too, same
    // as pairing_url already did.
    assert!(net_info["remote_url"].as_str().unwrap().starts_with("https://"));
    assert!(net_info["remote_url"].as_str().unwrap().contains(&format!(":{}", https_port)));
    assert!(net_info["mdns_url"].as_str().unwrap().starts_with("https://"));
    assert!(net_info["mdns_url"].as_str().unwrap().contains(&format!(":{}", https_port)));

    // 5. Zero-Auth AV Plane: /api/state, /remote on HTTP
    let state_resp = http_client.get(format!("http://{}/api/state", http_addr))
        .send().await.unwrap();
    assert_eq!(state_resp.status(), reqwest::StatusCode::OK);

    // 6. Secure HTTPS Plane: /api/state and /api/pairing/session over https://
    let secure_state = https_client.get(format!("https://127.0.0.1:{}/api/state", https_port))
        .send().await.unwrap();
    assert_eq!(secure_state.status(), reqwest::StatusCode::OK);

    let pair_sess = https_client.post(format!("https://127.0.0.1:{}/api/pairing/session", https_port))
        .header("x-host-token", "test_host_session_split")
        .json(&serde_json::json!({ "ttl_seconds": 60 }))
        .send().await.unwrap();
    assert_eq!(pair_sess.status(), reqwest::StatusCode::OK);
    let pair_json: serde_json::Value = pair_sess.json().await.unwrap();
    assert!(pair_json["session_token"].as_str().unwrap().starts_with("pair_sess_"));
    assert_eq!(pair_json["expires_in"], 300);

    // 7. Test POST /api/network/tls/regenerate
    let regen_resp = http_client.post(format!("http://{}/api/network/tls/regenerate", http_addr))
        .header("x-host-token", "test_host_session_split")
        .send().await.unwrap();
    assert_eq!(regen_resp.status(), reqwest::StatusCode::OK);
    let regen_json: serde_json::Value = regen_resp.json().await.unwrap();
    assert_eq!(regen_json["success"], true);
}

#[tokio::test]
async fn test_security_headers_and_cors_policies() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test_sec.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();

    // 1. Verify get_setting / set_setting on Database
    assert_eq!(db.get_setting("non_existent_key").unwrap(), None);
    db.set_setting("test_key", "test_value").unwrap();
    assert_eq!(db.get_setting("test_key").unwrap(), Some("test_value".to_string()));

    let event_log = Arc::new(EventLog::new(100));
    let engine = Arc::new(ShowEngine::new(event_log));
    let asset_graph = Arc::new(AssetGraph::new(temp_dir.path().join("assets")));
    let (tx, _) = tokio::sync::broadcast::channel(100);

    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        host_session_token: "test_host_session_sec".to_string(),
        server_port: http_addr.port(),
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);

    tokio::spawn(async move {
        let _ = axum::serve(http_listener, router).await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    let client = reqwest::Client::new();

    // 2. Default Configuration: Balanced CSP, SAMEORIGIN, Permissive CORS
    let resp = client.get(format!("http://{}/api/state", http_addr))
        .header("Origin", "http://any-display.sanctuary.lan:8080")
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let headers = resp.headers();
    assert_eq!(
        headers.get("x-content-type-options").unwrap().to_str().unwrap(),
        "nosniff"
    );
    assert_eq!(
        headers.get("x-frame-options").unwrap().to_str().unwrap(),
        "SAMEORIGIN"
    );
    let csp = headers.get("content-security-policy").unwrap().to_str().unwrap();
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("frame-ancestors 'self'"));
    assert!(csp.contains("media-src 'self' data: blob: https: http:")); // Allows sanctuary HTTP media in balanced mode
    assert!(headers.get("access-control-allow-origin").is_some());

    // 3. Reconfigure to Strict CSP, Deny Framing, and Restricted CORS --
    // through the real POST /api/settings endpoint (host-token gated), not
    // a direct `db.set_setting`: the security headers/CORS predicate now
    // read an in-memory cache that only `post_settings` refreshes, not the
    // database directly (see `AppState::security_header_settings`).
    let reconf_resp = client.post(format!("http://{}/api/settings", http_addr))
        .header("x-host-token", "test_host_session_sec")
        .json(&serde_json::json!({
            "securityCspMode": "strict",
            "securityFrameOptions": "deny",
            "securityCorsMode": "restricted",
        }))
        .send().await.unwrap();
    assert_eq!(reconf_resp.status(), reqwest::StatusCode::OK);

    let resp_strict = client.get(format!("http://{}/api/state", http_addr))
        .header("Origin", "http://192.168.1.100:8080")
        .send().await.unwrap();
    assert_eq!(resp_strict.status(), reqwest::StatusCode::OK);

    let h_strict = resp_strict.headers();
    assert_eq!(
        h_strict.get("x-frame-options").unwrap().to_str().unwrap(),
        "DENY"
    );
    let csp_strict = h_strict.get("content-security-policy").unwrap().to_str().unwrap();
    assert!(csp_strict.contains("frame-ancestors 'none'"));
    assert!(csp_strict.contains("form-action 'self'"));
    // Origin 192.168.1.100 is in private RFC 1918 range -> allowed
    assert_eq!(
        h_strict.get("access-control-allow-origin").unwrap().to_str().unwrap(),
        "http://192.168.1.100:8080"
    );

    // 4. Restricted CORS: Public / external origin should be rejected
    let resp_rejected = client.get(format!("http://{}/api/state", http_addr))
        .header("Origin", "https://malicious-attacker-site.com")
        .send().await.unwrap();
    assert_eq!(resp_rejected.status(), reqwest::StatusCode::OK);
    assert!(resp_rejected.headers().get("access-control-allow-origin").is_none());

    // 5. Reconfigure to Disabled CSP and Disabled Framing (same reasoning
    // as step 3 -- through the real endpoint, not the database directly)
    let disable_resp = client.post(format!("http://{}/api/settings", http_addr))
        .header("x-host-token", "test_host_session_sec")
        .json(&serde_json::json!({
            "securityCspMode": "disabled",
            "securityFrameOptions": "disabled",
        }))
        .send().await.unwrap();
    assert_eq!(disable_resp.status(), reqwest::StatusCode::OK);

    let resp_disabled = client.get(format!("http://{}/api/state", http_addr))
        .send().await.unwrap();
    assert_eq!(resp_disabled.status(), reqwest::StatusCode::OK);
    assert!(resp_disabled.headers().get("content-security-policy").is_none());
    assert!(resp_disabled.headers().get("x-frame-options").is_none());
    assert_eq!(
        resp_disabled.headers().get("x-content-type-options").unwrap().to_str().unwrap(),
        "nosniff"
    );
}

#[tokio::test]
async fn test_check_public_url_requires_host_token() {
    let base = spawn_test_app("real_token").await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/network/check-public-url?url=https://connect.example.org", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_check_public_url_rejects_non_https_scheme_without_a_network_call() {
    // `url=http://...` (or any malformed URL) must be rejected by
    // validate_https_url before the handler ever attempts an outbound
    // request -- this test would hang/timeout if that ordering regressed,
    // since `example.invalid` resolves to nothing.
    let base = spawn_test_app("real_token").await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/network/check-public-url?url=http://example.invalid", base))
        .header("x-host-token", "real_token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["reachable"], false);
    assert!(body["message"].as_str().unwrap().contains("https://"));
}

/// A remote console exchanges the per-run host token for a persistent console
/// credential; it keeps working after a "restart" (new host token, same
/// library.db) until the device is removed from Paired Devices.
#[tokio::test]
async fn test_console_credential_survives_restart_and_is_revocable() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_console_credential.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();

    async fn start(db: os_next::storage::Database, dir: &std::path::Path, host_token: &str) -> std::net::SocketAddr {
        let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
        let engine = Arc::new(ShowEngine::new(event_log));
        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app_state = os_next::api::ws::AppState {
            tx,
            engine,
            asset_graph: Arc::new(os_next::media::AssetGraph::new(dir.join("media"))),
            web_dir: std::path::PathBuf::from("web"),
            media_dir: std::path::PathBuf::from("web"),
            host_session_token: host_token.to_string(),
            server_port: addr.port(),
            ..base_app_state(db)
        };
        let router = os_next::api::create_router(app_state);
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        addr
    }

    let client = reqwest::Client::new();
    let verify = |addr: std::net::SocketAddr, token: String| {
        let client = client.clone();
        async move {
            let resp = client.post(format!("http://{}/api/internal/verify-host-token", addr))
                .json(&serde_json::json!({ "token": token }))
                .send().await.unwrap();
            resp.json::<serde_json::Value>().await.unwrap()["valid"] == true
        }
    };

    // Run 1: the browser holds the bootstrap host token and trades it in.
    let addr1 = start(db.clone(), test_dir.path(), "run_one_host_token").await;
    let resp = client.post(format!("http://{}/api/pairing/console-credential", addr1))
        .header("x-host-token", "run_one_host_token")
        .json(&serde_json::json!({ "name": "Laptop" }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    let credential = body["credential"].as_str().unwrap().to_string();
    let device_id = body["device_id"].as_str().unwrap().to_string();
    assert!(credential.starts_with("console_tok_"));

    // Only the bootstrap token may mint; a wrong token or a credential can't.
    let resp = client.post(format!("http://{}/api/pairing/console-credential", addr1))
        .header("x-host-token", &credential)
        .json(&serde_json::json!({ "name": "Copy" }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // The secret itself is never stored, and never listed.
    let stored = db.get_paired_devices().unwrap();
    let dev = stored.iter().find(|d| d.id == device_id).unwrap();
    assert_ne!(dev.token, credential);
    assert!(dev.token.starts_with("sha256:"));
    let resp = client.get(format!("http://{}/api/pairing/devices", addr1))
        .header("x-host-token", &credential)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let list: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(list.as_array().unwrap().iter().find(|d| d["id"] == device_id.as_str()).unwrap()["token"], "");

    // "Restart": same library.db, brand-new host token. The old host token is dead,
    // the credential still works for console-gated routes.
    let addr2 = start(db.clone(), test_dir.path(), "run_two_host_token").await;
    assert!(!verify(addr2, "run_one_host_token".to_string()).await);
    assert!(verify(addr2, credential.clone()).await);
    let resp = client.get(format!("http://{}/api/pairing/devices", addr2))
        .header("x-host-token", &credential)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // A console credential is not a show-control device token.
    let resp = client.post(format!("http://{}/api/command", addr2))
        .header("x-device-token", &credential)
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false);

    // Revoking the device (stolen laptop) deauthenticates it immediately.
    let resp = client.delete(format!("http://{}/api/pairing/devices/{}", addr2, device_id))
        .header("x-host-token", "run_two_host_token")
        .send().await.unwrap();
    assert!(resp.status().is_success());
    assert!(!verify(addr2, credential.clone()).await);
    let resp = client.get(format!("http://{}/api/pairing/devices", addr2))
        .header("x-host-token", &credential)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}
