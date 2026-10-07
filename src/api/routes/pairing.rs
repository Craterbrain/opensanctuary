use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::api::ws::AppState;

use super::common::require_host_token;

pub(super) async fn redirect_to_remote() -> impl IntoResponse {
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
pub(super) async fn create_remote_pairing_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Console-only — otherwise any LAN client could mint itself a live
    // remote-control token without ever touching the console or scanning a
    // QR.
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

pub(super) async fn create_pairing_session_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Only the console itself mints pairing sessions — "operator clicks Pair
    // TV App" in the design (docs/CLIENT_PAIRING.md). Without this, any LAN
    // client could mint its own session and self-pair a rogue device.
    // `authorize_pairing_handler` below stays open — it's called by the
    // technician's *phone*, a third device that was never the console — but
    // it's gated in practice by
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

pub(super) async fn authorize_pairing_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
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

    if device_id.len() > 128 || req.name.len() > 128 || req.platform.len() > 64 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "device_id, name or platform too long" })),
        ));
    }

    // The session token is reusable (one console session pairs several TVs)
    // and travels in QR codes/URLs, so it must not let its holder replace an
    // already-paired device's token. Re-pairing an existing id needs the
    // console's host token.
    if matches!(app.db.get_paired_device(device_id), Ok(Some(_))) && require_host_token(&headers, &app).is_err() {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "A device with this ID is already paired. Remove it from the console first."
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

    // Marks this device as owed exactly one token delivery via its own
    // status poll (see `get_pairing_status_handler` and
    // `pending_pairing_token_delivery`'s doc comment) -- the phone that just
    // authorized it doesn't need the token itself, only the TV/Roku device
    // polling its own `device_id` does.
    if let Ok(mut pending) = app.pending_pairing_token_delivery.lock() {
        pending.insert(device.id.clone());
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

/// Unauthenticated by design (docs/CLIENT_PAIRING.md's two-way QR bridge):
/// the TV/Roku app polls this with its own `device_id` before it has any
/// credential at all, waiting for the operator's phone to authorize it.
/// Deliberately does NOT hand out `token` for every poll of every
/// `device_id` forever, though -- `device_id` is an operator/QR-supplied,
/// human-guessable string, not a secret, so that would let any LAN caller
/// read an already-paired device's permanent bearer token by naming it. The
/// token is included exactly once: the first poll after
/// `authorize_pairing_handler` marks it pending delivery (see
/// `pending_pairing_token_delivery`), matching the one moment the real TV
/// app is actually waiting on it.
pub(super) async fn get_pairing_status_handler(
    State(app): State<AppState>,
    Query(query): Query<PairingStatusQuery>,
) -> impl IntoResponse {
    match app.db.get_paired_device(&query.device_id) {
        Ok(Some(dev)) => {
            let _ = app.db.touch_paired_device(&query.device_id);
            let token_owed = app
                .pending_pairing_token_delivery
                .lock()
                .ok()
                .map(|mut pending| pending.remove(&query.device_id))
                .unwrap_or(false);
            Json(serde_json::json!({
                "paired": true,
                "device_id": dev.id,
                "name": dev.name,
                "token": if token_owed { Some(dev.token) } else { None },
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

pub(super) async fn get_paired_devices_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Returns every paired device's permanent bearer token — console-only.
    require_host_token(&headers, &app)?;
    let devices = app.db.get_paired_devices().unwrap_or_default();
    Ok(Json(devices))
}

pub(super) async fn delete_paired_device_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Console-only — otherwise any LAN client could revoke a live sanctuary
    // display mid-service.
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

pub(super) async fn verify_paired_device_handler(
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

/// GET /api/tv-provision/status
///
/// Console-only: reports whether `adb` is on this machine's PATH and
/// whether a bundled TV APK is available (`src/storage/paths.rs::resolve_tv_apk_path`),
/// so the UI can show a clear "ADB not found" / "no TV build available"
/// state before the operator's typed an IP, instead of a confusing failure
/// partway through.
pub(super) async fn get_tv_provision_status_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;
    let apk_path = crate::storage::paths::resolve_tv_apk_path();
    let availability = crate::network::adb::check_availability(apk_path.as_deref()).await;
    Ok(Json(availability))
}

#[derive(Deserialize)]
pub(super) struct StartTvProvisionRequest {
    ip: String,
    port: Option<u16>,
    #[serde(default)]
    is_stage_mode: bool,
    device_name: Option<String>,
}

/// POST /api/tv-provision/start
///
/// Console-only. Mints a pairing session the same way `POST /api/pairing/session`
/// does (`app.create_pairing_session`, not reimplemented), then hands the
/// whole connect/install/launch sequence to `crate::network::adb::provision`
/// as a background task -- returns a `task_id` immediately, same
/// fire-and-forget-plus-poll shape as the yt-dlp import flow
/// (`GET /api/media/ytdlp/progress/:task_id`).
pub(super) async fn start_tv_provision_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<StartTvProvisionRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;

    let ip = req.ip.trim().to_string();
    if ip.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "TV IP address is required" }))));
    }
    if ip.parse::<std::net::IpAddr>().is_err() {
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "TV IP address is not a valid IP" }))));
    }

    let Some(apk_path) = crate::storage::paths::resolve_tv_apk_path() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "No Android TV build is bundled with this install." })),
        ));
    };

    // The TV app connects over HTTPS only (apps/android-tv's MainActivity.kt
    // hardcodes the scheme) -- app.server_port is the loopback-only console
    // plane (src/main.rs) and would be unreachable from the TV's own
    // network connection. If HTTPS itself failed to start, there's no
    // reachable port to hand the TV at all; fail clearly rather than
    // silently provisioning it with a dead address.
    let Some(https_port) = app.https_port else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "HTTPS is not currently running on this console -- it's required for the TV to connect. Check the console's startup log." })),
        ));
    };
    let server_ip = crate::network::get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    let server_port = req.port.unwrap_or(https_port);
    // Passed through `adb shell`, which re-parses it with the TV's shell:
    // restrict to plain characters.
    let device_name = req.device_name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Sanctuary TV".to_string());
    if device_name.len() > 40 || !device_name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_')) {
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Device name may only contain letters, numbers, spaces, '.', '-' and '_' (max 40)." }))));
    }

    // Lets the TV app pin and verify this console's actual certificate from
    // its very first HTTPS connection, instead of trusting any self-signed
    // cert handed to it (PairingManager.kt). Delivered over ADB, not
    // fetched over the network -- the TV already trusts ADB access, so this
    // doesn't depend on the HTTPS connection it's meant to verify.
    let Some(cert_pem) = app.db.get_setting(crate::network::tls::SETTING_KEY_TLS_CERT_PEM).ok().flatten() else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "HTTPS is running but its certificate could not be read -- cannot safely provision the TV." })),
        ));
    };
    let cert_fingerprint = match crate::network::tls::cert_fingerprint_sha256(&cert_pem) {
        Ok(fp) => fp,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to compute certificate fingerprint: {e}") })),
            ));
        }
    };

    // Same session the console-driven "Pair TV App" QR flow mints -- the TV
    // app self-authorizes with it (see PairingManager.kt) instead of a
    // phone scanning a second QR, since ADB access already proves the same
    // level of physical/network trust a phone scan would.
    let pairing_session_token = app.create_pairing_session(300).await;

    let task_id = format!("tvprov_{}", uuid::Uuid::new_v4().simple());
    tokio::spawn(crate::network::adb::provision(
        task_id.clone(),
        ip,
        apk_path,
        server_ip,
        server_port,
        req.is_stage_mode,
        device_name,
        pairing_session_token,
        cert_fingerprint,
    ));

    Ok(Json(serde_json::json!({ "task_id": task_id })))
}

/// GET /api/tv-provision/progress/:task_id
pub(super) async fn get_tv_provision_progress_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;
    match crate::network::adb::get_progress(&task_id) {
        Some(progress) => Ok(Json(serde_json::json!({ "progress": progress }))),
        None => Ok(Json(serde_json::json!({ "progress": null }))),
    }
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{delete, get, post};
    Router::new()
        .route("/remote", get(redirect_to_remote))
        .route("/api/pairing/remote-session", post(create_remote_pairing_handler))
        .route("/api/pairing/session", post(create_pairing_session_handler))
        .route("/api/pairing/authorize", post(authorize_pairing_handler))
        .route("/api/pairing/status", get(get_pairing_status_handler))
        .route("/api/pairing/devices", get(get_paired_devices_handler))
        .route("/api/pairing/devices/:id", delete(delete_paired_device_handler))
        .route("/api/pairing/verify", post(verify_paired_device_handler))
        .route("/api/tv-provision/status", get(get_tv_provision_status_handler))
        .route("/api/tv-provision/start", post(start_tv_provision_handler))
        .route("/api/tv-provision/progress/:task_id", get(get_tv_provision_progress_handler))
}
