//! Hostname sanitization/conflict probing (RFC 1035) and the small set of
//! local-IP/QR utilities that existed alongside it in the original
//! `network/mod.rs` before that file was split by domain.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostnameCheckResult {
    pub hostname: String,
    pub sanitized_hostname: String,
    pub conflict_detected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflicting_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_hostname: Option<String>,
    pub message: String,
}

/// Sanitizes a hostname according to RFC 1035:
/// - Lowercase ASCII letters, numbers, and hyphens only.
/// - Cannot start or end with a hyphen.
/// - Length between 1 and 63 characters.
pub fn sanitize_rfc1035_hostname(name: &str) -> String {
    let trimmed = name.trim().to_ascii_lowercase();
    let mut clean = String::with_capacity(trimmed.len());
    let mut prev_hyphen = false;

    for c in trimmed.chars() {
        if c.is_ascii_alphanumeric() {
            clean.push(c);
            prev_hyphen = false;
        } else if (c == '-' || c == ' ' || c == '_') && !clean.is_empty() && !prev_hyphen {
            clean.push('-');
            prev_hyphen = true;
        }
    }

    // Strip trailing hyphen
    while clean.ends_with('-') {
        clean.pop();
    }

    // Clamp length to 63 chars
    if clean.len() > 63 {
        clean.truncate(63);
        while clean.ends_with('-') {
            clean.pop();
        }
    }

    if clean.is_empty() {
        "opensanctuary".to_string()
    } else {
        clean
    }
}

/// Probes the local network to see if `hostname` is already used by a foreign device.
/// Compares any resolved IP against `self_ips` to avoid false-positive self-collisions.
pub fn check_hostname_conflict(hostname: &str, self_ips: &[String]) -> HostnameCheckResult {
    use std::net::ToSocketAddrs;

    let sanitized = sanitize_rfc1035_hostname(hostname);
    let mut resolved_ips: Vec<String> = Vec::new();

    // 1. Probe mDNS (.local)
    let mdns_target = format!("{}.local:80", sanitized);
    if let Ok(iter) = mdns_target.to_socket_addrs() {
        for addr in iter {
            resolved_ips.push(addr.ip().to_string());
        }
    }

    // 2. Probe standard DNS / NetBIOS hostname
    let dns_target = format!("{}:80", sanitized);
    if let Ok(iter) = dns_target.to_socket_addrs() {
        for addr in iter {
            resolved_ips.push(addr.ip().to_string());
        }
    }

    // Check if any resolved IP belongs to a foreign machine
    let mut conflict_ip = None;
    for ip in &resolved_ips {
        if !self_ips.contains(ip)
            && ip != "127.0.0.1"
            && ip != "::1"
            && ip != "0.0.0.0"
            && ip != "::"
            && !ip.starts_with("fe80:")
        {
            conflict_ip = Some(ip.clone());
            break;
        }
    }

    if let Some(foreign_ip) = conflict_ip {
        let suggested = format!("{}-2", sanitized);
        HostnameCheckResult {
            hostname: hostname.to_string(),
            sanitized_hostname: sanitized.clone(),
            conflict_detected: true,
            conflicting_ip: Some(foreign_ip.clone()),
            suggested_hostname: Some(suggested.clone()),
            message: format!(
                "Conflict detected: Hostname '{}' is already in use by {} on this network. Recommended: '{}'.",
                sanitized, foreign_ip, suggested
            ),
        }
    } else {
        HostnameCheckResult {
            hostname: hostname.to_string(),
            sanitized_hostname: sanitized.clone(),
            conflict_detected: false,
            conflicting_ip: None,
            suggested_hostname: None,
            message: format!("Hostname '{}' is available on this network.", sanitized),
        }
    }
}

/// Returns all IPv4 and IPv6 addresses across all interfaces for conflict detection self-exclusion.
/// The single LAN-routable IP this machine would use to reach the outside
/// world -- found via the classic "connect a UDP socket, never actually
/// sends a packet, just makes the OS pick a route" trick, so it works
/// without touching the network. Used wherever one sensible address needs
/// to be shown/advertised (e.g. `/api/network/info`'s `lan_ip`, and the
/// startup banner's remote-console URL in `src/main.rs`) -- unlike
/// `get_all_local_ips` below, which deliberately returns every bindable
/// address for hostname-conflict checking, not "the" address to hand out.
pub fn get_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    let ip = addr.ip().to_string();
    if ip == "0.0.0.0" {
        None
    } else {
        Some(ip)
    }
}

/// Renders `data` as a QR code directly to the terminal using half-block
/// Unicode characters (▀▄█, two QR module-rows packed per terminal line --
/// the same convention tools like `qrencode -t ANSIUTF8` use), with a
/// quiet-zone margin so a phone or laptop camera can actually scan it. Used
/// by the startup banner (`src/main.rs`) to print the remote-console URL
/// (docs/CLIENT_PAIRING.md "Remote console access") as something scannable,
/// not just readable/copyable text.
pub fn render_terminal_qr(data: &str) -> Vec<String> {
    let code = match qrcode::QrCode::new(data.as_bytes()) {
        Ok(c) => c,
        Err(_) => return vec!["(QR code generation failed)".to_string()],
    };
    let width = code.width();
    let colors = code.to_colors();
    let is_dark = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x as usize >= width || y as usize >= width {
            false // quiet zone is always light
        } else {
            colors[y as usize * width + x as usize] == qrcode::Color::Dark
        }
    };

    const MARGIN: i32 = 2;
    let size = width as i32;
    let mut lines = Vec::new();
    let mut y = -MARGIN;
    while y < size + MARGIN {
        let mut line = String::new();
        for x in -MARGIN..size + MARGIN {
            let top = is_dark(x, y);
            let bottom = is_dark(x, y + 1);
            line.push(match (top, bottom) {
                (true, true) => '\u{2588}',  // █ full block
                (true, false) => '\u{2580}', // ▀ upper half block
                (false, true) => '\u{2584}', // ▄ lower half block
                (false, false) => ' ',
            });
        }
        lines.push(line);
        y += 2;
    }
    lines
}

