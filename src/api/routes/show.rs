use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::http::StatusCode;
use axum::{Json, Router};
use serde::Serialize;

use crate::api::ws::AppState;
use crate::core::commands::ShowCommand;
use crate::core::models::ToScheduleItem;

use super::common::require_host_token_and_console_lock;

pub(super) async fn get_state(State(app): State<AppState>) -> impl IntoResponse {
    Json(app.engine.snapshot())
}

#[derive(Serialize)]
pub struct CommandResponse {
    pub success: bool,
    pub state: crate::core::engine::StateSnapshot,
    pub events: Vec<crate::core::events::EventEnvelope>,
    /// Set only on a rejection whose reason the caller should distinguish
    /// from a generic auth failure -- today just `"console_locked"` (see
    /// `require_host_token_and_console_lock`), so the frontend can show
    /// "another console is connected" instead of implying the token itself
    /// was wrong. `None` (omitted) on success and on ordinary auth failures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// If `item` has no explicit theme applied, auto-applies that item type's default
/// theme (Themes tab > right-click > Set as Default) — mirrors what a manual
/// drag-and-drop theme application does (ScheduleItemThemeSet in engine.rs),
/// matching EasyWorship's "randomly-added items render with the default design"
/// behavior. `item_type` is `ScheduleItem::item_type` ("song"/"scripture"/
/// "presentation"/"media"/"header" — only the first three have themes).
pub(crate) fn apply_default_theme_if_unset(item: &mut crate::core::models::ScheduleItem, db: &crate::storage::Database) {
    if item.theme_name.is_some() {
        return;
    }
    let category = match item.item_type.as_str() {
        "song" => "song",
        "scripture" => "scripture",
        "presentation" => "presentation",
        _ => return,
    };
    if let Ok(Some(theme)) = db.get_default_theme_for_category(category) {
        item.theme_name = Some(theme.name);
        item.background = Some(theme.background);
        for slide in item.slides.iter_mut() {
            slide.background = None;
        }
    }
}

pub(super) async fn post_command(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(mut value): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Paired remote-control clients (see /remote, paired via the Pairing
    // menu's QR code) must present a valid, non-revoked device token here.
    // A request with no `x-device-token` header at all is instead required
    // to present `x-host-token` plus `x-console-session-id` (see
    // `require_host_token_and_console_lock` below and
    // docs/CLIENT_PAIRING.md "Console (host) authentication" / "Remote
    // console access") — only the real operator console ever obtains the
    // host token, and the session id enforces "one console at a time."
    if let Some(token_header) = headers.get("x-device-token") {
        let token = token_header.to_str().unwrap_or("").trim();
        match app.db.get_paired_device_by_token(token) {
            Ok(Some(dev)) => {
                let _ = app.db.touch_paired_device(&dev.id);
            }
            _ => {
                tracing::warn!("REST command rejected: invalid/unknown device token");
                return Json(CommandResponse {
                    success: false,
                    state: app.engine.snapshot(),
                    events: vec![],
                    error: None,
                });
            }
        }
    } else if let Err((_, Json(err_body))) = require_host_token_and_console_lock(&headers, &app) {
        let reason = err_body.get("error").and_then(|v| v.as_str()).map(|s| s.to_string());
        tracing::warn!("REST command rejected: {}", reason.as_deref().unwrap_or("missing/invalid host token"));
        return Json(CommandResponse {
            success: false,
            state: app.engine.snapshot(),
            events: vec![],
            error: reason,
        });
    }

    // Intercept frontend payload: { "AddToSchedule": { "item_type": "...", "item_id": "..." } }
    if let Some(add_cmd) = value.get("AddToSchedule") {
        if add_cmd.get("item_type").is_some() && add_cmd.get("item_id").is_some() {
            let item_type = add_cmd["item_type"].as_str().unwrap_or("");
            let item_id = add_cmd["item_id"].as_str().unwrap_or("");

            let sched_item = match item_type {
                "song" => app.db.get_song_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
                "scripture" => app.db.get_scripture_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
                "media" => app.db.get_media_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
                "presentation" => app.db.get_presentation_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
                _ => None,
            };

            if let Some(mut item) = sched_item {
                apply_default_theme_if_unset(&mut item, &app.db);
                if let Ok(item_val) = serde_json::to_value(item) {
                    let mut new_cmd = serde_json::Map::new();
                    new_cmd.insert("AddToSchedule".to_string(), item_val);
                    value = serde_json::Value::Object(new_cmd);
                }
            } else {
                tracing::error!("Item not found in DB for AddToSchedule: {} {}", item_type, item_id);
            }
        }
    }

    // Intercept frontend payload: { "StageItem": { "item_type": "...", "item_id": "..." } }
    // — a Library Preview click staging an item that isn't in the schedule
    // yet, distinct from the real StageItem { item_index, slide_index } shape.
    if let Some(stage_cmd) = value.get("StageItem") {
        if stage_cmd.get("item_type").is_some() && stage_cmd.get("item_id").is_some() {
            let item_type = stage_cmd["item_type"].as_str().unwrap_or("");
            let item_id = stage_cmd["item_id"].as_str().unwrap_or("");

            let sched_item = match item_type {
                "song" => app.db.get_song_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
                "scripture" => app.db.get_scripture_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
                "media" => app.db.get_media_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
                "presentation" => app.db.get_presentation_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
                _ => None,
            };

            if let Some(item) = sched_item {
                if let Ok(item_val) = serde_json::to_value(item) {
                    let mut new_cmd = serde_json::Map::new();
                    new_cmd.insert("StageItemDirect".to_string(), item_val);
                    value = serde_json::Value::Object(new_cmd);
                }
            } else {
                tracing::error!("Item not found in DB for StageItem preview: {} {}", item_type, item_id);
            }
        }
    }

    let cmd_str = serde_json::to_string(&value).unwrap_or_default();
    if !app.plugin_manager.check_command(&cmd_str) {
        tracing::warn!("REST command blocked by plugin: {}", cmd_str);
        return Json(CommandResponse {
            success: false,
            state: app.engine.snapshot(),
            events: vec![],
            error: None,
        });
    }

    let cmd: ShowCommand = match serde_json::from_value(value.clone()) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to parse command: {} (Payload: {})", e, value);
            return Json(CommandResponse {
                success: false,
                state: app.engine.snapshot(),
                events: vec![],
                error: None,
            });
        }
    };

