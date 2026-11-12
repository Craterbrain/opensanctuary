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
mod licensed;

pub use live::{effective_arrangement, resolve_slide_at, resolve_background_at, reconcile_arrangement};
pub use schedule_ops::slot_id_base;

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

/// The engine state a replay starts from instead of event #1: stored by
/// `Database::compact_event_log`, restored by `ShowEngine::restore_from_snapshot`.
pub struct EngineSnapshot {
    /// The last event already folded into `state`/`schedule`.
    pub sequence_number: u64,
    pub state: ShowState,
    pub schedule: Schedule,
}

/// Where an Undo/Redo replay starts: the state and schedule as they stood after every event the
/// event log's bounded history no longer holds. `sequence_number` is the last of those events (0
/// while nothing has been dropped), and so the earliest point a replay can reach.
#[derive(Default)]
struct ReplayBase {
    state: ShowState,
    schedule: Schedule,
    sequence_number: u64,
}

pub struct ShowEngine {
    state: RwLock<ShowState>,
    schedule: RwLock<Schedule>,
    event_log: Arc<EventLog>,
    /// Never held while taking the `state`/`schedule` locks (`plan_history_events` takes it the
    /// other way round, under their read guards).
    replay_base: RwLock<ReplayBase>,
    undo_seq_stack: RwLock<Vec<u64>>,
    redo_seq_stack: RwLock<Vec<u64>>,
}

