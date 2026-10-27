//! The bundled public-domain hymn library.
//!
//! `assets/public_domain_songs.json` is compiled into the program, so every package (the `.deb`, the Windows installer, the tarball, the
//! Flatpak) carries it without any file having to land in a particular folder (the per-user songs folder is where the app reads songs from,
//! so a file shipped beside the program would never be seen). Hymns are added to the library by id and by title+author, never overwriting
//! a song that is already there: your edits and your own copies win.
//!
//! What is in it, and why anything was left out, is in `docs/public-domain-songs.md`. The rule is evidence, not a blank copyright field:
//! a song is included only when it has a named historical author (a text from before about 1930) or is a clearly ancient anonymous
//! carol or hymn.

use std::collections::HashSet;

use serde::Serialize;

use crate::core::models::Song;
use crate::storage::Database;

const LIBRARY_JSON: &str = include_str!("../../assets/public_domain_songs.json");

/// Remembers that the first-run seeding was decided (done, or skipped because the library already had songs), so it never runs again.
const SEEDED_SETTING: &str = "publicDomainHymnsSeeded";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SeedReport {
    /// Hymns in the bundled library.
    pub total: usize,
    /// Added to the song library just now.
    pub added: usize,
    /// Left alone because the library already has that hymn (same id, or same title and author).
    pub already_present: usize,
}

/// The bundled hymns.
pub fn library() -> Result<Vec<Song>, String> {
    serde_json::from_str(LIBRARY_JSON).map_err(|e| format!("the bundled hymn library is damaged: {e}"))
}

fn key(title: &str, author: &str) -> (String, String) {
    (title.trim().to_lowercase(), author.trim().to_lowercase())
}

/// The ids and the (title, author) pairs of the songs the library already has.
type Held = (HashSet<String>, HashSet<(String, String)>);

fn existing(db: &Database) -> Result<Held, String> {
    let songs = db.get_songs().map_err(|e| e.to_string())?;
    Ok((songs.iter().map(|s| s.id.clone()).collect(), songs.iter().map(|s| key(&s.title, &s.author)).collect()))
}

/// How many of the bundled hymns the library does not have yet.
pub fn missing(db: &Database) -> Result<SeedReport, String> {
    let hymns = library()?;
    let (ids, keys) = existing(db)?;
    let present = hymns.iter().filter(|h| ids.contains(&h.id) || keys.contains(&key(&h.title, &h.author))).count();
    Ok(SeedReport { total: hymns.len(), added: 0, already_present: present })
}

