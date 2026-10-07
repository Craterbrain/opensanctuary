use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use os_next::api::ws::{AppState, SecurityHeaderSettings};
use os_next::api::create_router;
use os_next::core::engine::ShowEngine;
use os_next::core::event_log::EventLog;
use os_next::media::asset_graph::AssetGraph;
use os_next::storage::Database;

#[derive(Parser, Debug)]
#[command(name = "os-next", version = "0.2.0", about = "Next-Generation Church Presentation Engine in Rust")]
struct Args {
    #[arg(short, long, help = "Loopback-only port for the native desktop console window [default: 8080, auto-increments if in use] -- never reachable from the network")]
    port: Option<u16>,

    #[arg(long, help = "HTTPS port, reachable from the network -- the only way any other device (Live/Stage/Remote/Android TV) reaches this app [default: 8443]")]
    https_port: Option<u16>,

    #[arg(long, help = "SQLite library database path [default: platform data dir, or ./library.db in dev/portable mode -- see docs/paths.md]")]
    db_path: Option<String>,

    #[arg(long)]
    bibles_dir: Option<String>,

    #[arg(long)]
    songs_dir: Option<String>,

    #[arg(long, help = "Directory for user-downloaded/searched background media [default: alongside the database -- see docs/paths.md]")]
    media_dir: Option<String>,

    #[arg(long, help = "Static web UI assets directory [default: auto-detected next to the executable, or ./web in dev mode -- see docs/paths.md]")]
    web_dir: Option<String>,

    #[arg(long)]
    headless: bool,

    #[arg(long)]
    no_open: bool,