    let events = app.engine.execute_command(cmd);
    for env in &events {
        if let Err(e) = app.db.save_event(env) {
            tracing::warn!("Failed to persist event log entry #{}: {}", env.sequence_number, e);
        }
    }
    let state = crate::api::ws::broadcast_snapshot(&app);
    Json(CommandResponse {
        success: true,
        state,
        events,
        error: None,
    })
}

#[derive(Serialize)]
pub struct TimeResponse {
    pub server_time_ms: u64,
}

pub(super) async fn get_time() -> impl IntoResponse {
    Json(TimeResponse {
        server_time_ms: chrono::Utc::now().timestamp_millis() as u64,
    })
}

#[derive(Debug, Serialize)]
pub struct DisplayStateLive {
    pub title: String,
    pub subtitle: Option<String>,
    pub item_type: String,
    pub slide_index: usize,
    pub slide_count: usize,
    pub current_slide_text: String,
    pub next_slide_text: Option<String>,
    pub speaker_notes: Option<String>,
    /// "solid" | "image" | "video" — never raw CSS. A gradient is pre-reduced to its
    /// first stop's color (see `classify_background`) since Roku's LiveView only knows
    /// how to render a flat color, an image (`Poster`), or a video loop (`Video`).
    pub background_kind: String,
    pub background_value: String,
}

#[derive(Debug, Serialize)]
pub struct DisplayStateResponse {
    pub sequence_number: u64,
    pub server_time_ms: u64,
    pub is_blackout: bool,
    pub is_clear_text: bool,
    pub is_logo_override: bool,
    pub alert_message: Option<String>,
    /// `None` when nothing is live — a native client should show its idle/blackout state.
    pub live: Option<DisplayStateLive>,
    /// `None` unless a dedicated media (video) item is the live content — see
    /// `MediaPlaybackState` doc comment for how a client should project `current_time`
    /// forward using `timestamp_ms`/`start_at_epoch_ms`.
    pub media_playback: Option<crate::core::models::MediaPlaybackState>,
}

