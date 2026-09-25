use std::sync::{Arc, RwLock};
use std::borrow::Cow;
use crate::core::commands::ShowCommand;
use crate::core::events::{EventEnvelope, ShowEvent};
use crate::core::event_log::EventLog;
use crate::core::models::{ArrangementEntry, Schedule, ScheduleItem, ShowState, Slide};
use serde::{Deserialize, Serialize};

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

/// Resolves the effective arrangement for a schedule item.
/// If `item.arrangement` is not empty, borrows it.
/// If empty, generates an identity arrangement mapping each slide to its authored position.
pub fn effective_arrangement(item: &ScheduleItem) -> Cow<'_, [ArrangementEntry]> {
    if !item.arrangement.is_empty() {
        Cow::Borrowed(item.arrangement.as_slice())
    } else {
        Cow::Owned(crate::core::models::identity_arrangement(&item.slides))
    }
}

/// Resolves the master slide referenced at a given arrangement play position.
pub fn resolve_slide_at<'a>(
    item: &'a ScheduleItem,
    play_position: usize,
) -> Option<&'a Slide> {
    if !item.arrangement.is_empty() {
        let entry = item.arrangement.get(play_position)?;
        item.slides.get(entry.source_slide_index)
    } else {
        item.slides.get(play_position)
    }
}

/// Resolves the background for a given arrangement play position.
/// Returns `arrangement[pos].background_override` if Some, else `slides[source_slide_index].background`.
pub fn resolve_background_at<'a>(
    item: &'a ScheduleItem,
    play_position: usize,
) -> Option<&'a str> {
    if !item.arrangement.is_empty() {
        let entry = item.arrangement.get(play_position)?;
        if let Some(ref bg) = entry.background_override {
            Some(bg.as_str())
        } else {
            item.slides.get(entry.source_slide_index).and_then(|s| s.background.as_deref())
        }
    } else {
        item.slides.get(play_position).and_then(|s| s.background.as_deref())
    }
}

/// Helper to extract or derive a section ID for a slide.
pub fn slide_section_id(slide: &Slide, index: usize) -> String {
    slide
        .tag
        .as_ref()
        .or(slide.label.as_ref())
        .cloned()
        .unwrap_or_else(|| format!("S{}", index + 1))
}

/// Resolves an item's play-order slide sequence — each slide's content paired with
/// any per-position background override — in a single pass over its arrangement,
/// for flattening into an independent copy (e.g. when merging two items together).
fn resolved_play_sequence(item: &ScheduleItem) -> Vec<(Slide, Option<String>)> {
    effective_arrangement(item)
        .iter()
        .filter_map(|entry| {
            item.slides
                .get(entry.source_slide_index)
                .map(|slide| (slide.clone(), entry.background_override.clone()))
        })
        .collect()
}

