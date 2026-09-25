

use axum::extract::{ConnectInfo, DefaultBodyLimit, Path as AxumPath, Query, Request, State};
use std::net::SocketAddr;
use axum::http::{
    header::{HeaderName, CACHE_CONTROL},
    HeaderMap, HeaderValue, StatusCode,
};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::api::ws::{ws_handler, AppState};
use crate::core::commands::ShowCommand;
use crate::core::models::{MediaItem, Presentation, ScriptureItem, Song, Theme};

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

/// Merges database-backed resource rows with whatever plugins contribute for the
/// same Resource Library `category` (see `OsPlugin::provide_resources`). When a
/// search `query` is active, plugin-contributed items are filtered by a simple
/// case-insensitive substring match over their serialized JSON — plugins return
/// plain `serde_json::Value`s of arbitrary shape, so this is the one filter that
/// works uniformly across all of them, mirroring the crude-but-effective substring
/// matching `PluginManager::check_command` already uses for the same reason.
fn merge_with_plugin_resources<T: Serialize>(
    app: &AppState,
    category: &str,
    items: Vec<T>,
    query: Option<&str>,
) -> Vec<serde_json::Value> {
    let mut combined: Vec<serde_json::Value> = items
        .into_iter()
        .filter_map(|item| serde_json::to_value(item).ok())
        .collect();
    let mut plugin_items = app.plugin_manager.collect_resources(category);
    if let Some(q) = query.filter(|q| !q.trim().is_empty()) {
        let q_lower = q.to_lowercase();
        plugin_items.retain(|item| {
            serde_json::to_string(item)
                .map(|s| s.to_lowercase().contains(&q_lower))
                .unwrap_or(false)
        });
    }
    combined.extend(plugin_items);
    combined
}



// --- Restored Importer Routes ---

use crate::storage::freeshow_import;
use crate::storage::genius_import::GeniusImporter;
use crate::storage::ytdlp_import::{YtDlpImporter, YtDlpDownloadOptions};

pub async fn get_bibles(State(app): State<AppState>) -> impl IntoResponse {
    let bibles = app.db.get_installed_bibles().unwrap_or_default();
    Json(bibles)
}

#[derive(Deserialize)]
pub struct ParseRefQuery {
    q: String,
}

pub async fn parse_bible_ref(Query(query): Query<ParseRefQuery>) -> impl IntoResponse {
    if let Some(parsed) = freeshow_import::parse_scripture_reference(&query.q) {
        Json(serde_json::json!({ "success": true, "parsed": parsed }))
    } else {
        Json(serde_json::json!({ "success": false }))
    }
}

#[derive(Deserialize)]
pub struct BiblePassageQuery {
    pub book: String,
    pub chapter: u32,
    pub version: String,
    pub verse_start: Option<u32>,
    pub verse_end: Option<u32>,
}

pub async fn get_bible_passage(
    State(app): State<AppState>,
    Query(query): Query<BiblePassageQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let version = query.version.trim();
    if version.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Missing or empty Bible version".to_string()));
    }
    let book = query.book.trim();
    if book.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Missing or empty book name".to_string()));
    }

    match app.db.get_passage(version, book, query.chapter) {
        Ok(Some(mut item)) => {
            if let Some(v_start) = query.verse_start {
                let v_end = query.verse_end.unwrap_or(v_start);
                item.verses.retain(|v| v.verse_number >= v_start && v.verse_number <= v_end);
                item.verse_start = v_start;
                item.verse_end = v_end;
                item.reference = format!("{} {}:{}-{} ({})", item.book, item.chapter, v_start, v_end, item.version);
            }
            Ok(Json(serde_json::json!({
                "success": true,
                "passage": item,
                "verses": item.verses
            })))
        }
        Ok(None) => Ok(Json(serde_json::json!({
            "success": false,
            "error": format!("Passage not found: {} {} in {}", book, query.chapter, version)
        }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e))),
    }
}

pub async fn delete_bible(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    match app.db.delete_installed_bible(&id) {
        Ok(true) => Ok(Json(serde_json::json!({ "success": true, "deleted": id }))),
        Ok(false) => Ok(Json(serde_json::json!({ "success": false, "error": format!("Bible translation '{}' not found", id) }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Database error deleting Bible: {}", e))),
    }
}

#[derive(Deserialize)]
pub struct GeniusSearchQuery {
    q: String,
}

pub async fn genius_search(Query(query): Query<GeniusSearchQuery>) -> impl IntoResponse {
    match GeniusImporter::search_christian_lyrics(&query.q).await {
        Ok(results) => Json(serde_json::json!({ "success": true, "results": results })),
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
pub struct GeniusImportReq {
    url: String,
    title: Option<String>,
    artist: Option<String>,
}

pub async fn genius_import(State(app): State<AppState>, Json(req): Json<GeniusImportReq>) -> impl IntoResponse {
    match GeniusImporter::fetch_and_import_lyrics(&req.url, req.title.as_deref(), req.artist.as_deref()).await {
        Ok(song) => {
            if let Err(e) = app.db.insert_song(&song) {
                tracing::warn!("Failed to persist Genius imported song '{}': {}", song.title, e);
            }
            Json(serde_json::json!({ "success": true, "song": song }))
        }
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

pub async fn ytdlp_download(State(app): State<AppState>, Json(options): Json<YtDlpDownloadOptions>) -> impl IntoResponse {
    let task_id = YtDlpImporter::start_background_download(options, app.web_dir.clone(), app.db.clone());
    Json(serde_json::json!({ "success": true, "task_id": task_id }))
}

pub async fn ytdlp_progress(AxumPath(task_id): AxumPath<String>) -> impl IntoResponse {
    if let Some(progress) = YtDlpImporter::get_progress(&task_id) {
        Json(serde_json::json!({ "success": true, "progress": progress }))
    } else {
        Json(serde_json::json!({ "success": false, "error": "Task not found" }))
    }
}



#[derive(Deserialize, Default)]
pub struct OpenLpImportReq {
    pub path: Option<String>,
    pub file_name: Option<String>,
    pub file_data_base64: Option<String>,
}

/// Import OpenLP SQLite databases (songs.sqlite or *.sqlite bibles) or auto-detect local OpenLP installation
pub async fn import_openlp(
    State(app): State<AppState>,
    Json(req): Json<OpenLpImportReq>,
) -> impl IntoResponse {
    let media_dir = app.web_dir.join("media").join("images");
    // 1. If payload contains base64 data or path, process the uploaded file
    if req.file_data_base64.is_some() || req.path.is_some() {
        let (bytes, stem) = if let Some(ref b64) = req.file_data_base64 {
            use base64::Engine;
            let decoded = match base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
                Ok(b) => b,
                Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Invalid base64: {}", e) }))),
            };
            let name_stem = req.file_name.as_deref()
                .and_then(|f| std::path::Path::new(f).file_stem().and_then(|s| s.to_str()))
                .unwrap_or("openlp_import");
            (decoded, name_stem.to_string())
        } else if let Some(ref p) = req.path {
            let val_path = match validate_safe_path(p) {
                Ok(path) => path,
                Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": e.1 }))),
            };
            let file_bytes = match std::fs::read(&val_path) {
                Ok(b) => b,
                Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Could not read path: {}", e) }))),
            };
            let name_stem = val_path.file_stem().and_then(|s| s.to_str()).unwrap_or("openlp_import").to_string();
            (file_bytes, name_stem)
        } else {
            unreachable!();
        };

        // Write bytes to temp file to query via rusqlite
        let mut tmp_file = match tempfile::NamedTempFile::new() {
            Ok(f) => f,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Tempfile creation failed: {}", e) }))),
        };
        if let Err(e) = std::io::Write::write_all(&mut tmp_file, &bytes) {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Failed writing bytes: {}", e) })));
        }

        let conn = match rusqlite::Connection::open(tmp_file.path()) {
            Ok(c) => c,
            Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Failed to open SQLite database: {}", e) }))),
        };

        // Inspect sqlite_master for songs or bible tables
        let has_songs: bool = conn.query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='songs';",
            [],
            |r| r.get(0),
        ).unwrap_or(false);

        let has_bible: bool = conn.query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='verse';",
            [],
            |r| r.get(0),
        ).unwrap_or(false);

        if has_songs {
            match crate::storage::OpenLPImporter::import_songs_db_with_media(tmp_file.path(), Some(&media_dir)) {
                Ok(songs) => {
                    let mut imported = 0;
                    for song in &songs {
                        if let Ok(()) = app.db.insert_song(song) {
                            imported += 1;
                        }
                    }
                    (StatusCode::OK, Json(serde_json::json!({
                        "success": true,
                        "type": "songs",
                        "count": imported,
                        "total": songs.len(),
                        "message": format!("Successfully imported {} songs from OpenLP database ({})", imported, stem)
                    })))
                }
                Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Error parsing OpenLP songs: {}", e) }))),
            }
        } else if has_bible {
            match crate::storage::OpenLPImporter::import_bible_db(tmp_file.path()) {
                Ok(scriptures) => {
                    if scriptures.is_empty() {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": "No verses found in OpenLP Bible database" })));
                    }
                    let version = scriptures[0].version.clone();
                    match app.db.insert_scriptures_batch(&scriptures) {
                        Ok(()) => (StatusCode::OK, Json(serde_json::json!({
                            "success": true,
                            "type": "bibles",
                            "count": scriptures.len(),
                            "version": version,
                            "message": format!("Successfully imported Bible '{}' ({} chapters) from OpenLP database", version, scriptures.len())
                        }))),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Database error saving Bible: {}", e) }))),
                    }
                }
                Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Error parsing OpenLP Bible: {}", e) }))),
            }
        } else {
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "success": false,
                "message": "Unrecognized OpenLP database format. Expected a songs database (with 'songs' table) or a Bible database (with 'verse' table)."
            })))
        }
    } else {
        // 2. No file provided — trigger auto-detection of local OpenLP installation
        match crate::storage::OpenLPImporter::detect_and_import_local(&app.db, Some(&media_dir)) {
            Ok(summary) => {
                let status = if summary.success { StatusCode::OK } else { StatusCode::NOT_FOUND };
                (status, Json(serde_json::json!(summary)))
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Auto-detect scan failed: {}", e) }))),
        }
    }
}

#[derive(Deserialize, Default)]
pub struct PptxImportReq {
    pub file_name: Option<String>,
    pub file_data_base64: String,
}

/// Import a PowerPoint `.pptx` file's slide text into the Presentations library.
/// Text-only fidelity (no layout/background/speaker-notes reconstruction) — see
/// `PptxImporter::import_pptx_bytes` doc comment for why.
pub async fn import_pptx(
    State(app): State<AppState>,
    Json(req): Json<PptxImportReq>,
) -> impl IntoResponse {
    use base64::Engine;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(req.file_data_base64.trim()) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Invalid base64: {}", e) }))),
    };
    let stem = req.file_name.as_deref()
        .and_then(|f| std::path::Path::new(f).file_stem().and_then(|s| s.to_str()))
        .unwrap_or("Imported Presentation")
        .to_string();

    let media_dir = app.web_dir.join("media").join("images");
    match crate::storage::PptxImporter::import_pptx_bytes(&bytes, &stem, &media_dir) {
        Ok(pres) => match app.db.insert_presentation(&pres) {
            Ok(()) => (StatusCode::OK, Json(serde_json::json!({
                "success": true,
                "id": pres.id,
                "title": pres.title,
                "slide_count": pres.slides.len(),
                "message": format!("Successfully imported '{}' ({} slides) from PowerPoint", pres.title, pres.slides.len())
            }))),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Database error saving presentation: {}", e) }))),
        },
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Error parsing .pptx: {}", e) }))),
    }
}

