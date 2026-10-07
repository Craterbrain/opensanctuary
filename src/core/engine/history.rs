use crate::core::commands::ShowCommand;
use crate::core::events::ShowEvent;
use crate::core::models::{Schedule, ShowState};

use super::ShowEngine;

impl ShowEngine {
    /// Undo/Redo both need `self.event_log`/`undo_seq_stack`/`redo_seq_stack`, unlike
    /// every other command family, so this stays a `ShowEngine` method rather than a
    /// free function. `state` is the read-lock guard the caller (`plan_events`) already
    /// holds — it's passed in rather than re-acquired here to keep locking behavior
    /// identical to before the split (no extra lock, no risk of deadlock).
    pub(super) fn plan_history_events(&self, state: &ShowState, cmd: &ShowCommand) -> Vec<ShowEvent> {
        let mut events = Vec::new();
        match cmd {
            ShowCommand::Undo => {
                let target_opt = self.undo_seq_stack.write().unwrap_or_else(|e| e.into_inner()).pop();
                if let Some(target) = target_opt {
                    self.redo_seq_stack.write().unwrap_or_else(|e| e.into_inner()).push(state.sequence_number);
                    let mut replay_state = ShowState::default();
                    let mut replay_schedule = Schedule::default();
                    let all_events = self.event_log.all_events();
                    for env in all_events {
                        if env.sequence_number <= target {
                            Self::apply_event(&mut replay_state, &mut replay_schedule, &env.event);
                        }
                    }

                    events.push(ShowEvent::ScheduleLoaded { schedule: replay_schedule });
                    events.push(ShowEvent::ItemWentLive { item: replay_state.live_item, slide_index: replay_state.live_slide_index });
                    events.push(ShowEvent::ItemStaged { item: replay_state.staged_item, slide_index: replay_state.staged_slide_index });
                    if replay_state.is_blackout != state.is_blackout {
                        events.push(ShowEvent::BlackoutToggled { is_blackout: replay_state.is_blackout });
                    }
                    if replay_state.is_clear_text != state.is_clear_text {
                        events.push(ShowEvent::ClearTextToggled { is_clear_text: replay_state.is_clear_text });
                    }
                    if replay_state.is_logo != state.is_logo {
                        events.push(ShowEvent::LogoToggled { is_logo: replay_state.is_logo });
                    }
                    if replay_state.alert_text != state.alert_text {
                        events.push(ShowEvent::AlertChanged { alert: replay_state.alert_text });
                    }
                    if replay_state.web_stream != state.web_stream {
                        events.push(ShowEvent::WebStreamChanged { web_stream: replay_state.web_stream.clone() });
                    }
                    if replay_state.transition != state.transition {
                        events.push(ShowEvent::TransitionChanged { transition: replay_state.transition.clone() });
                    }
                    if replay_state.global_theme != state.global_theme || replay_state.global_background != state.global_background {
                        if let (Some(theme), Some(bg)) = (replay_state.global_theme, replay_state.global_background) {
                            events.push(ShowEvent::GlobalThemeSet { theme_name: theme, background: bg });
                        }
                    }
                }
            },
            ShowCommand::Redo => {
                let target_opt = self.redo_seq_stack.write().unwrap_or_else(|e| e.into_inner()).pop();
                if let Some(target) = target_opt {
                    self.undo_seq_stack.write().unwrap_or_else(|e| e.into_inner()).push(state.sequence_number);
                    let mut replay_state = ShowState::default();
                    let mut replay_schedule = Schedule::default();
                    let all_events = self.event_log.all_events();
                    for env in all_events {
                        if env.sequence_number <= target {
                            Self::apply_event(&mut replay_state, &mut replay_schedule, &env.event);
                        }
                    }

                    events.push(ShowEvent::ScheduleLoaded { schedule: replay_schedule });
                    events.push(ShowEvent::ItemWentLive { item: replay_state.live_item, slide_index: replay_state.live_slide_index });
                    events.push(ShowEvent::ItemStaged { item: replay_state.staged_item, slide_index: replay_state.staged_slide_index });
                    if replay_state.is_blackout != state.is_blackout {
                        events.push(ShowEvent::BlackoutToggled { is_blackout: replay_state.is_blackout });
                    }
                    if replay_state.is_clear_text != state.is_clear_text {
                        events.push(ShowEvent::ClearTextToggled { is_clear_text: replay_state.is_clear_text });
                    }
                    if replay_state.is_logo != state.is_logo {
                        events.push(ShowEvent::LogoToggled { is_logo: replay_state.is_logo });
                    }
                    if replay_state.alert_text != state.alert_text {
                        events.push(ShowEvent::AlertChanged { alert: replay_state.alert_text });
                    }
                    if replay_state.web_stream != state.web_stream {
                        events.push(ShowEvent::WebStreamChanged { web_stream: replay_state.web_stream.clone() });
                    }
                    if replay_state.transition != state.transition {
                        events.push(ShowEvent::TransitionChanged { transition: replay_state.transition.clone() });
                    }
                    if replay_state.global_theme != state.global_theme || replay_state.global_background != state.global_background {
                        if let (Some(theme), Some(bg)) = (replay_state.global_theme, replay_state.global_background) {
                            events.push(ShowEvent::GlobalThemeSet { theme_name: theme, background: bg });
                        }
                    }
                }
            },
            _ => unreachable!("plan_history_events called with a non-history command"),
        }
        events
    }
}
