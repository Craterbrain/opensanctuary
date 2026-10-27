//! Files the desktop asked OpenSanctuary to open (double-click, "Open with"): `network::open_files`.
//!
//! The console lists them, then takes each one: the bytes go through the same inspect / copyright pop-up / import steps as the Import dialog.
//! Both routes need the host token, so only the console can see the names or read a file's bytes.

use axum::body::Body;
use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::api::ws::AppState;
use crate::network::open_files;

/// GET /api/open-file/pending -- `[{ id, name }]`, the files waiting for the console.
async fn get_pending() -> impl IntoResponse {
    Json(open_files::pending())
}

/// POST /api/open-file/{id}/take -- the file's bytes, once: it leaves the waiting list as it is read.
async fn take_file(Path(id): Path<String>) -> Result<Response, StatusCode> {
    let file = open_files::take(&id).ok_or(StatusCode::NOT_FOUND)?;
    // Re-check at read time: the file may have been replaced since it was queued.
    if !open_files::is_openable(&file.path) {
        return Err(StatusCode::NOT_FOUND);
    }
    let bytes = tokio::fs::read(&file.path).await.map_err(|_| StatusCode::NOT_FOUND)?;
    let disposition = format!("attachment; filename*=UTF-8''{}", urlencode(&file.name));
    Ok(([(header::CONTENT_TYPE, "application/octet-stream".to_string()), (header::CONTENT_DISPOSITION, disposition)], Body::from(bytes)).into_response())
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/api/open-file/pending", get(get_pending)).route("/api/open-file/{id}/take", post(take_file))
}
