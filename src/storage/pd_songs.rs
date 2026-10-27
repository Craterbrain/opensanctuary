//! The public-domain hymn library, downloaded on request.
//!
//! The hymns live in the Bibles repository (`songs/public-domain-hymns.json`, Craterbrain/opensanctuary-bibles), not in the program: the first-run
//! wizard and Import > Public-domain hymns fetch the file and add the hymns by id and by title+author, never overwriting a song that is already
//! there, so your edits and your own copies win. Nothing is fetched while you run a service.
//!
//! What is in it, and why anything was left out, is in that repository's `SONGS.md` (mirrored in `docs/public-domain-songs.md`). The downloaded
//! file is checked again here before anything is stored: every song must say "Public Domain" and carry no CCLI number.

use std::collections::HashSet;

use serde::Serialize;

use crate::core::models::Song;
use crate::storage::Database;

const DEFAULT_URL: &str = "https://raw.githubusercontent.com/Craterbrain/opensanctuary-bibles/main/songs/public-domain-hymns.json";
/// Tests (and anyone mirroring the library) point the download somewhere else with this.
const URL_ENV: &str = "OPENSANCTUARY_HYMNS_URL";
/// The real file is ~0.4 MB; this only stops a runaway response.
const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SeedReport {
    /// Hymns in the downloaded library.
    pub total: usize,
    /// Added to the song library just now.
    pub added: usize,
    /// Left alone because the library already has that hymn (same id, or same title and author).
    pub already_present: usize,
}

/// Downloads the hymn library.
pub async fn download() -> Result<Vec<Song>, String> {
    let url = std::env::var(URL_ENV).ok().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| DEFAULT_URL.to_string());
    let client = reqwest::Client::builder()
        .user_agent(concat!("opensanctuary/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("couldn't start the download: {e}"))?;
    let resp = client.get(&url).send().await.map_err(|e| format!("couldn't reach the hymn library ({e}); check the internet connection"))?;
    if !resp.status().is_success() {
        return Err(format!("the hymn library couldn't be downloaded (HTTP {})", resp.status().as_u16()));
    }
    let body = resp.bytes().await.map_err(|e| format!("the download was interrupted: {e}"))?;
    if body.len() > MAX_BYTES {
        return Err("the hymn library is larger than expected; not using it".into());
    }
    parse(&body)
}

/// Reads and checks a downloaded library: nothing that isn't marked public domain is accepted.
pub fn parse(bytes: &[u8]) -> Result<Vec<Song>, String> {
    let hymns: Vec<Song> = serde_json::from_slice(bytes).map_err(|e| format!("the downloaded hymn library is damaged: {e}"))?;
    if hymns.is_empty() {
        return Err("the downloaded hymn library is empty".into());
    }
    for h in &hymns {
        if h.title.trim().is_empty() || h.id.trim().is_empty() {
            return Err("the downloaded hymn library has a hymn with no title".into());
        }
        if h.copyright.as_deref() != Some("Public Domain") || h.ccli_number.is_some() {
            return Err(format!("\"{}\" isn't marked public domain; the download was refused", h.title));
        }
    }
    Ok(hymns)
}

fn key(title: &str, author: &str) -> (String, String) {
    (title.trim().to_lowercase(), author.trim().to_lowercase())
}

/// Adds every hymn the library doesn't have, never overwriting or duplicating.
pub fn seed(db: &Database, hymns: &[Song]) -> Result<SeedReport, String> {
    let songs = db.get_songs().map_err(|e| e.to_string())?;
    let ids: HashSet<String> = songs.iter().map(|s| s.id.clone()).collect();
    let mut keys: HashSet<(String, String)> = songs.iter().map(|s| key(&s.title, &s.author)).collect();
    let (mut added, mut already) = (0, 0);
    for h in hymns {
        let k = key(&h.title, &h.author);
        if ids.contains(&h.id) || keys.contains(&k) {
            already += 1;
            continue;
        }
        db.insert_song(h).map_err(|e| format!("couldn't add \"{}\": {e}", h.title))?;
        keys.insert(k);
        added += 1;
    }
    Ok(SeedReport { total: hymns.len(), added, already_present: already })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hymn(id: &str, title: &str, author: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "title": title, "author": author, "copyright": "Public Domain", "ccli_number": null, "theme_name": null,
            "slides": [{ "id": format!("{id}-1"), "text": "Amazing grace how sweet the sound that saved a wretch like me", "label": "Verse 1" }]
        })
    }
    fn library() -> Vec<Song> {
        let v = serde_json::json!([hymn("a", "Amazing Grace", "John Newton"), hymn("b", "Holy, Holy, Holy", "Reginald Heber")]);
        parse(v.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn a_download_that_is_not_public_domain_is_refused() {
        let mut bad = hymn("c", "Some Chorus", "Someone");
        bad["copyright"] = "© 2001 Somebody Music".into();
        let err = parse(serde_json::json!([hymn("a", "Amazing Grace", "John Newton"), bad]).to_string().as_bytes()).unwrap_err();
        assert!(err.contains("Some Chorus"), "{err}");
        let mut ccli = hymn("d", "Another", "Someone");
        ccli["ccli_number"] = "12345".into();
        assert!(parse(serde_json::json!([ccli]).to_string().as_bytes()).is_err());
        assert!(parse(b"[]").is_err());
        assert!(parse(b"<html>not found</html>").unwrap_err().contains("damaged"));
    }

    #[test]
    fn seeding_adds_everything_once_and_never_overwrites_or_duplicates() {
        let db = Database::in_memory().unwrap();
        let lib = library();
        let r = seed(&db, &lib).unwrap();
        assert_eq!((r.total, r.added, r.already_present), (2, 2, 0));

        // Your edit wins, and a second run adds nothing.
        let mut mine = db.get_song_by_id("a").unwrap().unwrap();
        mine.author = "Edited by my church".into();
        db.insert_song(&mine).unwrap();
        let r = seed(&db, &lib).unwrap();
        assert_eq!((r.added, r.already_present), (0, 2), "same id: left alone even though the author text differs now");
        assert_eq!(db.get_song_by_id("a").unwrap().unwrap().author, "Edited by my church");
    }

    #[test]
    fn a_hymn_you_already_have_under_another_id_is_not_added_twice() {
        let db = Database::in_memory().unwrap();
        let mut ag = library().remove(0);
        ag.id = "my_own_id".into();
        db.insert_song(&ag).unwrap();
        let r = seed(&db, &library()).unwrap();
        assert_eq!((r.added, r.already_present), (1, 1));
        assert_eq!(db.get_songs().unwrap().iter().filter(|s| s.title == "Amazing Grace").count(), 1);
    }
}
