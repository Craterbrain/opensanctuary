//! Network configuration, multi-interface management, dedicated MAC adapter control,
//! conflict detection, and DHCP Option 12 broadcasting for OpenSanctuary.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;
pub mod tls;
pub use tls::*;

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkInterfaceType {
    Ethernet,
    Wireless,
    Loopback,
    Virtual,
    Unknown,
}

impl Default for NetworkInterfaceType {
    fn default() -> Self {
        NetworkInterfaceType::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterfaceInfo {
    pub name: String,
    pub display_name: String,
    pub interface_type: NetworkInterfaceType,
    pub ipv4: Option<String>,
    pub netmask: Option<String>,
    pub broadcast: Option<String>,
    pub mac_address: Option<String>,
    pub is_loopback: bool,
    pub is_virtual: bool,
    pub is_up: bool,
    pub enabled: bool,
}

impl NetworkInterfaceInfo {
    pub fn discover_all() -> Vec<Self> {
        get_network_interfaces(None)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortCheckResult {
    pub port: u16,
    pub available: bool,
    pub in_use_by_current: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_alternative: Option<u16>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualAdapterResult {
    pub success: bool,
    pub adapter_name: String,
    pub parent_interface: String,
    pub interface_type: String, // "macvlan" or "ipvlan" or "vm_adapter"
    pub mac_address: Option<String>,
    pub dedicated_ip: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastResult {
    pub interface_name: String,
    pub target_ip: String,
    pub port: u16,
    pub success: bool,
    pub message: String,
}

// ============================================================================
// 1. Hostname Sanitization & Conflict Probing (RFC 1035 Compliance)
// ============================================================================

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

// ============================================================================
// 2. Port Conflict Probing
// ============================================================================

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

// ============================================================================
// 3. Network Interface Enumeration & Hardware Abstraction
// ============================================================================

/// Reads all host network interfaces using `get_if_addrs` and platform hardware inspection.
pub fn get_network_interfaces(enabled_filter: Option<&[String]>) -> Vec<NetworkInterfaceInfo> {
    let raw_interfaces = get_if_addrs::get_if_addrs().unwrap_or_default();
    let mut interfaces_map: HashMap<String, NetworkInterfaceInfo> = HashMap::new();

    for iface in raw_interfaces {
        let name = iface.name.clone();
        let is_loopback = iface.is_loopback();
        let is_virtual = is_interface_virtual(&name);

        let iface_type = if is_loopback {
            NetworkInterfaceType::Loopback
        } else if is_virtual {
            NetworkInterfaceType::Virtual
        } else {
            classify_interface_hardware(&name)
        };

        let (ipv4, netmask, broadcast) = match iface.addr {
            get_if_addrs::IfAddr::V4(v4) => (
                Some(v4.ip.to_string()),
                Some(v4.netmask.to_string()),
                v4.broadcast.map(|b| b.to_string()),
            ),
            get_if_addrs::IfAddr::V6(_) => continue, // Focus on IPv4 for LAN presentation discovery
        };

        let mac = read_interface_mac(&name);
        let is_up = read_interface_is_up(&name);

        let display_name = match iface_type {
            NetworkInterfaceType::Ethernet => format!("Ethernet ({})", name),
            NetworkInterfaceType::Wireless => format!("Wi-Fi ({})", name),
            NetworkInterfaceType::Virtual => format!("Dedicated Virtual ({})", name),
            NetworkInterfaceType::Loopback => format!("Local Loopback ({})", name),
            NetworkInterfaceType::Unknown => name.clone(),
        };

        let is_enabled = if let Some(filter) = enabled_filter {
            filter.contains(&name)
        } else {
            // By default, physical interfaces and active virtual links are enabled; loopback disabled
            !is_loopback && is_up
        };

        let entry = interfaces_map.entry(name.clone()).or_insert_with(|| NetworkInterfaceInfo {
            name: name.clone(),
            display_name,
            interface_type: iface_type,
            ipv4: None,
            netmask: None,
            broadcast: None,
            mac_address: mac,
            is_loopback,
            is_virtual,
            is_up,
            enabled: is_enabled,
        });

        if entry.ipv4.is_none() && ipv4.is_some() {
            entry.ipv4 = ipv4;
            entry.netmask = netmask;
            entry.broadcast = broadcast;
        }
    }

    let mut list: Vec<NetworkInterfaceInfo> = interfaces_map.into_values().collect();
    // Sort physical Ethernet/Wi-Fi and virtual first, loopback last
    list.sort_by_key(|i| (i.is_loopback, !i.is_up, i.name.clone()));
    list
}

/// Checks if an interface is virtual across platforms.
pub fn is_interface_virtual(name: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::is_linux_interface_virtual(name)
    }
    #[cfg(target_os = "windows")]
    {
        windows::is_windows_interface_virtual(name)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        name.starts_with("os-macvlan") || name.starts_with("os-vlan") || name.contains("virtual")
    }
}

/// Classifies interface hardware type across platforms.
pub fn classify_interface_hardware(name: &str) -> NetworkInterfaceType {
    #[cfg(target_os = "linux")]
    {
        linux::classify_linux_interface(name)
    }
    #[cfg(target_os = "windows")]
    {
        windows::classify_windows_interface(name)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = name;
        NetworkInterfaceType::Unknown
    }
}

/// Reads interface MAC address across platforms.
pub fn read_interface_mac(name: &str) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        linux::read_linux_interface_mac(name)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = name;
        None
    }
}

/// Reads operational link state across platforms.
pub fn read_interface_is_up(name: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::read_linux_interface_is_up(name)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = name;
        true
    }
}

// ============================================================================
// 4. Virtual Adapter Management (Cross-Platform Facade)
// ============================================================================

pub struct VirtualAdapterManager;

impl VirtualAdapterManager {
    /// Dedicated virtual adapter name for Linux
    #[cfg(target_os = "linux")]
    pub const ADAPTER_NAME: &'static str = linux::ADAPTER_NAME;
    #[cfg(not(target_os = "linux"))]
    pub const ADAPTER_NAME: &'static str = "os-macvlan0";

    /// Dedicated virtual adapter name for Windows
    #[cfg(target_os = "windows")]
    pub const WINDOWS_ADAPTER_NAME: &'static str = windows::ADAPTER_NAME;
    #[cfg(not(target_os = "windows"))]
    pub const WINDOWS_ADAPTER_NAME: &'static str = "OpenSanctuaryAdapter";

    /// Checks if a dedicated virtual adapter currently exists on the system.
    pub fn is_dedicated_adapter_active() -> bool {
        let interfaces = get_network_interfaces(None);
        interfaces.iter().any(|i| {
            (i.name == Self::ADAPTER_NAME
                || i.name == Self::WINDOWS_ADAPTER_NAME
                || i.name.contains("OpenSanctuary"))
                && i.is_up
                && i.ipv4.is_some()
        })
    }

    /// Finds active dedicated adapter info if currently active and possessing an IPv4 address.
    pub fn get_dedicated_adapter_info() -> Option<NetworkInterfaceInfo> {
        let interfaces = get_network_interfaces(None);
        interfaces.into_iter().find(|i| {
            (i.name == Self::ADAPTER_NAME
                || i.name == Self::WINDOWS_ADAPTER_NAME
                || i.name.contains("OpenSanctuary"))
                && i.is_up
                && i.ipv4.is_some()
        })
    }

    /// Cross-platform dispatcher: Provisions a dedicated virtual MAC adapter.
    pub fn enable_dedicated_adapter(
        parent_iface: &str,
        hostname: &str,
    ) -> Result<VirtualAdapterResult, String> {
        #[cfg(target_os = "linux")]
        {
            linux::enable_dedicated_adapter(parent_iface, hostname)
        }
        #[cfg(target_os = "windows")]
        {
            windows::enable_dedicated_adapter(parent_iface, hostname)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = (parent_iface, hostname);
            Err("Dedicated virtual MAC adapter is supported on Linux (macvlan/ipvlan) and Windows (VMNetworkAdapter).".to_string())
        }
    }

    /// Cross-platform dispatcher: Removes the dedicated virtual adapter.
    pub fn disable_dedicated_adapter() -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            linux::disable_dedicated_adapter()
        }
        #[cfg(target_os = "windows")]
        {
            windows::disable_dedicated_adapter()
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Ok(())
        }
    }
}