#[derive(Deserialize, Default)]
pub struct FreeShowShowImportReq {
    pub file_name: Option<String>,
    pub file_data_base64: String,
}

/// Import a FreeShow `.show` or JSON file into the library (Songs or Presentations).
pub async fn import_freeshow_show(
    State(app): State<AppState>,
    Json(req): Json<FreeShowShowImportReq>,
) -> impl IntoResponse {
    use base64::Engine;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(req.file_data_base64.trim()) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Invalid base64: {}", e) }))),
    };

    let media_dir = app.web_dir.join("media").join("images");
    match crate::storage::FreeShowShowImporter::import_show_bytes(&bytes, &media_dir) {
        Ok(sched) => {
            if let Some(item) = sched.items.first() {
                let is_song = item.item_type == "song";
                let slide_count = item.slides.len();
                let title = item.title.clone();
                let author = item.subtitle.clone().unwrap_or_else(|| "Unknown".to_string());

                if is_song {
                    let mut song = Song::new(&title, &author);
                    song.id = format!("song_{}", uuid::Uuid::new_v4());
                    song.slides = item.slides.clone();
                    if let Some(ref notes) = item.notes {
                        if let Some(ccli_str) = notes.strip_prefix("CCLI: ") {
                            song.ccli_number = Some(ccli_str.trim().to_string());
                        }
                    }

                    match app.db.insert_song(&song) {
                        Ok(()) => (StatusCode::OK, Json(serde_json::json!({
                            "success": true,
                            "id": song.id,
                            "title": song.title,
                            "type": "song",
                            "slide_count": slide_count,
                            "message": format!("Successfully imported song '{}' ({} slides) from FreeShow", song.title, slide_count)
                        }))),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Database error saving song: {}", e) }))),
                    }
                } else {
                    let pres = crate::core::models::Presentation {
                        id: format!("pres_{}", uuid::Uuid::new_v4()),
                        title: title.clone(),
                        author: author.clone(),
                        slides: item.slides.clone(),
                        theme_name: None,
                    };

                    match app.db.insert_presentation(&pres) {
                        Ok(()) => (StatusCode::OK, Json(serde_json::json!({
                            "success": true,
                            "id": pres.id,
                            "title": pres.title,
                            "type": "presentation",
                            "slide_count": slide_count,
                            "message": format!("Successfully imported presentation '{}' ({} slides) from FreeShow", pres.title, slide_count)
                        }))),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Database error saving presentation: {}", e) }))),
                    }
                }
            } else {
                (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": "No slides found in FreeShow file" })))
            }
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Error parsing FreeShow show: {}", e) }))),
    }
}

use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

type CachedCatalog = Option<(Instant, Vec<crate::storage::BibleCatalogEntry>)>;
static CATALOG_CACHE: LazyLock<RwLock<CachedCatalog>> = LazyLock::new(|| RwLock::new(None));

/// GET /api/bibles/online/catalog
///
/// Aggregates the catalog from ALL registered `BibleProvider` plugins.
/// Results are cached for 1 hour to avoid hammering upstream APIs.
pub async fn fsb_catalog(State(app): State<AppState>) -> impl IntoResponse {
    {
        let cache_read = CATALOG_CACHE.read().await;
        if let Some((fetched_at, ref catalog)) = *cache_read {
            if fetched_at.elapsed() < Duration::from_secs(3600) {
                return Json(serde_json::json!({ "success": true, "catalog": catalog }));
            }
        }
    }

    let mut combined: Vec<crate::storage::BibleCatalogEntry> = Vec::new();
    for provider in app.bible_providers.providers.iter() {
        match provider.catalog().await {
            Ok(mut entries) => combined.append(&mut entries),
            Err(e) => tracing::warn!(
                "BibleProvider '{}' catalog error: {}",
                provider.provider_name(),
                e
            ),
        }
    }

    let mut cache_write = CATALOG_CACHE.write().await;
    *cache_write = Some((Instant::now(), combined.clone()));
    Json(serde_json::json!({ "success": true, "catalog": combined }))
}

#[derive(Deserialize)]
pub struct DownloadBibleReq {
    pub translation: String,
    /// Which provider to use. Defaults to "churchapps".
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Provider-specific opaque key (e.g. raw GitHub download URL).
    pub source_key: Option<String>,
}

fn default_provider() -> String {
    "churchapps".to_string()
}

/// POST /api/bibles/online/download
///
/// Routes the download request to the correct `BibleProvider` plugin based on
/// the `provider` field in the request body.
pub async fn download_bible_full(
    State(app): State<AppState>,
    Json(req): Json<DownloadBibleReq>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let provider = app
        .bible_providers
        .get(&req.provider)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                format!("Unknown Bible provider: '{}'", req.provider),
            )
        })?;

    let items = provider
        .download_bible(&req.translation, req.source_key.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total_chapters = items.len();
    let total_verses: usize = items.iter().map(|i| i.verses.len()).sum();

    let db = app.db.clone();
    tokio::task::spawn_blocking(move || db.insert_scriptures_batch(&items))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Task join error: {}", e)))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "translation": req.translation.to_uppercase(),
        "provider": req.provider,
        "chapters_count": total_chapters,
        "verses_count": total_verses
    })))
}

#[derive(Deserialize)]
pub struct FsbFetchReq {
    pub query: Option<String>,
    pub translation: Option<String>,
}

