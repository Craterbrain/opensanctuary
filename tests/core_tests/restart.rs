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

#[tokio::test]
async fn adopting_a_legacy_library_takes_only_the_one_the_server_found() {
    let s = start().await;
    let http = reqwest::Client::new();
    // A folder with a real-looking library.db, but not the one /api/server-info offered.
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("library.db"), b"SQLite format 3\0 not really").unwrap();
    let r = http
        .post(s.url("/api/system/adopt-legacy-library"))
        .header("x-host-token", &s.token)
        .json(&serde_json::json!({ "legacy_dir": elsewhere.path() }))
        .send()
        .await
        .unwrap();
    let j: serde_json::Value = r.json().await.unwrap();
    assert_eq!(j["ok"], false, "{j}");
    assert!(j.get("restart_required").is_none());
}