/// Reconciles an item's arrangement against its master slides when `item.slides` is mutated.
///
/// Invariants:
/// - Existing arrangement entries are never reordered
/// - Entries pointing at deleted master slides are dropped
/// - New master slides get appended entries at the end
/// - Entries whose source_slide_index shifts due to a master insertion are remapped to the new index
/// - section_id matching is by string equality, not by index
pub fn reconcile_arrangement(item: &mut ScheduleItem) {
    if item.slides.is_empty() {
        item.arrangement.clear();
        return;
    }

    if item.arrangement.is_empty() {
        item.arrangement = crate::core::models::identity_arrangement(&item.slides);
        return;
    }

    // If no slides have explicit tags or labels, and arrangement has no custom section ids, maintain identity
    let has_explicit_tags = item.slides.iter().any(|s| s.tag.is_some() || s.label.is_some());
    let has_custom_sections = item.arrangement.iter().any(|e| !e.section_id.starts_with('S'));
    if !has_explicit_tags && !has_custom_sections {
        item.arrangement = crate::core::models::identity_arrangement(&item.slides);
        return;
    }

    // 1. Gather all master slides' section IDs and their indices
    // section_id matching is by string equality, not by index
    let master_sections: Vec<(usize, String)> = item
        .slides
        .iter()
        .enumerate()
        .map(|(i, s)| (i, slide_section_id(s, i)))
        .collect();

    // 2. Track which section IDs existed in the arrangement before reconciliation
    let original_section_ids: std::collections::HashSet<String> = item
        .arrangement
        .iter()
        .map(|e| e.section_id.clone())
        .collect();

    // 3. Retain existing entries, remapping source_slide_index or dropping if deleted
    // - Existing arrangement entries are never reordered
    // - Entries pointing at deleted master slides are dropped
    // - Entries whose source_slide_index shifts are remapped to the new index
    item.arrangement.retain_mut(|entry| {
        if let Some((new_idx, _)) = master_sections
            .iter()
            .find(|(_, sec_id)| sec_id == &entry.section_id)
        {
            entry.source_slide_index = *new_idx;
            true
        } else {
            false
        }
    });

    // 4. Append entries for new master slides
    // - New master slides get appended entries at the end
    for (idx, sec_id) in &master_sections {
        if !original_section_ids.contains(sec_id) {
            item.arrangement.push(ArrangementEntry {
                section_id: sec_id.clone(),
                source_slide_index: *idx,
                background_override: None,
            });
        }
    }

    // If all entries were deleted, fall back to identity arrangement
    if item.arrangement.is_empty() {
        item.arrangement = crate::core::models::identity_arrangement(&item.slides);
    }
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
        let state = self.state.read().unwrap().clone();
        let schedule = self.schedule.read().unwrap().clone();
        StateSnapshot {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            sequence_number: state.sequence_number,
            timestamp_ms: state.server_time_ms,
            state,
            schedule,
        }
    }

    pub fn execute_command(&self, cmd: ShowCommand) -> Vec<EventEnvelope> {
        let prev_seq = self.state.read().unwrap().sequence_number;
        let events = self.plan_events(&cmd);
        if events.is_empty() {
            return Vec::new();
        }

        if !matches!(cmd, ShowCommand::Undo | ShowCommand::Redo) {
            self.undo_seq_stack.write().unwrap().push(prev_seq);
            self.redo_seq_stack.write().unwrap().clear();
        }

        let mut envelopes = Vec::new();

        for event in events {
            // 1. Append to event log and generate sequence envelope
            let envelope = self.event_log.append(event);

            // 2. Mutate state & schedule via deterministic reducer + update state sequence metadata in a single write lock
            {
                let mut state = self.state.write().unwrap();
                let mut schedule = self.schedule.write().unwrap();
                Self::apply_event(&mut state, &mut schedule, &envelope.event);
                state.sequence_number = envelope.sequence_number;
                state.last_event_id = envelope.event_id.clone();
                state.server_time_ms = envelope.timestamp_ms;
            }

            envelopes.push(envelope);
        }

        envelopes
    }

    fn plan_events(&self, cmd: &ShowCommand) -> Vec<ShowEvent> {
        let state = self.state.read().unwrap();
        let schedule = self.schedule.read().unwrap();
        let mut events = Vec::new();

        match cmd {
            ShowCommand::GoLive { item_index, slide_index } => {
                let uses_staged_item = item_index.is_none() && state.staged_item.is_some();
                let target_item = match item_index {
                    Some(idx) => schedule.items.get(*idx).cloned(),
                    None => match &state.staged_item {
                        Some(item) => Some(item.clone()),
                        None => {
                            if let Some(sel_idx) = state.selected_item_index {
                                schedule.items.get(sel_idx).cloned()
                            } else {
                                state.live_item.clone()
                            }
                        }
                    },
                };
                // When no explicit slide was requested and we're sending the
                // staged item live, preserve whichever slide the operator had
                // staged (state.staged_slide_index) instead of resetting to 0 —
                // otherwise Enter/Page Down silently discards the operator's
                // staged position within the item.
                let slide_idx = slide_index.unwrap_or_else(|| {
                    if uses_staged_item { state.staged_slide_index } else { 0 }
                });
                events.push(ShowEvent::ItemWentLive {
                    item: target_item,
                    slide_index: slide_idx,
                });
            }
            ShowCommand::StageItem { item_index, slide_index } => {
                let slide_idx = slide_index.unwrap_or(0);
                match item_index {
                    Some(idx) => {
                        let target_item = schedule.items.get(*idx).cloned();
                        events.push(ShowEvent::ItemStaged {
                            item: target_item,
                            slide_index: slide_idx,
                        });
                    }
                    None => {
                        if state.staged_item.is_some() {
                            events.push(ShowEvent::StagedSlideChanged {
                                slide_index: slide_idx,
                            });
                        } else if let Some(sel_idx) = state.selected_item_index {
                            let target_item = schedule.items.get(sel_idx).cloned();
                            events.push(ShowEvent::ItemStaged {
                                item: target_item,
                                slide_index: slide_idx,
                            });
                        }
                    }
                }
            }
            ShowCommand::StageItemDirect(item) => {
                events.push(ShowEvent::ItemStaged {
                    item: Some(item.clone()),
                    slide_index: 0,
                });
            }
            ShowCommand::NextSlide => {
                if let Some(ref live) = state.live_item {
                    let live_len = effective_arrangement(live).len();
                    if state.live_slide_index + 1 < live_len {
                        events.push(ShowEvent::LiveSlideChanged {
                            slide_index: state.live_slide_index + 1,
                        });
                    } else if let Some(curr_idx) = state.selected_item_index {
                        // Advance to next schedule item
                        if curr_idx + 1 < schedule.items.len() {
                            let next_item = schedule.items.get(curr_idx + 1).cloned();
                            events.push(ShowEvent::ItemWentLive {
                                item: next_item,
                                slide_index: 0,
                            });
                        }
                    }
                }
            }
            ShowCommand::PrevSlide => {
                if state.live_slide_index > 0 {
                    events.push(ShowEvent::LiveSlideChanged {
                        slide_index: state.live_slide_index.saturating_sub(1),
                    });
                } else if let Some(curr_idx) = state.selected_item_index {
                    if curr_idx > 0 {
                        if let Some(prev_item) = schedule.items.get(curr_idx - 1).cloned() {
                            let last_slide = effective_arrangement(&prev_item).len().saturating_sub(1);
                            events.push(ShowEvent::ItemWentLive {
                                item: Some(prev_item),
                                slide_index: last_slide,
                            });
                        }
                    }
                }
            }
            ShowCommand::NextItem => {
                let current_idx = state.selected_item_index.unwrap_or(0);
                if current_idx + 1 < schedule.items.len() {
                    let next_item = schedule.items.get(current_idx + 1).cloned();
                    events.push(ShowEvent::ItemWentLive {
                        item: next_item,
                        slide_index: 0,
                    });
                }
            }
            ShowCommand::PrevItem => {
                let current_idx = state.selected_item_index.unwrap_or(0);
                if current_idx > 0 {
                    let prev_item = schedule.items.get(current_idx - 1).cloned();
                    events.push(ShowEvent::ItemWentLive {
                        item: prev_item,
                        slide_index: 0,
                    });
                }
            }
            ShowCommand::JumpSlide(idx) => {
                if let Some(ref live) = state.live_item {
                    if *idx < effective_arrangement(live).len() {
                        events.push(ShowEvent::LiveSlideChanged {
                            slide_index: *idx,
                        });
                    }
                }
            }
            ShowCommand::JumpSection(prefix) => {
                if let Some(ref live) = state.live_item {
                    let prefix_upper = prefix.to_uppercase();
                    let arr = effective_arrangement(live);
                    let matches: Vec<usize> = arr.iter().enumerate()
                        .filter(|(_, e)| e.section_id.to_uppercase().starts_with(&prefix_upper))
                        .map(|(i, _)| i)
                        .collect();
                    if !matches.is_empty() {
                        // Cycle forward from wherever this letter was last visited
                        // (see ShowState::section_jump_cursor) instead of always
                        // landing back on the first match — repeated presses of
                        // the same letter step through verse 1, 2, 3, ... and
                        // switching away to another letter and back resumes
                        // right after the one you left.
                        let next_idx = match state.section_jump_cursor.get(&prefix_upper) {
                            Some(&last) => match matches.iter().position(|&i| i == last) {
                                Some(pos) => matches[(pos + 1) % matches.len()],
                                None => matches[0],
                            },
                            None => matches[0],
                        };
                        events.push(ShowEvent::LiveSlideChanged {
                            slide_index: next_idx,
                        });
                    }
                }
            }
            ShowCommand::ToggleBlackout => {
                events.push(ShowEvent::BlackoutToggled {
                    is_blackout: !state.is_blackout,
                });
            }
            ShowCommand::ToggleClearText => {
                events.push(ShowEvent::ClearTextToggled {
                    is_clear_text: !state.is_clear_text,
                });
            }
            ShowCommand::ToggleLogo => {
                events.push(ShowEvent::LogoToggled {
                    is_logo: !state.is_logo,
                });
            }
            ShowCommand::SetAlert(alert) => {
                events.push(ShowEvent::AlertChanged {
                    alert: alert.clone(),
                });
            }
            ShowCommand::SetWebStream(web_stream) => {
                events.push(ShowEvent::WebStreamChanged {
                    web_stream: web_stream.clone(),
                });
            }
            ShowCommand::SetTransition(trans) => {
                events.push(ShowEvent::TransitionChanged {
                    transition: trans.clone(),
                });
            }
            ShowCommand::AddToSchedule(item) => {
                let index = schedule.items.len();
                events.push(ShowEvent::ScheduleItemAdded {
                    item: item.clone(),
                    index,
                });
            }
            ShowCommand::AddScheduleHeader { title, index } => {
                let idx = index.unwrap_or(schedule.items.len());
                let header_item = ScheduleItem {
                    id: format!("header_{}", uuid::Uuid::new_v4()),
                    title: title.clone(),
                    item_type: "header".to_string(),
                    author_or_ref: String::new(),
                    slides: Vec::new(),
                    background: None,
                    theme_name: None,
                    is_expanded: true,
                    is_section_header: true, notes: None, subtitle: None,
                    arrangement: Vec::new(),
                    default_slide_duration_seconds: None,
                    slideshow_loop: false,
                };
                events.push(ShowEvent::ScheduleItemAdded {
                    item: header_item,
                    index: idx,
                });
            }
            ShowCommand::RemoveFromSchedule(idx) => {
                if *idx < schedule.items.len() {
                    events.push(ShowEvent::ScheduleItemRemoved { index: *idx });
                }
            }
            ShowCommand::ReorderSchedule { from, to } => {
                if *from < schedule.items.len() && *to < schedule.items.len() {
                    events.push(ShowEvent::ScheduleReordered {
                        from: *from,
                        to: *to,
                    });
                }
            }
            ShowCommand::ReorderItemSlides { item_index, from, to } => {
                match item_index {
                    Some(idx) => {
                        if let Some(item) = schedule.items.get(*idx) {
                            let len = effective_arrangement(item).len();
                            if *from < len && *to < len {
                                events.push(ShowEvent::ScheduleSlidesReordered {
                                    item_index: Some(*idx),
                                    from: *from,
                                    to: *to,
                                });
                            }
                        }
                    }
                    None => {
                        if let Some(ref staged) = state.staged_item {
                            let len = effective_arrangement(staged).len();
                            if *from < len && *to < len {
                                events.push(ShowEvent::ScheduleSlidesReordered {
                                    item_index: None,
                                    from: *from,
                                    to: *to,
                                });
                            }
                        }
                    }
                }
            }
            ShowCommand::DuplicateItemSlide { item_index, slide_index } => {
                if let Some(item) = schedule.items.get(*item_index) {
                    if *slide_index < effective_arrangement(item).len() {
                        events.push(ShowEvent::ScheduleSlideDuplicated {
                            item_index: *item_index,
                            slide_index: *slide_index,
                        });
                    }
                }
            }
            ShowCommand::RemoveItemSlide { item_index, slide_index } => {
                if let Some(item) = schedule.items.get(*item_index) {
                    let len = effective_arrangement(item).len();
                    if *slide_index < len {
                        if len > 1 {
                            events.push(ShowEvent::ScheduleSlideRemoved {
                                item_index: *item_index,
                                slide_index: *slide_index,
                            });
                        } else {
                            events.push(ShowEvent::ScheduleItemRemoved {
                                index: *item_index,
                            });
                        }
                    }
                }
            }
            ShowCommand::MergeScheduleItems { first_item_index, second_item_index } => {
                let earlier_idx = (*first_item_index).min(*second_item_index);
                let later_idx = (*first_item_index).max(*second_item_index);
                // Require true adjacency: the feature is documented and exposed to the UI
                // as merging neighbors, and silently collapsing a wider span would delete
                // every item in between with no confirmation.
                let is_adjacent = later_idx == earlier_idx + 1;
                if let (Some(earlier), Some(later)) =
                    (schedule.items.get(earlier_idx), schedule.items.get(later_idx))
                {
                    // Regardless of which of the two the user right-clicked (merge-with-next
                    // vs. merge-with-previous), content always concatenates in schedule order.
                    if is_adjacent && !earlier.is_section_header && !later.is_section_header {
                        let mut resolved = resolved_play_sequence(earlier);
                        resolved.extend(resolved_play_sequence(later));
                        let (merged_slides, overrides): (Vec<Slide>, Vec<Option<String>>) = resolved.into_iter().unzip();
                        let mut arrangement = crate::core::models::identity_arrangement(&merged_slides);
                        for (entry, bg) in arrangement.iter_mut().zip(overrides.into_iter()) {
                            entry.background_override = bg;
                        }
                        let merged_item = ScheduleItem {
                            id: format!("item_{}", uuid::Uuid::new_v4()),
                            title: earlier.title.clone(),
                            subtitle: None,
                            notes: Some(format!("Merged from \"{}\" and \"{}\"", earlier.title, later.title)),
                            item_type: "presentation".to_string(),
                            author_or_ref: earlier.author_or_ref.clone(),
                            arrangement,
                            slides: merged_slides,
                            background: earlier.background.clone(),
                            theme_name: earlier.theme_name.clone(),
                            is_expanded: false,
                            is_section_header: false,
                            default_slide_duration_seconds: earlier.default_slide_duration_seconds,
                            slideshow_loop: earlier.slideshow_loop,
                        };
                        events.push(ShowEvent::ScheduleItemsMerged {
                            first_index: earlier_idx,
                            second_index: later_idx,
                            merged_item,
                        });
                    }
                }
            }
            ShowCommand::SetArrangement { item_index, arrangement } => {
                events.push(ShowEvent::ArrangementSet {
                    item_index: *item_index,
                    arrangement: arrangement.clone(),
                });
            }
            ShowCommand::ToggleScheduleItemExpand(idx) => {
                if let Some(item) = schedule.items.get(*idx) {
                    events.push(ShowEvent::ScheduleItemExpandToggled {
                        index: *idx,
                        is_expanded: !item.is_expanded,
                    });
                }
            }
            ShowCommand::SetItemTheme { item_index, theme_name, background } => {
                if *item_index < schedule.items.len() {
                    events.push(ShowEvent::ScheduleItemThemeSet {
                        item_index: *item_index,
                        theme_name: theme_name.clone(),
                        background: background.clone(),
                    });
                }
            }
            ShowCommand::SetSlideBackground { item_index, slide_index, background } => {
                events.push(ShowEvent::SlideBackgroundSet {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    background: background.clone(),
                });
            }
            ShowCommand::SetSlideDuration { item_index, slide_index, duration_seconds } => {
                if let Some(item) = schedule.items.get(*item_index) {
                    if *slide_index < item.slides.len() {
                        events.push(ShowEvent::SlideDurationSet {
                            item_index: *item_index,
                            slide_index: *slide_index,
                            duration_seconds: *duration_seconds,
                        });
                    }
                }
            }
            ShowCommand::SetPresentationDefaultDuration { item_index, duration_seconds } => {
                if schedule.items.get(*item_index).is_some() {
                    events.push(ShowEvent::PresentationDefaultDurationSet {
                        item_index: *item_index,
                        duration_seconds: *duration_seconds,
                    });
                }
            }
            ShowCommand::SetPresentationLoop { item_index, loop_enabled } => {
                if schedule.items.get(*item_index).is_some() {
                    events.push(ShowEvent::PresentationLoopSet {
                        item_index: *item_index,
                        loop_enabled: *loop_enabled,
                    });
                }
            }
            ShowCommand::SetGlobalTheme { theme_name, background } => {
                events.push(ShowEvent::GlobalThemeSet {
                    theme_name: theme_name.clone(),
                    background: background.clone(),
                });
            }
            ShowCommand::SetScheduleTitle(title) => {
                events.push(ShowEvent::ScheduleTitleSet {
                    title: title.clone(),
                });
            }
            ShowCommand::NewSchedule => {
                events.push(ShowEvent::ScheduleCleared);
            }
            ShowCommand::LoadSchedule(sched) => {
                events.push(ShowEvent::ScheduleLoaded {
                    schedule: sched.clone(),
                });
            },
            ShowCommand::MediaScheduledStart { target_pts, start_at_epoch_ms } => {
                let safe_pts = if target_pts.is_finite() && *target_pts >= 0.0 { *target_pts } else { 0.0 };
                events.push(ShowEvent::MediaScheduledStart { target_pts: safe_pts, start_at_epoch_ms: *start_at_epoch_ms });
            },
            ShowCommand::MediaPause { current_time } => {
                let safe_time = if current_time.is_finite() && *current_time >= 0.0 { *current_time } else { 0.0 };
                events.push(ShowEvent::MediaPaused { current_time: safe_time });
            },
            ShowCommand::MediaSetLoop(is_looping) => { events.push(ShowEvent::MediaLoopSet { is_looping: *is_looping }); },
            ShowCommand::MediaSetMute(is_muted) => { events.push(ShowEvent::MediaMuteSet { is_muted: *is_muted }); },
            ShowCommand::MediaSetVolume(volume) => {
                let safe_vol = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 1.0 };
                events.push(ShowEvent::MediaVolumeSet { volume: safe_vol });
            },
            ShowCommand::MediaPreroll { target_pts } => {
                let safe_pts = if target_pts.is_finite() && *target_pts >= 0.0 { *target_pts } else { 0.0 };
                events.push(ShowEvent::MediaPrerolled { target_pts: safe_pts });
            },
            ShowCommand::AddSlideElement { item_index, slide_index, element, at_index } => {
                events.push(ShowEvent::SlideElementAdded {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    element: element.clone(),
                    at_index: *at_index,
                });
            },
            ShowCommand::RemoveSlideElement { item_index, slide_index, element_id } => {
                events.push(ShowEvent::SlideElementRemoved {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    element_id: element_id.clone(),
                });
            },
            ShowCommand::UpdateSlideElementTransform { item_index, slide_index, element_id, transform } => {
                events.push(ShowEvent::SlideElementTransformUpdated {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    element_id: element_id.clone(),
                    transform: transform.clone(),
                });
            },
            ShowCommand::UpdateTextBlockContent { item_index, slide_index, element_id, runs, paragraph_style } => {
                events.push(ShowEvent::TextBlockContentUpdated {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    element_id: element_id.clone(),
                    runs: runs.clone(),
                    paragraph_style: paragraph_style.clone(),
                });
            },
            ShowCommand::UpdateElementEffects { item_index, slide_index, element_id, effects } => {
                events.push(ShowEvent::ElementEffectsUpdated {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    element_id: element_id.clone(),
                    effects: effects.clone(),
                });
            },
            ShowCommand::ReorderSlideElements { item_index, slide_index, element_id, to_z } => {
                events.push(ShowEvent::SlideElementsReordered {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    element_id: element_id.clone(),
                    to_z: *to_z,
                });
            },
            ShowCommand::GroupSlideElements { item_index, slide_index, element_ids } => {
                events.push(ShowEvent::SlideElementsGrouped {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    element_ids: element_ids.clone(),
                });
            },
            ShowCommand::UngroupSlideElements { item_index, slide_index, group_id } => {
                events.push(ShowEvent::SlideElementsUngrouped {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    group_id: group_id.clone(),
                });
            },
            ShowCommand::SetSlideBackgroundV2 { item_index, slide_index, background } => {
                events.push(ShowEvent::SlideBackgroundV2Set {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    background: background.clone(),
                });
            },
            ShowCommand::SetSlideTransition { item_index, slide_index, transition } => {
                events.push(ShowEvent::SlideTransitionSet {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    transition: transition.clone(),
                });
            },
            ShowCommand::SetSlideSpeakerNotes { item_index, slide_index, notes } => {
                events.push(ShowEvent::SlideSpeakerNotesSet {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    notes: notes.clone(),
                });
            },
            ShowCommand::SetSlideCcliMetadata { item_index, slide_index, metadata } => {
                events.push(ShowEvent::SlideCcliMetadataSet {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    metadata: metadata.clone(),
                });
            },
            ShowCommand::BatchSlideEdit { item_index, slide_index, ops } => {
                events.push(ShowEvent::SlideEditedBatch {
                    item_index: *item_index,
                    slide_index: *slide_index,
                    ops: ops.clone(),
                });
            },
            ShowCommand::Undo => {
                let target_opt = self.undo_seq_stack.write().unwrap().pop();
                if let Some(target) = target_opt {
                    self.redo_seq_stack.write().unwrap().push(state.sequence_number);
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
                let target_opt = self.redo_seq_stack.write().unwrap().pop();
                if let Some(target) = target_opt {
                    self.undo_seq_stack.write().unwrap().push(state.sequence_number);
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

        }

        events
    }

    /// Records the current live slide's section letter (the first character
    /// of its section_id, e.g. "V2" -> "V") as that letter's JumpSection
    /// cursor — so cycling through verses stays in sync even when the
    /// operator gets there by Next/Prev/a numbered jump instead of pressing
    /// 'V' itself, not just when JumpSection is used directly.
    fn update_section_jump_cursor(state: &mut ShowState) {
        if let Some(ref live) = state.live_item {
            let arr = effective_arrangement(live);
            if let Some(entry) = arr.get(state.live_slide_index) {
                if let Some(first_char) = entry.section_id.chars().next() {
                    let letter = first_char.to_uppercase().to_string();
                    state.section_jump_cursor.insert(letter, state.live_slide_index);
                }
            }
        }
    }

    /// Pure deterministic reducer for State and Schedule
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
            ShowEvent::ItemWentLive { item, slide_index } => {
                // A new (or cleared) live item invalidates any in-flight dedicated media
                // playback state — it described the item that just left, and would
                // otherwise mislead every client into resuming/seeking the wrong content.
                state.media_playback = None;
                state.live_item = item.clone();
                state.live_slide_index = *slide_index;
                // A different item's verse/chorus/etc. positions aren't meaningful
                // for this one — start each letter's JumpSection cycle fresh.
                state.section_jump_cursor.clear();
                Self::update_section_jump_cursor(state);

                // Sync selected item index
                if let Some(ref live) = item {
                    if let Some(pos) = schedule.items.iter().position(|i| i.id == live.id) {
                        state.selected_item_index = Some(pos);
                    }
                }
            }
            ShowEvent::LiveSlideChanged { slide_index } => {
                state.live_slide_index = *slide_index;
                Self::update_section_jump_cursor(state);
            }
            ShowEvent::ItemStaged { item, slide_index } => {
                state.staged_item = item.clone();
                state.staged_slide_index = *slide_index;

                // Sync selected item index if staged item is in schedule
                if let Some(ref staged) = item {
                    if let Some(pos) = schedule.items.iter().position(|i| i.id == staged.id) {
                        state.selected_item_index = Some(pos);
                    }
                }
            }
            ShowEvent::StagedSlideChanged { slide_index } => {
                state.staged_slide_index = *slide_index;
            }
            ShowEvent::BlackoutToggled { is_blackout } => {
                state.is_blackout = *is_blackout;
            }
            ShowEvent::ClearTextToggled { is_clear_text } => {
                state.is_clear_text = *is_clear_text;
            }
            ShowEvent::LogoToggled { is_logo } => {
                state.is_logo = *is_logo;
            }
            ShowEvent::AlertChanged { alert } => {
                state.alert_text = alert.clone();
            }
            ShowEvent::WebStreamChanged { web_stream } => {
                state.web_stream = web_stream.clone();
            }
            ShowEvent::TransitionChanged { transition } => {
                state.transition = transition.clone();
            }
            ShowEvent::ScheduleItemAdded { item, index } => {
                let mut item = item.clone();
                for slide in &mut item.slides {
                    slide.synthesize_v2_if_needed();
                }
                if *index <= schedule.items.len() {
                    schedule.items.insert(*index, item);
                } else {
                    schedule.items.push(item);
                }
                schedule.is_modified = true;
            }
            ShowEvent::ScheduleItemRemoved { index } => {
                if *index < schedule.items.len() {
                    schedule.items.remove(*index);
                    schedule.is_modified = true;
                    if let Some(sel) = state.selected_item_index {
                        if sel >= schedule.items.len() {
                            state.selected_item_index = if schedule.items.is_empty() {
                                None
                            } else {
                                Some(schedule.items.len() - 1)
                            };
                        }
                    }
                }
            }
            ShowEvent::ScheduleItemsMerged { first_index, second_index, merged_item } => {
                if *first_index < *second_index && *second_index < schedule.items.len() {
                    schedule.items.remove(*second_index);
                    schedule.items.remove(*first_index);
                    let mut item = merged_item.clone();
                    for slide in &mut item.slides {
                        slide.synthesize_v2_if_needed();
                    }
                    let insert_at = (*first_index).min(schedule.items.len());
                    schedule.items.insert(insert_at, item);
                    schedule.is_modified = true;
                    // Two items are removed and one is reinserted at first_index, a net
                    // shift of -1 — but only for indices *after* second_index. Anything
                    // strictly between first_index and second_index is unaffected: the
                    // removal of first_index shifts it down by one, and the reinsertion
                    // at first_index shifts it right back up by one.
                    if let Some(sel) = state.selected_item_index {
                        state.selected_item_index = Some(if sel == *first_index || sel == *second_index {
                            insert_at
                        } else if sel > *second_index {
                            sel - 1
                        } else {
                            sel
                        });
                    }
                }
            }
            ShowEvent::ScheduleReordered { from, to } => {
                if *from < schedule.items.len() && *to < schedule.items.len() {
                    let item = schedule.items.remove(*from);
                    schedule.items.insert(*to, item);
                    schedule.is_modified = true;
                    if let Some(sel) = state.selected_item_index {
                        if sel == *from {
                            state.selected_item_index = Some(*to);
                        } else if *from < sel && *to >= sel {
                            state.selected_item_index = Some(sel - 1);
                        } else if *from > sel && *to <= sel {
                            state.selected_item_index = Some(sel + 1);
                        }
                    }
                }
            }
            ShowEvent::ScheduleSlidesReordered { item_index, from, to } => {
                if let Some(idx) = item_index {
                    if let Some(item) = schedule.items.get_mut(*idx) {
                        let len = effective_arrangement(item).len();
                        if *from < len && *to < len {
                            let is_identity = item.arrangement.is_empty() || item.arrangement == crate::core::models::identity_arrangement(&item.slides);
                            if is_identity && *from < item.slides.len() && *to < item.slides.len() {
                                let slide = item.slides.remove(*from);
                                item.slides.insert(*to, slide);
                                item.arrangement = crate::core::models::identity_arrangement(&item.slides);
                            } else {
                                let entry = item.arrangement.remove(*from);
                                item.arrangement.insert(*to, entry);
                            }
                            schedule.is_modified = true;

                            // Sync staged item if it matches
                            if let Some(ref mut staged) = state.staged_item {
                                if staged.id == item.id {
                                    *staged = item.clone();
                                    if state.staged_slide_index == *from {
                                        state.staged_slide_index = *to;
                                    } else if *from < *to && state.staged_slide_index > *from && state.staged_slide_index <= *to {
                                        state.staged_slide_index = state.staged_slide_index.saturating_sub(1);
                                    } else if *from > *to && state.staged_slide_index >= *to && state.staged_slide_index < *from {
                                        state.staged_slide_index = state.staged_slide_index.saturating_add(1);
                                    }
                                }
                            }

                            // Also sync live item if it matches
                            if let Some(ref mut live) = state.live_item {
                                if live.id == item.id {
                                    *live = item.clone();
                                    if state.live_slide_index == *from {
                                        state.live_slide_index = *to;
                                    } else if *from < *to && state.live_slide_index > *from && state.live_slide_index <= *to {
                                        state.live_slide_index = state.live_slide_index.saturating_sub(1);
                                    } else if *from > *to && state.live_slide_index >= *to && state.live_slide_index < *from {
                                        state.live_slide_index = state.live_slide_index.saturating_add(1);
                                    }
                                }
                            }
                        }
                    }
                } else if let Some(ref mut staged) = state.staged_item {
                    let len = effective_arrangement(staged).len();
                    if *from < len && *to < len {
                        let is_identity = staged.arrangement.is_empty() || staged.arrangement == crate::core::models::identity_arrangement(&staged.slides);
                        if is_identity && *from < staged.slides.len() && *to < staged.slides.len() {
                            let slide = staged.slides.remove(*from);
                            staged.slides.insert(*to, slide);
                            staged.arrangement = crate::core::models::identity_arrangement(&staged.slides);
                        } else {
                            let entry = staged.arrangement.remove(*from);
                            staged.arrangement.insert(*to, entry);
                        }
                        if state.staged_slide_index == *from {
                            state.staged_slide_index = *to;
                        } else if *from < *to && state.staged_slide_index > *from && state.staged_slide_index <= *to {
                            state.staged_slide_index = state.staged_slide_index.saturating_sub(1);
                        } else if *from > *to && state.staged_slide_index >= *to && state.staged_slide_index < *from {
                            state.staged_slide_index = state.staged_slide_index.saturating_add(1);
                        }

                        // Also sync live item if it matches staged
                        if let Some(ref mut live) = state.live_item {
                            if live.id == staged.id {
                                *live = staged.clone();
                                state.live_slide_index = state.staged_slide_index;
                            }
                        }
                    }
                }
            }
            ShowEvent::ScheduleSlideDuplicated { item_index, slide_index } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    let len = effective_arrangement(item).len();
                    if *slide_index < len {
                        let is_identity = item.arrangement.is_empty() || item.arrangement == crate::core::models::identity_arrangement(&item.slides);
                        if is_identity && *slide_index < item.slides.len() {
                            if let Some(slide) = item.slides.get(*slide_index).cloned() {
                                item.slides.insert(slide_index + 1, slide);
                                item.arrangement = crate::core::models::identity_arrangement(&item.slides);
                            }
                        } else {
                            let entry = item.arrangement[*slide_index].clone();
                            item.arrangement.insert(slide_index + 1, entry);
                        }
                        schedule.is_modified = true;
                        if let Some(ref mut staged) = state.staged_item {
                            if staged.id == item.id {
                                *staged = item.clone();
                            }
                        }
                        if let Some(ref mut live) = state.live_item {
                            if live.id == item.id {
                                *live = item.clone();
                            }
                        }
                    }
                }
            }
            ShowEvent::ScheduleSlideRemoved { item_index, slide_index } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    let len = effective_arrangement(item).len();
                    if *slide_index < len && len > 1 {
                        let is_identity = item.arrangement.is_empty() || item.arrangement == crate::core::models::identity_arrangement(&item.slides);
                        if is_identity && *slide_index < item.slides.len() {
                            item.slides.remove(*slide_index);
                            item.arrangement = crate::core::models::identity_arrangement(&item.slides);
                        } else {
                            item.arrangement.remove(*slide_index);
                        }
                        schedule.is_modified = true;
                        if let Some(ref mut staged) = state.staged_item {
                            if staged.id == item.id {
                                *staged = item.clone();
                                let new_staged_len = effective_arrangement(staged).len();
                                if state.staged_slide_index > *slide_index {
                                    state.staged_slide_index = state.staged_slide_index.saturating_sub(1);
                                } else if state.staged_slide_index >= new_staged_len {
                                    state.staged_slide_index = new_staged_len.saturating_sub(1);
                                }
                            }
                        }
                        if let Some(ref mut live) = state.live_item {
                            if live.id == item.id {
                                *live = item.clone();
                                let new_live_len = effective_arrangement(live).len();
                                if state.live_slide_index > *slide_index {
                                    state.live_slide_index = state.live_slide_index.saturating_sub(1);
                                } else if state.live_slide_index >= new_live_len {
                                    state.live_slide_index = new_live_len.saturating_sub(1);
                                }
                            }
                        }
                    }
                }
            }
            ShowEvent::ArrangementSet { item_index, arrangement } => {
                let target_id = if let Some(idx) = item_index {
                    if let Some(item) = schedule.items.get_mut(*idx) {
                        item.arrangement = arrangement.clone();
                        schedule.is_modified = true;
                        Some(item.id.clone())
                    } else {
                        None
                    }
                } else if let Some(ref mut staged) = state.staged_item {
                    staged.arrangement = arrangement.clone();
                    Some(staged.id.clone())
                } else {
                    None
                };

                if let Some(ref id) = target_id {
                    // Sync staged item if id matches
                    if let Some(ref mut staged) = state.staged_item {
                        if &staged.id == id {
                            staged.arrangement = arrangement.clone();
                            let len = effective_arrangement(staged).len();
                            if state.staged_slide_index >= len && len > 0 {
                                state.staged_slide_index = len - 1;
                            }
                        }
                    }
                    // Sync live item if id matches
                    if let Some(ref mut live) = state.live_item {
                        if &live.id == id {
                            live.arrangement = arrangement.clone();
                            let len = effective_arrangement(live).len();
                            if state.live_slide_index >= len && len > 0 {
                                state.live_slide_index = len - 1;
                            }
                        }
                    }
                }
            }
            ShowEvent::ScheduleItemExpandToggled { index, is_expanded } => {
                if let Some(item) = schedule.items.get_mut(*index) {
                    item.is_expanded = *is_expanded;
                }
            }
            ShowEvent::ScheduleItemThemeSet { item_index, theme_name, background } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    item.theme_name = Some(theme_name.clone());
                    item.background = background.clone();
                    // Per-slide `Slide::background` outranks the item-level background in
                    // the rendering cascade (see resolveSlideBackground in
                    // presentation_helpers.ts), so a slide with its own override would
                    // silently ignore this theme change unless we clear it here too —
                    // this is exactly what made "apply theme" appear to do nothing.
                    for slide in item.slides.iter_mut() {
                        slide.background = background.clone();
                    }
                    schedule.is_modified = true;
                    if let Some(ref mut live) = state.live_item {
                        if live.id == item.id {
                            *live = item.clone();
                        }
                    }
                    if let Some(ref mut staged) = state.staged_item {
                        if staged.id == item.id {
                            *staged = item.clone();
                        }
                    }
                }
            }
            ShowEvent::SlideBackgroundSet { item_index, slide_index, background } => {
                if let Some(idx) = item_index {
                    if let Some(item) = schedule.items.get_mut(*idx) {
                        if !item.arrangement.is_empty() {
                            if let Some(entry) = item.arrangement.get_mut(*slide_index) {
                                entry.background_override = Some(background.clone());
                                schedule.is_modified = true;
                            }
                        } else if let Some(slide) = item.slides.get_mut(*slide_index) {
                            slide.background = Some(background.clone());
                            schedule.is_modified = true;
                        }
                        if let Some(ref mut live) = state.live_item {
                            if live.id == item.id {
                                *live = item.clone();
                            }
                        }
                        if let Some(ref mut staged) = state.staged_item {
                            if staged.id == item.id {
                                *staged = item.clone();
                            }
                        }
                    }
                } else if let Some(ref mut live) = state.live_item {
                    if !live.arrangement.is_empty() {
                        if let Some(entry) = live.arrangement.get_mut(*slide_index) {
                            entry.background_override = Some(background.clone());
                        }
                    } else if let Some(slide) = live.slides.get_mut(*slide_index) {
                        slide.background = Some(background.clone());
                    }
                }
            }
            ShowEvent::SlideDurationSet { item_index, slide_index, duration_seconds } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        slide.duration_seconds = *duration_seconds;
                        schedule.is_modified = true;
                    }
                    if let Some(ref mut live) = state.live_item {
                        if live.id == item.id {
                            *live = item.clone();
                        }
                    }
                    if let Some(ref mut staged) = state.staged_item {
                        if staged.id == item.id {
                            *staged = item.clone();
                        }
                    }
                }
            }
            ShowEvent::PresentationDefaultDurationSet { item_index, duration_seconds } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    item.default_slide_duration_seconds = *duration_seconds;
                    schedule.is_modified = true;
                    if let Some(ref mut live) = state.live_item {
                        if live.id == item.id {
                            *live = item.clone();
                        }
                    }
                    if let Some(ref mut staged) = state.staged_item {
                        if staged.id == item.id {
                            *staged = item.clone();
                        }
                    }
                }
            }
            ShowEvent::PresentationLoopSet { item_index, loop_enabled } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    item.slideshow_loop = *loop_enabled;
                    schedule.is_modified = true;
                    if let Some(ref mut live) = state.live_item {
                        if live.id == item.id {
                            *live = item.clone();
                        }
                    }
                    if let Some(ref mut staged) = state.staged_item {
                        if staged.id == item.id {
                            *staged = item.clone();
                        }
                    }
                }
            }
            ShowEvent::GlobalThemeSet { theme_name, background } => {
                state.global_theme = Some(theme_name.clone());
                state.global_background = Some(background.clone());
            }
            ShowEvent::ScheduleLoaded { schedule: new_sched } => {
                // `new_sched.selected_item_index` is the deprecated on-Schedule field
                // (still populated by the EWSX/OpenLP/FreeShow importers) — funnel it
                // into ShowState, which is what the engine and clients actually use now.
                state.selected_item_index = new_sched.selected_item_index;
                *schedule = new_sched.clone();
                schedule.is_modified = false;
            }
            ShowEvent::ScheduleCleared => {
                *schedule = Schedule::default();
                state.live_item = None;
                state.live_slide_index = 0;
                state.staged_item = None;
                state.staged_slide_index = 0;
                state.selected_item_index = None;
            }
            ShowEvent::ScheduleTitleSet { title } => {
                schedule.title = title.clone();
                schedule.is_modified = true;
            }
            ShowEvent::SlideElementAdded { item_index, slide_index, element, at_index } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        if let Some(idx) = at_index {
                            if *idx <= slide.elements.len() {
                                slide.elements.insert(*idx, element.clone());
                            } else {
                                slide.elements.push(element.clone());
                            }
                        } else {
                            slide.elements.push(element.clone());
                        }
                        normalize_z_indices(&mut slide.elements);
                        slide.project_text_from_elements();
                        slide.slide_document_version = 1;
                        schedule.is_modified = true;
                        let id = item.id.clone();
                        sync_item_in_state(state, schedule, &id);
                    }
                }
            }
            ShowEvent::SlideElementRemoved { item_index, slide_index, element_id } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        slide.elements.retain(|e| e.id() != element_id);
                        normalize_z_indices(&mut slide.elements);
                        slide.project_text_from_elements();
                        schedule.is_modified = true;
                        let id = item.id.clone();
                        sync_item_in_state(state, schedule, &id);
                    }
                }
            }
            ShowEvent::SlideElementTransformUpdated { item_index, slide_index, element_id, transform } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                            *el.transform_mut() = transform.clone().normalized();
                            schedule.is_modified = true;
                            let id = item.id.clone();
                            sync_item_in_state(state, schedule, &id);
                        }
                    }
                }
            }
            ShowEvent::TextBlockContentUpdated { item_index, slide_index, element_id, runs, paragraph_style } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                            if let crate::core::models::SlideElement::TextBlock { block, .. } = el {
                                block.runs = merge_contiguous_text_runs(runs);
                                if let Some(style) = paragraph_style {
                                    block.paragraph_style = style.clone();
                                }
                                slide.project_text_from_elements();
                                schedule.is_modified = true;
                                let id = item.id.clone();
                                sync_item_in_state(state, schedule, &id);
                            }
                        }
                    }
                }
            }
            ShowEvent::ElementEffectsUpdated { item_index, slide_index, element_id, effects } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                            if let crate::core::models::SlideElement::TextBlock { block, .. } = el {
                                block.effects = effects.clone();
                                schedule.is_modified = true;
                                let id = item.id.clone();
                                sync_item_in_state(state, schedule, &id);
                            }
                        }
                    }
                }
            }
            ShowEvent::SlideElementsReordered { item_index, slide_index, element_id, to_z } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                            el.transform_mut().z_index = *to_z;
                        }
                        normalize_z_indices(&mut slide.elements);
                        schedule.is_modified = true;
                        let id = item.id.clone();
                        sync_item_in_state(state, schedule, &id);
                    }
                }
            }
            ShowEvent::SlideElementsGrouped { item_index, slide_index, element_ids } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        group_elements(&mut slide.elements, element_ids);
                        schedule.is_modified = true;
                        let id = item.id.clone();
                        sync_item_in_state(state, schedule, &id);
                    }
                }
            }
            ShowEvent::SlideElementsUngrouped { item_index, slide_index, group_id } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        ungroup_elements(&mut slide.elements, group_id);
                        schedule.is_modified = true;
                        let id = item.id.clone();
                        sync_item_in_state(state, schedule, &id);
                    }
                }
            }
            ShowEvent::SlideBackgroundV2Set { item_index, slide_index, background } => {
                if let Some(idx) = item_index {
                    if let Some(item) = schedule.items.get_mut(*idx) {
                        if let Some(slide) = item.slides.get_mut(*slide_index) {
                            slide.background_v2 = Some(background.clone());
                            slide.background = Some(format_background_v2_to_css(background));
                            schedule.is_modified = true;
                            let id = item.id.clone();
                            sync_item_in_state(state, schedule, &id);
                        }
                    }
                } else if let Some(ref mut live) = state.live_item {
                    if let Some(slide) = live.slides.get_mut(*slide_index) {
                        slide.background_v2 = Some(background.clone());
                        slide.background = Some(format_background_v2_to_css(background));
                    }
                }
            }
            ShowEvent::SlideTransitionSet { item_index, slide_index, transition } => {
                if let Some(idx) = item_index {
                    if let Some(item) = schedule.items.get_mut(*idx) {
                        if let Some(slide) = item.slides.get_mut(*slide_index) {
                            slide.transition = Some(transition.clone());
                            schedule.is_modified = true;
                            let id = item.id.clone();
                            sync_item_in_state(state, schedule, &id);
                        }
                    }
                }
            }
            ShowEvent::SlideSpeakerNotesSet { item_index, slide_index, notes } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        slide.speaker_notes = Some(notes.clone());
                        slide.notes = Some(notes.clone());
                        schedule.is_modified = true;
                        let id = item.id.clone();
                        sync_item_in_state(state, schedule, &id);
                    }
                }
            }
            ShowEvent::SlideCcliMetadataSet { item_index, slide_index, metadata } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        slide.ccli_metadata = Some(metadata.clone());
                        schedule.is_modified = true;
                        let id = item.id.clone();
                        sync_item_in_state(state, schedule, &id);
                    }
                }
            }
            ShowEvent::SlideEditedBatch { item_index, slide_index, ops } => {
                if let Some(item) = schedule.items.get_mut(*item_index) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        for op in ops {
                            apply_slide_edit_op(slide, op);
                        }
                        slide.project_text_from_elements();
                        slide.slide_document_version = 1;
                        schedule.is_modified = true;
                        let id = item.id.clone();
                        sync_item_in_state(state, schedule, &id);
                    }
                }
            }
            ShowEvent::MediaScheduledStart { target_pts, start_at_epoch_ms } => {
                let mut playback = state.media_playback.take().unwrap_or_default();
                playback.is_playing = true;
                playback.current_time = *target_pts;
                playback.timestamp_ms = chrono::Utc::now().timestamp_millis() as u64;
                playback.start_at_epoch_ms = Some(*start_at_epoch_ms);
                playback.sync_version = playback.sync_version.wrapping_add(1);
                state.media_playback = Some(playback);
            }
            ShowEvent::MediaPaused { current_time } => {
                let mut playback = state.media_playback.take().unwrap_or_default();
                playback.is_playing = false;
                playback.current_time = *current_time;
                playback.timestamp_ms = chrono::Utc::now().timestamp_millis() as u64;
                playback.start_at_epoch_ms = None;
                playback.sync_version = playback.sync_version.wrapping_add(1);
                state.media_playback = Some(playback);
            }
            ShowEvent::MediaLoopSet { is_looping } => {
                let mut playback = state.media_playback.take().unwrap_or_default();
                playback.is_looping = *is_looping;
                playback.sync_version = playback.sync_version.wrapping_add(1);
                state.media_playback = Some(playback);
            }
            ShowEvent::MediaMuteSet { is_muted } => {
                let mut playback = state.media_playback.take().unwrap_or_default();
                playback.is_muted = *is_muted;
                playback.sync_version = playback.sync_version.wrapping_add(1);
                state.media_playback = Some(playback);
            }
            ShowEvent::MediaVolumeSet { volume } => {
                let mut playback = state.media_playback.take().unwrap_or_default();
                playback.volume = *volume;
                playback.sync_version = playback.sync_version.wrapping_add(1);
                state.media_playback = Some(playback);
            }
            ShowEvent::MediaPrerolled { target_pts } => {
                let mut playback = state.media_playback.take().unwrap_or_default();
                playback.is_playing = false;
                playback.current_time = *target_pts;
                playback.timestamp_ms = chrono::Utc::now().timestamp_millis() as u64;
                playback.start_at_epoch_ms = None;
                playback.sync_version = playback.sync_version.wrapping_add(1);
                state.media_playback = Some(playback);
            }


        }
    }

    pub fn restore_from_events(&self, events: Vec<crate::core::events::EventEnvelope>) {
        if events.is_empty() {
            return;
        }
        let mut state = self.state.write().unwrap();
        let mut schedule = self.schedule.write().unwrap();
        for env in &events {
            Self::apply_event(&mut state, &mut schedule, &env.event);
            state.sequence_number = env.sequence_number;
            state.last_event_id = env.event_id.clone();
            state.server_time_ms = env.timestamp_ms;
        }
        self.event_log.load_history(&events);
    }
}

