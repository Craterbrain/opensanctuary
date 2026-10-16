use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::error::AppError;
use crate::api::keyring_policy::{self, Op, Verdict};
use crate::api::ws::AppState;
use crate::core::plugin_permissions::PermissionSet;

pub(super) enum KeyringCaller {
    Host,
    /// The plugin's id and what its token carries (already narrowed to its live grant).
    Plugin { id: String, scopes: PermissionSet },
}

pub(super) async fn authenticate_keyring_caller(
    headers: &HeaderMap,
    app: &AppState,
) -> Result<KeyringCaller, AppError> {
    if let Some(token_header) = headers.get("x-plugin-token") {
        if let Ok(token_str) = token_header.to_str() {
            if let Some(caller) = app.resolve_plugin_caller(token_str).await {
                if caller.scopes.has("keyring.own") || caller.scopes.has("keyring.service") {
                    return Ok(KeyringCaller::Plugin { id: caller.plugin_id, scopes: caller.scopes });
                }
            }
        }
        return Err(AppError::Unauthorized("Invalid or expired plugin token".to_string()));
    }

    if let Some(token_header) = headers.get("x-host-token") {
        if let Ok(token_str) = token_header.to_str() {
            if app.is_console_credential(token_str) {
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
    // A token unlocks the plugin's keyring namespace, so it is only minted for a plugin the
    // operator has approved (and not since changed) with `keyring.own` granted.
    let row = match app.db.get_plugin_grant(name).ok().flatten() {
        Some(r) => Some(r),
        None => {
            let app2 = app.clone();
            let _ = tokio::task::spawn_blocking(move || super::plugins::rescan(&app2)).await;
            app.db.get_plugin_grant(name).ok().flatten()
        }
    };
    let allowed = row.as_ref().is_some_and(|r| {
        crate::core::plugin_registry::is_usable(&app.db, &app.data_dir, r) && crate::core::plugin_registry::parse_granted(r).has("keyring.own")
    });
    if !allowed {
        return Err(AppError::Forbidden(
            "This plugin hasn't been approved with keyring access (Settings > Plugins)".to_string(),
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
    let svc = req.service.trim();
    let acct = req.account.trim();
    let owner_tag = match &caller {
        KeyringCaller::Host if keyring_policy::in_shared_area(svc) => {
            // The console manages every shared row, including ones a plugin created.
            keyring_policy::HOST_OWNER_TAG.to_string()
        }
        KeyringCaller::Host => "host".to_string(),
        KeyringCaller::Plugin { id, scopes } => {
            let own_service = format!("OpenSanctuary:Plugin:{}", id);
            if svc == own_service {
                if !scopes.has("keyring.own") {
                    return Err(AppError::Forbidden("This plugin wasn't granted its own keyring namespace".to_string()));
                }
                format!("plugin:{}", id)
            } else if keyring_policy::in_shared_area(svc) {
                match keyring_policy::check(&app.db, id, scopes, Op::Set, svc, acct) {
                    Verdict::Allow { owner } => {
                        keyring_policy::audit_once(&app.db, id, svc, acct, Op::Set, false);
                        owner
                    }
                    Verdict::Deny(why) => {
                        keyring_policy::audit_once(&app.db, id, svc, acct, Op::Set, true);
                        return Err(AppError::Forbidden(why));
                    }
                    Verdict::NotFound => return Err(AppError::Forbidden("not allowed".to_string())),
                }
            } else {
                return Err(AppError::Forbidden(format!("Plugins can only create secrets in their own namespace ('{}') or a shared one they were granted", own_service)));
            }
        }
    };

    // Check if an existing credential belongs to a different owner (shared rows were decided above).
    let shared_row = keyring_policy::in_shared_area(svc);
    if !shared_row {
        if let Ok(Some(existing_owner)) = app.db.get_keyring_owner(svc, acct) {
            if existing_owner != owner_tag {
                return Err(AppError::Forbidden(format!("Permission denied. Secret is owned by '{}'", existing_owner)));
            }
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
    if let KeyringCaller::Plugin { id, scopes } = &caller {
        let svc = req.service.trim();
        let acct = req.account.trim();
        let own_service = format!("OpenSanctuary:Plugin:{}", id);
        if svc == own_service {
            if !scopes.has("keyring.own") {
                return Err(AppError::Forbidden("This plugin wasn't granted its own keyring namespace".to_string()));
            }
            let required_owner = format!("plugin:{}", id);
            let owner = app.db.get_keyring_owner(svc, acct).unwrap_or(None);
            if owner.as_deref() != Some(&required_owner) {
                return Err(AppError::Forbidden("Plugins can only view secrets they created".to_string()));
            }
        } else if keyring_policy::in_shared_area(svc) {
            match keyring_policy::check(&app.db, id, scopes, Op::Get, svc, acct) {
                Verdict::Allow { .. } => keyring_policy::audit_once(&app.db, id, svc, acct, Op::Get, false),
                _ => {
                    keyring_policy::audit_once(&app.db, id, svc, acct, Op::Get, true);
                    return Ok(Json(serde_json::json!({ "success": true, "secret": serde_json::Value::Null })));
                }
            }
        } else {
            return Err(AppError::Forbidden("Plugins can only view secrets in their own namespace or a shared one they were granted".to_string()));
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
    if let KeyringCaller::Plugin { id, scopes } = &caller {
        let svc = req.service.trim();
        let acct = req.account.trim();
        let not_found = || Ok(Json(serde_json::json!({ "success": true, "exists": false })));
        let own_service = format!("OpenSanctuary:Plugin:{}", id);
        if svc == own_service {
            if !scopes.has("keyring.own") {
                return not_found();
            }
            let required_owner = format!("plugin:{}", id);
            let owner = app.db.get_keyring_owner(svc, acct).unwrap_or(None);
            if owner.as_deref() != Some(&required_owner) {
                return not_found();
            }
        } else if keyring_policy::in_shared_area(svc) {
            if !matches!(keyring_policy::check(&app.db, id, scopes, Op::Has, svc, acct), Verdict::Allow { .. }) {
                return not_found();
            }
        } else {
            return not_found();
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
    if let KeyringCaller::Plugin { id, scopes } = &caller {
        let svc = req.service.trim();
        let acct = req.account.trim();
        let own_service = format!("OpenSanctuary:Plugin:{}", id);
        if svc == own_service {
            if !scopes.has("keyring.own") {
                return Err(AppError::Forbidden("This plugin wasn't granted its own keyring namespace".to_string()));
            }
            let required_owner = format!("plugin:{}", id);
            let owner = app.db.get_keyring_owner(svc, acct).unwrap_or(None);
            if owner.as_deref() != Some(&required_owner) {
                return Err(AppError::Forbidden("Plugins can only delete secrets they created".to_string()));
            }
        } else if keyring_policy::in_shared_area(svc) {
            match keyring_policy::check(&app.db, id, scopes, Op::Delete, svc, acct) {
                Verdict::Allow { .. } => keyring_policy::audit_once(&app.db, id, svc, acct, Op::Delete, false),
                Verdict::NotFound => return Err(AppError::Forbidden("not allowed".to_string())),
                Verdict::Deny(m) => {
                    keyring_policy::audit_once(&app.db, id, svc, acct, Op::Delete, true);
                    return Err(AppError::Forbidden(m));
                }
            }
        } else {
            return Err(AppError::Forbidden("Plugins can only delete secrets in their own namespace or a shared one they were granted".to_string()));
        }
    }

    let deleted = crate::storage::KeyringService::delete_secret(req.service.trim(), req.account.trim())
        .map_err(AppError::Internal)?;

    if let Err(e) = app.db.delete_keyring_metadata(req.service.trim(), req.account.trim()) {
        tracing::warn!("keyring secret for {}/{} was deleted but clearing ownership metadata failed: {}", req.service.trim(), req.account.trim(), e);
    }

    Ok(Json(serde_json::json!({ "success": true, "deleted": deleted })))
}

/// GET /api/keyring/shared -- which shared-namespace entries exist and who owns them. Console only;
/// it never returns a secret's value.
pub(super) async fn keyring_shared_list(State(app): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, AppError> {
    if !matches!(authenticate_keyring_caller(&headers, &app).await?, KeyringCaller::Host) {
        return Err(AppError::Forbidden("Only the console can list shared secrets".to_string()));
    }
    let entries = app.db.list_shared_keyring_entries().map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "entries": entries.into_iter().map(|(service, account, owner)| serde_json::json!({ "service": service, "account": account, "owner": owner })).collect::<Vec<_>>(),
    })))
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/keyring/shared", get(keyring_shared_list))
        .route("/api/keyring/set", post(keyring_set))
        .route("/api/keyring/get", post(keyring_get))
        .route("/api/keyring/has", post(keyring_has))
        .route("/api/keyring/delete", post(keyring_delete))
        .route("/api/keyring/plugin/token", post(keyring_plugin_token))
}

/// `OpenSanctuary:System` holds the TLS private key; it is managed only by
/// the server itself, never through this HTTP surface.
fn reject_system_namespace(service: &str) -> Result<(), AppError> {
    // Anything starting with the System prefix, in any case (not just the exact name): the TLS key's namespace.
    if service.trim().to_ascii_lowercase().starts_with("opensanctuary:system") {
        return Err(AppError::Forbidden("The system keyring namespace is not accessible over HTTP".to_string()));
    }
    Ok(())
}
