//! Saving to, and opening from, OpenSanctuary's own file formats (`.ossong`, `.ospres`, `.osservice`,
//! `.osbible` -- see `crate::storage::bundle` for the format and its safety rules).
//!
//! Bundles can be large (they carry pictures and videos), so neither direction holds one in memory:
//! an upload is streamed to a temporary file in the data directory before it's read, and an export is
//! written to one and streamed back. Every route here changes or reveals library content, so all of
//! them need the console's host token (`/api/export/` is gated by prefix; the others are `POST`s, which
//! `needs_host_token` gates by default).

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use crate::api::ws::AppState;
use crate::storage::bundle::{self, BundleError, BundleKind, ImportOptions};

use super::common::require_host_token_and_console_lock;
use super::schedule::{apply_loaded_schedule, sanitize_download_filename};

/// The format's own ceiling on a bundle's media (8 GiB) plus room for its text.
const MAX_BUNDLE_UPLOAD: usize = (8 << 30) + (512 << 20);
/// Spooled files older than this are leftovers of a crash and are swept up.
const STALE_SPOOL_SECS: u64 = 24 * 60 * 60;

type ApiError = (StatusCode, Json<serde_json::Value>);

fn api_error(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(serde_json::json!({ "success": false, "message": message.into() })))
}

fn bundle_error(e: BundleError) -> ApiError {
    let status = match &e {
        BundleError::Invalid(_) | BundleError::UnsupportedVersion { .. } | BundleError::WrongKind { .. } | BundleError::Zip(_) | BundleError::Json(_) => StatusCode::BAD_REQUEST,
        BundleError::AlreadyInstalled(_) => StatusCode::CONFLICT,
        BundleError::NotFound(_) => StatusCode::NOT_FOUND,
        BundleError::Db(_) | BundleError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    api_error(status, e.to_string())
}

fn join_error(e: tokio::task::JoinError) -> ApiError {
    api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("the task failed: {e}"))
}

fn spool_dir(app: &AppState) -> std::path::PathBuf {
    app.data_dir.join("tmp")
}

/// Deletes spool files a crashed request left behind (a finished request removes its own).
fn sweep_stale_spool(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("bundle-") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() > STALE_SPOOL_SECS);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn new_spool_file(app: &AppState) -> Result<tempfile::NamedTempFile, ApiError> {
    let dir = spool_dir(app);
    std::fs::create_dir_all(&dir).map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("could not prepare a working folder: {e}")))?;
    sweep_stale_spool(&dir);
    tempfile::Builder::new()
        .prefix("bundle-")
        .suffix(".tmp")
        .tempfile_in(&dir)
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("could not create a working file: {e}")))
}

/// Streams an upload to a temporary file, so a multi-gigabyte bundle never sits in memory.
async fn spool_upload(app: &AppState, body: Body) -> Result<tempfile::NamedTempFile, ApiError> {
    let tmp = new_spool_file(app)?;
    let handle = tmp.reopen().map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut file = tokio::fs::File::from_std(handle);
    let mut stream = body.into_data_stream();
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| api_error(StatusCode::BAD_REQUEST, format!("the upload was interrupted: {e}")))?;
        total += chunk.len() as u64;
        file.write_all(&chunk).await.map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("could not store the upload (is the disk full?): {e}")))?;
    }
    file.flush().await.map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if total == 0 {
        return Err(api_error(StatusCode::BAD_REQUEST, "no file was sent"));
    }
    Ok(tmp)
}

/// Removes a path when dropped. It sits after the open file in the stream's state, so the file is
/// closed first -- which matters on Windows, where an open file can't be deleted.
struct RemoveOnDrop(std::path::PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn stream_and_remove(path: std::path::PathBuf) -> std::io::Result<(Body, u64)> {
    let file = tokio::fs::File::open(&path).await?;
    let len = file.metadata().await?.len();
    let guard = RemoveOnDrop(path);
    let stream = futures_util::stream::unfold((file, guard), |(mut file, guard)| async move {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 256 * 1024];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<_, std::io::Error>(axum::body::Bytes::from(buf)), (file, guard)))
            }
            Err(e) => Some((Err(e), (file, guard))),
        }
    });
    Ok((Body::from_stream(stream), len))
}

#[derive(Deserialize)]
pub(super) struct ExportRequest {
    kind: BundleKind,
    /// Songs or presentations to export; all of them when absent.
    #[serde(default)]
    ids: Option<Vec<String>>,
    /// For a Bible, the abbreviation of the installed translation.
    #[serde(default)]
    version: Option<String>,
    /// For a service, the name to save it under (defaults to the schedule's own title).
    #[serde(default)]
    title: Option<String>,
}

