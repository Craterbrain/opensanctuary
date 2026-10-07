use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::ws::AppState;
use crate::core::models::{MediaItem, Presentation, ScriptureItem, Song, Theme};

use super::common::{merge_with_plugin_resources, require_host_token, SearchQuery};

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
    Ok(Json(merge_with_plugin_resources(&app, "songs", songs, query.q.as_deref())))
}

pub(super) async fn create_song(
    State(app): State<AppState>,
    Json(song): Json<Song>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_song(&song).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(song))
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
    Ok(Json(merge_with_plugin_resources(&app, "scriptures", scriptures, query.q.as_deref())))
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
    let images_dir = app.media_dir.join("images");
    let _ = app.db.sync_media_folder(&images_dir);
    let videos_dir = app.media_dir.join("videos");
    let _ = app.db.sync_media_videos_folder(&videos_dir);
    let media = app.db.get_media().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(merge_with_plugin_resources(&app, "media", media, None)))
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

/// GET /api/media/online/search?q=<query>
///
/// Aggregates thumbnail results from every configured online image provider
/// (Pexels, Pixabay). API keys are read fresh from settings on each call —
/// a provider with no key configured is silently skipped, not an error.
pub(super) async fn search_online_media(
    State(app): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let q = query.q.unwrap_or_default();
    if q.trim().is_empty() {
        return Ok(Json(serde_json::json!({ "success": true, "results": Vec::<crate::storage::MediaSearchResult>::new() })));
    }
    let settings = app.db.get_settings().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut api_keys = std::collections::HashMap::new();

    for provider in &app.media_search.providers {
        let name = provider.provider_name();
        let capitalized = {
            let mut chars = name.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        };
        let key = crate::storage::KeyringService::get_secret(
            &format!("OpenSanctuary:Provider:{}", name),
            "api_key",
        )
        .ok()
        .flatten()
        .filter(|k| !k.trim().is_empty())
        .or_else(|| {
            crate::storage::KeyringService::get_secret(
                &format!("OpenSanctuary:{}", capitalized),
                "api_key",
            )
            .ok()
            .flatten()
            .filter(|k| !k.trim().is_empty())
        })
        .or_else(|| settings.get(&format!("{}ApiKey", name)).cloned());

        if let Some(k) = key {
            api_keys.insert(name.to_string(), k);
        }
    }
    let results = app.media_search.search_all(&q, &api_keys).await;
    Ok(Json(serde_json::json!({ "success": true, "results": results })))
}

#[derive(Deserialize)]
pub(super) struct ImportOnlineMediaReq {
    result: crate::storage::MediaSearchResult,
}

