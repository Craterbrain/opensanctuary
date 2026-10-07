use std::sync::{Arc, RwLock};
use crate::core::commands::ShowCommand;
use crate::core::events::{EventEnvelope, ShowEvent};
use crate::core::event_log::EventLog;
use crate::core::models::{Schedule, ShowState};
use serde::{Deserialize, Serialize};

mod live;
mod overlays;
mod schedule_ops;
mod themes;
mod media;
mod elements;
mod history;

pub use live::{effective_arrangement, resolve_slide_at, resolve_background_at, reconcile_arrangement};

pub const CURRENT_PROTOCOL_VERSION: u32 = 2;

fn default_protocol_version() -> u32 {
    CURRENT_PROTOCOL_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    #[serde(default = "default_protocol_version")]
    pub protocol_version: u32,
    pub sequence_number: u64,
    pub timestamp_ms: u64,
    pub state: ShowState,
    pub schedule: Schedule,
}

pub struct ShowEngine {
    state: RwLock<ShowState>,
    schedule: RwLock<Schedule>,
    event_log: Arc<EventLog>,
    undo_seq_stack: RwLock<Vec<u64>>,
    redo_seq_stack: RwLock<Vec<u64>>,
}

impl ShowEngine {
    pub fn new(event_log: Arc<EventLog>) -> Self {
        Self {
            state: RwLock::new(ShowState::default()),
            schedule: RwLock::new(Schedule::default()),
            event_log,
            undo_seq_stack: RwLock::new(Vec::new()),
            redo_seq_stack: RwLock::new(Vec::new()),
        }
    }

