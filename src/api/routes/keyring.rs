use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::error::AppError;
use crate::api::ws::AppState;

pub(super) enum KeyringCaller {
    Host,
    Plugin(String),
}

pub(super) async fn authenticate_keyring_caller(
    headers: &HeaderMap,
    app: &AppState,
) -> Result<KeyringCaller, AppError> {
    if let Some(token_header) = headers.get("x-plugin-token") {
        if let Ok(token_str) = token_header.to_str() {
            if let Some(plugin_name) = app.resolve_plugin_token(token_str).await {
                return Ok(KeyringCaller::Plugin(plugin_name));
            }
        }
        return Err(AppError::Unauthorized("Invalid or expired plugin token".to_string()));
    }

    if let Some(token_header) = headers.get("x-host-token") {
        if let Ok(token_str) = token_header.to_str() {
            if token_str == app.host_session_token {
                return Ok(KeyringCaller::Host);
            }
        }
        return Err(AppError::Unauthorized("Invalid host session token".to_string()));
    }

    if !app.host_session_token.is_empty() {
        return Err(AppError::Unauthorized("Missing authorization: x-host-token or x-plugin-token required".to_string()));
    }

    Ok(KeyringCaller::Host)
}

#[derive(Deserialize)]
pub(super) struct KeyringPluginTokenReq {
    plugin_name: String,
}

/// POST /api/keyring/plugin/token
/// Whether `name` matches a plugin file actually present under
/// `<web_dir>/plugins/` — the server's own source of truth for "what plugins
/// exist," rather than trusting whatever name a caller asserts. This closes
/// a known exploit (minting a token for a made-up name like
/// "planning_center" that was never installed at all).
///
/// It does NOT, on its own, stop a malicious script from claiming to be a
/// *real* installed plugin (e.g. "hello_world") — plugins execute in the
/// same JS realm as the host with no Worker/iframe boundary (see
/// `web/src/core/plugins.ts`), so there's no way for the server to verify
/// which script actually made the request without real realm isolation,
/// which is a larger change than this pass covers. Disclosed here rather
/// than implied fixed.
pub(super) fn is_known_plugin_name(app: &AppState, name: &str) -> bool {
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

pub(super) async fn keyring_plugin_token(
    State(app): State<AppState>,
    Json(req): Json<KeyringPluginTokenReq>,
) -> Result<impl IntoResponse, AppError> {
    let name = req.plugin_name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("plugin_name must not be empty".to_string()));
    }
    if !is_known_plugin_name(&app, name) {
        return Err(AppError::NotFound("No installed plugin with that name".to_string()));
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
pub(super) struct KeyringSetReq {
    service: String,
    account: String,
    secret: String,
}

#[derive(Deserialize)]
pub(super) struct KeyringQueryReq {
    service: String,
    account: String,
}

/// POST /api/keyring/set
pub(super) async fn keyring_set(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<KeyringSetReq>,
) -> Result<impl IntoResponse, AppError> {
    if req.service.trim().is_empty() || req.account.trim().is_empty() {
        return Err(AppError::BadRequest("service and account must not be empty".to_string()));
    }

    reject_system_namespace(&req.service)?;
    let caller = authenticate_keyring_caller(&headers, &app).await?;
    let owner_tag = match &caller {
        KeyringCaller::Host => "host".to_string(),
        KeyringCaller::Plugin(name) => {
            let expected_service = format!("OpenSanctuary:Plugin:{}", name);
            if req.service.trim() != expected_service {
                return Err(AppError::Forbidden(format!("Plugins can only create secrets in their own namespace ('{}')", expected_service)));
            }
            format!("plugin:{}", name)
        }
    };

    // Check if an existing credential belongs to a different owner
    if let Ok(Some(existing_owner)) = app.db.get_keyring_owner(req.service.trim(), req.account.trim()) {
        if existing_owner != owner_tag {
            return Err(AppError::Forbidden(format!("Permission denied. Secret is owned by '{}'", existing_owner)));
        }
    }

    crate::storage::KeyringService::set_secret(req.service.trim(), req.account.trim(), &req.secret)
        .map_err(AppError::Internal)?;

    if let Err(e) = app.db.record_keyring_owner(req.service.trim(), req.account.trim(), &owner_tag) {
        tracing::warn!("keyring secret for {}/{} was set but recording ownership metadata failed: {}", req.service.trim(), req.account.trim(), e);
    }

    Ok(Json(serde_json::json!({ "success": true })))
}

/// POST /api/keyring/get
pub(super) async fn keyring_get(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<KeyringQueryReq>,
) -> Result<impl IntoResponse, AppError> {
    if req.service.trim().is_empty() || req.account.trim().is_empty() {
        return Err(AppError::BadRequest("service and account must not be empty".to_string()));
    }

    reject_system_namespace(&req.service)?;
    let caller = authenticate_keyring_caller(&headers, &app).await?;
    if let KeyringCaller::Plugin(name) = &caller {
        let expected_service = format!("OpenSanctuary:Plugin:{}", name);
        if req.service.trim() != expected_service {
            return Err(AppError::Forbidden("Plugins can only view secrets in their own namespace".to_string()));
        }
        let required_owner = format!("plugin:{}", name);
        let owner = app.db.get_keyring_owner(req.service.trim(), req.account.trim()).unwrap_or(None);
        if owner.as_deref() != Some(&required_owner) {
            return Err(AppError::Forbidden("Plugins can only view secrets they created".to_string()));
        }
    }

    let secret = crate::storage::KeyringService::get_secret(req.service.trim(), req.account.trim())
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "success": true, "secret": secret })))
}

