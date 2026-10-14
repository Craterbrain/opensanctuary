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
    serde_json::json!({
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
const TOKEN_SCOPES: [&str; 4] = ["keyring.own", "commands.send", "network.outbound", "library.write"];

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
async fn plugin_caller(app: &AppState, headers: &HeaderMap) -> Result<crate::api::ws::PluginCaller, ApiErr> {
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

fn current_folder_digest(id: &str, dir: &std::path::Path) -> Option<String> {
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
    let file = dir.join(&rel);
    match std::fs::symlink_metadata(&file) {
        Ok(m) if m.is_file() => {}
        _ => return not_found(),
    }
    let Ok(bytes) = std::fs::read(&file) else { return not_found() };

    let granted = parse_granted(&row);
    let mut sources: Vec<String> = Vec::new();
    for p in granted.0.iter().filter(|p| p.id == "network.outbound") {
        for h in p.hosts.iter().filter(|h| csp_host_ok(h)) {
            sources.push(format!("https://{h} wss://{h} http://{h} ws://{h}"));
        }
    }
    let connect = if sources.is_empty() { "'none'".to_string() } else { sources.join(" ") };
    let csp = format!(
        "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
         font-src 'self' data:; media-src 'self' blob:; connect-src {connect}; base-uri 'none'; form-action 'none'; \
         frame-ancestors 'self'; sandbox allow-scripts"
    );
    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type_for(&rel))
        .header(header::CONTENT_SECURITY_POLICY, csp)
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(bytes))
        .unwrap_or_else(|_| not_found())
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
}