/// POST /api/media/online/import
///
/// Downloads the full-resolution image for a search result into the local
/// media library (`web/media/images/`) and inserts a MediaItem row for it,
/// so it behaves like any other locally-stored background from then on.
pub(super) async fn import_online_media(
    State(app): State<AppState>,
    Json(req): Json<ImportOnlineMediaReq>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let item = req.result;
    if !(item.full_url.starts_with("https://") || item.full_url.starts_with("http://")) {
        return Err((StatusCode::BAD_REQUEST, "full_url must be an http(s) URL".to_string()));
    }
    let client = reqwest::Client::builder()
        .user_agent("OpenSanctuary/1.0 (Church Presentation Engine; https://github.com/opensanctuary)")
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if is_private_host(attempt.url()) || attempt.previous().len() >= 3 { attempt.stop() } else { attempt.follow() }
        }))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Refuse direct requests to private/loopback hosts (LAN probing).
    match reqwest::Url::parse(&item.full_url) {
        Ok(u) if !is_private_host(&u) => {}
        _ => return Err((StatusCode::BAD_REQUEST, "full_url must be a valid public http(s) URL".to_string())),
    }

    let resp = client
        .get(&item.full_url)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Failed to download image: {}", e)))?
        .error_for_status()
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Image download failed: {}", e)))?;

    const MAX_ONLINE_MEDIA_BYTES: usize = 64 * 1024 * 1024;
    if resp.content_length().is_some_and(|n| n > MAX_ONLINE_MEDIA_BYTES as u64) {
        return Err((StatusCode::BAD_GATEWAY, "Remote file is too large".to_string()));
    }
    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))? {
        if buf.len() + chunk.len() > MAX_ONLINE_MEDIA_BYTES {
            return Err((StatusCode::BAD_GATEWAY, "Remote file is too large".to_string()));
        }
        buf.extend_from_slice(&chunk);
    }
    let bytes = buf;

    // Extension is derived from the downloaded bytes (not the URL, which a
    // caller controls) and `provider` is reduced to a safe file-name prefix,
    // so this can't plant `.html`/`.svg` or a path-escaping name under /media.
    let media_dir = app.media_dir.join("images");
    let provider_prefix: String = item.provider.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
    let provider_prefix = if provider_prefix.is_empty() { "online".to_string() } else { provider_prefix };
    let file_name = crate::storage::media_sniff::write_sniffed_media(&media_dir, &provider_prefix, &bytes)
        .ok_or((StatusCode::BAD_REQUEST, "Downloaded file is not a supported image or video".to_string()))?;
    let file_path = media_dir.join(&file_name);

    // Lead with tags/description when the provider gave us one — it's what
    // makes the item findable later via a plain local Media-library search
    // (which only matches name/file_path/media_type), not just its provider
    // credit. Falls back to the photographer credit, then a bare provider tag.
    let display_name = match item.tags.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(tags) => {
            let short_tags: String = tags.chars().take(60).collect();
            let ellipsis = if tags.chars().count() > 60 { "…" } else { "" };
            format!("{}{} ({} photo, {})", short_tags, ellipsis, item.provider, item.id)
        }
        None => item
            .photographer
            .as_deref()
            .map(|p| format!("{} photo by {} ({})", item.provider, p, item.id))
            .unwrap_or_else(|| format!("{} photo ({})", item.provider, item.id)),
    };

    let media_item = MediaItem {
        id: format!("{}_import_{}", item.provider, uuid::Uuid::new_v4()),
        name: display_name,
        media_type: "image".to_string(),
        file_path: format!("/media/images/{}", file_name),
        duration_seconds: None,
        thumbnail_path: None,
        loop_playback: false,
    };
    if let Err(e) = app.db.insert_media(&media_item) {
        let _ = std::fs::remove_file(&file_path); // don't leave an orphaned file if the DB row failed
        return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }

    Ok(Json(media_item))
}

pub(super) async fn get_themes(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let themes = app.db.get_themes().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(merge_with_plugin_resources(&app, "themes", themes, None)))
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
    Ok(Json(merge_with_plugin_resources(&app, "presentations", presentations, None)))
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

pub(super) fn router() -> Router<AppState> {
    use axum::routing::delete;
    use axum::routing::get;
    Router::new()
        .route("/api/songs", get(get_songs).post(create_song))
        .route("/api/songs/:id", delete(delete_song))
        .route("/api/scriptures", get(get_scriptures).post(create_scripture))
        .route("/api/scriptures/:id", delete(delete_scripture))
        .route("/api/media", get(get_media).post(create_media))
        .route("/api/media/:id", delete(delete_media))
        .route("/api/media/online/search", get(search_online_media))
        .route("/api/media/online/import", axum::routing::post(import_online_media))
        .route("/api/themes", get(get_themes).post(create_theme))
        .route("/api/themes/:name", delete(delete_theme))
        .route("/api/themes/default", axum::routing::post(set_default_theme))
        .route("/api/presentations", get(get_presentations).post(create_presentation))
        .route("/api/presentations/:id", delete(delete_presentation))
        .route("/api/slide-templates", get(get_slide_templates).post(create_slide_template))
        .route("/api/slide-templates/:id", delete(delete_slide_template))
}

/// True for localhost and private/loopback/link-local IP literals.
fn is_private_host(u: &reqwest::Url) -> bool {
    let Some(host) = u.host_str() else { return true };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Err(_) => false,
    }
}
