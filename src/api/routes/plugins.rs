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
use crate::core::plugin_registry::{discover, parse_granted, sync, Discovered};
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

/// Short, stable id for a publisher key, for display.
fn signer_id(key: &str) -> String {
    crate::core::plugin_permissions::sha256_hex(key.as_bytes())[..16].to_string()
}

fn read_manifest(row: &PluginGrantRow) -> Option<crate::core::plugin_permissions::PluginManifest> {
    std::fs::read(std::path::Path::new(&row.source_path).join("plugin.json")).ok().and_then(|b| crate::core::plugin_permissions::PluginManifest::parse(&b).ok())
}

fn view(app: &AppState, row: &PluginGrantRow) -> serde_json::Value {
    let requested: Vec<PermissionRequest> = serde_json::from_str(&row.requested_json).unwrap_or_default();
    let granted: Vec<PermissionRequest> = serde_json::from_str(&row.granted_json).unwrap_or_default();
    let state = row.effective_state();
    let describe = |p: &PermissionRequest| {
        let mut v = serde_json::to_value(p).unwrap_or_default();
        v["tier"] = serde_json::to_value(p.tier()).unwrap_or_default();
        v["enforcement"] = serde_json::Value::String(p.enforcement_for(&row.kind).to_string());
        v
    };
    let native = row.kind == "native" || row.kind == "legacy-native";
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
    serde_json::json!({
        "commands": commands,
        "tools": tools,
        "placements": read_manifest(row).and_then(|m| m.ui).map(|u| u.placements).unwrap_or_default(),
        "entry": entry,
        "id": row.plugin_id,
        "kind": row.kind,
        "name": row.name,
        "version": row.version,
        "sandboxed": row.kind == "sandbox",
        "state": state,
        "status": row.status,
        "error": row.error,
        "digest": row.observed_digest,
        "signed": row.observed_signer.is_some(),
        "signer_id": row.observed_signer.as_deref().map(signer_id),
        // Approved before under a different publisher key, or it used to be signed and now isn't.
        "signer_changed": row.approved_signer.is_some() && row.approved_signer != row.observed_signer,
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
            Some("web") => r.kind == "web" || r.kind == "legacy-web" || r.kind == "sandbox",
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
        .set_plugin_decision(&id, "approved", &granted_json, row.observed_manifest_sha256.as_deref(), Some(&row.observed_digest), row.observed_signer.as_deref(), &actor)
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
        .set_plugin_decision(&id, status, "[]", None, None, None, &actor)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;
    app.db.add_plugin_audit(&id, &actor, if status == "denied" { "denied" } else { "revoked" }, "{}");
    // Immediate for everything the host mediates (hooks, resources).
    app.plugin_manager.revoke(&id);
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
const TOKEN_SCOPES: [&str; 6] = ["keyring.own", "commands.send", "network.outbound", "library.write", "process.run", "notifications.post"];

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
    if row.effective_state() != "active" {
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
    match crate::api::plugin_net::fetch(&hosts, &req.url).await {
        Ok(f) => Ok(Json(serde_json::json!({
            "success": true, "status": f.status, "content_type": f.content_type, "body": f.body, "url": f.final_url,
        }))),
        Err(e) => {
            app.db.add_plugin_audit(
                &caller.plugin_id,
                &format!("plugin:{}", caller.plugin_id),
                "denied_call",
                &serde_json::json!({ "fetch": req.url.chars().take(200).collect::<String>(), "reason": e }).to_string(),
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
const MAX_VERSE_TEXT: usize = 4000;

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
            return Err(err(StatusCode::BAD_REQUEST, "a chapter needs 1-400 verses of up to 4000 characters"));
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
        .filter(|r| r.kind == "sandbox" && r.effective_state() == "active")
        .ok_or_else(|| err(StatusCode::FORBIDDEN, "this plugin isn't approved"))?;
    let dir = app.web_dir.join("plugins").join(plugin_id);
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
    !h.is_empty()
        && h.len() <= 255
        && h.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '*'))
        && (!h.contains('*') || h.starts_with("*."))
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
    format!(
        "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
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
    if row.kind != "sandbox" || row.effective_state() != "active" {
        return not_found();
    }
    let dir = app.web_dir.join("plugins").join(&id);
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

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/audit", get(plugin_audit))
        .route("/api/plugins/:id/approve", post(approve_plugin))
        .route("/api/plugins/:id/deny", post(deny_plugin))
        .route("/api/plugins/:id/revoke", post(revoke_plugin))
        .route("/api/plugins/:id/token", post(mint_plugin_token))
        .route("/plugin-frame/:id/*path", get(serve_plugin_frame))
        .route("/api/plugin-host/fetch", post(plugin_fetch))
        .route("/api/plugin-host/songs", post(plugin_add_song))
        .route("/api/plugin-host/scriptures", post(plugin_add_scriptures))
        .route("/api/plugin-host/media", post(plugin_add_media))
        .route("/api/plugin-host/process/start", post(process_start))
        .route("/api/plugin-host/process/poll", post(process_poll))
        .route("/api/plugin-host/process/cancel", post(process_cancel))
        .route("/api/plugin-host/process/release", post(process_release))
        .route("/api/plugin-host/tool/status", post(tool_status))
        .route("/api/plugin-host/tool/update", post(tool_update))
}
