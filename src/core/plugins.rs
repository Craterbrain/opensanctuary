use std::path::PathBuf;
use std::sync::Arc;
use crate::core::plugin_permissions::PermissionSet;
use libloading::{Library, Symbol};

/// The plugin hooks. **Never add or reorder methods** -- the trait object's vtable is the ABI
/// between the host and every plugin library built against it. New host capabilities go through
/// `PluginHost` instead.
pub trait OsPlugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn on_state_change(&self, state: &crate::core::models::ShowState);
    fn on_command(&self, cmd: &str) -> bool; // Return false to intercept/block

    /// Contributes additional resource items to a Resource Library category, merged
    /// alongside the ones already stored in the local database. `category` is one of
    /// "songs", "scriptures", "media", "presentations", "themes" — matching the
    /// frontend's tab names and the shape of `GET /api/<category>`'s normal rows
    /// (e.g. return objects shaped like `Song` for "songs"). Items are plain JSON so a
    /// plugin isn't coupled to OS-Next's exact internal struct definitions.
    ///
    /// Default: contributes nothing. Override only for categories this plugin provides.
    fn provide_resources(&self, _category: &str) -> Vec<serde_json::Value> {
        Vec::new()
    }
}

/// A failed host call from a plugin (a missing permission, a full queue, ...).
#[derive(Debug, Clone)]
pub struct PluginError(pub String);

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PluginError {}

/// What the host offers an ABI 2 plugin: every call is checked against that plugin's *live*
/// grant, so a revoke stops it even in the middle of a call sequence. A plugin receives this in
/// `create_plugin_v2`; a v1 plugin has none. See `export_plugin!` and docs/PLUGIN_PERMISSIONS.md.
///
/// The ABI is a Rust trait object, so a plugin must be built with the same compiler and the same
/// OS-Next version as the host (checked at load time via `OS_PLUGIN_HOST_VERSION`).
pub trait PluginHost: Send + Sync {
    fn plugin_id(&self) -> &str;
    /// Whether the plugin currently holds the permission id (e.g. `"show.observe"`).
    fn has(&self, permission: &str) -> bool;
    /// `show.observe`
    fn snapshot(&self) -> Result<crate::core::engine::StateSnapshot, PluginError>;
    /// `commands.send`, limited to the granted command names. Queued and run later through the
    /// normal veto/execution path -- never inline, so a hook can't re-enter the engine.
    fn send_command(&self, cmd: crate::core::commands::ShowCommand) -> Result<(), PluginError>;
    /// `library.read` for the category.
    fn library_list(&self, category: &str) -> Result<Vec<serde_json::Value>, PluginError>;
    /// `keyring.own`: the plugin's own namespace only.
    fn keyring_get(&self, account: &str) -> Result<Option<String>, PluginError>;
    fn keyring_set(&self, account: &str, secret: &str) -> Result<(), PluginError>;
    fn keyring_delete(&self, account: &str) -> Result<bool, PluginError>;
    /// `filesystem.plugin_data`: a private folder for this plugin, created on demand.
    fn data_dir(&self) -> Result<PathBuf, PluginError>;
    /// `level`: 0 error, 1 warn, 2 info, 3 debug.
    fn log(&self, level: u8, msg: &str);
    // --- appended in a later release: never reorder or insert above this line ---
    /// `keyring.service`: reads a secret from a shared namespace (`OpenSanctuary:Shared:<name>`) the
    /// plugin was granted. `None` when there is none or the plugin may not see it.
    fn keyring_shared_get(&self, service: &str, account: &str) -> Result<Option<String>, PluginError>;
    /// `keyring.service` with `write` access: stores a secret there, as this plugin's own entry.
    fn keyring_shared_set(&self, service: &str, account: &str, secret: &str) -> Result<(), PluginError>;
    /// `keyring.service` with `write` access: deletes an entry this plugin created.
    fn keyring_shared_delete(&self, service: &str, account: &str) -> Result<bool, PluginError>;
}

/// A plugin's live grant state. Shared (`Arc`) so a revoke from the console takes effect
/// immediately even though the manager itself is immutable once the server is running.
pub struct GrantCell {
    enabled: std::sync::atomic::AtomicBool,
    perms: std::sync::RwLock<PermissionSet>,
}

