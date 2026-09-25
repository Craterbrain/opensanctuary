//! Linux-specific network interface classification, hardware inspection via sysfs,
//! virtual adapter management (macvlan mode bridge for Ethernet, ipvlan mode l2 for Wi-Fi),
//! and elevated command execution via Polkit (`pkexec`).

use std::path::Path;
use std::process::Command;
use super::{sanitize_rfc1035_hostname, get_network_interfaces, NetworkInterfaceType, VirtualAdapterResult};

/// Default name for the Linux dedicated virtual interface
pub const ADAPTER_NAME: &'static str = "os-macvlan0";

/// Classifies interface type using Linux sysfs and standard device naming conventions.
pub fn classify_linux_interface(name: &str) -> NetworkInterfaceType {
    // Linux /sys/class/net/<name>/wireless or phy80211 indicates Wi-Fi
    let wireless_path = format!("/sys/class/net/{}/wireless", name);
    let phy80211_path = format!("/sys/class/net/{}/phy80211", name);
    if Path::new(&wireless_path).exists() || Path::new(&phy80211_path).exists() || name.starts_with("wl") {
        return NetworkInterfaceType::Wireless;
    }
    if name.starts_with("en") || name.starts_with("eth") {
        return NetworkInterfaceType::Ethernet;
    }
    NetworkInterfaceType::Unknown
}

/// Reads hardware MAC address from Linux sysfs (`/sys/class/net/<name>/address`).
pub fn read_linux_interface_mac(name: &str) -> Option<String> {
    let address_path = format!("/sys/class/net/{}/address", name);
    if let Ok(addr) = std::fs::read_to_string(address_path) {
        let trimmed = addr.trim().to_string();
        if !trimmed.is_empty() && trimmed != "00:00:00:00:00:00" {
            return Some(trimmed);
        }
    }
    None
}

/// Reads interface operational link state from Linux sysfs (`/sys/class/net/<name>/operstate`).
pub fn read_linux_interface_is_up(name: &str) -> bool {
    let operstate_path = format!("/sys/class/net/{}/operstate", name);
    if let Ok(state) = std::fs::read_to_string(operstate_path) {
        let s = state.trim().to_lowercase();
        return s == "up" || s == "unknown";
    }
    true
}

/// Determines whether an interface is virtual via sysfs (`/sys/devices/virtual/net/<name>`).
pub fn is_linux_interface_virtual(name: &str) -> bool {
    name.starts_with("os-macvlan")
        || name.starts_with("os-vlan")
        || Path::new(&format!("/sys/devices/virtual/net/{}", name)).exists()
}

/// Validates that an interface name conforms to Linux kernel naming rules (<= 15 chars, alphanumeric + hyphens/underscores/dots).
pub fn is_valid_interface_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 15
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Builds an atomic script combining teardown, link provisioning, up state, and DHCP lease request in a single elevated subshell.
pub fn build_atomic_enable_script(parent_iface: &str, is_wireless: bool, vlan_name: &str, clean_hostname: &str) -> String {
    let vlan_type = if is_wireless {
        "ipvlan mode l2"
    } else {
        "macvlan mode bridge"
    };
    format!(
        "ip link delete {vlan_name} 2>/dev/null || true; \
         ip link add {vlan_name} link {parent_iface} type {vlan_type} && \
         ip link set {vlan_name} up && \
         (dhclient -H {clean_hostname} {vlan_name} 2>/dev/null || dhcpcd -h {clean_hostname} {vlan_name} 2>/dev/null || true)"
    )
}

/// Builds the shell script to create and bring up a dedicated macvlan or ipvlan link.
pub fn build_create_adapter_script(parent_iface: &str, is_wireless: bool, vlan_name: &str) -> String {
    let vlan_type = if is_wireless {
        "ipvlan mode l2"
    } else {
        "macvlan mode bridge"
    };
    format!(
        "ip link add {} link {} type {} && ip link set {} up",
        vlan_name, parent_iface, vlan_type, vlan_name
    )
}

/// Builds the shell script to delete a virtual adapter.
pub fn build_delete_adapter_script(vlan_name: &str) -> String {
    format!("ip link delete {} 2>/dev/null || true", vlan_name)
}

/// Builds the shell script to request a DHCP lease with Option 12.
pub fn build_dhcp_request_script(vlan_name: &str, clean_hostname: &str) -> String {
    format!(
        "dhclient -H {} {} 2>/dev/null || dhcpcd -h {} {} 2>/dev/null || true",
        clean_hostname, vlan_name, clean_hostname, vlan_name
    )
}

