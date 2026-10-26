//! Importing a Bible from a link or a file over HTTP: the token gate, USFM/OSIS/Zefania, replace policy, and the
//! refusals that need no network (the link route never reaches the internet for a link it won't follow).

use super::bundles::{start, Server};
use std::io::Write;

const USFM: &str = "\\id JHN Test\n\\c 3\n\\p\n\\v 16 For God so loved the world.\n\\v 17 For God did not send his Son.\n";

async fn post_file(server: &Server, query: &str, bytes: Vec<u8>, token: bool) -> reqwest::Response {
    let mut r = reqwest::Client::new().post(server.url(&format!("/api/bibles/import/file?{query}"))).body(bytes);
    if token {
        r = r.header("x-host-token", &server.token);
    }
    r.send().await.unwrap()
}

async fn post_link(server: &Server, body: serde_json::Value, token: bool) -> reqwest::Response {
    let mut r = reqwest::Client::new().post(server.url("/api/bibles/import/link")).json(&body);
    if token {
        r = r.header("x-host-token", &server.token);
    }
    r.send().await.unwrap()
}

fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (n, t) in files {
        w.start_file(*n, zip::write::FileOptions::<()>::default()).unwrap();
        w.write_all(t.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

#[tokio::test]
async fn a_bible_file_is_read_and_installed_and_replacing_needs_asking() {
    let s = start().await;
    // No token, no import.
    assert_eq!(post_file(&s, "name=a.usfm", USFM.as_bytes().to_vec(), false).await.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(s.db.get_installed_bibles().unwrap().is_empty());

    let r = post_file(&s, "name=test-web.usfm&abbreviation=tst", USFM.as_bytes().to_vec(), true).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!((body["bible"]["abbreviation"].as_str(), body["bible"]["format"].as_str(), body["bible"]["verses"].as_u64()), (Some("TST"), Some("USFM"), Some(2)));
    assert!(s.db.get_installed_bibles().unwrap().iter().any(|b| b.id == "TST"));
    assert_eq!(s.db.get_scriptures().unwrap().len(), 1);

    // Again: 409 with what the file holds, nothing replaced.
    let r = post_file(&s, "name=x.usfm&abbreviation=TST", "\\id GEN\n\\c 1\n\\v 1 Changed.\n".as_bytes().to_vec(), true).await;
    assert_eq!(r.status(), reqwest::StatusCode::CONFLICT);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!((body["code"].as_str(), body["abbreviation"].as_str()), (Some("already_installed"), Some("TST")));
    assert_eq!(s.db.get_scriptures().unwrap()[0].book, "John");

    // With replace=true the old chapters are gone.
    let r = post_file(&s, "name=x.usfm&abbreviation=TST&replace=true", "\\id GEN\n\\c 1\n\\v 1 Changed.\n".as_bytes().to_vec(), true).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    assert_eq!(s.db.get_scriptures().unwrap().iter().map(|x| x.book.clone()).collect::<Vec<_>>(), ["Genesis"]);
}

#[tokio::test]
async fn zip_osis_and_zefania_come_in_the_same_way() {
    let s = start().await;
    let z = zip_of(&[("a/JHN.usfm", USFM), ("readme.txt", "hello")]);
    assert_eq!(post_file(&s, "name=ebible.zip&abbreviation=ZIPB", z, true).await.status(), reqwest::StatusCode::OK);
    let osis = r#"<osis><osisText osisIDWork="OSB"><div type="book" osisID="Gen"><chapter osisID="Gen.1"><verse osisID="Gen.1.1">In the beginning.</verse></chapter></div></osisText></osis>"#;
    let r = post_file(&s, "name=o.xml", osis.as_bytes().to_vec(), true).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    assert_eq!(r.json::<serde_json::Value>().await.unwrap()["bible"]["abbreviation"], "OSB", "the abbreviation comes from the file when none is given");
    let zef = r#"<XMLBIBLE biblename="Z"><BIBLEBOOK bnumber="1"><CHAPTER cnumber="1"><VERS vnumber="1">In the beginning.</VERS></CHAPTER></BIBLEBOOK></XMLBIBLE>"#;
    assert_eq!(post_file(&s, "name=zz.xml", zef.as_bytes().to_vec(), true).await.status(), reqwest::StatusCode::OK);
    let ids: Vec<String> = s.db.get_installed_bibles().unwrap().into_iter().map(|b| b.id).collect();
    for want in ["ZIPB", "OSB", "ZZ"] {
        assert!(ids.iter().any(|i| i == want), "{want} in {ids:?}");
    }
}

#[tokio::test]
async fn files_that_are_not_bibles_leave_nothing_behind() {
    let s = start().await;
    for (name, bytes) in [("a.txt", b"hello".to_vec()), ("b.zip", b"PK\x03\x04junk".to_vec()), ("c.xml", b"<feed/>".to_vec()), ("d.usfm", b"\\id GEN\n\\c 1\n".to_vec())] {
        let r = post_file(&s, &format!("name={name}"), bytes, true).await;
        assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST, "{name}");
        assert!(!r.json::<serde_json::Value>().await.unwrap()["error"].as_str().unwrap_or("").is_empty());
    }
    assert!(s.db.get_installed_bibles().unwrap().is_empty());
}

#[tokio::test]
async fn the_link_route_refuses_what_it_must_without_touching_the_network() {
    let s = start().await;
    assert_eq!(post_link(&s, serde_json::json!({ "url": "https://ebible.org/a.zip" }), false).await.status(), reqwest::StatusCode::UNAUTHORIZED);
    for url in [
        "http://ebible.org/a.zip", "https://127.0.0.1/a.zip", "https://localhost/a.zip", "https://[::1]/x", "https://169.254.169.254/latest/meta-data/",
        "https://10.1.2.3/a.zip", "https://user:pw@ebible.org/a.zip", "https://ebible.org:8443/a.zip", "file:///etc/passwd", "not a url",
    ] {
        let r = post_link(&s, serde_json::json!({ "url": url }), true).await;
        assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST, "{url}");
        assert!(!r.json::<serde_json::Value>().await.unwrap()["error"].as_str().unwrap_or("").is_empty(), "{url}");
    }
    assert!(s.db.get_installed_bibles().unwrap().is_empty());
}
