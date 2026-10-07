use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use axum::extract::ConnectInfo;
use std::net::SocketAddr;
use serde::{Deserialize, Serialize};

use crate::api::ws::AppState;

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

/// Merges database-backed resource rows with whatever plugins contribute for the
/// same Resource Library `category` (see `OsPlugin::provide_resources`). When a
/// search `query` is active, plugin-contributed items are filtered by a simple
/// case-insensitive substring match over their serialized JSON — plugins return
/// plain `serde_json::Value`s of arbitrary shape, so this is the one filter that
/// works uniformly across all of them, mirroring the crude-but-effective substring
/// matching `PluginManager::check_command` already uses for the same reason.
pub(super) fn merge_with_plugin_resources<T: Serialize>(
    app: &AppState,
    category: &str,
    items: Vec<T>,
    query: Option<&str>,
) -> Vec<serde_json::Value> {
    let mut combined: Vec<serde_json::Value> = items
        .into_iter()
        .filter_map(|item| serde_json::to_value(item).ok())
        .collect();
    let mut plugin_items = app.plugin_manager.collect_resources(category);
    if let Some(q) = query.filter(|q| !q.trim().is_empty()) {
        let q_lower = q.to_lowercase();
        plugin_items.retain(|item| {
            serde_json::to_string(item)
                .map(|s| s.to_lowercase().contains(&q_lower))
                .unwrap_or(false)
        });
    }
    combined.extend(plugin_items);
    combined
}

/// Shared gate for console-only management actions (minting pairing/remote
/// sessions, listing or revoking paired devices) — anything that either
/// mints new trust or exposes an existing secret/listing. Requires the
/// `x-host-token` header to match this server instance's host session
/// token, which only the actual console can ever obtain (see
/// `get_internal_host_token` above and `src/webview/mod.rs`).
pub(super) fn require_host_token(headers: &HeaderMap, app: &AppState) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let ok = headers
        .get("x-host-token")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == app.host_session_token)
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "success": false, "error": "Missing or invalid x-host-token" })),
        ))
    }
}

/// Combines `require_host_token` with the "one console at a time, first one
/// connected wins" claim (`AppState::try_claim_console`) for the actual
/// show-control surface -- `/api/command` (unpaired callers) and
/// `/api/schedule/open`. Deliberately NOT applied to the console-only admin
/// actions `require_host_token` alone still gates (pairing management,
/// keyring host access): those aren't about who's driving the live show.
/// `x-console-session-id` identifies which browser tab/window is asking, a
/// random id the frontend generates once per page load, so the same
/// console's own HTTP fallback (used when its own WS briefly drops) keeps
/// working while a genuinely different console gets a distinct rejection
/// instead of being silently ignored.
pub(super) fn require_host_token_and_console_lock(headers: &HeaderMap, app: &AppState) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    require_host_token(headers, app)?;
    let session_id = headers
        .get("x-console-session-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim()
        .to_string();
    if session_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "success": false, "error": "Missing x-console-session-id" })),
        ));
    }
    app.try_claim_console(&session_id).map_err(|_| (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "success": false,
            "error": "console_locked",
            "message": "Another console is already connected. Only one console can control the show at a time.",
        })),
    ))
}