pub async fn fsb_fetch(State(app): State<AppState>, Json(req): Json<FsbFetchReq>) -> impl IntoResponse {
    let q = req.query.as_deref().unwrap_or("");
    if q.is_empty() {
        return Json(serde_json::json!({ "success": false, "error": "No query provided" }));
    }

    match freeshow_import::FreeShowImporter::fetch_online_passage(q, req.translation.as_deref()).await {
        Ok(items) => {
            for item in &items {
                if let Err(e) = app.db.insert_scripture(item) {
                    tracing::warn!("Failed to persist scripture item '{}': {}", item.reference, e);
                }
            }
            Json(serde_json::json!({ "success": true, "items": items }))
        }
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

/// POST /api/bibles/github/catalog
///
/// Browse a user-supplied GitHub repo for Bible JSON files.
/// Request body: `{ "repo": "owner/repo", "versions_path": "versions" }`
pub async fn github_bible_catalog(
    Json(req): Json<crate::storage::GitHubRepoCatalogRequest>,
) -> impl IntoResponse {
    let provider = match req.into_provider() {
        Ok(p) => p,
        Err(e) => return Json(serde_json::json!({ "success": false, "error": e })),
    };

    match crate::storage::bible_providers::BibleProvider::catalog(&provider).await {
        Ok(entries) => Json(serde_json::json!({ "success": true, "catalog": entries })),
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}



/// Determines if an Origin header value represents an authorized sanctuary origin:
/// - localhost, 127.0.0.1, ::1
/// - Private RFC 1918 subnets (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
/// - Link-local (169.254.0.0/16 or fe80::/10)
/// - mDNS domains ending with .local
fn is_allowed_sanctuary_origin(origin_str: &str) -> bool {
    let host_with_port = if let Some(stripped) = origin_str.strip_prefix("http://") {
        stripped
    } else if let Some(stripped) = origin_str.strip_prefix("https://") {
        stripped
    } else {
        origin_str
    };

    let host = host_with_port.split('/').next().unwrap_or(host_with_port);
    let host = host.split(':').next().unwrap_or(host);

    if host == "localhost" || host == "127.0.0.1" || host == "::1" || host.ends_with(".local") {
        return true;
    }

    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        match ip {
            std::net::IpAddr::V4(ipv4) => {
                ipv4.is_loopback() || ipv4.is_private() || ipv4.is_link_local()
            }
            std::net::IpAddr::V6(ipv6) => {
                ipv6.is_loopback() || ipv6.is_unicast_link_local()
            }
        }
    } else {
        false
    }
}

/// Injects defense-in-depth security headers (X-Content-Type-Options, X-Frame-Options,
/// and Content-Security-Policy) based on the current persisted settings in AppState.db.
async fn security_headers_middleware(
    State(app): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let mut response = next.run(req).await;

    let csp_mode = app
        .db
        .get_setting("securityCspMode")
        .unwrap_or(None)
        .unwrap_or_else(|| "balanced".to_string());
    let frame_mode = app
        .db
        .get_setting("securityFrameOptions")
        .unwrap_or(None)
        .unwrap_or_else(|| "sameorigin".to_string());

    let headers = response.headers_mut();

    // 1. X-Content-Type-Options: nosniff
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );

    // 2. X-Frame-Options
    match frame_mode.as_str() {
        "deny" => {
            headers.insert(
                HeaderName::from_static("x-frame-options"),
                HeaderValue::from_static("DENY"),
            );
        }
        "disabled" => {
            // Omitted to permit all framing (e.g., OBS Studio browser sources)
        }
        _ => {
            // Default: sameorigin
            headers.insert(
                HeaderName::from_static("x-frame-options"),
                HeaderValue::from_static("SAMEORIGIN"),
            );
        }
    }

    // 3. Content-Security-Policy
    let frame_ancestor = match frame_mode.as_str() {
        "deny" => "frame-ancestors 'none';",
        "disabled" => "frame-ancestors *;",
        _ => "frame-ancestors 'self';",
    };

    match csp_mode.as_str() {
        "strict" => {
            let csp_val = format!(
                "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; media-src 'self' data: blob: https:; connect-src 'self' ws: wss: https:; font-src 'self' data: https:; object-src 'none'; base-uri 'self'; form-action 'self'; {}",
                frame_ancestor
            );
            if let Ok(val) = HeaderValue::from_str(&csp_val) {
                headers.insert(HeaderName::from_static("content-security-policy"), val);
            }
        }
        "disabled" => {
            // Omitted
        }
        _ => {
            // Default: balanced
            let csp_val = format!(
                "default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https: http:; media-src 'self' data: blob: https: http:; connect-src 'self' ws: wss: https: http:; font-src 'self' data: https:; object-src 'none'; base-uri 'self'; {}",
                frame_ancestor
            );
            if let Ok(val) = HeaderValue::from_str(&csp_val) {
                headers.insert(HeaderName::from_static("content-security-policy"), val);
            }
        }
    }

    response
}

pub fn create_router(app_state: AppState) -> Router {
    let db_for_cors = app_state.db.clone();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin, _parts| {
            let mode = db_for_cors
                .get_setting("securityCorsMode")
                .unwrap_or(None)
                .unwrap_or_else(|| "permissive".to_string());
            if mode == "restricted" {
                if let Ok(origin_str) = origin.to_str() {
                    is_allowed_sanctuary_origin(origin_str)
                } else {
                    false
                }
            } else {
                true
            }
        }))
        .allow_methods(Any)
        .allow_headers(Any);

    let web_dir = app_state.web_dir.clone();

    Router::new()
        // WebSocket
        .route("/ws", get(ws_handler))

        // State & Commands
        .route("/api/state", get(get_state))
        .route("/api/command", post(post_command))
        .route("/api/time", get(get_time))
        .route("/api/server-info", get(get_server_info))
        .route("/api/internal/host-token", get(get_internal_host_token))
        .route("/api/display-state", get(get_display_state))
        .route("/api/displays", get(get_displays))
        .route("/api/displays/refresh", post(post_displays_refresh))
        .route("/api/displays/open", post(post_displays_open))
        .route("/api/displays/close", post(post_displays_close))

        // Songs
        .route("/api/songs", get(get_songs).post(create_song))
        .route("/api/songs/:id", delete(delete_song))

        // Scriptures
        .route("/api/scriptures", get(get_scriptures).post(create_scripture))
        .route("/api/scriptures/:id", delete(delete_scripture))

        // Media
        .route("/api/media", get(get_media).post(create_media))
        .route("/api/media/:id", delete(delete_media))
        .route("/api/media/online/search", get(search_online_media))
        .route("/api/media/online/import", post(import_online_media))

        // Themes
        .route("/api/themes", get(get_themes).post(create_theme))
        .route("/api/themes/:name", delete(delete_theme))
        .route("/api/themes/default", post(set_default_theme))

        // Presentations
        .route("/api/presentations", get(get_presentations).post(create_presentation))
        .route("/api/presentations/:id", delete(delete_presentation))

        // Slide Templates (reusable author-designed layouts)
        .route("/api/slide-templates", get(get_slide_templates).post(create_slide_template))
        .route("/api/slide-templates/:id", delete(delete_slide_template))

        // Settings
        .route("/api/settings", get(get_settings).post(post_settings))

        // Keyring (OS-native credential vault)
        .route("/api/keyring/set", post(keyring_set))
        .route("/api/keyring/get", post(keyring_get))
        .route("/api/keyring/has", post(keyring_has))
        .route("/api/keyring/delete", post(keyring_delete))
        .route("/api/keyring/plugin/token", post(keyring_plugin_token))

        // Remote Control & Mobile Webpage
        .route("/remote", get(redirect_to_remote))
        .route("/api/pairing/remote-session", post(create_remote_pairing_handler))
        .route("/api/network/info", get(get_network_info))
        .route("/api/network/interfaces", get(get_network_interfaces_handler).post(post_network_interfaces_handler))
        .route("/api/network/check-port", get(check_port_handler))
        .route("/api/network/check-hostname", get(check_hostname_handler))
        .route("/api/network/dedicated-mac/toggle", post(toggle_dedicated_mac_handler))
        .route("/api/network/broadcast-option12", post(broadcast_option12_handler))
        .route("/api/network/tls/regenerate", post(regenerate_tls_cert_handler))

        // Client Pairing (Two-Way QR-Bridge for Android TV / Roku)
        .route("/api/pairing/session", post(create_pairing_session_handler))
        .route("/api/pairing/authorize", post(authorize_pairing_handler))
        .route("/api/pairing/status", get(get_pairing_status_handler))
        .route("/api/pairing/devices", get(get_paired_devices_handler))
        .route("/api/pairing/devices/:id", delete(delete_paired_device_handler))
        .route("/api/pairing/verify", post(verify_paired_device_handler))

        // Schedule Export / Import
        .route("/api/schedule/save", post(save_schedule))
        .route("/api/schedule/export", get(export_schedule_ewsx))
        .route("/api/schedule/open", post(open_schedule))
        .route("/api/bibles", get(get_bibles))
        .route("/api/bibles/:id", delete(delete_bible))
        .route("/api/bibles/passage", get(get_bible_passage))
        .route("/api/bibles/parse-ref", get(parse_bible_ref))
        .route("/api/import/openlp", post(import_openlp))
        .route("/api/import/pptx", post(import_pptx))
        .route("/api/import/freeshow-show", post(import_freeshow_show))
        .route("/api/bibles/online/catalog", get(fsb_catalog))
        .route("/api/bibles/online/fetch", post(fsb_fetch))
        .route("/api/bibles/online/download", post(download_bible_full))
        .route("/api/bibles/github/catalog", post(github_bible_catalog))
        .route("/api/songs/genius/search", get(genius_search))
        .route("/api/songs/genius/import", post(genius_import))
        .route("/api/media/ytdlp/download", post(ytdlp_download))
        .route("/api/media/ytdlp/progress/:task_id", get(ytdlp_progress))

        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(cors)
        .fallback_service(
            tower::layer::Layer::layer(
                &SetResponseHeaderLayer::overriding(
                    CACHE_CONTROL,
                    HeaderValue::from_static("no-cache, no-store, must-revalidate"),
                ),
                ServeDir::new(web_dir),
            )
        )
        .layer(from_fn_with_state(app_state.clone(), security_headers_middleware))
        .with_state(app_state)
}