impl GrantCell {
    pub fn new(perms: PermissionSet) -> Self {
        Self { enabled: std::sync::atomic::AtomicBool::new(true), perms: std::sync::RwLock::new(perms) }
    }

    /// Runs `f` on the current grant, or returns `None` once the plugin has been revoked.
    pub fn with<R>(&self, f: impl FnOnce(&PermissionSet) -> R) -> Option<R> {
        self.enabled
            .load(std::sync::atomic::Ordering::Acquire)
            .then(|| f(&self.perms.read().unwrap_or_else(|e| e.into_inner())))
    }

    pub fn allows(&self, check: impl FnOnce(&PermissionSet) -> bool) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::Acquire)
            && check(&self.perms.read().unwrap_or_else(|e| e.into_inner()))
    }
}

struct LoadedPlugin {
    id: String,
    grants: Arc<GrantCell>,
    plugin: Box<dyn OsPlugin>,
    /// Only a plugin that was granted `commands.modify` *at load time* and exports a modifier has one.
    modifier: Option<ModifierWorker>,
}

/// `commands.modify`: rewrite a command before it runs. Return `None` to leave it unchanged or
/// `Some(json)` to replace it. The host only shows you commands in your granted list, the kind of
/// command may not change, and a slow or failing modifier is skipped (and eventually switched off).
/// It is not a security control (any failure lets the original command run): use `commands.intercept`
/// to veto. Exported with `export_command_modifier!`.
pub trait CommandModifier: Send + Sync {
    fn modify_command(&self, cmd_json: &str) -> Option<String>;
}

/// The longest one modifier may take for one command, and the most all of them may take together.
pub const MODIFIER_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(50);
pub const MODIFIER_BUDGET: std::time::Duration = std::time::Duration::from_millis(100);
/// This many invalid rewrites or panics in a row switch a modifier off until restart.
const MODIFIER_MAX_FAILURES: u32 = 3;

enum CallResult {
    Unchanged,
    Rewritten(String),
    Panicked,
    TimedOut,
    Busy,
}

/// One modifier on its own thread. Rust can't pre-empt a thread, so a hung modifier keeps its thread;
/// the only isolation available is to stop waiting (and never call it again).
struct ModifierWorker {
    tx: std::sync::mpsc::SyncSender<(String, std::sync::mpsc::SyncSender<Option<Option<String>>>)>,
    tripped: std::sync::atomic::AtomicBool,
    failures: std::sync::atomic::AtomicU32,
    changed: std::sync::atomic::AtomicU64,
    /// A call is outstanding (it timed out earlier and hasn't returned).
    in_flight: Arc<std::sync::atomic::AtomicBool>,
}

impl ModifierWorker {
    fn spawn(id: &str, modifier: Box<dyn CommandModifier>) -> Self {
        use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
        let (tx, rx) = std::sync::mpsc::sync_channel::<(String, std::sync::mpsc::SyncSender<Option<Option<String>>>)>(1);
        let in_flight = Arc::new(AtomicBool::new(false));
        let flag = in_flight.clone();
        let _ = std::thread::Builder::new().name(format!("plugin-modify-{id}")).spawn(move || {
            while let Ok((text, reply)) = rx.recv() {
                let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| modifier.modify_command(&text)));
                flag.store(false, Ordering::Release);
                // A closed reply channel just means the host stopped waiting.
                let _ = reply.send(out.ok());
            }
        });
        Self { tx, tripped: AtomicBool::new(false), failures: AtomicU32::new(0), changed: AtomicU64::new(0), in_flight }
    }

    fn call(&self, text: String, timeout: std::time::Duration) -> CallResult {
        use std::sync::atomic::Ordering;
        if self.in_flight.swap(true, Ordering::AcqRel) {
            return CallResult::Busy;
        }
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        if self.tx.try_send((text, reply_tx)).is_err() {
            self.in_flight.store(false, Ordering::Release);
            return CallResult::Busy;
        }
        match reply_rx.recv_timeout(timeout) {
            Ok(Some(None)) => CallResult::Unchanged,
            Ok(Some(Some(rewritten))) => CallResult::Rewritten(rewritten),
            Ok(None) => CallResult::Panicked,
            Err(_) => CallResult::TimedOut,
        }
    }

    fn trip(&self) {
        self.tripped.store(true, std::sync::atomic::Ordering::Release);
    }

    fn is_tripped(&self) -> bool {
        self.tripped.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// What happened while modifiers ran on a command, for the caller to audit and tell the operator about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModifyEvent {
    Modified { plugin: String, variant: String, before_sha: String, after_sha: String },
    Rejected { plugin: String, variant: String, reason: String },
    /// A modifier was switched off until restart.
    Disabled { plugin: String, reason: String },
    /// The 100 ms budget ran out; the remaining modifiers were skipped for this command.
    BudgetExhausted { variant: String },
}

