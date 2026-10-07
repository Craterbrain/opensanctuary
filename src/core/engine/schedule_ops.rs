use crate::core::commands::ShowCommand;
use crate::core::events::ShowEvent;
use crate::core::models::{Schedule, ScheduleItem, ShowState, Slide};

use super::live::effective_arrangement;

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

pub(super) fn plan_schedule_events(state: &ShowState, schedule: &Schedule, cmd: &ShowCommand) -> Vec<ShowEvent> {
    let mut events = Vec::new();
    match cmd {
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
        _ => unreachable!("plan_schedule_events called with a non-schedule command"),
    }
    events
}

pub(super) fn apply_schedule_event(state: &mut ShowState, schedule: &mut Schedule, event: &ShowEvent) {
    match event {
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
        _ => unreachable!("apply_schedule_event called with a non-schedule event"),
    }
}
