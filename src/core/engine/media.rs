use crate::core::commands::ShowCommand;
use crate::core::events::ShowEvent;
use crate::core::models::{Schedule, ShowState};

pub(super) fn plan_media_events(_state: &ShowState, _schedule: &Schedule, cmd: &ShowCommand) -> Vec<ShowEvent> {
    let mut events = Vec::new();
    match cmd {
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
        _ => unreachable!("plan_media_events called with a non-media command"),
    }
    events
}

pub(super) fn apply_media_event(state: &mut ShowState, _schedule: &mut Schedule, event: &ShowEvent) {
    match event {
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
        _ => unreachable!("apply_media_event called with a non-media event"),
    }
}
