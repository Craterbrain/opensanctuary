use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// How often the event loop re-checks whether a pending display output's target
/// monitor has become available (e.g. a projector powered on after startup).
const MONITOR_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

/// A physical monitor as reported by the OS, indexed by its position in
/// `available_monitors()` at the moment of the last (re)enumeration. The index is
/// what callers pass back in `DisplayOpenRequest::monitor_index` — monitors can be
/// unplugged/replugged, so this is only stable within one enumeration, not across
/// hotplug events (a "Refresh Displays" re-enumerates and callers should re-pick).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub index: usize,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub is_primary: bool,
}

/// How large to make a display output window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DisplayResolution {
    /// True borderless fullscreen filling the target monitor's current resolution.
    Native,
    /// A fixed pixel size, positioned at the target monitor's top-left corner
    /// (still borderless/windowless) — useful when something downstream (a
    /// capture card, a streaming encoder's window-capture source) expects an
    /// exact resolution rather than "whatever this monitor happens to be".
    Fixed { width: u32, height: u32 },
}

/// A request to open one configured display output window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayOpenRequest {
    pub id: String,
    /// Path under the web root to load, e.g. "live.html" or "stage.html".
    pub content_path: String,
    pub monitor_index: usize,
    pub resolution: DisplayResolution,
    pub always_on_top: bool,
}

/// Current runtime state of one configured display output, tracked independent of
/// its persisted configuration — a slot can be configured but not currently open.
#[derive(Debug, Clone, Serialize, Default)]
pub struct DisplayStatus {
    pub open: bool,
    pub monitor_index: Option<usize>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Message sent from an Axum handler thread into the native event loop thread —
/// the only thread allowed to create/destroy `tao` windows.
#[derive(Debug, Clone)]
pub enum DisplayEvent {
    Open(DisplayOpenRequest),
    Close { id: String },
    RefreshMonitors,
}

/// Shared handle Axum handlers use to talk to the native event loop and read its
/// last-known state. `proxy` starts `None` and is filled in by
/// `launch_desktop_webview_sync` right after the event loop is created (before it
/// starts blocking in `run`), so a request that races startup just sees "not ready
/// yet" instead of panicking.
pub struct DisplayManagerHandle {
    proxy: Mutex<Option<tao::event_loop::EventLoopProxy<DisplayEvent>>>,
    monitors: Mutex<Vec<MonitorInfo>>,
    statuses: Mutex<HashMap<String, DisplayStatus>>,
}

impl DisplayManagerHandle {
    pub fn new() -> Self {
        Self {
            proxy: Mutex::new(None),
            monitors: Mutex::new(Vec::new()),
            statuses: Mutex::new(HashMap::new()),
        }
    }

    pub fn monitors(&self) -> Vec<MonitorInfo> {
        self.monitors.lock().unwrap().clone()
    }

    pub fn statuses(&self) -> HashMap<String, DisplayStatus> {
        self.statuses.lock().unwrap().clone()
    }

    /// `Some` only once the native event loop has actually started.
    pub fn is_ready(&self) -> bool {
        self.proxy.lock().unwrap().is_some()
    }

    pub fn open(&self, req: DisplayOpenRequest) -> Result<(), String> {
        self.send(DisplayEvent::Open(req))
    }

    pub fn close(&self, id: String) -> Result<(), String> {
        self.send(DisplayEvent::Close { id })
    }

    pub fn refresh_monitors(&self) -> Result<(), String> {
        self.send(DisplayEvent::RefreshMonitors)
    }

    fn send(&self, event: DisplayEvent) -> Result<(), String> {
        let guard = self.proxy.lock().unwrap();
        match guard.as_ref() {
            Some(proxy) => proxy
                .send_event(event)
                .map_err(|_| "Native display event loop has shut down".to_string()),
            None => Err("Native desktop window is still starting up".to_string()),
        }
    }
}

impl Default for DisplayManagerHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Mirrors the frontend's `DisplayOutputConfig` (web/src/core/display_config.ts) JSON
/// shape exactly, so the "displayOutputs" setting it writes can be read back here.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedDisplayOutput {
    id: String,
    content_path: String,
    monitor_index: usize,
    #[serde(default = "default_resolution_mode")]
    resolution_mode: String,
    fixed_width: Option<u32>,
    fixed_height: Option<u32>,
    #[serde(default)]
    always_on_top: bool,
}