/// Lean, single-poll payload for native display clients that can't run the full web
/// frontend (Roku today; see apps/roku/) — covers both Live (FOH projection) and
/// Foldback (stage confidence monitor) needs from one endpoint so a client only needs
/// one poller. See docs/CLIENT_PAIRING.md for how such a client should identify and
/// pin to this console (via `/api/server-info`) before trusting this data.
pub(super) async fn get_display_state(State(app): State<AppState>) -> impl IntoResponse {
    let snapshot = app.engine.snapshot();
    let state = &snapshot.state;

    let live = state.live_item.as_ref().map(|item| {
        let play_pos = state.live_slide_index;
        let current = crate::core::engine::resolve_slide_at(item, play_pos);
        let next = crate::core::engine::resolve_slide_at(item, play_pos + 1);
        let slide_count = crate::core::engine::effective_arrangement(item).len();

        let raw_background = crate::core::engine::resolve_background_at(item, play_pos)
            .or(item.background.as_deref())
            .or(state.global_background.as_deref());
        let (background_kind, background_value) = classify_background(raw_background);

        DisplayStateLive {
            title: item.title.clone(),
            subtitle: item.subtitle.clone(),
            item_type: item.item_type.clone(),
            slide_index: play_pos,
            slide_count,
            current_slide_text: current.map(|s| s.text.clone()).unwrap_or_default(),
            next_slide_text: next.map(|s| s.text.clone()),
            speaker_notes: current.and_then(|s| s.speaker_notes.clone().or_else(|| s.notes.clone())),
            background_kind,
            background_value,
        }
    });

    Json(DisplayStateResponse {
        sequence_number: snapshot.sequence_number,
        server_time_ms: chrono::Utc::now().timestamp_millis() as u64,
        is_blackout: state.is_blackout,
        is_clear_text: state.is_clear_text,
        is_logo_override: state.is_logo,
        alert_message: state.alert_text.clone(),
        live,
        media_playback: state.media_playback.clone(),
    })
}

/// Classifies an already-resolved background string — as baked by the engine into
/// `Slide.background`/`ScheduleItem.background`/`ShowState.global_background` (see
/// `format_background_v2_to_css`) — into a `(kind, value)` pair a native client can act
/// on directly, since it can't interpret arbitrary CSS. Mirrors the precedence
/// `extractImageUrl`/`isVideoBackground`/`formatCssBackground` already use client-side
/// (web/src/app_core.ts) so a slide looks the same on every client.
pub(super) fn classify_background(raw: Option<&str>) -> (String, String) {
    const DEFAULT_SOLID: &str = "#102027"; // matches Theme::default_bg
    let trimmed = match raw.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(s) => s,
        None => return ("solid".to_string(), DEFAULT_SOLID.to_string()),
    };
    let lower = trimmed.to_lowercase();

    let is_video = lower.ends_with(".mp4")
        || lower.ends_with(".webm")
        || lower.ends_with(".mkv")
        || lower.ends_with(".mov")
        || lower.ends_with(".m4v")
        || lower.contains("/media/videos/");
    if is_video {
        return ("video".to_string(), trimmed.to_string());
    }

    if lower.starts_with("linear-gradient(") || lower.starts_with("radial-gradient(") {
        let color = extract_first_gradient_color(trimmed).unwrap_or_else(|| DEFAULT_SOLID.to_string());
        return ("solid".to_string(), color);
    }

    if lower.starts_with('#') || lower.starts_with("rgb(") || lower.starts_with("rgba(") {
        return ("solid".to_string(), trimmed.to_string());
    }

    if let Some(rel_start) = lower.find("url(") {
        if let Some(rel_end) = trimmed[rel_start..].find(')') {
            let inner = trimmed[rel_start + 4..rel_start + rel_end]
                .trim()
                .trim_matches(|c| c == '\'' || c == '"');
            if !inner.is_empty() {
                return ("image".to_string(), inner.to_string());
            }
        }
    }

    if lower.starts_with("data:image/") {
        return ("image".to_string(), trimmed.to_string());
    }

    let is_image = lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
        || lower.ends_with(".bmp")
        || lower.ends_with(".svg")
        || lower.contains("/media/images/")
        || lower.contains("/images/")
        || lower.contains("/backgrounds/");
    if is_image {
        return ("image".to_string(), trimmed.to_string());
    }

    // Unrecognized strings (named CSS colors, etc.) render as a flat color — a
    // reasonable v1 fallback rather than a hard failure.
    ("solid".to_string(), trimmed.to_string())
}

/// Pulls the first color stop out of a `linear-gradient(...)`/`radial-gradient(...)`
/// CSS string, skipping the leading angle/shape token when present. Gradients this app
/// generates are always in the controlled `"linear-gradient(<deg>deg, <color> <pct>%, ...)"`
/// shape (see `format_background_v2_to_css`), so a plain comma split is safe here even
/// though it wouldn't be for arbitrary user-authored CSS.
pub(super) fn extract_first_gradient_color(css: &str) -> Option<String> {
    let start = css.find('(')?;
    let end = css.rfind(')')?;
    if end <= start {
        return None;
    }
    for part in css[start + 1..end].split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let lower = part.to_lowercase();
        if lower.ends_with("deg") || lower == "circle" || lower == "ellipse" || lower.starts_with("to ") {
            continue;
        }
        let color = part.split_whitespace().next().unwrap_or(part);
        return Some(color.to_string());
    }
    None
}

