//! Windows-specific network interface classification, Hyper-V Virtual Switch & `VMNetworkAdapter`
//! management via elevated PowerShell (UAC), pre-flight Hyper-V feature inspection, and modular
//! fallback architecture.

use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::process::Command;
#[allow(unused_imports)]
use super::{sanitize_rfc1035_hostname, get_network_interfaces, NetworkInterfaceType, VirtualAdapterResult};

/// Default name for the Windows dedicated virtual network adapter
pub const ADAPTER_NAME: &'static str = "OpenSanctuaryAdapter";

/// Default name for the Windows Hyper-V Virtual Switch
pub const VIRTUAL_SWITCH_NAME: &'static str = "OpenSanctuarySwitch";

/// Available or planned virtual adapter backends for Windows
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsAdapterBackend {
    /// Native Hyper-V External Virtual Switch & VMNetworkAdapter (Layer 2 MAC + OS Winsock binding)
    HyperV,
    /// Npcap NDIS filter driver + smoltcp userspace TCP/IP stack (requires userspace reverse proxy)
    NpcapSmoltcp,
}

impl Default for WindowsAdapterBackend {
    fn default() -> Self {
        WindowsAdapterBackend::HyperV
    }
}

/// Classifies interface type using Windows adapter name conventions and aliases.
pub fn classify_windows_interface(name: &str) -> NetworkInterfaceType {
    let lower = name.to_lowercase();
    if lower.contains("wi-fi")
        || lower.contains("wireless")
        || lower.contains("wlan")
        || lower.contains("802.11")
    {
        return NetworkInterfaceType::Wireless;
    }
    if lower.contains("ethernet")
        || lower.contains("local area")
        || lower.contains("eth")
        || lower.contains("gigabit")
    {
        return NetworkInterfaceType::Ethernet;
    }
    NetworkInterfaceType::Unknown
}

/// Checks if an interface name corresponds to a Windows virtual or hypervisor adapter.
pub fn is_windows_interface_virtual(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("opensanctuary")
        || lower.contains("hyper-v")
        || lower.contains("vethernet")
        || lower.contains("virtual")
        || lower.contains("wsl")
        || lower.contains("npcap")
        || lower.contains("loopback")
}

use base64::prelude::*;

/// Encodes a PowerShell script into UTF-16LE Base64 for safe execution via `-EncodedCommand`.
pub fn encode_powershell_command(command: &str) -> String {
    let utf16_bytes: Vec<u8> = command
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    BASE64_STANDARD.encode(&utf16_bytes)
}

/// Builds the PowerShell script to create an External Virtual Switch and bind a dedicated VMNetworkAdapter.
pub fn build_enable_adapter_script(parent_iface: &str, adapter_name: &str, switch_name: &str) -> String {
    let safe_parent = parent_iface.replace('\'', "''");
    let safe_adapter = adapter_name.replace('\'', "''");
    let safe_switch = switch_name.replace('\'', "''");
    format!(
        "if (-not (Get-VMSwitch -Name '{safe_switch}' -ErrorAction SilentlyContinue)) {{ \
             New-VMSwitch -Name '{safe_switch}' -NetAdapterName '{safe_parent}' -AllowManagementOS $true \
         }}; \
         if (-not (Get-VMNetworkAdapter -ManagementOS -Name '{safe_adapter}' -ErrorAction SilentlyContinue)) {{ \
             Add-VMNetworkAdapter -ManagementOS -Name '{safe_adapter}' -SwitchName '{safe_switch}' \
         }}"
    )
}

/// Builds the PowerShell script to remove the dedicated VMNetworkAdapter.
pub fn build_disable_adapter_script(adapter_name: &str) -> String {
    let safe_adapter = adapter_name.replace('\'', "''");
    format!(
        "Remove-VMNetworkAdapter -ManagementOS -Name '{safe_adapter}' -ErrorAction SilentlyContinue"
    )
}

/// Wraps a PowerShell script command with UAC elevation using `-EncodedCommand` to prevent quoting or injection issues.
pub fn build_uac_powershell_wrapper(command: &str) -> String {
    let encoded = encode_powershell_command(command);
    format!(
        "Start-Process powershell -Verb RunAs -ArgumentList '-NoProfile', '-EncodedCommand', '{encoded}'"
    )
}

/// Pre-flight probe checking whether Hyper-V cmdlets and virtualization are available on this system.
pub fn check_hyperv_support() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "if (Get-Command Add-VMNetworkAdapter -ErrorAction SilentlyContinue) { Write-Output 'OK' }",
            ])
            .output()
            .map_err(|e| format!("Failed to query PowerShell for Hyper-V support: {e}"))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Ok(stdout.trim() == "OK");
        }
        Ok(false)
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(false)
    }
}