fn default_resolution_mode() -> String {
    "native".to_string()
}

impl PersistedDisplayOutput {
    fn into_open_request(self) -> DisplayOpenRequest {
        let resolution = if self.resolution_mode == "fixed" {
            DisplayResolution::Fixed { width: self.fixed_width.unwrap_or(1920), height: self.fixed_height.unwrap_or(1080) }
        } else {
            DisplayResolution::Native
        };
        DisplayOpenRequest {
            id: self.id,
            content_path: self.content_path,
            monitor_index: self.monitor_index,
            resolution,
            always_on_top: self.always_on_top,
        }
    }
}

/// Reads the user-configured display output slots (Settings > Display) from the
/// generic settings key-value store and converts them into open requests, so the
/// desktop app can automatically open them on startup instead of requiring a manual
/// "Open" click every launch. Malformed/missing config just yields no auto-opens.
pub fn load_configured_outputs(db: &crate::storage::Database) -> Vec<DisplayOpenRequest> {
    let raw = match db.get_settings() {
        Ok(settings) => settings.get("displayOutputs").cloned().unwrap_or_default(),
        Err(_) => return Vec::new(),
    };
    if raw.trim().is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<Vec<PersistedDisplayOutput>>(&raw) {
        Ok(list) => list.into_iter().map(PersistedDisplayOutput::into_open_request).collect(),
        Err(e) => {
            tracing::warn!("Failed to parse 'displayOutputs' setting, skipping auto-open: {}", e);
            Vec::new()
        }
    }
}

fn enumerate_monitors(iter: impl Iterator<Item = tao::monitor::MonitorHandle>, primary: Option<tao::monitor::MonitorHandle>) -> Vec<MonitorInfo> {
    iter.enumerate()
        .map(|(index, m)| {
            let pos = m.position();
            let size = m.size();
            let is_primary = primary
                .as_ref()
                .map(|p| p.position() == pos && p.size() == size)
                .unwrap_or(index == 0);
            MonitorInfo {
                index,
                name: m.name().unwrap_or_else(|| format!("Display {}", index + 1)),
                width: size.width,
                height: size.height,
                x: pos.x,
                y: pos.y,
                is_primary,
            }
        })
        .collect()
}

#[cfg(feature = "desktop-webview")]
fn build_display_window(
    target: &tao::event_loop::EventLoopWindowTarget<DisplayEvent>,
    monitor: &tao::monitor::MonitorHandle,
    req: &DisplayOpenRequest,
) -> Option<(tao::window::Window, (u32, u32))> {
    // Deliberately not using tao's `Fullscreen::Borderless(Some(monitor))` here: that
    // request is applied before the GTK window is realized/mapped, and several window
    // managers then ignore the specific-monitor hint and fullscreen the window wherever
    // it currently sits (typically the primary monitor). Explicitly positioning an
    // undecorated, exactly-monitor-sized window instead sidesteps WM fullscreen-monitor
    // targeting entirely and is visually indistinguishable from real fullscreen.
    let reported_size = match &req.resolution {
        DisplayResolution::Native => {
            let s = monitor.size();
            (s.width, s.height)
        }
        DisplayResolution::Fixed { width, height } => (*width, *height),
    };

    let builder = tao::window::WindowBuilder::new()
        .with_title("OS-Next Display Output")
        .with_decorations(false)
        .with_always_on_top(req.always_on_top)
        .with_position(monitor.position())
        .with_inner_size(tao::dpi::PhysicalSize::new(reported_size.0, reported_size.1))
        .with_visible(true);

    match builder.build(target) {
        Ok(w) => Some((w, reported_size)),
        Err(e) => {
            tracing::error!("Failed to create display output window for '{}': {}", req.id, e);
            None
        }
    }
}

