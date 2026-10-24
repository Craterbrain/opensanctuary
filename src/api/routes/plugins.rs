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
use crate::core::plugin_registry::{check_plugin_folder, discover_all, parse_granted, sync, user_plugins_dir, Discovered};
use crate::storage::db::PluginGrantRow;

use super::common::require_host_token;

type ApiErr = (StatusCode, Json<serde_json::Value>);

fn err(status: StatusCode, msg: &str) -> ApiErr {
    (status, Json(serde_json::json!({ "success": false, "error": msg })))
}

pub(super) fn rescan(app: &AppState) -> Vec<Discovered> {
    // A newer signed revocation list takes effect here, before anything is judged by publisher.
    crate::core::plugin_revocation::apply(&app.db, &app.plugin_release_key, &crate::core::plugin_revocation::list_paths(&app.web_dir, &app.data_dir));
    let found = discover_all(&app.web_dir.join("plugins"), Some(&user_plugins_dir(&app.data_dir)), env!("CARGO_PKG_VERSION"));
    sync(&app.db, &found);
    found
}

/// Is this plugin's folder one that ships with the app (rather than one the operator installed)?
fn is_builtin(app: &AppState, row: &PluginGrantRow) -> bool {
    crate::core::plugin_registry::is_builtin(&app.data_dir, std::path::Path::new(&row.source_path))
}

/// Whether `row` is approved, unchanged, and allowed to load under the console's plugin
/// signature policy (docs/PLUGIN_SIGNING.md section 6) -- the policy-aware replacement for a
/// bare `row.effective_state() == "active"` check everywhere a plugin is actually served, not
/// just displayed.
fn plugin_usable(app: &AppState, row: &PluginGrantRow) -> bool {
    crate::core::plugin_registry::is_usable(&app.db, &app.data_dir, row)
}

/// The console's current plugin signature policy setting (docs/PLUGIN_SIGNING.md section 6),
/// `allow` unless the operator set it otherwise in Settings > Plugins.
fn plugin_signature_policy(app: &AppState) -> crate::core::plugin_policy::Policy {
    crate::core::plugin_policy::Policy::parse(app.db.get_setting(crate::core::plugin_policy::SETTING_KEY).ok().flatten().as_deref())
}

/// The keys of every plugin the operator has approved: a signature by one of these is a "known" publisher.
fn known_keys(app: &AppState) -> std::collections::HashSet<String> {
    app.db.list_plugin_grants().unwrap_or_default().into_iter().filter(|r| r.status == "approved").filter_map(|r| r.approved_signer).collect()
}

/// The key a plugin should be judged by: its own, or the key it proved it replaces (a rotation statement signed by
/// that old key) unless the old key was revoked. Returns `(effective key, the old key if rotation is being honored)`.
fn effective_signer(app: &AppState, row: &PluginGrantRow) -> (Option<String>, Option<String>) {
    let Some(new_key) = row.observed_signer.clone() else { return (None, None) };
    if let Some(old) = row.observed_rotation_from.clone() {
        let old_revoked = app.db.get_publisher(&old).ok().flatten().is_some_and(|p| p.revoked);
        if !old_revoked && old != new_key {
            return (Some(old.clone()), Some(old));
        }
    }
    (Some(new_key), None)
}

fn trust_view(app: &AppState, row: &PluginGrantRow) -> serde_json::Value {
    use crate::core::plugin_trust::{trust_level, TrustLevel};
    let (effective, rotated_from) = effective_signer(app, row);
    let publisher = effective.as_deref().and_then(|k| app.db.get_publisher(k).ok().flatten());
    let builtin = is_builtin(app, row);
    let level = trust_level(row, builtin, publisher.as_ref(), &known_keys(app), effective.as_deref(), &app.plugin_release_key);
    let release_signed = row.observed_signer.as_deref() == Some(app.plugin_release_key.as_str()) && row.error.is_none();
    // The manifest's `publisher` is just text the plugin wrote about itself. It is only vouched for when the plugin
    // is built in, signed by the release key, or the operator trusted this key (then it is the name they saw then).
    let claimed = read_manifest(row).map(|m| m.publisher).filter(|n| !n.is_empty());
    let vouched = matches!(level, TrustLevel::Builtin) || release_signed || level == TrustLevel::Trusted;
    let name = if release_signed {
        Some("OpenSanctuary".to_string())
    } else if level == TrustLevel::Trusted {
        publisher.as_ref().map(|p| p.name.clone()).filter(|n| !n.is_empty()).or(claimed)
    } else {
        claimed
    };
    let new_key_trusted = row.observed_signer.as_deref().and_then(|k| app.db.get_publisher(k).ok().flatten()).is_some_and(|p| p.trusted);
    serde_json::json!({
        "level": level.as_str(),
        "publisher_name": name,
        "name_is_claim": !vouched && name.is_some(),
        "key_id": row.observed_signer.as_deref().map(signer_id),
        "release_signed": release_signed,
        // Signed by a key the publisher proved (with the old key's signature) it moved to: the old key's standing carries over.
        "rotated_from_key_id": rotated_from.as_deref().map(signer_id),
        // The operator can choose to trust a signed publisher that isn't the app's own, trusted already or revoked.
        "can_trust": !release_signed && (matches!(level, TrustLevel::New | TrustLevel::Known) || (rotated_from.is_some() && !new_key_trusted)),
    })
}

/// Short, stable id for a publisher key, for display.
fn signer_id(key: &str) -> String {
    crate::core::plugin_trust::key_id(key)
}

fn read_manifest(row: &PluginGrantRow) -> Option<crate::core::plugin_permissions::PluginManifest> {
    std::fs::read(std::path::Path::new(&row.source_path).join("plugin.json")).ok().and_then(|b| crate::core::plugin_permissions::PluginManifest::parse(&b).ok())
}

