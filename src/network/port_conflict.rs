//! TCP port conflict probing.

use std::net::SocketAddr;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortCheckResult {
    pub port: u16,
    pub available: bool,
    pub in_use_by_current: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_alternative: Option<u16>,
}

/// Tests TCP port availability on `0.0.0.0:port`.
/// Excludes `current_active_port` so the active process doesn't flag itself as a collision.
pub fn check_port_conflict(port: u16, current_active_port: Option<u16>) -> PortCheckResult {
    if port == 0 {
        return PortCheckResult {
            port: 0,
            available: false,
            in_use_by_current: false,
            message: "Port 0 is reserved and cannot be assigned.".to_string(),
            suggested_alternative: Some(8080),
        };
    }

    if Some(port) == current_active_port {
        return PortCheckResult {
            port,
            available: true,
            in_use_by_current: true,
            message: format!("Port {} is currently active and assigned to OpenSanctuary.", port),
            suggested_alternative: None,
        };
    }

    match std::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))) {
        Ok(_) => PortCheckResult {
            port,
            available: true,
            in_use_by_current: false,
            message: format!("Port {} is available.", port),
            suggested_alternative: None,
        },
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            // Find next open port between port+1 and port+50
            let suggested = (port + 1..=port + 50)
                .find(|&p| std::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], p))).is_ok());
            PortCheckResult {
                port,
                available: false,
                in_use_by_current: false,
                message: format!("Port {} is already in use by another process.", port),
                suggested_alternative: suggested,
            }
        }
        Err(e) => PortCheckResult {
            port,
            available: false,
            in_use_by_current: false,
            message: format!("Cannot bind to port {}: {}", port, e),
            suggested_alternative: None,
        },
    }
}

/// Tests TCP port availability across multiple active ports (e.g. HTTP 8080 and HTTPS 8443).
pub fn check_port_conflict_multi(port: u16, active_ports: &[u16]) -> PortCheckResult {
    if active_ports.contains(&port) {
        return PortCheckResult {
            port,
            available: true,
            in_use_by_current: true,
            message: format!("Port {} is currently active and assigned to OpenSanctuary.", port),
            suggested_alternative: None,
        };
    }
    check_port_conflict(port, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_port_conflict_ignores_current_server_port() {
        let current_port = 8080;
        let res = check_port_conflict(8080, Some(current_port));
        assert!(res.available);
        assert!(res.in_use_by_current);
        assert!(res.suggested_alternative.is_none());
    }

    #[test]
    fn test_check_port_conflict_detects_occupied_port() {
        // Bind an ephemeral listener to occupy a port
        let listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let occupied_port = listener.local_addr().unwrap().port();

        let res = check_port_conflict(occupied_port, None);
        assert!(!res.available);
        assert!(!res.in_use_by_current);
        assert!(res.suggested_alternative.is_some());
        drop(listener);
    }

    #[test]
    fn test_check_port_conflict_rejects_port_0() {
        let res = check_port_conflict(0, None);
        assert!(!res.available);
        assert_eq!(res.suggested_alternative, Some(8080));
    }
}