/// Adds every bundled hymn the library doesn't have.
pub fn seed(db: &Database) -> Result<SeedReport, String> {
    let hymns = library()?;
    let (ids, mut keys) = existing(db)?;
    let (mut added, mut already) = (0, 0);
    for h in &hymns {
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

/// First run: a brand-new library (no songs at all) gets the hymns once. A library that already has songs is left as it is, and either way
/// this is decided only once, so deleting hymns later never brings them back by itself (Import > Public-domain hymns adds them on request).
pub fn seed_if_fresh(db: &Database) -> Result<Option<SeedReport>, String> {
    if db.get_setting(SEEDED_SETTING).map_err(|e| e.to_string())?.is_some() {
        return Ok(None);
    }
    let fresh = db.get_songs().map_err(|e| e.to_string())?.is_empty();
    let report = if fresh { Some(seed(db)?) } else { None };
    db.set_setting(SEEDED_SETTING, if fresh { "seeded" } else { "skipped" }).map_err(|e| e.to_string())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_library_is_well_formed() {
        let hymns = library().unwrap();
        assert!(hymns.len() > 100, "got {}", hymns.len());
        let mut ids = HashSet::new();
        let mut keys = HashSet::new();
        for h in &hymns {
            assert!(!h.title.trim().is_empty() && !h.author.trim().is_empty(), "{h:?}");
            assert!(ids.insert(h.id.clone()), "duplicate id {}", h.id);
            assert!(keys.insert(key(&h.title, &h.author)), "duplicate song {} / {}", h.title, h.author);
            assert_eq!(h.copyright.as_deref(), Some("Public Domain"), "{} must say so", h.title);
            assert!(h.ccli_number.is_none(), "{}: a public-domain hymn has no CCLI number", h.title);
            let words: usize = h.slides.iter().map(|s| s.text.split_whitespace().count()).sum();
            assert!(words >= 12, "{} has almost no lyrics ({words} words)", h.title);
            assert!(Database::is_public_domain_song(h), "{} would be filed as copyrighted", h.title);
        }
    }

    #[test]
    fn nothing_without_a_named_author_or_a_stated_basis_and_no_test_data_is_in_it() {
        // The rule (docs/public-domain-songs.md): a named historical author, or an anonymous/traditional text whose age is stated in `basis`.
        let raw: Vec<serde_json::Value> = serde_json::from_str(LIBRARY_JSON).unwrap();
        for h in &raw {
            let title = h["title"].as_str().unwrap_or_default();
            let author = h["author"].as_str().unwrap_or_default().to_lowercase();
            let basis = h["basis"].as_str().unwrap_or_default();
            let unnamed = ["", "author unknown", "unknown", "custom", "media ministry"].contains(&author.trim());
            let vague = author.split(|c: char| !c.is_alphabetic()).any(|w| ["anonymous", "traditional", "spiritual", "carol"].contains(&w));
            assert!(!unnamed || basis.len() > 10, "{title} has no named author and no stated basis");
            assert!(!vague || basis.len() > 10, "{title}: an anonymous or traditional text must say how we know it is old (the `basis` field)");
            let t = title.to_lowercase();
            assert!(!t.contains("test") && t != "ufu", "test data: {title}");
            // Known not-public-domain titles that must never come back (see the doc's "left out" list).
            for banned in ["here i am to worship", "god of grace and god of glory", "he is lord", "bless the lord, o my soul", "the virgin mary had a baby boy"] {
                assert_ne!(t, banned);
            }
            let lyrics = h["slides"].as_array().unwrap().iter().filter_map(|s| s["text"].as_str()).collect::<Vec<_>>().join(" ").to_lowercase();
            for marker in ["©", "ccli", "all rights reserved", "used by permission"] {
                assert!(!lyrics.contains(marker), "{title} carries a rights notice in its lyrics");
            }
        }
        // Every song added from the Cyber Hymnal names where it came from.
        for h in raw.iter().filter(|h| h.get("source").is_some()) {
            assert!(h["basis"].as_str().is_some_and(|b| !b.is_empty()), "{} has a source but no basis", h["title"]);
        }
    }

    #[test]
    fn seeding_adds_everything_once_and_never_overwrites_or_duplicates() {
        let db = Database::in_memory().unwrap();
        let total = library().unwrap().len();
        let r = seed(&db).unwrap();
        assert_eq!((r.total, r.added, r.already_present), (total, total, 0));
        assert_eq!(db.get_songs().unwrap().len(), total);

        // Your edit wins, and a second run adds nothing.
        let mut mine = db.get_song_by_id(&library().unwrap()[0].id).unwrap().unwrap();
        mine.author = "Edited by my church".into();
        db.insert_song(&mine).unwrap();
        let r = seed(&db).unwrap();
        assert_eq!((r.added, r.already_present), (0, total), "same id: left alone even though the author text differs now");
        assert_eq!(db.get_song_by_id(&mine.id).unwrap().unwrap().author, "Edited by my church");
        assert_eq!(missing(&db).unwrap().already_present, total);
    }

    #[test]
    fn a_hymn_you_already_have_under_another_id_is_not_added_twice() {
        let db = Database::in_memory().unwrap();
        let mut ag = library().unwrap().into_iter().find(|h| h.title == "Amazing Grace").unwrap();
        ag.id = "my_own_id".into();
        db.insert_song(&ag).unwrap();
        let r = seed(&db).unwrap();
        assert_eq!(r.already_present, 1);
        assert_eq!(db.get_songs().unwrap().iter().filter(|s| s.title == "Amazing Grace").count(), 1);
    }

    #[test]
    fn first_run_seeds_a_fresh_library_once_and_leaves_a_used_one_alone() {
        let fresh = Database::in_memory().unwrap();
        assert!(seed_if_fresh(&fresh).unwrap().is_some());
        let n = fresh.get_songs().unwrap().len();
        // Deleting one never brings it back on its own: the question was decided once.
        let one = fresh.get_songs().unwrap()[0].id.clone();
        fresh.delete_song(&one).unwrap();
        assert!(seed_if_fresh(&fresh).unwrap().is_none());
        assert_eq!(fresh.get_songs().unwrap().len(), n - 1);

        let used = Database::in_memory().unwrap();
        let mut own = library().unwrap().remove(0);
        own.id = "mine".into();
        own.title = "My church's own song".into();
        used.insert_song(&own).unwrap();
        assert!(seed_if_fresh(&used).unwrap().is_none());
        assert_eq!(used.get_songs().unwrap().len(), 1, "a library that already has songs is not touched");
        assert!(seed_if_fresh(&used).unwrap().is_none());
    }
}