fn view(app: &AppState, row: &PluginGrantRow) -> serde_json::Value {
    let requested: Vec<PermissionRequest> = serde_json::from_str(&row.requested_json).unwrap_or_default();
    let granted: Vec<PermissionRequest> = serde_json::from_str(&row.granted_json).unwrap_or_default();
    // Approved and unchanged, but the console's signature policy stops it from actually loading
    // right now (docs/PLUGIN_SIGNING.md section 6) -- a distinct, honest state rather than
    // silently claiming "active" for something that in fact isn't running.
    let state = if row.effective_state() == "active" && !plugin_usable(app, row) { "policy_blocked" } else { row.effective_state() };
    let describe = |p: &PermissionRequest| {
        let mut v = serde_json::to_value(p).unwrap_or_default();
        v["tier"] = serde_json::to_value(p.tier()).unwrap_or_default();
        v["enforcement"] = serde_json::Value::String(p.enforcement_for(&row.kind).to_string());
        v
    };
    // A sandboxed plugin's entry page comes from its manifest (the folder is the plugin).
    let entry = (row.kind == "sandbox")
        .then(|| std::fs::read(std::path::Path::new(&row.source_path).join("plugin.json")).ok())
        .flatten()
        .and_then(|b| crate::core::plugin_permissions::PluginManifest::parse(&b).ok())
        .and_then(|m| m.web.map(|w| w.entry));
    // What a sandboxed plugin can run, exactly as the operator must read it at review.
    let (commands, tools) = match (row.kind == "sandbox").then(|| read_manifest(row)).flatten() {
        Some(m) => (
            m.commands
                .iter()
                .map(|c| serde_json::json!({
                    "id": c.id, "tool": c.tool, "line": c.display(),
                    "params": c.params.iter().map(|(k, p)| serde_json::json!({
                        "name": k, "type": p.kind, "values": p.values, "optional": p.optional,
                    })).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
            m.tools
                .iter()
                .map(|t| {
                    let key = std::fs::read(std::path::Path::new(&row.source_path).join(&t.signature.key_file)).unwrap_or_default();
                    serde_json::json!({
                        "id": t.id, "github": t.github, "assets": t.assets, "checksums": t.checksums,
                        "signature": t.signature.asset, "system_command": t.system_command,
                        "key_sha256": crate::core::plugin_permissions::sha256_hex(&key),
                    })
                })
                .collect::<Vec<_>>(),
        ),
        None => (Vec::new(), Vec::new()),
    };
    let version_floor = crate::core::plugin_registry::version_floor(&app.db, &row.plugin_id, row.observed_signer.as_deref());
    let is_downgrade_from_floor = crate::core::plugin_registry::is_downgrade_from_floor(version_floor.as_deref(), row.version.as_deref());
    // Granted differently from what the plugin asked for (the Advanced override): extra, dropped or re-scoped permissions.
    let diff = override_summary(&requested, &granted);
    let non_empty = |k: &str| diff[k].as_array().is_some_and(|a| !a.is_empty());
    // Dropping an *optional* permission is an ordinary choice; extra, re-scoped or missing required ones are an override.
    let overridden = state == "active"
        && (non_empty("added") || non_empty("changed") || requested.iter().filter(|r| !r.optional).any(|r| !granted.iter().any(|g| g.id == r.id)));
    let override_diff = overridden.then(|| diff.clone());
    // Other plugins that share a keyring namespace this one asks for or holds, so the operator sees who else can read it.
    let shared_with = {
        let mine: Vec<&PermissionRequest> = (if state == "active" { &granted } else { &requested }).iter().filter(|p| p.id == "keyring.service").collect();
        let others = app.db.list_plugin_grants().unwrap_or_default();
        let mut out = Vec::new();
        for svc in mine.iter().flat_map(|p| p.services.iter()) {
            let mut plugins = Vec::new();
            for o in others.iter().filter(|o| o.plugin_id != row.plugin_id) {
                let o_usable = plugin_usable(app, o);
                let theirs: Vec<PermissionRequest> = serde_json::from_str(if o_usable { &o.granted_json } else { &o.requested_json }).unwrap_or_default();
                if let Some(p) = theirs.iter().find(|p| p.id == "keyring.service" && p.services.contains(svc)) {
                    plugins.push(serde_json::json!({ "id": o.plugin_id, "name": o.name, "access": p.access.clone().unwrap_or_else(|| "read".to_string()), "state": if o_usable { "active" } else { o.effective_state() } }));
                }
            }
            out.push(serde_json::json!({ "service": svc, "plugins": plugins }));
        }
        out
    };
    let manifest = read_manifest(row);
    serde_json::json!({
        "shared_with": shared_with,
        "overridden": overridden,
        "override_diff": override_diff,
        "commands": commands,
        "tools": tools,
        "placements": manifest.as_ref().and_then(|m| m.ui.as_ref()).map(|u| u.placements.clone()).unwrap_or_default(),
        // Ribbon buttons come from the manifest, which a sandboxed plugin's digest covers.
        "toolbar": manifest.as_ref().filter(|_| row.kind == "sandbox").and_then(|m| m.ui.as_ref()).map(|u| u.toolbar.clone()).unwrap_or_default(),
        // What it says it does, for the console's "available plugins" hints (Store, setup) -- display only.
        "description": manifest.as_ref().map(|m| m.description.clone()).unwrap_or_default(),
        "entry": entry,
        "id": row.plugin_id,
        "kind": row.kind,
        "name": row.name,
        "version": row.version,
        // The highest version ever approved from this same signer, and whether this one is
        // older than that -- holds across revoke/re-approve and uninstall/reinstall, unlike
        // `changes.direction` below, which only compares against the single most recent
        // approval (docs/PLUGIN_SIGNING.md section 7).
        "version_floor": version_floor,
        "is_downgrade_from_floor": is_downgrade_from_floor,
        "sandboxed": row.kind == "sandbox",
        "changes": crate::core::plugin_registry::describe_update(row),
        "trust": trust_view(app, row),
        "removable": row.kind == "sandbox" && is_installed_folder(app, std::path::Path::new(&row.source_path)),
        "state": state,
        "status": row.status,
        "error": row.error,
        "digest": row.observed_digest,
        "signed": row.observed_signer.is_some(),
        // What the signature covers: "unsigned", "manifest" (plugin.json only) or "full" (every file). Rows seen
        // before this was recorded are "manifest" if signed, which is all they could have been.
        "signature_scope": row.observed_signature_scope.clone().unwrap_or_else(|| if row.observed_signer.is_some() { "manifest".to_string() } else { "unsigned".to_string() }),
        // It was fully signed when approved and isn't now: less assurance than the operator agreed to.
        "signature_weaker": row.approved_signature_scope.as_deref() == Some("full") && row.observed_signature_scope.as_deref() != Some("full"),
        "signer_id": row.observed_signer.as_deref().map(signer_id),
        // Approved before under a different publisher key, or it used to be signed and now isn't.
        "signer_changed": row.approved_signer.is_some() && row.approved_signer != row.observed_signer
            && !(row.observed_rotation_from.is_some() && row.observed_rotation_from == row.approved_signer
                && !row.observed_rotation_from.as_deref().and_then(|k| app.db.get_publisher(k).ok().flatten()).is_some_and(|p| p.revoked)),
        "approved_digest": row.approved_digest,
        "requested": requested.iter().map(describe).collect::<Vec<_>>(),
        "granted": granted.iter().map(describe).collect::<Vec<_>>(),
        "decided_at": row.decided_at,
        "decided_by": row.decided_by,
        "first_seen": row.first_seen,
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
            Some("web") => r.kind == "web" || r.kind == "sandbox",
            _ => true,
        })
        .filter(|r| !only_active || plugin_usable(&app, r))
        .map(|r| view(&app, r))
        .collect();
    Ok(Json(list))
}

#[derive(Deserialize)]
pub(super) struct ApproveReq {
    /// The digest the operator reviewed; refused if the files have changed since.
    digest: String,
    /// Ids of the requested permissions being granted (all non-optional ones are required).
    #[serde(default)]
    granted: Vec<String>,
    /// The Advanced override: the exact permissions to grant, replacing the plugin's own request.
    /// May drop permissions it asked for (even required ones), change their scopes and add ones it
    /// never asked for. When present, `granted` is ignored. The console shows a warning first.
    #[serde(default, rename = "override")]
    override_grants: Option<Vec<PermissionRequest>>,
    /// Also trust this plugin's publisher (its signing key) from now on. Needs a signed plugin.
    #[serde(default)]
    trust_publisher: bool,
    /// Required to approve a plugin signed by a publisher the operator revoked.
    #[serde(default)]
    confirm_revoked_publisher: bool,
    /// Required to approve an unsigned plugin when the console's signature policy is `warn`, or
    /// when it's being granted a Critical permission regardless of policy (docs/PLUGIN_SIGNING.md
    /// section 6).
    #[serde(default)]
    confirm_unsigned: bool,
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
    let publisher = row.observed_signer.as_deref().and_then(|k| app.db.get_publisher(k).ok().flatten());
    let revoked_publisher = publisher.as_ref().is_some_and(|p| p.revoked);
    if revoked_publisher && !req.confirm_revoked_publisher {
        return Err(err(StatusCode::CONFLICT, "this plugin is signed by a publisher you revoked; confirm to approve it anyway"));
    }
    if req.trust_publisher && (row.observed_signer.is_none() || revoked_publisher) {
        return Err(err(StatusCode::BAD_REQUEST, "only a signed plugin from a publisher you haven't revoked can be trusted"));
    }
    let signature_policy = plugin_signature_policy(&app);
    let builtin = is_builtin(&app, &row);
    let signed = row.observed_signer.is_some();
    // `granting_critical` doesn't matter for `Refused` (policy `require` refuses an unsigned,
    // non-built-in plugin outright, whatever it asks for), so this early check can run before
    // `granted` exists yet.
    if crate::core::plugin_policy::approval_gate(signature_policy, builtin, signed, false) == crate::core::plugin_policy::ApprovalGate::Refused {
        return Err(err(
            StatusCode::FORBIDDEN,
            "this console's plugin signature policy (Settings > Plugins) requires a signed plugin or a built-in; loosen it to Warn or Allow to approve this one",
        ));
    }
    let requested: Vec<PermissionRequest> = serde_json::from_str(&row.requested_json).unwrap_or_default();
    let mut override_diff = None;
    let granted: Vec<PermissionRequest> = if let Some(custom) = req.override_grants {
        let mut seen = std::collections::HashSet::new();
        for p in &custom {
            crate::core::plugin_permissions::validate_grant(p, &row.kind).map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
            if !seen.insert(p.id.clone()) {
                return Err(err(StatusCode::BAD_REQUEST, &format!("'{}' is listed twice", p.id)));
            }
        }
        override_diff = Some(override_summary(&requested, &custom));
        // Keep the plugin's own reason text where the permission was one it asked for.
        custom
            .into_iter()
            .map(|mut p| {
                p.reason = requested.iter().find(|r| r.id == p.id).and_then(|r| r.reason.clone());
                p.optional = false;
                p
            })
            .collect()
    } else {
        for p in requested.iter().filter(|p| !p.optional) {
            if !req.granted.iter().any(|g| g == &p.id) {
                return Err(err(StatusCode::BAD_REQUEST, &format!("'{}' is required by this plugin", p.id)));
            }
        }
        if let Some(unknown) = req.granted.iter().find(|g| !requested.iter().any(|p| &p.id == *g)) {
            return Err(err(StatusCode::BAD_REQUEST, &format!("'{unknown}' was not requested by this plugin")));
        }
        requested.into_iter().filter(|p| req.granted.contains(&p.id)).collect()
    };
    let granting_critical = granted.iter().any(|p| p.tier() == crate::core::plugin_permissions::Tier::Critical);
    let unsigned_confirmation_needed =
        crate::core::plugin_policy::approval_gate(signature_policy, builtin, signed, granting_critical) == crate::core::plugin_policy::ApprovalGate::NeedsConfirmation;
    if unsigned_confirmation_needed && !req.confirm_unsigned {
        let why = if signature_policy == crate::core::plugin_policy::Policy::Warn {
            "this console's plugin signature policy warns on unsigned plugins"
        } else {
            "this plugin is unsigned and is being granted a Critical permission"
        };
        return Err(err(StatusCode::CONFLICT, &format!("{why}; confirm to approve it anyway")));
    }
    let granted_json = serde_json::to_string(&granted).unwrap_or_else(|_| "[]".to_string());
    let actor = actor_of(&headers, &app);
    app.db
        .set_plugin_decision(&id, "approved", &granted_json, row.observed_manifest_sha256.as_deref(), Some(&row.observed_digest), row.observed_signer.as_deref(), &actor)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    crate::core::plugin_registry::remember_approved_version(&app.db, &id, row.observed_signer.as_deref(), row.version.as_deref());
    app.db.add_plugin_audit(
        &id,
        &actor,
        "approved",
        &serde_json::json!({
            "granted": granted.iter().map(|p| &p.id).collect::<Vec<_>>(), "digest": row.observed_digest, "override": override_diff,
            "revoked_publisher_accepted": revoked_publisher, "unsigned_accepted": unsigned_confirmation_needed,
        }).to_string(),
    );
    // A publisher that proved it moved from a key you trusted (the old key signed the statement) keeps your trust.
    if let (Some(new_key), Some(old_key)) = (row.observed_signer.as_deref(), row.observed_rotation_from.as_deref()) {
        let old = app.db.get_publisher(old_key).ok().flatten();
        if old.as_ref().is_some_and(|p| p.trusted && !p.revoked) && !app.db.get_publisher(new_key).ok().flatten().is_some_and(|p| p.revoked) {
            let name = old.map(|p| p.name).unwrap_or_default();
            if app.db.trust_publisher(new_key, &name, &actor).is_ok() {
                app.db.add_plugin_audit(&id, &actor, "key_rotated", &serde_json::json!({ "from_key_id": signer_id(old_key), "to_key_id": signer_id(new_key) }).to_string());
            }
        }
    }
    if req.trust_publisher {
        if let Some(key) = row.observed_signer.as_deref() {
            let name = read_manifest(&row).map(|m| m.publisher).filter(|n| !n.is_empty()).unwrap_or_else(|| row.name.clone());
            if app.db.trust_publisher(key, &name, &actor).is_ok() {
                app.db.add_plugin_audit(&id, &actor, "publisher_trusted", &serde_json::json!({ "publisher_key_id": signer_id(key), "name": name }).to_string());
            }
        }
    }
    if override_diff.is_some() {
        tracing::warn!("plugin '{id}' was approved with an operator override of its requested permissions");
    }
    let fresh = app.db.get_plugin_grant(&id).ok().flatten().ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    Ok(Json(view(&app, &fresh)))
}

/// What an Advanced override changed relative to what the plugin asked for.
fn override_summary(requested: &[PermissionRequest], granted: &[PermissionRequest]) -> serde_json::Value {
    crate::core::plugin_permissions::permission_diff(requested, granted)
}

/// GET /api/plugins/permissions -- every permission the host knows, for the Advanced override.
pub(super) async fn permission_catalog(State(app): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    Ok(Json(crate::core::plugin_permissions::catalog_json()))
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
        .set_plugin_decision(&id, status, "[]", None, None, None, &actor)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    app.db.add_plugin_audit(&id, &actor, if status == "denied" { "denied" } else { "revoked" }, "{}");
    // Immediate: every token the plugin holds stops working.
    app.revoke_plugin_tokens(&id).await;
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

#[derive(Deserialize, Default)]
pub(super) struct TokenReq {
    /// What the token may do; defaults to the plugin's own keyring if that was granted.
    #[serde(default)]
    scopes: Option<Vec<PermissionRequest>>,
}

/// Permissions a token can carry at all.
const TOKEN_SCOPES: [&str; 7] = ["keyring.own", "keyring.service", "commands.send", "network.outbound", "library.write", "process.run", "notifications.post"];

/// POST /api/plugins/:id/token -- the console mints a token for an approved plugin, limited to
/// (a subset of) what it was granted. The token is checked against the live grant on every use.
pub(super) async fn mint_plugin_token(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    body: Option<Json<TokenReq>>,
) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    let row = app
        .db
        .get_plugin_grant(&id)
        .ok()
        .flatten()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "no such plugin"))?;
    if !plugin_usable(&app, &row) {
        return Err(err(StatusCode::FORBIDDEN, "this plugin isn't approved"));
    }
    let granted = parse_granted(&row);
    let wanted = match body.and_then(|Json(b)| b.scopes) {
        Some(s) => PermissionSet(s),
        None => PermissionSet(vec![PermissionRequest::simple("keyring.own")]).narrowed_by(&granted),
    };
    if wanted.0.is_empty() {
        return Err(err(StatusCode::FORBIDDEN, "nothing to issue a token for"));
    }
    if let Some(p) = wanted.0.iter().find(|p| !TOKEN_SCOPES.contains(&p.id.as_str())) {
        return Err(err(StatusCode::BAD_REQUEST, &format!("'{}' can't be carried by a token", p.id)));
    }
    // Every requested scope must be (at least partly) inside the grant; ask for more than was
    // granted and the call fails rather than silently issuing less.
    let effective = wanted.narrowed_by(&granted);
    if effective != wanted {
        return Err(err(StatusCode::FORBIDDEN, "the plugin wasn't granted everything the token asks for"));
    }
    let token = app.issue_plugin_token_scoped(&id, effective.clone()).await;
    app.db.add_plugin_audit(
        &id,
        &actor_of(&headers, &app),
        "token_issued",
        &serde_json::json!({ "scopes": effective.0.iter().map(|p| &p.id).collect::<Vec<_>>() }).to_string(),
    );
    Ok(Json(serde_json::json!({
        "success": true,
        "token": token,
        "scopes": effective.0,
        "service": format!("OpenSanctuary:Plugin:{id}"),
    })))
}

/// Plugin command denials are logged once per (plugin, command) per run, so a misbehaving
/// plugin can't flood the audit table.
static DENIED_SEEN: std::sync::Mutex<Option<std::collections::HashSet<(String, String)>>> = std::sync::Mutex::new(None);

/// Authorizes a command sent with a plugin token (`x-plugin-token` over REST, `plugin_token` in a
/// WebSocket envelope). Returns the plugin id; the caller then runs the command through the same
/// plugin-veto and execution path as any other. Plugin commands don't take the console lock.
pub(crate) async fn authorize_plugin_command(app: &AppState, token: &str, cmd: &serde_json::Value) -> Result<String, String> {
    let variant = crate::core::plugin_permissions::command_variant(&cmd.to_string()).unwrap_or_default();
    let Some(caller) = app.resolve_plugin_caller(token).await else {
        return Err("invalid, expired or revoked plugin token".to_string());
    };
    if !caller.scopes.allows_command(&variant) {
        let first = {
            let mut guard = DENIED_SEEN.lock().unwrap_or_else(|e| e.into_inner());
            let set = guard.get_or_insert_with(Default::default);
            set.len() < 1000 && set.insert((caller.plugin_id.clone(), variant.clone()))
        };
        if first {
            app.db.add_plugin_audit(&caller.plugin_id, &format!("plugin:{}", caller.plugin_id), "denied_call", &serde_json::json!({ "command": variant }).to_string());
        }
        return Err(format!("this plugin may not send '{variant}'"));
    }
    Ok(caller.plugin_id)
}

/// The plugin credential on a host-API call (`x-plugin-token`), resolved against the plugin's
/// live grant.
pub(super) async fn plugin_caller(app: &AppState, headers: &HeaderMap) -> Result<crate::api::ws::PluginCaller, ApiErr> {
    let token = headers.get("x-plugin-token").and_then(|v| v.to_str().ok()).unwrap_or("");
    app.resolve_plugin_caller(token)
        .await
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "invalid, expired or revoked plugin token"))
}

