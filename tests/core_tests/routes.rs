//! The library, schedule-file and command routes over real HTTP: what the console's own calls
//! do end to end in-process (the e2e suite drives the same routes through a browser).

use super::*;
use serde_json::{json, Value};

struct Api {
    base: String,
    c: reqwest::Client,
    _dir: tempfile::TempDir,
}

const HOST: &str = "routes_host";

impl Api {
    async fn start() -> Api {
        let dir = tempfile::tempdir().unwrap();
        let db = open_db(dir.path().join("library.db")).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        serve(listener, os_next::api::ws::AppState {
            host_session_token: HOST.into(),
            media_dir: dir.path().join("media"),
            server_port: addr.port(),
            ..base_app_state(db)
        });
        Api { base: format!("http://{addr}"), c: reqwest::Client::new(), _dir: dir }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.c.request(method, format!("{}{path}", self.base)).header("x-host-token", HOST).header("x-console-session-id", "routes-test")
    }

    async fn get(&self, path: &str) -> Value {
        let r = self.req(reqwest::Method::GET, path).send().await.unwrap();
        assert!(r.status().is_success(), "GET {path}: {}", r.status());
        r.json().await.unwrap()
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        let r = self.req(reqwest::Method::POST, path).json(&body).send().await.unwrap();
        assert!(r.status().is_success(), "POST {path}: {}", r.status());
        r.json().await.unwrap()
    }

    async fn delete(&self, path: &str) {
        let r = self.req(reqwest::Method::DELETE, path).send().await.unwrap();
        assert!(r.status().is_success(), "DELETE {path}: {}", r.status());
    }

    async fn titles(&self, path: &str, field: &str) -> Vec<String> {
        let mut v: Vec<String> = self.get(path).await.as_array().unwrap().iter().map(|x| x[field].as_str().unwrap_or_default().to_string()).collect();
        v.sort();
        v
    }