/// Tries to open one configured display output. If its target monitor isn't
/// connected yet, queues it in `pending_opens` instead of failing outright — the
/// event loop's periodic monitor poll (see `MONITOR_POLL_INTERVAL`) retries pending
/// opens on every tick, so plugging in a projector after startup (or after this was
/// requested) opens it automatically without any further user action.
#[cfg(all(feature = "desktop-webview", target_os = "linux"))]
fn try_open_display(
    target: &tao::event_loop::EventLoopWindowTarget<DisplayEvent>,
    monitors: &[tao::monitor::MonitorHandle],
    req: DisplayOpenRequest,
    port: u16,
    display_windows: &mut HashMap<String, (tao::window::Window, wry::WebView)>,
    display_manager: &DisplayManagerHandle,
    pending_opens: &mut Vec<DisplayOpenRequest>,
) {
    use tao::platform::unix::WindowExtUnix;
    use wry::WebViewBuilderExtUnix;

    let monitor = match monitors.get(req.monitor_index) {
        Some(m) => m,
        None => {
            tracing::warn!(
                "Display '{}': monitor index {} not yet available ({} monitor(s) detected) — will open automatically once connected",
                req.id, req.monitor_index, monitors.len()
            );
            pending_opens.push(req);
            return;
        }
    };

    if let Some((win, (w, h))) = build_display_window(target, monitor, &req) {
        let display_url = format!("http://127.0.0.1:{}/{}", port, req.content_path);
        let vbox = match win.default_vbox() {
            Some(v) => v,
            None => {
                tracing::error!("Display '{}': failed to get GTK vbox", req.id);
                return;
            }
        };
        match wry::WebViewBuilder::new_gtk(vbox).with_url(&display_url).build() {
            Ok(webview) => {
                display_manager.statuses.lock().unwrap().insert(
                    req.id.clone(),
                    DisplayStatus { open: true, monitor_index: Some(req.monitor_index), width: Some(w), height: Some(h) },
                );
                display_windows.insert(req.id, (win, webview));
            }
            Err(e) => tracing::error!("Display '{}': failed to build webview: {}", req.id, e),
        }
    }
}

#[cfg(all(feature = "desktop-webview", not(target_os = "linux")))]
fn try_open_display(
    target: &tao::event_loop::EventLoopWindowTarget<DisplayEvent>,
    monitors: &[tao::monitor::MonitorHandle],
    req: DisplayOpenRequest,
    port: u16,
    display_windows: &mut HashMap<String, (tao::window::Window, wry::WebView)>,
    display_manager: &DisplayManagerHandle,
    pending_opens: &mut Vec<DisplayOpenRequest>,
) {
    let monitor = match monitors.get(req.monitor_index) {
        Some(m) => m,
        None => {
            tracing::warn!(
                "Display '{}': monitor index {} not yet available ({} monitor(s) detected) — will open automatically once connected",
                req.id, req.monitor_index, monitors.len()
            );
            pending_opens.push(req);
            return;
        }
    };

    if let Some((win, (w, h))) = build_display_window(target, monitor, &req) {
        let display_url = format!("http://127.0.0.1:{}/{}", port, req.content_path);
        match wry::WebViewBuilder::new(&win).with_url(&display_url).build() {
            Ok(webview) => {
                display_manager.statuses.lock().unwrap().insert(
                    req.id.clone(),
                    DisplayStatus { open: true, monitor_index: Some(req.monitor_index), width: Some(w), height: Some(h) },
                );
                display_windows.insert(req.id, (win, webview));
            }
            Err(e) => tracing::error!("Display '{}': failed to build webview: {}", req.id, e),
        }
    }
}