#[derive(Deserialize)]
pub(super) struct PluginFetchReq {
    url: String,
    /// `Authorization` / `Accept` / `Accept-Language` only (see `plugin_net::clean_headers`).
    #[serde(default)]
    headers: serde_json::Map<String, serde_json::Value>,
}

/// POST /api/plugin-host/fetch -- the host fetches a URL for a plugin granted `network.outbound`,
/// only from the hosts it was granted (see `plugin_net`). Returns the status, content type and
/// (size-capped) text body; a non-2xx status is the plugin's to interpret.
pub(super) async fn plugin_fetch(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PluginFetchReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let caller = plugin_caller(&app, &headers).await?;
    let hosts = caller.scopes.outbound_hosts();
    if hosts.is_empty() {
        return Err(err(StatusCode::FORBIDDEN, "this plugin wasn't granted network access"));
    }
    let extra = crate::api::plugin_net::clean_headers(&req.headers).map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    match crate::api::plugin_net::fetch(&hosts, &req.url, &extra).await {
        Ok(f) => Ok(Json(serde_json::json!({
            "success": true, "status": f.status, "content_type": f.content_type, "body": f.body, "url": f.final_url,
        }))),
        Err(e) => {
            app.db.add_plugin_audit(
                &caller.plugin_id,
                &format!("plugin:{}", caller.plugin_id),
                "denied_call",
                // Without the query string: an API key can ride in it (Pixabay's does).
                &serde_json::json!({ "fetch": req.url.split(['?', '#']).next().unwrap_or("").chars().take(200).collect::<String>(), "reason": e }).to_string(),
            );
            Err(err(StatusCode::FORBIDDEN, &e))
        }
    }
}

#[derive(Deserialize)]
pub(super) struct PluginSongReq {
    song: serde_json::Value,
}

/// POST /api/plugin-host/songs -- adds a song to the library for a plugin granted `library.write`
/// on `songs`. The id is always minted here, and size limits apply, so a plugin can add songs but
/// can't overwrite one or flood the library with an oversized payload.
pub(super) async fn plugin_add_song(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PluginSongReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let caller = plugin_caller(&app, &headers).await?;
    if !caller.scopes.writes("songs") {
        return Err(err(StatusCode::FORBIDDEN, "this plugin wasn't granted write access to songs"));
    }
    // The id is the host's to choose: whatever the plugin sent (or didn't) is replaced below.
    let mut raw = req.song;
    if let Some(obj) = raw.as_object_mut() {
        obj.insert("id".to_string(), serde_json::Value::String(String::new()));
    }
    let mut song: crate::core::models::Song =
        serde_json::from_value(raw).map_err(|e| err(StatusCode::BAD_REQUEST, &format!("not a song: {e}")))?;
    if song.title.trim().is_empty() || song.title.len() > 300 || song.author.len() > 300 {
        return Err(err(StatusCode::BAD_REQUEST, "a song needs a title (up to 300 characters)"));
    }
    if song.slides.is_empty() || song.slides.len() > 200 || song.slides.iter().any(|s| s.text.len() > 20_000) {
        return Err(err(StatusCode::BAD_REQUEST, "a song needs 1-200 slides of up to 20,000 characters"));
    }
    song.id = format!("song_{}", uuid::Uuid::new_v4());
    app.db.insert_song(&song).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("database error: {e}")))?;
    app.db.add_plugin_audit(
        &caller.plugin_id,
        &format!("plugin:{}", caller.plugin_id),
        "song_added",
        &serde_json::json!({ "id": song.id, "title": song.title }).to_string(),
    );
    Ok(Json(serde_json::json!({ "success": true, "id": song.id, "song": song })))
}

#[derive(Deserialize)]
pub(super) struct PluginScripturesReq {
    items: Vec<serde_json::Value>,
}

