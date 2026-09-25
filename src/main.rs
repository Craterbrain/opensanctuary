use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use os_next::api::ws::AppState;
use os_next::api::create_router;
use os_next::core::engine::ShowEngine;
use os_next::core::event_log::EventLog;
use os_next::media::asset_graph::AssetGraph;
use os_next::storage::Database;

#[derive(Parser, Debug)]
#[command(name = "os-next", version = "0.2.0", about = "Next-Generation Church Presentation Engine in Rust")]
struct Args {
    #[arg(short, long, help = "Server listening port [default: 8080, auto-increments if in use]")]
    port: Option<u16>,

    #[arg(long, help = "Enable HTTPS admin & pairing server [default: true]")]
    https: Option<bool>,

    #[arg(long, help = "HTTPS listening port [default: 8443]")]
    https_port: Option<u16>,

    #[arg(long, default_value = "library.db")]
    db_path: String,

    #[arg(long)]
    bibles_dir: Option<String>,

    #[arg(long)]
    songs_dir: Option<String>,

    #[arg(long, default_value = "web")]
    web_dir: String,

    #[arg(long)]
    headless: bool,

    #[arg(long)]
    no_open: bool,
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

    let db = match (&args.bibles_dir, &args.songs_dir) {
        (Some(b_dir), Some(s_dir)) => Database::new_with_dirs(&args.db_path, b_dir, s_dir)?,
        (Some(b_dir), None) => {
            let base = std::path::Path::new(&args.db_path).parent().unwrap_or_else(|| std::path::Path::new("."));
            Database::new_with_dirs(&args.db_path, b_dir, base.join("songs"))?
        }
        (None, Some(s_dir)) => {
            let base = std::path::Path::new(&args.db_path).parent().unwrap_or_else(|| std::path::Path::new("."));
            Database::new_with_dirs(&args.db_path, base.join("bibles"), s_dir)?
        }
        (None, None) => Database::new(&args.db_path)?,
    };
    tracing::info!(
        "SQLite storage initialized with WAL mode at '{}' (bibles: {:?}, songs: {:?})",
        args.db_path,
        db.bibles_dir(),
        db.songs_dir()
    );

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

    #[cfg(target_os = "windows")]
    let plugin_dir = PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("OpenSanctuary/plugins");
    #[cfg(not(target_os = "windows"))]
    let plugin_dir = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/OpenSanctuary/plugins");
    let mut plugin_manager = os_next::core::plugins::PluginManager::new();
    if let Err(e) = plugin_manager.load_plugins_from_dir(&plugin_dir) {
        tracing::error!("Failed to load backend plugins: {}", e);
    } else {
        tracing::info!("Backend Plugin Manager initialized from {:?}", plugin_dir);
    }
    let plugin_manager = Arc::new(plugin_manager);

    let web_path = PathBuf::from(&args.web_dir);
    let images_dir = web_path.join("media").join("images");
    if let Err(e) = db.sync_media_folder(&images_dir) {
        tracing::warn!("Failed to sync media images folder {:?}: {}", images_dir, e);
    }
    let videos_dir = web_path.join("media").join("videos");
    if let Err(e) = db.sync_media_videos_folder(&videos_dir) {
        tracing::warn!("Failed to sync media videos folder {:?}: {}", videos_dir, e);
    }
    let asset_graph = Arc::new(AssetGraph::new(&web_path));

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

    let db_settings = db.get_settings().unwrap_or_default();
    let configured_port = args.port.or_else(|| {
        db_settings.get("networkPort").and_then(|p| p.parse::<u16>().ok())
    });

    // Bind TCP listener before spawning background thread or desktop webview
    let (std_listener, server_port) = match configured_port {
        Some(explicit_port) => {
            let addr = SocketAddr::from(([0, 0, 0, 0], explicit_port));
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
                let addr = SocketAddr::from(([0, 0, 0, 0], p));
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

    let https_requested = args.https.unwrap_or_else(|| {
        db_settings.get("httpsEnabled").map(|v| v != "false").unwrap_or(true)
    });
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

    let (https_port_actual, tls_pem_pair) = if https_requested {
        let interfaces = os_next::network::NetworkInterfaceInfo::discover_all();
        match os_next::network::tls::get_or_create_tls_certificate(&db, network_hostname, &interfaces) {
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
                    tracing::warn!("No available port for HTTPS between {} and {}, disabling HTTPS", configured_https_port, configured_https_port + 10);
                    (None, None)
                }
            }
            Err(e) => {
                tracing::warn!("Failed to get or create TLS certificate: {}, disabling HTTPS", e);
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    let host_session_token = uuid::Uuid::new_v4().to_string();
    let app_state = AppState {
        tx: tx.clone(),
        engine: engine.clone(),
        db: db.clone(),
        asset_graph: asset_graph.clone(),
        web_dir: web_path.clone(),
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
    println!("  Operator Console : http://127.0.0.1:{}/", server_port);
    if let Some(hp) = https_port_actual {
        println!("  Secure Console   : https://127.0.0.1:{}/", hp);
        println!("  Mobile Pairing   : https://127.0.0.1:{}/pairing.html", hp);
    }
    println!("  Live Display (FOH): http://127.0.0.1:{}/live.html (Zero-Auth)", server_port);
    println!("  Stage Foldback   : http://127.0.0.1:{}/stage.html (Zero-Auth)", server_port);
    println!("  WebSocket Stream : ws://127.0.0.1:{}/ws (Open Timecode)", server_port);
    println!("============================================================\n");

    // Run Axum server in a background Tokio runtime thread
    let graph_for_task = asset_graph.clone();
    let startup_broadcast_hostname = network_hostname.to_string();
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
                // (docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #5) — but it
                // shouldn't broadcast a colliding hostname with zero
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

            let router = create_router(app_state);
            let addr = SocketAddr::from(([0, 0, 0, 0], server_port));
            tracing::info!("OS-Next cleartext AV server listening on http://{}", addr);

            if let (Some(hp), Some((cert_pem, key_pem))) = (https_port_actual, tls_pem_pair) {
                match os_next::network::tls::create_rustls_config(&cert_pem, &key_pem).await {
                    Ok(rustls_config) => {
                        let https_addr = SocketAddr::from(([0, 0, 0, 0], hp));
                        let https_router = router.clone();
                        tokio::spawn(async move {
                            tracing::info!("OS-Next secure admin & pairing server listening on https://{}", https_addr);
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
                        tracing::error!("Failed to create Rustls config: {}", e);
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
        os_next::webview::launch_desktop_webview_sync(server_port, manager, configured_outputs, host_session_token);
    } else {
        tracing::info!("Running in headless mode. Press Ctrl+C to exit.");
        std::thread::park(); // Keep main thread alive
    }

    Ok(())
}
