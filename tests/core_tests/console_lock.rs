//! "One console at a time" for a console that only uses the HTTP fallback (no WebSocket to release the lock when it goes).

use super::*;
use std::time::{Duration, Instant};

fn state() -> os_next::api::ws::AppState {
    os_next::api::ws::AppState { host_session_token: "tok".into(), ..base_app_state(Database::in_memory().unwrap()) }
}

#[test]
fn an_http_only_console_gives_the_lock_back_when_it_closes_or_goes_idle() {
    let app = state();
    assert!(app.try_claim_console("a").is_ok());
    assert!(app.try_claim_console("a").is_ok(), "the owner keeps it");
    assert!(app.try_claim_console("b").is_err(), "first one wins");

    // The page closed and said so.
    assert!(!app.release_http_console("b"), "another session can't release it");
    assert!(app.release_http_console("a"));
    assert!(app.try_claim_console("b").is_ok());

    // A console that vanished without saying anything loses it after the idle time.
    assert!(app.try_claim_console("c").is_err());
    app.active_console.lock().unwrap().as_mut().unwrap().last_seen = Instant::now().checked_sub(Duration::from_secs(61)).unwrap();
    assert!(app.try_claim_console("c").is_ok());
}

#[test]
fn a_lock_held_by_a_live_websocket_is_not_released_by_the_http_route() {
    let app = state();
    assert!(app.try_claim_console("ws").is_ok());
    app.attach_console_ws("ws");
    assert!(!app.release_http_console("ws"), "its socket's disconnect releases it, not a page-close message");
    assert!(app.try_claim_console("other").is_err());
    app.release_console_if_held("ws");
    assert!(app.try_claim_console("other").is_ok());
}
