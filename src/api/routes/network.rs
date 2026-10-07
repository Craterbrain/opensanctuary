use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::http::StatusCode;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::ws::AppState;

use super::common::{require_host_token, validate_https_url};

/// GET /api/network/info
/// Returns host network LAN IP, port, remote URL, and mDNS URL for phone QR scanning.
pub(super) async fn get_network_info(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let dedicated = crate::network::VirtualAdapterManager::get_dedicated_adapter_info();
    let (lan_ip, is_dedicated) = if let Some(ref d) = dedicated {
        (
            d.ipv4
                .clone()
                .unwrap_or_else(|| crate::network::get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())),
            true,
        )
    } else {
        (
            crate::network::get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string()),
            false,
        )
    };

    let http_port = app.server_port;
    let https_port = app.https_port;
    let https_enabled = app.https_enabled;

    let port = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(extract_port_from_host_header)
        .unwrap_or(app.server_port);

    let db_settings = app.db.get_settings().unwrap_or_default();
    let hostname = db_settings
        .get("networkHostname")
        .cloned()
        .unwrap_or_else(|| "opensanctuary".to_string());
    let clean_hostname = crate::network::sanitize_rfc1035_hostname(&hostname);
    let public_https_url = db_settings
        .get("publicHttpsUrl")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // https_port/http_port: the loopback-only plane (http_port, a.k.a.
    // app.server_port) is never reachable from the network -- see
    // src/main.rs's loopback-bind comment -- so every URL handed to a
    // non-loopback caller (which is the whole point of this endpoint) must
    // go over HTTPS. The http:// fallback only applies in the rare case
    // HTTPS itself failed to start (src/main.rs logs this loudly); even
    // then the resulting URL won't actually be reachable remotely, but it's
    // at least valid for whoever's sitting at this exact machine.
    // A configured Public HTTPS URL (docs/TUNNELS.md -- a church running
    // Caddy as a reverse proxy with a real Let's Encrypt cert) always wins
    // over the self-signed lan_ip:https_port form: it's the whole point of
    // setting it, and unlike the self-signed cert, a browser never shows a
    // warning for it.
    let remote_url = if let Some(ref base) = public_https_url {
        format!("{}/remote", base)
    } else if https_enabled && https_port.is_some() {
        format!("https://{}:{}/remote", lan_ip, https_port.unwrap())
    } else {
        format!("http://{}:{}/remote", lan_ip, http_port)
    };
    let mdns_url = if https_enabled && https_port.is_some() {
        format!("https://{}.local:{}/remote", clean_hostname, https_port.unwrap())
    } else {
        format!("http://{}.local:{}/remote", clean_hostname, http_port)
    };

    let pairing_url = if let Some(ref base) = public_https_url {
        format!("{}/pairing.html", base)
    } else if https_enabled && https_port.is_some() {
        format!("https://{}:{}/pairing.html", lan_ip, https_port.unwrap())
    } else {
        format!("http://{}:{}/pairing.html", lan_ip, http_port)
    };

    let mdns_pairing_url = if https_enabled && https_port.is_some() {
        format!("https://{}.local:{}/pairing.html", clean_hostname, https_port.unwrap())
    } else {
        format!("http://{}.local:{}/pairing.html", clean_hostname, http_port)
    };

    Json(serde_json::json!({
        "success": true,
        "lan_ip": lan_ip,
        "port": port,
        "http_port": http_port,
        "https_port": https_port,
        "https_enabled": https_enabled,
        "public_https_url": public_https_url,
        "remote_url": remote_url,
        "mdns_url": mdns_url,
        "pairing_url": pairing_url,
        "mdns_pairing_url": mdns_pairing_url,
        "hostname": hostname,
        "is_dedicated": is_dedicated,
        "dedicated_mac": dedicated.and_then(|d| d.mac_address)
    }))
}

pub(super) fn extract_port_from_host_header(host: &str) -> Option<u16> {
    if host.ends_with(']') {
        return None;
    }
    let idx = host.rfind(':')?;
    if let Some(bracket_idx) = host.rfind(']') {
        if idx < bracket_idx {
            return None;
        }
    }
    host[idx + 1..].parse::<u16>().ok()
}