/// Provisions a dedicated VMNetworkAdapter on Windows using elevated PowerShell (UAC).
pub fn enable_dedicated_adapter(
    parent_iface: &str,
    hostname: &str,
) -> Result<VirtualAdapterResult, String> {
    #[cfg(target_os = "windows")]
    {
        // 1. Check Hyper-V feature availability before triggering UAC prompt
        match check_hyperv_support() {
            Ok(true) => {}
            _ => {
                return Err(
                    "Hyper-V is not installed or enabled on this system. \
                     A dedicated virtual MAC adapter requires the Windows Hyper-V feature. \
                     Please enable Hyper-V in 'Turn Windows features on or off' or run OpenSanctuary on an existing interface."
                        .to_string(),
                );
            }
        }

        let clean_hostname = sanitize_rfc1035_hostname(hostname);
        let inner_cmd = build_enable_adapter_script(parent_iface, ADAPTER_NAME, VIRTUAL_SWITCH_NAME);
        let uac_cmd = build_uac_powershell_wrapper(&inner_cmd);

        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", &uac_cmd])
            .output()
            .map_err(|e| format!("Failed to invoke PowerShell UAC: {e}"))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("UAC elevation failed: {err}"));
        }

        // Allow Windows time to initialize the virtual adapter and negotiate DHCP lease
        std::thread::sleep(std::time::Duration::from_millis(2000));

        let interfaces = get_network_interfaces(None);
        let created = interfaces
            .iter()
            .find(|i| i.name == ADAPTER_NAME || i.name.contains("OpenSanctuary"));

        let (ip, mac) = if let Some(info) = created {
            (info.ipv4.clone(), info.mac_address.clone())
        } else {
            (None, None)
        };

        Ok(VirtualAdapterResult {
            success: true,
            adapter_name: ADAPTER_NAME.to_string(),
            parent_interface: parent_iface.to_string(),
            interface_type: "vm_network_adapter".to_string(),
            mac_address: mac,
            dedicated_ip: ip,
            message: format!(
                "Windows Hyper-V virtual adapter '{}' requested on {} with Option 12 '{}'",
                ADAPTER_NAME, parent_iface, clean_hostname
            ),
        })
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (parent_iface, hostname);
        Err("Windows virtual adapter provisioning can only be executed on Windows systems.".to_string())
    }
}

/// Removes the dedicated VMNetworkAdapter on Windows using elevated PowerShell (UAC).
pub fn disable_dedicated_adapter() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let inner_cmd = build_disable_adapter_script(ADAPTER_NAME);
        let uac_cmd = build_uac_powershell_wrapper(&inner_cmd);
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", &uac_cmd])
            .status();
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the real `powershell.exe` round-trip on an actual Windows machine
    /// (not achievable under Wine). On this nested-virtualization test VM Hyper-V is
    /// not installed, so this is expected to return `Ok(false)`, not `Err`.
    #[test]
    fn test_check_hyperv_support_real_powershell_roundtrip() {
        let result = check_hyperv_support();
        println!("check_hyperv_support() => {:?}", result);
        assert!(result.is_ok(), "PowerShell invocation itself should succeed even if Hyper-V is absent");
    }

    #[test]
    fn test_windows_powershell_script_generation() {
        let enable_script = build_enable_adapter_script("Ethernet 1", "OpenSanctuaryAdapter", "OpenSanctuarySwitch");
        assert!(enable_script.contains("Get-VMSwitch -Name 'OpenSanctuarySwitch'"));
        assert!(enable_script.contains("New-VMSwitch -Name 'OpenSanctuarySwitch' -NetAdapterName 'Ethernet 1' -AllowManagementOS $true"));
        assert!(enable_script.contains("Add-VMNetworkAdapter -ManagementOS -Name 'OpenSanctuaryAdapter' -SwitchName 'OpenSanctuarySwitch'"));

        // Verify quote escaping
        let quote_script = build_enable_adapter_script("John's Wi-Fi", "OpenSanctuaryAdapter", "Switch's");
        assert!(quote_script.contains("-NetAdapterName 'John''s Wi-Fi'"));
        assert!(quote_script.contains("-Name 'Switch''s'"));

        let disable_script = build_disable_adapter_script("OpenSanctuaryAdapter");
        assert_eq!(
            disable_script,
            "Remove-VMNetworkAdapter -ManagementOS -Name 'OpenSanctuaryAdapter' -ErrorAction SilentlyContinue"
        );

        let uac_script = build_uac_powershell_wrapper("Write-Output 'Hello'");
        assert!(uac_script.starts_with("Start-Process powershell -Verb RunAs -ArgumentList '-NoProfile', '-EncodedCommand'"));
        // Decode base64 to ensure roundtrip UTF-16LE
        let encoded_part = uac_script.split('\'').nth(5).unwrap();
        let decoded = BASE64_STANDARD.decode(encoded_part).unwrap();
        let utf16_u16s: Vec<u16> = decoded.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let decoded_str = String::from_utf16(&utf16_u16s).unwrap();
        assert_eq!(decoded_str, "Write-Output 'Hello'");
    }

    #[test]
    fn test_classify_windows_interface() {
        assert_eq!(classify_windows_interface("Ethernet"), NetworkInterfaceType::Ethernet);
        assert_eq!(classify_windows_interface("Local Area Connection 2"), NetworkInterfaceType::Ethernet);
        assert_eq!(classify_windows_interface("Wi-Fi 3"), NetworkInterfaceType::Wireless);
        assert_eq!(classify_windows_interface("Wireless Network Connection"), NetworkInterfaceType::Wireless);
        assert_eq!(classify_windows_interface("Bluetooth Device"), NetworkInterfaceType::Unknown);
    }

    #[test]
    fn test_is_windows_interface_virtual() {
        assert!(is_windows_interface_virtual("vEthernet (OpenSanctuarySwitch)"));
        assert!(is_windows_interface_virtual("OpenSanctuaryAdapter"));
        assert!(is_windows_interface_virtual("Hyper-V Virtual Ethernet Adapter"));
        assert!(is_windows_interface_virtual("vEthernet (WSL)"));
        assert!(!is_windows_interface_virtual("Realtek PCIe GbE Family Controller"));
    }
}
