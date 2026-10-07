use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};

use crate::api::ws::AppState;

use super::common::{require_host_token, validate_safe_path};

#[derive(serde::Deserialize)]
pub(super) struct MoveDataDirReq {
    target_dir: String,
    #[serde(default)]
    delete_source: bool,
}

/// POST /api/system/move-data-dir
///
/// Moves user data (library.db, bibles, songs, media) to a new target directory,
/// verifies all copied files with SHA-256 checksums, updates `install.json`, and
/// optionally deletes original source files if confirmed and verified.
///
/// Console-only, like the other admin/settings actions gated by
/// `require_host_token` -- without this, any LAN client could relocate (and,
/// with `delete_source`, destroy) the entire data directory with one
/// unauthenticated POST.
pub(super) async fn post_move_data_dir(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<MoveDataDirReq>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    require_host_token(&headers, &app).map_err(|(status, Json(body))| {
        (status, body.get("error").and_then(|v| v.as_str()).unwrap_or("Unauthorized").to_string())
    })?;

    let target_str = req.target_dir.trim();
    if target_str.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Target directory cannot be empty".to_string()));
    }

    let target_path = validate_safe_path(target_str)?;

    match crate::storage::paths::move_data_dir(&app.data_dir, &target_path, Some(&app.db), req.delete_source) {
        Ok(result) => {
            tracing::info!(
                "Data directory successfully moved from {:?} to {:?} ({} files copied, sha verified: {}, source deleted: {})",
                result.source_dir,
                result.target_dir,
                result.files_copied,
                result.sha_verified,
                result.source_deleted
            );
            Ok(Json(serde_json::json!({
                "ok": true,
                "source_dir": result.source_dir.to_string_lossy(),
                "target_dir": result.target_dir.to_string_lossy(),
                "files_copied": result.files_copied,
                "sha_verified": result.sha_verified,
                "source_deleted": result.source_deleted,
                "restart_required": true,
            })))
        }
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
            Err((StatusCode::BAD_REQUEST, e.to_string()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err((StatusCode::CONFLICT, e.to_string()))
        }
        Err(e) => {
            tracing::error!("move_data_dir failed: {}", e);
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to move data directory: {}", e)))
        }
    }
}

/// POST /api/system/reveal-data-dir
///
/// Opens the resolved data directory (`app.data_dir`) in the host OS's file
/// manager (Explorer/Finder/the Linux file manager), via the same
/// `open::that` primitive the update-installer flow already uses --
/// `docs/paths.md`'s "Future Work" item 3. Console-only, like the other
/// settings actions gated by `require_host_token`: this opens a window on
/// whatever machine the *server* is running on, not the viewer's own
/// device, so an unauthenticated LAN caller triggering it on a console
/// they don't have physical access to would be pointless at best.
pub(super) async fn post_reveal_data_dir(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;

    #[cfg(feature = "open")]
    let open_result = crate::network::updater::open_with_system_handler(&app.data_dir);
    #[cfg(not(feature = "open"))]
    let open_result: Result<(), String> = Err("Opening a file manager isn't supported in this build".to_string());

    Ok(match open_result {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e })),
    })
}

#[derive(serde::Deserialize)]
pub(super) struct AdoptLegacyLibraryReq {
    legacy_dir: String,
}

