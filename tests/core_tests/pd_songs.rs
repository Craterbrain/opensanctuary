//! The public-domain hymn download over HTTP: the token gate, adding once, and refusing a bad download.

use super::bundles::start;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A one-file web server standing in for the hymn repository: answers every request with `status` and `body`.
async fn serve(status: &'static str, body: String) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hymns.json", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = sock.write_all(reply.as_bytes()).await;
            });
        }
    });
    url
}

fn hymn(id: &str, title: &str, author: &str, copyright: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "title": title, "author": author, "copyright": copyright, "ccli_number": null, "theme_name": null,
        "slides": [{ "id": format!("{id}-1"), "text": "Amazing grace how sweet the sound that saved a wretch like me", "label": "Verse 1" }]
    })
}

#[tokio::test]
async fn the_hymn_library_is_downloaded_added_once_and_gated() {
    let good = serde_json::json!([
        hymn("pd-ag", "Amazing Grace", "John Newton", "Public Domain"),
        hymn("pd-hh", "Holy, Holy, Holy", "Reginald Heber", "Public Domain"),
    ]);
    let ok_url = serve("200 OK", good.to_string()).await;
    let bad_url = serve("200 OK", serde_json::json!([hymn("x", "A Modern Chorus", "Somebody", "© 2001 Somebody Music")]).to_string()).await;
    let gone_url = serve("404 Not Found", "nope".into()).await;

    let s = start().await;
    let http = reqwest::Client::new();
    let post = || http.post(s.url("/api/songs/public-domain")).header("x-host-token", &s.token).send();

    // Adding changes the library, so it needs the console's token.
    assert_eq!(http.post(s.url("/api/songs/public-domain")).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);

    // (The three cases run one after another in this one test because the download address is process-wide.)
    unsafe { std::env::set_var("OPENSANCTUARY_HYMNS_URL", &bad_url) };
    let r = post().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_GATEWAY);
    assert!(r.text().await.unwrap().contains("A Modern Chorus"));
    unsafe { std::env::set_var("OPENSANCTUARY_HYMNS_URL", &gone_url) };
    assert_eq!(post().await.unwrap().status(), reqwest::StatusCode::BAD_GATEWAY);
    assert!(s.db.get_songs().unwrap().is_empty(), "a refused download adds nothing");

    unsafe { std::env::set_var("OPENSANCTUARY_HYMNS_URL", &ok_url) };
    let j: serde_json::Value = post().await.unwrap().json().await.unwrap();
    assert_eq!((j["success"].as_bool(), j["added"].as_u64(), j["already_present"].as_u64()), (Some(true), Some(2), Some(0)));
    assert_eq!(s.db.get_songs().unwrap().len(), 2);

    // Again: nothing is added twice.
    let j: serde_json::Value = post().await.unwrap().json().await.unwrap();
    assert_eq!((j["added"].as_u64(), j["already_present"].as_u64()), (Some(0), Some(2)));
    assert_eq!(s.db.get_songs().unwrap().len(), 2);
}
