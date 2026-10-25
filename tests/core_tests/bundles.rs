//! `.ossong` / `.ospres` / `.osservice` / `.osbible` over HTTP: the token gate, uploads bigger than the
//! global body limit, streaming both ways, the console lock for services, and hostile files.
//! (The format itself is tested next to it, in `src/storage/bundle/tests.rs`.)

use super::*;
use os_next::api::ws::AppState;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake-but-recognisable-png-bytes";

struct Server {
    addr: SocketAddr,
    token: String,
    db: Database,
    engine: Arc<ShowEngine>,
    media: PathBuf,
    data: PathBuf,
    _dirs: tempfile::TempDir,
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

async fn start() -> Server {
    let dirs = tempfile::tempdir().unwrap();
    let media = dirs.path().join("media");
    let data = dirs.path().join("data");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let db = Database::new(data.join("library.db").to_str().unwrap()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = "bundle_test_host_token".to_string();
    let state = AppState { media_dir: media.clone(), data_dir: data.clone(), host_session_token: token.clone(), server_port: addr.port(), ..base_app_state(db.clone()) };
    let engine = state.engine.clone();
    let router = os_next::api::create_router(state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Server { addr, token, db, engine, media, data, _dirs: dirs }
}

fn song_with_picture(id: &str, title: &str, copyright: Option<&str>) -> Song {
    let mut s = Song::new(title, "A. Writer");
    s.id = id.to_string();
    s.copyright = copyright.map(str::to_string);
    s.slides = vec![Slide { text: "Verse one".into(), background: Some("url('/media/images/dawn.png')".into()), ..Default::default() }];
    s
}

fn put_picture(server: &Server) {
    std::fs::create_dir_all(server.media.join("images")).unwrap();
    std::fs::write(server.media.join("images/dawn.png"), PNG).unwrap();
}

async fn export(server: &Server, body: serde_json::Value) -> reqwest::Response {
    reqwest::Client::new().post(server.url("/api/export/bundle")).header("x-host-token", &server.token).json(&body).send().await.unwrap()
}

async fn upload(server: &Server, path: &str, bytes: Vec<u8>) -> reqwest::Response {
    reqwest::Client::new().post(server.url(path)).header("x-host-token", &server.token).body(bytes).send().await.unwrap()
}

fn spool_leftovers(server: &Server) -> Vec<String> {
    std::fs::read_dir(server.data.join("tmp"))
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("bundle-")).collect())
        .unwrap_or_default()
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(files_under(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[tokio::test]
async fn songs_move_between_two_libraries_over_http_with_their_pictures() {
    let a = start().await;
    put_picture(&a);
    a.db.insert_song(&song_with_picture("s_pd", "Old Hymn", Some("Public Domain"))).unwrap();
    a.db.insert_song(&song_with_picture("s_cr", "New Anthem", Some("© 2020 Label"))).unwrap();

    // Gate: no token, no file.
    let denied = reqwest::Client::new().post(a.url("/api/export/bundle")).json(&serde_json::json!({ "kind": "song" })).send().await.unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::UNAUTHORIZED);

    let resp = export(&a, serde_json::json!({ "kind": "song", "ids": ["s_pd", "s_cr"] })).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let disposition = resp.headers()["content-disposition"].to_str().unwrap().to_string();
    assert!(disposition.contains("2 songs.ossong"), "{disposition}");
    assert_eq!(resp.headers()["x-os-media-count"], "1", "the picture both songs use is stored once");
    assert_eq!(resp.headers()["x-os-missing-media"], "0");
    let declared: usize = resp.headers()["content-length"].to_str().unwrap().parse().unwrap();
    let bytes = resp.bytes().await.unwrap().to_vec();
    assert_eq!(bytes.len(), declared);
    assert!(spool_leftovers(&a).is_empty(), "the export's working file is removed once it has been sent");

    let b = start().await;
    // Gate again, on both import routes.
    for path in ["/api/bundle/inspect", "/api/bundle/import"] {
        let denied = reqwest::Client::new().post(b.url(path)).body(bytes.clone()).send().await.unwrap();
        assert_eq!(denied.status(), reqwest::StatusCode::UNAUTHORIZED, "{path}");
    }
    assert!(b.db.get_songs().unwrap().is_empty());

    // Inspect says what's inside, flags the copyrighted song, and changes nothing.
    let inspected: serde_json::Value = upload(&b, "/api/bundle/inspect", bytes.clone()).await.json().await.unwrap();
    assert_eq!(inspected["summary"]["kind"], "song");
    assert_eq!(inspected["summary"]["copyrighted"].as_array().unwrap().len(), 1);
    assert_eq!(inspected["summary"]["copyrighted"][0]["title"], "New Anthem");
    assert_eq!(inspected["summary"]["library"]["new"], 2);
    assert!(b.db.get_songs().unwrap().is_empty());
    assert!(files_under(&b.media).is_empty());

    let imported: serde_json::Value = upload(&b, "/api/bundle/import", bytes.clone()).await.json().await.unwrap();
    assert_eq!(imported["success"], true);
    assert_eq!(imported["result"]["added"], 2);
    assert_eq!(imported["result"]["media_written"], 1);
    assert_eq!(b.db.get_songs().unwrap().len(), 2);
    let restored = files_under(&b.media);
    assert_eq!(restored.len(), 1);
    assert_eq!(std::fs::read(&restored[0]).unwrap(), PNG);
    assert!(b.db.get_song_by_id("s_cr").unwrap().unwrap().slides[0].background.as_ref().unwrap().contains("/media/images/dawn-"));
    assert!(spool_leftovers(&b).is_empty());

    // Again: nothing new, nothing duplicated.
    let again: serde_json::Value = upload(&b, "/api/bundle/import", bytes).await.json().await.unwrap();
    assert_eq!((again["result"]["added"].as_u64(), again["result"]["skipped_identical"].as_u64()), (Some(0), Some(2)));
    assert_eq!(b.db.get_songs().unwrap().len(), 2);
}

#[tokio::test]
async fn a_bundle_bigger_than_the_global_upload_limit_still_goes_through() {
    // The router's default body limit is 32 MiB; bundles carry video, so these routes lift it. This is the
    // regression test for that layering, and for uploads being streamed to disk rather than held in memory.
    let a = start().await;
    std::fs::create_dir_all(a.media.join("videos")).unwrap();
    let mut video = b"\0\0\0\x18ftypmp42\0\0\0\0".to_vec();
    video.extend((0..40 * 1024 * 1024u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8));
    std::fs::write(a.media.join("videos/big.mp4"), &video).unwrap();
    let mut s = Song::new("Big Loop", "x");
    s.id = "big".into();
    s.slides = vec![Slide { text: "t".into(), background: Some("/media/videos/big.mp4".into()), ..Default::default() }];
    a.db.insert_song(&s).unwrap();

    let bytes = export(&a, serde_json::json!({ "kind": "song" })).await.bytes().await.unwrap().to_vec();
    assert!(bytes.len() > 40 * 1024 * 1024, "stored, not shrunk: {}", bytes.len());

    let b = start().await;
    let resp = upload(&b, "/api/bundle/import", bytes).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "{}", resp.text().await.unwrap());
    let restored = files_under(&b.media);
    assert_eq!(restored.len(), 1);
    assert_eq!(std::fs::read(&restored[0]).unwrap(), video);
    assert!(spool_leftovers(&b).is_empty());
}

#[tokio::test]
async fn damaged_and_hostile_uploads_are_refused_and_leave_nothing_behind() {
    let b = start().await;
    let garbage = upload(&b, "/api/bundle/import", b"definitely not a zip".to_vec()).await;
    assert_eq!(garbage.status(), reqwest::StatusCode::BAD_REQUEST);
    let message = garbage.json::<serde_json::Value>().await.unwrap()["message"].as_str().unwrap().to_string();
    assert!(message.contains("isn't an OpenSanctuary file"), "{message}");

    let empty = upload(&b, "/api/bundle/import", Vec::new()).await;
    assert_eq!(empty.status(), reqwest::StatusCode::BAD_REQUEST);

    // A real zip with a path-escaping entry.
    let evil = make_zip(&[("manifest.json", br#"{"format":"opensanctuary-bundle","format_version":1,"kind":"song"}"#), ("content.json", b"{}"), ("../../evil.txt", b"x")]);
    let resp = upload(&b, "/api/bundle/import", evil).await;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(resp.text().await.unwrap().contains("unexpected entry"));

    assert!(b.db.get_songs().unwrap().is_empty());
    assert!(files_under(&b.media).is_empty());
    assert!(!b.data.parent().unwrap().join("evil.txt").exists());
    assert!(spool_leftovers(&b).is_empty(), "rejected uploads are cleaned up too");
}

#[tokio::test]
async fn a_service_opens_into_the_live_show_and_only_for_the_console_that_holds_the_lock() {
    let a = start().await;
    put_picture(&a);
    a.engine.execute_command(ShowCommand::AddToSchedule(ScheduleItem { background: Some("/media/images/dawn.png".into()), ..simple_item("i1", "Welcome", &["Hello"]) }));
    a.engine.execute_command(ShowCommand::AddToSchedule(simple_item("i2", "Announcements", &["News"])));

    let resp = export(&a, serde_json::json!({ "kind": "service", "title": "Easter / Sunrise" })).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let disposition = resp.headers()["content-disposition"].to_str().unwrap().to_string();
    assert!(disposition.contains("Easter _ Sunrise.osservice"), "a title can't smuggle a path into the file name: {disposition}");
    let bytes = resp.bytes().await.unwrap().to_vec();

    let b = start().await;
    // No console session id: refused before a single file is written.
    let refused = upload(&b, "/api/bundle/import", bytes.clone()).await;
    assert_eq!(refused.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(files_under(&b.media).is_empty());
    assert!(b.engine.snapshot().schedule.items.is_empty());

    let resp = reqwest::Client::new()
        .post(b.url("/api/bundle/import"))
        .header("x-host-token", &b.token)
        .header("x-console-session-id", "console-1")
        .body(bytes.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let schedule = b.engine.snapshot().schedule.clone();
    assert_eq!(schedule.title, "Easter / Sunrise", "only the file name is sanitized, never the title");
    assert_eq!(schedule.items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Welcome", "Announcements"]);
    assert!(schedule.items[0].background.as_ref().unwrap().starts_with("/media/images/dawn-"));
    assert_eq!(files_under(&b.media).len(), 1);

    // Append mode adds to what's there instead of replacing it.
    let resp = reqwest::Client::new()
        .post(b.url("/api/bundle/import?mode=append"))
        .header("x-host-token", &b.token)
        .header("x-console-session-id", "console-1")
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(b.engine.snapshot().schedule.items.len(), 4);
}

#[tokio::test]
async fn a_bible_installs_once_and_replacing_it_is_an_explicit_choice() {
    let a = start().await;
    let item = |book: &str, ch: u32| ScriptureItem {
        id: format!("{book}{ch}"),
        book: book.into(),
        chapter: ch,
        verse_start: 1,
        verse_end: 1,
        version: "TST".into(),
        reference: format!("{book} {ch}"),
        verses: vec![ScriptureVerse { verse_number: 1, text: format!("{book} {ch} text") }],
        theme_name: None,
    };
    a.db.insert_scriptures_batch(&[item("Genesis", 1), item("John", 3)]).unwrap();
    let bytes = export(&a, serde_json::json!({ "kind": "bible", "version": "tst" })).await.bytes().await.unwrap().to_vec();

    let missing = export(&a, serde_json::json!({ "kind": "bible", "version": "NOPE" })).await;
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    let no_version = export(&a, serde_json::json!({ "kind": "bible" })).await;
    assert_eq!(no_version.status(), reqwest::StatusCode::BAD_REQUEST);

    let b = start().await;
    let first = upload(&b, "/api/bundle/import", bytes.clone()).await;
    assert_eq!(first.status(), reqwest::StatusCode::OK);
    assert_eq!(b.db.get_scriptures_for_version("TST").unwrap().len(), 2);

    let inspected: serde_json::Value = upload(&b, "/api/bundle/inspect", bytes.clone()).await.json().await.unwrap();
    assert_eq!(inspected["summary"]["bible_installed"], true);

    let second = upload(&b, "/api/bundle/import", bytes.clone()).await;
    assert_eq!(second.status(), reqwest::StatusCode::CONFLICT);
    let replaced = upload(&b, "/api/bundle/import?replace=true", bytes).await;
    assert_eq!(replaced.status(), reqwest::StatusCode::OK);
    assert_eq!(b.db.get_scriptures_for_version("TST").unwrap().len(), 2);
}