// --- Handlers ---

async fn get_state(State(app): State<AppState>) -> impl IntoResponse {
    Json(app.engine.snapshot())
}

#[derive(Serialize)]
pub struct CommandResponse {
    pub success: bool,
    pub state: crate::core::engine::StateSnapshot,
    pub events: Vec<crate::core::events::EventEnvelope>,
}

use crate::core::models::ToScheduleItem;

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

async fn post_command(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(mut value): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Paired remote-control clients (see /remote, paired via the Pairing
    // menu's QR code) must present a valid, non-revoked device token here.
    // Requests with no `x-device-token` header at all (the operator
    // console's own commands) are unaffected — this only gates the surface
    // that's meant to require pairing.
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
                });
            }
        }
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
    })
}
#[derive(Serialize)]
pub struct TimeResponse {
    pub server_time_ms: u64,
}

async fn get_time() -> impl IntoResponse {
    Json(TimeResponse {
        server_time_ms: chrono::Utc::now().timestamp_millis() as u64,
    })
}

/// This console's stable identity — the same instance_id advertised in the
/// mDNS TXT record (see src/discovery.rs). A future Roku/Android TV client
/// reads this on first connect and pins to it; see docs/CLIENT_PAIRING.md.
///
/// Deliberately zero-auth (any LAN client can read this) — it does NOT carry
/// `host_token` any more. It used to, which meant anyone on the LAN could GET
/// this one route and use the returned token for unrestricted keyring access;
/// see docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #2. The host token is now only
/// ever handed to the actual console — see `get_internal_host_token` below.
async fn get_server_info(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let settings = app.db.get_settings().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let instance_id = crate::discovery::get_or_create_instance_id(&app.db);
    let name = settings.get("churchName").cloned().unwrap_or_default();
    Ok(Json(serde_json::json!({
        "instance_id": instance_id,
        "name": name,
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": 2,
    })))
}

/// GET /api/internal/host-token
///
/// The only way a web client can ever obtain `host_session_token` — and only
/// when the request's own TCP connection originates from loopback. A native
/// desktop webview gets the token a different way entirely (injected
/// in-process at window creation, never over the network — see
/// `with_initialization_script` in src/webview/mod.rs); this route exists
/// for the case of an operator using a plain browser tab pointed at
/// 127.0.0.1/localhost, the same address the console's own startup banner
/// already advertises. A LAN client — even a paired one — is never loopback
/// from the server's point of view, so this never reaches them.
async fn get_internal_host_token(
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse, StatusCode> {
    if !addr.ip().is_loopback() {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(Json(serde_json::json!({ "host_token": app.host_session_token })))
}

/// Shared gate for console-only management actions (minting pairing/remote
/// sessions, listing or revoking paired devices) — anything that either
/// mints new trust or exposes an existing secret/listing. Requires the
/// `x-host-token` header to match this server instance's host session
/// token, which only the actual console can ever obtain (see
/// `get_internal_host_token` above and `src/webview/mod.rs`).
fn require_host_token(headers: &HeaderMap, app: &AppState) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let ok = headers
        .get("x-host-token")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == app.host_session_token)
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "success": false, "error": "Missing or invalid x-host-token" })),
        ))
    }
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
async fn get_display_state(State(app): State<AppState>) -> impl IntoResponse {
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
fn classify_background(raw: Option<&str>) -> (String, String) {
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
fn extract_first_gradient_color(css: &str) -> Option<String> {
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

async fn get_songs(
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

async fn create_song(
    State(app): State<AppState>,
    Json(song): Json<Song>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_song(&song).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(song))
}

async fn delete_song(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.delete_song(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

async fn get_scriptures(
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

async fn create_scripture(
    State(app): State<AppState>,
    Json(scrip): Json<ScriptureItem>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_scripture(&scrip).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(scrip))
}

async fn delete_scripture(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.delete_scripture(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

async fn get_media(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let images_dir = app.web_dir.join("media").join("images");
    let _ = app.db.sync_media_folder(&images_dir);
    let videos_dir = app.web_dir.join("media").join("videos");
    let _ = app.db.sync_media_videos_folder(&videos_dir);
    let media = app.db.get_media().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(merge_with_plugin_resources(&app, "media", media, None)))
}

async fn create_media(
    State(app): State<AppState>,
    Json(item): Json<MediaItem>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_media(&item).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(item))
}

async fn delete_media(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.delete_media(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

/// GET /api/media/online/search?q=<query>
///
/// Aggregates thumbnail results from every configured online image provider
/// (Pexels, Pixabay). API keys are read fresh from settings on each call —
/// a provider with no key configured is silently skipped, not an error.
async fn search_online_media(
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
struct ImportOnlineMediaReq {
    result: crate::storage::MediaSearchResult,
}

/// POST /api/media/online/import
///
/// Downloads the full-resolution image for a search result into the local
/// media library (`web/media/images/`) and inserts a MediaItem row for it,
/// so it behaves like any other locally-stored background from then on.
async fn import_online_media(
    State(app): State<AppState>,
    Json(req): Json<ImportOnlineMediaReq>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let item = req.result;
    let client = reqwest::Client::builder()
        .user_agent("OpenSanctuary/1.0 (Church Presentation Engine; https://github.com/opensanctuary)")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let resp = client
        .get(&item.full_url)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Failed to download image: {}", e)))?
        .error_for_status()
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Image download failed: {}", e)))?;

    let ext = std::path::Path::new(&item.full_url)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.len() <= 5)
        .unwrap_or("jpg")
        .to_string();

    let bytes = resp.bytes().await.map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    let media_dir = app.web_dir.join("media").join("images");
    std::fs::create_dir_all(&media_dir).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let file_name = format!("{}_{}.{}", item.provider, uuid::Uuid::new_v4(), ext);
    let file_path = media_dir.join(&file_name);
    std::fs::write(&file_path, &bytes).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

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

async fn get_themes(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let themes = app.db.get_themes().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(merge_with_plugin_resources(&app, "themes", themes, None)))
}

async fn create_theme(
    State(app): State<AppState>,
    Json(theme): Json<Theme>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_theme(&theme).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(theme))
}

async fn delete_theme(
    State(app): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.delete_theme(&name).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": name })))
}

#[derive(Deserialize)]
struct SetDefaultThemeRequest {
    category: String,
    name: String,
}

async fn set_default_theme(
    State(app): State<AppState>,
    Json(req): Json<SetDefaultThemeRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.set_default_theme(&req.category, &req.name).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let themes = app.db.get_themes().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(themes))
}

async fn get_slide_templates(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let templates = app.db.get_slide_templates().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(templates))
}

async fn create_slide_template(
    State(app): State<AppState>,
    Json(template): Json<crate::core::models::SlideTemplate>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_slide_template(&template).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(template))
}

async fn delete_slide_template(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.delete_slide_template(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

async fn get_presentations(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let presentations = app.db.get_presentations().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(merge_with_plugin_resources(&app, "presentations", presentations, None)))
}

async fn create_presentation(
    State(app): State<AppState>,
    Json(pres): Json<Presentation>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.insert_presentation(&pres).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(pres))
}

async fn delete_presentation(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, StatusCode> {
    app.db.delete_presentation(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

async fn get_settings(State(app): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
    let settings = app.db.get_settings().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(settings))
}

async fn post_settings(
    State(app): State<AppState>,
    Json(mut settings): Json<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse, StatusCode> {
    // `networkHostname`/`networkPort` used to save unconditionally here even
    // though the read-only /api/network/check-hostname and /check-port
    // endpoints already compute whether the value collides with something
    // else on the network — the UI's warning badge had nothing enforcing it
    // server-side. See docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #5. A flagged
    // key is now held back (the rest of the payload still saves normally —
    // e.g. CSP/CORS settings bundled in the same request); the frontend can
    // resubmit with `_confirmOverrides` (comma-separated key names) once the
    // operator has explicitly acknowledged the warning it was already shown.
    let forced: std::collections::HashSet<String> = settings
        .remove("_confirmOverrides")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    let mut conflicts = serde_json::Map::new();

    if let Some(hostname) = settings.get("networkHostname").cloned() {
        if !forced.contains("networkHostname") {
            let self_ips = crate::network::get_all_local_ips();
            let result = crate::network::check_hostname_conflict(&hostname, &self_ips);
            if result.conflict_detected {
                settings.remove("networkHostname");
                conflicts.insert(
                    "networkHostname".to_string(),
                    serde_json::json!({
                        "conflicting_ip": result.conflicting_ip,
                        "suggested_hostname": result.suggested_hostname,
                        "message": result.message,
                    }),
                );
            }
        }
    }

    if let Some(port_str) = settings.get("networkPort").cloned() {
        if !forced.contains("networkPort") {
            if let Ok(port) = port_str.parse::<u16>() {
                let result = crate::network::check_port_conflict(port, Some(app.server_port));
                if !result.available {
                    settings.remove("networkPort");
                    conflicts.insert(
                        "networkPort".to_string(),
                        serde_json::json!({
                            "message": result.message,
                            "suggested_alternative": result.suggested_alternative,
                        }),
                    );
                }
            }
        }
    }

    for (k, v) in settings {
        if let Err(e) = app.db.set_setting(&k, &v) {
            tracing::warn!("Failed to persist setting key '{}': {}", k, e);
        }
    }
    let updated = app.db.get_settings().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut response = serde_json::to_value(&updated).unwrap_or_default();
    if !conflicts.is_empty() {
        if let Some(obj) = response.as_object_mut() {
            obj.insert("_conflicts".to_string(), serde_json::Value::Object(conflicts));
        }
    }
    Ok(Json(response))
}

enum KeyringCaller {
    Host,
    Plugin(String),
}

async fn authenticate_keyring_caller(
    headers: &HeaderMap,
    app: &AppState,
) -> Result<KeyringCaller, (StatusCode, Json<serde_json::Value>)> {
    if let Some(token_header) = headers.get("x-plugin-token") {
        if let Ok(token_str) = token_header.to_str() {
            if let Some(plugin_name) = app.resolve_plugin_token(token_str).await {
                return Ok(KeyringCaller::Plugin(plugin_name));
            }
        }
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "success": false, "error": "Invalid or expired plugin token" })),
        ));
    }

    if let Some(token_header) = headers.get("x-host-token") {
        if let Ok(token_str) = token_header.to_str() {
            if token_str == app.host_session_token {
                return Ok(KeyringCaller::Host);
            }
        }
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "success": false, "error": "Invalid host session token" })),
        ));
    }

    if !app.host_session_token.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "success": false, "error": "Missing authorization: x-host-token or x-plugin-token required" })),
        ));
    }

    Ok(KeyringCaller::Host)
}

