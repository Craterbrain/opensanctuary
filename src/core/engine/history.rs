use crate::core::commands::ShowCommand;
use crate::core::events::ShowEvent;
use crate::core::models::{ScheduleItem, ShowState};

use super::ShowEngine;

impl ShowEngine {
    /// Undo/Redo both need `self.event_log`/`undo_seq_stack`/`redo_seq_stack`, unlike
    /// every other command family, so this stays a `ShowEngine` method rather than a
    /// free function. `state` is the read-lock guard the caller (`plan_events`) already
    /// holds — it's passed in rather than re-acquired here to keep locking behavior
    /// identical to before the split (no extra lock, no risk of deadlock).
    pub(super) fn plan_history_events(&self, state: &ShowState, cmd: &ShowCommand) -> Vec<ShowEvent> {
        // Undo steps back to the top of the undo stack and leaves where it came from on the redo
        // stack; Redo is the same move with the stacks swapped.
        if let ShowCommand::UndoRemoval { item_id } = cmd {
            if !self.last_change_removed(state, item_id) {
                return Vec::new();
            }
        }
        let (from, onto) = match cmd {
            ShowCommand::Undo | ShowCommand::UndoRemoval { .. } => (&self.undo_seq_stack, &self.redo_seq_stack),
            ShowCommand::Redo => (&self.redo_seq_stack, &self.undo_seq_stack),
            _ => unreachable!("plan_history_events called with a non-history command"),
        };
        let Some(target) = from.write().unwrap_or_else(|e| e.into_inner()).pop() else {
            return Vec::new();
        };
        let Some((replay_state, mut replay_schedule)) = self.replay_to(target) else {
            // Older than the log can rebuild -- and whatever is under it on the stack is older still.
            from.write().unwrap_or_else(|e| e.into_inner()).clear();
            return Vec::new();
        };
        onto.write().unwrap_or_else(|e| e.into_inner()).push(state.sequence_number);

        // `ScheduleLoaded` first: the CCLI usage report tells a replayed `ItemWentLive` from a real
        // one by the replay `ScheduleLoaded` right before it.
        let position = |item: &Option<ScheduleItem>| {
            item.as_ref().and_then(|i| replay_schedule.items.iter().position(|s| s.id == i.id))
        };
        let (live_pos, staged_pos) = (position(&replay_state.live_item), position(&replay_state.staged_item));
        replay_schedule.selected_item_index = replay_state.selected_item_index;
        let mut events = vec![ShowEvent::ScheduleLoaded { schedule: replay_schedule, replay: true }];
        // Where the events below leave the selection: each new live/staged item moves it to its own slot.
        let mut selection = replay_state.selected_item_index;
        // The live and staged items are touched only when the step actually changes them: a new
        // `ItemWentLive` drops the media playback state, which would restart a playing video for an
        // undo that had nothing to do with it.
        if replay_state.live_item != state.live_item {
            selection = live_pos.or(selection);
            events.push(ShowEvent::ItemWentLive { item: replay_state.live_item, slide_index: replay_state.live_slide_index });
        } else if replay_state.live_slide_index != state.live_slide_index {
            events.push(ShowEvent::LiveSlideChanged { slide_index: replay_state.live_slide_index });
        }
        if replay_state.staged_item != state.staged_item {
            selection = staged_pos.or(selection);
            events.push(ShowEvent::ItemStaged { item: replay_state.staged_item, slide_index: replay_state.staged_slide_index });
        } else if replay_state.staged_slide_index != state.staged_slide_index {
            events.push(ShowEvent::StagedSlideChanged { slide_index: replay_state.staged_slide_index });
        }
        if selection != replay_state.selected_item_index {
            events.push(ShowEvent::SelectionSet { index: replay_state.selected_item_index });
        }
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
        events
    }

    /// Whether the most recent change was one command whose only event removed schedule item `item_id`.
    fn last_change_removed(&self, state: &ShowState, item_id: &str) -> bool {
        let Some(&before) = self.undo_seq_stack.read().unwrap_or_else(|e| e.into_inner()).last() else { return false };
        let since = self.event_log.get_events_since(before);
        let [only] = since.as_slice() else { return false };
        let ShowEvent::ScheduleItemRemoved { index } = only.event else { return false };
        only.sequence_number == state.sequence_number
            && self.replay_to(before).is_some_and(|(_, schedule)| schedule.items.get(index).is_some_and(|i| i.id == item_id))
    }
}