#[cfg(all(feature = "desktop-webview", target_os = "linux"))]
pub fn launch_desktop_webview_sync(port: u16, display_manager: Arc<DisplayManagerHandle>, configured_outputs: Vec<DisplayOpenRequest>, host_session_token: String) {
    let url = format!("http://127.0.0.1:{}/", port);
    tracing::info!("Launching native desktop webview at {}", url);

    let event_loop = tao::event_loop::EventLoopBuilder::<DisplayEvent>::with_user_event().build();
    *display_manager.proxy.lock().unwrap() = Some(event_loop.create_proxy());

    let window = tao::window::WindowBuilder::new()
        .with_title("OS-Next Operator Console")
        .with_inner_size(tao::dpi::LogicalSize::new(1440.0, 900.0))
        .build(&event_loop)
        .expect("Failed to create tao window");

    use tao::platform::unix::WindowExtUnix;
    use wry::WebViewBuilderExtUnix;
    let vbox = window.default_vbox().expect("Failed to get vbox on GTK");

    // Hands this window's host session token directly to its own JS context
    // before any page script runs — never over HTTP/WS, so no network peer
    // (even a paired one) can ever observe it. This is what makes it safe
    // for this specific window to carry full keyring/pairing-management
    // trust: that trust is "you are this actual native process," not a
    // network-transmitted secret. See docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #2/#3
    // and `web/src/core/host_session.ts`.
    let init_script = format!("window.__OS_HOST_TOKEN__ = {:?};", host_session_token);

    let _webview = wry::WebViewBuilder::new_gtk(vbox)
        .with_url(&url)
        .with_initialization_script(&init_script)
        .with_devtools(true)
        .build()
        .expect("Failed to build wry webview on Linux");

    *display_manager.monitors.lock().unwrap() = enumerate_monitors(window.available_monitors(), window.primary_monitor());

    let mut display_windows: HashMap<String, (tao::window::Window, wry::WebView)> = HashMap::new();
    let mut pending_opens: Vec<DisplayOpenRequest> = Vec::new();
    let mut startup_outputs = Some(configured_outputs);
    let mut next_monitor_poll = std::time::Instant::now() + MONITOR_POLL_INTERVAL;

    event_loop.run(move |event, target, control_flow| {
        *control_flow = if pending_opens.is_empty() {
            tao::event_loop::ControlFlow::Wait
        } else {
            tao::event_loop::ControlFlow::WaitUntil(next_monitor_poll)
        };
        match event {
            tao::event::Event::NewEvents(tao::event::StartCause::Init) => {
                if let Some(outputs) = startup_outputs.take() {
                    let monitors: Vec<_> = target.available_monitors().collect();
                    for req in outputs {
                        try_open_display(target, &monitors, req, port, &mut display_windows, &display_manager, &mut pending_opens);
                    }
                }
            }
            tao::event::Event::MainEventsCleared => {
                if !pending_opens.is_empty() && std::time::Instant::now() >= next_monitor_poll {
                    let monitors: Vec<_> = target.available_monitors().collect();
                    *display_manager.monitors.lock().unwrap() = enumerate_monitors(target.available_monitors(), target.primary_monitor());
                    let retry = std::mem::take(&mut pending_opens);
                    for req in retry {
                        try_open_display(target, &monitors, req, port, &mut display_windows, &display_manager, &mut pending_opens);
                    }
                    next_monitor_poll = std::time::Instant::now() + MONITOR_POLL_INTERVAL;
                }
            }
            tao::event::Event::WindowEvent { window_id, event: tao::event::WindowEvent::CloseRequested, .. } => {
                if window_id == window.id() {
                    *control_flow = tao::event_loop::ControlFlow::Exit;
                } else if let Some(id) = display_windows.iter().find(|(_, (w, _))| w.id() == window_id).map(|(k, _)| k.clone()) {
                    display_windows.remove(&id);
                    display_manager.statuses.lock().unwrap().insert(id, DisplayStatus::default());
                }
            }
            tao::event::Event::UserEvent(DisplayEvent::Open(req)) => {
                let monitors: Vec<_> = target.available_monitors().collect();
                try_open_display(target, &monitors, req, port, &mut display_windows, &display_manager, &mut pending_opens);
            }
            tao::event::Event::UserEvent(DisplayEvent::Close { id }) => {
                display_windows.remove(&id);
                pending_opens.retain(|r| r.id != id);
                display_manager.statuses.lock().unwrap().insert(id, DisplayStatus::default());
            }
            tao::event::Event::UserEvent(DisplayEvent::RefreshMonitors) => {
                *display_manager.monitors.lock().unwrap() = enumerate_monitors(target.available_monitors(), target.primary_monitor());
            }
            _ => {}
        }
    });
}