#[derive(Deserialize)]
struct KeyringPluginTokenReq {
    plugin_name: String,
}

/// POST /api/keyring/plugin/token
/// Whether `name` matches a plugin file actually present under
/// `<web_dir>/plugins/` — the server's own source of truth for "what plugins
/// exist," rather than trusting whatever name a caller asserts. This closes
/// the specific exploit found in review (minting a token for a made-up name
/// like "planning_center" that was never installed at all) — see
/// docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #2.
///
/// It does NOT, on its own, stop a malicious script from claiming to be a
/// *real* installed plugin (e.g. "hello_world") — plugins execute in the
/// same JS realm as the host with no Worker/iframe boundary (see
/// `web/src/core/plugins.ts`), so there's no way for the server to verify
/// which script actually made the request without real realm isolation,
/// which is a larger change than this pass covers. Disclosed here rather
/// than implied fixed.
fn is_known_plugin_name(app: &AppState, name: &str) -> bool {
    let plugins_dir = app.web_dir.join("plugins");
    let Ok(entries) = std::fs::read_dir(&plugins_dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|stem| stem == name)
            .unwrap_or(false)
    })
}

async fn keyring_plugin_token(
    State(app): State<AppState>,
    Json(req): Json<KeyringPluginTokenReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let name = req.plugin_name.trim();
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "success": false, "error": "plugin_name must not be empty" })),
        ));
    }
    if !is_known_plugin_name(&app, name) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "success": false, "error": "No installed plugin with that name" })),
        ));
    }
    let token = app.issue_plugin_token(name).await;
    let service = format!("OpenSanctuary:Plugin:{}", name);
    Ok(Json(serde_json::json!({
        "success": true,
        "token": token,
        "service": service,
    })))
}

#[derive(Deserialize)]
struct KeyringSetReq {
    service: String,
    account: String,
    secret: String,
}

#[derive(Deserialize)]
struct KeyringQueryReq {
    service: String,
    account: String,
}

/// POST /api/keyring/set
async fn keyring_set(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<KeyringSetReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    if req.service.trim().is_empty() || req.account.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "success": false, "error": "service and account must not be empty" })),
        ));
    }

    let caller = authenticate_keyring_caller(&headers, &app).await?;
    let owner_tag = match &caller {
        KeyringCaller::Host => "host".to_string(),
        KeyringCaller::Plugin(name) => {
            let expected_service = format!("OpenSanctuary:Plugin:{}", name);
            if req.service.trim() != expected_service {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "success": false,
                        "error": format!("Plugins can only create secrets in their own namespace ('{}')", expected_service)
                    })),
                ));
            }
            format!("plugin:{}", name)
        }
    };

    // Check if an existing credential belongs to a different owner
    if let Ok(Some(existing_owner)) = app.db.get_keyring_owner(req.service.trim(), req.account.trim()) {
        if existing_owner != owner_tag {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "success": false,
                    "error": format!("Permission denied. Secret is owned by '{}'", existing_owner)
                })),
            ));
        }
    }

    crate::storage::KeyringService::set_secret(req.service.trim(), req.account.trim(), &req.secret)
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "success": false, "error": e })),
        ))?;

    let _ = app.db.record_keyring_owner(req.service.trim(), req.account.trim(), &owner_tag);

    Ok(Json(serde_json::json!({ "success": true })))
}

/// POST /api/keyring/get
async fn keyring_get(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<KeyringQueryReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    if req.service.trim().is_empty() || req.account.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "success": false, "error": "service and account must not be empty" })),
        ));
    }

    let caller = authenticate_keyring_caller(&headers, &app).await?;
    if let KeyringCaller::Plugin(name) = &caller {
        let expected_service = format!("OpenSanctuary:Plugin:{}", name);
        if req.service.trim() != expected_service {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "success": false,
                    "error": "Plugins can only view secrets in their own namespace"
                })),
            ));
        }
        let required_owner = format!("plugin:{}", name);
        let owner = app.db.get_keyring_owner(req.service.trim(), req.account.trim()).unwrap_or(None);
        if owner.as_deref() != Some(&required_owner) {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "success": false,
                    "error": "Plugins can only view secrets they created"
                })),
            ));
        }
    }

    let secret = crate::storage::KeyringService::get_secret(req.service.trim(), req.account.trim())
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "success": false, "error": e })),
        ))?;
    Ok(Json(serde_json::json!({ "success": true, "secret": secret })))
}

