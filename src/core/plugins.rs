use std::path::{Path, PathBuf};
use std::sync::Arc;
use crate::core::plugin_permissions::PermissionSet;
use libloading::{Library, Symbol};

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

/// A plugin's live grant state. Shared (`Arc`) so a revoke from the console takes effect
/// immediately even though the manager itself is immutable once the server is running.
pub struct GrantCell {
    enabled: std::sync::atomic::AtomicBool,
    perms: std::sync::RwLock<PermissionSet>,
}

impl GrantCell {
    fn new(perms: PermissionSet) -> Self {
        Self { enabled: std::sync::atomic::AtomicBool::new(true), perms: std::sync::RwLock::new(perms) }
    }

    fn allows(&self, check: impl FnOnce(&PermissionSet) -> bool) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::Acquire)
            && check(&self.perms.read().unwrap_or_else(|e| e.into_inner()))
    }
}

struct LoadedPlugin {
    id: String,
    grants: Arc<GrantCell>,
    plugin: Box<dyn OsPlugin>,
}

/// A native library the registry found approved and unchanged, ready to load.
pub struct ApprovedNative {
    pub id: String,
    pub library: PathBuf,
    pub grants: PermissionSet,
}

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
    /// rest from loading.
    pub fn load_approved(&mut self, approved: Vec<ApprovedNative>) {
        for a in approved {
            match self.load_library(&a.library) {
                Ok((plugin, lib)) => {
                    tracing::info!("Loaded Backend Plugin: {} ({})", a.id, plugin.name());
                    self.plugins.push(LoadedPlugin { id: a.id, grants: Arc::new(GrantCell::new(a.grants)), plugin });
                    self._libraries.push(lib);
                }
                Err(e) => tracing::error!("Failed to load plugin {:?}: {}", a.library, e),
            }
        }
    }

    fn load_library(&self, path: &Path) -> Result<(Box<dyn OsPlugin>, Library), Box<dyn std::error::Error>> {
        unsafe {
            let lib = Library::new(path)?;

            // Expected symbol: create_plugin() -> *mut dyn OsPlugin
            let constructor: Symbol<unsafe extern "C" fn() -> *mut dyn OsPlugin> = lib.get(b"create_plugin")?;

            let raw_plugin = constructor();
            Ok((Box::from_raw(raw_plugin), lib))
        }
    }

    /// Registers an in-process plugin with the full v1 permission set. For code that is
    /// part of the host (and tests); anything loaded from disk goes through
    /// `load_approved` and the grant flow.
    pub fn register_plugin(&mut self, plugin: Box<dyn OsPlugin>) {
        let id = plugin.name().to_string();
        self.register_plugin_with_grants(id, plugin, PermissionSet::legacy_full());
    }

    pub fn register_plugin_with_grants(&mut self, id: String, plugin: Box<dyn OsPlugin>, grants: PermissionSet) {
        tracing::info!("Registered Plugin: {}", plugin.name());
        self.plugins.push(LoadedPlugin { id, grants: Arc::new(GrantCell::new(grants)), plugin });
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
