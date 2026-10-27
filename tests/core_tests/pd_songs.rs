//! The bundled public-domain hymn library over HTTP: how many are missing, adding them, and the token gate.

use super::bundles::start;

#[tokio::test]
async fn the_hymn_library_is_offered_added_once_and_gated() {
    let s = start().await;
    let http = reqwest::Client::new();

    let j: serde_json::Value = http.get(s.url("/api/songs/public-domain")).send().await.unwrap().json().await.unwrap();
    let total = j["total"].as_u64().unwrap();
    assert!(total > 150, "got {total}");
    assert_eq!(j["present"], 0);
    assert_eq!(j["missing"], total);

    // Adding changes the library, so it needs the console's token.
    assert_eq!(http.post(s.url("/api/songs/public-domain")).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(s.db.get_songs().unwrap().is_empty());

    let j: serde_json::Value = http.post(s.url("/api/songs/public-domain")).header("x-host-token", &s.token).send().await.unwrap().json().await.unwrap();
    assert_eq!((j["success"].as_bool(), j["added"].as_u64(), j["already_present"].as_u64()), (Some(true), Some(total), Some(0)));
    let songs = s.db.get_songs().unwrap();
    assert_eq!(songs.len() as u64, total);
    let ag = songs.iter().find(|x| x.title == "Amazing Grace").expect("a classic is in it");
    assert_eq!(ag.copyright.as_deref(), Some("Public Domain"));
    assert!(ag.slides.len() >= 3);

    // Again: nothing is added twice, and nothing is reported missing.
    let j: serde_json::Value = http.post(s.url("/api/songs/public-domain")).header("x-host-token", &s.token).send().await.unwrap().json().await.unwrap();
    assert_eq!((j["added"].as_u64(), j["already_present"].as_u64()), (Some(0), Some(total)));
    let j: serde_json::Value = http.get(s.url("/api/songs/public-domain")).send().await.unwrap().json().await.unwrap();
    assert_eq!(j["missing"], 0);
    assert_eq!(s.db.get_songs().unwrap().len() as u64, total);
}