/// POST /api/keyring/has
async fn keyring_has(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<KeyringQueryReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    if req.service.trim().is_empty() || req.account.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "success": false, "error": "service and account must not be empty" })),
        ));
    }

    let caller = authenticate_keyring_caller(&headers, &app).await?;
    if let KeyringCaller::Plugin(name) = &caller {
        let expected_service = format!("OpenSanctuary:Plugin:{}", name);
        if req.service.trim() != expected_service {
            return Ok(Json(serde_json::json!({ "success": true, "exists": false })));
        }
        let required_owner = format!("plugin:{}", name);
        let owner = app.db.get_keyring_owner(req.service.trim(), req.account.trim()).unwrap_or(None);
        if owner.as_deref() != Some(&required_owner) {
            return Ok(Json(serde_json::json!({ "success": true, "exists": false })));
        }
    }

    let exists = crate::storage::KeyringService::has_secret(req.service.trim(), req.account.trim())
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "success": false, "error": e })),
        ))?;
    Ok(Json(serde_json::json!({ "success": true, "exists": exists })))
}

/// POST /api/keyring/delete
async fn keyring_delete(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<KeyringQueryReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    if req.service.trim().is_empty() || req.account.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "success": false, "error": "service and account must not be empty" })),
        ));
    }

    let caller = authenticate_keyring_caller(&headers, &app).await?;
    if let KeyringCaller::Plugin(name) = &caller {
        let expected_service = format!("OpenSanctuary:Plugin:{}", name);
        if req.service.trim() != expected_service {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "success": false,
                    "error": "Plugins can only delete secrets in their own namespace"
                })),
            ));
        }
        let required_owner = format!("plugin:{}", name);
        let owner = app.db.get_keyring_owner(req.service.trim(), req.account.trim()).unwrap_or(None);
        if owner.as_deref() != Some(&required_owner) {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "success": false,
                    "error": "Plugins can only delete secrets they created"
                })),
            ));
        }
    }

    let deleted = crate::storage::KeyringService::delete_secret(req.service.trim(), req.account.trim())
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "success": false, "error": e })),
        ))?;

    let _ = app.db.delete_keyring_metadata(req.service.trim(), req.account.trim());

    Ok(Json(serde_json::json!({ "success": true, "deleted": deleted })))
}

async fn redirect_to_remote() -> impl IntoResponse {
    Redirect::temporary("/remote.html")
}

/// POST /api/pairing/remote-session
///
/// Mints a real, persistent device token for a mobile remote client, using
/// the same `paired_devices` table and revoke UI (Settings > Paired
/// Devices) as TV/Roku pairing — but single-step, since a phone scanning
/// the console's own QR code doesn't need the two-way session bridge that
/// exists to link a *separate* TV device to a phone. Called by the
/// console's Pairing menu each time the "Mobile Remote" QR is shown; the
/// resulting token is embedded in that QR's URL and the client presents it
/// on every command it sends (see `/remote`, `web/src/remote_client.ts`).
async fn create_remote_pairing_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Console-only — otherwise any LAN client could mint itself a live
    // remote-control token without ever touching the console or scanning a
    // QR. See docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #3.
    require_host_token(&headers, &app)?;
    let device_id = format!("remote_{}", uuid::Uuid::new_v4().simple());
    let dev_token = format!("dev_tok_{}", uuid::Uuid::new_v4().simple());
    let now = chrono::Utc::now().timestamp_millis();
    let device = crate::storage::PairedDevice {
        id: device_id.clone(),
        name: "Mobile Remote".to_string(),
        platform: "mobile-remote".to_string(),
        paired_at: now,
        last_seen: now,
        token: dev_token.clone(),
    };
    if let Err(e) = app.db.upsert_paired_device(&device) {
        return Ok(Json(serde_json::json!({
            "success": false,
            "error": format!("Database error: {}", e)
        })));
    }
    Ok(Json(serde_json::json!({
        "success": true,
        "device_id": device_id,
        "token": dev_token,
    })))
}

// --- Two-Way QR-Bridge Client Pairing Handlers ---

#[derive(Serialize)]
pub struct PairingSessionResponse {
    pub session_token: String,
    pub expires_in: u64,
}

pub async fn create_pairing_session_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Only the console itself mints pairing sessions — "operator clicks Pair
    // TV App" in the design (docs/CLIENT_PAIRING.md). Without this, any LAN
    // client could mint its own session and self-pair a rogue device; see
    // docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #3. `authorize_pairing_handler`
    // below stays open — it's called by the technician's *phone*, a third
    // device that was never the console — but it's gated in practice by
    // needing a session_token that only a host-authenticated console could
    // have minted in the first place.
    require_host_token(&headers, &app)?;
    let ttl = 300; // 5 minutes validity
    let session_token = app.create_pairing_session(ttl).await;
    Ok(Json(PairingSessionResponse {
        session_token,
        expires_in: ttl,
    }))
}

#[derive(Deserialize)]
pub struct AuthorizeDeviceRequest {
    pub session_token: String,
    pub device_id: String,
    pub name: String,
    pub platform: String,
}

pub async fn authorize_pairing_handler(
    State(app): State<AppState>,
    Json(req): Json<AuthorizeDeviceRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let is_valid = app.validate_pairing_session(&req.session_token).await;
    if !is_valid {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "Invalid or expired pairing session token"
            })),
        ));
    }

    let device_id = req.device_id.trim();
    if device_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Device ID cannot be empty"
            })),
        ));
    }

    let dev_token = format!("dev_tok_{}", uuid::Uuid::new_v4().simple());
    let now = chrono::Utc::now().timestamp_millis();
    let device = crate::storage::PairedDevice {
        id: device_id.to_string(),
        name: if req.name.trim().is_empty() { "Sanctuary Display".to_string() } else { req.name.trim().to_string() },
        platform: if req.platform.trim().is_empty() { "android-tv".to_string() } else { req.platform.trim().to_string() },
        paired_at: now,
        last_seen: now,
        token: dev_token.clone(),
    };

    if let Err(e) = app.db.upsert_paired_device(&device) {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Database error: {}", e)
            })),
        ));
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "device_id": device.id,
        "name": device.name,
        "platform": device.platform,
        "token": dev_token,
    })))
}

#[derive(Deserialize)]
pub struct PairingStatusQuery {
    pub device_id: String,
}

pub async fn get_pairing_status_handler(
    State(app): State<AppState>,
    Query(query): Query<PairingStatusQuery>,
) -> impl IntoResponse {
    match app.db.get_paired_device(&query.device_id) {
        Ok(Some(dev)) => {
            let _ = app.db.touch_paired_device(&query.device_id);
            Json(serde_json::json!({
                "paired": true,
                "device_id": dev.id,
                "name": dev.name,
                "token": dev.token,
            }))
        }
        _ => Json(serde_json::json!({
            "paired": false,
            "device_id": query.device_id,
            "name": null,
            "token": null,
        })),
    }
}

pub async fn get_paired_devices_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Returns every paired device's permanent bearer token — console-only.
    // See docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #3.
    require_host_token(&headers, &app)?;
    let devices = app.db.get_paired_devices().unwrap_or_default();
    Ok(Json(devices))
}

pub async fn delete_paired_device_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Console-only — otherwise any LAN client could revoke a live sanctuary
    // display mid-service. See docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #3.
    require_host_token(&headers, &app)?;
    Ok(match app.db.delete_paired_device(&id) {
        Ok(deleted) => Json(serde_json::json!({ "success": deleted })),
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    })
}

#[derive(Deserialize)]
pub struct VerifyDeviceRequest {
    pub token: String,
}

pub async fn verify_paired_device_handler(
    State(app): State<AppState>,
    Json(req): Json<VerifyDeviceRequest>,
) -> impl IntoResponse {
    match app.db.get_paired_device_by_token(&req.token) {
        Ok(Some(dev)) => {
            let _ = app.db.touch_paired_device(&dev.id);
            Json(serde_json::json!({
                "valid": true,
                "device_id": dev.id,
                "name": dev.name,
                "platform": dev.platform,
            }))
        }
        _ => Json(serde_json::json!({
            "valid": false,
        })),
    }
}