/// GET /api/network/interfaces
/// Returns all network interfaces, enabled states, and dedicated adapter status.
pub(super) async fn get_network_interfaces_handler(State(app): State<AppState>) -> impl IntoResponse {
    let db_settings = app.db.get_settings().unwrap_or_default();
    let enabled_filter_str = db_settings.get("networkEnabledInterfaces");
    let enabled_filter: Option<Vec<String>> = enabled_filter_str.map(|s| {
        s.split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect()
    });

    let interfaces = crate::network::get_network_interfaces(enabled_filter.as_deref());
    let broadcast_all = db_settings
        .get("networkBroadcastAll")
        .map(|v| v != "false")
        .unwrap_or(true);
    let hostname = db_settings
        .get("networkHostname")
        .cloned()
        .unwrap_or_else(|| "opensanctuary".to_string());
    let dedicated_adapter = crate::network::VirtualAdapterManager::get_dedicated_adapter_info();
    let is_dedicated_active = crate::network::VirtualAdapterManager::is_dedicated_adapter_active();
    let https_port = app.https_port;
    let https_enabled = app.https_enabled;

    Json(serde_json::json!({
        "success": true,
        "interfaces": interfaces,
        "broadcast_all": broadcast_all,
        "hostname": hostname,
        "port": app.server_port,
        "http_port": app.server_port,
        "https_port": https_port,
        "https_enabled": https_enabled,
        "dedicated_active": is_dedicated_active,
        "dedicated_adapter": dedicated_adapter
    }))
}

#[derive(Deserialize)]
pub(super) struct SaveInterfacesRequest {
    enabled_interfaces: Vec<String>,
    broadcast_all: Option<bool>,
}

/// POST /api/network/interfaces
/// Saves enabled interfaces and broadcast_all preference.
pub(super) async fn post_network_interfaces_handler(
    State(app): State<AppState>,
    Json(payload): Json<SaveInterfacesRequest>,
) -> impl IntoResponse {
    let joined = payload.enabled_interfaces.join(",");
    let _ = app.db.set_setting("networkEnabledInterfaces", &joined);
    if let Some(b_all) = payload.broadcast_all {
        let _ = app
            .db
            .set_setting("networkBroadcastAll", if b_all { "true" } else { "false" });
    }
    Json(serde_json::json!({ "success": true }))
}

#[derive(Deserialize)]
pub(super) struct CheckPortQuery {
    port: u16,
}

/// GET /api/network/check-port?port=8080
/// Checks if a port is available, accounting for the current running server port.
pub(super) async fn check_port_handler(
    State(app): State<AppState>,
    Query(query): Query<CheckPortQuery>,
) -> impl IntoResponse {
    let result = crate::network::check_port_conflict(query.port, Some(app.server_port));
    Json(serde_json::json!({
        "success": true,
        "port": result.port,
        "available": result.available,
        "in_use_by_current": result.in_use_by_current,
        "message": result.message,
        "suggested_alternative": result.suggested_alternative
    }))
}

#[derive(Deserialize)]
pub(super) struct CheckHostnameQuery {
    name: String,
}

/// GET /api/network/check-hostname?name=opensanctuary
/// Checks whether a hostname has conflicts on the local network, excluding self IPs.
pub(super) async fn check_hostname_handler(Query(query): Query<CheckHostnameQuery>) -> impl IntoResponse {
    let self_ips = crate::network::get_all_local_ips();
    let result = crate::network::check_hostname_conflict(&query.name, &self_ips);
    Json(serde_json::json!({
        "success": true,
        "hostname": result.hostname,
        "sanitized_hostname": result.sanitized_hostname,
        "conflict_detected": result.conflict_detected,
        "conflicting_ip": result.conflicting_ip,
        "suggested_hostname": result.suggested_hostname,
        "message": result.message
    }))
}

