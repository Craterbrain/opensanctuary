//! LAN discovery for OS-Next consoles.
//!
//! Broadcasts this server's presence via mDNS (Bonjour/Zeroconf) so that
//! clients on the same network — the operator console itself doesn't need
//! this, but a Roku or Android TV app displaying `live.html`/`stage.html`
//! does — can find it without the user typing an IP address.
//!
//! This module only handles *discovery* (finding the console and knowing
//! which one you found). It does not implement the client-pairing /
//! hijack-prevention model described in `docs/CLIENT_PAIRING.md` — that's
//! deliberately left as a documented design for the future Roku and Android
//! TV apps to implement, since it's meaningless without a real client on the
//! other end. See that document before building either app.

use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;

use crate::storage::Database;

const SETTING_KEY_INSTANCE_ID: &str = "server_instance_id";
const SERVICE_TYPE: &str = "_opensanctuary._tcp.local.";

/// Returns this server's stable identity, generating and persisting a new
/// one on first run. This is the value a client pins to when it first pairs
/// (see `docs/CLIENT_PAIRING.md`) — it must never change for the lifetime of
/// a given install, or every paired client will (correctly) treat the
/// console as a different one and refuse it.
pub fn get_or_create_instance_id(db: &Database) -> String {
    if let Ok(settings) = db.get_settings() {
        if let Some(id) = settings.get(SETTING_KEY_INSTANCE_ID) {
            if !id.trim().is_empty() {
                return id.clone();
            }
        }
    }
    let new_id = uuid::Uuid::new_v4().to_string();
    if let Err(e) = db.set_setting(SETTING_KEY_INSTANCE_ID, &new_id) {
        tracing::warn!("Failed to persist server_instance_id (will regenerate on next boot): {}", e);
    }
    new_id
}

/// Starts advertising this console on the local network via mDNS. Returns
/// the `ServiceDaemon` — keep it alive for as long as the server should stay
/// discoverable; dropping it stops the advertisement (its background thread
/// unregisters the service on shutdown).
///
/// Advertises under the custom service type `_opensanctuary._tcp.local.`
/// (not plain `_http._tcp`) with `<hostname>.local.` so a client can
/// browse specifically for OS-Next consoles instead of every HTTP server on
/// the LAN. Note: mDNS only resolves the `.local` TLD (RFC 6762) — `.lan` is
/// not a real mDNS domain and nothing will answer for it.
pub fn start_mdns_broadcast(
    instance_id: &str,
    church_name: &str,
    hostname: &str,
    port: u16,
    https_port: Option<u16>,
) -> Result<ServiceDaemon, String> {
    let daemon = ServiceDaemon::new().map_err(|e| format!("failed to start mDNS daemon: {e}"))?;

    let instance_name = if church_name.trim().is_empty() {
        "OpenSanctuary Console".to_string()
    } else {
        church_name.trim().to_string()
    };

    let clean_hostname = crate::network::sanitize_rfc1035_hostname(hostname);
    let advertised_hostname = format!("{}.local.", clean_hostname);

    let mut properties: HashMap<String, String> = HashMap::new();
    properties.insert("id".to_string(), instance_id.to_string());
    properties.insert("name".to_string(), instance_name.clone());
    properties.insert("version".to_string(), env!("CARGO_PKG_VERSION").to_string());
    properties.insert("hostname".to_string(), clean_hostname.clone());
    properties.insert("option12".to_string(), clean_hostname.clone());
    if let Some(h_port) = https_port {
        properties.insert("https_port".to_string(), h_port.to_string());
        properties.insert("tls".to_string(), "1".to_string());
    }

    // Empty string for the IP lets mdns-sd auto-detect this host's local
    // network interfaces rather than hardcoding one, since a server may be
    // reachable over Ethernet, WiFi, or both.
    let service = ServiceInfo::new(
        SERVICE_TYPE,
        &instance_name,
        &advertised_hostname,
        "",
        port,
        Some(properties),
    )
    .map_err(|e| format!("failed to build mDNS service info: {e}"))?
    .enable_addr_auto();

    daemon
        .register(service)
        .map_err(|e| format!("failed to register mDNS service: {e}"))?;

    tracing::info!(
        "mDNS broadcasting as '{}' ({}) on {} port {}",
        instance_name,
        advertised_hostname,
        SERVICE_TYPE,
        port
    );

    Ok(daemon)
}