/// GET /api/network/info
/// Returns host network LAN IP, port, remote URL, and mDNS URL for phone QR scanning.
async fn get_network_info(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let dedicated = crate::network::VirtualAdapterManager::get_dedicated_adapter_info();
    let (lan_ip, is_dedicated) = if let Some(ref d) = dedicated {
        (
            d.ipv4
                .clone()
                .unwrap_or_else(|| get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())),
            true,
        )
    } else {
        (
            get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string()),
            false,
        )
    };

    let http_port = app.server_port;
    let https_port = app.https_port;
    let https_enabled = app.https_enabled;

    let port = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(extract_port_from_host_header)
        .unwrap_or(app.server_port);

    let db_settings = app.db.get_settings().unwrap_or_default();
    let hostname = db_settings
        .get("networkHostname")
        .cloned()
        .unwrap_or_else(|| "opensanctuary".to_string());
    let clean_hostname = crate::network::sanitize_rfc1035_hostname(&hostname);

    let remote_url = format!("http://{}:{}/remote", lan_ip, http_port);
    let mdns_url = format!("http://{}.local:{}/remote", clean_hostname, http_port);

    let pairing_url = if https_enabled && https_port.is_some() {
        format!("https://{}:{}/pairing.html", lan_ip, https_port.unwrap())
    } else {
        format!("http://{}:{}/pairing.html", lan_ip, http_port)
    };

    let mdns_pairing_url = if https_enabled && https_port.is_some() {
        format!("https://{}.local:{}/pairing.html", clean_hostname, https_port.unwrap())
    } else {
        format!("http://{}.local:{}/pairing.html", clean_hostname, http_port)
    };

    Json(serde_json::json!({
        "success": true,
        "lan_ip": lan_ip,
        "port": port,
        "http_port": http_port,
        "https_port": https_port,
        "https_enabled": https_enabled,
        "remote_url": remote_url,
        "mdns_url": mdns_url,
        "pairing_url": pairing_url,
        "mdns_pairing_url": mdns_pairing_url,
        "hostname": hostname,
        "is_dedicated": is_dedicated,
        "dedicated_mac": dedicated.and_then(|d| d.mac_address)
    }))
}

fn extract_port_from_host_header(host: &str) -> Option<u16> {
    if host.ends_with(']') {
        return None;
    }
    let idx = host.rfind(':')?;
    if let Some(bracket_idx) = host.rfind(']') {
        if idx < bracket_idx {
            return None;
        }
    }
    host[idx + 1..].parse::<u16>().ok()
}

fn get_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    let ip = addr.ip().to_string();
    if ip == "0.0.0.0" {
        None
    } else {
        Some(ip)
    }
}

/// GET /api/network/interfaces
/// Returns all network interfaces, enabled states, and dedicated adapter status.
async fn get_network_interfaces_handler(State(app): State<AppState>) -> impl IntoResponse {
    let db_settings = app.db.get_settings().unwrap_or_default();
    let enabled_filter_str = db_settings.get("networkEnabledInterfaces");
    let enabled_filter: Option<Vec<String>> = enabled_filter_str.map(|s| {
        s.split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect()
    });

    let interfaces = crate::network::get_network_interfaces(enabled_filter.as_deref());
    let broadcast_all = db_settings
        .get("networkBroadcastAll")
        .map(|v| v != "false")
        .unwrap_or(true);
    let hostname = db_settings
        .get("networkHostname")
        .cloned()
        .unwrap_or_else(|| "opensanctuary".to_string());
    let dedicated_adapter = crate::network::VirtualAdapterManager::get_dedicated_adapter_info();
    let is_dedicated_active = crate::network::VirtualAdapterManager::is_dedicated_adapter_active();
    let https_port = app.https_port;
    let https_enabled = app.https_enabled;

    Json(serde_json::json!({
        "success": true,
        "interfaces": interfaces,
        "broadcast_all": broadcast_all,
        "hostname": hostname,
        "port": app.server_port,
        "http_port": app.server_port,
        "https_port": https_port,
        "https_enabled": https_enabled,
        "dedicated_active": is_dedicated_active,
        "dedicated_adapter": dedicated_adapter
    }))
}

#[derive(Deserialize)]
struct SaveInterfacesRequest {
    enabled_interfaces: Vec<String>,
    broadcast_all: Option<bool>,
}

/// POST /api/network/interfaces
/// Saves enabled interfaces and broadcast_all preference.
async fn post_network_interfaces_handler(
    State(app): State<AppState>,
    Json(payload): Json<SaveInterfacesRequest>,
) -> impl IntoResponse {
    let joined = payload.enabled_interfaces.join(",");
    let _ = app.db.set_setting("networkEnabledInterfaces", &joined);
    if let Some(b_all) = payload.broadcast_all {
        let _ = app
            .db
            .set_setting("networkBroadcastAll", if b_all { "true" } else { "false" });
    }
    Json(serde_json::json!({ "success": true }))
}

#[derive(Deserialize)]
struct CheckPortQuery {
    port: u16,
}

/// GET /api/network/check-port?port=8080
/// Checks if a port is available, accounting for the current running server port.
async fn check_port_handler(
    State(app): State<AppState>,
    Query(query): Query<CheckPortQuery>,
) -> impl IntoResponse {
    let result = crate::network::check_port_conflict(query.port, Some(app.server_port));
    Json(serde_json::json!({
        "success": true,
        "port": result.port,
        "available": result.available,
        "in_use_by_current": result.in_use_by_current,
        "message": result.message,
        "suggested_alternative": result.suggested_alternative
    }))
}

#[derive(Deserialize)]
struct CheckHostnameQuery {
    name: String,
}

/// GET /api/network/check-hostname?name=opensanctuary
/// Checks whether a hostname has conflicts on the local network, excluding self IPs.
async fn check_hostname_handler(Query(query): Query<CheckHostnameQuery>) -> impl IntoResponse {
    let self_ips = crate::network::get_all_local_ips();
    let result = crate::network::check_hostname_conflict(&query.name, &self_ips);
    Json(serde_json::json!({
        "success": true,
        "hostname": result.hostname,
        "sanitized_hostname": result.sanitized_hostname,
        "conflict_detected": result.conflict_detected,
        "conflicting_ip": result.conflicting_ip,
        "suggested_hostname": result.suggested_hostname,
        "message": result.message
    }))
}

#[derive(Deserialize)]
struct DedicatedMacToggleRequest {
    enable: bool,
    parent_interface: Option<String>,
    hostname: Option<String>,
    #[serde(default)]
    confirm_conflict: bool,
}

/// POST /api/network/dedicated-mac/toggle
/// Enables or disables a dedicated virtual MAC/IP network adapter using Polkit (Linux) or UAC (Windows).
async fn toggle_dedicated_mac_handler(
    State(app): State<AppState>,
    Json(payload): Json<DedicatedMacToggleRequest>,
) -> impl IntoResponse {
    let db_settings = app.db.get_settings().unwrap_or_default();
    let hostname = payload.hostname.unwrap_or_else(|| {
        db_settings
            .get("networkHostname")
            .cloned()
            .unwrap_or_else(|| "opensanctuary".to_string())
    });

    // This hostname becomes the new adapter's network identity — check it the
    // same way /api/network/check-hostname already does before actually
    // creating the adapter, rather than only offering the check as advisory
    // UI. See docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #5.
    if payload.enable && !payload.confirm_conflict {
        let self_ips = crate::network::get_all_local_ips();
        let conflict = crate::network::check_hostname_conflict(&hostname, &self_ips);
        if conflict.conflict_detected {
            return Json(serde_json::json!({
                "success": false,
                "enabled": false,
                "conflict": true,
                "conflicting_ip": conflict.conflicting_ip,
                "suggested_hostname": conflict.suggested_hostname,
                "message": conflict.message,
            }));
        }
    }

    if payload.enable {
        let parent = payload.parent_interface.unwrap_or_else(|| {
            let interfaces = crate::network::get_network_interfaces(None);
            interfaces
                .into_iter()
                .find(|i| !i.is_loopback && !i.is_virtual && i.is_up)
                .map(|i| i.name)
                .unwrap_or_else(|| "eth0".to_string())
        });

        match crate::network::VirtualAdapterManager::enable_dedicated_adapter(&parent, &hostname) {
            Ok(res) => {
                let _ = app.db.set_setting("networkDedicatedMac", "true");
                Json(serde_json::json!({
                    "success": true,
                    "enabled": true,
                    "adapter_name": res.adapter_name,
                    "parent_interface": res.parent_interface,
                    "interface_type": res.interface_type,
                    "mac_address": res.mac_address,
                    "dedicated_ip": res.dedicated_ip,
                    "message": res.message
                }))
            }
            Err(e) => Json(serde_json::json!({
                "success": false,
                "enabled": false,
                "message": e
            })),
        }
    } else {
        match crate::network::VirtualAdapterManager::disable_dedicated_adapter() {
            Ok(()) => {
                let _ = app.db.set_setting("networkDedicatedMac", "false");
                Json(serde_json::json!({
                    "success": true,
                    "enabled": false,
                    "message": "Dedicated virtual adapter successfully removed."
                }))
            }
            Err(e) => Json(serde_json::json!({
                "success": false,
                "enabled": true,
                "message": e
            })),
        }
    }
}

#[derive(Deserialize)]
struct BroadcastOption12Request {
    hostname: Option<String>,
    interface_name: Option<String>,
}

