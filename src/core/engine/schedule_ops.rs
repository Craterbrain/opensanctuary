use crate::core::commands::ShowCommand;
use crate::core::events::ShowEvent;
use crate::core::models::{identity_arrangement, ArrangementEntry, Schedule, ScheduleItem, ShowState, Slide};

use super::live::effective_arrangement;

/// A slot id without its `~N` copy suffix (see `unique_slot_id`): the id every scheduled copy of one
/// library item shares.
pub fn slot_id_base(id: &str) -> &str {
    match id.rsplit_once('~') {
        Some((base, n)) if !base.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => id,
    }
}

/// A library item's schedule id comes from its library id (`sched_song_12`), so the same song scheduled
/// twice would share one id -- and everything that finds a slot by id (the selection following the live
/// item, the live/staged copies following an edit) would land on the first copy. The first copy keeps
/// the plain id; a later one takes the smallest free `~N` suffix (`sched_song_12~2`).
fn unique_slot_id(id: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(id) {
        return id.to_string();
    }
    let base = slot_id_base(id);
    (2u32..).map(|n| format!("{base}~{n}")).find(|candidate| !taken(candidate)).unwrap_or_else(|| id.to_string())
}

/// The play order `new` (an edited copy of the slot's `old` item) takes over from `old`, or `None` when
/// `old` was still in its authored order and there is nothing to keep. Each entry finds its slide again
/// by section id (tag, else label, else position) -- the nth "Chorus" stays the nth "Chorus" -- an entry
/// whose slide is gone is dropped, and a slide that wasn't there before plays at the end.
fn carried_arrangement(old: &ScheduleItem, new: &ScheduleItem) -> Option<Vec<ArrangementEntry>> {
    let old_sections = identity_arrangement(&old.slides);
    if old.arrangement.is_empty() || old.arrangement == old_sections {
        return None;
    }
    let new_sections = identity_arrangement(&new.slides);
    let nth_of = |sections: &[ArrangementEntry], idx: usize| {
        sections[..idx].iter().filter(|e| e.section_id == sections[idx].section_id).count()
    };
    let mut arrangement: Vec<ArrangementEntry> = old
        .arrangement
        .iter()
        .filter_map(|entry| {
            let section = &old_sections.get(entry.source_slide_index)?.section_id;
            let nth = nth_of(&old_sections, entry.source_slide_index);
            let target = new_sections.iter().filter(|e| e.section_id == *section).nth(nth)?;
            Some(ArrangementEntry { source_slide_index: target.source_slide_index, ..entry.clone() })
        })
        .collect();
    for (idx, entry) in new_sections.iter().enumerate() {
        let nth = nth_of(&new_sections, idx);
        if old_sections.iter().filter(|e| e.section_id == entry.section_id).nth(nth).is_none() {
            arrangement.push(entry.clone());
        }
    }
    (!arrangement.is_empty()).then_some(arrangement)
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

/// `item` as it should replace the slot's `old` item: what the operator chose for the slot stays.
/// - An item that arrives in its slides' own order (the editor, the library copy) has no opinion on the
///   play order, so the slot's arrangement is carried over.
/// - A themed slot keeps its theme. Applying one writes it into every slide's background, which outranks
///   the item's, so slides coming from elsewhere (the library copy, with a theme of its own or none) get
///   it written in again. The editor's own save keeps the slot's theme name and its slides as edited.
fn replacement_for(old: &ScheduleItem, item: &ScheduleItem) -> ScheduleItem {
    let mut item = item.clone();
    if item.arrangement.is_empty() || item.arrangement == identity_arrangement(&item.slides) {
        if let Some(arrangement) = carried_arrangement(old, &item) {
            item.arrangement = arrangement;
        }
    }
    // A theme, or a picture the operator set as the slot's background (with no theme name).
    let slot_has_look = old.theme_name.is_some() || old.background.is_some();
    if slot_has_look && (item.theme_name != old.theme_name || item.background != old.background) {
        item.theme_name = old.theme_name.clone();
        item.background = old.background.clone();
        for slide in &mut item.slides {
            slide.background = old.background.clone();
        }
    }
    item
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
        ShowCommand::AddToScheduleAndGoLive(item) => {
            // The live copy is the item exactly as the schedule will hold it (slides brought up to date,
            // its slot id made unique), so `ItemWentLive` finds and selects this slot, not an older copy.
            let index = schedule.items.len();
            let mut item = item.clone();
            for slide in &mut item.slides {
                slide.synthesize_v2_if_needed();
            }
            item.id = unique_slot_id(&item.id, |id| schedule.items.iter().any(|i| i.id == id));
            let first = super::live::visible_start(&item, 0, false);
            events.push(ShowEvent::ScheduleItemAdded { item: item.clone(), index });
            // An item hidden throughout is still added; it just can't go live (as GoLive refuses it).
            if let Some(slide_index) = first {
                events.push(ShowEvent::ItemWentLive { item: Some(item), slide_index });
            }
        }
        ShowCommand::ReplaceScheduledCopies { item } => {
            // Every copy of this library item, wherever it is: the engine owns the slot id scheme.
            let base = slot_id_base(&item.id);
            for (index, old) in schedule.items.iter().enumerate() {
                if !old.is_section_header && !item.is_section_header && slot_id_base(&old.id) == base {
                    events.push(ShowEvent::ScheduleItemReplaced { index, item: replacement_for(old, item) });
                }
            }
        }
        ShowCommand::ReplaceScheduleItem { item_index, item, expected_id } => {
            // The sender's index is from the snapshot it had when its editor opened, and the schedule
            // may have been reordered since. When it says which item it means, that item is replaced
            // wherever it is now -- and nothing is, if it is gone.
            let index = match expected_id {
                Some(id) if schedule.items.get(*item_index).is_none_or(|i| i.id != *id) => {
                    schedule.items.iter().position(|i| i.id == *id)
                }
                _ => Some(*item_index),
            };
            if let Some((index, old)) = index.and_then(|i| Some((i, schedule.items.get(i)?))) {
                if !old.is_section_header && !item.is_section_header {
                    events.push(ShowEvent::ScheduleItemReplaced { index, item: replacement_for(old, item) });
                }
            }
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
                licensed: None,
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
                    let mut arrangement = identity_arrangement(&merged_slides);
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
                        licensed: None,
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
            // A schedule just opened from a file has nothing unsaved. Cleared here, in the event,
            // rather than when it's applied: Undo/Redo also apply a `ScheduleLoaded` (the replayed
            // schedule), and that one must keep the flag the replay worked out.
            events.push(ShowEvent::ScheduleLoaded {
                schedule: Schedule { is_modified: false, ..sched.clone() },
                replay: false,
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
            item.id = unique_slot_id(&item.id, |id| schedule.items.iter().any(|i| i.id == id));
            let index = (*index).min(schedule.items.len());
            schedule.items.insert(index, item);
            schedule.is_modified = true;
            // The selection stays on its item when something (a header, say) lands above it.
            if let Some(sel) = state.selected_item_index.filter(|sel| index <= *sel) {
                state.selected_item_index = Some(sel + 1);
            }
        }
        ShowEvent::ScheduleItemReplaced { item, index } => {
            if let Some(old) = schedule.items.get(*index) {
                let mut new_item = item.clone();
                for slide in &mut new_item.slides {
                    slide.synthesize_v2_if_needed();
                }
                // What the operator chose for this slot survives the library copy being swapped in.
                new_item.is_expanded = old.is_expanded;
                if old.theme_name.is_some() {
                    new_item.theme_name = old.theme_name.clone();
                    new_item.background = old.background.clone();
                }
                // The slot keeps its id when the item is still the same library item (so a second copy
                // of a song stays the second copy); a different item brings its own, kept unique.
                let old_id = old.id.clone();
                new_item.id = if slot_id_base(&new_item.id) == slot_id_base(&old_id) {
                    old_id.clone()
                } else {
                    unique_slot_id(&new_item.id, |id| schedule.items.iter().enumerate().any(|(i, it)| i != *index && it.id == id))
                };
                schedule.items[*index] = new_item.clone();
                schedule.is_modified = true;
                if let Some(staged) = state.staged_item.as_mut() {
                    if staged.id == old_id {
                        *staged = new_item.clone();
                        let len = effective_arrangement(staged).len();
                        state.staged_slide_index = state.staged_slide_index.min(len.saturating_sub(1));
                    }
                }
                if let Some(live) = state.live_item.as_mut() {
                    if live.id == old_id {
                        *live = new_item;
                        let len = effective_arrangement(live).len();
                        state.live_slide_index = state.live_slide_index.min(len.saturating_sub(1));
                    }
                }
            }
        }
        ShowEvent::ScheduleItemRemoved { index } => {
            if *index < schedule.items.len() {
                schedule.items.remove(*index);
                schedule.is_modified = true;
                if let Some(sel) = state.selected_item_index {
                    // The selection stays on its item when one above it goes. When the selected item
                    // itself goes, the one that slides into its place is selected (the last, at the end).
                    let sel = if *index < sel { sel - 1 } else { sel };
                    state.selected_item_index = Some(sel);
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
                        let is_identity = item.arrangement.is_empty() || item.arrangement == identity_arrangement(&item.slides);
                        if is_identity && *from < item.slides.len() && *to < item.slides.len() {
                            let slide = item.slides.remove(*from);
                            item.slides.insert(*to, slide);
                            item.arrangement = identity_arrangement(&item.slides);
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
                    let is_identity = staged.arrangement.is_empty() || staged.arrangement == identity_arrangement(&staged.slides);
                    if is_identity && *from < staged.slides.len() && *to < staged.slides.len() {
                        let slide = staged.slides.remove(*from);
                        staged.slides.insert(*to, slide);
                        staged.arrangement = identity_arrangement(&staged.slides);
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
                    let is_identity = item.arrangement.is_empty() || item.arrangement == identity_arrangement(&item.slides);
                    if is_identity && *slide_index < item.slides.len() {
                        if let Some(slide) = item.slides.get(*slide_index).cloned() {
                            item.slides.insert(slide_index + 1, slide);
                            item.arrangement = identity_arrangement(&item.slides);
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
                    let is_identity = item.arrangement.is_empty() || item.arrangement == identity_arrangement(&item.slides);
                    if is_identity && *slide_index < item.slides.len() {
                        item.slides.remove(*slide_index);
                        item.arrangement = identity_arrangement(&item.slides);
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
        ShowEvent::ScheduleLoaded { schedule: new_sched, .. } => {
            // `new_sched.selected_item_index` is the deprecated on-Schedule field
            // (still populated by the EWSX/OpenLP/FreeShow importers) — funnel it
            // into ShowState, which is what the engine and clients actually use now.
            state.selected_item_index = new_sched.selected_item_index;
            *schedule = new_sched.clone();
            // A file saved (or a log written) before slot ids were kept unique may hold the same id twice.
            let mut seen = std::collections::HashSet::new();
            for item in &mut schedule.items {
                if seen.contains(&item.id) {
                    item.id = unique_slot_id(&item.id, |id| seen.contains(id));
                }
                seen.insert(item.id.clone());
            }
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
        ShowEvent::SelectionSet { index } => {
            state.selected_item_index = index.filter(|i| *i < schedule.items.len());
        }
        _ => unreachable!("apply_schedule_event called with a non-schedule event"),
    }
}
