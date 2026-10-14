//! Notification bus routes (see `api::notifications`).
//!
//! * `POST /api/plugin-host/notify`     a plugin granted `notifications.post` posts one
//! * `GET  /api/notifications`          console: what's new since `after` (optionally held open with `wait`)
//! * `POST /api/notifications/dismiss`  console: remove one by id
//! * `POST /api/notifications/clear`    console: remove all, or one source's

use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde::Deserialize;

use super::common::require_host_token;
use super::plugins::plugin_caller;
use crate::api::notifications::{Level, NewNotification};
use crate::api::ws::AppState;

type ApiErr = (StatusCode, Json<serde_json::Value>);

fn err(status: StatusCode, msg: &str) -> ApiErr {
    (status, Json(serde_json::json!({ "success": false, "error": msg })))
}

#[derive(Deserialize)]
pub(super) struct NotifyReq {
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    sticky: bool,
}

pub(super) async fn plugin_notify(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NotifyReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let caller = plugin_caller(&app, &headers).await?;
    if !caller.scopes.has("notifications.post") {
        return Err(err(StatusCode::FORBIDDEN, "this plugin wasn't granted notifications"));
    }
    let level = match req.level.as_deref() {
        None => None,
        Some(l) => Some(Level::parse(l).ok_or_else(|| err(StatusCode::BAD_REQUEST, "level must be info, success, warning or error"))?),
    };
    let name = app.db.get_plugin_grant(&caller.plugin_id).ok().flatten().map(|r| r.name).unwrap_or_else(|| caller.plugin_id.clone());
    let source = format!("plugin:{}", caller.plugin_id);
    let id = app
        .notifications
        .post(&source, &name, NewNotification { level, title: req.title, body: req.message, key: req.key, sticky: req.sticky })
        .map_err(|e| err(if e.contains("too fast") { StatusCode::TOO_MANY_REQUESTS } else { StatusCode::BAD_REQUEST }, &e))?;
    Ok(Json(serde_json::json!({ "success": true, "id": id })))
}

#[derive(Deserialize)]
pub(super) struct ListQuery {
    #[serde(default)]
    after: u64,
    /// Seconds to wait for something new (capped).
    #[serde(default)]
    wait: u64,
}

pub(super) async fn list(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    let items = app.notifications.since_wait(q.after, Duration::from_secs(q.wait.min(25))).await;
    Ok(Json(serde_json::json!({ "notifications": items, "latest": app.notifications.latest_id() })))
}

#[derive(Deserialize)]
pub(super) struct DismissReq {
    id: u64,
}

pub(super) async fn dismiss(State(app): State<AppState>, headers: HeaderMap, Json(req): Json<DismissReq>) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    Ok(Json(serde_json::json!({ "success": true, "removed": app.notifications.dismiss(req.id) })))
}

#[derive(Deserialize, Default)]
pub(super) struct ClearReq {
    #[serde(default)]
    source: Option<String>,
}

pub(super) async fn clear(State(app): State<AppState>, headers: HeaderMap, body: Option<Json<ClearReq>>) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    app.notifications.clear(body.and_then(|b| b.0.source).as_deref());
    Ok(Json(serde_json::json!({ "success": true })))
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/plugin-host/notify", post(plugin_notify))
        .route("/api/notifications", get(list))
        .route("/api/notifications/dismiss", post(dismiss))
        .route("/api/notifications/clear", post(clear))
}
