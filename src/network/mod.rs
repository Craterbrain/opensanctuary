//! Network configuration, multi-interface management, dedicated MAC adapter control,
//! conflict detection, and DHCP Option 12 broadcasting for OpenSanctuary.
//!
//! Split by domain (see each submodule's own doc comment): `hostname` (RFC
//! 1035 sanitization/conflict probing + local-IP/QR utilities),
//! `port_conflict`, `interfaces` (enumeration + hardware abstraction),
//! `virtual_adapter` (dedicated MAC adapter management), and `dhcp` (Option
//! 12 broadcasting). Everything is re-exported here so `crate::network::X`
//! call sites are unaffected by the split.

pub mod adb;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;
pub mod tls;
pub mod updater;
pub mod ytdlp_updater;

pub mod dhcp;
pub mod hostname;
pub mod interfaces;
pub mod port_conflict;
pub mod virtual_adapter;

pub use tls::*;

pub use dhcp::*;
pub use hostname::*;
pub use interfaces::*;
pub use port_conflict::*;
pub use virtual_adapter::*;
