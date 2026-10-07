use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::ws::AppState;
use crate::core::models::{Presentation, Song};

use super::common::{require_host_token, validate_safe_path};

use crate::storage::freeshow_import;
use crate::storage::genius_import::GeniusImporter;
use crate::storage::ytdlp_import::{YtDlpImporter, YtDlpDownloadOptions};

pub(super) async fn get_bibles(State(app): State<AppState>) -> impl IntoResponse {
    let bibles = app.db.get_installed_bibles().unwrap_or_default();
    Json(bibles)
}

#[derive(Deserialize)]
pub struct ParseRefQuery {
    q: String,
}

pub(super) async fn parse_bible_ref(Query(query): Query<ParseRefQuery>) -> impl IntoResponse {
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

pub(super) async fn get_bible_passage(
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

pub(super) async fn delete_bible(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    require_host_token(&headers, &app).map_err(|(status, Json(body))| {
        (status, body.get("error").and_then(|v| v.as_str()).unwrap_or("Unauthorized").to_string())
    })?;
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

pub(super) async fn genius_search(Query(query): Query<GeniusSearchQuery>) -> impl IntoResponse {
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

pub(super) async fn genius_import(State(app): State<AppState>, Json(req): Json<GeniusImportReq>) -> impl IntoResponse {
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

pub(super) async fn ytdlp_download(State(app): State<AppState>, Json(options): Json<YtDlpDownloadOptions>) -> impl IntoResponse {
    let task_id = YtDlpImporter::start_background_download(options, app.media_dir.clone(), app.data_dir.join("tools"), app.db.clone());
    Json(serde_json::json!({ "success": true, "task_id": task_id }))
}

pub(super) async fn ytdlp_progress(AxumPath(task_id): AxumPath<String>) -> impl IntoResponse {
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
pub(super) async fn import_openlp(
    State(app): State<AppState>,
    Json(req): Json<OpenLpImportReq>,
) -> impl IntoResponse {
    let media_dir = app.media_dir.join("images");
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
pub(super) async fn import_pptx(
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

    let media_dir = app.media_dir.join("images");
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
pub(super) async fn import_freeshow_show(
    State(app): State<AppState>,
    Json(req): Json<FreeShowShowImportReq>,
) -> impl IntoResponse {
    use base64::Engine;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(req.file_data_base64.trim()) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Invalid base64: {}", e) }))),
    };

    let media_dir = app.media_dir.join("images");
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
                    let pres = Presentation {
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
pub(super) async fn fsb_catalog(State(app): State<AppState>) -> impl IntoResponse {
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
pub(super) async fn download_bible_full(
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

pub(super) async fn fsb_fetch(State(app): State<AppState>, Json(req): Json<FsbFetchReq>) -> impl IntoResponse {
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
pub(super) async fn github_bible_catalog(
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

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/bibles", get(get_bibles))
        .route("/api/bibles/:id", axum::routing::delete(delete_bible))
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
}
