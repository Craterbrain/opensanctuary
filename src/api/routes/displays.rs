use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::api::ws::AppState;

#[derive(Serialize)]
struct DisplaysStateResponse {
    available: bool,
    monitors: Vec<crate::webview::MonitorInfo>,
    statuses: std::collections::HashMap<String, crate::webview::DisplayStatus>,
}

/// Physical-monitor/window management for native "Display" output windows
/// (borderless, always-on-top, one per configured output slot). Distinct from
/// `/api/display-state` above, which reports the current live SHOW CONTENT for
/// confidence-monitor consumers, not window/monitor state.
pub(super) async fn get_displays(State(app): State<AppState>) -> impl IntoResponse {
    match &app.display_manager {
        Some(mgr) if mgr.is_ready() => Json(DisplaysStateResponse {
            available: true,
            monitors: mgr.monitors(),
            statuses: mgr.statuses(),
        }),
        Some(_) => Json(DisplaysStateResponse { available: false, monitors: vec![], statuses: Default::default() }),
        None => Json(DisplaysStateResponse { available: false, monitors: vec![], statuses: Default::default() }),
    }
}

pub(super) async fn post_displays_refresh(State(app): State<AppState>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mgr = app.display_manager.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE, "Native desktop window is not running".to_string()))?;
    mgr.refresh_monitors().map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// POST /api/system/pick-folder
///
/// Shows a native "choose a folder" dialog from the operator console's own
/// window (see docs/paths.md's Settings > Storage directory overrides).
/// Only meaningful in the desktop-webview build with a window actually
/// running -- `available: false` in headless/browser-only mode tells the
/// frontend to fall back to a plain text path field instead of showing a
/// "Browse..." button that can't do anything.
pub(super) async fn pick_folder(State(app): State<AppState>) -> impl IntoResponse {
    let Some(mgr) = &app.display_manager else {
        return Json(serde_json::json!({ "available": false, "path": null }));
    };
    match mgr.pick_folder().await {
        Ok(path) => Json(serde_json::json!({ "available": true, "path": path })),
        Err(e) => {
            tracing::warn!("pick_folder failed: {}", e);
            Json(serde_json::json!({ "available": false, "path": null }))
        }
    }
}

/// POST /api/system/pick-file
///
/// Same as `pick_folder` but a single-file dialog -- used by the manual
/// "Install Update from File" flow (docs/update.md) to let an operator
/// select an update package they downloaded themselves.
pub(super) async fn pick_file(State(app): State<AppState>) -> impl IntoResponse {
    let Some(mgr) = &app.display_manager else {
        return Json(serde_json::json!({ "available": false, "path": null }));
    };
    match mgr.pick_file().await {
        Ok(path) => Json(serde_json::json!({ "available": true, "path": path })),
        Err(e) => {
            tracing::warn!("pick_file failed: {}", e);
            Json(serde_json::json!({ "available": false, "path": null }))
        }
    }
}

pub(super) async fn post_displays_open(
    State(app): State<AppState>,
    Json(req): Json<crate::webview::DisplayOpenRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mgr = app.display_manager.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE, "Native desktop window is not running".to_string()))?;
    mgr.open(req).map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(super) struct DisplayCloseRequest {
    id: String,
}

pub(super) async fn post_displays_close(
    State(app): State<AppState>,
    Json(req): Json<DisplayCloseRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mgr = app.display_manager.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE, "Native desktop window is not running".to_string()))?;
    mgr.close(req.id).map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::post;
    Router::new()
        .route("/api/displays", axum::routing::get(get_displays))
        .route("/api/displays/refresh", post(post_displays_refresh))
        .route("/api/displays/open", post(post_displays_open))
        .route("/api/displays/close", post(post_displays_close))
        .route("/api/system/pick-folder", post(pick_folder))
        .route("/api/system/pick-file", post(pick_file))
}
