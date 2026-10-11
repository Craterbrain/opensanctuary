//! Pre-rendered animated background clips (see `storage::pattern_cache`).
//!
//! The console renders each frame of a pattern's loop in the browser, POSTs
//! them here in order, and the server pipes them through ffmpeg to a VP9
//! `.webm`. The live output asks `GET /api/pattern-cache/status` whether a clip
//! exists and, if so, plays it from `/media/pattern_cache/...` as a looping
//! video instead of redrawing the procedural pattern every frame.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::ws::AppState;
use crate::storage::pattern_cache::{self, ClipSpec, StartOutcome, Status};

#[derive(Deserialize)]
struct SpecQuery {
    key: String,
    rev: String,
    width: u32,
    height: u32,
    #[serde(default = "default_fps")]
    fps: u32,
}

fn default_fps() -> u32 {
    30
}

impl SpecQuery {
    fn spec(&self) -> ClipSpec {
        ClipSpec { key: self.key.clone(), rev: self.rev.clone(), width: self.width, height: self.height, fps: self.fps }
    }
}

#[derive(Deserialize)]
struct StartBody {
    key: String,
    rev: String,
    width: u32,
    height: u32,
    #[serde(default = "default_fps")]
    fps: u32,
    frames: u32,
}

fn err(e: (u16, String)) -> (StatusCode, String) {
    (StatusCode::from_u16(e.0).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), e.1)
}

/// GET /api/pattern-cache/status?key=&rev=&width=&height=&fps=
async fn get_status(State(app): State<AppState>, Query(q): Query<SpecQuery>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let spec = q.spec();
    spec.validate().map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(match pattern_cache::status(&app.media_dir, &spec).await {
        Status::Ready(url) => serde_json::json!({ "state": "ready", "url": url }),
        Status::Rendering => serde_json::json!({ "state": "rendering" }),
        Status::Missing => serde_json::json!({ "state": "missing" }),
    }))
}

#[derive(Deserialize)]
struct LookupQuery {
    key: String,
    rev: String,
}

/// GET /api/pattern-cache/lookup?key=&rev= -- best finished clip at any resolution.
async fn get_lookup(State(app): State<AppState>, Query(q): Query<LookupQuery>) -> impl IntoResponse {
    let media_dir = app.media_dir.clone();
    let url = tokio::task::spawn_blocking(move || pattern_cache::find_best(&media_dir, &q.key, &q.rev)).await.ok().flatten();
    Json(serde_json::json!({ "url": url }))
}

/// POST /api/pattern-cache/start -- begins a render (or reports it's cached/underway).
async fn post_start(State(app): State<AppState>, Json(b): Json<StartBody>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let spec = ClipSpec { key: b.key, rev: b.rev, width: b.width, height: b.height, fps: b.fps };
    Ok(Json(match pattern_cache::start(&app.media_dir, spec, b.frames).await.map_err(err)? {
        StartOutcome::Ready(url) => serde_json::json!({ "state": "ready", "url": url }),
        StartOutcome::AlreadyRendering => serde_json::json!({ "state": "rendering" }),
        StartOutcome::Started { job_id } => serde_json::json!({ "state": "started", "job_id": job_id }),
    }))
}

/// POST /api/pattern-cache/jobs/:id/frame -- raw PNG body, frames in order.
async fn post_frame(Path(id): Path<String>, body: Bytes) -> Result<impl IntoResponse, (StatusCode, String)> {
    let n = pattern_cache::push_frame(&id, &body).await.map_err(err)?;
    Ok(Json(serde_json::json!({ "frames": n })))
}

/// POST /api/pattern-cache/jobs/:id/finish
async fn post_finish(Path(id): Path<String>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let url = pattern_cache::finish(&id).await.map_err(err)?;
    Ok(Json(serde_json::json!({ "state": "ready", "url": url })))
}

/// DELETE /api/pattern-cache/jobs/:id -- cancel a render.
async fn delete_job(Path(id): Path<String>) -> impl IntoResponse {
    pattern_cache::abort(&id).await;
    Json(serde_json::json!({ "ok": true }))
}

/// GET /api/pattern-cache -- size of the clip cache and how long unused clips are kept.
async fn get_info(State(app): State<AppState>) -> impl IntoResponse {
    Json(pattern_cache::info(&app.media_dir))
}

/// DELETE /api/pattern-cache -- remove every cached clip.
async fn delete_all(State(app): State<AppState>) -> impl IntoResponse {
    let media_dir = app.media_dir.clone();
    let removed = tokio::task::spawn_blocking(move || pattern_cache::clear(&media_dir)).await.unwrap_or(0);
    Json(serde_json::json!({ "removed": removed }))
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/pattern-cache", get(get_info).delete(delete_all))
        .route("/api/pattern-cache/status", get(get_status))
        .route("/api/pattern-cache/lookup", get(get_lookup))
        .route("/api/pattern-cache/start", post(post_start))
        .route("/api/pattern-cache/jobs/:id/frame", post(post_frame))
        .route("/api/pattern-cache/jobs/:id/finish", post(post_finish))
        .route("/api/pattern-cache/jobs/:id", delete(delete_job))
}
