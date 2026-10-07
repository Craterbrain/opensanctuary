//! Network interface enumeration and cross-platform hardware abstraction.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
use super::linux;
#[cfg(target_os = "windows")]
use super::windows;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_network_interfaces_returns_unique_sorted_entries() {
        // A real discovery pass against whatever interfaces this machine
        // actually has -- can't assert exact names/types (that's the
        // platform-specific modules' own job, see linux.rs's tests), but the
        // enumeration/dedup/sort contract in get_network_interfaces itself
        // should hold regardless of what hardware is present.
        let interfaces = get_network_interfaces(None);

        let mut seen = std::collections::HashSet::new();
        for iface in &interfaces {
            assert!(seen.insert(iface.name.clone()), "duplicate interface name {:?} in discovery result", iface.name);
        }

        // Loopback (if present, which it always is on any machine capable of
        // running this test) sorts last, matching the doc comment's "loopback
        // last" ordering guarantee that VirtualAdapterManager and the
        // network settings UI both rely on.
        if let Some(loopback_pos) = interfaces.iter().position(|i| i.is_loopback) {
            assert_eq!(loopback_pos, interfaces.len() - 1, "loopback interface must sort last");
        }

        // discover_all() is just a convenience wrapper -- confirm it actually
        // delegates rather than silently returning something else.
        assert_eq!(NetworkInterfaceInfo::discover_all().len(), interfaces.len());
    }

    #[test]
    fn test_classify_interface_hardware_unknown_for_fake_name() {
        // A name that matches no real platform classification rule falls
        // back to Unknown rather than panicking or guessing.
        assert_eq!(classify_interface_hardware("os-next-test-does-not-exist-0"), NetworkInterfaceType::Unknown);
    }
}