#[derive(Deserialize)]
pub(super) struct CheckPublicUrlQuery {
    url: String,
}

/// GET /api/network/check-public-url?url=https://connect.yourchurch.org
/// Live-verifies a candidate Public HTTPS URL (docs/TUNNELS.md) actually
/// round-trips back to *this* server, not just "some HTTPS server
/// answered" -- by GETting `{url}/api/server-info` (zero-auth, see
/// `show.rs::get_server_info`) and comparing its `instance_id` against this
/// server's own (`crate::discovery::get_or_create_instance_id`).
///
/// Requires the host token even though the probe target itself is
/// zero-auth: this makes the *server* perform an outbound HTTPS request to
/// an admin-supplied URL, which is exactly the shape of an SSRF primitive
/// if any unauthenticated LAN caller could trigger it (e.g. port/service-
/// probing other LAN hosts under cover of "checking my public URL"). An
/// operator who already holds the host token could do the same probe from
/// their own browser anyway, so this isn't granting new capability, only
/// keeping it restricted to who could already reach it.
pub(super) async fn check_public_url_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CheckPublicUrlQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;

    let base = match validate_https_url(&query.url) {
        Ok(u) => u,
        Err(message) => {
            return Ok(Json(serde_json::json!({ "success": true, "reachable": false, "message": message })));
        }
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return Ok(Json(serde_json::json!({
                "success": true,
                "reachable": false,
                "message": format!("Failed to build HTTP client: {}", e)
            })));
        }
    };

    let expected_instance_id = crate::discovery::get_or_create_instance_id(&app.db);
    let probe_url = format!("{}/api/server-info", base);

    let (reachable, message) = match client.get(&probe_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let body = resp.json::<serde_json::Value>().await.ok();
            classify_probe_response(&expected_instance_id, true, body.as_ref())
        }
        Ok(resp) => (false, format!("Reached a server, but it returned HTTP {}.", resp.status())),
        Err(e) if e.is_timeout() => (
            false,
            "Timed out -- DNS may not have propagated yet, or Caddy/the port-forward isn't up.".to_string(),
        ),
        Err(e) => (false, format!("Not reachable: {}", e)),
    };

    Ok(Json(serde_json::json!({
        "success": true,
        "reachable": reachable,
        "message": message
    })))
}

/// Decides reachable/not-reachable from an already-received `/api/server-info`
/// response, separated out from `check_public_url_handler` so this decision
/// logic -- the part actually worth testing -- doesn't require a real HTTPS
/// round trip (self-signed-vs-real-cert handling is reqwest's own,
/// already-tested job, not this function's).
pub(super) fn classify_probe_response(
    expected_instance_id: &str,
    status_is_success: bool,
    body: Option<&serde_json::Value>,
) -> (bool, String) {
    if !status_is_success {
        return (false, "Reached a server at this URL, but it returned an error status.".to_string());
    }
    let Some(body) = body else {
        return (false, "Reached a server at this URL, but it didn't look like OS-Next.".to_string());
    };
    let got_id = body.get("instance_id").and_then(|v| v.as_str()).unwrap_or("");
    if !got_id.is_empty() && got_id == expected_instance_id {
        (true, "Reachable -- this URL reaches this server.".to_string())
    } else {
        (false, "Reached a server at this URL, but it isn't this one -- check the domain and local DNS override.".to_string())
    }
}

#[derive(Deserialize)]
pub(super) struct DedicatedMacToggleRequest {
    enable: bool,
    parent_interface: Option<String>,
    hostname: Option<String>,
    #[serde(default)]
    confirm_conflict: bool,
}