/// Provisions macvlan (Ethernet) or ipvlan L2 (Wi-Fi) using `pkexec` (Polkit).
pub fn enable_dedicated_adapter(
    parent_iface: &str,
    hostname: &str,
) -> Result<VirtualAdapterResult, String> {
    if !is_valid_interface_name(parent_iface) {
        return Err(format!("Invalid interface name: '{parent_iface}'"));
    }

    let is_wireless = classify_linux_interface(parent_iface) == NetworkInterfaceType::Wireless;
    let vlan_name = ADAPTER_NAME;
    let clean_hostname = sanitize_rfc1035_hostname(hostname);

    // Single atomic elevated command execution via Polkit
    let script = build_atomic_enable_script(parent_iface, is_wireless, vlan_name, &clean_hostname);

    let output = Command::new("pkexec")
        .arg("sh")
        .arg("-c")
        .arg(&script)
        .output()
        .map_err(|e| format!("Failed to invoke pkexec: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Polkit elevation failed: {err}"));
    }

    // Brief tick to allow kernel and DHCP lease assignment
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // Retrieve acquired IP and MAC
    let interfaces = get_network_interfaces(None);
    let created = interfaces.iter().find(|i| i.name == vlan_name);

    let (ip, mac) = if let Some(info) = created {
        (info.ipv4.clone(), info.mac_address.clone())
    } else {
        (None, None)
    };

    Ok(VirtualAdapterResult {
        success: true,
        adapter_name: vlan_name.to_string(),
        parent_interface: parent_iface.to_string(),
        interface_type: if is_wireless { "ipvlan_l2".to_string() } else { "macvlan_bridge".to_string() },
        mac_address: mac,
        dedicated_ip: ip,
        message: format!(
            "Dedicated adapter '{}' successfully created on {} ({}) with Option 12 '{}'",
            vlan_name, parent_iface, if is_wireless { "ipvlan L2" } else { "macvlan bridge" }, clean_hostname
        ),
    })
}

/// Removes virtual adapter cleanly using `pkexec`.
pub fn disable_dedicated_adapter() -> Result<(), String> {
    let script = build_delete_adapter_script(ADAPTER_NAME);
    let _ = Command::new("pkexec").arg("sh").arg("-c").arg(&script).status();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linux_script_generation() {
        let eth_script = build_create_adapter_script("eth0", false, "os-macvlan0");
        assert_eq!(
            eth_script,
            "ip link add os-macvlan0 link eth0 type macvlan mode bridge && ip link set os-macvlan0 up"
        );

        let wlan_script = build_create_adapter_script("wlan0", true, "os-macvlan0");
        assert_eq!(
            wlan_script,
            "ip link add os-macvlan0 link wlan0 type ipvlan mode l2 && ip link set os-macvlan0 up"
        );

        let del_script = build_delete_adapter_script("os-macvlan0");
        assert_eq!(del_script, "ip link delete os-macvlan0 2>/dev/null || true");

        let dhcp_script = build_dhcp_request_script("os-macvlan0", "opensanctuary");
        assert!(dhcp_script.contains("dhclient -H opensanctuary os-macvlan0"));
        assert!(dhcp_script.contains("dhcpcd -h opensanctuary os-macvlan0"));

        let atomic_script = build_atomic_enable_script("eth0", false, "os-macvlan0", "opensanctuary");
        assert!(atomic_script.contains("ip link delete os-macvlan0 2>/dev/null || true;"));
        assert!(atomic_script.contains("ip link add os-macvlan0 link eth0 type macvlan mode bridge"));
        assert!(atomic_script.contains("ip link set os-macvlan0 up"));
        assert!(atomic_script.contains("dhclient -H opensanctuary os-macvlan0"));
    }

    #[test]
    fn test_is_valid_interface_name() {
        assert!(is_valid_interface_name("eth0"));
        assert!(is_valid_interface_name("eno1"));
        assert!(is_valid_interface_name("wlan0.100"));
        assert!(is_valid_interface_name("os-macvlan0"));
        assert!(!is_valid_interface_name(""));
        assert!(!is_valid_interface_name("eth0; rm -rf /"));
        assert!(!is_valid_interface_name("eth 0"));
        assert!(!is_valid_interface_name("a".repeat(16).as_str())); // Exceeds IFNAMSIZ (15 chars)
    }

    #[test]
    fn test_classify_linux_interface_by_prefix() {
        assert_eq!(classify_linux_interface("eth0"), NetworkInterfaceType::Ethernet);
        assert_eq!(classify_linux_interface("eno1"), NetworkInterfaceType::Ethernet);
        assert_eq!(classify_linux_interface("enp3s0"), NetworkInterfaceType::Ethernet);
        assert_eq!(classify_linux_interface("wlan0"), NetworkInterfaceType::Wireless);
        assert_eq!(classify_linux_interface("wlp2s0"), NetworkInterfaceType::Wireless);
        assert_eq!(classify_linux_interface("docker0"), NetworkInterfaceType::Unknown);
    }
}
