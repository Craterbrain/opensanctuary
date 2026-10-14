//! The one path every show command takes, whatever door it came through (REST `/api/command`,
//! the `/ws` envelope, or a sandboxed plugin's `sendCommand`).
//!
//! Two steps, kept apart because the doors differ in *how* they carry a credential but not in
//! what happens afterwards:
//!
//! 1. [`authenticate_command`]: who is this, and may they send this command? (plugin token,
//!    paired remote, or console + the one-console-at-a-time lock).
//! 2. [`run_command_pipeline`]: expand the frontend's shorthand, parse, execute, persist the
//!    events and broadcast the new state. A rejection looks the same from every door.

use serde_json::Value;

use crate::api::ws::{broadcast_snapshot, AppState};
use crate::core::commands::ShowCommand;
use crate::core::engine::StateSnapshot;
use crate::core::events::EventEnvelope;
use crate::core::models::ToScheduleItem;

/// Who a command is being run on behalf of (logging, and what the caller may do next).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOrigin {
    Console { session_id: String },
    Device { id: String },
    Plugin { id: String },
}

/// The credential a door extracted from its own transport (headers or a WebSocket envelope).
pub enum CommandCredential<'a> {
    Plugin { token: &'a str },
    Device { token: &'a str },
    Host { token: &'a str, session_id: &'a str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAuthError {
    /// A plugin token that is invalid/expired/revoked, or a command outside its scope.
    PluginDenied(String),
    /// A paired display (as opposed to a remote): displays never send show commands.
    DisplayDevice,
    UnknownDevice,
    BadHostToken,
    MissingSession,
    /// A different console already holds the one-console-at-a-time lock.
    ConsoleLocked,
}

impl CommandAuthError {
    /// The `error` a REST caller is given. `None` for an ordinary credential failure, so it
    /// isn't mistaken for something the caller could fix.
    pub fn reason(&self) -> Option<String> {
        match self {
            CommandAuthError::PluginDenied(r) => Some(r.clone()),
            CommandAuthError::DisplayDevice => Some("display_token_cannot_control".to_string()),
            CommandAuthError::UnknownDevice => None,
            CommandAuthError::BadHostToken => Some("Missing or invalid x-host-token".to_string()),
            CommandAuthError::MissingSession => Some("Missing x-console-session-id".to_string()),
            CommandAuthError::ConsoleLocked => Some("console_locked".to_string()),
        }
    }

    pub fn log_line(&self) -> String {
        match self {
            CommandAuthError::PluginDenied(r) => r.clone(),
            CommandAuthError::DisplayDevice => "paired display tokens cannot send show commands".to_string(),
            CommandAuthError::UnknownDevice => "invalid/unknown device token".to_string(),
            CommandAuthError::BadHostToken => "missing/invalid host token".to_string(),
            CommandAuthError::MissingSession => "missing console session id".to_string(),
            CommandAuthError::ConsoleLocked => "another console is already connected".to_string(),
        }
    }
}

/// Checks a credential. `cmd` is the bare command (needed to check a plugin's scope against
/// *which* command it is sending). Trims tokens; never logs them.
pub async fn authenticate_command(
    app: &AppState,
    credential: &CommandCredential<'_>,
    cmd: &Value,
) -> Result<CommandOrigin, CommandAuthError> {
    match credential {
        CommandCredential::Plugin { token } => crate::api::routes::plugins::authorize_plugin_command(app, token.trim(), cmd)
            .await
            .map(|id| CommandOrigin::Plugin { id })
            .map_err(CommandAuthError::PluginDenied),
        CommandCredential::Device { token } => match app.db.get_paired_device_by_token(token.trim()) {
            Ok(Some(dev)) if dev.can_control_show() => {
                let _ = app.db.touch_paired_device(&dev.id);
                Ok(CommandOrigin::Device { id: dev.id })
            }
            Ok(Some(_)) => Err(CommandAuthError::DisplayDevice),
            _ => Err(CommandAuthError::UnknownDevice),
        },
        CommandCredential::Host { token, session_id } => {
            if !app.is_console_credential(token) {
                return Err(CommandAuthError::BadHostToken);
            }
            let session_id = session_id.trim();
            if session_id.is_empty() {
                return Err(CommandAuthError::MissingSession);
            }
            app.try_claim_console(session_id).map_err(|_| CommandAuthError::ConsoleLocked)?;
            Ok(CommandOrigin::Console { session_id: session_id.to_string() })
        }
    }
}

#[derive(Debug)]
pub enum CommandRejection {
    /// The payload isn't a `ShowCommand`.
    Unparseable(String),
}

pub struct Executed {
    pub state: StateSnapshot,
    pub events: Vec<EventEnvelope>,
}

fn item_for(app: &AppState, item_type: &str, item_id: &str) -> Option<crate::core::models::ScheduleItem> {
    match item_type {
        "song" => app.db.get_song_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
        "scripture" => app.db.get_scripture_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
        "media" => app.db.get_media_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
        "presentation" => app.db.get_presentation_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
        _ => None,
    }
}

/// Turns the frontend's shorthand into real commands:
/// `{"AddToSchedule":{"item_type","item_id"}}` becomes `AddToSchedule(<full item>)` (with the
/// default theme applied), and a Library Preview click `{"StageItem":{"item_type","item_id"}}`
/// -- distinct from the real `StageItem{item_index,slide_index}` -- becomes `StageItemDirect`.
/// Anything else, and a shorthand whose item can't be found, is returned unchanged.
pub fn expand_shorthand(app: &AppState, value: Value) -> Value {
    if let Some(add) = value.get("AddToSchedule") {
        if add.get("item_type").is_some() && add.get("item_id").is_some() {
            let (t, id) = (add["item_type"].as_str().unwrap_or(""), add["item_id"].as_str().unwrap_or(""));
            match item_for(app, t, id) {
                Some(mut item) => {
                    crate::api::routes::apply_default_theme_if_unset(&mut item, &app.db);
                    if let Ok(v) = serde_json::to_value(item) {
                        return serde_json::json!({ "AddToSchedule": v });
                    }
                }
                None => tracing::error!("Item not found in DB for AddToSchedule: {} {}", t, id),
            }
        }
    }
    if let Some(stage) = value.get("StageItem") {
        if stage.get("item_type").is_some() && stage.get("item_id").is_some() {
            let (t, id) = (stage["item_type"].as_str().unwrap_or(""), stage["item_id"].as_str().unwrap_or(""));
            match item_for(app, t, id) {
                Some(item) => {
                    if let Ok(v) = serde_json::to_value(item) {
                        return serde_json::json!({ "StageItemDirect": v });
                    }
                }
                None => tracing::error!("Item not found in DB for StageItem preview: {} {}", t, id),
            }
        }
    }
    value
}

/// Runs an already-authenticated command. `value` is the bare command (not an envelope).
pub fn run_command_pipeline(app: &AppState, value: Value, origin: &CommandOrigin) -> Result<Executed, CommandRejection> {
    let value = expand_shorthand(app, value);

    let cmd: ShowCommand = serde_json::from_value(value.clone()).map_err(|e| {
        tracing::warn!("command from {:?} failed to parse, dropped: {} (payload: {})", origin, e, value);
        CommandRejection::Unparseable(e.to_string())
    })?;

    let events = app.engine.execute_command(cmd);
    for env in &events {
        if let Err(e) = app.db.save_event(env) {
            tracing::warn!("Failed to persist event log entry #{}: {}", env.sequence_number, e);
        }
    }
    let state = broadcast_snapshot(app);
    Ok(Executed { state, events })
}