/// POST /api/network/dedicated-mac/toggle
/// Enables or disables a dedicated virtual MAC/IP network adapter using Polkit (Linux) or UAC (Windows).
pub(super) async fn toggle_dedicated_mac_handler(
    State(app): State<AppState>,
    Json(payload): Json<DedicatedMacToggleRequest>,
) -> impl IntoResponse {
    let db_settings = app.db.get_settings().unwrap_or_default();
    let hostname = payload.hostname.unwrap_or_else(|| {
        db_settings
            .get("networkHostname")
            .cloned()
            .unwrap_or_else(|| "opensanctuary".to_string())
    });

    // This hostname becomes the new adapter's network identity — check it the
    // same way /api/network/check-hostname already does before actually
    // creating the adapter, rather than only offering the check as advisory
    // UI.
    if payload.enable && !payload.confirm_conflict {
        let self_ips = crate::network::get_all_local_ips();
        let conflict = crate::network::check_hostname_conflict(&hostname, &self_ips);
        if conflict.conflict_detected {
            return Json(serde_json::json!({
                "success": false,
                "enabled": false,
                "conflict": true,
                "conflicting_ip": conflict.conflicting_ip,
                "suggested_hostname": conflict.suggested_hostname,
                "message": conflict.message,
            }));
        }
    }

    if payload.enable {
        let parent = payload.parent_interface.unwrap_or_else(|| {
            let interfaces = crate::network::get_network_interfaces(None);
            interfaces
                .into_iter()
                .find(|i| !i.is_loopback && !i.is_virtual && i.is_up)
                .map(|i| i.name)
                .unwrap_or_else(|| "eth0".to_string())
        });

        // Interface names reach elevated shell/PowerShell scripts; allow only
        // plain characters on every platform (Windows names may contain spaces).
        let parent_ok = !parent.is_empty()
            && parent.len() <= 64
            && parent.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && parent.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '(' | ')'));
        if !parent_ok {
            return Json(serde_json::json!({ "success": false, "error": "Invalid parent interface name" }));
        }

        match crate::network::VirtualAdapterManager::enable_dedicated_adapter(&parent, &hostname) {
            Ok(res) => {
                let _ = app.db.set_setting("networkDedicatedMac", "true");
                Json(serde_json::json!({
                    "success": true,
                    "enabled": true,
                    "adapter_name": res.adapter_name,
                    "parent_interface": res.parent_interface,
                    "interface_type": res.interface_type,
                    "mac_address": res.mac_address,
                    "dedicated_ip": res.dedicated_ip,
                    "message": res.message
                }))
            }
            Err(e) => Json(serde_json::json!({
                "success": false,
                "enabled": false,
                "message": e
            })),
        }
    } else {
        match crate::network::VirtualAdapterManager::disable_dedicated_adapter() {
            Ok(()) => {
                let _ = app.db.set_setting("networkDedicatedMac", "false");
                Json(serde_json::json!({
                    "success": true,
                    "enabled": false,
                    "message": "Dedicated virtual adapter successfully removed."
                }))
            }
            Err(e) => Json(serde_json::json!({
                "success": false,
                "enabled": true,
                "message": e
            })),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct BroadcastOption12Request {
    hostname: Option<String>,
    interface_name: Option<String>,
}

/// POST /api/network/broadcast-option12
/// Broadcasts DHCP Option 12 Host Name across interfaces.
pub(super) async fn broadcast_option12_handler(
    State(app): State<AppState>,
    Json(payload): Json<BroadcastOption12Request>,
) -> impl IntoResponse {
    let db_settings = app.db.get_settings().unwrap_or_default();
    let hostname = payload.hostname.unwrap_or_else(|| {
        db_settings
            .get("networkHostname")
            .cloned()
            .unwrap_or_else(|| "opensanctuary".to_string())
    });

    let mut interfaces = crate::network::get_network_interfaces(None);
    if let Some(specific_iface) = payload.interface_name {
        interfaces.retain(|i| i.name == specific_iface);
    } else if let Some(enabled_str) = db_settings.get("networkEnabledInterfaces") {
        let enabled: Vec<&str> = enabled_str.split(',').map(|s| s.trim()).collect();
        interfaces.retain(|i| enabled.contains(&i.name.as_str()));
    }

    let results = crate::network::DhcpOption12Client::broadcast_option_12(&hostname, &interfaces);
    let success_count = results.iter().filter(|r| r.success).count();

    Json(serde_json::json!({
        "success": success_count > 0 || results.is_empty(),
        "hostname": hostname,
        "results": results,
        "success_count": success_count,
        "total_targets": results.len()
    }))
}

/// POST /api/network/tls/regenerate
/// Regenerates self-signed TLS certificate with current SANs and persists it.
pub(super) async fn regenerate_tls_cert_handler(State(app): State<AppState>) -> impl IntoResponse {
    let db_settings = app.db.get_settings().unwrap_or_default();
    let hostname = db_settings
        .get("networkHostname")
        .cloned()
        .unwrap_or_else(|| "opensanctuary".to_string());
    let enabled_filter_str = db_settings.get("networkEnabledInterfaces");
    let enabled_filter: Option<Vec<String>> = enabled_filter_str.map(|s| {
        s.split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect()
    });
    let interfaces = crate::network::get_network_interfaces(enabled_filter.as_deref());

    match crate::network::tls::regenerate_tls_certificate(&app.db, &hostname, &interfaces) {
        Ok(_) => Json(serde_json::json!({
            "success": true,
            "message": "TLS certificate regenerated successfully. Restart server to apply new certificate."
        })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": e
            }))
        ).into_response(),
    }
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/network/info", get(get_network_info))
        .route("/api/network/interfaces", get(get_network_interfaces_handler).post(post_network_interfaces_handler))
        .route("/api/network/check-port", get(check_port_handler))
        .route("/api/network/check-hostname", get(check_hostname_handler))
        .route("/api/network/check-public-url", get(check_public_url_handler))
        .route("/api/network/dedicated-mac/toggle", post(toggle_dedicated_mac_handler))
        .route("/api/network/broadcast-option12", post(broadcast_option12_handler))
        .route("/api/network/tls/regenerate", post(regenerate_tls_cert_handler))
}