/// Minimal validation for a user-supplied public HTTPS URL (the "Public
/// HTTPS URL" setting, docs/TUNNELS.md) -- deliberately not a full RFC 3986
/// parse (no new dependency for this one check), just enough to catch the
/// realistic mistakes (wrong scheme, no host, stray whitespace) before the
/// value is persisted and handed out in QR codes, the startup banner, and
/// `/api/network/info`. Strips a trailing slash so later
/// `format!("{url}/pairing.html")`-style joins never produce a double slash.
pub(super) fn validate_https_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("URL must not be empty".to_string());
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("URL must not contain whitespace or control characters".to_string());
    }
    let Some(rest) = trimmed.strip_prefix("https://") else {
        return Err("URL must start with https://".to_string());
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return Err("URL must include a host, e.g. https://connect.yourchurch.org".to_string());
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

pub(super) fn validate_safe_path(path_str: &str) -> Result<std::path::PathBuf, (StatusCode, String)> {
    let p = std::path::Path::new(path_str);
    for component in p.components() {
        if component == std::path::Component::ParentDir {
            return Err((StatusCode::BAD_REQUEST, "Path traversal forbidden: '..' is not allowed in file paths".to_string()));
        }
    }
    Ok(p.to_path_buf())
}

/// POST /api/internal/verify-host-token
///
/// Lets a non-loopback console (see docs/CLIENT_PAIRING.md "Remote console
/// access") confirm a token it was handed out-of-band -- the printed
/// terminal URL/QR, or one manually pasted in -- actually matches this
/// server's `host_session_token`, before committing to caching it. Safe to
/// expose without the loopback restriction `GET /api/internal/host-token`
/// has: like `POST /api/pairing/verify` above, this only ever answers a
/// yes/no about a caller-supplied guess against a 122-bit random value, not
/// a way to discover or enumerate it.
#[derive(Deserialize)]
pub(super) struct VerifyDeviceRequest {
    pub token: String,
}

pub(super) async fn verify_host_token_handler(
    State(app): State<AppState>,
    Json(req): Json<VerifyDeviceRequest>,
) -> impl IntoResponse {
    Json(serde_json::json!({ "valid": !app.host_session_token.is_empty() && req.token == app.host_session_token }))
}

/// GET /api/internal/host-token
///
/// The only way a web client can ever obtain `host_session_token` — and only
/// when the request's real origin is loopback. A native desktop webview
/// gets the token a different way entirely (injected in-process at window
/// creation, never over the network — see `with_initialization_script` in
/// src/webview/mod.rs); this route exists for the case of an operator using
/// a plain browser tab pointed at 127.0.0.1/localhost, the same address the
/// console's own startup banner already advertises. A LAN client — even a
/// paired one — is never loopback from the server's point of view, so this
/// never reaches them.
///
/// "Real origin" is `resolve_real_client_ip`, not the raw TCP peer: a
/// reverse proxy (Caddy, see docs/TUNNELS.md) running on this same machine
/// makes every request's TCP peer loopback regardless of who's actually
/// asking, since that's genuinely who connected to this process's listener.
/// Trusting `X-Forwarded-For` *only* when the TCP peer is itself loopback
/// closes that gap without opening a new one: no off-machine caller can set
/// that header to fake being loopback, because their own TCP connection
/// isn't loopback in the first place.
pub(super) async fn get_internal_host_token(
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    // A loopback TCP peer isn't enough on its own: any web page the operator
    // has open can make their browser call this loopback address (and, with
    // the default permissive CORS mode, read the answer), and DNS rebinding
    // makes a hostile name resolve to 127.0.0.1. So also require that the
    // browser itself says this is a same-machine, non-cross-site request.
    if !is_same_machine_browser_request(&headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    let forwarded_for = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok());
    let real_ip = crate::network::resolve_real_client_ip(addr.ip(), forwarded_for);
    if !real_ip.is_loopback() {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(Json(serde_json::json!({ "host_token": app.host_session_token })))
}

/// True when `Host` names loopback, any `Origin` present is loopback too, and
/// the browser didn't flag the request `Sec-Fetch-Site: cross-site`. Non-browser
/// clients (curl on the same machine) send no Origin/Sec-Fetch headers and
/// pass as long as Host is loopback.
pub(super) fn is_same_machine_browser_request(headers: &HeaderMap) -> bool {
    fn host_is_loopback(authority: &str) -> bool {
        // strip scheme (Origin values), path, and port; keep IPv6 brackets intact
        let a = authority.split("://").last().unwrap_or(authority);
        let a = a.split('/').next().unwrap_or(a);
        let host = if let Some(rest) = a.strip_prefix('[') {
            rest.split(']').next().unwrap_or("")
        } else {
            a.split(':').next().unwrap_or(a)
        };
        host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
    }
    let host_ok = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(host_is_loopback)
        .unwrap_or(false);
    let origin_ok = headers
        .get(axum::http::header::ORIGIN)
        .map(|v| v.to_str().map(host_is_loopback).unwrap_or(false))
        .unwrap_or(true);
    let site_ok = headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .map(|v| v != "cross-site")
        .unwrap_or(true);
    host_ok && origin_ok && site_ok
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/internal/host-token", get(get_internal_host_token))
        .route("/api/internal/verify-host-token", post(verify_host_token_handler))
}

#[cfg(test)]
mod common_tests {
    use super::{is_same_machine_browser_request, validate_https_url};
    use axum::http::HeaderMap;

    fn hm(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
        }
        h
    }

    #[test]
    fn host_token_requires_a_same_machine_browser_request() {
        assert!(is_same_machine_browser_request(&hm(&[("host", "127.0.0.1:8080")])));
        assert!(is_same_machine_browser_request(&hm(&[("host", "localhost:8080"), ("origin", "http://localhost:8080"), ("sec-fetch-site", "same-origin")])));
        assert!(is_same_machine_browser_request(&hm(&[("host", "[::1]:8080")])));
        // cross-origin page, DNS rebinding, missing Host
        assert!(!is_same_machine_browser_request(&hm(&[("host", "127.0.0.1:8080"), ("origin", "https://evil.example")])));
        assert!(!is_same_machine_browser_request(&hm(&[("host", "127.0.0.1:8080"), ("sec-fetch-site", "cross-site")])));
        assert!(!is_same_machine_browser_request(&hm(&[("host", "attacker.test:8080")])));
        assert!(!is_same_machine_browser_request(&hm(&[("origin", "http://127.0.0.1:8080")])));
        assert!(!is_same_machine_browser_request(&hm(&[("host", "localhost.evil.test")])));
    }

    #[test]
    fn accepts_a_well_formed_https_url_and_strips_trailing_slash() {
        assert_eq!(validate_https_url("https://connect.yourchurch.org").unwrap(), "https://connect.yourchurch.org");
        assert_eq!(validate_https_url("https://connect.yourchurch.org/").unwrap(), "https://connect.yourchurch.org");
        assert_eq!(validate_https_url("  https://connect.yourchurch.org  ").unwrap(), "https://connect.yourchurch.org");
    }

    #[test]
    fn rejects_empty_wrong_scheme_or_missing_host() {
        assert!(validate_https_url("").is_err());
        assert!(validate_https_url("   ").is_err());
        assert!(validate_https_url("http://connect.yourchurch.org").is_err());
        assert!(validate_https_url("connect.yourchurch.org").is_err());
        assert!(validate_https_url("https://").is_err());
        assert!(validate_https_url("https:///path").is_err());
    }

    #[test]
    fn rejects_whitespace_or_control_characters_anywhere_in_the_url() {
        assert!(validate_https_url("https://connect.yourchurch.org/ evil").is_err());
        assert!(validate_https_url("https://connect.yourchurch\n.org").is_err());
        assert!(validate_https_url("https://connect.your\tchurch.org").is_err());
    }
}