    pub fn snapshot(&self) -> StateSnapshot {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner()).clone();
        let schedule = self.schedule.read().unwrap_or_else(|e| e.into_inner()).clone();
        StateSnapshot {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            sequence_number: state.sequence_number,
            timestamp_ms: state.server_time_ms,
            state,
            schedule,
        }
    }

    pub fn execute_command(&self, cmd: ShowCommand) -> Vec<EventEnvelope> {
        let prev_seq = self.state.read().unwrap_or_else(|e| e.into_inner()).sequence_number;
        let events = self.plan_events(&cmd);
        if events.is_empty() {
            return Vec::new();
        }

        if !matches!(cmd, ShowCommand::Undo | ShowCommand::Redo) {
            self.undo_seq_stack.write().unwrap_or_else(|e| e.into_inner()).push(prev_seq);
            self.redo_seq_stack.write().unwrap_or_else(|e| e.into_inner()).clear();
        }

        let mut envelopes = Vec::new();

        for event in events {
            // 1. Append to event log and generate sequence envelope
            let envelope = self.event_log.append(event);

            // 2. Mutate state & schedule via deterministic reducer + update state sequence metadata in a single write lock
            {
                let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
                let mut schedule = self.schedule.write().unwrap_or_else(|e| e.into_inner());
                Self::apply_event(&mut state, &mut schedule, &envelope.event);
                state.sequence_number = envelope.sequence_number;
                state.last_event_id = envelope.event_id.clone();
                state.server_time_ms = envelope.timestamp_ms;
            }

            envelopes.push(envelope);
        }

        envelopes
    }

    /// Dispatches each `ShowCommand` family to its own module's `plan_x_events`.
    /// `Undo`/`Redo` are the one family that needs more than `state`/`schedule` (see
    /// `history::plan_history_events`), so they go through a `ShowEngine` method instead.
    fn plan_events(&self, cmd: &ShowCommand) -> Vec<ShowEvent> {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        let schedule = self.schedule.read().unwrap_or_else(|e| e.into_inner());

        match cmd {
            ShowCommand::GoLive { .. }
            | ShowCommand::StageItem { .. }
            | ShowCommand::StageItemDirect(_)
            | ShowCommand::NextSlide
            | ShowCommand::PrevSlide
            | ShowCommand::NextItem
            | ShowCommand::PrevItem
            | ShowCommand::JumpSlide(_)
            | ShowCommand::JumpSection(_) => live::plan_live_events(&state, &schedule, cmd),

            ShowCommand::ToggleBlackout
            | ShowCommand::ToggleClearText
            | ShowCommand::ToggleLogo
            | ShowCommand::SetAlert(_)
            | ShowCommand::SetWebStream(_)
            | ShowCommand::SetTransition(_) => overlays::plan_overlay_events(&state, &schedule, cmd),

            ShowCommand::AddToSchedule(_)
            | ShowCommand::AddScheduleHeader { .. }
            | ShowCommand::RemoveFromSchedule(_)
            | ShowCommand::ReorderSchedule { .. }
            | ShowCommand::ReorderItemSlides { .. }
            | ShowCommand::DuplicateItemSlide { .. }
            | ShowCommand::RemoveItemSlide { .. }
            | ShowCommand::MergeScheduleItems { .. }
            | ShowCommand::SetArrangement { .. }
            | ShowCommand::ToggleScheduleItemExpand(_)
            | ShowCommand::SetScheduleTitle(_)
            | ShowCommand::NewSchedule
            | ShowCommand::LoadSchedule(_) => schedule_ops::plan_schedule_events(&state, &schedule, cmd),

            ShowCommand::SetItemTheme { .. }
            | ShowCommand::SetSlideBackground { .. }
            | ShowCommand::SetSlideDuration { .. }
            | ShowCommand::SetPresentationDefaultDuration { .. }
            | ShowCommand::SetPresentationLoop { .. }
            | ShowCommand::SetGlobalTheme { .. } => themes::plan_theme_events(&state, &schedule, cmd),

            ShowCommand::MediaScheduledStart { .. }
            | ShowCommand::MediaPause { .. }
            | ShowCommand::MediaSetLoop(_)
            | ShowCommand::MediaSetMute(_)
            | ShowCommand::MediaSetVolume(_)
            | ShowCommand::MediaPreroll { .. } => media::plan_media_events(&state, &schedule, cmd),

            ShowCommand::AddSlideElement { .. }
            | ShowCommand::RemoveSlideElement { .. }
            | ShowCommand::UpdateSlideElementTransform { .. }
            | ShowCommand::UpdateTextBlockContent { .. }
            | ShowCommand::UpdateElementEffects { .. }
            | ShowCommand::ReorderSlideElements { .. }
            | ShowCommand::GroupSlideElements { .. }
            | ShowCommand::UngroupSlideElements { .. }
            | ShowCommand::SetSlideBackgroundV2 { .. }
            | ShowCommand::SetSlideTransition { .. }
            | ShowCommand::SetSlideSpeakerNotes { .. }
            | ShowCommand::SetSlideCcliMetadata { .. }
            | ShowCommand::BatchSlideEdit { .. } => elements::plan_element_events(&state, &schedule, cmd),

            ShowCommand::Undo | ShowCommand::Redo => self.plan_history_events(&state, cmd),
        }
    }

    /// Pure deterministic reducer for State and Schedule — dispatches each `ShowEvent`
    /// family to its own module's `apply_x_event`.
    pub fn apply_event(state: &mut ShowState, schedule: &mut Schedule, event: &ShowEvent) {
        // Bump `schedule.schedule_version` for every event EXCEPT the known ShowState-only
        // ones below, rather than incrementing inside each schedule-mutating match arm —
        // inverting the check this way means a future event variant defaults to "assume it
        // touched the schedule" (an occasional unnecessary re-send) instead of silently
        // going stale if someone forgets to add an increment to a new arm. See the
        // `schedule_version` field doc comment (src/core/models.rs) and src/api/ws.rs for
        // why this exists: it lets the WebSocket broadcast skip re-sending the full
        // schedule on state-only changes (blackout, live navigation, alerts, media
        // transport), none of which mutate `schedule` — `state.live_item`/`staged_item` are
        // already denormalized copies, not references into it.
        let touches_schedule = !matches!(
            event,
            ShowEvent::ItemWentLive { .. }
                | ShowEvent::LiveSlideChanged { .. }
                | ShowEvent::ItemStaged { .. }
                | ShowEvent::StagedSlideChanged { .. }
                | ShowEvent::BlackoutToggled { .. }
                | ShowEvent::ClearTextToggled { .. }
                | ShowEvent::LogoToggled { .. }
                | ShowEvent::AlertChanged { .. }
                | ShowEvent::WebStreamChanged { .. }
                | ShowEvent::TransitionChanged { .. }
                | ShowEvent::GlobalThemeSet { .. }
                | ShowEvent::MediaScheduledStart { .. }
                | ShowEvent::MediaPaused { .. }
                | ShowEvent::MediaLoopSet { .. }
                | ShowEvent::MediaMuteSet { .. }
                | ShowEvent::MediaVolumeSet { .. }
                | ShowEvent::MediaPrerolled { .. }
        );
        if touches_schedule {
            schedule.schedule_version = schedule.schedule_version.wrapping_add(1);
        }

        match event {
            ShowEvent::ItemWentLive { .. }
            | ShowEvent::LiveSlideChanged { .. }
            | ShowEvent::ItemStaged { .. }
            | ShowEvent::StagedSlideChanged { .. } => live::apply_live_event(state, schedule, event),

            ShowEvent::BlackoutToggled { .. }
            | ShowEvent::ClearTextToggled { .. }
            | ShowEvent::LogoToggled { .. }
            | ShowEvent::AlertChanged { .. }
            | ShowEvent::WebStreamChanged { .. }
            | ShowEvent::TransitionChanged { .. } => overlays::apply_overlay_event(state, schedule, event),

            ShowEvent::ScheduleItemAdded { .. }
            | ShowEvent::ScheduleItemRemoved { .. }
            | ShowEvent::ScheduleItemsMerged { .. }
            | ShowEvent::ScheduleReordered { .. }
            | ShowEvent::ScheduleSlidesReordered { .. }
            | ShowEvent::ScheduleSlideDuplicated { .. }
            | ShowEvent::ScheduleSlideRemoved { .. }
            | ShowEvent::ArrangementSet { .. }
            | ShowEvent::ScheduleLoaded { .. }
            | ShowEvent::ScheduleCleared
            | ShowEvent::ScheduleTitleSet { .. } => schedule_ops::apply_schedule_event(state, schedule, event),

            ShowEvent::ScheduleItemExpandToggled { .. }
            | ShowEvent::ScheduleItemThemeSet { .. }
            | ShowEvent::SlideBackgroundSet { .. }
            | ShowEvent::SlideDurationSet { .. }
            | ShowEvent::PresentationDefaultDurationSet { .. }
            | ShowEvent::PresentationLoopSet { .. }
            | ShowEvent::GlobalThemeSet { .. } => themes::apply_theme_event(state, schedule, event),

            ShowEvent::SlideElementAdded { .. }
            | ShowEvent::SlideElementRemoved { .. }
            | ShowEvent::SlideElementTransformUpdated { .. }
            | ShowEvent::TextBlockContentUpdated { .. }
            | ShowEvent::ElementEffectsUpdated { .. }
            | ShowEvent::SlideElementsReordered { .. }
            | ShowEvent::SlideElementsGrouped { .. }
            | ShowEvent::SlideElementsUngrouped { .. }
            | ShowEvent::SlideBackgroundV2Set { .. }
            | ShowEvent::SlideTransitionSet { .. }
            | ShowEvent::SlideSpeakerNotesSet { .. }
            | ShowEvent::SlideCcliMetadataSet { .. }
            | ShowEvent::SlideEditedBatch { .. } => elements::apply_element_event(state, schedule, event),

            ShowEvent::MediaScheduledStart { .. }
            | ShowEvent::MediaPaused { .. }
            | ShowEvent::MediaLoopSet { .. }
            | ShowEvent::MediaMuteSet { .. }
            | ShowEvent::MediaVolumeSet { .. }
            | ShowEvent::MediaPrerolled { .. } => media::apply_media_event(state, schedule, event),
        }
    }

    pub fn restore_from_events(&self, events: Vec<crate::core::events::EventEnvelope>) {
        if events.is_empty() {
            return;
        }
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        let mut schedule = self.schedule.write().unwrap_or_else(|e| e.into_inner());
        for env in &events {
            Self::apply_event(&mut state, &mut schedule, &env.event);
            state.sequence_number = env.sequence_number;
            state.last_event_id = env.event_id.clone();
            state.server_time_ms = env.timestamp_ms;
        }
        self.event_log.load_history(&events);
    }
}