fn sync_item_in_state(state: &mut ShowState, schedule: &Schedule, item_id: &str) {
    if let Some(item) = schedule.items.iter().find(|i| i.id == item_id) {
        if let Some(ref mut live) = state.live_item {
            if live.id == item.id {
                *live = item.clone();
            }
        }
        if let Some(ref mut staged) = state.staged_item {
            if staged.id == item.id {
                *staged = item.clone();
            }
        }
    }
}

fn apply_slide_edit_op(slide: &mut Slide, op: &crate::core::commands::SlideEditOp) {
    match op.op_type.as_str() {
        "AddElement" => {
            if let Ok(el) = serde_json::from_value::<crate::core::models::SlideElement>(op.payload.clone()) {
                slide.elements.push(el);
                normalize_z_indices(&mut slide.elements);
            }
        }
        "RemoveElement" => {
            if let Some(id) = op.payload.get("element_id").and_then(|v| v.as_str()) {
                slide.elements.retain(|e| e.id() != id);
                normalize_z_indices(&mut slide.elements);
            }
        }
        "UpdateTransform" => {
            if let (Some(id), Some(t_val)) = (
                op.payload.get("element_id").and_then(|v| v.as_str()),
                op.payload.get("transform"),
            ) {
                if let Ok(transform) = serde_json::from_value::<crate::core::models::ElementTransform>(t_val.clone()) {
                    if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == id) {
                        *el.transform_mut() = transform.normalized();
                    }
                }
            }
        }
        "UpdateTextBlockContent" => {
            if let Some(id) = op.payload.get("element_id").and_then(|v| v.as_str()) {
                if let Some(runs_val) = op.payload.get("runs") {
                    if let Ok(runs) = serde_json::from_value::<Vec<crate::core::models::TextRun>>(runs_val.clone()) {
                        if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == id) {
                            if let crate::core::models::SlideElement::TextBlock { block, .. } = el {
                                block.runs = merge_contiguous_text_runs(&runs);
                                if let Some(p_val) = op.payload.get("paragraph_style") {
                                    if let Ok(p_style) = serde_json::from_value::<crate::core::models::TextParagraphStyle>(p_val.clone()) {
                                        block.paragraph_style = p_style;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        "UpdateElementEffects" => {
            if let (Some(id), Some(eff_val)) = (
                op.payload.get("element_id").and_then(|v| v.as_str()),
                op.payload.get("effects"),
            ) {
                if let Ok(effects) = serde_json::from_value::<crate::core::models::ElementEffects>(eff_val.clone()) {
                    if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == id) {
                        if let crate::core::models::SlideElement::TextBlock { block, .. } = el {
                            block.effects = effects;
                        }
                    }
                }
            }
        }
        "ReorderElements" => {
            if let (Some(id), Some(to_z)) = (
                op.payload.get("element_id").and_then(|v| v.as_str()),
                op.payload.get("to_z").and_then(|v| v.as_i64()),
            ) {
                if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == id) {
                    el.transform_mut().z_index = to_z as i32;
                }
                normalize_z_indices(&mut slide.elements);
            }
        }
        "GroupElements" => {
            if let Some(ids_val) = op.payload.get("element_ids").and_then(|v| v.as_array()) {
                let ids: Vec<String> = ids_val.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
                group_elements(&mut slide.elements, &ids);
            }
        }
        "UngroupElements" => {
            if let Some(group_id) = op.payload.get("group_id").and_then(|v| v.as_str()) {
                ungroup_elements(&mut slide.elements, group_id);
            }
        }
        "SetSpeakerNotes" => {
            if let Some(notes) = op.payload.get("notes").and_then(|v| v.as_str()) {
                slide.speaker_notes = Some(notes.to_string());
                slide.notes = Some(notes.to_string());
            }
        }
        "SetCcliMetadata" => {
            if let Some(meta_val) = op.payload.get("metadata") {
                if let Ok(metadata) = serde_json::from_value::<crate::core::models::CcliMetadata>(meta_val.clone()) {
                    slide.ccli_metadata = Some(metadata);
                }
            }
        }
        "SetBackground" => {
            if let Some(bg_val) = op.payload.get("background") {
                if let Ok(background) = serde_json::from_value::<crate::core::models::SlideBackground>(bg_val.clone()) {
                    slide.background = Some(format_background_v2_to_css(&background));
                    slide.background_v2 = Some(background);
                }
            }
        }
        "SetTransition" => {
            if let Some(t_val) = op.payload.get("transition") {
                if let Ok(transition) = serde_json::from_value::<crate::core::models::SlideTransition>(t_val.clone()) {
                    slide.transition = Some(transition);
                }
            }
        }
        _ => {}
    }
}

fn normalize_z_indices(elements: &mut [crate::core::models::SlideElement]) {
    elements.sort_by_key(|e| e.transform().z_index);
    for (idx, el) in elements.iter_mut().enumerate() {
        el.transform_mut().z_index = idx as i32;
    }
}

fn group_elements(elements: &mut Vec<crate::core::models::SlideElement>, ids: &[String]) {
    let mut children = Vec::new();
    let mut min_x = 1.0;
    let mut min_y = 1.0;
    let mut max_x = 0.0;
    let mut max_y = 0.0;
    let mut max_z = 0;

    let mut i = 0;
    while i < elements.len() {
        if ids.contains(&elements[i].id().to_string()) {
            let el = elements.remove(i);
            let t = el.transform();
            if t.x < min_x { min_x = t.x; }
            if t.y < min_y { min_y = t.y; }
            if t.x + t.w > max_x { max_x = t.x + t.w; }
            if t.y + t.h > max_y { max_y = t.y + t.h; }
            if t.z_index > max_z { max_z = t.z_index; }
            children.push(el);
        } else {
            i += 1;
        }
    }

    if !children.is_empty() {
        let group_transform = crate::core::models::ElementTransform {
            x: min_x.clamp(0.0, 1.0),
            y: min_y.clamp(0.0, 1.0),
            w: (max_x - min_x).clamp(0.0, 1.0),
            h: (max_y - min_y).clamp(0.0, 1.0),
            rotation_deg: 0.0,
            z_index: max_z,
            locked: false,
            opacity: 1.0,
        };
        let group = crate::core::models::SlideElement::Group {
            id: format!("group_{}", uuid::Uuid::new_v4()),
            transform: group_transform,
            children,
        };
        elements.push(group);
        normalize_z_indices(elements);
    }
}

fn ungroup_elements(elements: &mut Vec<crate::core::models::SlideElement>, group_id: &str) {
    if let Some(pos) = elements.iter().position(|e| e.id() == group_id) {
        let el = elements.remove(pos);
        if let crate::core::models::SlideElement::Group { children, .. } = el {
            elements.extend(children);
            normalize_z_indices(elements);
        }
    }
}

fn merge_contiguous_text_runs(runs: &[crate::core::models::TextRun]) -> Vec<crate::core::models::TextRun> {
    let mut merged: Vec<crate::core::models::TextRun> = Vec::new();
    for run in runs {
        if run.text.is_empty() {
            continue;
        }
        if let Some(last) = merged.last_mut() {
            if last.bold == run.bold
                && last.italic == run.italic
                && last.underline == run.underline
                && last.strike == run.strike
                && last.color == run.color
                && last.font_family == run.font_family
                && (last.font_size_pt - run.font_size_pt).abs() < 0.01
                && (last.letter_spacing_px - run.letter_spacing_px).abs() < 0.01
                && last.baseline_shift == run.baseline_shift
            {
                last.text.push_str(&run.text);
                continue;
            }
        }
        merged.push(run.clone());
    }
    merged
}

fn format_background_v2_to_css(bg: &crate::core::models::SlideBackground) -> String {
    match bg {
        crate::core::models::SlideBackground::Solid(color) => color.clone(),
        crate::core::models::SlideBackground::Gradient { kind, stops, angle_deg } => {
            let stop_strs = stops.iter().map(|s| format!("{} {}%", s.color, (s.offset * 100.0) as u32)).collect::<Vec<_>>().join(", ");
            if kind == "radial" {
                format!("radial-gradient(circle, {})", stop_strs)
            } else {
                let angle = angle_deg.unwrap_or(135.0);
                format!("linear-gradient({}deg, {})", angle as u32, stop_strs)
            }
        }
        crate::core::models::SlideBackground::Image { file_path, .. } => format!("url('{}')", file_path),
        crate::core::models::SlideBackground::Video { file_path, .. } => file_path.clone(),
        crate::core::models::SlideBackground::Pattern { bg_color, .. } => bg_color.clone(),
    }
}

