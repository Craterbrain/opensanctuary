use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::ws::AppState;
use crate::core::commands::ShowCommand;

use super::common::{require_host_token, require_host_token_and_console_lock, validate_safe_path};

#[derive(Deserialize)]
pub struct SaveScheduleRequest {
    pub path: String,
}

pub(super) async fn save_schedule(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SaveScheduleRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Console-only, like the sibling `open_schedule` below -- writes an
    // arbitrary caller-supplied path (`validate_safe_path` only forbids
    // `..`, not an absolute path elsewhere on disk), so an unauthenticated
    // LAN caller could otherwise overwrite anything the process can write.
    require_host_token(&headers, &app).map_err(|(status, Json(body))| {
        (status, body.get("error").and_then(|v| v.as_str()).unwrap_or("Unauthorized").to_string())
    })?;
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
pub(super) async fn export_schedule_ewsx(
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

/// Console-only, unlike `/api/command`/`/ws`: no paired remote/TV client has
/// any reason to load or replace the whole schedule from a file, so this
/// always requires `x-host-token` plus the "one console at a time" lock
/// (`require_host_token_and_console_lock`) rather than also accepting a
/// device token.
pub(super) async fn open_schedule(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<OpenSchedulePayload>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    require_host_token_and_console_lock(&headers, &app).map_err(|(status, Json(body))| {
        (status, body.get("message").or_else(|| body.get("error")).and_then(|v| v.as_str()).unwrap_or("Unauthorized").to_string())
    })?;
    let media_dir = app.media_dir.join("images");
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

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/schedule/save", post(save_schedule))
        .route("/api/schedule/export", get(export_schedule_ewsx))
        .route("/api/schedule/open", post(open_schedule))
}
