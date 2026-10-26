//! An EasyWorship presentation package (`.ewpx`) added to the Presentations library over HTTP.
//! The real package (a church's announcement deck, so it can't be committed) is read from `$OS_TEST_DOCS`.

use super::bundles::{start, Server};
use super::test_doc;
use base64::Engine;

async fn post(server: &Server, body: serde_json::Value, token: bool) -> reqwest::Response {
    let mut r = reqwest::Client::new().post(server.url("/api/import/ewpx")).json(&body);
    if token {
        r = r.header("x-host-token", &server.token);
    }
    r.send().await.unwrap()
}

fn body_of(path: &std::path::Path, extra: serde_json::Value) -> serde_json::Value {
    let mut b = serde_json::json!({
        "file_name": path.file_name().unwrap().to_string_lossy(),
        "file_data_base64": base64::engine::general_purpose::STANDARD.encode(std::fs::read(path).unwrap()),
    });
    b.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    b
}

#[tokio::test]
async fn an_ewpx_deck_joins_the_library_and_slides_with_a_missing_picture_fall_back_to_the_theme() {
    let Some(path) = test_doc("LCspring.ewpx") else { return };
    let s = start().await;

    assert_eq!(post(&s, body_of(&path, serde_json::json!({})), false).await.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(s.db.get_presentations().unwrap().is_empty());

    let r = post(&s, body_of(&path, serde_json::json!({})), true).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let j: serde_json::Value = r.json().await.unwrap();
    assert_eq!(j["success"], true);
    assert_eq!(j["title"], "LCspring");
    let n = j["slide_count"].as_u64().unwrap();
    assert!(n >= 10, "the whole deck comes in, got {n}");
    // The package carries no pictures, so every slide's background was a dangling file name.
    assert_eq!(j["backgrounds_replaced"].as_u64().unwrap(), n);

    let pres = s.db.get_presentations().unwrap();
    assert_eq!(pres.len(), 1);
    assert!(pres[0].slides.iter().all(|sl| sl.background.is_none() && sl.background_v2.is_none()), "no slide is left pointing at a missing picture");
    assert!(pres[0].slides.iter().any(|sl| !sl.elements.is_empty()), "text and positions are kept");
    // The deck is written in light text and the default presentation theme is a white page, so it gets a dark theme.
    assert_eq!(pres[0].theme_name.as_deref(), Some("Presentation Dark"));

    // Importing again makes a second copy; it never overwrites the first.
    let j: serde_json::Value = post(&s, body_of(&path, serde_json::json!({})), true).await.json().await.unwrap();
    assert_eq!(j["title"], "LCspring 2");
    assert_eq!(s.db.get_presentations().unwrap().len(), 2);

    // A theme that doesn't exist is refused and nothing is added.
    let r = post(&s, body_of(&path, serde_json::json!({ "theme_name": "No Such Theme" })), true).await;
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(s.db.get_presentations().unwrap().len(), 2);
}

#[tokio::test]
async fn a_picture_that_is_in_the_library_is_kept_and_junk_is_refused() {
    let Some(path) = test_doc("LCspring.ewpx") else { return };
    let s = start().await;
    // The deck's own background picture, now present in the library, stays on the slides.
    let images = s.media_dir().join("images");
    std::fs::create_dir_all(&images).unwrap();
    std::fs::write(images.join("ready for church.jpg"), b"\xff\xd8\xff\xe0 not really a jpeg").unwrap();
    let j: serde_json::Value = post(&s, body_of(&path, serde_json::json!({})), true).await.json().await.unwrap();
    assert_eq!(j["backgrounds_replaced"].as_u64().unwrap(), 0);
    let pres = s.db.get_presentations().unwrap();
    assert!(pres[0].slides.iter().all(|sl| sl.background.as_deref().is_some_and(|b| b.contains("ready for church"))));

    for junk in ["!!!not base64", &base64::engine::general_purpose::STANDARD.encode(b"not an ewpx at all")] {
        let r = post(&s, serde_json::json!({ "file_name": "x.ewpx", "file_data_base64": junk }), true).await;
        assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);
    }
    assert_eq!(s.db.get_presentations().unwrap().len(), 1);
}

/// Adding a presentation to the schedule applies the default theme, but must not wipe a slide's own picture
/// (an imported deck's backgrounds, a logo slide).
#[tokio::test]
async fn adding_a_presentation_to_the_schedule_keeps_each_slides_own_background() {
    use os_next::core::models::{Presentation, Slide, SlideBackground};
    let s = start().await;
    let logo = "/media/images/logo-slide.png".to_string();
    let with_logo = Slide {
        text: "".into(),
        background: Some(logo.clone()),
        background_v2: Some(SlideBackground::Image { file_path: logo.clone(), opacity: 1.0 }),
        ..Default::default()
    };
    let plain = Slide { text: "Welcome".into(), ..Default::default() };
    s.db.insert_presentation(&Presentation { id: "pres_t".into(), title: "Deck".into(), author: String::new(), slides: vec![with_logo, plain], theme_name: None }).unwrap();

    let r = reqwest::Client::new()
        .post(s.url("/api/command"))
        .header("x-host-token", &s.token)
        .header("x-console-session-id", "t")
        .json(&serde_json::json!({ "AddToSchedule": { "item_type": "presentation", "item_id": "pres_t" } }))
        .send()
        .await
        .unwrap();
    let j: serde_json::Value = r.json().await.unwrap();
    assert_eq!(j["success"], true, "{j}");
    let item = &j["state"]["schedule"]["items"][0];
    assert_eq!(item["theme_name"], "Presentation Light", "the default presentation theme is applied");
    assert_eq!(item["slides"][0]["background"], logo.as_str(), "the logo slide keeps its picture");
    assert!(item["slides"][1]["background"].is_null(), "a slide with none shows the theme");
}
