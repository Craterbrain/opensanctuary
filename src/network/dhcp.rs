//! DHCP Option 12 packet building, parsing & broadcasting (RFC 2131/2132).

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use serde::{Deserialize, Serialize};

use super::hostname::sanitize_rfc1035_hostname;
use super::interfaces::NetworkInterfaceInfo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastResult {
    pub interface_name: String,
    pub target_ip: String,
    pub port: u16,
    pub success: bool,
    pub message: String,
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
