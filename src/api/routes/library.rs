use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::ws::AppState;
use crate::core::models::{MediaItem, Presentation, ScriptureItem, Song, Theme};

use super::common::{require_host_token, SearchQuery};

pub(super) async fn get_songs(
    State(app): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let songs = if let Some(q) = query.q.as_deref() {
        if !q.trim().is_empty() {
            app.db.search_songs(q).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        } else {
            app.db.get_songs().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        }
    } else {
        app.db.get_songs().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    Ok(Json(songs))
}

pub(super) async fn create_song(
    State(app): State<AppState>,
    Json(song): Json<Song>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_song(&song).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(song))
}

/// POST /api/songs/public-domain -- downloads the public-domain hymn library and adds the hymns the library doesn't have (never overwriting or duplicating a song).
pub(super) async fn post_public_domain_hymns(State(app): State<AppState>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let hymns = crate::storage::pd_songs::download().await.map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    let r = crate::storage::pd_songs::seed(&app.db, &hymns).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "success": true, "total": r.total, "added": r.added, "already_present": r.already_present })))
}

/// DELETE /api/licensed-scripture -- Settings > Storage > "Delete kept Bible passages": removes every licensed passage (the NIV and others
/// kept only for the publisher's time limit) now instead of waiting for them to expire. A schedule that names one just asks the plugin again.
pub(super) async fn delete_licensed_scripture(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let removed = app.db.purge_all_licensed().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "success": true, "removed": removed })))
}

pub(super) async fn delete_song(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    require_host_token(&headers, &app).map_err(|(status, _)| status)?;
    app.db.delete_song(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

pub(super) async fn get_scriptures(
    State(app): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let scriptures = if let Some(q) = query.q.as_deref() {
        if !q.trim().is_empty() {
            app.db.search_scriptures(q).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        } else {
            app.db.get_scriptures().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        }
    } else {
        app.db.get_scriptures().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    Ok(Json(scriptures))
}

pub(super) async fn create_scripture(
    State(app): State<AppState>,
    Json(scrip): Json<ScriptureItem>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_scripture(&scrip).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(scrip))
}

pub(super) async fn delete_scripture(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    require_host_token(&headers, &app).map_err(|(status, _)| status)?;
    app.db.delete_scripture(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

pub(super) async fn get_media(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    // This route is unauthenticated, and a folder rescan + DB writes on every hit
    // is a cheap DoS lever: rescan at most every few seconds, off the async workers.
    static LAST_SYNC: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    let due = {
        let mut last = LAST_SYNC.lock().unwrap_or_else(|e| e.into_inner());
        if last.map_or(true, |t| t.elapsed() >= std::time::Duration::from_secs(5)) {
            *last = Some(std::time::Instant::now());
            true
        } else {
            false
        }
    };
    if due {
        let db = app.db.clone();
        let images_dir = app.media_dir.join("images");
        let videos_dir = app.media_dir.join("videos");
        let _ = tokio::task::spawn_blocking(move || {
            let _ = db.sync_media_folder(&images_dir);
            let _ = db.sync_media_videos_folder(&videos_dir);
        })
        .await;
    }
    let media = app.db.get_media().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(media))
}

pub(super) async fn create_media(
    State(app): State<AppState>,
    Json(item): Json<MediaItem>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_media(&item).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(item))
}

pub(super) async fn delete_media(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    require_host_token(&headers, &app).map_err(|(status, _)| status)?;
    app.db.delete_media(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

pub(super) async fn get_themes(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let themes = app.db.get_themes().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(themes))
}

pub(super) async fn create_theme(
    State(app): State<AppState>,
    Json(theme): Json<Theme>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_theme(&theme).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(theme))
}

pub(super) async fn delete_theme(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(name): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    require_host_token(&headers, &app).map_err(|(status, _)| status)?;
    app.db.delete_theme(&name).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": name })))
}

#[derive(Deserialize)]
pub(super) struct SetDefaultThemeRequest {
    category: String,
    name: String,
}

pub(super) async fn set_default_theme(
    State(app): State<AppState>,
    Json(req): Json<SetDefaultThemeRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.set_default_theme(&req.category, &req.name).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let themes = app.db.get_themes().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(themes))
}

pub(super) async fn get_slide_templates(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let templates = app.db.get_slide_templates().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(templates))
}

pub(super) async fn create_slide_template(
    State(app): State<AppState>,
    Json(template): Json<crate::core::models::SlideTemplate>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_slide_template(&template).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(template))
}

pub(super) async fn delete_slide_template(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    require_host_token(&headers, &app).map_err(|(status, _)| status)?;
    app.db.delete_slide_template(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

pub(super) async fn get_presentations(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let presentations = app.db.get_presentations().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(presentations))
}

pub(super) async fn create_presentation(
    State(app): State<AppState>,
    Json(pres): Json<Presentation>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_presentation(&pres).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(pres))
}

pub(super) async fn delete_presentation(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    require_host_token(&headers, &app).map_err(|(status, _)| status)?;
    app.db.delete_presentation(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

/// GET /api/export/odp/:id -- a presentation as a real `.odp` package for the browser's Save
/// dialog to download directly, same shape as `/api/schedule/export`'s EWSX download.
pub(super) async fn export_presentation_odp(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    require_host_token(&headers, &app).map_err(|(status, body)| (status, body.0["error"].as_str().unwrap_or("unauthorized").to_string()))?;
    let pres = app
        .db
        .get_presentation_by_id(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "no such presentation".to_string()))?;
    let bytes = crate::storage::OdpExporter::export_presentation(&pres, &app.media_dir).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let filename = super::schedule::sanitize_download_filename(&pres.title);

    axum::http::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/vnd.oasis.opendocument.presentation")
        .header(axum::http::header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}.odp\"", filename))
        .body(axum::body::Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::delete;
    use axum::routing::get;
    Router::new()
        .route("/api/songs", get(get_songs).post(create_song))
        .route("/api/songs/public-domain", axum::routing::post(post_public_domain_hymns))
        .route("/api/licensed-scripture", delete(delete_licensed_scripture))
        .route("/api/songs/{id}", delete(delete_song))
        .route("/api/scriptures", get(get_scriptures).post(create_scripture))
        .route("/api/scriptures/{id}", delete(delete_scripture))
        .route("/api/media", get(get_media).post(create_media))
        .route("/api/media/{id}", delete(delete_media))
        .route("/api/themes", get(get_themes).post(create_theme))
        .route("/api/themes/{name}", delete(delete_theme))
        .route("/api/themes/default", axum::routing::post(set_default_theme))
        .route("/api/presentations", get(get_presentations).post(create_presentation))
        .route("/api/presentations/{id}", delete(delete_presentation))
        .route("/api/export/odp/{id}", get(export_presentation_odp))
        .route("/api/slide-templates", get(get_slide_templates).post(create_slide_template))
        .route("/api/slide-templates/{id}", delete(delete_slide_template))
}
