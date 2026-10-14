//! The host side of the ABI 2 plugin interface (`PluginHost`): what a native plugin can ask
//! the server to do, each call checked against that plugin's live grant
//! (docs/PLUGIN_PERMISSIONS.md).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use crate::api::ws::AppState;
use crate::core::commands::ShowCommand;
use crate::core::engine::StateSnapshot;
use crate::core::plugin_permissions::command_variant;
use crate::core::plugins::{GrantCell, PluginError, PluginHost};

/// The server's `AppState` doesn't exist yet when plugins are loaded, so hosts hold a slot that
/// `main` fills in once it does. Calls made before then fail with a clear error.
pub type AppSlot = Arc<OnceLock<AppState>>;

/// A command a plugin queued; run by `drain_plugin_commands` through the normal veto and
/// execution path. Queued rather than run inline so a hook can never re-enter the engine
/// (`on_state_change` -> `send_command` -> `on_state_change` ...).
pub struct QueuedPluginCommand {
    pub plugin_id: String,
    pub variant: String,
    pub cmd: ShowCommand,
}

pub type CommandQueue = tokio::sync::mpsc::Sender<QueuedPluginCommand>;

/// Per-plugin ceiling: at most this many commands per rolling second.
const COMMANDS_PER_SECOND: usize = 20;

pub struct AppPluginHost {
    plugin_id: String,
    grants: Arc<GrantCell>,
    app: AppSlot,
    queue: CommandQueue,
    recent: std::sync::Mutex<std::collections::VecDeque<std::time::Instant>>,
}

impl AppPluginHost {
    pub fn new(plugin_id: &str, grants: Arc<GrantCell>, app: AppSlot, queue: CommandQueue) -> Self {
        Self { plugin_id: plugin_id.to_string(), grants, app, queue, recent: Default::default() }
    }

    fn app(&self) -> Result<&AppState, PluginError> {
        self.app.get().ok_or_else(|| PluginError("the server is still starting".to_string()))
    }

    fn need(&self, permission: &str) -> Result<(), PluginError> {
        if self.grants.allows(|g| g.has(permission)) {
            Ok(())
        } else {
            Err(PluginError(format!("permission '{permission}' not granted")))
        }
    }

    fn namespace(&self) -> String {
        format!("OpenSanctuary:Plugin:{}", self.plugin_id)
    }

    fn owner_tag(&self) -> String {
        format!("plugin:{}", self.plugin_id)
    }
}

impl AppPluginHost {
    /// Applies the shared-keyring policy (the same one the HTTP routes use) for one operation.
    fn shared_verdict(&self, op: crate::api::keyring_policy::Op, service: &str, account: &str) -> Result<crate::api::keyring_policy::Verdict, PluginError> {
        use crate::api::keyring_policy::{audit_once, check, Verdict};
        let app = self.app()?;
        let verdict = self
            .grants
            .with(|g| check(&app.db, &self.plugin_id, g, op, service, account))
            .ok_or_else(|| PluginError("this plugin has been revoked".to_string()))?;
        audit_once(&app.db, &self.plugin_id, service, account, op, !matches!(verdict, Verdict::Allow { .. }));
        Ok(verdict)
    }
}

impl PluginHost for AppPluginHost {
    fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    fn has(&self, permission: &str) -> bool {
        self.grants.allows(|g| g.has(permission))
    }

    fn snapshot(&self) -> Result<StateSnapshot, PluginError> {
        self.need("show.observe")?;
        Ok(self.app()?.engine.snapshot())
    }

    fn send_command(&self, cmd: ShowCommand) -> Result<(), PluginError> {
        let text = serde_json::to_string(&cmd).map_err(|e| PluginError(e.to_string()))?;
        let variant = command_variant(&text).unwrap_or_default();
        if !self.grants.allows(|g| g.allows_command(&variant)) {
            return Err(PluginError(format!("sending '{variant}' not granted")));
        }
        {
            let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
            let now = std::time::Instant::now();
            while recent.front().is_some_and(|t| now.duration_since(*t) > std::time::Duration::from_secs(1)) {
                recent.pop_front();
            }
            if recent.len() >= COMMANDS_PER_SECOND {
                return Err(PluginError("sending commands too fast".to_string()));
            }
            recent.push_back(now);
        }
        self.queue
            .try_send(QueuedPluginCommand { plugin_id: self.plugin_id.clone(), variant, cmd })
            .map_err(|_| PluginError("the command queue is full".to_string()))
    }

    fn library_list(&self, category: &str) -> Result<Vec<serde_json::Value>, PluginError> {
        if !self.grants.allows(|g| g.reads(category)) {
            return Err(PluginError(format!("reading '{category}' not granted")));
        }
        let db = &self.app()?.db;
        let rows = match category {
            "songs" => db.get_songs().map(|v| v.into_iter().filter_map(|x| serde_json::to_value(x).ok()).collect()),
            "scriptures" => db.get_scriptures().map(|v| v.into_iter().filter_map(|x| serde_json::to_value(x).ok()).collect()),
            "media" => db.get_media().map(|v| v.into_iter().filter_map(|x| serde_json::to_value(x).ok()).collect()),
            "presentations" => db.get_presentations().map(|v| v.into_iter().filter_map(|x| serde_json::to_value(x).ok()).collect()),
            "themes" => db.get_themes().map(|v| v.into_iter().filter_map(|x| serde_json::to_value(x).ok()).collect()),
            other => return Err(PluginError(format!("unknown category '{other}'"))),
        };
        rows.map_err(|e| PluginError(format!("database error: {e}")))
    }