    #[arg(long, help = "Never auto-show the first-time setup wizard (docs/first-time.md) -- for automated tests, which always boot against a fresh temp database and would otherwise see it on every run")]
    skip_first_time_setup: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("GDK_BACKEND").is_none() {
            std::env::set_var("GDK_BACKEND", "x11,wayland");
        }
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        if std::env::var_os("GTK_USE_PORTAL").is_none() {
            std::env::set_var("GTK_USE_PORTAL", "1");
        }
    }

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();

    // Resolve where data actually lives (see docs/paths.md): an explicit CLI
    // flag always wins; otherwise a relocated-data-dir pointer file;
    // otherwise an already-populated platform data directory; otherwise
    // today's CWD-relative default, so `cargo run` and the existing
    // portable-folder distribution keep working unchanged.
    let resolved_paths = os_next::storage::paths::resolve(
        args.db_path.as_deref(),
        args.bibles_dir.as_deref(),
        args.songs_dir.as_deref(),
        args.media_dir.as_deref(),
    );

    // Finish any data-directory move a previous run started (Settings ->
    // Move Data Directory): that run copied everything and pointed
    // install.json here, but deferred deleting the old directory until now,
    // since it was still running against those files at the time. Must
    // happen before opening the database below, while nothing has anything
    // under the old directory open yet.
    os_next::storage::paths::cleanup_pending_source_dir(&resolved_paths.data_dir());

    let mut db = Database::new_with_dirs(&resolved_paths.db_path, &resolved_paths.bibles_dir, &resolved_paths.songs_dir)?;
    tracing::info!(
        "SQLite storage initialized with WAL mode at {:?} (bibles: {:?}, songs: {:?}, source: {:?})",
        resolved_paths.db_path,
        db.bibles_dir(),
        db.songs_dir(),
        resolved_paths.source,
    );

    let mut db_settings = db.get_settings().unwrap_or_default();

    // Settings-configured directory overrides (Settings -> Storage) need a
    // restart to take effect, same as networkPort/httpsPort below -- and
    // since bibles_dir/songs_dir are baked in at Database construction
    // (they open their own SQLite connections), applying an override means
    // reopening it. A CLI flag for a given directory always wins and skips
    // this reload for that directory.
    let settings_bibles_dir = db_settings.get("biblesDirectory").filter(|s| !s.trim().is_empty()).cloned();
    let settings_songs_dir = db_settings.get("songsDirectory").filter(|s| !s.trim().is_empty()).cloned();
    let effective_bibles_dir = args.bibles_dir.clone().or(settings_bibles_dir).map(PathBuf::from).unwrap_or_else(|| resolved_paths.bibles_dir.clone());
    let effective_songs_dir = args.songs_dir.clone().or(settings_songs_dir).map(PathBuf::from).unwrap_or_else(|| resolved_paths.songs_dir.clone());
    if effective_bibles_dir != resolved_paths.bibles_dir || effective_songs_dir != resolved_paths.songs_dir {
        tracing::info!(
            "Applying Settings-configured content directories: bibles={:?}, songs={:?}",
            effective_bibles_dir,
            effective_songs_dir
        );
        db = Database::new_with_dirs(&resolved_paths.db_path, &effective_bibles_dir, &effective_songs_dir)?;
        db_settings = db.get_settings().unwrap_or_default();
    }
    let media_dir = args.media_dir.clone()
        .or_else(|| db_settings.get("mediaCacheDirectory").filter(|s| !s.trim().is_empty()).cloned())
        .map(PathBuf::from)
        .unwrap_or_else(|| resolved_paths.media_dir.clone());

    let max_seq = db.get_max_event_sequence().unwrap_or(0);
    let event_log = Arc::new(EventLog::with_initial_sequence(2000, max_seq));
    let engine = Arc::new(ShowEngine::new(event_log));
    if max_seq > 0 {
        if let Ok(events) = db.load_events() {
            if !events.is_empty() {
                engine.restore_from_events(events);
                tracing::info!("Restored schedule and state from event_log (resuming at sequence #{})", max_seq);
            }
        }
    } else {
        tracing::info!("Event-sourced ShowEngine initialized (sequence #0)");
    }

    let plugin_dir = os_next::storage::paths::plugin_dir();
    let mut plugin_manager = os_next::core::plugins::PluginManager::new();
    if let Err(e) = plugin_manager.load_plugins_from_dir(&plugin_dir) {
        tracing::error!("Failed to load backend plugins: {}", e);
    } else {
        tracing::info!("Backend Plugin Manager initialized from {:?}", plugin_dir);
    }
    let plugin_manager = Arc::new(plugin_manager);

    let web_path = os_next::storage::paths::resolve_web_dir(args.web_dir.as_deref());
    // The initial images/videos folder sync used to happen right here, on
    // the plain pre-runtime main thread. Moved into the async block below
    // (near the update-check task) so it runs with a live Tokio handle --
    // sync_media_videos_folder's background transcode-check
    // (src/storage/transcode.rs) needs `Handle::try_current()` to succeed
    // to actually spawn, which it can't do this early. A file already
    // sitting in media/videos/ before the app's first launch was silently
    // never transcode-checked under the old ordering: the one scan that
    // ever saw it as "new" ran with no runtime, and every later scan
    // (including the one `get_media`'s route handler does on each request)
    // finds it already tracked and skips it.
    let asset_graph = Arc::new(AssetGraph::new(&media_dir));

    let (tx, _rx) = tokio::sync::broadcast::channel(512);

    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    // Only created when a native window will actually be launched (below); stays
    // None in --headless/--no-open mode, and API handlers report `available: false`.
    let display_manager = if !args.headless && !args.no_open {
        Some(Arc::new(os_next::webview::DisplayManagerHandle::new()))
    } else {
        None
    };

    let configured_port = args.port.or_else(|| {
        db_settings.get("networkPort").and_then(|p| p.parse::<u16>().ok())
    });

    // Bind TCP listener before spawning background thread or desktop webview.
    // Loopback-only (127.0.0.1), not 0.0.0.0: this plane exists purely for
    // the native desktop webview's own windows (src/webview/mod.rs) to load
    // -- 127.0.0.1/localhost is a secure context by spec regardless of
    // scheme, so the console loses nothing by staying on plain HTTP here.
    // Every other device (a second machine's browser, Remote Control
    // phones, Android TV, a Stage Foldback monitor on another box) now
    // reaches this app exclusively over the HTTPS plane below -- there is
    // no cleartext path reachable from the network anymore.
    let (std_listener, server_port) = match configured_port {
        Some(explicit_port) => {
            let addr = SocketAddr::from(([127, 0, 0, 1], explicit_port));
            match std::net::TcpListener::bind(addr) {
                Ok(l) => (l, explicit_port),
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                    eprintln!("\nError: Port {} is already in use by another process.", explicit_port);
                    eprintln!("Please specify a different port using --port <PORT> or stop the conflicting process.\n");
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("\nError: Failed to bind to port {}: {}\n", explicit_port, e);
                    std::process::exit(1);
                }
            }
        }
        None => {
            let mut bound = None;
            for p in 8080..=8090 {
                let addr = SocketAddr::from(([127, 0, 0, 1], p));
                match std::net::TcpListener::bind(addr) {
                    Ok(l) => {
                        bound = Some((l, p));
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                        continue;
                    }
                    Err(e) => {
                        eprintln!("\nError: Failed to bind to port {}: {}\n", p, e);
                        std::process::exit(1);
                    }
                }
            }
            match bound {
                Some((l, p)) => {
                    if p != 8080 {
                        tracing::warn!("Default port 8080 is in use. Automatically falling back to available port {}", p);
                        println!("Note: Port 8080 is in use (another OS-Next instance may be running). Using port {} instead.", p);
                    }
                    (l, p)
                }
                None => {
                    eprintln!("\nError: Default port 8080 and fallback ports 8081-8090 are all in use.");
                    eprintln!("Please specify an available port using --port <PORT> or stop existing processes.\n");
                    std::process::exit(1);
                }
            }
        }
    };
    std_listener.set_nonblocking(true)?;

    let configured_https_port: u16 = args.https_port.unwrap_or_else(|| {
        db_settings
            .get("httpsPort")
            .and_then(|p| p.parse().ok())
            .unwrap_or(8443)
    });

    let network_hostname = db_settings
        .get("networkHostname")
        .map(|s| s.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("opensanctuary");

    // HTTPS is no longer optional: it's the only plane reachable from the
    // network (see the loopback-bind comment above). A failure here is not
    // "disabling HTTPS" as an accepted configuration anymore -- it means
    // every non-loopback device is unreachable until it's fixed, so this
    // warns loudly rather than quietly degrading. The console itself still
    // works either way (the loopback plane is independent).
    let interfaces = os_next::network::NetworkInterfaceInfo::discover_all();
    let (https_port_actual, tls_pem_pair) = match os_next::network::tls::get_or_create_tls_certificate(&db, network_hostname, &interfaces) {
        Ok(pair) => {
            let mut chosen_port = None;
            for p in configured_https_port..=configured_https_port + 10 {
                if p == server_port {
                    continue;
                }
                if let Ok(_l) = std::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], p))) {
                    chosen_port = Some(p);
                    break;
                }
            }
            if let Some(p) = chosen_port {
                if p != configured_https_port {
                    tracing::warn!("HTTPS port {} in use, using fallback port {}", configured_https_port, p);
                }
                (Some(p), Some(pair))
            } else {
                tracing::error!(
                    "No available port for HTTPS between {} and {} -- every device other than this console is now unreachable until this is fixed",
                    configured_https_port, configured_https_port + 10
                );
                (None, None)
            }
        }
        Err(e) => {
            tracing::error!(
                "Failed to get or create TLS certificate: {} -- every device other than this console is now unreachable until this is fixed",
                e
            );
            (None, None)
        }
    };

    let host_session_token = uuid::Uuid::new_v4().to_string();
    let security_header_settings = SecurityHeaderSettings {
        csp_mode: db_settings.get("securityCspMode").cloned().unwrap_or_else(|| "balanced".to_string()),
        frame_mode: db_settings.get("securityFrameOptions").cloned().unwrap_or_else(|| "sameorigin".to_string()),
        cors_mode: db_settings.get("securityCorsMode").cloned().unwrap_or_else(|| "permissive".to_string()),
    };
    let app_state = AppState {
        tx: tx.clone(),
        engine: engine.clone(),
        db: db.clone(),
        asset_graph: asset_graph.clone(),
        web_dir: web_path.clone(),
        media_dir: media_dir.clone(),
        data_dir: resolved_paths.data_dir(),
        skip_first_time_setup: args.skip_first_time_setup,
        plugin_manager: plugin_manager.clone(),
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: display_manager.clone(),
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: host_session_token.clone(),
        server_port,
        https_port: https_port_actual,
        https_enabled: https_port_actual.is_some(),
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(security_header_settings)),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let headless = args.headless;
    let no_open = args.no_open;

    // LAN discovery: broadcast this console via mDNS as <hostname>.local
    // so Roku/Android TV clients can find it without a typed-in IP.
    let instance_id = os_next::discovery::get_or_create_instance_id(&db);
    let church_name = db_settings.get("churchName").cloned().unwrap_or_default();

    let _mdns_daemon = match os_next::discovery::start_mdns_broadcast(
        &instance_id,
        &church_name,
        network_hostname,
        server_port,
        https_port_actual,
    ) {
        Ok(daemon) => Some(daemon),
        Err(e) => {
            tracing::warn!("mDNS broadcast unavailable, console will only be reachable by IP: {}", e);
            None
        }
    };

    println!("\n============================================================");
    println!("  OS-Next - Next-Gen Church Presentation Engine (Rust)");
    println!("============================================================");
    println!("  Operator Console : http://127.0.0.1:{}/ (loopback only -- the native console window)", server_port);
    println!("  Live Display (FOH): http://127.0.0.1:{}/live.html (Zero-Auth, loopback only)", server_port);
    println!("  Stage Foldback   : http://127.0.0.1:{}/stage.html (Zero-Auth, loopback only)", server_port);
    if let Some(hp) = https_port_actual {
        let net_ip = os_next::network::get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
        println!("  Mobile Pairing   : https://127.0.0.1:{}/pairing.html", hp);
        println!("  -- Reachable from any other device on the network (Live/Stage/Remote/Android TV) --");
        println!("  Live Display (FOH): https://{}:{}/live.html (Zero-Auth)", net_ip, hp);
        println!("  Stage Foldback   : https://{}:{}/stage.html (Zero-Auth)", net_ip, hp);
        println!("  WebSocket Stream : wss://{}:{}/ws", net_ip, hp);
    } else {
        println!("  WARNING: HTTPS failed to start -- no device other than this console can reach OS-Next right now.");
    }
    println!("============================================================");

    // Lets the console be operated from a different machine on the LAN --
    // e.g. this server running headless, driven from the operator's own
    // laptop (see docs/CLIENT_PAIRING.md "Remote console access"). The
    // token is embedded in the URL fragment (never sent to the server as
    // part of the page request, same reasoning as /remote's token) --
    // whoever can read this terminal or its logs is the only one who gets
    // it, matching the existing "host-level trust means real access to this
    // machine" design, just extended to bootstrap a remote session instead
    // of only a loopback one.
    let lan_ip = os_next::network::get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    let self_signed_console_url = if let Some(hp) = https_port_actual {
        format!("https://{}:{}/#host_token={}", lan_ip, hp, host_session_token)
    } else {
        format!("http://{}:{}/#host_token={}", lan_ip, server_port, host_session_token)
    };
    // A configured Public HTTPS URL (docs/TUNNELS.md) always wins as the
    // *primary* line/QR -- it's a real, browser-trusted cert, which is the
    // whole reason to set it up -- but the self-signed URL is still printed
    // underneath, never silently dropped: it depends on external DNS/Caddy
    // actually being up, which this banner has no way to verify at boot.
    let public_https_url = db
        .get_settings()
        .ok()
        .and_then(|s| s.get("publicHttpsUrl").cloned())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let remote_console_url = public_https_url
        .as_ref()
        .map(|base| format!("{}/#host_token={}", base, host_session_token))
        .unwrap_or_else(|| self_signed_console_url.clone());
    println!("  Remote Console Access : {}", remote_console_url);
    if public_https_url.is_some() {
        println!("    (Self-signed fallback, if the above isn't reachable: {})", self_signed_console_url);
    }
    if https_port_actual.is_none() {
        println!("    (NOT ENCRYPTED -- HTTPS failed to start; this token will travel in cleartext over the LAN until it's fixed)");
    }
    println!("============================================================");
    for line in os_next::network::render_terminal_qr(&remote_console_url) {
        println!("  {}", line);
    }
    println!("============================================================\n");

    // Run Axum server in a background Tokio runtime thread
    let graph_for_task = asset_graph.clone();
    let startup_broadcast_hostname = network_hostname.to_string();
    let ytdlp_tools_dir_for_task = resolved_paths.tools_dir();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("Failed to initialize Tokio runtime: {}", e);
                return;
            }
        };
        rt.block_on(async move {
            let graph_clone = graph_for_task.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
                loop {
                    interval.tick().await;
                    graph_clone.evict_older_than(300_000);
                }
            });

            // Asynchronously broadcast DHCP Option 12 across interfaces
            let opt12_hostname = startup_broadcast_hostname.clone();
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(600)).await;
                let interfaces = os_next::network::NetworkInterfaceInfo::discover_all();
                // No operator to redirect at boot, so this can't fail-closed
                // the way the settings-save and adapter-toggle paths now do
                // — but it shouldn't broadcast a colliding hostname with zero
                // visibility either, so at least log it.
                //
                // check_hostname_conflict does blocking DNS/mDNS resolution
                // (std::net::to_socket_addrs) — spawn_blocking so it can't
                // stall this Tokio worker thread's other tasks (e.g. an
                // in-flight WebSocket command) while it waits on the
                // network. Confirmed this mattered: without it, an e2e test
                // clicking a live-output toggle right after boot started
                // missing its 5s deadline, consistent with this call
                // blocking whatever worker thread the WS task landed on.
                let hostname_for_check = opt12_hostname.clone();
                let conflict_check = tokio::task::spawn_blocking(move || {
                    let self_ips = os_next::network::get_all_local_ips();
                    os_next::network::check_hostname_conflict(&hostname_for_check, &self_ips)
                })
                .await;
                if let Ok(conflict) = conflict_check {
                    if conflict.conflict_detected {
                        tracing::warn!(
                            "Startup DHCP Option 12 broadcast: hostname '{}' conflicts with {} — {}",
                            opt12_hostname,
                            conflict.conflicting_ip.as_deref().unwrap_or("another device"),
                            conflict.message
                        );
                    }
                }
                let _ = os_next::network::DhcpOption12Client::broadcast_option_12(&opt12_hostname, &interfaces);
            });

            // Automated update check (docs/update.md "Check flow"): once on
            // startup, then re-checked every 12h -- `tokio::time::interval`'s
            // first `tick()` resolves immediately, so a single loop gives
            // "check now, then periodically" for free, same shape as the
            // asset-graph eviction task above. Never blocks boot -- this is
            // its own spawned task, and `create_router` below doesn't wait
            // on it.
            let update_status_for_task = app_state.update_status.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(12 * 60 * 60));
                loop {
                    interval.tick().await;
                    let result = match os_next::network::updater::check_latest_release().await {
                        Ok(latest) => os_next::network::updater::UpdateCheckResult {
                            checked_at_ms: chrono::Utc::now().timestamp_millis(),
                            current_version: env!("CARGO_PKG_VERSION").to_string(),
                            latest,
                            error: None,
                        },
                        Err(e) => {
                            tracing::warn!("Update check failed: {}", e);
                            os_next::network::updater::UpdateCheckResult {
                                checked_at_ms: chrono::Utc::now().timestamp_millis(),
                                current_version: env!("CARGO_PKG_VERSION").to_string(),
                                latest: None,
                                error: Some(e),
                            }
                        }
                    };
                    *update_status_for_task.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                }
            });

            // Initial media-folder sync (moved here from the pre-runtime
            // main thread -- see the comment where `images_dir`/
            // `videos_dir` used to be computed, above `AssetGraph::new`).
            // `get_media`'s route handler re-syncs both on every request
            // anyway (for files added while already running); this is
            // specifically so a file already present at first launch gets
            // its one-time transcode-check too, which needs a live runtime.
            if let Err(e) = app_state.db.sync_media_folder(&app_state.media_dir.join("images")) {
                tracing::warn!("Failed to sync media images folder: {}", e);
            }
            if let Err(e) = app_state.db.sync_media_videos_folder(&app_state.media_dir.join("videos")) {
                tracing::warn!("Failed to sync media videos folder: {}", e);
            }

            // Keeps yt-dlp current (src/network/ytdlp_updater.rs): once on
            // startup, then re-checked every 12h, same "tick() resolves
            // immediately" shape as the app's own update-check task above.
            // Never blocks boot -- its own spawned task, `create_router`
            // below doesn't wait on it. yt-dlp ships releases far more
            // often than this app does (sites keep breaking extraction),
            // which is why this exists at all instead of just bundling a
            // copy once at install time.
            let ytdlp_update_status_for_task = app_state.ytdlp_update_status.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(12 * 60 * 60));
                loop {
                    interval.tick().await;
                    let outcome = os_next::network::ytdlp_updater::check_and_update(&ytdlp_tools_dir_for_task).await;
                    if outcome.updated || outcome.error {
                        tracing::info!("yt-dlp updater: {}", outcome.message);
                    } else {
                        tracing::debug!("yt-dlp updater: {}", outcome.message);
                    }
                    *ytdlp_update_status_for_task.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
                }
            });

            let db_for_tls = app_state.db.clone();
            let router = create_router(app_state);
            let addr = SocketAddr::from(([127, 0, 0, 1], server_port));
            tracing::info!("OS-Next local console server (loopback only, never reachable from the network) listening on http://{}", addr);

            if let (Some(hp), Some((cert_pem, key_pem))) = (https_port_actual, tls_pem_pair) {
                match os_next::network::tls::create_rustls_config(&cert_pem, &key_pem).await {
                    Ok(rustls_config) => {
                        let https_addr = SocketAddr::from(([0, 0, 0, 0], hp));
                        let https_router = router.clone();
                        tokio::spawn(async move {
                            tracing::info!("OS-Next network server listening on https://{}", https_addr);
                            // with_connect_info: GET /api/internal/host-token needs the real
                            // peer address to enforce its loopback-only check.
                            if let Err(e) = axum_server::bind_rustls(https_addr, rustls_config)
                                .serve(https_router.into_make_service_with_connect_info::<SocketAddr>())
                                .await
                            {
                                tracing::error!("Axum HTTPS server encountered an error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Failed to create Rustls config with initial certificate: {}. Attempting fallback regeneration...", e);
                        let interfaces = os_next::network::NetworkInterfaceInfo::discover_all();
                        match os_next::network::tls::regenerate_tls_certificate(
                            &db_for_tls,
                            &startup_broadcast_hostname,
                            &interfaces,
                        ) {
                            Ok((new_cert, new_key)) => {
                                match os_next::network::tls::create_rustls_config(&new_cert, &new_key).await {
                                    Ok(rustls_config) => {
                                        let https_addr = SocketAddr::from(([0, 0, 0, 0], hp));
                                        let https_router = router.clone();
                                        tokio::spawn(async move {
                                            tracing::info!("OS-Next network server listening on https://{} (recovered via fallback regeneration)", https_addr);
                                            if let Err(e) = axum_server::bind_rustls(https_addr, rustls_config)
                                                .serve(https_router.into_make_service_with_connect_info::<SocketAddr>())
                                                .await
                                            {
                                                tracing::error!("Axum HTTPS server encountered an error: {}", e);
                                            }
                                        });
                                    }
                                    Err(err) => {
                                        tracing::error!("Failed to create Rustls config after regenerating certificate: {}", err);
                                    }
                                }
                            }
                            Err(err) => {
                                tracing::error!("Failed to regenerate TLS certificate: {}", err);
                            }
                        }
                    }
                }
            }

            match tokio::net::TcpListener::from_std(std_listener) {
                Ok(listener) => {
                    // with_connect_info: same reason as the HTTPS listener above.
                    if let Err(e) = axum::serve(
                        listener,
                        router.into_make_service_with_connect_info::<SocketAddr>(),
                    )
                    .await
                    {
                        tracing::error!("Axum server encountered an error: {}", e);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to convert std TcpListener to Tokio TcpListener: {}", e);
                }
            }
        });
    });

    // Main thread must handle native OS windows / event loop
    if !headless && !no_open {
        let manager = display_manager.expect("display_manager is Some whenever the desktop webview launches");
        let configured_outputs = os_next::webview::load_configured_outputs(&db);
        os_next::webview::launch_desktop_webview_sync(server_port, manager, configured_outputs, host_session_token, resolved_paths.data_dir());
    } else {
        tracing::info!("Running in headless mode. Press Ctrl+C to exit.");
        std::thread::park(); // Keep main thread alive
    }

    Ok(())
}