/// Most chapters, and verses per chapter, a plugin may add in one call (a whole Bible is sent in
/// batches); a verse's text is capped too.
const MAX_SCRIPTURE_ITEMS: usize = 500;
const MAX_VERSES_PER_ITEM: usize = 400;
/// Bytes, not characters. Real scripture needs more than the old 4000: the letters in the Greek Esther
/// additions run past 4600 bytes, and Burmese (3 bytes a character, plus invisible word-break marks) past
/// 5800; the longest verse in the OpenSanctuary Bibles repository is 10481. (The whole request is capped
/// at 32 MB separately.)
const MAX_VERSE_TEXT: usize = 16_000;

/// POST /api/plugin-host/scriptures -- adds Bible chapters to the library for a plugin granted
/// `library.write` on `scriptures`. Ids are derived here from (book, chapter, verses, version) --
/// the same way the built-in importer did -- so downloading a translation again replaces its chapters
/// instead of duplicating them. NOTE: that also means a plugin with this grant can replace chapters
/// of any installed translation whose version name it uses; the grant's reason says so.
pub(super) async fn plugin_add_scriptures(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PluginScripturesReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let caller = plugin_caller(&app, &headers).await?;
    if !caller.scopes.writes("scriptures") {
        return Err(err(StatusCode::FORBIDDEN, "this plugin wasn't granted write access to scriptures"));
    }
    if req.items.is_empty() || req.items.len() > MAX_SCRIPTURE_ITEMS {
        return Err(err(StatusCode::BAD_REQUEST, &format!("send 1-{MAX_SCRIPTURE_ITEMS} chapters per call")));
    }
    let mut items: Vec<crate::core::models::ScriptureItem> = Vec::with_capacity(req.items.len());
    for raw in req.items {
        let mut raw = raw;
        if let Some(obj) = raw.as_object_mut() {
            // Everything derived from the content is the host's to fill in.
            for (k, v) in [("id", ""), ("reference", "")] {
                obj.insert(k.to_string(), serde_json::Value::String(v.to_string()));
            }
            for k in ["verse_start", "verse_end"] {
                obj.entry(k.to_string()).or_insert(serde_json::Value::from(0));
            }
        }
        let mut item: crate::core::models::ScriptureItem =
            serde_json::from_value(raw).map_err(|e| err(StatusCode::BAD_REQUEST, &format!("not a chapter: {e}")))?;
        let version = item.version.trim().to_uppercase();
        let book = item.book.trim().to_string();
        if book.is_empty() || book.len() > 100 || version.is_empty() || version.len() > 40 {
            return Err(err(StatusCode::BAD_REQUEST, "a chapter needs a book (<=100 chars) and a version (<=40 chars)"));
        }
        if item.chapter == 0 || item.chapter > 1000 {
            return Err(err(StatusCode::BAD_REQUEST, "chapter number out of range"));
        }
        if item.verses.is_empty()
            || item.verses.len() > MAX_VERSES_PER_ITEM
            || item.verses.iter().any(|v| v.verse_number == 0 || v.verse_number > 1000 || v.text.len() > MAX_VERSE_TEXT)
        {
            return Err(err(StatusCode::BAD_REQUEST, &format!("a chapter needs 1-{MAX_VERSES_PER_ITEM} verses of up to {MAX_VERSE_TEXT} bytes each")));
        }
        item.verses.sort_by_key(|v| v.verse_number);
        item.verse_start = item.verses.first().map(|v| v.verse_number).unwrap_or(1);
        item.verse_end = item.verses.last().map(|v| v.verse_number).unwrap_or(item.verse_start);
        item.book = book;
        item.version = version;
        item.theme_name = None;
        item.reference = format!("{} {}", item.book, item.chapter);
        item.id = format!(
            "scrip_{}",
            uuid::Uuid::new_v5(
                &uuid::Uuid::NAMESPACE_OID,
                format!("{}:{}:{}-{}:{}", item.book, item.chapter, item.verse_start, item.verse_end, item.version).as_bytes()
            )
        );
        items.push(item);
    }
    let count = items.len();
    let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
    let verses: usize = items.iter().map(|i| i.verses.len()).sum();
    let versions: std::collections::BTreeSet<String> = items.iter().map(|i| i.version.clone()).collect();
    let db = app.db.clone();
    tokio::task::spawn_blocking(move || db.insert_scriptures_batch(&items))
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("task failed: {e}")))?
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("database error: {e}")))?;
    app.db.add_plugin_audit(
        &caller.plugin_id,
        &format!("plugin:{}", caller.plugin_id),
        "scriptures_added",
        &serde_json::json!({ "chapters": count, "verses": verses, "versions": versions }).to_string(),
    );
    Ok(Json(serde_json::json!({ "success": true, "chapters": count, "verses": verses, "ids": ids })))
}

// ---- tools, processes and media for plugins granted `process.run` -----------------------------------

use crate::core::plugin_permissions::{CommandSpec, PluginManifest, ToolSpec};

/// The plugin's manifest, but only while the plugin is approved *and* both its folder and its
/// manifest still match what the operator reviewed (the template that runs must be the template
/// that was read).
async fn active_manifest(app: &AppState, plugin_id: &str) -> Result<(PluginGrantRow, PluginManifest), ApiErr> {
    let row = app
        .db
        .get_plugin_grant(plugin_id)
        .ok()
        .flatten()
        .filter(|r| r.kind == "sandbox" && plugin_usable(app, r))
        .ok_or_else(|| err(StatusCode::FORBIDDEN, "this plugin isn't approved"))?;
    let dir = plugin_folder(&row);
    let (id, dir2) = (plugin_id.to_string(), dir.clone());
    let approved = row.approved_digest.clone().unwrap_or_default();
    let folder_ok = tokio::task::spawn_blocking(move || current_folder_digest(&id, &dir2).is_some_and(|d| d == approved))
        .await
        .unwrap_or(false);
    if !folder_ok {
        return Err(err(StatusCode::CONFLICT, "the plugin changed since it was approved"));
    }
    let bytes = std::fs::read(dir.join("plugin.json")).map_err(|_| err(StatusCode::CONFLICT, "the plugin's manifest is missing"))?;
    if Some(crate::core::plugin_permissions::sha256_hex(&bytes)) != row.approved_manifest_sha256 {
        return Err(err(StatusCode::CONFLICT, "the plugin's manifest changed since it was approved"));
    }
    let manifest = PluginManifest::parse(&bytes).map_err(|e| err(StatusCode::CONFLICT, &e.to_string()))?;
    Ok((row, manifest))
}

/// Where a sandboxed plugin's files are: the folder discovery found it in (built-in or installed).
fn plugin_folder(row: &PluginGrantRow) -> std::path::PathBuf {
    std::path::PathBuf::from(&row.source_path)
}

/// Is this folder one the operator installed (so it can be removed), as opposed to one that ships with the app?
fn is_installed_folder(app: &AppState, dir: &std::path::Path) -> bool {
    crate::core::plugin_registry::is_installed_folder(&app.data_dir, dir)
}

fn tools_dir(app: &AppState) -> std::path::PathBuf {
    app.data_dir.join("tools")
}

fn find_command<'a>(manifest: &'a PluginManifest, id: &str) -> Result<&'a CommandSpec, ApiErr> {
    manifest.commands.iter().find(|c| c.id == id).ok_or_else(|| err(StatusCode::NOT_FOUND, "no such command"))
}

fn find_tool<'a>(manifest: &'a PluginManifest, id: &str) -> Result<&'a ToolSpec, ApiErr> {
    manifest.tools.iter().find(|t| t.id == id).ok_or_else(|| err(StatusCode::NOT_FOUND, "no such tool"))
}

/// Needs any `process.run` grant at all (tools are the commands' programs).
async fn runner_caller(app: &AppState, headers: &HeaderMap) -> Result<crate::api::ws::PluginCaller, ApiErr> {
    let caller = plugin_caller(app, headers).await?;
    if !caller.scopes.0.iter().any(|p| p.id == "process.run") {
        return Err(err(StatusCode::FORBIDDEN, "this plugin wasn't granted permission to run commands"));
    }
    Ok(caller)
}

#[derive(Deserialize)]
pub(super) struct ProcessStartReq {
    command: String,
    #[serde(default)]
    params: serde_json::Map<String, serde_json::Value>,
}

/// POST /api/plugin-host/process/start
pub(super) async fn process_start(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProcessStartReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let caller = runner_caller(&app, &headers).await?;
    if !caller.scopes.runs(&req.command) {
        return Err(err(StatusCode::FORBIDDEN, &format!("this plugin wasn't granted '{}'", req.command)));
    }
    let (_row, manifest) = active_manifest(&app, &caller.plugin_id).await?;
    let spec = find_command(&manifest, &req.command)?.clone();
    let tool = find_tool(&manifest, &spec.tool)?;
    let program = crate::api::plugin_tools::resolve_command(&tools_dir(&app), &caller.plugin_id, tool)
        .ok_or_else(|| err(StatusCode::CONFLICT, "the tool isn't installed yet; update it first"))?;
    let ffmpeg = crate::storage::paths::resolve_ffmpeg_command();
    let ffmpeg_dir = ffmpeg.parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf());
    let params = req.params;
    let job = crate::api::plugin_process::start(&app.data_dir, &caller.plugin_id, &spec.id, program, |out| {
        crate::api::plugin_process::expand(&spec, &params, out, ffmpeg_dir.as_deref())
    })
    .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    app.db.add_plugin_audit(
        &caller.plugin_id,
        &format!("plugin:{}", caller.plugin_id),
        "process_started",
        &serde_json::json!({ "command": spec.id, "job": job.id }).to_string(),
    );
    Ok(Json(serde_json::json!({ "success": true, "job": job.id })))
}

#[derive(Deserialize)]
pub(super) struct ProcessJobReq {
    job: String,
    #[serde(default)]
    since: usize,
}

