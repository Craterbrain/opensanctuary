//! Integration tests against the `os_next` library, one module per area. One test binary
//! (`cargo test --test core_tests`), so splitting these out doesn't add link time.

use os_next::core::models::{ScriptureItem, ScriptureVerse};
use std::sync::Arc;
use os_next::core::commands::ShowCommand;
use os_next::core::engine::ShowEngine;
use os_next::core::event_log::EventLog;
use os_next::core::events::ShowEvent;
use os_next::core::models::{ArrangementEntry, Presentation, Schedule, ScheduleItem, Slide, SlideBackground, SlideElement, Song, Theme, ToScheduleItem};
use os_next::media::asset_graph::AssetGraph;
use os_next::storage::db::Database;
use os_next::storage::ewsx::EwsxManager;
use os_next::storage::freeshow_import::parse_scripture_reference;
use os_next::storage::freeshow_show_import::FreeShowShowImporter;
use os_next::storage::openlp_import::OpenLPImporter;
use os_next::storage::pptx_import::PptxImporter;

mod bundles;
mod engine;
mod importers;
mod network;
mod packaging;
mod plugins;
mod storage;

/// An `AppState` with the defaults nearly every test wants (an empty engine, a throwaway
/// broadcast channel, no HTTPS, no display, a fresh plugin manager, port 8080). A test overrides
/// only what it cares about: `AppState { host_session_token: "x".into(), ..base_app_state(db) }`.
fn base_app_state(db: Database) -> os_next::api::ws::AppState {
    let engine = Arc::new(ShowEngine::new(Arc::new(EventLog::with_initial_sequence(5000, 0))));
    os_next::api::ws::AppState {
        engine,
        db,
        asset_graph: Arc::new(AssetGraph::new("web")),
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        tx: tokio::sync::broadcast::channel(16).0,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        notifications: Arc::new(os_next::api::notifications::NotificationBus::new()),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: "test_host_session_token".to_string(),
        server_port: 8080,
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        plugin_release_key: os_next::network::updater::RELEASE_PUBLIC_KEY_B64.to_string(),
    }
}

/// A blank presentation item: every field at its default except `id` and `title`. Tests state only what
/// they care about: `ScheduleItem { item_type: "song".into(), slides: …, ..item("a", "A") }`.
fn item(id: &str, title: &str) -> ScheduleItem {
    ScheduleItem {
        id: id.to_string(),
        title: title.to_string(),
        item_type: "presentation".to_string(),
        author_or_ref: String::new(),
        slides: Vec::new(),
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false, subtitle: None, notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    }
}

/// `item` with one plain slide per text.
fn simple_item(id: &str, title: &str, texts: &[&str]) -> ScheduleItem {
    ScheduleItem { slides: texts.iter().map(|t| Slide { text: t.to_string(), ..Default::default() }).collect(), ..item(id, title) }
}

// ---------------------------------------------------------------------------------------------
// Installing and removing plugins from a package (POST /api/plugins/install, DELETE /api/plugins/:id)
// ---------------------------------------------------------------------------------------------

fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::FileOptions::<()>::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

// ---------------------------------------------------------------------------------------------
// Plugin signing phase 3: the release key, the revocation list, key rotation
// (uses the throwaway keys in testdata/signing/, see its README)
// ---------------------------------------------------------------------------------------------

struct TestKey {
    sk: minisign::SecretKey,
    b64: String,
}

fn test_key(name: &str) -> TestKey {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/signing");
    let sk = minisign::SecretKeyBox::from_string(&std::fs::read_to_string(dir.join(format!("{name}.key"))).unwrap()).unwrap().into_unencrypted_secret_key().unwrap();
    let pk_text = std::fs::read_to_string(dir.join(format!("{name}.pub"))).unwrap();
    let b64 = pk_text.lines().nth(1).unwrap().trim().to_string();
    TestKey { sk, b64 }
}

/// A real-world document that can't be committed (someone's own content, e.g. a church's announcement
/// deck): looked up in `$OS_TEST_DOCS`, default `~/Documents/OpenSanctuary-test-docs` (docs/workflow.md,
/// "Real-world test documents"). `None` -- and the caller skips -- when it isn't on this machine (CI).
fn test_doc(name: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::var("OS_TEST_DOCS").map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Documents/OpenSanctuary-test-docs"));
    let path = dir.join(name);
    if path.exists() { Some(path) } else { eprintln!("skipping: {} not present ($OS_TEST_DOCS)", path.display()); None }
}
