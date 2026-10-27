//! "Restart to finish updating": the status flag and the restart route over HTTP. A test process is never a replaced program, so
//! the route must refuse; that a replaced program really restarts is covered by `network::relaunch` unit tests and by trying it
//! on a real install (docs/update.md).

use super::bundles::start;

#[tokio::test]
async fn status_says_whether_a_restart_is_needed_and_the_route_refuses_when_it_is_not() {
    let s = start().await;
    let http = reqwest::Client::new();

    // Console-only: no token, no answer.
    assert_eq!(http.get(s.url("/api/updates/status")).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(http.post(s.url("/api/system/restart")).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);

    let j: serde_json::Value = http.get(s.url("/api/updates/status")).header("x-host-token", &s.token).send().await.unwrap().json().await.unwrap();
    assert_eq!(j["restart_needed"], false);

    // Nothing was replaced, so asking to restart is refused and the process carries on (this test is still running).
    let r = http.post(s.url("/api/system/restart")).header("x-host-token", &s.token).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::CONFLICT);
    let j: serde_json::Value = r.json().await.unwrap();
    assert_eq!(j["ok"], false);
    assert!(j["error"].as_str().unwrap().contains("already the installed version"));
}
