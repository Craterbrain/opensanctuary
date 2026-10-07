use crate::core::commands::ShowCommand;
use crate::core::events::ShowEvent;
use crate::core::models::{Schedule, ShowState};
use super::elements::sync_item_in_state;

pub(super) fn plan_theme_events(_state: &ShowState, schedule: &Schedule, cmd: &ShowCommand) -> Vec<ShowEvent> {
    let mut events = Vec::new();
    match cmd {
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
        _ => unreachable!("plan_theme_events called with a non-theme command"),
    }
    events
}

pub(super) fn apply_theme_event(state: &mut ShowState, schedule: &mut Schedule, event: &ShowEvent) {
    match event {
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
                sync_item_in_state(state, schedule, *item_index);
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
                    sync_item_in_state(state, schedule, *idx);
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
                sync_item_in_state(state, schedule, *item_index);
            }
        }
        ShowEvent::PresentationDefaultDurationSet { item_index, duration_seconds } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                item.default_slide_duration_seconds = *duration_seconds;
                schedule.is_modified = true;
                sync_item_in_state(state, schedule, *item_index);
            }
        }
        ShowEvent::PresentationLoopSet { item_index, loop_enabled } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                item.slideshow_loop = *loop_enabled;
                schedule.is_modified = true;
                sync_item_in_state(state, schedule, *item_index);
            }
        }
        ShowEvent::GlobalThemeSet { theme_name, background } => {
            state.global_theme = Some(theme_name.clone());
            state.global_background = Some(background.clone());
        }
        _ => unreachable!("apply_theme_event called with a non-theme event"),
    }
}