/// A native library the registry found approved and unchanged, ready to load.
pub struct ApprovedNative {
    pub id: String,
    pub library: PathBuf,
    pub grants: PermissionSet,
}

/// Builds the `PluginHost` handed to an ABI 2 plugin.
pub type HostFactory<'a> = &'a dyn Fn(&str, Arc<GrantCell>) -> Arc<dyn PluginHost>;

/// How many bytes of `OS_PLUGIN_HOST_VERSION` a v2 plugin exports (NUL padded).
pub const ABI_VERSION_FIELD_LEN: usize = 32;

pub struct PluginManager {
    plugins: Vec<LoadedPlugin>,
    _libraries: Vec<Library>, // Keep libraries loaded in memory
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            _libraries: Vec::new(),
        }
    }

    /// Loads only the libraries the plugin registry has verified as approved with a
    /// matching digest (docs/PLUGIN_PERMISSIONS.md). One broken plugin must not stop the
    /// rest from loading. `host` is how ABI 2 plugins get their `PluginHost`; without one
    /// (tests) a v2 plugin is refused.
    pub fn load_approved(&mut self, approved: Vec<ApprovedNative>, host: Option<HostFactory<'_>>) {
        for a in approved {
            let grants = Arc::new(GrantCell::new(a.grants.clone()));
            match self.load_library(&a, &grants, host) {
                Ok((plugin, modifier, lib)) => {
                    tracing::info!("Loaded Backend Plugin: {} ({})", a.id, plugin.name());
                    self.plugins.push(LoadedPlugin { id: a.id, grants, plugin, modifier });
                    self._libraries.push(lib);
                }
                Err(e) => tracing::error!("Failed to load plugin {:?}: {}", a.library, e),
            }
        }
    }

    fn load_library(
        &self,
        a: &ApprovedNative,
        grants: &Arc<GrantCell>,
        host: Option<HostFactory<'_>>,
    ) -> Result<(Box<dyn OsPlugin>, Option<ModifierWorker>, Library), Box<dyn std::error::Error>> {
        unsafe {
            let lib = Library::new(&a.library)?;
            // The library must say it speaks ABI 2 and must have been built for *this* host.
            let exported_abi: Option<u32> = lib.get::<*const u32>(b"OS_PLUGIN_ABI").ok().map(|s| **s);
            if exported_abi != Some(2) {
                return Err("the library doesn't export OS_PLUGIN_ABI = 2 (plugins built for the old ABI 1 are no longer supported)".into());
            }
            let ver: libloading::Symbol<*const [u8; ABI_VERSION_FIELD_LEN]> = lib.get(b"OS_PLUGIN_HOST_VERSION")?;
            let bytes: [u8; ABI_VERSION_FIELD_LEN] = **ver;
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(ABI_VERSION_FIELD_LEN);
            let built_for = std::str::from_utf8(&bytes[..end]).unwrap_or("?");
            if built_for != env!("CARGO_PKG_VERSION") {
                return Err(format!(
                    "plugin was built for OS-Next {built_for}, this is {}; rebuild it against this version",
                    env!("CARGO_PKG_VERSION")
                )
                .into());
            }
            let factory = host.ok_or("this build can't give a plugin a host handle")?;
            let handle: Arc<dyn PluginHost> = factory(&a.id, grants.clone());
            let constructor: Symbol<unsafe extern "C" fn(*const Arc<dyn PluginHost>) -> *mut dyn OsPlugin> =
                lib.get(b"create_plugin_v2")?;
            let raw_plugin = constructor(&handle as *const Arc<dyn PluginHost>);
            let plugin = Box::from_raw(raw_plugin);
            // commands.modify: the optional exported modifier is only ever constructed when the
            // permission was granted. A library that exports one without the grant is never asked.
            let granted = grants.with(|g| g.0.iter().any(|p| p.id == "commands.modify")).unwrap_or(false);
            let symbol = lib.get::<unsafe extern "C" fn(*const Arc<dyn PluginHost>) -> *mut dyn CommandModifier>(b"create_command_modifier_v1");
            let modifier = match (granted, symbol) {
                (true, Ok(ctor)) => {
                    let raw = ctor(&handle as *const Arc<dyn PluginHost>);
                    Some(ModifierWorker::spawn(&a.id, Box::from_raw(raw)))
                }
                (true, Err(_)) => {
                    tracing::warn!("plugin '{}' was granted commands.modify but doesn't export a modifier", a.id);
                    None
                }
                (false, Ok(_)) => {
                    tracing::info!("plugin '{}' exports a command modifier but wasn't granted commands.modify; it will not be called", a.id);
                    None
                }
                (false, Err(_)) => None,
            };
            Ok((plugin, modifier, lib))
        }
    }

    /// Gives an already-registered in-process plugin a command modifier (for host code and tests; a
    /// library's modifier is created at load time and only if it was granted `commands.modify`).
    pub fn set_modifier(&mut self, id: &str, modifier: Box<dyn CommandModifier>) -> bool {
        match self.plugins.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.modifier = Some(ModifierWorker::spawn(id, modifier));
                true
            }
            None => false,
        }
    }

    /// `(commands changed this run, switched off until restart)` for a plugin that has a modifier.
    pub fn modifier_stats(&self, id: &str) -> Option<(u64, bool)> {
        let w = self.plugins.iter().find(|p| p.id == id)?.modifier.as_ref()?;
        Some((w.changed.load(std::sync::atomic::Ordering::Relaxed), w.is_tripped()))
    }

    /// Runs the plugins' modifiers over a command (`commands.modify`). Called after shorthand
    /// expansion and before the veto, so `commands.intercept` plugins judge the command that will
    /// actually run. Emergency controls and anything modifiable by nobody are returned untouched
    /// without consulting anyone. Modifiers run in plugin-id order, each on the previous one's output,
    /// once; any failure keeps the last good command (this never stops a show). Returns the command to
    /// run and what the caller should audit.
    pub fn apply_modifiers(&self, value: serde_json::Value) -> (serde_json::Value, Vec<ModifyEvent>) {
        use crate::core::plugin_permissions::{command_variant, is_safety_critical, sha256_hex};
        let mut events = Vec::new();
        if !self.plugins.iter().any(|p| p.modifier.is_some()) {
            return (value, events);
        }
        let text = serde_json::to_string(&value).unwrap_or_default();
        let Some(variant) = command_variant(&text) else { return (value, events) };
        // Decided before any modifier runs, so the emergency path is never delayed or touched.
        if is_safety_critical(&text) || crate::core::commands::NEVER_MODIFIABLE.contains(&variant.as_str()) {
            return (value, events);
        }
        let mut candidates: Vec<&LoadedPlugin> = self
            .plugins
            .iter()
            .filter(|p| p.modifier.as_ref().is_some_and(|m| !m.is_tripped()))
            .filter(|p| p.grants.allows(|g| g.modifies(&variant)))
            .collect();
        candidates.sort_by(|a, b| a.id.cmp(&b.id));
        let started = std::time::Instant::now();
        let mut current = value;
        for p in candidates {
            let Some(worker) = p.modifier.as_ref() else { continue };
            let spent = started.elapsed();
            if spent >= MODIFIER_BUDGET {
                events.push(ModifyEvent::BudgetExhausted { variant: variant.clone() });
                break;
            }
            let cur_text = serde_json::to_string(&current).unwrap_or_default();
            let wait = MODIFIER_TIMEOUT.min(MODIFIER_BUDGET - spent);
            use std::sync::atomic::Ordering;
            let mut fail = |reason: &str, events: &mut Vec<ModifyEvent>| {
                events.push(ModifyEvent::Rejected { plugin: p.id.clone(), variant: variant.clone(), reason: reason.to_string() });
                if worker.failures.fetch_add(1, Ordering::AcqRel) + 1 >= MODIFIER_MAX_FAILURES {
                    worker.trip();
                    events.push(ModifyEvent::Disabled { plugin: p.id.clone(), reason: "it kept failing while changing commands".to_string() });
                }
            };
            match worker.call(cur_text.clone(), wait) {
                CallResult::Unchanged => worker.failures.store(0, Ordering::Release),
                CallResult::Rewritten(new_text) => match crate::core::commands::validate_rewrite(&current, &variant, &new_text) {
                    Ok(new_value) => {
                        worker.failures.store(0, Ordering::Release);
                        let after = serde_json::to_string(&new_value).unwrap_or_default();
                        if after != cur_text {
                            worker.changed.fetch_add(1, Ordering::Relaxed);
                            events.push(ModifyEvent::Modified { plugin: p.id.clone(), variant: variant.clone(), before_sha: sha256_hex(cur_text.as_bytes()), after_sha: sha256_hex(after.as_bytes()) });
                            current = new_value;
                        }
                    }
                    Err(why) => fail(why, &mut events),
                },
                CallResult::Panicked => fail("it panicked", &mut events),
                CallResult::TimedOut => {
                    worker.trip();
                    events.push(ModifyEvent::Disabled { plugin: p.id.clone(), reason: "it was too slow while changing a command".to_string() });
                }
                CallResult::Busy => {
                    worker.trip();
                    events.push(ModifyEvent::Disabled { plugin: p.id.clone(), reason: "it was still busy with an earlier command".to_string() });
                }
            }
        }
        (current, events)
    }

    /// Whether the loaded plugin currently holds `commands.send` for this command name.
    /// Checked again when a queued command is about to run, so a revoke in between wins.
    pub fn command_allowed(&self, id: &str, variant: &str) -> bool {
        self.plugins.iter().find(|p| p.id == id).is_some_and(|p| p.grants.allows(|g| g.allows_command(variant)))
    }

    /// Registers an in-process plugin with the full v1 permission set. For code that is
    /// part of the host (and tests); anything loaded from disk goes through
    /// `load_approved` and the grant flow.
    pub fn register_plugin(&mut self, plugin: Box<dyn OsPlugin>) {
        let id = plugin.name().to_string();
        self.register_plugin_with_grants(id, plugin, PermissionSet::in_process());
    }

    pub fn register_plugin_with_grants(&mut self, id: String, plugin: Box<dyn OsPlugin>, grants: PermissionSet) {
        tracing::info!("Registered Plugin: {}", plugin.name());
        self.plugins.push(LoadedPlugin { id, grants: Arc::new(GrantCell::new(grants)), plugin, modifier: None });
    }

    /// Replaces a loaded plugin's grants and re-enables it. Returns whether it is loaded.
    pub fn set_grants(&self, id: &str, grants: PermissionSet) -> bool {
        match self.plugins.iter().find(|p| p.id == id) {
            Some(p) => {
                *p.grants.perms.write().unwrap_or_else(|e| e.into_inner()) = grants;
                p.grants.enabled.store(true, std::sync::atomic::Ordering::Release);
                true
            }
            None => false,
        }
    }

    /// Stops every host-mediated hook for the plugin at once. (A native library stays
    /// mapped and any threads it started keep running until the next restart.)
    pub fn revoke(&self, id: &str) -> bool {
        match self.plugins.iter().find(|p| p.id == id) {
            Some(p) => {
                p.grants.enabled.store(false, std::sync::atomic::Ordering::Release);
                *p.grants.perms.write().unwrap_or_else(|e| e.into_inner()) = PermissionSet::empty();
                true
            }
            None => false,
        }
    }

    pub fn is_loaded(&self, id: &str) -> bool {
        self.plugins.iter().any(|p| p.id == id)
    }

    pub fn is_safety_critical_command(cmd: &str) -> bool {
        crate::core::plugin_permissions::is_safety_critical(cmd)
    }

    pub fn check_command(&self, cmd: &str) -> bool {
        // Safety-critical emergency overrides can never be vetoed by any plugin
        if Self::is_safety_critical_command(cmd) {
            return true;
        }

        for loaded in &self.plugins {
            if !loaded.grants.allows(|g| g.has("commands.intercept")) {
                continue;
            }
            let plugin = &loaded.plugin;
            let plugin_name = plugin.name();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                plugin.on_command(cmd)
            }));

            match result {
                Ok(allowed) => {
                    if !allowed {
                        tracing::warn!("Command '{}' vetoed by plugin '{}'", cmd, plugin_name);
                        return false;
                    }
                }
                Err(_) => {
                    tracing::error!(
                        "Plugin '{}' panicked inside on_command()! Panic caught and isolated; command permitted.",
                        plugin_name
                    );
                }
            }
        }
        true
    }

    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    /// Collects every loaded plugin's contributed items for a Resource Library
    /// category (see `OsPlugin::provide_resources`), in plugin registration order.
    /// Only plugins granted `resources.provide` for that category are asked, and each
    /// item is stamped with `_plugin` so the console can show where it came from.
    /// A panicking plugin is logged and contributes nothing for this call, same as
    /// the other hooks.
    pub fn collect_resources(&self, category: &str) -> Vec<serde_json::Value> {
        let mut items = Vec::new();
        for loaded in &self.plugins {
            if !loaded.grants.allows(|g| g.provides(category)) {
                continue;
            }
            let plugin = &loaded.plugin;
            let plugin_name = plugin.name();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                plugin.provide_resources(category)
            }));
            match result {
                Ok(mut contributed) => {
                    for item in contributed.iter_mut() {
                        if let Some(obj) = item.as_object_mut() {
                            obj.insert("_plugin".to_string(), serde_json::Value::String(loaded.id.clone()));
                        }
                    }
                    items.append(&mut contributed)
                }
                Err(_) => {
                    tracing::error!(
                        "Plugin '{}' panicked inside provide_resources('{}')! Panic caught and isolated.",
                        plugin_name, category
                    );
                }
            }
        }
        items
    }

    pub fn notify_state_change(&self, state: &crate::core::models::ShowState) {
        for loaded in &self.plugins {
            if !loaded.grants.allows(|g| g.has("show.observe")) {
                continue;
            }
            let plugin = &loaded.plugin;
            let plugin_name = plugin.name();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                plugin.on_state_change(state);
            }));

            if result.is_err() {
                tracing::error!(
                    "Plugin '{}' panicked inside on_state_change()! Panic caught and isolated.",
                    plugin_name
                );
            }
        }
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Exports a plugin from a `cdylib` crate built against this crate (ABI 2). Emits the
/// version/ABI markers the host checks before it will call anything, plus `create_plugin_v2`.
///
/// ```ignore
/// os_next::export_plugin!(|host| MyPlugin::new(host));
/// ```
#[macro_export]
macro_rules! export_plugin {
    ($ctor:expr) => {
        #[no_mangle]
        pub static OS_PLUGIN_ABI: u32 = 2;

        #[no_mangle]
        pub static OS_PLUGIN_HOST_VERSION: [u8; $crate::core::plugins::ABI_VERSION_FIELD_LEN] = {
            let v = env!("CARGO_PKG_VERSION").as_bytes();
            let mut out = [0u8; $crate::core::plugins::ABI_VERSION_FIELD_LEN];
            let mut i = 0;
            while i < v.len() && i < out.len() - 1 {
                out[i] = v[i];
                i += 1;
            }
            out
        };

        #[no_mangle]
        #[allow(improper_ctypes_definitions)]
        pub extern "C" fn create_plugin_v2(
            host: *const ::std::sync::Arc<dyn $crate::core::plugins::PluginHost>,
        ) -> *mut dyn $crate::core::plugins::OsPlugin {
            let host: ::std::sync::Arc<dyn $crate::core::plugins::PluginHost> = unsafe { (*host).clone() };
            let plugin: ::std::boxed::Box<dyn $crate::core::plugins::OsPlugin> = ::std::boxed::Box::new(($ctor)(host));
            ::std::boxed::Box::into_raw(plugin)
        }
    };
}

/// Exports a command modifier (`commands.modify`) from a plugin library, alongside `export_plugin!`.
/// The host only constructs it when the operator granted `commands.modify`.
///
/// ```ignore
/// os_next::export_command_modifier!(|host| MyModifier::new(host));
/// ```
#[macro_export]
macro_rules! export_command_modifier {
    ($ctor:expr) => {
        #[no_mangle]
        #[allow(improper_ctypes_definitions)]
        pub extern "C" fn create_command_modifier_v1(
            host: *const ::std::sync::Arc<dyn $crate::core::plugins::PluginHost>,
        ) -> *mut dyn $crate::core::plugins::CommandModifier {
            let host: ::std::sync::Arc<dyn $crate::core::plugins::PluginHost> = unsafe { (*host).clone() };
            let modifier: ::std::boxed::Box<dyn $crate::core::plugins::CommandModifier> = ::std::boxed::Box::new(($ctor)(host));
            ::std::boxed::Box::into_raw(modifier)
        }
    };
}
