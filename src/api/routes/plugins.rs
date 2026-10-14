//! Console-only management of plugin permissions (docs/PLUGIN_PERMISSIONS.md).
//!
//! Every route here needs the console's host token or a persistent console credential
//! (`require_host_token`). Paired-device tokens and plugin tokens are never accepted,
//! so a plugin can't approve itself and a TV or phone can't grant anything.

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::ws::AppState;
use crate::core::plugin_permissions::{PermissionRequest, PermissionSet};
use crate::core::plugin_registry::{discover, sync, Discovered};
use crate::storage::db::PluginGrantRow;

use super::common::require_host_token;

type ApiErr = (StatusCode, Json<serde_json::Value>);

fn err(status: StatusCode, msg: &str) -> ApiErr {
    (status, Json(serde_json::json!({ "success": false, "error": msg })))
}

pub(super) fn rescan(app: &AppState) -> Vec<Discovered> {
    let found = discover(
        &crate::storage::paths::plugin_dir(),
        &app.web_dir.join("plugins"),
        env!("CARGO_PKG_VERSION"),
    );
    sync(&app.db, &found);
    found
}

fn view(app: &AppState, row: &PluginGrantRow) -> serde_json::Value {
    let requested: Vec<PermissionRequest> = serde_json::from_str(&row.requested_json).unwrap_or_default();
    let granted: Vec<PermissionRequest> = serde_json::from_str(&row.granted_json).unwrap_or_default();
    let state = row.effective_state();
    let describe = |p: &PermissionRequest| {
        let mut v = serde_json::to_value(p).unwrap_or_default();
        v["tier"] = serde_json::to_value(p.tier()).unwrap_or_default();
        v["enforcement"] = serde_json::Value::String(p.enforcement().to_string());
        v
    };
    let native = row.kind == "native" || row.kind == "legacy-native";
    serde_json::json!({
        "id": row.plugin_id,
        "kind": row.kind,
        "name": row.name,
        "version": row.version,
        "state": state,
        "status": row.status,
        "error": row.error,
        "digest": row.observed_digest,
        "approved_digest": row.approved_digest,
        "requested": requested.iter().map(describe).collect::<Vec<_>>(),
        "granted": granted.iter().map(describe).collect::<Vec<_>>(),
        "decided_at": row.decided_at,
        "decided_by": row.decided_by,
        "first_seen": row.first_seen,
        "loaded": app.plugin_manager.is_loaded(&row.plugin_id),
        // A native library is only loaded at startup, so approving it takes effect then.
        "restart_required": native && state == "active" && !app.plugin_manager.is_loaded(&row.plugin_id),
    })
}

#[derive(Deserialize)]
pub(super) struct ListQuery {
    kind: Option<String>,
    active: Option<String>,
}

/// GET /api/plugins -- rescans the plugin folders, then lists every plugin with its state.
pub(super) async fn list_plugins(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    let app2 = app.clone();
    tokio::task::spawn_blocking(move || rescan(&app2)).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "scan failed"))?;
    let rows = app.db.list_plugin_grants().map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    let only_active = q.active.as_deref() == Some("1");
    let list: Vec<_> = rows
        .iter()
        .filter(|r| match q.kind.as_deref() {
            Some("web") => r.kind == "web" || r.kind == "legacy-web",
            Some("native") => r.kind == "native" || r.kind == "legacy-native",
            _ => true,
        })
        .filter(|r| !only_active || r.effective_state() == "active")
        .map(|r| view(&app, r))
        .collect();
    Ok(Json(list))
}

#[derive(Deserialize)]
pub(super) struct ApproveReq {
    /// The digest the operator reviewed; refused if the files have changed since.
    digest: String,
    /// Ids of the requested permissions being granted (all non-optional ones are required).
    granted: Vec<String>,
}

fn actor_of(headers: &HeaderMap, app: &AppState) -> String {
    // The per-run host token is "host"; a persistent console credential is attributed to
    // the browser's paired-device entry so the audit trail says which console decided.
    let presented = headers.get("x-host-token").and_then(|v| v.to_str().ok()).unwrap_or("");
    if crate::api::ct_eq(presented, &app.host_session_token) {
        return "host".to_string();
    }
    let hashed = crate::storage::db::console_credential_hash(presented);
    match app.db.get_paired_device_by_token(&hashed) {
        Ok(Some(dev)) => format!("console:{}", dev.id),
        _ => "console".to_string(),
    }
}