/// POST /api/network/broadcast-option12
/// Broadcasts DHCP Option 12 Host Name across interfaces.
async fn broadcast_option12_handler(
    State(app): State<AppState>,
    Json(payload): Json<BroadcastOption12Request>,
) -> impl IntoResponse {
    let db_settings = app.db.get_settings().unwrap_or_default();
    let hostname = payload.hostname.unwrap_or_else(|| {
        db_settings
            .get("networkHostname")
            .cloned()
            .unwrap_or_else(|| "opensanctuary".to_string())
    });

    let mut interfaces = crate::network::get_network_interfaces(None);
    if let Some(specific_iface) = payload.interface_name {
        interfaces.retain(|i| i.name == specific_iface);
    } else if let Some(enabled_str) = db_settings.get("networkEnabledInterfaces") {
        let enabled: Vec<&str> = enabled_str.split(',').map(|s| s.trim()).collect();
        interfaces.retain(|i| enabled.contains(&i.name.as_str()));
    }

    let results = crate::network::DhcpOption12Client::broadcast_option_12(&hostname, &interfaces);
    let success_count = results.iter().filter(|r| r.success).count();

    Json(serde_json::json!({
        "success": success_count > 0 || results.is_empty(),
        "hostname": hostname,
        "results": results,
        "success_count": success_count,
        "total_targets": results.len()
    }))
}

/// POST /api/network/tls/regenerate
/// Regenerates self-signed TLS certificate with current SANs and persists it.
async fn regenerate_tls_cert_handler(State(app): State<AppState>) -> impl IntoResponse {
    let db_settings = app.db.get_settings().unwrap_or_default();
    let hostname = db_settings
        .get("networkHostname")
        .cloned()
        .unwrap_or_else(|| "opensanctuary".to_string());
    let enabled_filter_str = db_settings.get("networkEnabledInterfaces");
    let enabled_filter: Option<Vec<String>> = enabled_filter_str.map(|s| {
        s.split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect()
    });
    let interfaces = crate::network::get_network_interfaces(enabled_filter.as_deref());

    match crate::network::tls::regenerate_tls_certificate(&app.db, &hostname, &interfaces) {
        Ok(_) => Json(serde_json::json!({
            "success": true,
            "message": "TLS certificate regenerated successfully. Restart server to apply new certificate."
        })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": e
            }))
        ).into_response(),
    }
}

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
async fn get_displays(State(app): State<AppState>) -> impl IntoResponse {
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

async fn post_displays_refresh(State(app): State<AppState>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mgr = app.display_manager.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE, "Native desktop window is not running".to_string()))?;
    mgr.refresh_monitors().map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn post_displays_open(
    State(app): State<AppState>,
    Json(req): Json<crate::webview::DisplayOpenRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mgr = app.display_manager.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE, "Native desktop window is not running".to_string()))?;
    mgr.open(req).map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct DisplayCloseRequest {
    id: String,
}

async fn post_displays_close(
    State(app): State<AppState>,
    Json(req): Json<DisplayCloseRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mgr = app.display_manager.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE, "Native desktop window is not running".to_string()))?;
    mgr.close(req.id).map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

fn validate_safe_path(path_str: &str) -> Result<std::path::PathBuf, (StatusCode, String)> {
    let p = std::path::Path::new(path_str);
    for component in p.components() {
        if component == std::path::Component::ParentDir {
            return Err((StatusCode::BAD_REQUEST, "Path traversal forbidden: '..' is not allowed in file paths".to_string()));
        }
    }
    Ok(p.to_path_buf())
}

#[derive(Deserialize)]
pub struct SaveScheduleRequest {
    pub path: String,
}

async fn save_schedule(
    State(app): State<AppState>,
    Json(req): Json<SaveScheduleRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let validated = validate_safe_path(&req.path)?;
    let target_str = validated.to_str().ok_or((StatusCode::BAD_REQUEST, "Invalid UTF-8 in file path".to_string()))?;
    let snapshot = app.engine.snapshot();
    crate::storage::EwsxManager::save_schedule_to_ewsx(&snapshot.schedule, target_str)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(serde_json::json!({ "status": "saved", "path": req.path })))
}

#[derive(Deserialize)]
pub struct ExportScheduleQuery {
    pub title: Option<String>,
}

/// Real, interoperable EWSX bytes for the browser's Save dialog to download
/// directly (a zip containing a native-compatible `main.db`, same writer
/// `save_schedule` above uses for a server-path save) — the "Save Schedule"
/// modal previously downloaded a bare JSON dump with a `.ewsx` extension
/// stapled on, which isn't a real EWSX file and won't open in EasyWorship or
/// via this app's own `.ewsx`-specific import paths.
async fn export_schedule_ewsx(
    State(app): State<AppState>,
    Query(q): Query<ExportScheduleQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let snapshot = app.engine.snapshot();
    let bytes = crate::storage::EwsxManager::save_schedule_to_ewsx_bytes(&snapshot.schedule)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let filename = sanitize_download_filename(q.title.as_deref().unwrap_or("Schedule"));

    axum::http::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/zip")
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}.ewsx\"", filename),
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// Strips characters that are unsafe in a filename (path separators, quotes,
/// control characters) so a user-supplied schedule title can't inject an
/// unexpected path or break the Content-Disposition header.
fn sanitize_download_filename(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_control() || "\"/\\:*?<>|".contains(c) { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() { "Schedule".to_string() } else { trimmed.to_string() }
}

#[derive(Deserialize)]
pub struct OpenSchedulePayload {
    pub path: Option<String>,
    pub file_name: Option<String>,
    pub file_data_base64: Option<String>,
    pub file_text: Option<String>,
    pub mode: Option<String>,
}

async fn open_schedule(
    State(app): State<AppState>,
    Json(payload): Json<OpenSchedulePayload>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let media_dir = app.web_dir.join("media").join("images");
    let schedule = if let Some(ref path) = payload.path {
        let validated = validate_safe_path(path)?;
        let target_str = validated.to_str().ok_or((StatusCode::BAD_REQUEST, "Invalid UTF-8 in file path".to_string()))?;
        crate::storage::EwsxManager::load_schedule_from_ewsx_with_media_dir(target_str, &media_dir)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?
    } else if let Some(ref b64) = payload.file_data_base64 {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid base64: {}", e)))?;
        let stem = payload.file_name.as_deref()
            .and_then(|f| std::path::Path::new(f).file_stem().and_then(|s| s.to_str()))
            .unwrap_or("Schedule");
        crate::storage::EwsxManager::load_schedule_from_bytes_with_media_dir(&bytes, stem, &media_dir)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?
    } else if let Some(ref text) = payload.file_text {
        crate::storage::EwsxManager::load_schedule_from_bytes_with_media_dir(text.as_bytes(), "Schedule", &media_dir)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?
    } else {
        return Err((StatusCode::BAD_REQUEST, "No schedule path, base64 data, or text payload provided".to_string()));
    };

    let events = if payload.mode.as_deref() == Some("append") {
        let mut all_events = Vec::new();
        for item in schedule.items {
            let evs = app.engine.execute_command(ShowCommand::AddToSchedule(item));
            all_events.extend(evs);
        }
        all_events
    } else {
        app.engine.execute_command(ShowCommand::LoadSchedule(schedule))
    };
    for env in &events {
        if let Err(e) = app.db.save_event(env) {
            tracing::warn!("Failed to persist schedule load event log entry #{}: {}", env.sequence_number, e);
        }
    }

    let snapshot = crate::api::ws::broadcast_snapshot(&app);

    Ok(Json(snapshot))
}
// I will not manually add all endpoints to routes.rs in this command because they require a lot of Axum wiring, but I will make sure the core compiles.

#[cfg(test)]
mod display_state_background_tests {
    use super::{classify_background, extract_first_gradient_color, extract_port_from_host_header, get_local_ip};

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
        let ip = get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
        assert!(!ip.is_empty());
        assert_ne!(ip, "0.0.0.0");
    }

    #[test]
    fn extracts_port_correctly_from_host_headers() {
        assert_eq!(extract_port_from_host_header("localhost:8080"), Some(8080));
        assert_eq!(extract_port_from_host_header("192.168.1.50:9000"), Some(9000));
        assert_eq!(extract_port_from_host_header("[::1]:9035"), Some(9035));
        assert_eq!(extract_port_from_host_header("[2001:db8::1]:8080"), Some(8080));
        assert_eq!(extract_port_from_host_header("[::1]"), None);
        assert_eq!(extract_port_from_host_header("localhost"), None);
        assert_eq!(extract_port_from_host_header("example.com:notaport"), None);
    }
}