/// This console's stable identity — the same instance_id advertised in the
/// mDNS TXT record (see src/discovery.rs). A future Roku/Android TV client
/// reads this on first connect and pins to it; see docs/CLIENT_PAIRING.md.
///
/// Deliberately zero-auth (any LAN client can read this) — it does NOT carry
/// `host_token` any more. It used to, which meant anyone on the LAN could GET
/// this one route and use the returned token for unrestricted keyring access.
/// The host token is now only ever handed to the actual console — see
/// `get_internal_host_token` below.
pub(super) async fn get_server_info(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let settings = app.db.get_settings().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let instance_id = crate::discovery::get_or_create_instance_id(&app.db);
    let name = settings.get("churchName").cloned().unwrap_or_default();
    Ok(Json(serde_json::json!({
        "instance_id": instance_id,
        "name": name,
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": 2,
        // Effective resolved paths (docs/paths.md) -- shown read-only in
        // Settings > Storage, and as the pre-filled starting value for the
        // bibles/songs/media directory overrides (empty until the operator
        // actually sets one).
        "data_dir": app.data_dir.to_string_lossy(),
        "bibles_dir": app.db.bibles_dir().map(|p| p.to_string_lossy().to_string()),
        "songs_dir": app.db.songs_dir().map(|p| p.to_string_lossy().to_string()),
        "media_dir": app.media_dir.to_string_lossy(),
        // Drives the first-time setup overlay (docs/first-time.md). A
        // settings flag rather than "did this process just create
        // library.db" -- the latter stays true for every page load across
        // that process's whole lifetime, not just the first one; this way
        // reloading mid-setup keeps showing it (expected) but finishing or
        // dismissing it suppresses it for good, across restarts too.
        "first_run": !app.skip_first_time_setup && !settings.contains_key("firstTimeSetupCompleted"),
        // Surfaced in the first-time setup wizard's "Data directory" step
        // (docs/first-time.md step 2, docs/paths.md "Legacy Import
        // Detection") -- an older portable/dev install's library.db sitting
        // next to the exe or in CWD that this run isn't already using.
        "legacy_library_detected": crate::storage::paths::detect_legacy_portable_library(&app.data_dir)
            .map(|p| p.to_string_lossy().to_string()),
    })))
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/state", get(get_state))
        .route("/api/command", post(post_command))
        .route("/api/time", get(get_time))
        .route("/api/server-info", get(get_server_info))
        .route("/api/display-state", get(get_display_state))
}

#[cfg(test)]
mod display_state_background_tests {
    use super::{classify_background, extract_first_gradient_color};

    #[test]
    fn classifies_video_by_extension_and_folder() {
        assert_eq!(classify_background(Some("clip.mp4")), ("video".to_string(), "clip.mp4".to_string()));
        assert_eq!(classify_background(Some("/media/videos/loop.MOV")), ("video".to_string(), "/media/videos/loop.MOV".to_string()));
    }

    #[test]
    fn classifies_solid_colors() {
        assert_eq!(classify_background(Some("#102030")), ("solid".to_string(), "#102030".to_string()));
        assert_eq!(classify_background(Some("rgba(10, 20, 30, 0.5)")), ("solid".to_string(), "rgba(10, 20, 30, 0.5)".to_string()));
    }

    #[test]
    fn classifies_wrapped_and_bare_images() {
        assert_eq!(
            classify_background(Some("url('/media/images/cross.jpg')")),
            ("image".to_string(), "/media/images/cross.jpg".to_string())
        );
        assert_eq!(classify_background(Some("/backgrounds/sunset.png")), ("image".to_string(), "/backgrounds/sunset.png".to_string()));
    }

    #[test]
    fn reduces_gradient_to_its_first_stop_color() {
        let (kind, value) = classify_background(Some("linear-gradient(135deg, #0f2027 0%, #203a43 50%, #2c5364 100%)"));
        assert_eq!(kind, "solid");
        assert_eq!(value, "#0f2027");
    }

    #[test]
    fn falls_back_to_default_solid_when_empty_or_none() {
        assert_eq!(classify_background(None), ("solid".to_string(), "#102027".to_string()));
        assert_eq!(classify_background(Some("   ")), ("solid".to_string(), "#102027".to_string()));
    }

    #[test]
    fn gradient_color_extraction_skips_leading_angle_token() {
        assert_eq!(extract_first_gradient_color("linear-gradient(90deg, #abcdef, #123456)"), Some("#abcdef".to_string()));
        assert_eq!(extract_first_gradient_color("radial-gradient(circle, #ff0000, #00ff00)"), Some("#ff0000".to_string()));
    }

    #[test]
    fn network_local_ip_resolves_or_degrades_gracefully() {
        let ip = crate::network::get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
        assert!(!ip.is_empty());
        assert_ne!(ip, "0.0.0.0");
    }
}
