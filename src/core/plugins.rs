use std::path::Path;
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

pub struct PluginManager {
    plugins: Vec<Box<dyn OsPlugin>>,
    _libraries: Vec<Library>, // Keep libraries loaded in memory
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            _libraries: Vec::new(),
        }
    }

    pub fn load_plugins_from_dir<P: AsRef<Path>>(&mut self, dir: P) -> Result<(), Box<dyn std::error::Error>> {
        let dir = dir.as_ref();
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    // Load .so (Linux) or .dll (Windows) or .dylib (macOS)
                    if ext == "so" || ext == "dll" || ext == "dylib" {
                        self.load_plugin(&path)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn load_plugin(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        unsafe {
            let lib = Library::new(path)?;
            
            // Expected symbol: create_plugin() -> *mut dyn OsPlugin
            let constructor: Symbol<unsafe extern "C" fn() -> *mut dyn OsPlugin> = lib.get(b"create_plugin")?;
            
            let raw_plugin = constructor();
            let plugin = Box::from_raw(raw_plugin);
            
            tracing::info!("Loaded Backend Plugin: {}", plugin.name());
            
            self.plugins.push(plugin);
            self._libraries.push(lib);
        }
        Ok(())
    }

    pub fn register_plugin(&mut self, plugin: Box<dyn OsPlugin>) {
        tracing::info!("Registered Plugin: {}", plugin.name());
        self.plugins.push(plugin);
    }

    pub fn is_safety_critical_command(cmd: &str) -> bool {
        cmd.contains("ToggleBlackout")
            || cmd.contains("ToggleClearText")
            || cmd.contains("ToggleLogo")
            || cmd.contains("SetAlert")
    }

    pub fn check_command(&self, cmd: &str) -> bool {
        // Safety-critical emergency overrides can never be vetoed by any plugin
        if Self::is_safety_critical_command(cmd) {
            return true;
        }

        for plugin in &self.plugins {
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
    /// A panicking plugin is logged and contributes nothing for this call, same as
    /// the other hooks.
    pub fn collect_resources(&self, category: &str) -> Vec<serde_json::Value> {
        let mut items = Vec::new();
        for plugin in &self.plugins {
            let plugin_name = plugin.name();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                plugin.provide_resources(category)
            }));
            match result {
                Ok(mut contributed) => items.append(&mut contributed),
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
        for plugin in &self.plugins {
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
