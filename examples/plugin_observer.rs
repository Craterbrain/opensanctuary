//! Template for a native OpenSanctuary plugin (ABI 2).
//!
//! Build:  cargo build --release --example plugin_observer
//! Install: put the library in `<plugins dir>/<id>/lib/` next to a `plugin.json` that pins its
//! SHA-256 and lists the permissions below, then approve it in Settings > Plugins. See
//! docs/PLUGINS.md and docs/PLUGIN_PERMISSIONS.md.
//!
//! The plugin must be built with the same compiler and the same OS-Next version as the host:
//! the interface is a Rust trait object, and the host refuses a library built for another version.
//!
//! What it does: on every state change it records, in its private data folder, what the host
//! allowed it to do -- so a permission that wasn't granted shows up as "denied".

use os_next::core::models::ShowState;
use os_next::core::plugins::{CommandModifier, OsPlugin, PluginHost};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Observer {
    host: Arc<dyn PluginHost>,
    seen: AtomicUsize,
}

fn word<T, E>(r: &Result<T, E>) -> &'static str {
    if r.is_ok() { "ok" } else { "denied" }
}

impl OsPlugin for Observer {
    fn name(&self) -> &'static str {
        "plugin_observer"
    }

    fn on_state_change(&self, _state: &ShowState) {
        let n = self.seen.fetch_add(1, Ordering::SeqCst) + 1;
        self.host.log(2, "observed a state change");

        // Each of these is checked by the host against what the operator granted.
        let snapshot = self.host.snapshot();
        let keyring = self.host.keyring_set("probe", "x");
        let send = self.host.send_command(os_next::core::commands::ShowCommand::NextSlide);

        if let Ok(dir) = self.host.data_dir() {
            let line = format!(
                "n={n} snapshot={} keyring={} send={}\n",
                word(&snapshot),
                word(&keyring),
                word(&send)
            );
            let _ = std::fs::write(dir.join("observed.txt"), line);
        }
    }

    fn on_command(&self, _cmd: &str) -> bool {
        true
    }
}

os_next::export_plugin!(|host| Observer { host, seen: AtomicUsize::new(0) });

/// `commands.modify`: shouts the schedule title. The host only builds this when the operator granted
/// `commands.modify` (listing `SetScheduleTitle`); it can change that command's data, never turn it
/// into another command, and never sees the emergency controls.
struct Shouter;

impl CommandModifier for Shouter {
    fn modify_command(&self, cmd_json: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(cmd_json).ok()?;
        let title = v.get("SetScheduleTitle")?.as_str()?;
        Some(serde_json::json!({ "SetScheduleTitle": title.to_uppercase() }).to_string())
    }
}

os_next::export_command_modifier!(|_host| Shouter);