impl ShowEngine {
    pub fn new(event_log: Arc<EventLog>) -> Self {
        Self {
            state: RwLock::new(ShowState::default()),
            schedule: RwLock::new(Schedule::default()),
            event_log,
            replay_base: RwLock::new(ReplayBase::default()),
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

        if !matches!(cmd, ShowCommand::Undo | ShowCommand::Redo | ShowCommand::UndoRemoval { .. }) {
            self.undo_seq_stack.write().unwrap_or_else(|e| e.into_inner()).push(prev_seq);
            self.redo_seq_stack.write().unwrap_or_else(|e| e.into_inner()).clear();
        }

        let mut envelopes = Vec::new();

        for event in events {
            // 1. Append to event log and generate sequence envelope. An event the full log drops to
            // make room moves into the replay base, under its lock so drops are folded in order.
            let envelope = {
                let mut base = self.replay_base.write().unwrap_or_else(|e| e.into_inner());
                let (envelope, evicted) = self.event_log.append_evicting(event);
                if let Some(old) = evicted {
                    let base = &mut *base;
                    Self::apply_event(&mut base.state, &mut base.schedule, &old.event);
                    base.sequence_number = old.sequence_number;
                }
                envelope
            };

            // 2. Mutate state & schedule via deterministic reducer + update state sequence metadata in a single write lock
            {
                // A reducer bug that panicked here would leave the state half changed, out of step with the
                // event log that already holds the event. Release builds unwind (so a bug elsewhere fails
                // only its request), so this one place still stops the process: a restart replays a log
                // that agrees with itself, where carrying on would not.
                struct AbortIfPanicking;
                impl Drop for AbortIfPanicking {
                    fn drop(&mut self) {
                        if std::thread::panicking() {
                            eprintln!("the show engine failed while applying an event; stopping so a restart can replay a consistent log");
                            std::process::abort();
                        }
                    }
                }
                let _consistent = AbortIfPanicking;
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
            | ShowCommand::AddToScheduleAndGoLive(_)
            | ShowCommand::ReplaceScheduleItem { .. }
            | ShowCommand::ReplaceScheduledCopies { .. }
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

            ShowCommand::SetLicensedPassage { .. } => licensed::plan_licensed_events(&schedule, cmd),

            ShowCommand::SetItemTheme { .. }
            | ShowCommand::SetSlideBackground { .. }
            | ShowCommand::SetSlideDuration { .. }
            | ShowCommand::SetSlideHidden { .. }
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

            ShowCommand::Undo | ShowCommand::Redo | ShowCommand::UndoRemoval { .. } => self.plan_history_events(&state, cmd),
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
                | ShowEvent::SelectionSet { .. }
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
            | ShowEvent::ScheduleItemReplaced { .. }
            | ShowEvent::ScheduleItemRemoved { .. }
            | ShowEvent::ScheduleItemsMerged { .. }
            | ShowEvent::ScheduleReordered { .. }
            | ShowEvent::ScheduleSlidesReordered { .. }
            | ShowEvent::ScheduleSlideDuplicated { .. }
            | ShowEvent::ScheduleSlideRemoved { .. }
            | ShowEvent::ArrangementSet { .. }
            | ShowEvent::ScheduleLoaded { .. }
            | ShowEvent::ScheduleCleared
            | ShowEvent::ScheduleTitleSet { .. }
            | ShowEvent::SelectionSet { .. } => schedule_ops::apply_schedule_event(state, schedule, event),

            ShowEvent::LicensedPassageSet { .. } => licensed::apply_licensed_event(state, schedule, event),

            ShowEvent::ScheduleItemExpandToggled { .. }
            | ShowEvent::ScheduleItemThemeSet { .. }
            | ShowEvent::SlideBackgroundSet { .. }
            | ShowEvent::SlideDurationSet { .. }
            | ShowEvent::SlideHiddenSet { .. }
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
        // Whatever changed the items (a slide hidden, slides reordered, an arrangement set, a schedule
        // loaded), the live and staged copies never rest on a hidden slide.
        if touches_schedule {
            live::keep_off_hidden(state);
        }
    }

    pub fn restore_from_events(&self, events: Vec<crate::core::events::EventEnvelope>) {
        self.restore_from_snapshot(None, events);
    }

    /// Restores the engine to `snapshot` (the state after its event `sequence_number`, or a clean
    /// engine) followed by `events`, the ones after it.
    pub fn restore_from_snapshot(&self, snapshot: Option<EngineSnapshot>, events: Vec<crate::core::events::EventEnvelope>) {
        if snapshot.is_none() && events.is_empty() {
            return;
        }
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        let mut schedule = self.schedule.write().unwrap_or_else(|e| e.into_inner());
        if let Some(snapshot) = snapshot {
            *state = snapshot.state.clone();
            *schedule = snapshot.schedule.clone();
            state.sequence_number = snapshot.sequence_number;
            *self.replay_base.write().unwrap_or_else(|e| e.into_inner()) = ReplayBase {
                state: snapshot.state,
                schedule: snapshot.schedule,
                sequence_number: snapshot.sequence_number,
            };
        }
        // The log keeps only the newest `capacity` of these; the replay base is everything before.
        let kept_from = events.len().saturating_sub(self.event_log.capacity());
        for (i, env) in events.iter().enumerate() {
            if i == kept_from && i > 0 {
                *self.replay_base.write().unwrap_or_else(|e| e.into_inner()) = ReplayBase {
                    state: state.clone(),
                    schedule: schedule.clone(),
                    sequence_number: events[i - 1].sequence_number,
                };
            }
            Self::apply_event(&mut state, &mut schedule, &env.event);
            state.sequence_number = env.sequence_number;
            state.last_event_id = env.event_id.clone();
            state.server_time_ms = env.timestamp_ms;
        }
        self.event_log.load_history(&events);
    }

    /// Where the earliest possible Undo replay starts: everything before the events the log still holds.
    /// What `Database::compact_event_log` stores, so the next start needn't replay those events again.
    pub fn replay_base(&self) -> EngineSnapshot {
        let base = self.replay_base.read().unwrap_or_else(|e| e.into_inner());
        EngineSnapshot { sequence_number: base.sequence_number, state: base.state.clone(), schedule: base.schedule.clone() }
    }

    /// The state and schedule as they stood right after event `target`, rebuilt from the replay base
    /// and the events still in the log. `None` when `target` is older than the log reaches.
    fn replay_to(&self, target: u64) -> Option<(ShowState, Schedule)> {
        let base = self.replay_base.read().unwrap_or_else(|e| e.into_inner());
        if target < base.sequence_number {
            return None;
        }
        let mut state = base.state.clone();
        let mut schedule = base.schedule.clone();
        for env in self.event_log.all_events() {
            if env.sequence_number <= target {
                Self::apply_event(&mut state, &mut schedule, &env.event);
            }
        }
        Some((state, schedule))
    }
}
