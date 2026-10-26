//! Licensed Bible text (YouVersion's NIV, ...) may be shown but never kept: not in the database, the
//! event log, a saved `.ewsx` or an exported `.osservice`. What survives is the reference, which is
//! enough to look the passage up again. (The sealed store itself is tested in `storage/db/licensed.rs`.)

use super::*;
use os_next::core::events::EventEnvelope;
use os_next::core::models::{LicensedRef, ScriptureVerse};
use os_next::storage::bundle;
use os_next::storage::db::LicensedMeta;
use std::io::{Cursor, Read};

const SECRET: &str = "ZZ-LICENSED-VERSE-TEXT-ZZ";

fn passage() -> (ScriptureItem, LicensedMeta) {
    let item = ScriptureItem {
        id: "lic_test1".into(), book: "John".into(), chapter: 3, verse_start: 16, verse_end: 17, version: "NIV".into(),
        reference: "John 3".into(),
        verses: vec![
            ScriptureVerse { verse_number: 16, text: format!("{SECRET} sixteen") },
            ScriptureVerse { verse_number: 17, text: format!("{SECRET} seventeen") },
        ],
        theme_name: None,
    };
    let meta = LicensedMeta {
        id: item.id.clone(), source: "youversion".into(), bible_id: 111, usfm: "JHN.3.16-17".into(), tag: "NIV \u{b7} \u{a9} Biblica".into(),
        copyright: "Copyright Biblica".into(), link: "https://www.biblica.com".into(), expires_at_ms: i64::MAX / 2,
    };
    (item, meta)
}

fn licensed_ref(item: &ScriptureItem, meta: &LicensedMeta) -> LicensedRef {
    LicensedRef {
        source: meta.source.clone(), bible_id: meta.bible_id, usfm: meta.usfm.clone(), tag: meta.tag.clone(), copyright: meta.copyright.clone(),
        link: meta.link.clone(), version: item.version.clone(), book: item.book.clone(), chapter: item.chapter, resolved: true,
    }
}

fn schedule_with_licensed() -> Schedule {
    let (item, meta) = passage();
    let sched_item = item.to_licensed_schedule_item(licensed_ref(&item, &meta));
    Schedule { id: "sched".into(), title: "Sunday".into(), items: vec![sched_item], selected_item_index: Some(0), is_modified: true, schedule_version: 1 }
}

/// Every file inside a zip, decompressed, as one searchable byte soup.
fn unzipped(bytes: &[u8]) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut all = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        all.extend_from_slice(&buf);
        all.push(b'\n');
    }
    all
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn a_licensed_slide_shows_the_notice_and_carries_its_reference() {
    let s = schedule_with_licensed();
    let item = &s.items[0];
    assert_eq!(item.item_type, "scripture");
    assert_eq!(item.slides.len(), 2);
    for (slide, n) in item.slides.iter().zip([16, 17]) {
        assert!(slide.text.starts_with(SECRET), "{}", slide.text);
        assert!(slide.text.ends_with("NIV \u{b7} \u{a9} Biblica"), "the notice is the last line of every slide: {}", slide.text);
        assert_eq!(slide.reference_label.as_deref(), Some(&*format!("John 3:{n} \u{b7} NIV \u{b7} \u{a9} Biblica")));
    }
    let r = item.licensed.as_ref().unwrap();
    assert_eq!((r.source.as_str(), r.bible_id, r.usfm.as_str(), r.resolved), ("youversion", 111, "JHN.3.16-17", true));
}

#[test]
fn stripped_copies_keep_the_reference_and_lose_the_text() {
    let s = schedule_with_licensed();
    let stripped = s.without_licensed_text();
    let json = serde_json::to_string(&stripped).unwrap();
    assert!(!json.contains(SECRET));
    let item = &stripped.items[0];
    assert!(!item.licensed.as_ref().unwrap().resolved);
    assert_eq!(item.licensed.as_ref().unwrap().usfm, "JHN.3.16-17");
    assert_eq!(item.slides.len(), 2, "the slides stay so the verses can be filled back in");
    assert!(item.slides[0].text.contains("John 3:16"));
    // The live copy is untouched.
    assert!(s.items[0].slides[0].text.contains(SECRET));
    // Nothing licensed: no change at all.
    let mut ordinary = s.items[0].clone();
    ordinary.licensed = None;
    let plain = Schedule { items: vec![ordinary], ..s.clone() };
    assert_eq!(plain.without_licensed_text(), plain);
}