fn owned_job(caller: &crate::api::ws::PluginCaller, job: &str) -> Result<std::sync::Arc<crate::api::plugin_process::Job>, ApiErr> {
    crate::api::plugin_process::job_for(&caller.plugin_id, job).ok_or_else(|| err(StatusCode::NOT_FOUND, "no such job"))
}

/// POST /api/plugin-host/process/poll
pub(super) async fn process_poll(State(app): State<AppState>, headers: HeaderMap, Json(req): Json<ProcessJobReq>) -> Result<impl IntoResponse, ApiErr> {
    let caller = runner_caller(&app, &headers).await?;
    let job = owned_job(&caller, &req.job)?;
    let mut v = crate::api::plugin_process::poll(&job, req.since);
    v["success"] = serde_json::Value::Bool(true);
    Ok(Json(v))
}

/// POST /api/plugin-host/process/cancel
pub(super) async fn process_cancel(State(app): State<AppState>, headers: HeaderMap, Json(req): Json<ProcessJobReq>) -> Result<impl IntoResponse, ApiErr> {
    let caller = runner_caller(&app, &headers).await?;
    crate::api::plugin_process::cancel(&*owned_job(&caller, &req.job)?);
    Ok(Json(serde_json::json!({ "success": true })))
}

/// POST /api/plugin-host/process/release -- forget a job and delete its folder.
pub(super) async fn process_release(State(app): State<AppState>, headers: HeaderMap, Json(req): Json<ProcessJobReq>) -> Result<impl IntoResponse, ApiErr> {
    let caller = runner_caller(&app, &headers).await?;
    Ok(Json(serde_json::json!({ "success": crate::api::plugin_process::release(&caller.plugin_id, &req.job) })))
}

#[derive(Deserialize)]
pub(super) struct ToolReq {
    tool: String,
}

type UpdateKey = (String, String);
fn tool_updates() -> &'static std::sync::Mutex<std::collections::HashMap<UpdateKey, (bool, Option<crate::api::plugin_tools::UpdateOutcome>)>> {
    static M: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<UpdateKey, (bool, Option<crate::api::plugin_tools::UpdateOutcome>)>>> =
        std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

async fn tool_report(app: &AppState, plugin_id: &str, tool: &ToolSpec) -> serde_json::Value {
    let st = crate::api::plugin_tools::status(&tools_dir(app), plugin_id, tool).await;
    let (updating, last) = tool_updates()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(plugin_id.to_string(), tool.id.clone()))
        .cloned()
        .unwrap_or((false, None));
    serde_json::json!({ "success": true, "installed": st.installed, "source": st.source, "version": st.version, "updating": updating, "last_update": last })
}

/// POST /api/plugin-host/tool/status
pub(super) async fn tool_status(State(app): State<AppState>, headers: HeaderMap, Json(req): Json<ToolReq>) -> Result<impl IntoResponse, ApiErr> {
    let caller = runner_caller(&app, &headers).await?;
    let (_row, manifest) = active_manifest(&app, &caller.plugin_id).await?;
    let tool = find_tool(&manifest, &req.tool)?;
    Ok(Json(tool_report(&app, &caller.plugin_id, tool).await))
}

/// POST /api/plugin-host/tool/update -- installs or updates the tool in the background (a release is
/// tens of megabytes); poll `tool/status` for `updating` and `last_update`.
pub(super) async fn tool_update(State(app): State<AppState>, headers: HeaderMap, Json(req): Json<ToolReq>) -> Result<impl IntoResponse, ApiErr> {
    let caller = runner_caller(&app, &headers).await?;
    let (row, manifest) = active_manifest(&app, &caller.plugin_id).await?;
    let tool = find_tool(&manifest, &req.tool)?.clone();
    let key_path = std::path::Path::new(&row.source_path).join(&tool.signature.key_file);
    let key = std::fs::read_to_string(&key_path).map_err(|_| err(StatusCode::CONFLICT, "the tool's signing key is missing from the plugin"))?;
    let key_id = (caller.plugin_id.clone(), tool.id.clone());
    let already_running = {
        let mut m = tool_updates().lock().unwrap_or_else(|e| e.into_inner());
        let entry = m.entry(key_id.clone()).or_insert((false, None));
        let was = entry.0;
        entry.0 = true;
        was
    };
    if already_running {
        return Ok(Json(tool_report(&app, &caller.plugin_id, &tool).await));
    }
    let (tools, plugin_id, db) = (tools_dir(&app), caller.plugin_id.clone(), app.db.clone());
    let (bus, plugin_name) = (app.notifications.clone(), row.name.clone());
    let tool2 = tool.clone();
    tokio::spawn(async move {
        let outcome = crate::api::plugin_tools::check_and_update(&tools, &plugin_id, &tool2, &key).await;
        if outcome.updated || outcome.error {
            db.add_plugin_audit(
                &plugin_id,
                &format!("plugin:{plugin_id}"),
                if outcome.error { "tool_update_failed" } else { "tool_updated" },
                &serde_json::json!({ "tool": tool2.id, "version": outcome.version, "message": outcome.message }).to_string(),
            );
            // The operator may have left the plugin's panel while a release downloaded.
            let (level, title) = if outcome.error {
                (crate::api::notifications::Level::Error, format!("{plugin_name}: couldn't update {}", tool2.id))
            } else {
                (crate::api::notifications::Level::Success, format!("{plugin_name}: {} updated", tool2.id))
            };
            bus.post_host(level, &title, &outcome.message);
        }
        tool_updates().lock().unwrap_or_else(|e| e.into_inner()).insert(key_id, (false, Some(outcome)));
    });
    Ok(Json(tool_report(&app, &caller.plugin_id, &tool).await))
}

#[derive(Deserialize)]
pub(super) struct AddMediaUrlReq {
    url: String,
    title: String,
    #[serde(default)]
    id: Option<String>,
}

const MAX_MEDIA_URL_BYTES: usize = 64 * 1024 * 1024;

/// POST /api/plugin-host/media-url -- downloads a picture or video from a host the plugin was
/// granted (`network.outbound`) into the Media library (`library.write` on `media`). The file type
/// comes from the downloaded bytes, never from the URL or the server's say-so, and the name on disk
/// is the host's, so a plugin can't plant a page or script under `/media`.
pub(super) async fn plugin_add_media_url(State(app): State<AppState>, headers: HeaderMap, Json(req): Json<AddMediaUrlReq>) -> Result<impl IntoResponse, ApiErr> {
    let caller = plugin_caller(&app, &headers).await?;
    if !caller.scopes.writes("media") {
        return Err(err(StatusCode::FORBIDDEN, "this plugin wasn't granted write access to media"));
    }
    let hosts = caller.scopes.outbound_hosts();
    if hosts.is_empty() {
        return Err(err(StatusCode::FORBIDDEN, "this plugin wasn't granted network access"));
    }
    let title: String = req.title.trim().chars().filter(|c| !c.is_control()).take(300).collect();
    if title.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "a title is required"));
    }
    let f = crate::api::plugin_net::fetch_bytes(&hosts, &req.url, &[], MAX_MEDIA_URL_BYTES)
        .await
        .map_err(|e| err(StatusCode::FORBIDDEN, &e))?;
    if !(200..300).contains(&f.status) {
        return Err(err(StatusCode::BAD_GATEWAY, &format!("the download failed ({})", f.status)));
    }
    let Some(ext) = crate::storage::media_sniff::sniff_media_ext(&f.body) else {
        return Err(err(StatusCode::BAD_REQUEST, "the download isn't a supported image or video"));
    };
    let is_video = VIDEO_EXTS.contains(&ext);
    let (sub, media_type) = if is_video { ("videos", crate::core::models::MediaType::Video) } else { ("images", crate::core::models::MediaType::Image) };
    let dir = app.media_dir.join(sub);
    let prefix: String = caller.plugin_id.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
    let file_name = crate::storage::media_sniff::write_sniffed_media(&dir, &prefix, &f.body)
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "couldn't save the file"))?;
    let clean_id: String = req.id.as_deref().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(64).collect();
    let mut item = crate::core::models::MediaItem::new(&title, &format!("/media/{sub}/{file_name}"), media_type);
    item.id = if clean_id.is_empty() { format!("plugin_{}_{}", caller.plugin_id, uuid::Uuid::new_v4().simple()) } else { format!("plugin_{}_{}_{}", caller.plugin_id, clean_id, uuid::Uuid::new_v4().simple()) };
    if let Err(e) = app.db.insert_media(&item) {
        let _ = std::fs::remove_file(dir.join(&file_name)); // no orphaned file
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, &format!("database error: {e}")));
    }
    app.db.add_plugin_audit(
        &caller.plugin_id,
        &format!("plugin:{}", caller.plugin_id),
        "media_added",
        &serde_json::json!({ "id": item.id, "title": title, "file": file_name, "from": f.final_url.chars().take(200).collect::<String>() }).to_string(),
    );
    Ok(Json(serde_json::json!({ "success": true, "id": item.id, "item": item })))
}

#[derive(Deserialize)]
pub(super) struct AddMediaReq {
    job: String,
    /// A plain file name in the job's folder.
    file: String,
    /// `video` or `audio`.
    media_type: String,
    title: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    thumbnail: Option<String>,
}

const VIDEO_EXTS: [&str; 6] = ["mp4", "webm", "mkv", "mov", "m4v", "avi"];
const AUDIO_EXTS: [&str; 7] = ["mp3", "m4a", "opus", "ogg", "wav", "flac", "aac"];