/// POST /api/keyring/has
pub(super) async fn keyring_has(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<KeyringQueryReq>,
) -> Result<impl IntoResponse, AppError> {
    if req.service.trim().is_empty() || req.account.trim().is_empty() {
        return Err(AppError::BadRequest("service and account must not be empty".to_string()));
    }

    reject_system_namespace(&req.service)?;
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
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "success": true, "exists": exists })))
}

/// POST /api/keyring/delete
pub(super) async fn keyring_delete(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<KeyringQueryReq>,
) -> Result<impl IntoResponse, AppError> {
    if req.service.trim().is_empty() || req.account.trim().is_empty() {
        return Err(AppError::BadRequest("service and account must not be empty".to_string()));
    }

    reject_system_namespace(&req.service)?;
    let caller = authenticate_keyring_caller(&headers, &app).await?;
    if let KeyringCaller::Plugin(name) = &caller {
        let expected_service = format!("OpenSanctuary:Plugin:{}", name);
        if req.service.trim() != expected_service {
            return Err(AppError::Forbidden("Plugins can only delete secrets in their own namespace".to_string()));
        }
        let required_owner = format!("plugin:{}", name);
        let owner = app.db.get_keyring_owner(req.service.trim(), req.account.trim()).unwrap_or(None);
        if owner.as_deref() != Some(&required_owner) {
            return Err(AppError::Forbidden("Plugins can only delete secrets they created".to_string()));
        }
    }

    let deleted = crate::storage::KeyringService::delete_secret(req.service.trim(), req.account.trim())
        .map_err(AppError::Internal)?;

    if let Err(e) = app.db.delete_keyring_metadata(req.service.trim(), req.account.trim()) {
        tracing::warn!("keyring secret for {}/{} was deleted but clearing ownership metadata failed: {}", req.service.trim(), req.account.trim(), e);
    }

    Ok(Json(serde_json::json!({ "success": true, "deleted": deleted })))
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::post;
    Router::new()
        .route("/api/keyring/set", post(keyring_set))
        .route("/api/keyring/get", post(keyring_get))
        .route("/api/keyring/has", post(keyring_has))
        .route("/api/keyring/delete", post(keyring_delete))
        .route("/api/keyring/plugin/token", post(keyring_plugin_token))
}

/// `OpenSanctuary:System` holds the TLS private key; it is managed only by
/// the server itself, never through this HTTP surface.
fn reject_system_namespace(service: &str) -> Result<(), AppError> {
    if service.trim().eq_ignore_ascii_case("OpenSanctuary:System") {
        return Err(AppError::Forbidden("The system keyring namespace is not accessible over HTTP".to_string()));
    }
    Ok(())
}