/// Resolves the "real" client IP for a request, honoring `X-Forwarded-For`
/// only when the immediate TCP connection is itself loopback -- i.e. this
/// request genuinely arrived via something running on this same machine (a
/// local reverse proxy like Caddy), not spoofed by an arbitrary network
/// client setting its own header. A request whose immediate peer is *not*
/// loopback gets no such override, so nobody off-machine can bypass a
/// loopback check just by adding this header themselves.
///
/// A direct connection with no proxy in front behaves exactly as before:
/// `forwarded_for` will be `None` in that case, so this just returns
/// `connect_ip` unchanged.
///
/// See docs/TUNNELS.md "A real gap this setup exposes" for why this exists:
/// without it, a reverse proxy on the same machine (e.g. Caddy fronting
/// OS-Next with a trusted public HTTPS domain) makes every real visitor
/// look like a loopback caller to the app, since that connection genuinely
/// is loopback from the app's listener's point of view -- Caddy made it.
pub fn resolve_real_client_ip(connect_ip: std::net::IpAddr, forwarded_for: Option<&str>) -> std::net::IpAddr {
    if connect_ip.is_loopback() {
        if let Some(header) = forwarded_for {
            // X-Forwarded-For is a comma-separated hop chain,
            // "client, proxy1, proxy2, ..."; the first entry is the
            // original client as seen by the nearest (here, only) proxy.
            if let Some(first) = header.split(',').next() {
                if let Ok(ip) = first.trim().parse::<std::net::IpAddr>() {
                    return ip;
                }
            }
        }
    }
    connect_ip
}

pub fn get_all_local_ips() -> Vec<String> {
    let mut ips = vec![
        "127.0.0.1".to_string(),
        "::1".to_string(),
        "0.0.0.0".to_string(),
        "::".to_string(),
    ];
    if let Ok(ifaddrs) = get_if_addrs::get_if_addrs() {
        for iface in ifaddrs {
            ips.push(iface.ip().to_string());
        }
    }
    ips
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_rfc1035_hostname() {
        assert_eq!(sanitize_rfc1035_hostname("opensanctuary"), "opensanctuary");
        assert_eq!(sanitize_rfc1035_hostname("My Church Sanctuary"), "my-church-sanctuary");
        assert_eq!(sanitize_rfc1035_hostname("---church---123---"), "church-123");
        assert_eq!(sanitize_rfc1035_hostname("worship_room_alpha"), "worship-room-alpha");
        assert_eq!(sanitize_rfc1035_hostname(""), "opensanctuary");

        let long_name = "a".repeat(100);
        let sanitized = sanitize_rfc1035_hostname(&long_name);
        assert_eq!(sanitized.len(), 63);
    }

    #[test]
    fn test_resolve_real_client_ip_trusts_forwarded_header_only_from_loopback() {
        use std::net::IpAddr;
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        let lan_ip: IpAddr = "192.168.1.50".parse().unwrap();

        // No header at all -- a direct connection with no proxy in front,
        // today's existing behavior, completely unaffected.
        assert_eq!(resolve_real_client_ip(loopback, None), loopback);
        assert_eq!(resolve_real_client_ip(lan_ip, None), lan_ip);

        // The TCP peer IS loopback (a local reverse proxy like Caddy) and it
        // reports a real, non-loopback original client -- that client is
        // correctly NOT treated as loopback.
        assert_eq!(resolve_real_client_ip(loopback, Some("203.0.113.7")), "203.0.113.7".parse::<IpAddr>().unwrap());

        // The TCP peer IS loopback and the header itself says loopback too
        // (a browser tab hitting the proxy from the same machine) -- still
        // loopback, as expected.
        assert_eq!(resolve_real_client_ip(loopback, Some("127.0.0.1")), loopback);

        // Multi-hop header: first entry is the original client.
        assert_eq!(resolve_real_client_ip(loopback, Some("203.0.113.7, 10.0.0.1")), "203.0.113.7".parse::<IpAddr>().unwrap());

        // The critical negative case: the TCP peer is NOT loopback (a real
        // network caller), so its own forged `X-Forwarded-For: 127.0.0.1`
        // must NOT grant it loopback trust -- otherwise anyone on the LAN
        // could just add this header to bypass the loopback check entirely.
        let spoofed = resolve_real_client_ip(lan_ip, Some("127.0.0.1"));
        assert_eq!(spoofed, lan_ip, "a non-loopback TCP peer must not gain loopback trust via a forged header");
        assert!(!spoofed.is_loopback());

        // Garbage header value -- falls back to the real TCP peer rather
        // than panicking or silently trusting nonsense.
        assert_eq!(resolve_real_client_ip(loopback, Some("not-an-ip")), loopback);
    }

    #[test]
    fn test_check_hostname_conflict_ignores_self_ip() {
        // If an IP matches our self IPs, it shouldn't trigger a conflict
        let self_ips = vec!["127.0.0.1".to_string(), "192.168.1.250".to_string()];
        let res = check_hostname_conflict("localhost", &self_ips);
        assert!(!res.conflict_detected);
    }
}
