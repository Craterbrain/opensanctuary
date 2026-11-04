//! Timing for the library read paths, against real data: the repo's public-domain song file
//! and real Bibles from ../opensanctuary-bibles. Ignored by default:
//! `cargo test --release --test core_tests read_bench -- --ignored --nocapture`
use os_next::core::models::{ScriptureItem, ScriptureVerse};
use crate::open_db_with_dirs;
use std::path::Path;
use std::time::Instant;

fn time<T>(label: &str, n: u32, mut f: impl FnMut() -> T) -> T {
    let mut out = f(); // warm
    let t = Instant::now();
    for _ in 0..n { out = std::hint::black_box(f()); }
    println!("{label:<48} {:>10.3} ms", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
    out
}

#[test]
#[ignore = "timing, run with --ignored --nocapture"]
fn read_bench() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bibles_src = repo.join("../opensanctuary-bibles/bibles");
    let dir = std::env::temp_dir().join(format!("os-read-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("songs")).unwrap();
    std::fs::copy(repo.join("songs/public_domain.db"), dir.join("songs/public_domain.db")).unwrap();
    let db = open_db_with_dirs(dir.join("library.db"), dir.join("bibles"), dir.join("songs")).unwrap();
    let bibles = ["eng-kjv", "eng-web"];
    for b in &bibles {
        // The opensanctuary-bibles format: {"Book": {"1": {"1": "text", ...}, ...}, ...}
        let json: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&std::fs::read(bibles_src.join(format!("{b}.json"))).unwrap()).unwrap();
        let version = b.to_uppercase().replace('-', "");
        let mut items = Vec::new();
        for (book, chapters) in &json {
            for (ch, verses) in chapters.as_object().unwrap() {
                let mut vs: Vec<ScriptureVerse> = verses.as_object().unwrap().iter().filter_map(|(n, t)| Some(ScriptureVerse { verse_number: n.parse().ok()?, text: t.as_str()?.to_string() })).collect();
                vs.sort_by_key(|v| v.verse_number);
                let chapter: u32 = ch.parse().unwrap();
                let end = vs.last().map_or(1, |v| v.verse_number);
                items.push(ScriptureItem {
                    id: format!("scrip_{}_{}_{}", book.replace(' ', "_"), chapter, version),
                    book: book.clone(), chapter, verse_start: 1, verse_end: end, version: version.clone(),
                    reference: format!("{book} {chapter}:1-{end} ({version})"), verses: vs, theme_name: None,
                });
            }
        }
        db.insert_scriptures_batch(&items).unwrap();
    }
    println!("\n{} songs, bibles: {:?}", db.get_songs().unwrap().len(), bibles);

    let songs = time("get_songs", 50, || db.get_songs().unwrap());
    time("  + serde_json::to_vec(songs)", 50, || serde_json::to_vec(&songs).unwrap().len());
    let some_id = songs[songs.len() / 2].id.clone();
    time("get_song_by_id", 2000, || db.get_song_by_id(&some_id).unwrap());
    time("search_songs(\"grace\")", 50, || db.search_songs("grace").unwrap().len());
    let scriptures = time("get_scriptures (all chapters, all bibles)", 5, || db.get_scriptures().unwrap());
    let json_len = time("  + serde_json::to_vec(scriptures)", 5, || serde_json::to_vec(&scriptures).unwrap().len());
    println!("    {} chapters, {:.1} MB JSON", scriptures.len(), json_len as f64 / 1e6);
    time("get_scriptures_json (what /api/scriptures sends)", 5, || db.get_scriptures_json().unwrap().len());
    let v = bibles[0].to_uppercase().replace('-', "");
    time("get_scriptures_for_version", 5, || db.get_scriptures_for_version(&v).unwrap().len());
    let sid = scriptures.iter().find(|s| s.book == "John" && s.chapter == 3 && s.version.to_uppercase() == v).map(|s| s.id.clone()).unwrap();
    time("get_scripture_by_id (John 3)", 2000, || db.get_scripture_by_id(&sid).unwrap().is_some());
    time("get_passage (John 3)", 2000, || db.get_passage(&v, "John", 3).unwrap().is_some());
    time("search_scriptures(\"love\")", 5, || db.search_scriptures("love").unwrap().len());
    time("get_installed_bibles", 200, || db.get_installed_bibles().unwrap().len());
    time("get_themes", 2000, || db.get_themes().unwrap().len());
    time("get_presentations", 2000, || db.get_presentations().unwrap().len());
    time("get_media", 2000, || db.get_media().unwrap().len());
    let _ = std::fs::remove_dir_all(&dir);
}