#[cfg(all(feature = "desktop-webview", not(target_os = "linux")))]
pub fn launch_desktop_webview_sync(port: u16, display_manager: Arc<DisplayManagerHandle>, configured_outputs: Vec<DisplayOpenRequest>, host_session_token: String) {
    let url = format!("http://127.0.0.1:{}/", port);
    tracing::info!("Launching native desktop webview at {}", url);

    let event_loop = tao::event_loop::EventLoopBuilder::<DisplayEvent>::with_user_event().build();
    *display_manager.proxy.lock().unwrap() = Some(event_loop.create_proxy());

    let window = tao::window::WindowBuilder::new()
        .with_title("OS-Next Operator Console")
        .with_inner_size(tao::dpi::LogicalSize::new(1440.0, 900.0))
        .build(&event_loop)
        .expect("Failed to create tao window");

    // See the matching comment in the Linux/GTK branch above.
    let init_script = format!("window.__OS_HOST_TOKEN__ = {:?};", host_session_token);

    let _webview = wry::WebViewBuilder::new(&window)
        .with_url(&url)
        .with_initialization_script(&init_script)
        .with_devtools(true)
        .build()
        .expect("Failed to build wry webview on Windows/macOS");

    *display_manager.monitors.lock().unwrap() = enumerate_monitors(window.available_monitors(), window.primary_monitor());

    let mut display_windows: HashMap<String, (tao::window::Window, wry::WebView)> = HashMap::new();
    let mut pending_opens: Vec<DisplayOpenRequest> = Vec::new();
    let mut startup_outputs = Some(configured_outputs);
    let mut next_monitor_poll = std::time::Instant::now() + MONITOR_POLL_INTERVAL;

    event_loop.run(move |event, target, control_flow| {
        *control_flow = if pending_opens.is_empty() {
            tao::event_loop::ControlFlow::Wait
        } else {
            tao::event_loop::ControlFlow::WaitUntil(next_monitor_poll)
        };
        match event {
            tao::event::Event::NewEvents(tao::event::StartCause::Init) => {
                if let Some(outputs) = startup_outputs.take() {
                    let monitors: Vec<_> = target.available_monitors().collect();
                    for req in outputs {
                        try_open_display(target, &monitors, req, port, &mut display_windows, &display_manager, &mut pending_opens);
                    }
                }
            }
            tao::event::Event::MainEventsCleared => {
                if !pending_opens.is_empty() && std::time::Instant::now() >= next_monitor_poll {
                    let monitors: Vec<_> = target.available_monitors().collect();
                    *display_manager.monitors.lock().unwrap() = enumerate_monitors(target.available_monitors(), target.primary_monitor());
                    let retry = std::mem::take(&mut pending_opens);
                    for req in retry {
                        try_open_display(target, &monitors, req, port, &mut display_windows, &display_manager, &mut pending_opens);
                    }
                    next_monitor_poll = std::time::Instant::now() + MONITOR_POLL_INTERVAL;
                }
            }
            tao::event::Event::WindowEvent { window_id, event: tao::event::WindowEvent::CloseRequested, .. } => {
                if window_id == window.id() {
                    *control_flow = tao::event_loop::ControlFlow::Exit;
                } else if let Some(id) = display_windows.iter().find(|(_, (w, _))| w.id() == window_id).map(|(k, _)| k.clone()) {
                    display_windows.remove(&id);
                    display_manager.statuses.lock().unwrap().insert(id, DisplayStatus::default());
                }
            }
            tao::event::Event::UserEvent(DisplayEvent::Open(req)) => {
                let monitors: Vec<_> = target.available_monitors().collect();
                try_open_display(target, &monitors, req, port, &mut display_windows, &display_manager, &mut pending_opens);
            }
            tao::event::Event::UserEvent(DisplayEvent::Close { id }) => {
                display_windows.remove(&id);
                pending_opens.retain(|r| r.id != id);
                display_manager.statuses.lock().unwrap().insert(id, DisplayStatus::default());
            }
            tao::event::Event::UserEvent(DisplayEvent::RefreshMonitors) => {
                *display_manager.monitors.lock().unwrap() = enumerate_monitors(target.available_monitors(), target.primary_monitor());
            }
            _ => {}
        }
    });
}

#[cfg(not(feature = "desktop-webview"))]
pub fn launch_desktop_webview_sync(_port: u16, _display_manager: Arc<DisplayManagerHandle>, _configured_outputs: Vec<DisplayOpenRequest>, _host_session_token: String) {
    tracing::info!("Desktop webview feature disabled. Running in headless mode.");
}
