use crate::core::commands::ShowCommand;
use crate::core::events::ShowEvent;
use crate::core::models::{Schedule, ShowState};

pub(super) fn plan_overlay_events(state: &ShowState, _schedule: &Schedule, cmd: &ShowCommand) -> Vec<ShowEvent> {
    let mut events = Vec::new();
    match cmd {
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
        _ => unreachable!("plan_overlay_events called with a non-overlay command"),
    }
    events
}

pub(super) fn apply_overlay_event(state: &mut ShowState, _schedule: &mut Schedule, event: &ShowEvent) {
    match event {
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
        _ => unreachable!("apply_overlay_event called with a non-overlay event"),
    }
}