#[test]
fn the_event_log_never_gets_the_text() {
    let db = Database::in_memory().unwrap();
    let s = schedule_with_licensed();
    let item = s.items[0].clone();
    let env = |n: u64, event: ShowEvent| EventEnvelope { sequence_number: n, event_id: format!("e{n}"), timestamp_ms: 1, event };
    db.save_event(&env(1, ShowEvent::ScheduleItemAdded { item: item.clone(), index: 0 })).unwrap();
    db.save_event(&env(2, ShowEvent::ItemWentLive { item: Some(item.clone()), slide_index: 0 })).unwrap();
    db.save_event(&env(3, ShowEvent::ItemStaged { item: Some(item.clone()), slide_index: 1 })).unwrap();
    db.save_event(&env(4, ShowEvent::ScheduleLoaded { schedule: s.clone() })).unwrap();
    let raw: Vec<String> = {
        let conn = db.conn();
        let mut stmt = conn.prepare("SELECT payload FROM event_log ORDER BY sequence_number").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().flatten().collect()
    };
    assert_eq!(raw.len(), 4);
    assert!(raw.iter().all(|p| !p.contains(SECRET)), "{raw:?}");
    // Replaying them gives back the reference and a placeholder, not an empty item.
    let events = db.load_events().unwrap();
    match &events[0].event {
        ShowEvent::ScheduleItemAdded { item, .. } => {
            assert_eq!(item.licensed.as_ref().unwrap().usfm, "JHN.3.16-17");
            assert!(item.slides[0].text.contains("fetching the text"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn saved_and_exported_schedules_carry_only_the_reference() {
    let s = schedule_with_licensed();

    let ewsx = EwsxManager::save_schedule_to_ewsx_bytes(&s).unwrap();
    let all = unzipped(&ewsx);
    assert!(!contains(&all, SECRET), "the .ewsx must not hold the text");
    assert!(contains(&all, "JHN.3.16-17"), "...but must hold the reference to look it up again");

    // Reopened, the item still knows what to fetch and is marked unresolved.
    let back = EwsxManager::load_schedule_from_bytes(&ewsx, "Sunday").unwrap();
    let r = back.items[0].licensed.as_ref().expect("licensed reference survives the round trip");
    assert_eq!((r.usfm.as_str(), r.bible_id, r.resolved), ("JHN.3.16-17", 111, false));

    let db = Database::in_memory().unwrap();
    let media = tempfile::tempdir().unwrap();
    let mut out = Cursor::new(Vec::new());
    bundle::export_service(&db, media.path(), &s, None, &mut out).unwrap();
    let all = unzipped(&out.into_inner());
    assert!(!contains(&all, SECRET), "the .osservice must not hold the text");
    assert!(contains(&all, "JHN.3.16-17"));
}

#[test]
fn a_reopened_schedule_gets_its_licensed_text_back_everywhere_it_is_shown() {
    use os_next::core::commands::ShowCommand;
    use os_next::core::models::LicensedVerse;

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);
    // What a reopened file gives: the reference and a placeholder.
    let stripped = schedule_with_licensed().without_licensed_text();
    let item_id = stripped.items[0].id.clone();
    engine.execute_command(ShowCommand::LoadSchedule(stripped));
    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(0) });
    let before = engine.snapshot();
    assert!(before.state.live_item.as_ref().unwrap().slides[0].text.contains("fetching the text"));
    assert!(!before.schedule.items[0].licensed.as_ref().unwrap().resolved);

    let verses = vec![
        LicensedVerse { verse_number: 16, text: "FIRST\n\nNIV".into() },
        LicensedVerse { verse_number: 17, text: "SECOND\n\nNIV".into() },
    ];
    let events = engine.execute_command(ShowCommand::SetLicensedPassage { item_id: item_id.clone(), verses: verses.clone() });
    assert_eq!(events.len(), 1);
    let after = engine.snapshot();
    assert_eq!(after.schedule.items[0].slides[1].text, "SECOND\n\nNIV");
    assert!(after.schedule.items[0].licensed.as_ref().unwrap().resolved);
    assert_eq!(after.state.live_item.as_ref().unwrap().slides[0].text, "FIRST\n\nNIV", "the live copy is updated too");
    assert!(!after.schedule.is_modified || before.schedule.is_modified, "looking text up is not an edit");

    // The stored form of that event carries no text, so a restart brings the placeholder back.
    let stored = serde_json::to_string(&*events[0].event.without_licensed_text()).unwrap();
    assert!(!stored.contains("FIRST") && !stored.contains("SECOND"), "{stored}");

    // Only licensed items can be filled in, and only ones that exist.
    let mut ordinary = schedule_with_licensed();
    ordinary.items[0].licensed = None;
    ordinary.items[0].id = "plain".into();
    engine.execute_command(ShowCommand::LoadSchedule(ordinary));
    assert!(engine.execute_command(ShowCommand::SetLicensedPassage { item_id: "plain".into(), verses: verses.clone() }).is_empty());
    assert!(engine.execute_command(ShowCommand::SetLicensedPassage { item_id: "missing".into(), verses }).is_empty());
}

#[test]
fn a_reference_only_result_becomes_slides_when_its_verses_arrive() {
    use os_next::core::commands::ShowCommand;
    use os_next::core::models::{LicensedVerse, ScheduleItem};

    let (item, meta) = passage();
    let r = licensed_ref(&item, &meta);
    // A verse range: one placeholder slide per verse asked for, no text, nothing resolved.
    let range = ScheduleItem::licensed_placeholder(r.clone(), 16, 18);
    assert_eq!(range.title, "John 3:16-18");
    assert_eq!(range.slides.iter().map(|s| s.tag.clone().unwrap()).collect::<Vec<_>>(), ["V16", "V17", "V18"]);
    assert!(range.slides.iter().all(|s| s.text.contains("fetching the text") && !s.text.contains(SECRET)));
    assert!(!range.licensed.as_ref().unwrap().resolved);
    assert!(range.id.starts_with("sched_lic_ref_"));
    // A whole chapter: one placeholder, however many verses the chapter turns out to have.
    let chapter = ScheduleItem::licensed_placeholder(r, 0, 0);
    assert_eq!((chapter.title.as_str(), chapter.slides.len()), ("John 3", 1));
    assert_ne!(chapter.id, ScheduleItem::licensed_placeholder(licensed_ref(&item, &meta), 0, 0).id, "each add is its own item");

    let engine = ShowEngine::new(Arc::new(EventLog::new(100)));
    engine.execute_command(ShowCommand::AddToSchedule(chapter.clone()));
    let verses: Vec<LicensedVerse> = (1..=5).map(|n| LicensedVerse { verse_number: n, text: format!("v{n}\n\nNIV") }).collect();
    engine.execute_command(ShowCommand::SetLicensedPassage { item_id: chapter.id.clone(), verses });
    let got = &engine.snapshot().schedule.items[0];
    assert_eq!(got.slides.len(), 5, "the one placeholder became the five verses");
    assert_eq!(got.slides[4].text, "v5\n\nNIV");
    assert_eq!(got.slides[2].header.as_deref(), Some("John 3:3"));
    assert_eq!(got.slides[2].reference_label.as_deref(), Some("John 3:3 \u{b7} NIV \u{b7} \u{a9} Biblica"));
    assert_eq!(got.arrangement.len(), 5);
    assert!(got.licensed.as_ref().unwrap().resolved);
}
