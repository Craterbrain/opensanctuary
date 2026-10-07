mod common;
mod displays;
mod import;
mod keyring;
mod library;
mod network;
mod pairing;
mod reports;
mod schedule;
mod settings;
mod show;
mod system;

pub(crate) use show::apply_default_theme_if_unset;

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::header::CACHE_CONTROL;
use axum::http::HeaderValue;
use axum::middleware::{from_fn_with_state, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::api::ws::{ws_handler, AppState};

/// Determines if an Origin header value represents an authorized sanctuary origin:
/// - localhost, 127.0.0.1, ::1
/// - Private RFC 1918 subnets (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
/// - Link-local (169.254.0.0/16 or fe80::/10)
/// - mDNS domains ending with .local
fn is_allowed_sanctuary_origin(origin_str: &str) -> bool {
    let host_with_port = if let Some(stripped) = origin_str.strip_prefix("http://") {
        stripped
    } else if let Some(stripped) = origin_str.strip_prefix("https://") {
        stripped
    } else {
        origin_str
    };

    let host = host_with_port.split('/').next().unwrap_or(host_with_port);
    let host = host.split(':').next().unwrap_or(host);

    if host == "localhost" || host == "127.0.0.1" || host == "::1" || host.ends_with(".local") {
        return true;
    }

    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        match ip {
            std::net::IpAddr::V4(ipv4) => {
                ipv4.is_loopback() || ipv4.is_private() || ipv4.is_link_local()
            }
            std::net::IpAddr::V6(ipv6) => {
                ipv6.is_loopback() || ipv6.is_unicast_link_local()
            }
        }
    } else {
        false
    }
}

/// Whether `method path` must carry the console's `x-host-token`.
///
/// Deny-by-default for every state-changing `/api/` request, so a newly added
/// POST/DELETE route is protected without anyone remembering to gate its
/// handler. The exceptions authenticate themselves some other way (device
/// token, one-time pairing session token, plugin/host keyring token) or only
/// answer a yes/no about a caller-supplied guess. A few read-only routes that
/// expose secrets or make the server do outbound work/probing are listed too.
/// Read-only show/library data stays open: the live/stage/remote pages that
/// have no host token need it.
pub(crate) fn needs_host_token(method: &axum::http::Method, path: &str) -> bool {
    use axum::http::Method;
    if !path.starts_with("/api/") {
        return false;
    }
    let state_changing = !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    if state_changing {
        return !matches!(
            path,
            "/api/command"                       // device token, or host token + console lock
                | "/api/pairing/authorize"       // one-time pairing session token
                | "/api/pairing/verify"          // yes/no on a guessed device token
                | "/api/internal/verify-host-token"
        ) && (path == "/api/keyring/plugin/token" || !path.starts_with("/api/keyring/"));
    }
    matches!(
        path,
        "/api/network/interfaces"
            | "/api/network/check-port"
            | "/api/network/check-hostname"
            | "/api/schedule/export"
            | "/api/media/online/search"
    ) || path.starts_with("/api/media/ytdlp/progress/")
}

/// Rejects requests `needs_host_token` flags unless `x-host-token` matches.
async fn host_token_gate_middleware(
    axum::extract::State(app): axum::extract::State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    use axum::response::IntoResponse;
    if needs_host_token(req.method(), req.uri().path()) {
        if let Err((status, body)) = common::require_host_token(req.headers(), &app) {
            return (status, body).into_response();
        }
    }
    next.run(req).await
}

/// Injects defense-in-depth security headers (X-Content-Type-Options, X-Frame-Options,
/// and Content-Security-Policy) based on `AppState.security_header_settings` -- an
/// in-memory cache (see its doc comment) rather than a fresh `db.get_setting` pair on
/// every single request/response.
async fn security_headers_middleware(
    axum::extract::State(app): axum::extract::State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let is_media = req.uri().path().starts_with("/media/");
    let mut response = next.run(req).await;

    let (csp_mode, frame_mode) = {
        let cached = app.security_header_settings.read().unwrap_or_else(|e| e.into_inner());
        (cached.csp_mode.clone(), cached.frame_mode.clone())
    };

    let headers = response.headers_mut();

    // 1. X-Content-Type-Options: nosniff
    headers.insert(
        axum::http::header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );

    // 2. X-Frame-Options
    match frame_mode.as_str() {
        "deny" => {
            headers.insert(
                axum::http::header::HeaderName::from_static("x-frame-options"),
                HeaderValue::from_static("DENY"),
            );
        }
        "disabled" => {
            // Omitted to permit all framing (e.g., OBS Studio browser sources)
        }
        _ => {
            // Default: sameorigin
            headers.insert(
                axum::http::header::HeaderName::from_static("x-frame-options"),
                HeaderValue::from_static("SAMEORIGIN"),
            );
        }
    }

    // User-supplied files under /media must never run as a page on the app's
    // origin, whatever their content or extension: sandbox them (no scripts,
    // unique opaque origin) regardless of the operator's CSP mode.
    if is_media {
        headers.insert(
            axum::http::header::HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
        );
        return response;
    }

    // 3. Content-Security-Policy
    let frame_ancestor = match frame_mode.as_str() {
        "deny" => "frame-ancestors 'none';",
        "disabled" => "frame-ancestors *;",
        _ => "frame-ancestors 'self';",
    };

    match csp_mode.as_str() {
        "strict" => {
            let csp_val = format!(
                "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; media-src 'self' data: blob: https:; connect-src 'self' ws: wss: https:; font-src 'self' data: https:; object-src 'none'; base-uri 'self'; form-action 'self'; {}",
                frame_ancestor
            );
            if let Ok(val) = HeaderValue::from_str(&csp_val) {
                headers.insert(axum::http::header::HeaderName::from_static("content-security-policy"), val);
            }
        }
        "disabled" => {
            // Omitted
        }
        _ => {
            // Default: balanced
            let csp_val = format!(
                "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https: http:; media-src 'self' data: blob: https: http:; connect-src 'self' ws: wss:; font-src 'self' data: https:; object-src 'none'; base-uri 'self'; {}",
                frame_ancestor
            );
            if let Ok(val) = HeaderValue::from_str(&csp_val) {
                headers.insert(axum::http::header::HeaderName::from_static("content-security-policy"), val);
            }
        }
    }

    response
}

