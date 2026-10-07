//! Dedicated virtual MAC adapter management (cross-platform facade).

use serde::{Deserialize, Serialize};

use super::interfaces::{get_network_interfaces, NetworkInterfaceInfo};

#[cfg(target_os = "linux")]
use super::linux;
#[cfg(target_os = "windows")]
use super::windows;

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