// ============================================================================
// 5. DHCP Option 12 Packet Building, Parsing & Broadcasting (RFC 2131/2132)
// ============================================================================

pub struct DhcpOption12Client;

impl DhcpOption12Client {
    pub const DHCP_MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99]; // 0x63, 0x82, 0x53, 0x63
    pub const OPTION_HOST_NAME: u8 = 12;
    pub const OPTION_DHCP_MESSAGE_TYPE: u8 = 53;
    pub const OPTION_PARAM_REQUEST_LIST: u8 = 55;
    pub const OPTION_END: u8 = 255;
    pub const DHCP_MESSAGE_TYPE_INFORM: u8 = 8;

    /// Builds a compliant RFC 2131 BOOTREQUEST / DHCPINFORM packet containing Option 12.
    pub fn build_dhcp_inform_packet(
        xid: u32,
        client_ip: Ipv4Addr,
        client_mac: [u8; 6],
        hostname: &str,
    ) -> Vec<u8> {
        let clean_hostname = sanitize_rfc1035_hostname(hostname);
        let hostname_bytes = clean_hostname.as_bytes();

        let mut packet = Vec::with_capacity(300);

        // --- Fixed BOOTP Header (236 bytes) ---
        packet.push(1); // op: 1 = BOOTREQUEST
        packet.push(1); // htype: 1 = 10Mb Ethernet
        packet.push(6); // hlen: 6 = hardware address length (MAC)
        packet.push(0); // hops: 0

        // xid: 4 bytes (transaction ID)
        packet.extend_from_slice(&xid.to_be_bytes());

        // secs: 2 bytes (0)
        packet.extend_from_slice(&[0, 0]);

        // flags: 2 bytes (unicast = 0x0000)
        packet.extend_from_slice(&[0, 0]);

        // ciaddr: 4 bytes (client IP address)
        packet.extend_from_slice(&client_ip.octets());

        // yiaddr: 4 bytes (your/client IP - 0.0.0.0 for INFORM)
        packet.extend_from_slice(&[0, 0, 0, 0]);

        // siaddr: 4 bytes (server IP - 0.0.0.0)
        packet.extend_from_slice(&[0, 0, 0, 0]);

        // giaddr: 4 bytes (gateway IP - 0.0.0.0)
        packet.extend_from_slice(&[0, 0, 0, 0]);

        // chaddr: 16 bytes (client hardware address, padded with zeros)
        packet.extend_from_slice(&client_mac);
        packet.extend_from_slice(&[0u8; 10]);

        // sname: 64 bytes zeros (server host name)
        packet.extend_from_slice(&[0u8; 64]);

        // file: 128 bytes zeros (boot file name)
        packet.extend_from_slice(&[0u8; 128]);

        // --- DHCP Magic Cookie (4 bytes: 99, 130, 83, 99) ---
        packet.extend_from_slice(&Self::DHCP_MAGIC_COOKIE);

        // --- Option 53: DHCP Message Type = 8 (DHCPINFORM) ---
        packet.push(Self::OPTION_DHCP_MESSAGE_TYPE);
        packet.push(1);
        packet.push(Self::DHCP_MESSAGE_TYPE_INFORM);

        // --- Option 12: Host Name ---
        let host_len = (hostname_bytes.len().min(63)) as u8;
        packet.push(Self::OPTION_HOST_NAME);
        packet.push(host_len);
        packet.extend_from_slice(&hostname_bytes[..host_len as usize]);

        // --- Option 55: Parameter Request List (1=Subnet, 3=Router, 6=DNS, 12=Hostname, 15=Domain) ---
        packet.push(Self::OPTION_PARAM_REQUEST_LIST);
        packet.push(5);
        packet.extend_from_slice(&[1, 3, 6, 12, 15]);

        // --- Option 255: End ---
        packet.push(Self::OPTION_END);

        // RFC 1542 Section 2.1 & RFC 2131: Vendor area must be at least 64 octets,
        // yielding a minimum packet size of 300 octets. Many managed switches and routers drop undersized frames.
        while packet.len() < 300 {
            packet.push(0); // PAD byte
        }

        packet
    }

    /// Parses DHCP options from a raw packet buffer, extracting Option 12.
    pub fn parse_dhcp_options(data: &[u8]) -> HashMap<u8, Vec<u8>> {
        let mut options = HashMap::new();
        // Need at least fixed header (236 bytes) + magic cookie (4 bytes)
        if data.len() < 240 {
            return options;
        }

        if &data[236..240] != &Self::DHCP_MAGIC_COOKIE {
            return options;
        }

        let mut idx = 240;
        while idx < data.len() {
            let code = data[idx];
            idx += 1;
            if code == 0 {
                // Pad byte, continue
                continue;
            }
            if code == Self::OPTION_END {
                break;
            }
            if idx >= data.len() {
                break;
            }
            let len = data[idx] as usize;
            idx += 1;
            if idx + len <= data.len() {
                options.insert(code, data[idx..idx + len].to_vec());
                idx += len;
            } else {
                break;
            }
        }
        options
    }

    /// Broadcasts DHCP Option 12 inform packets across all enabled network interfaces.
    pub fn broadcast_option_12(
        hostname: &str,
        interfaces: &[NetworkInterfaceInfo],
    ) -> Vec<BroadcastResult> {
        let clean_hostname = sanitize_rfc1035_hostname(hostname);
        let mut results = Vec::new();

        let socket = match UdpSocket::bind("0.0.0.0:0") {
            Ok(s) => s,
            Err(e) => {
                results.push(BroadcastResult {
                    interface_name: "all".to_string(),
                    target_ip: "255.255.255.255".to_string(),
                    port: 67,
                    success: false,
                    message: format!("Failed to bind local UDP broadcast socket: {e}"),
                });
                return results;
            }
        };

        let _ = socket.set_broadcast(true);

        for iface in interfaces {
            if !iface.enabled || !iface.is_up {
                continue;
            }
            let ip: Ipv4Addr = match iface.ipv4.as_ref().and_then(|s| s.parse().ok()) {
                Some(addr) => addr,
                None => continue,
            };

            // Parse MAC or default
            let mut mac_bytes = [0u8; 6];
            if let Some(ref mac_str) = iface.mac_address {
                let parts: Vec<u8> = mac_str
                    .split(':')
                    .filter_map(|b| u8::from_str_radix(b, 16).ok())
                    .collect();
                if parts.len() == 6 {
                    mac_bytes.copy_from_slice(&parts);
                }
            }

            let xid = rand_xid();
            let packet = Self::build_dhcp_inform_packet(xid, ip, mac_bytes, &clean_hostname);

            // 1. Broadcast to general 255.255.255.255:67
            let dest_global: SocketAddr = "255.255.255.255:67".parse().unwrap();
            let res_global = socket.send_to(&packet, dest_global);
            let mut send_ok = res_global.is_ok();
            let mut target_used = dest_global.ip().to_string();

            // 2. Broadcast to subnet-directed broadcast if available (e.g. 192.168.1.255:67)
            if let Some(ref bcast_str) = iface.broadcast {
                if let Ok(dest_subnet) = format!("{}:67", bcast_str).parse::<SocketAddr>() {
                    if socket.send_to(&packet, dest_subnet).is_ok() {
                        send_ok = true;
                        if res_global.is_err() {
                            target_used = dest_subnet.ip().to_string();
                        }
                    }
                }
            }

            if send_ok {
                results.push(BroadcastResult {
                    interface_name: iface.name.clone(),
                    target_ip: target_used,
                    port: 67,
                    success: true,
                    message: format!(
                        "Broadcast DHCP Option 12 ('{}') sent on {} ({})",
                        clean_hostname, iface.name, ip
                    ),
                });
            } else {
                results.push(BroadcastResult {
                    interface_name: iface.name.clone(),
                    target_ip: dest_global.ip().to_string(),
                    port: 67,
                    success: false,
                    message: format!("Failed sending on {}: {:?}", iface.name, res_global.err()),
                });
            }
        }

        results
    }
}

