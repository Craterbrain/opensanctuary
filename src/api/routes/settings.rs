use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};

use crate::api::ws::AppState;

use super::common::{require_host_token, validate_https_url};

/// Setting keys that hold credentials (e.g. `pexelsApiKey`, `pixabayApiKey`).
fn is_secret_setting_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ["apikey", "password", "secret", "token"].iter().any(|s| k.contains(s))
}

/// Unauthenticated callers (live/stage/remote pages, any LAN client) get the
/// settings with credential-bearing keys removed; only the console, holding
/// the host token, sees them (the Settings dialog edits them).
pub(super) async fn get_settings(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let mut settings = app.db.get_settings().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if require_host_token(&headers, &app).is_err() {
        settings.retain(|k, _| !is_secret_setting_key(k));
    }
    Ok(Json(settings))
}

pub(super) async fn post_settings(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(mut settings): Json<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse, StatusCode> {
    require_host_token(&headers, &app).map_err(|(status, _)| status)?;

    // `networkHostname`/`networkPort` used to save unconditionally here even
    // though the read-only /api/network/check-hostname and /check-port
    // endpoints already compute whether the value collides with something
    // else on the network — the UI's warning badge had nothing enforcing it
    // server-side. A flagged key is now held back (the rest of the payload
    // still saves normally —
    // e.g. CSP/CORS settings bundled in the same request); the frontend can
    // resubmit with `_confirmOverrides` (comma-separated key names) once the
    // operator has explicitly acknowledged the warning it was already shown.
    let forced: std::collections::HashSet<String> = settings
        .remove("_confirmOverrides")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    let mut conflicts = serde_json::Map::new();

    if let Some(hostname) = settings.get("networkHostname").cloned() {
        if !forced.contains("networkHostname") {
            let self_ips = crate::network::get_all_local_ips();
            let result = crate::network::check_hostname_conflict(&hostname, &self_ips);
            if result.conflict_detected {
                settings.remove("networkHostname");
                conflicts.insert(
                    "networkHostname".to_string(),
                    serde_json::json!({
                        "conflicting_ip": result.conflicting_ip,
                        "suggested_hostname": result.suggested_hostname,
                        "message": result.message,
                    }),
                );
            }
        }
    }

    // Unlike the hostname/port conflicts below, an invalid Public HTTPS URL
    // has no legitimate "save it anyway" override -- it's a format error
    // (wrong scheme, no host), not a soft warning about a LAN collision the
    // operator might knowingly accept. An empty string clears the setting
    // (no validation needed to remove something).
    if let Some(url) = settings.get("publicHttpsUrl").cloned() {
        if !url.trim().is_empty() {
            if let Err(message) = validate_https_url(&url) {
                settings.remove("publicHttpsUrl");
                conflicts.insert(
                    "publicHttpsUrl".to_string(),
                    serde_json::json!({ "message": message }),
                );
            }
        }
    }

    // Same reasoning as publicHttpsUrl above: a malformed date has no
    // legitimate "save it anyway" -- it would just silently break the
    // due-date reminder's own date math. Empty clears the setting.
    if let Some(due_date) = settings.get("ccliReportingDueDate").cloned() {
        if !due_date.trim().is_empty() && chrono::NaiveDate::parse_from_str(due_date.trim(), "%Y-%m-%d").is_err() {
            settings.remove("ccliReportingDueDate");
            conflicts.insert(
                "ccliReportingDueDate".to_string(),
                serde_json::json!({ "message": "Date must be in YYYY-MM-DD format" }),
            );
        }
    }

    if let Some(port_str) = settings.get("networkPort").cloned() {
        if !forced.contains("networkPort") {
            if let Ok(port) = port_str.parse::<u16>() {
                let result = crate::network::check_port_conflict(port, Some(app.server_port));
                if !result.available {
                    settings.remove("networkPort");
                    conflicts.insert(
                        "networkPort".to_string(),
                        serde_json::json!({
                            "message": result.message,
                            "suggested_alternative": result.suggested_alternative,
                        }),
                    );
                }
            }
        }
    }

    for (k, v) in settings {
        if let Err(e) = app.db.set_setting(&k, &v) {
            tracing::warn!("Failed to persist setting key '{}': {}", k, e);
        }
    }
    let updated = app.db.get_settings().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Keep the in-memory cache `security_headers_middleware`/the CORS
    // predicate read on every request in sync with what was just persisted
    // -- see `AppState::security_header_settings`'s doc comment.
    if let Ok(mut cached) = app.security_header_settings.write() {
        cached.csp_mode = updated.get("securityCspMode").cloned().unwrap_or_else(|| "balanced".to_string());
        cached.frame_mode = updated.get("securityFrameOptions").cloned().unwrap_or_else(|| "sameorigin".to_string());
        cached.cors_mode = updated.get("securityCorsMode").cloned().unwrap_or_else(|| "permissive".to_string());
    }

    let mut response = serde_json::to_value(&updated).unwrap_or_default();
    if !conflicts.is_empty() {
        if let Some(obj) = response.as_object_mut() {
            obj.insert("_conflicts".to_string(), serde_json::Value::Object(conflicts));
        }
    }
    Ok(Json(response))
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::get;
    Router::new().route("/api/settings", get(get_settings).post(post_settings))
}

#[cfg(test)]
mod tests {
    use super::is_secret_setting_key;

    #[test]
    fn flags_credential_keys_only() {
        assert!(is_secret_setting_key("pexelsApiKey"));
        assert!(is_secret_setting_key("pixabayApiKey"));
        assert!(is_secret_setting_key("ccliPassword"));
        assert!(!is_secret_setting_key("churchName"));
        assert!(!is_secret_setting_key("networkPort"));
    }
}