pub fn create_router(app_state: AppState) -> Router {
    let security_header_settings_for_cors = app_state.security_header_settings.clone();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin, _parts| {
            let mode = security_header_settings_for_cors
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .cors_mode
                .clone();
            if mode == "restricted" {
                if let Ok(origin_str) = origin.to_str() {
                    is_allowed_sanctuary_origin(origin_str)
                } else {
                    false
                }
            } else {
                true
            }
        }))
        .allow_methods(Any)
        .allow_headers(Any);

    let web_dir = app_state.web_dir.clone();
    let media_dir = app_state.media_dir.clone();

    Router::new()
        // WebSocket
        .route("/ws", get(ws_handler))

        // User-downloaded/searched background media -- deliberately not
        // under `web_dir` (see docs/paths.md); served at the same `/media`
        // prefix media was reachable at before the split, so every stored
        // `file_path` like `/media/images/foo.jpg` keeps working unchanged.
        .nest_service("/media", ServeDir::new(media_dir))

        .merge(show::router())
        .merge(common::router())
        .merge(library::router())
        .merge(settings::router())
        .merge(keyring::router())
        .merge(pairing::router())
        .merge(network::router())
        .merge(displays::router())
        .merge(system::router())
        .merge(schedule::router())
        .merge(import::router())
        .merge(reports::router())

        .layer(from_fn_with_state(app_state.clone(), host_token_gate_middleware))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(cors)
        .fallback_service(
            tower::layer::Layer::layer(
                &SetResponseHeaderLayer::overriding(
                    CACHE_CONTROL,
                    HeaderValue::from_static("no-cache, no-store, must-revalidate"),
                ),
                ServeDir::new(web_dir),
            )
        )
        .layer(from_fn_with_state(app_state.clone(), security_headers_middleware))
        .with_state(app_state)
}

#[cfg(test)]
mod gate_tests {
    use super::needs_host_token;
    use axum::http::Method;

    #[test]
    fn state_changing_api_routes_are_gated_by_default() {
        for p in [
            "/api/songs", "/api/scriptures", "/api/media", "/api/themes", "/api/themes/default",
            "/api/presentations", "/api/slide-templates", "/api/import/openlp", "/api/import/pptx",
            "/api/import/freeshow-show", "/api/bibles/online/download", "/api/bibles/online/fetch",
            "/api/bibles/github/catalog", "/api/songs/genius/import", "/api/media/ytdlp/download",
            "/api/media/online/import", "/api/displays/open", "/api/displays/close",
            "/api/displays/refresh", "/api/system/pick-folder", "/api/system/pick-file",
            "/api/network/tls/regenerate", "/api/network/interfaces", "/api/keyring/plugin/token",
            "/api/some/future/route",
        ] {
            assert!(needs_host_token(&Method::POST, p), "POST {p}");
        }
        assert!(needs_host_token(&Method::DELETE, "/api/songs/x"));
    }

    #[test]
    fn self_authenticating_and_read_only_routes_stay_open() {
        for p in ["/api/command", "/api/pairing/authorize", "/api/pairing/verify",
                  "/api/internal/verify-host-token", "/api/keyring/get", "/api/keyring/set"] {
            assert!(!needs_host_token(&Method::POST, p), "POST {p}");
        }
        for p in ["/api/state", "/api/settings", "/api/songs", "/api/media", "/api/themes",
                  "/api/network/info", "/api/server-info", "/api/pairing/status", "/api/internal/host-token"] {
            assert!(!needs_host_token(&Method::GET, p), "GET {p}");
        }
        assert!(!needs_host_token(&Method::OPTIONS, "/api/songs"));
        assert!(!needs_host_token(&Method::POST, "/media/x"));
        assert!(needs_host_token(&Method::GET, "/api/schedule/export"));
        assert!(needs_host_token(&Method::GET, "/api/network/interfaces"));
        assert!(needs_host_token(&Method::GET, "/api/media/ytdlp/progress/abc"));
    }
}