/// POST /api/export/bundle -- the bytes of a `.ossong`, `.ospres`, `.osservice` or `.osbible` file.
pub(super) async fn export_bundle(State(app): State<AppState>, Json(req): Json<ExportRequest>) -> Result<Response, ApiError> {
    let tmp = new_spool_file(&app)?;
    let (file, temp_path) = tmp.into_parts();
    let path = temp_path.keep().map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // From here the path is removed when `guard` drops, whether the export fails or the stream ends.
    let guard = RemoveOnDrop(path.clone());

    let db = app.db.clone();
    let media_dir = app.media_dir.clone();
    let kind = req.kind;
    let schedule = (kind == BundleKind::Service).then(|| app.engine.snapshot().schedule.clone());
    let report = tokio::task::spawn_blocking(move || {
        let mut out = file;
        match kind {
            BundleKind::Song => bundle::export_songs(&db, &media_dir, req.ids.as_deref(), &mut out),
            BundleKind::Presentation => bundle::export_presentations(&db, &media_dir, req.ids.as_deref(), &mut out),
            BundleKind::Service => bundle::export_service(&db, &media_dir, &schedule.expect("a service export always reads the schedule"), req.title.as_deref(), &mut out),
            BundleKind::Bible => {
                let version = req.version.as_deref().ok_or_else(|| BundleError::Invalid("say which Bible to export".into()))?;
                bundle::export_bible(&db, version, &mut out)
            }
        }
    })
    .await
    .map_err(join_error)?
    .map_err(bundle_error)?;

    let (body, len) = stream_and_remove(path).await.map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    drop(guard); // the stream now owns removal; the early guard was only for the failure paths above
    let filename = format!("{}.{}", sanitize_download_filename(&report.title), kind.extension());
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"; filename*=UTF-8''{}", ascii_fallback(&filename), percent_encode(&filename)))
            .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
    );
    headers.insert("x-os-media-count", HeaderValue::from(report.media_count as u64));
    headers.insert("x-os-missing-media", HeaderValue::from(report.missing_media.len() as u64));
    Ok((StatusCode::OK, headers, body).into_response())
}

fn ascii_fallback(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii() && !c.is_control() && c != '"' && c != '\\' { c } else { '_' }).collect()
}

fn percent_encode(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// POST /api/bundle/inspect -- what a bundle holds and how it relates to the library, without
/// changing anything. The body is the file itself.
pub(super) async fn inspect_bundle(State(app): State<AppState>, body: Body) -> Result<Json<serde_json::Value>, ApiError> {
    let tmp = spool_upload(&app, body).await?;
    let db = app.db.clone();
    let media_dir = app.media_dir.clone();
    let summary = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(tmp.path())?;
        let opened = bundle::open(file, None)?;
        bundle::inspect(&opened, &db, &media_dir)
    })
    .await
    .map_err(join_error)?
    .map_err(bundle_error)?;
    Ok(Json(serde_json::json!({ "success": true, "summary": summary })))
}

#[derive(Deserialize)]
pub(super) struct ImportQuery {
    /// Replace a Bible that is already installed under the same abbreviation.
    #[serde(default)]
    replace: bool,
    /// For a service: `append` adds its items to the current schedule instead of replacing it.
    #[serde(default)]
    mode: Option<String>,
}

/// POST /api/bundle/import -- puts a bundle's contents into the library (or, for a service, into the
/// live show). The body is the file itself.
pub(super) async fn import_bundle(State(app): State<AppState>, Query(q): Query<ImportQuery>, headers: HeaderMap, body: Body) -> Result<Json<serde_json::Value>, ApiError> {
    // `tmp` stays alive to the end: it is the file the opened bundle reads from.
    let tmp = spool_upload(&app, body).await?;
    let path = tmp.path().to_path_buf();
    let mut opened = tokio::task::spawn_blocking(move || bundle::open(std::fs::File::open(path)?, None))
        .await
        .map_err(join_error)?
        .map_err(bundle_error)?;

    // A service takes over the live show, so it follows the same "one console at a time" rule as opening
    // any other schedule file. Checked now that the file is known to be a valid bundle and before the
    // first thing is written.
    if opened.manifest.kind == BundleKind::Service {
        require_host_token_and_console_lock(&headers, &app).map_err(|(status, Json(body))| {
            let msg = body.get("message").or_else(|| body.get("error")).and_then(|v| v.as_str()).unwrap_or("Unauthorized").to_string();
            api_error(status, msg)
        })?;
    }

    let db = app.db.clone();
    let media_dir = app.media_dir.clone();
    let opts = ImportOptions { replace_bible: q.replace };
    let title = opened.manifest.title.clone();
    let mut result = tokio::task::spawn_blocking(move || bundle::import(&mut opened, &db, &media_dir, &opts))
        .await
        .map_err(join_error)?
        .map_err(bundle_error)?;

    if let Some(schedule) = result.schedule.take() {
        apply_loaded_schedule(&app, schedule, q.mode.as_deref() == Some("append"));
    }
    drop(tmp);
    Ok(Json(serde_json::json!({ "success": true, "title": title, "result": result })))
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/export/bundle", post(export_bundle))
        .route("/api/bundle/inspect", post(inspect_bundle).layer(axum::extract::DefaultBodyLimit::max(MAX_BUNDLE_UPLOAD)))
        .route("/api/bundle/import", post(import_bundle).layer(axum::extract::DefaultBodyLimit::max(MAX_BUNDLE_UPLOAD)))
}