/// POST /api/plugin-host/media -- moves a finished file out of one of the plugin's own job folders
/// into the media library and registers it. Needs `library.write` on `media`.
pub(super) async fn plugin_add_media(State(app): State<AppState>, headers: HeaderMap, Json(req): Json<AddMediaReq>) -> Result<impl IntoResponse, ApiErr> {
    let caller = plugin_caller(&app, &headers).await?;
    if !caller.scopes.writes("media") {
        return Err(err(StatusCode::FORBIDDEN, "this plugin wasn't granted write access to media"));
    }
    let job = owned_job(&caller, &req.job)?;
    if job.state.lock().unwrap_or_else(|e| e.into_inner()).status == "running" {
        return Err(err(StatusCode::CONFLICT, "the job is still running"));
    }
    let (sub, exts, media_type) = match req.media_type.as_str() {
        "video" => ("videos", &VIDEO_EXTS[..], crate::core::models::MediaType::Video),
        "audio" => ("audio", &AUDIO_EXTS[..], crate::core::models::MediaType::Audio),
        _ => return Err(err(StatusCode::BAD_REQUEST, "media_type must be 'video' or 'audio'")),
    };
    let name = req.file.as_str();
    if name.is_empty() || name.len() > 255 || name.contains(['/', '\\']) || name.starts_with('.') || name.chars().any(|c| c.is_control()) {
        return Err(err(StatusCode::BAD_REQUEST, "file must be a plain file name from the job's folder"));
    }
    let ext = std::path::Path::new(name).extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    if !exts.contains(&ext.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, &format!("a {} file must be one of: {}", req.media_type, exts.join(", "))));
    }
    let src = job.output_dir.join(name);
    match std::fs::symlink_metadata(&src) {
        Ok(m) if m.is_file() => {}
        _ => return Err(err(StatusCode::NOT_FOUND, "no such file in the job's folder")),
    }
    // The name's extension says what this is *supposed* to be; it is never what makes it so. Read enough
    // of the real bytes to check: a command's own tool decides what it writes, and the manifest promises
    // "video" or "audio", not "whatever that tool felt like producing".
    {
        let mut head = vec![0u8; 4096];
        let read = {
            use std::io::Read;
            std::fs::File::open(&src).and_then(|mut f| f.read(&mut head)).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("couldn't read the file: {e}")))?
        };
        head.truncate(read);
        let sniffed = match media_type {
            crate::core::models::MediaType::Video => crate::storage::media_sniff::sniff_media_ext(&head).filter(|e| VIDEO_EXTS.contains(e)),
            crate::core::models::MediaType::Audio => crate::storage::media_sniff::sniff_audio_ext(&head),
            _ => None,
        };
        if sniffed.is_none() {
            return Err(err(StatusCode::BAD_REQUEST, &format!("'{name}' doesn't look like a {} file (its contents don't match a known {0} format, whatever its name claims)", req.media_type)));
        }
    }
    let title: String = req.title.trim().chars().filter(|c| !c.is_control()).take(300).collect();
    if title.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "a title is required"));
    }

    let dest_dir = app.media_dir.join(sub);
    std::fs::create_dir_all(&dest_dir).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("can't create the media folder: {e}")))?;
    let stem = std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or("media").to_string();
    let mut dest = dest_dir.join(name);
    let mut n = 2;
    while dest.exists() {
        dest = dest_dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    if std::fs::rename(&src, &dest).is_err() {
        // Different filesystems: copy, then remove the original.
        std::fs::copy(&src, &dest).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("couldn't move the file: {e}")))?;
        let _ = std::fs::remove_file(&src);
    }
    let file_name = dest.file_name().and_then(|f| f.to_str()).unwrap_or(name).to_string();

    let clean_id: String = req.id.as_deref().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(64).collect();
    let id = if clean_id.is_empty() { format!("plugin_{}_{}", caller.plugin_id, uuid::Uuid::new_v4().simple()) } else { format!("plugin_{}_{}", caller.plugin_id, clean_id) };
    let mut item = crate::core::models::MediaItem::new(&title, &format!("/media/{sub}/{file_name}"), media_type);
    item.id = id;
    item.duration_seconds = req.duration.filter(|d| d.is_finite() && *d > 0.0 && *d < 1.0e6);
    item.thumbnail_path = req.thumbnail.filter(|t| t.len() <= 2048 && (t.starts_with("https://") || t.starts_with("http://")));
    app.db.insert_media(&item).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("database error: {e}")))?;
    app.db.add_plugin_audit(
        &caller.plugin_id,
        &format!("plugin:{}", caller.plugin_id),
        "media_added",
        &serde_json::json!({ "id": item.id, "title": title, "file": file_name }).to_string(),
    );

    // H.264 -> VP9 for native playback, in the background (the same convenience the media folder
    // scan gives files dropped in by hand); the item's path is updated when it finishes.
    if req.media_type == "video" {
        let (db, disk_path, mut for_update) = (app.db.clone(), dest.clone(), item.clone());
        tokio::spawn(async move {
            if crate::storage::transcode::needs_transcode(&disk_path).await {
                match crate::storage::transcode::transcode_h264_to_vp9(&disk_path, |_| {}).await {
                    Ok(new_path) => {
                        if let Some(f) = new_path.file_name().and_then(|f| f.to_str()) {
                            for_update.file_path = format!("/media/videos/{f}");
                            let _ = db.insert_media(&for_update);
                        }
                    }
                    Err(e) => tracing::warn!("transcode skipped for {:?}: {}", disk_path, e),
                }
            }
        });
    }
    Ok(Json(serde_json::json!({ "success": true, "id": item.id, "item": item })))
}

/// True when `file_name` (e.g. `hello_world.js`) is an approved, unchanged web plugin.
/// The file is re-hashed on every request, so editing it after approval stops it being
/// served until it is reviewed again.
pub(crate) fn web_plugin_file_is_active(app: &AppState, file_name: &str) -> bool {
    let Some(stem) = file_name.strip_suffix(".js") else { return false };
    let Some(row) = app.db.get_plugin_grant(stem).ok().flatten() else { return false };
    if row.kind != "web" || !plugin_usable(app, &row) {
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

/// Digest of a sandboxed plugin folder, recomputed only when a file's size or mtime moved, so
/// serving doesn't re-hash a folder on every request but an edit is still caught at once.
static FRAME_DIGESTS: std::sync::Mutex<Option<std::collections::HashMap<String, (u64, String)>>> =
    std::sync::Mutex::new(None);

fn folder_signature(dir: &std::path::Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    fn walk(dir: &std::path::Path, h: &mut std::collections::hash_map::DefaultHasher) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).map(|r| r.flatten().collect()).unwrap_or_default();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            if let Ok(m) = std::fs::symlink_metadata(&path) {
                path.hash(h);
                m.len().hash(h);
                m.modified().ok().hash(h);
                if m.is_dir() {
                    walk(&path, h);
                }
            }
        }
    }
    walk(dir, &mut h);
    h.finish()
}

pub(super) fn current_folder_digest(id: &str, dir: &std::path::Path) -> Option<String> {
    let sig = folder_signature(dir);
    {
        let guard = FRAME_DIGESTS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((s, d)) = guard.as_ref().and_then(|m| m.get(id)) {
            if *s == sig {
                return Some(d.clone());
            }
        }
    }
    let files = crate::core::plugin_registry::sandbox_dir_files(dir).ok()?;
    let digest = crate::core::plugin_permissions::artifact_digest(&files);
    FRAME_DIGESTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(id.to_string(), (sig, digest.clone()));
    Some(digest)
}

fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Only plain host[:port] (optionally with a `*.` wildcard) may reach a CSP source list.
fn csp_host_ok(h: &str) -> bool {
    crate::core::plugin_permissions::valid_host_entry(h)
}

/// The CSP every frame response carries: nothing but the hosts the plugin was granted, and a sandbox.
fn frame_csp(row: &PluginGrantRow) -> String {
    let granted = parse_granted(row);
    let mut sources: Vec<String> = Vec::new();
    for p in granted.0.iter().filter(|p| p.id == "network.outbound") {
        for h in p.hosts.iter().filter(|h| csp_host_ok(h)) {
            sources.push(format!("https://{h} wss://{h} http://{h} ws://{h}"));
        }
    }
    let connect = if sources.is_empty() { "'none'".to_string() } else { sources.join(" ") };
    // Pictures may load from the same hosts the plugin may already talk to (a stock-photo plugin's
    // thumbnails); that adds no reach beyond connect-src.
    let img = granted
        .0
        .iter()
        .filter(|p| p.id == "network.outbound")
        .flat_map(|p| p.hosts.iter().filter(|h| csp_host_ok(h)).map(|h| format!("https://{h}")))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: {img}; \
         font-src 'self' data:; media-src 'self' blob:; connect-src {connect}; base-uri 'none'; form-action 'none'; \
         frame-ancestors 'self'; sandbox allow-scripts"
    )
}

/// `icons/<set>/<name>.svg` with a plain set and icon name (letters, digits, dashes) -> (set, name).
fn icon_path(shared: &str) -> Option<(&str, &str)> {
    let rest = shared.strip_prefix("icons/")?;
    let (set, file) = rest.split_once('/')?;
    let name = file.strip_suffix(".svg")?;
    let plain = |s: &str| !s.is_empty() && s.len() <= 40 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    (plain(set) && plain(name)).then_some((set, name))
}