fn rand_xid() -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    (now & 0xFFFFFFFF) as u32
}

// ============================================================================
// Unit Tests
// ============================================================================

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
    fn test_check_hostname_conflict_ignores_self_ip() {
        // If an IP matches our self IPs, it shouldn't trigger a conflict
        let self_ips = vec!["127.0.0.1".to_string(), "192.168.1.250".to_string()];
        let res = check_hostname_conflict("localhost", &self_ips);
        assert!(!res.conflict_detected);
    }

    #[test]
    fn test_check_port_conflict_rejects_port_0() {
        let res = check_port_conflict(0, None);
        assert!(!res.available);
        assert_eq!(res.suggested_alternative, Some(8080));
    }

    #[test]
    fn test_build_dhcp_inform_packet_contains_option_12() {
        let xid = 0x12345678;
        let ip: Ipv4Addr = "192.168.1.100".parse().unwrap();
        let mac = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        let hostname = "worship-pc";

        let packet = DhcpOption12Client::build_dhcp_inform_packet(xid, ip, mac, hostname);
        // Verify minimum BOOTP packet length per RFC 1542 / 2131
        assert!(packet.len() >= 300);

        // Verify BOOTP header fields
        assert_eq!(packet[0], 1); // BOOTREQUEST
        assert_eq!(packet[1], 1); // Ethernet
        assert_eq!(packet[2], 6); // MAC len
        assert_eq!(&packet[4..8], &xid.to_be_bytes());
        assert_eq!(&packet[12..16], &ip.octets());
        assert_eq!(&packet[28..34], &mac);

        // Verify Magic Cookie
        assert_eq!(&packet[236..240], &DhcpOption12Client::DHCP_MAGIC_COOKIE);

        // Verify parsed options
        let options = DhcpOption12Client::parse_dhcp_options(&packet);
        assert_eq!(
            options.get(&DhcpOption12Client::OPTION_DHCP_MESSAGE_TYPE),
            Some(&vec![DhcpOption12Client::DHCP_MESSAGE_TYPE_INFORM])
        );

        let opt12 = options.get(&DhcpOption12Client::OPTION_HOST_NAME).unwrap();
        assert_eq!(String::from_utf8_lossy(opt12), "worship-pc");
    }

    #[test]
    fn test_parse_dhcp_options_extracts_option_12() {
        let mut raw = vec![0u8; 236]; // Header
        raw.extend_from_slice(&DhcpOption12Client::DHCP_MAGIC_COOKIE);
        // Option 12: len 14: "opensanctuary"
        raw.push(12);
        raw.push(13);
        raw.extend_from_slice(b"opensanctuary");
        raw.push(255); // End

        let options = DhcpOption12Client::parse_dhcp_options(&raw);
        let opt12 = options.get(&12).unwrap();
        assert_eq!(String::from_utf8(opt12.clone()).unwrap(), "opensanctuary");
    }
}