    async fn schedule_titles(&self) -> Vec<String> {
        self.get("/api/state").await["schedule"]["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap().to_string()).collect()
    }
}

fn slide(text: &str) -> Value {
    json!({ "text": text, "header": null, "label": null, "background": null })
}

#[tokio::test]
async fn library_items_are_created_listed_searched_and_deleted() {
    let api = Api::start().await;

    // Writes need the console's token (host_token_gate_middleware).
    let anon = api.c.post(format!("{}/api/songs", api.base)).json(&json!({})).send().await.unwrap();
    assert_eq!(anon.status(), reqwest::StatusCode::UNAUTHORIZED);

    for (id, title) in [("s1", "Amazing Grace"), ("s2", "Be Thou My Vision")] {
        api.post("/api/songs", json!({ "id": id, "title": title, "author": "Trad.", "copyright": null, "ccli_number": null, "slides": [slide(title)] })).await;
    }
    assert_eq!(api.titles("/api/songs", "title").await, ["Amazing Grace", "Be Thou My Vision"]);
    assert_eq!(api.titles("/api/songs?q=vision", "title").await, ["Be Thou My Vision"]);
    api.delete("/api/songs/s1").await;
    assert_eq!(api.titles("/api/songs", "title").await, ["Be Thou My Vision"]);

    // GET /api/scriptures hands back the stored JSON as is (get_scriptures_json); a search goes through the model.
    api.post("/api/scriptures", json!({
        "id": "kjv_john_3", "book": "John", "chapter": 3, "verse_start": 1, "verse_end": 2, "version": "KJV", "reference": "John 3",
        "verses": [{ "verse_number": 1, "text": "There was a man of the Pharisees" }, { "verse_number": 2, "text": "The same came to Jesus by night" }],
        "theme_name": null,
    })).await;
    let all = api.get("/api/scriptures").await;
    assert_eq!(all[0]["verses"][1]["text"], "The same came to Jesus by night");
    assert_eq!(api.titles("/api/scriptures?q=John 3", "reference").await, ["John 3"]);
    api.delete("/api/scriptures/kjv_john_3").await;
    assert_eq!(api.get("/api/scriptures").await, json!([]));

    api.post("/api/presentations", json!({ "id": "p1", "title": "Announcements", "author": "", "slides": [slide("Welcome")] })).await;
    assert_eq!(api.titles("/api/presentations", "title").await, ["Announcements"]);
    api.delete("/api/presentations/p1").await;
    assert!(api.titles("/api/presentations", "title").await.is_empty());

    let masters = api.titles("/api/slide-templates", "name").await; // the built-in masters are seeded
    api.post("/api/slide-templates", json!({ "id": "t1", "name": "Lower third" })).await;
    assert!(api.titles("/api/slide-templates", "name").await.contains(&"Lower third".to_string()));
    api.delete("/api/slide-templates/t1").await;
    assert_eq!(api.titles("/api/slide-templates", "name").await, masters);

    let before = api.titles("/api/themes", "name").await;
    api.post("/api/themes", json!({ "name": "Routes Test Theme", "background": "#123456" })).await;
    assert!(api.titles("/api/themes", "name").await.contains(&"Routes Test Theme".to_string()));
    api.delete("/api/themes/Routes Test Theme").await;
    assert_eq!(api.titles("/api/themes", "name").await, before);
}

#[tokio::test]
async fn add_to_schedule_by_library_id_and_reopening_a_saved_schedule() {
    let api = Api::start().await;
    api.post("/api/songs", json!({ "id": "s1", "title": "Amazing Grace", "author": "Newton", "copyright": null, "ccli_number": null, "slides": [slide("Amazing grace"), slide("How sweet the sound")] })).await;
    api.post("/api/presentations", json!({ "id": "p1", "title": "Announcements", "author": "", "slides": [slide("Welcome")] })).await;

    // The console adds library items by id; the server fills in the item (expand_shorthand).
    api.post("/api/command", json!({ "AddToSchedule": { "item_type": "song", "item_id": "s1" } })).await;
    api.post("/api/command", json!({ "AddToSchedule": { "item_type": "presentation", "item_id": "p1" } })).await;
    let items = api.get("/api/state").await["schedule"]["items"].clone();
    assert_eq!(items[0]["slides"][1]["text"], "How sweet the sound");
    assert_eq!(api.schedule_titles().await, ["Amazing Grace", "Announcements"]);
    // An id that isn't in the library adds nothing.
    let _ = api.req(reqwest::Method::POST, "/api/command").json(&json!({ "AddToSchedule": { "item_type": "song", "item_id": "nope" } })).send().await.unwrap();
    assert_eq!(api.schedule_titles().await.len(), 2);

    // Save it as a real .ewsx, then open it back: "replace" swaps the schedule, "append" adds to it.
    let saved = api.req(reqwest::Method::GET, "/api/schedule/export?title=Sunday").send().await.unwrap().bytes().await.unwrap();
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&saved);
    api.post("/api/command", json!("NewSchedule")).await;
    assert!(api.schedule_titles().await.is_empty());

    let open = |mode: &'static str| json!({ "file_name": "Sunday.ewsx", "file_data_base64": b64, "mode": mode });
    api.post("/api/schedule/open", open("replace")).await;
    assert_eq!(api.schedule_titles().await, ["Amazing Grace", "Announcements"]);
    api.post("/api/schedule/open", open("replace")).await;
    assert_eq!(api.schedule_titles().await, ["Amazing Grace", "Announcements"]);
    api.post("/api/schedule/open", open("append")).await;
    assert_eq!(api.schedule_titles().await, ["Amazing Grace", "Announcements", "Amazing Grace", "Announcements"]);

    let nothing = api.req(reqwest::Method::POST, "/api/schedule/open").json(&json!({ "mode": "replace" })).send().await.unwrap();
    assert_eq!(nothing.status(), reqwest::StatusCode::BAD_REQUEST);
}
