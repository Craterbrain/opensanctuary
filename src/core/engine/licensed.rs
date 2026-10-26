//! Filling in licensed passages (see `storage/db/licensed.rs`): the item is already on the schedule with a
//! reference and placeholder text; this puts the looked-up text into it, on the schedule and in the live
//! and staged copies. The event is stored without the text, so a restart brings back the placeholder.

use crate::core::commands::ShowCommand;
use crate::core::events::ShowEvent;
use crate::core::models::{Schedule, ShowState};

use super::elements::sync_item_in_state;

pub(super) fn plan_licensed_events(schedule: &Schedule, cmd: &ShowCommand) -> Vec<ShowEvent> {
    match cmd {
        ShowCommand::SetLicensedText { item_id, texts } => {
            let known = schedule.items.iter().any(|i| i.id == *item_id && i.licensed.is_some());
            if known && !texts.is_empty() {
                vec![ShowEvent::LicensedTextSet { item_id: item_id.clone(), texts: texts.clone() }]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

pub(super) fn apply_licensed_event(state: &mut ShowState, schedule: &mut Schedule, event: &ShowEvent) {
    if let ShowEvent::LicensedTextSet { item_id, texts } = event {
        let Some(idx) = schedule.items.iter().position(|i| i.id == *item_id && i.licensed.is_some()) else { return };
        let item = &mut schedule.items[idx];
        for t in texts {
            if let Some(slide) = item.slides.get_mut(t.slide_index) {
                slide.text = t.text.clone();
                slide.elements.clear();
                slide.synthesize_v2_if_needed();
            }
        }
        if let Some(l) = item.licensed.as_mut() {
            l.resolved = true;
        }
        // Not `is_modified`: the operator changed nothing.
        sync_item_in_state(state, schedule, idx);
    }
}