/// POST /api/plugins/:id/approve
pub(super) async fn approve_plugin(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<ApproveReq>,
) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    let app2 = app.clone();
    tokio::task::spawn_blocking(move || rescan(&app2)).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "scan failed"))?;
    let row = app
        .db
        .get_plugin_grant(&id)
        .ok()
        .flatten()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "no such plugin"))?;
    if let Some(e) = &row.error {
        return Err(err(StatusCode::BAD_REQUEST, &format!("this plugin can't be approved: {e}")));
    }
    if req.digest != row.observed_digest {
        return Err(err(StatusCode::CONFLICT, "the plugin changed since it was reviewed; reload and review again"));
    }
    let requested: Vec<PermissionRequest> = serde_json::from_str(&row.requested_json).unwrap_or_default();
    for p in requested.iter().filter(|p| !p.optional) {
        if !req.granted.iter().any(|g| g == &p.id) {
            return Err(err(StatusCode::BAD_REQUEST, &format!("'{}' is required by this plugin", p.id)));
        }
    }
    if let Some(unknown) = req.granted.iter().find(|g| !requested.iter().any(|p| &p.id == *g)) {
        return Err(err(StatusCode::BAD_REQUEST, &format!("'{unknown}' was not requested by this plugin")));
    }
    let granted: Vec<PermissionRequest> = requested.into_iter().filter(|p| req.granted.contains(&p.id)).collect();
    let granted_json = serde_json::to_string(&granted).unwrap_or_else(|_| "[]".to_string());
    let actor = actor_of(&headers, &app);
    app.db
        .set_plugin_decision(&id, "approved", &granted_json, row.observed_manifest_sha256.as_deref(), Some(&row.observed_digest), &actor)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    app.db.add_plugin_audit(
        &id,
        &actor,
        "approved",
        &serde_json::json!({ "granted": granted.iter().map(|p| &p.id).collect::<Vec<_>>(), "digest": row.observed_digest }).to_string(),
    );
    // A plugin that's already loaded (re-approval after a revoke) picks its grants up now.
    app.plugin_manager.set_grants(&id, PermissionSet(granted));
    let fresh = app.db.get_plugin_grant(&id).ok().flatten().ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    Ok(Json(view(&app, &fresh)))
}

async fn decide(
    app: AppState,
    headers: HeaderMap,
    id: String,
    status: &'static str,
) -> Result<Json<serde_json::Value>, ApiErr> {
    require_host_token(&headers, &app)?;
    if app.db.get_plugin_grant(&id).ok().flatten().is_none() {
        return Err(err(StatusCode::NOT_FOUND, "no such plugin"));
    }
    let actor = actor_of(&headers, &app);
    app.db
        .set_plugin_decision(&id, status, "[]", None, None, &actor)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    app.db.add_plugin_audit(&id, &actor, if status == "denied" { "denied" } else { "revoked" }, "{}");
    // Immediate for everything the host mediates (hooks, resources).
    app.plugin_manager.revoke(&id);
    let fresh = app.db.get_plugin_grant(&id).ok().flatten().ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    Ok(Json(view(&app, &fresh)))
}

pub(super) async fn deny_plugin(State(app): State<AppState>, headers: HeaderMap, AxumPath(id): AxumPath<String>) -> Result<impl IntoResponse, ApiErr> {
    decide(app, headers, id, "denied").await
}

pub(super) async fn revoke_plugin(State(app): State<AppState>, headers: HeaderMap, AxumPath(id): AxumPath<String>) -> Result<impl IntoResponse, ApiErr> {
    decide(app, headers, id, "revoked").await
}

#[derive(Deserialize)]
pub(super) struct AuditQuery {
    limit: Option<i64>,
}

/// GET /api/plugins/audit
pub(super) async fn plugin_audit(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<AuditQuery>,
) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    let rows = app.db.list_plugin_audit(q.limit.unwrap_or(200)).map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    Ok(Json(rows))
}

/// True when `file_name` (e.g. `hello_world.js`) is an approved, unchanged web plugin.
/// The file is re-hashed on every request, so editing it after approval stops it being
/// served until it is reviewed again.
pub(crate) fn web_plugin_file_is_active(app: &AppState, file_name: &str) -> bool {
    let Some(stem) = file_name.strip_suffix(".js") else { return false };
    let Some(row) = app.db.get_plugin_grant(stem).ok().flatten() else { return false };
    if !(row.kind == "web" || row.kind == "legacy-web") || row.effective_state() != "active" {
        return false;
    }
    let path = app.web_dir.join("plugins").join(file_name);
    match crate::core::plugin_permissions::sha256_file(&path) {
        Ok(sha) => {
            crate::core::plugin_permissions::artifact_digest(&[(file_name.to_string(), sha)])
                == row.approved_digest.clone().unwrap_or_default()
        }
        Err(_) => false,
    }
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/audit", get(plugin_audit))
        .route("/api/plugins/:id/approve", post(approve_plugin))
        .route("/api/plugins/:id/deny", post(deny_plugin))
        .route("/api/plugins/:id/revoke", post(revoke_plugin))
}