#[cfg(test)]
mod network_tests {
    use super::{classify_probe_response, extract_port_from_host_header};

    #[test]
    fn extracts_port_correctly_from_host_headers() {
        assert_eq!(extract_port_from_host_header("localhost:8080"), Some(8080));
        assert_eq!(extract_port_from_host_header("192.168.1.50:9000"), Some(9000));
        assert_eq!(extract_port_from_host_header("[::1]:9035"), Some(9035));
        assert_eq!(extract_port_from_host_header("[2001:db8::1]:8080"), Some(8080));
        assert_eq!(extract_port_from_host_header("[::1]"), None);
        assert_eq!(extract_port_from_host_header("localhost"), None);
        assert_eq!(extract_port_from_host_header("example.com:notaport"), None);
    }

    #[test]
    fn classify_probe_response_reachable_when_instance_id_matches() {
        let body = serde_json::json!({ "instance_id": "abc-123", "name": "" });
        let (reachable, _) = classify_probe_response("abc-123", true, Some(&body));
        assert!(reachable);
    }

    #[test]
    fn classify_probe_response_not_reachable_when_instance_id_differs() {
        // Proves a URL that happens to reach *some* HTTPS server (a
        // misconfigured DNS override pointing at the wrong box, say) isn't
        // mistaken for reaching THIS one.
        let body = serde_json::json!({ "instance_id": "someone-elses-server", "name": "" });
        let (reachable, message) = classify_probe_response("abc-123", true, Some(&body));
        assert!(!reachable);
        assert!(message.contains("isn't this one"));
    }

    #[test]
    fn classify_probe_response_not_reachable_on_error_status() {
        let (reachable, _) = classify_probe_response("abc-123", false, None);
        assert!(!reachable);
    }

    #[test]
    fn classify_probe_response_not_reachable_when_body_is_not_server_info_shaped() {
        let body = serde_json::json!({ "something_else": true });
        let (reachable, _) = classify_probe_response("abc-123", true, Some(&body));
        assert!(!reachable);
    }

    #[test]
    fn classify_probe_response_not_reachable_when_body_missing() {
        let (reachable, message) = classify_probe_response("abc-123", true, None);
        assert!(!reachable);
        assert!(message.contains("didn't look like OS-Next"));
    }
}
