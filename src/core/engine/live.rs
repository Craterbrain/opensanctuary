use std::borrow::Cow;
use std::collections::HashSet;

use crate::core::commands::ShowCommand;
use crate::core::events::ShowEvent;
use crate::core::models::{ArrangementEntry, Schedule, ScheduleItem, ShowState, Slide};

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
fn slide_section_id(slide: &Slide, index: usize) -> String {
    slide
        .tag
        .as_ref()
        .or(slide.label.as_ref())
        .cloned()
        .unwrap_or_else(|| format!("S{}", index + 1))
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
    let original_section_ids: HashSet<String> = item
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

pub(super) fn plan_live_events(state: &ShowState, schedule: &Schedule, cmd: &ShowCommand) -> Vec<ShowEvent> {
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
        _ => unreachable!("plan_live_events called with a non-live command"),
    }
    events
}

pub(super) fn apply_live_event(state: &mut ShowState, schedule: &mut Schedule, event: &ShowEvent) {
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
            update_section_jump_cursor(state);

            // Sync selected item index
            if let Some(ref live) = item {
                if let Some(pos) = schedule.items.iter().position(|i| i.id == live.id) {
                    state.selected_item_index = Some(pos);
                }
            }
        }
        ShowEvent::LiveSlideChanged { slide_index } => {
            state.live_slide_index = *slide_index;
            update_section_jump_cursor(state);
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
        _ => unreachable!("apply_live_event called with a non-live event"),
    }
}
