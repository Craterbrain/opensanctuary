//! Filling in licensed passages (see `storage/db/licensed.rs`): the item is already on the schedule with a
//! reference and placeholder text; this puts the looked-up verses into it, on the schedule and in the live
//! and staged copies. The event is stored without the text, so a restart brings back the placeholder.

use crate::core::commands::ShowCommand;
use crate::core::events::ShowEvent;
use crate::core::models::{Schedule, ShowState};

use super::elements::sync_item_in_state;

pub(super) fn plan_licensed_events(schedule: &Schedule, cmd: &ShowCommand) -> Vec<ShowEvent> {
    match cmd {
        ShowCommand::SetLicensedPassage { item_id, verses } => {
            let known = schedule.items.iter().any(|i| i.id == *item_id && i.licensed.is_some());
            if known && !verses.is_empty() {
                vec![ShowEvent::LicensedPassageSet { item_id: item_id.clone(), verses: verses.clone() }]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

pub(super) fn apply_licensed_event(state: &mut ShowState, schedule: &mut Schedule, event: &ShowEvent) {
    if let ShowEvent::LicensedPassageSet { item_id, verses } = event {
        let Some(idx) = schedule.items.iter().position(|i| i.id == *item_id && i.licensed.is_some()) else { return };
        schedule.items[idx].fill_licensed(verses);
        // Not `is_modified`: the operator changed nothing.
        sync_item_in_state(state, schedule, idx);
    }
}