/// A script defining `window.__OS_EMOJI` (glyph -> icon name, for the glyphs that have an icon).
fn icons_script(map_path: &std::path::Path) -> String {
    let entries: Vec<serde_json::Value> = std::fs::read(map_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut map = serde_json::Map::new();
    for e in entries {
        if let (Some(g), Some(i)) = (e.get("glyph").and_then(|v| v.as_str()), e.get("icon").and_then(|v| v.as_str())) {
            map.insert(g.replace('\u{fe0f}', ""), serde_json::Value::String(i.to_string()));
        }
    }
    format!("window.__OS_EMOJI = {};\n", serde_json::Value::Object(map))
}

/// GET /plugin-frame/:id/*path -- serves a sandboxed plugin's files, but only while the plugin is
/// approved and the folder still hashes to what was reviewed. Every response carries a CSP that
/// forbids everything except what the plugin was granted (`network.outbound` hosts), and a
/// `sandbox` directive so it runs with an opaque origin even if opened outside the console's iframe.
pub(super) async fn serve_plugin_frame(
    State(app): State<AppState>,
    AxumPath((id, rel)): AxumPath<(String, String)>,
) -> axum::response::Response {
    use axum::http::header;
    let not_found = || (StatusCode::NOT_FOUND, "not found").into_response();
    if !crate::core::plugin_permissions::valid_plugin_id(&id)
        || !crate::core::plugin_permissions::relative_path_is_contained(&rel)
        || rel.contains('\\')
    {
        return not_found();
    }
    let Some(row) = app.db.get_plugin_grant(&id).ok().flatten() else { return not_found() };
    if row.kind != "sandbox" || !plugin_usable(&app, &row) {
        return not_found();
    }
    let dir = plugin_folder(&row);
    // The console's shared plugin SDK and theme stylesheet are part of the app, not of the plugin:
    // served under `__os/` to any plugin that passes the checks below.
    let shared = rel.strip_prefix("__os/").map(|n| n.to_string());
    let (app2, id2, dir2) = (app.clone(), id.clone(), dir.clone());
    let approved = row.approved_digest.clone().unwrap_or_default();
    let ok = tokio::task::spawn_blocking(move || {
        let _ = &app2;
        current_folder_digest(&id2, &dir2).is_some_and(|d| d == approved)
    })
    .await
    .unwrap_or(false);
    if !ok {
        return not_found();
    }
    // `__os/icons.js`: the console's emoji -> icon map, as a script (a frame can't fetch JSON).
    if shared.as_deref() == Some("icons.js") {
        let body = icons_script(&app.web_dir.join("icons").join("emoji-map.json"));
        return axum::response::Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/javascript; charset=utf-8")
            .header(header::CONTENT_SECURITY_POLICY, frame_csp(&row))
            .header(header::CACHE_CONTROL, "no-store")
            .body(axum::body::Body::from(body))
            .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "couldn't build the response").into_response());
    }
    let file = match &shared {
        Some(name) if matches!(name.as_str(), "sdk.js" | "theme.css") => app.web_dir.join("plugins").join("_shared").join(name),
        // `__os/icons/<set>/<name>.svg`: the console's own icons (one folder per theme), plain names only.
        Some(name) => match icon_path(name) {
            Some((set, icon)) => app.web_dir.join("icons").join(set).join(format!("{icon}.svg")),
            None => return not_found(),
        },
        None => dir.join(&rel),
    };
    match std::fs::symlink_metadata(&file) {
        Ok(m) if m.is_file() => {}
        _ => return not_found(),
    }
    let Ok(bytes) = std::fs::read(&file) else { return not_found() };

    let csp = frame_csp(&row);
    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type_for(&rel))
        .header(header::CONTENT_SECURITY_POLICY, csp)
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(bytes))
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "couldn't build the response").into_response())
}


/// Largest plugin package accepted (the zip itself), and what it may unpack to.
const MAX_PACKAGE_BYTES: usize = 25 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 25 * 1024 * 1024;
const MAX_PACKAGE_FILES: usize = 300;

#[derive(Deserialize, Default)]
pub(super) struct InstallQuery {
    /// Replace an already-installed plugin of the same id (an update). Its approval is dropped back to review.
    replace: Option<bool>,
}

type InstallErr = (StatusCode, String);

fn bad(msg: impl Into<String>) -> InstallErr {
    (StatusCode::BAD_REQUEST, msg.into())
}

/// Unpacks `bytes` into a fresh staging folder and returns `(staging root, plugin id, folder to move)`.
/// Nothing in here trusts the archive: entry names are checked, links are refused, sizes are counted as
/// they are written, and the result is validated exactly as discovery would validate it.
fn unpack_package(app: &AppState, bytes: &[u8], staging: &std::path::Path) -> Result<(String, std::path::PathBuf), InstallErr> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| bad("that isn't a zip file"))?;
    if zip.len() > MAX_PACKAGE_FILES * 2 {
        return Err(bad("the package holds too many files"));
    }
    // Which entries to take, with their paths inside the plugin folder.
    let mut files: Vec<(usize, std::path::PathBuf)> = Vec::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|_| bad("the zip file is damaged"))?;
        let Some(name) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            return Err(bad(format!("unsafe path in the package: {}", entry.name())));
        };
        if name.components().any(|c| matches!(c, std::path::Component::Normal(n) if n == "__MACOSX")) || name.file_name().is_some_and(|n| n == ".DS_Store") {
            continue;
        }
        if entry.name().contains('\\') {
            return Err(bad(format!("unsafe path in the package: {}", entry.name())));
        }
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(bad(format!("the package contains a symbolic link ({}); plugins can't", entry.name())));
        }
        if entry.is_dir() {
            continue;
        }
        files.push((i, name));
    }
    if files.is_empty() {
        return Err(bad("the package is empty"));
    }
    if files.len() > MAX_PACKAGE_FILES {
        return Err(bad(format!("the package holds more than {MAX_PACKAGE_FILES} files")));
    }
    // A package is either the plugin's files at its top level, or one folder holding them.
    let at_root = files.iter().any(|(_, p)| p == std::path::Path::new("plugin.json"));
    let strip = if at_root {
        None
    } else {
        let first = |p: &std::path::Path| p.components().next().map(|c| c.as_os_str().to_os_string());
        let top = first(&files[0].1);
        if top.is_some() && files.iter().all(|(_, p)| first(p) == top && p.components().count() > 1) { top } else { None }
    };
    let root = staging.join("unpacked");
    std::fs::create_dir_all(&root).map_err(|_| bad("couldn't create a working folder"))?;
    let mut total: u64 = 0;
    for (i, name) in &files {
        let rel = match &strip {
            Some(top) => name.strip_prefix(top).unwrap_or(name).to_path_buf(),
            None => name.clone(),
        };
        let dest = root.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|_| bad("couldn't unpack the package"))?;
        }
        let entry = zip.by_index(*i).map_err(|_| bad("the zip file is damaged"))?;
        let mut out = std::fs::File::create(&dest).map_err(|_| bad("couldn't unpack the package"))?;
        // Count what is actually written, not what the archive claims: a zip bomb lies about its sizes.
        let copied = std::io::copy(&mut entry.take(MAX_UNPACKED_BYTES + 1 - total), &mut out).map_err(|_| bad("the zip file is damaged"))?;
        total += copied;
        if total > MAX_UNPACKED_BYTES {
            return Err(bad(format!("the package unpacks to more than {} MB", MAX_UNPACKED_BYTES / 1024 / 1024)));
        }
    }
    let manifest_bytes = std::fs::read(root.join("plugin.json")).map_err(|_| bad("there is no plugin.json at the top of the package"))?;
    if manifest_bytes.len() > 256 * 1024 {
        return Err(bad("plugin.json is too large"));
    }
    let manifest = PluginManifest::parse(&manifest_bytes).map_err(|e| bad(e.to_string()))?;
    if !crate::core::plugin_permissions::valid_plugin_id(&manifest.id) {
        return Err(bad(format!("invalid plugin id '{}'", manifest.id)));
    }
    // Discovery wants the folder to be named after the plugin.
    let named = staging.join("named").join(&manifest.id);
    std::fs::create_dir_all(named.parent().unwrap_or(staging)).map_err(|_| bad("couldn't create a working folder"))?;
    std::fs::rename(&root, &named).map_err(|_| bad("couldn't prepare the package"))?;
    let icons = app.web_dir.join("icons").join("crimson");
    check_plugin_folder(&named, env!("CARGO_PKG_VERSION"), Some(&icons)).map_err(bad)?;
    Ok((manifest.id, named))
}

fn install_package(app: &AppState, bytes: &[u8], replace: bool) -> Result<String, InstallErr> {
    let user_dir = user_plugins_dir(&app.data_dir);
    let staging = app.data_dir.join("plugins-staging").join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&staging).map_err(|_| bad("couldn't create a working folder"))?;
    let result = (|| {
        let (id, folder) = unpack_package(app, bytes, &staging)?;
        let bundled = app.web_dir.join("plugins");
        if bundled.join(&id).exists() || bundled.join(format!("{id}.js")).exists() || bundled.join(format!("{id}.plugin.json")).exists() {
            return Err((StatusCode::CONFLICT, format!("'{id}' is the id of a plugin that ships with the app; it can't be replaced")));
        }
        std::fs::create_dir_all(&user_dir).map_err(|_| bad("couldn't create the plugins folder"))?;
        let dest = user_dir.join(&id);
        let backup = staging.join("previous");
        let had_old = dest.exists();
        if had_old {
            if !replace {
                return Err((StatusCode::CONFLICT, format!("'{id}' is already installed; install it as an update to replace it")));
            }
            std::fs::rename(&dest, &backup).map_err(|_| bad("couldn't replace the installed copy"))?;
        }
        if std::fs::rename(&folder, &dest).is_err() {
            if had_old {
                let _ = std::fs::rename(&backup, &dest);
            }
            return Err(bad("couldn't install the plugin"));
        }
        Ok(id)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// POST /api/plugins/install -- the body is a plugin package (a zip with `plugin.json`). Installing
/// grants nothing: the plugin appears as "needs review" like any other.
pub(super) async fn install_plugin(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<InstallQuery>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    if body.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "choose a plugin package (.zip)"));
    }
    if body.len() > MAX_PACKAGE_BYTES {
        return Err(err(StatusCode::PAYLOAD_TOO_LARGE, "that package is too large"));
    }
    let replace = q.replace.unwrap_or(false);
    let app2 = app.clone();
    let id = tokio::task::spawn_blocking(move || install_package(&app2, &body, replace))
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "install failed"))?
        .map_err(|(status, msg)| err(status, &msg))?;
    let app3 = app.clone();
    tokio::task::spawn_blocking(move || rescan(&app3)).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "scan failed"))?;
    app.db.add_plugin_audit(&id, &actor_of(&headers, &app), if replace { "updated" } else { "installed" }, "{}");
    app.revoke_plugin_tokens(&id).await;
    let fresh = app.db.get_plugin_grant(&id).ok().flatten().ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    Ok(Json(view(&app, &fresh)))
}