/// POST /api/system/adopt-legacy-library
///
/// First-time setup's "Use it instead" action (docs/first-time.md step 2,
/// docs/paths.md "Legacy Import Detection") for a `legacy_library_detected`
/// path from `/api/server-info`. Unlike "Move Data Directory," this never
/// copies anything -- it just points `install.json` at the already-existing
/// legacy directory, so the fresh (and still essentially empty) database
/// this run created at the platform-default location is simply abandoned in
/// favor of it on restart. Re-validated server-side (not just trusting the
/// client-supplied path) since a stale/tampered value here would otherwise
/// point the app at a directory with no real database in it.
pub(super) async fn post_adopt_legacy_library(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AdoptLegacyLibraryReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;
    let legacy_path = std::path::PathBuf::from(&req.legacy_dir);
    if !legacy_path.join("library.db").is_file() {
        return Ok(Json(serde_json::json!({
            "ok": false,
            "error": "No library.db found at that location anymore.",
        })));
    }
    match crate::storage::paths::write_install_config(&legacy_path) {
        Ok(()) => Ok(Json(serde_json::json!({ "ok": true, "restart_required": true }))),
        Err(e) => Ok(Json(serde_json::json!({ "ok": false, "error": e.to_string() }))),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct ApplyLocalUpdateReq {
    path: String,
    /// Open anyway despite a `Mismatch`/`ChecksumFileUnverified` result --
    /// the frontend sets this only after the operator explicitly confirms
    /// a loud warning. Irrelevant for every other checksum outcome, which
    /// always opens.
    #[serde(default)]
    force: bool,
}

/// POST /api/updates/apply-local
///
/// Manual update path (docs/update.md): no automated update checking or
/// pushing happens during alpha, by design -- an operator downloads a
/// release themselves and picks the file via `pick_file` above. This
/// checks it against a sibling `checksums.txt`/`checksums.txt.minisig` if
/// present (informational, not a hard gate -- see
/// `check_local_file_against_sibling_checksums`'s doc comment for why),
/// then hands it to the OS's own installer. Never runs anything with
/// elevated privileges itself.
pub(super) async fn apply_local_update(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ApplyLocalUpdateReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Console-only: without this, any LAN caller could hand the OS's
    // default file-open handler an arbitrary path with no auth at all.
    require_host_token(&headers, &app)?;
    let path = std::path::Path::new(&req.path);
    let checksum = crate::network::updater::check_local_file_against_sibling_checksums(
        path,
        crate::network::updater::RELEASE_PUBLIC_KEY_B64,
    );

    let needs_confirmation = matches!(
        checksum,
        crate::network::updater::ChecksumCheck::Mismatch { .. }
            | crate::network::updater::ChecksumCheck::ChecksumFileUnverified { .. }
    );
    if needs_confirmation && !req.force {
        return Ok(Json(serde_json::json!({ "ok": false, "checksum": checksum, "needs_confirmation": true })));
    }

    #[cfg(feature = "open")]
    let open_result = crate::network::updater::open_with_system_handler(path);
    #[cfg(not(feature = "open"))]
    let open_result: Result<(), String> = Err("Opening an installer isn't supported in this build".to_string());

    Ok(match open_result {
        Ok(()) => Json(serde_json::json!({ "ok": true, "checksum": checksum })),
        Err(e) => Json(serde_json::json!({ "ok": false, "checksum": checksum, "error": e })),
    })
}

/// GET /api/updates/status
///
/// Returns the cached result of the last automated check (`AppState::update_status`,
/// populated by the background task in `main.rs` and by `post_update_check`
/// below) -- never makes a network call itself, so this is safe to poll.
/// `null` until the first check completes.
pub(super) async fn get_update_status(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;
    let status = app.update_status.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Ok(Json(serde_json::json!({ "status": status })))
}

/// GET /api/ytdlp-updater/status
///
/// Returns the cached result of the last yt-dlp self-update check
/// (`AppState::ytdlp_update_status`, populated by the background task in
/// `main.rs` -- see `src/network/ytdlp_updater.rs`) -- never makes a
/// network call itself, so this is safe to poll. `null` until the first
/// check completes.
pub(super) async fn get_ytdlp_updater_status(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;
    let status = app.ytdlp_update_status.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Ok(Json(serde_json::json!({ "status": status })))
}

/// POST /api/updates/check
///
/// Forces a real, right-now check against `crate::network::updater::RELEASES_REPO`
/// (for the manual "Check for Updates" button -- a caller clicking this
/// seconds after startup's own check should still see it do something, not
/// just re-read the same cached result). Updates `AppState::update_status`
/// with the fresh result before returning it.
/// Minimum gap between two real network checks, regardless of how often
/// this route is hit -- GitHub's REST API is generous on paper (60 req/hour
/// unauthenticated) but a short burst of automated/scripted-looking
/// requests can trip its separate, undocumented-threshold secondary abuse
/// detection well before the hourly count matters (confirmed directly: hit
/// a transient 403 from repeated `reqwest` calls during this feature's own
/// development, while the same IP's hourly budget still had room to
/// spare). An operator mashing "Check for Updates" a few times in a row is
/// a real scenario worth not needlessly hitting GitHub for.
const MIN_SECONDS_BETWEEN_REAL_CHECKS: i64 = 60;

pub(super) async fn post_update_check(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;

    if let Some(cached) = app.update_status.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        let age_secs = (chrono::Utc::now().timestamp_millis() - cached.checked_at_ms) / 1000;
        if age_secs < MIN_SECONDS_BETWEEN_REAL_CHECKS {
            return Ok(Json(serde_json::json!({ "status": cached, "throttled": true })));
        }
    }

    let result = match crate::network::updater::check_latest_release().await {
        Ok(latest) => crate::network::updater::UpdateCheckResult {
            checked_at_ms: chrono::Utc::now().timestamp_millis(),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            latest,
            error: None,
        },
        Err(e) => crate::network::updater::UpdateCheckResult {
            checked_at_ms: chrono::Utc::now().timestamp_millis(),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            latest: None,
            error: Some(e),
        },
    };
    *app.update_status.lock().unwrap_or_else(|e| e.into_inner()) = Some(result.clone());
    Ok(Json(serde_json::json!({ "status": result })))
}

/// POST /api/updates/download-and-install
///
/// Downloads and verifies the release `POST /api/updates/check` last found
/// (`crate::network::updater::download_and_verify_release`, which fails
/// closed on any signature/hash mismatch), then hands the verified file to
/// the OS's own installer -- same last step `apply_local_update` above
/// uses, not reimplemented. On Windows this launches the downloaded
/// `opensanctuary-setup-x64.exe` interactively (Inno Setup's own UI/
/// `CloseApplications` directive handles closing the running app and
/// relaunching after, same as a user double-clicking it themselves) -- not
/// the fully-silent `/VERYSILENT` flow `docs/update.md` originally sketched,
/// matching this app's existing "never a silent swap, always an explicit
/// choice" posture for the manual flow. **macOS only, still not built**:
/// no installer exists for it yet (`docs/installer.md`).
pub(super) async fn post_update_download_and_install(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;

    if cfg!(target_os = "macos") {
        return Ok(Json(serde_json::json!({
            "ok": false,
            "error": "Automated download-and-install isn't available on macOS yet -- there's no installer for it (see docs/installer.md).",
        })));
    }

    let release = match crate::network::updater::check_latest_release().await {
        Ok(Some(release)) => release,
        Ok(None) => return Ok(Json(serde_json::json!({ "ok": false, "error": "No update is available." }))),
        Err(e) => return Ok(Json(serde_json::json!({ "ok": false, "error": e }))),
    };

    let path = match crate::network::updater::download_and_verify_release(&release).await {
        Ok(path) => path,
        Err(e) => return Ok(Json(serde_json::json!({ "ok": false, "error": e }))),
    };

    #[cfg(feature = "open")]
    let open_result = crate::network::updater::open_with_system_handler(&path);
    #[cfg(not(feature = "open"))]
    let open_result: Result<(), String> = Err("Opening an installer isn't supported in this build".to_string());

    Ok(match open_result {
        Ok(()) => Json(serde_json::json!({ "ok": true, "version": release.version })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e })),
    })
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/system/move-data-dir", post(post_move_data_dir))
        .route("/api/system/reveal-data-dir", post(post_reveal_data_dir))
        .route("/api/system/adopt-legacy-library", post(post_adopt_legacy_library))
        .route("/api/updates/apply-local", post(apply_local_update))
        .route("/api/updates/status", get(get_update_status))
        .route("/api/updates/check", post(post_update_check))
        .route("/api/updates/download-and-install", post(post_update_download_and_install))
        .route("/api/ytdlp-updater/status", get(get_ytdlp_updater_status))
}