    fn keyring_get(&self, account: &str) -> Result<Option<String>, PluginError> {
        self.need("keyring.own")?;
        let app = self.app()?;
        let service = self.namespace();
        // Same ownership rule as the HTTP keyring routes: only secrets this plugin created.
        if app.db.get_keyring_owner(&service, account).unwrap_or(None).as_deref() != Some(self.owner_tag().as_str()) {
            return Ok(None);
        }
        crate::storage::KeyringService::get_secret(&service, account).map_err(PluginError)
    }

    fn keyring_set(&self, account: &str, secret: &str) -> Result<(), PluginError> {
        self.need("keyring.own")?;
        if account.trim().is_empty() {
            return Err(PluginError("account must not be empty".to_string()));
        }
        let app = self.app()?;
        let service = self.namespace();
        if let Ok(Some(owner)) = app.db.get_keyring_owner(&service, account) {
            if owner != self.owner_tag() {
                return Err(PluginError(format!("secret is owned by '{owner}'")));
            }
        }
        crate::storage::KeyringService::set_secret(&service, account, secret).map_err(PluginError)?;
        let _ = app.db.record_keyring_owner(&service, account, &self.owner_tag());
        Ok(())
    }

    fn keyring_delete(&self, account: &str) -> Result<bool, PluginError> {
        self.need("keyring.own")?;
        let app = self.app()?;
        let service = self.namespace();
        if app.db.get_keyring_owner(&service, account).unwrap_or(None).as_deref() != Some(self.owner_tag().as_str()) {
            return Ok(false);
        }
        let deleted = crate::storage::KeyringService::delete_secret(&service, account).map_err(PluginError)?;
        let _ = app.db.delete_keyring_metadata(&service, account);
        Ok(deleted)
    }

    fn data_dir(&self) -> Result<PathBuf, PluginError> {
        self.need("filesystem.plugin_data")?;
        let dir = self.app()?.data_dir.join("plugin-data").join(&self.plugin_id);
        std::fs::create_dir_all(&dir).map_err(|e| PluginError(format!("can't create {dir:?}: {e}")))?;
        Ok(dir)
    }

    fn keyring_shared_get(&self, service: &str, account: &str) -> Result<Option<String>, PluginError> {
        use crate::api::keyring_policy::{Op, Verdict};
        match self.shared_verdict(Op::Get, service, account)? {
            Verdict::Allow { .. } => crate::storage::KeyringService::get_secret(service, account).map_err(PluginError),
            Verdict::NotFound => Ok(None),
            Verdict::Deny(m) => Err(PluginError(m)),
        }
    }

    fn keyring_shared_set(&self, service: &str, account: &str, secret: &str) -> Result<(), PluginError> {
        use crate::api::keyring_policy::{Op, Verdict};
        if account.trim().is_empty() {
            return Err(PluginError("account must not be empty".to_string()));
        }
        match self.shared_verdict(Op::Set, service, account)? {
            Verdict::Allow { owner } => {
                crate::storage::KeyringService::set_secret(service, account, secret).map_err(PluginError)?;
                let _ = self.app()?.db.record_keyring_owner(service, account, &owner);
                Ok(())
            }
            Verdict::Deny(m) => Err(PluginError(m)),
            Verdict::NotFound => Err(PluginError("not allowed".to_string())),
        }
    }

    fn keyring_shared_delete(&self, service: &str, account: &str) -> Result<bool, PluginError> {
        use crate::api::keyring_policy::{Op, Verdict};
        match self.shared_verdict(Op::Delete, service, account)? {
            Verdict::Allow { .. } => {
                let app = self.app()?;
                let deleted = crate::storage::KeyringService::delete_secret(service, account).map_err(PluginError)?;
                let _ = app.db.delete_keyring_metadata(service, account);
                Ok(deleted)
            }
            Verdict::Deny(m) => Err(PluginError(m)),
            Verdict::NotFound => Err(PluginError("not allowed".to_string())),
        }
    }

    fn log(&self, level: u8, msg: &str) {
        let msg: String = msg.chars().take(2000).collect();
        match level {
            0 => tracing::error!(plugin = %self.plugin_id, "{msg}"),
            1 => tracing::warn!(plugin = %self.plugin_id, "{msg}"),
            2 => tracing::info!(plugin = %self.plugin_id, "{msg}"),
            _ => tracing::debug!(plugin = %self.plugin_id, "{msg}"),
        }
    }
}

/// Runs queued plugin commands. A plugin's grant is checked again here, so a revoke between
/// queueing and running wins, and the other plugins' `on_command` veto still applies.
pub async fn drain_plugin_commands(mut rx: tokio::sync::mpsc::Receiver<QueuedPluginCommand>, app: AppSlot) {
    while let Some(msg) = rx.recv().await {
        let Some(app) = app.get() else { continue };
        if !app.plugin_manager.command_allowed(&msg.plugin_id, &msg.variant) {
            tracing::warn!("dropped queued '{}' from plugin '{}': no longer granted", msg.variant, msg.plugin_id);
            continue;
        }
        tracing::info!("running '{}' from plugin '{}'", msg.variant, msg.plugin_id);
        // Same path as REST and WebSocket: other plugins' veto still applies.
        let origin = crate::api::command_pipeline::CommandOrigin::Plugin { id: msg.plugin_id.clone() };
        let value = serde_json::to_value(&msg.cmd).unwrap_or(serde_json::Value::Null);
        if crate::api::command_pipeline::run_command_pipeline(app, value, &origin).is_err() {
            tracing::warn!("command '{}' from plugin '{}' was not run", msg.variant, msg.plugin_id);
        }
    }
}