/// DELETE /api/plugins/:id -- removes a plugin the operator installed: its files, its approval, its
/// downloaded tools and the secrets it saved. Plugins that ship with the app can only be revoked.
pub(super) async fn uninstall_plugin(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    let row = app.db.get_plugin_grant(&id).ok().flatten().ok_or_else(|| err(StatusCode::NOT_FOUND, "no such plugin"))?;
    let dir = plugin_folder(&row);
    if row.kind != "sandbox" || !is_installed_folder(&app, &dir) {
        return Err(err(StatusCode::FORBIDDEN, "plugins that ship with the app can't be removed; revoke them instead"));
    }
    // Stop it first: every token it holds dies before its files do.
    app.revoke_plugin_tokens(&id).await;
    let (dir2, tools) = (dir.clone(), tools_dir(&app).join("plugins").join(&id));
    tokio::task::spawn_blocking(move || {
        let _ = std::fs::remove_dir_all(&dir2);
        let _ = std::fs::remove_dir_all(&tools);
    })
    .await
    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "couldn't remove the plugin"))?;
    if dir.exists() {
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "couldn't remove the plugin's files"));
    }
    // Its saved secrets go with it, so a different plugin installed later under the same id can't read them.
    let owners = [format!("plugin:{id}"), crate::api::keyring_policy::plugin_owner_tag(&id)];
    let mut secrets = 0;
    for (service, account) in app.db.take_keyring_entries_owned_by(&owners).unwrap_or_default() {
        if crate::storage::KeyringService::delete_secret(&service, &account).unwrap_or(false) {
            secrets += 1;
        }
    }
    let _ = app.db.delete_plugin_grant(&id);
    app.db.add_plugin_audit(&id, &actor_of(&headers, &app), "uninstalled", &serde_json::json!({ "secrets_deleted": secrets }).to_string());
    Ok(Json(serde_json::json!({ "success": true, "secrets_deleted": secrets })))
}

/// Finds the full key for a publisher's short id among the keys on record (a plugin's, or a saved publisher).
fn key_for_id(app: &AppState, key_id: &str) -> Option<String> {
    let mut keys: Vec<String> = app.db.list_plugin_grants().unwrap_or_default().into_iter().flat_map(|r| [r.observed_signer, r.approved_signer]).flatten().collect();
    keys.extend(app.db.list_publishers().unwrap_or_default().into_iter().map(|p| p.key));
    keys.into_iter().find(|k| signer_id(k) == key_id)
}

fn publisher_name_for(app: &AppState, key: &str) -> String {
    for r in app.db.list_plugin_grants().unwrap_or_default() {
        if r.observed_signer.as_deref() == Some(key) || r.approved_signer.as_deref() == Some(key) {
            if let Some(m) = read_manifest(&r) {
                if !m.publisher.is_empty() {
                    return m.publisher;
                }
            }
        }
    }
    app.db.get_publisher(key).ok().flatten().map(|p| p.name).unwrap_or_default()
}

/// GET /api/plugins/publishers -- the keys the operator trusted or revoked, and which plugins each signs.
pub(super) async fn list_publishers(State(app): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    let rows = app.db.list_publishers().map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    let out: Vec<_> = rows
        .iter()
        .map(|p| serde_json::json!({
            "key_id": signer_id(&p.key), "name": if p.name.is_empty() { publisher_name_for(&app, &p.key) } else { p.name.clone() }, "trusted": p.trusted, "revoked": p.revoked,
            "added_at": p.added_at, "added_by": p.added_by, "note": p.note,
            "plugins": app.db.plugins_signed_by(&p.key).unwrap_or_default(),
        }))
        .collect();
    Ok(Json(out))
}

/// GET /api/plugins/revocations -- what the signed revocation list has done, and any list file that couldn't be used.
pub(super) async fn revocation_status(State(app): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    let paths = crate::core::plugin_revocation::list_paths(&app.web_dir, &app.data_dir);
    let (newest, problems) = crate::core::plugin_revocation::newest_valid(&paths, &app.plugin_release_key);
    Ok(Json(serde_json::json!({
        "applied_sequence": crate::core::plugin_revocation::applied_sequence(&app.db),
        "newest_valid_sequence": newest.as_ref().map(|l| l.sequence),
        "listed_keys": newest.map(|l| l.revoked.len()).unwrap_or(0),
        "problems": problems,
    })))
}

/// POST /api/plugins/publishers/:key_id/trust
pub(super) async fn trust_publisher(State(app): State<AppState>, headers: HeaderMap, AxumPath(key_id): AxumPath<String>) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    let key = key_for_id(&app, &key_id).ok_or_else(|| err(StatusCode::NOT_FOUND, "no plugin is signed by that publisher"))?;
    if app.db.get_publisher(&key).ok().flatten().is_some_and(|p| p.revoked) {
        return Err(err(StatusCode::CONFLICT, "that publisher is revoked"));
    }
    let actor = actor_of(&headers, &app);
    let name = publisher_name_for(&app, &key);
    app.db.trust_publisher(&key, &name, &actor).map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    app.db.add_plugin_audit("*", &actor, "publisher_trusted", &serde_json::json!({ "publisher_key_id": key_id, "name": name }).to_string());
    Ok(Json(serde_json::json!({ "success": true })))
}

/// POST /api/plugins/publishers/:key_id/untrust
pub(super) async fn untrust_publisher(State(app): State<AppState>, headers: HeaderMap, AxumPath(key_id): AxumPath<String>) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    let key = key_for_id(&app, &key_id).ok_or_else(|| err(StatusCode::NOT_FOUND, "no such publisher"))?;
    let actor = actor_of(&headers, &app);
    app.db.untrust_publisher(&key).map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    app.db.add_plugin_audit("*", &actor, "publisher_untrusted", &serde_json::json!({ "publisher_key_id": key_id }).to_string());
    Ok(Json(serde_json::json!({ "success": true })))
}

/// POST /api/plugins/publishers/:key_id/revoke -- distrusts a key: every plugin it signed that was approved stops,
/// exactly as if the operator had revoked each one, and approving one again needs a confirmation.
pub(super) async fn revoke_publisher(State(app): State<AppState>, headers: HeaderMap, AxumPath(key_id): AxumPath<String>) -> Result<impl IntoResponse, ApiErr> {
    require_host_token(&headers, &app)?;
    let key = key_for_id(&app, &key_id).ok_or_else(|| err(StatusCode::NOT_FOUND, "no such publisher"))?;
    if key == app.plugin_release_key {
        return Err(err(StatusCode::CONFLICT, "the release key is the app's own and can't be revoked"));
    }
    let actor = actor_of(&headers, &app);
    let name = publisher_name_for(&app, &key);
    app.db.revoke_publisher(&key, &name, &actor, "").map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    let mut stopped = Vec::new();
    for id in app.db.plugins_signed_by(&key).unwrap_or_default() {
        let Some(row) = app.db.get_plugin_grant(&id).ok().flatten() else { continue };
        if row.status == "approved" && row.approved_signer.as_deref() == Some(key.as_str()) {
            let _ = app.db.set_plugin_decision(&id, "revoked", "[]", None, None, None, &actor);
            app.db.add_plugin_audit(&id, &actor, "revoked", &serde_json::json!({ "reason": "publisher revoked", "publisher_key_id": key_id }).to_string());
            app.revoke_plugin_tokens(&id).await;
            stopped.push(id);
        }
    }
    app.db.add_plugin_audit("*", &actor, "publisher_revoked", &serde_json::json!({ "publisher_key_id": key_id, "name": name, "stopped": stopped }).to_string());
    Ok(Json(serde_json::json!({ "success": true, "stopped": stopped })))
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/audit", get(plugin_audit))
        .route("/api/plugins/permissions", get(permission_catalog))
        .route("/api/plugins/publishers", get(list_publishers))
        .route("/api/plugins/revocations", get(revocation_status))
        .route("/api/plugins/publishers/{key_id}/trust", post(trust_publisher))
        .route("/api/plugins/publishers/{key_id}/untrust", post(untrust_publisher))
        .route("/api/plugins/publishers/{key_id}/revoke", post(revoke_publisher))
        .route("/api/plugins/install", post(install_plugin).layer(axum::extract::DefaultBodyLimit::max(MAX_PACKAGE_BYTES + 1024)))
        .route("/api/plugins/{id}", axum::routing::delete(uninstall_plugin))
        .route("/api/plugins/{id}/approve", post(approve_plugin))
        .route("/api/plugins/{id}/deny", post(deny_plugin))
        .route("/api/plugins/{id}/revoke", post(revoke_plugin))
        .route("/api/plugins/{id}/token", post(mint_plugin_token))
        .route("/plugin-frame/{id}/{*path}", get(serve_plugin_frame))
        .route("/api/plugin-host/fetch", post(plugin_fetch))
        .route("/api/plugin-host/songs", post(plugin_add_song))
        .route("/api/plugin-host/scriptures", post(plugin_add_scriptures))
        .route("/api/plugin-host/media", post(plugin_add_media))
        .route("/api/plugin-host/media-url", post(plugin_add_media_url))
        .route("/api/plugin-host/process/start", post(process_start))
        .route("/api/plugin-host/process/poll", post(process_poll))
        .route("/api/plugin-host/process/cancel", post(process_cancel))
        .route("/api/plugin-host/process/release", post(process_release))
        .route("/api/plugin-host/tool/status", post(tool_status))
        .route("/api/plugin-host/tool/update", post(tool_update))
}
