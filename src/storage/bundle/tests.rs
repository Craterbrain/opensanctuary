use super::*;
use crate::core::models::{CcliMetadata, ScheduleItem, Slide, SlideBackground, SlideElement};
use std::io::Cursor;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake-but-recognisable-png-bytes";
const OTHER_PNG: &[u8] = b"\x89PNG\r\n\x1a\nsome-other-picture";
const MP4: &[u8] = b"\0\0\0\x18ftypmp42\0\0\0\0video-bytes-here";

/// A media folder holding the files the tests refer to.
fn library_with_media() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("images")).unwrap();
    std::fs::create_dir_all(dir.path().join("videos")).unwrap();
    std::fs::write(dir.path().join("images/Easter Sunrise.png"), PNG).unwrap();
    std::fs::write(dir.path().join("images/Photo (1).png"), OTHER_PNG).unwrap();
    std::fs::write(dir.path().join("videos/loop.mp4"), MP4).unwrap();
    dir
}

fn slide(text: &str, background: Option<&str>) -> Slide {
    Slide { text: text.to_string(), background: background.map(str::to_string), ..Default::default() }
}

fn song(id: &str, title: &str, copyright: Option<&str>, ccli: Option<&str>, slides: Vec<Slide>) -> Song {
    Song {
        id: id.to_string(),
        title: title.to_string(),
        alternate_title: None,
        author: "A. Writer".to_string(),
        copyright: copyright.map(str::to_string),
        ccli_number: ccli.map(str::to_string),
        slides,
        public_domain_override: None,
        theme_name: None,
    }
}

fn export_to_vec(f: impl FnOnce(&mut Cursor<Vec<u8>>) -> Result<ExportReport, BundleError>) -> (Vec<u8>, ExportReport) {
    let mut cur = Cursor::new(Vec::new());
    let report = f(&mut cur).expect("export");
    (cur.into_inner(), report)
}

fn reopen(bytes: &[u8]) -> Bundle<Cursor<&[u8]>> {
    open(Cursor::new(bytes), None).expect("open")
}

#[test]
fn media_references_are_found_wherever_they_sit() {
    let refs = |s: &str| find_media_refs(s).into_iter().map(|r| (r.kind, r.file)).collect::<Vec<_>>();
    assert_eq!(refs("/media/images/bg.jpg"), vec![(MediaKind::Image, "bg.jpg".to_string())]);
    assert_eq!(refs("url('/media/images/bg.jpg')"), vec![(MediaKind::Image, "bg.jpg".to_string())]);
    assert_eq!(refs("url(/media/videos/loop.MP4)"), vec![(MediaKind::Video, "loop.MP4".to_string())]);
    assert_eq!(refs("linear-gradient(#000, #111), url(\"/media/images/a.webp\") center / cover"), vec![(MediaKind::Image, "a.webp".to_string())]);
    // The plugin host names collisions "name (2).ext": the parentheses belong to the name.
    assert_eq!(refs("/media/images/Photo (1).png"), vec![(MediaKind::Image, "Photo (1).png".to_string())]);
    assert_eq!(refs("url('/media/images/Photo (1).png')"), vec![(MediaKind::Image, "Photo (1).png".to_string())]);
    // Two references in one string, unquoted.
    assert_eq!(
        refs("url(/media/images/a.png), url(/media/audio/b.mp3)"),
        vec![(MediaKind::Image, "a.png".to_string()), (MediaKind::Audio, "b.mp3".to_string())]
    );
    // A query string stays out of the name.
    assert_eq!(refs("/media/images/a.png?v=3"), vec![(MediaKind::Image, "a.png".to_string())]);
    // Not references: other folders, other schemes, an extension that merely starts a longer one.
    assert!(refs("/media/other/a.png").is_empty());
    assert!(refs("https://example.com/media/images/").is_empty());
    assert!(refs("/media/images/a.pngx").is_empty());
    assert!(refs("plain lyrics, no path").is_empty());
    assert!(refs("#102027").is_empty());
}

#[test]
fn only_real_files_inside_their_folder_become_tokens() {
    let lib = library_with_media();
    let mut c = Collector::new(lib.path());
    // Exists: tokenized. Missing: left alone and reported. Climbing out of the folder: refused.
    assert!(c.tokenize_string("/media/images/Easter Sunrise.png").unwrap().starts_with(TOKEN_PREFIX));
    assert_eq!(c.tokenize_string("/media/images/gone.png"), None);
    assert_eq!(c.tokenize_string("/media/images/../videos/loop.mp4"), None);
    assert_eq!(c.tokenize_string("/media/images/.hidden.png"), None);
    assert!(c.missing.contains("/media/images/gone.png"));
    // A file that isn't really an image is not bundled whatever it's called.
    std::fs::write(lib.path().join("images/fake.png"), b"<html><script>alert(1)</script></html>").unwrap();
    assert_eq!(c.tokenize_string("/media/images/fake.png"), None);
    // The same file under two paths is stored once.
    std::fs::copy(lib.path().join("images/Easter Sunrise.png"), lib.path().join("images/copy.png")).unwrap();
    c.tokenize_string("/media/images/copy.png").unwrap();
    assert_eq!(c.entries.len(), 1);
}

#[test]
fn tokens_come_back_as_the_importing_librarys_paths() {
    let id = "a".repeat(64);
    let mut v = serde_json::json!({ "bg": format!("url('{TOKEN_PREFIX}{id}')"), "other": format!("{TOKEN_PREFIX}{}", "b".repeat(64)), "n": 3 });
    let mut paths = HashMap::new();
    paths.insert(id, "/media/images/x-aaaaaaaa.png".to_string());
    detokenize(&mut v, &paths);
    assert_eq!(v["bg"], "url('/media/images/x-aaaaaaaa.png')");
    // A token the bundle didn't define is left as it is, never turned into a path.
    assert!(v["other"].as_str().unwrap().starts_with(TOKEN_PREFIX));
    assert_eq!(v["n"], 3);
}

#[test]
fn a_song_with_pictures_survives_a_round_trip_into_another_library() {
    let lib_a = library_with_media();
    let db_a = Database::in_memory().unwrap();
    let mut s = song(
        "song_1",
        "Amazing Grace",
        Some("Public Domain"),
        None,
        vec![slide("Amazing grace", Some("url('/media/images/Easter Sunrise.png')")), slide("How sweet", Some("/media/videos/loop.mp4"))],
    );
    s.slides[0].elements.push(SlideElement::Image {
        id: "img1".into(),
        transform: Default::default(),
        file_path: "/media/images/Photo (1).png".into(),
        crop: None,
        mask_shape: None,
        alt_text: None,
    });
    s.slides[1].background_v2 = Some(SlideBackground::Image { file_path: "/media/images/Easter Sunrise.png".into(), opacity: 1.0 });
    db_a.insert_song(&s).unwrap();

    let (bytes, report) = export_to_vec(|out| export_songs(&db_a, lib_a.path(), None, out));
    assert_eq!(report.media_count, 3, "two pictures and a video, each stored once even though one picture is used twice");
    assert!(report.missing_media.is_empty());

    // Another machine: an empty library and an empty media folder.
    let lib_b = tempfile::tempdir().unwrap();
    let db_b = Database::in_memory().unwrap();
    let mut bundle = reopen(&bytes);
    let summary = inspect(&bundle, &db_b, lib_b.path()).unwrap();
    assert_eq!(summary.kind, BundleKind::Song);
    assert_eq!(summary.names, vec!["Amazing Grace".to_string()]);
    assert_eq!(summary.library.as_ref().unwrap().new, 1);
    assert!(summary.copyrighted.is_empty(), "public domain, so no advisory");

    let result = import(&mut bundle, &db_b, lib_b.path(), &ImportOptions::default()).unwrap();
    assert_eq!((result.added, result.media_written), (1, 3));
    // The two pictures (not the video) are listed in the Media library, so they can be picked as backgrounds elsewhere.
    assert_eq!(result.backgrounds_added, 2);
    let listed = db_b.get_media().unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|m| m.media_type == "image" && m.file_path.starts_with("/media/images/")));
    assert!(listed.iter().any(|m| m.name == "Easter Sunrise"), "{:?}", listed.iter().map(|m| &m.name).collect::<Vec<_>>());

    let got = db_b.get_song_by_id("song_1").unwrap().unwrap();
    assert_eq!(got.title, "Amazing Grace");
    // Every reference now points at a real file in lib_b holding the original bytes.
    let bg0 = got.slides[0].background.clone().unwrap();
    let refs = find_media_refs(&bg0);
    assert_eq!(refs.len(), 1);
    let on_disk = lib_b.path().join(refs[0].kind.dir()).join(&refs[0].file);
    assert_eq!(std::fs::read(on_disk).unwrap(), PNG);
    assert!(bg0.starts_with("url('/media/images/Easter_Sunrise-") && bg0.ends_with(".png')"), "{bg0}");
    let SlideElement::Image { file_path, .. } = &got.slides[0].elements[0] else { panic!("lost the image element") };
    assert_eq!(std::fs::read(lib_b.path().join("images").join(file_path.rsplit('/').next().unwrap())).unwrap(), OTHER_PNG);
    let Some(SlideBackground::Image { file_path, .. }) = &got.slides[1].background_v2 else { panic!("lost the structured background") };
    assert!(file_path.starts_with("/media/images/Easter_Sunrise-"));
    assert_eq!(std::fs::read(lib_b.path().join("videos").join(got.slides[1].background.as_ref().unwrap().rsplit('/').next().unwrap())).unwrap(), MP4);

    // Importing it again changes nothing: same song, same media, same names.
    let mut again = reopen(&bytes);
    let summary = inspect(&again, &db_b, lib_b.path()).unwrap();
    assert_eq!(summary.library.as_ref().unwrap().identical, 1);
    let result = import(&mut again, &db_b, lib_b.path(), &ImportOptions::default()).unwrap();
    assert_eq!((result.added, result.skipped_identical, result.media_written, result.media_reused), (0, 1, 0, 3));
    assert_eq!(result.backgrounds_added, 0, "a picture already listed is not listed twice");
    assert_eq!(db_b.get_media().unwrap().len(), 2);
    assert_eq!(db_b.get_songs().unwrap().len(), 1);
}

#[test]
fn importing_into_the_library_it_came_from_adds_nothing_new() {
    let lib = library_with_media();
    let db = Database::in_memory().unwrap();
    db.insert_song(&song("song_1", "Same Library", None, None, vec![slide("x", Some("/media/images/Easter Sunrise.png"))])).unwrap();
    let (bytes, _) = export_to_vec(|out| export_songs(&db, lib.path(), None, out));
    let mut bundle = reopen(&bytes);
    let status = inspect(&bundle, &db, lib.path()).unwrap().library.unwrap();
    assert_eq!((status.new, status.identical, status.different), (0, 1, 0));
    let result = import(&mut bundle, &db, lib.path(), &ImportOptions::default()).unwrap();
    assert_eq!((result.added, result.skipped_identical, result.added_as_copies), (0, 1, 0));
    assert_eq!(db.get_songs().unwrap().len(), 1);
}

#[test]
fn a_different_song_with_the_same_id_is_kept_as_a_copy_never_overwritten() {
    let lib = library_with_media();
    let source = Database::in_memory().unwrap();
    source.insert_song(&song("song_1", "Their Version", None, None, vec![slide("their words", None)])).unwrap();
    let (bytes, _) = export_to_vec(|out| export_songs(&source, lib.path(), None, out));

    let mine = Database::in_memory().unwrap();
    mine.insert_song(&song("song_1", "My Version", None, None, vec![slide("my words", None)])).unwrap();
    let mut bundle = reopen(&bytes);
    assert_eq!(inspect(&bundle, &mine, lib.path()).unwrap().library.unwrap().different, 1);
    let result = import(&mut bundle, &mine, lib.path(), &ImportOptions::default()).unwrap();
    assert_eq!((result.added, result.added_as_copies), (0, 1));

    let all = mine.get_songs().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(mine.get_song_by_id("song_1").unwrap().unwrap().title, "My Version", "the original is untouched");
    assert!(all.iter().any(|s| s.title == "Their Version" && s.id != "song_1"));
}

#[test]
fn copyrighted_songs_are_listed_for_the_advisory_and_public_domain_ones_are_not() {
    let lib = tempfile::tempdir().unwrap();
    let db = Database::in_memory().unwrap();
    db.insert_song(&song("s1", "10,000 Reasons", Some("© 2011 Worship Co"), Some("6016351"), vec![slide("a", None)])).unwrap();
    db.insert_song(&song("s2", "Old Hymn", Some("Public Domain"), None, vec![slide("b", None)])).unwrap();
    db.insert_song(&song("s3", "No Metadata", None, None, vec![slide("c", None)])).unwrap();
    let (bytes, _) = export_to_vec(|out| export_songs(&db, lib.path(), None, out));
    let summary = inspect(&reopen(&bytes), &Database::in_memory().unwrap(), lib.path()).unwrap();
    assert_eq!(summary.copyrighted.len(), 1);
    assert_eq!(summary.copyrighted[0].title, "10,000 Reasons");
    assert_eq!(summary.copyrighted[0].ccli_number, "6016351");
    assert_eq!(summary.names.len(), 3);
}

#[test]
fn a_service_carries_its_items_media_and_copyright_notes() {
    let lib = library_with_media();
    let db = Database::in_memory().unwrap();
    let mut item = ScheduleItem {
        id: "item_1".into(),
        title: "Opening Song".into(),
        subtitle: None,
        notes: None,
        item_type: "song".into(),
        author_or_ref: "Someone".into(),
        slides: vec![slide("la la", Some("/media/videos/loop.mp4"))],
        background: Some("/media/videos/loop.mp4".into()),
        theme_name: None,
        is_expanded: false,
        is_section_header: false,
        arrangement: vec![crate::core::models::ArrangementEntry { section_id: "V1".into(), source_slide_index: 0, background_override: Some("/media/images/Photo (1).png".into()) }],
        default_slide_duration_seconds: None,
        slideshow_loop: false,
        licensed: None,
    };
    item.slides[0].ccli_metadata = Some(CcliMetadata { song_title: None, author: Some("Someone".into()), copyright: Some("© 2020 Label".into()), ccli_number: Some("123".into()), public_domain: None });
    let schedule = Schedule { id: "sch".into(), title: "Sunday".into(), items: vec![item], selected_item_index: None, is_modified: true, schedule_version: 4 };

    let (bytes, report) = export_to_vec(|out| export_service(&db, lib.path(), &schedule, Some("Easter Service"), out));
    assert_eq!(report.media_count, 2, "the loop is used by the item, its slide, and nothing is stored twice");
    let bundle = reopen(&bytes);
    assert_eq!(bundle.manifest.title, "Easter Service");

    let dest = tempfile::tempdir().unwrap();
    let dest_db = Database::in_memory().unwrap();
    let summary = inspect(&bundle, &dest_db, dest.path()).unwrap();
    assert_eq!(summary.copyrighted.len(), 1, "a service's songs carry their notice on their slides");
    assert_eq!(summary.copyrighted[0].ccli_number, "123");

    let mut bundle = reopen(&bytes);
    let result = import(&mut bundle, &dest_db, dest.path(), &ImportOptions::default()).unwrap();
    let got = result.schedule.expect("a service comes back for the console to load");
    assert_eq!(got.title, "Easter Service");
    assert!(!got.is_modified);
    let bg = got.items[0].background.clone().unwrap();
    assert!(bg.starts_with("/media/videos/loop-"), "{bg}");
    assert_eq!(std::fs::read(dest.path().join("videos").join(bg.rsplit('/').next().unwrap())).unwrap(), MP4);
    assert!(got.items[0].arrangement[0].background_override.as_ref().unwrap().starts_with("/media/images/Photo_1-"));
    assert!(dest_db.get_songs().unwrap().is_empty(), "a service is loaded, not filed into the song library");
}

#[test]
fn custom_themes_travel_and_existing_themes_win() {
    let lib = tempfile::tempdir().unwrap();
    let db = Database::in_memory().unwrap();
    let theme = Theme { id: None, name: "Advent Purple".into(), background: "#301934".into(), ..Theme::default() };
    db.insert_theme(&theme).unwrap();
    let mut s = song("s1", "Advent Song", None, None, vec![slide("x", None)]);
    s.theme_name = Some("Advent Purple".into());
    db.insert_song(&s).unwrap();
    let (bytes, _) = export_to_vec(|out| export_songs(&db, lib.path(), None, out));
    assert_eq!(reopen(&bytes).manifest.themes, vec!["Advent Purple".to_string()]);

    let other = Database::in_memory().unwrap();
    let result = import(&mut reopen(&bytes), &other, lib.path(), &ImportOptions::default()).unwrap();
    assert_eq!(result.themes_added, vec!["Advent Purple".to_string()]);
    assert_eq!(other.get_theme_by_name("Advent Purple").unwrap().unwrap().background, "#301934");

    // A library that already has a theme of that name keeps its own.
    let mine = Database::in_memory().unwrap();
    mine.insert_theme(&Theme { id: None, name: "Advent Purple".into(), background: "#000000".into(), ..Theme::default() }).unwrap();
    let result = import(&mut reopen(&bytes), &mine, lib.path(), &ImportOptions::default()).unwrap();
    assert!(result.themes_added.is_empty());
    assert_eq!(mine.get_theme_by_name("Advent Purple").unwrap().unwrap().background, "#000000");
}

#[test]
fn presentations_keep_pictures_inside_groups() {
    let lib = library_with_media();
    let db = Database::in_memory().unwrap();
    let mut s = slide("", None);
    s.elements.push(SlideElement::Group {
        id: "g".into(),
        transform: Default::default(),
        children: vec![SlideElement::Video {
            id: "v".into(),
            transform: Default::default(),
            file_path: "/media/videos/loop.mp4".into(),
            in_point_s: None,
            out_point_s: None,
            loop_playback: true,
            is_muted: false,
            volume: 1.0,
        }],
    });
    let p = Presentation { id: "p1".into(), title: "Welcome".into(), author: "".into(), slides: vec![s], theme_name: None };
    db.insert_presentation(&p).unwrap();
    let (bytes, report) = export_to_vec(|out| export_presentations(&db, lib.path(), None, out));
    assert_eq!(report.media_count, 1);

    let dest = tempfile::tempdir().unwrap();
    let dest_db = Database::in_memory().unwrap();
    import(&mut reopen(&bytes), &dest_db, dest.path(), &ImportOptions::default()).unwrap();
    let got = dest_db.get_presentation_by_id("p1").unwrap().unwrap();
    let SlideElement::Group { children, .. } = &got.slides[0].elements[0] else { panic!() };
    let SlideElement::Video { file_path, .. } = &children[0] else { panic!() };
    assert_eq!(std::fs::read(dest.path().join("videos").join(file_path.rsplit('/').next().unwrap())).unwrap(), MP4);
}

fn sample_bible(db: &Database) {
    let item = |book: &str, chapter: u32, verses: &[&str]| ScriptureItem {
        id: format!("{book}-{chapter}"),
        book: book.into(),
        chapter,
        verse_start: 1,
        verse_end: verses.len() as u32,
        version: "TST".into(),
        reference: format!("{book} {chapter}"),
        verses: verses.iter().enumerate().map(|(i, t)| ScriptureVerse { verse_number: i as u32 + 1, text: t.to_string() }).collect(),
        theme_name: None,
    };
    // Genesis first, John second -- and "Genesis" does not sort before "John" only by luck of the alphabet
    // in the other direction: Revelation is added before Acts to prove order is install order, not sorted.
    db.insert_scriptures_batch(&[item("Genesis", 1, &["In the beginning", "And the earth"]), item("Revelation", 22, &["Amen"]), item("Acts", 1, &["The former treatise"])]).unwrap();
}

#[test]
fn a_bible_round_trips_in_its_own_book_order() {
    let src = Database::in_memory().unwrap();
    sample_bible(&src);
    src.set_setting(&bible_info_key("TST"), r#"{"abbreviation":"TST","name":"Test Version","language":"English","license":"Public Domain","attribution":"None needed"}"#).unwrap();
    let (bytes, report) = export_to_vec(|out| export_bible(&src, "tst", out));
    assert_eq!(report.media_count, 0);

    let dest = Database::in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut bundle = reopen(&bytes);
    let summary = inspect(&bundle, &dest, dir.path()).unwrap();
    assert_eq!((summary.kind, summary.counts.chapters, summary.bible_installed), (BundleKind::Bible, 3, false));
    assert_eq!(summary.bible.as_ref().unwrap().license, "Public Domain");

    let result = import(&mut bundle, &dest, dir.path(), &ImportOptions::default()).unwrap();
    assert_eq!((result.added, result.bible.as_deref()), (3, Some("TST")));
    let books: Vec<String> = dest.get_scriptures_for_version("TST").unwrap().into_iter().map(|c| c.book).collect();
    assert_eq!(books, ["Genesis", "Revelation", "Acts"], "install order is kept, not alphabetical");
    let genesis = dest.get_passage("TST", "Genesis", 1).unwrap().unwrap();
    assert_eq!(genesis.verses[1].text, "And the earth");
    // The translation's details are kept for the next export.
    assert_eq!(read_bible_info(&dest, "TST").name, "Test Version");
    // Importing the same translation again needs an explicit replace.
    let again = import(&mut reopen(&bytes), &dest, dir.path(), &ImportOptions::default());
    assert!(matches!(again, Err(BundleError::AlreadyInstalled(v)) if v == "TST"));
    assert!(inspect(&reopen(&bytes), &dest, dir.path()).unwrap().bible_installed);
    import(&mut reopen(&bytes), &dest, dir.path(), &ImportOptions { replace_bible: true }).unwrap();
    assert_eq!(dest.get_scriptures_for_version("TST").unwrap().len(), 3);
}

#[test]
fn exporting_something_that_is_not_there_is_an_error() {
    let db = Database::in_memory().unwrap();
    let lib = tempfile::tempdir().unwrap();
    assert!(matches!(export_to_vec_err(|o| export_bible(&db, "NOPE", o)), BundleError::NotFound(_)));
    assert!(matches!(export_to_vec_err(|o| export_songs(&db, lib.path(), Some(&["missing".to_string()]), o)), BundleError::NotFound(_)));
    assert!(matches!(export_to_vec_err(|o| export_songs(&db, lib.path(), None, o)), BundleError::Invalid(_)));
}

fn export_to_vec_err(f: impl FnOnce(&mut Cursor<Vec<u8>>) -> Result<ExportReport, BundleError>) -> BundleError {
    f(&mut Cursor::new(Vec::new())).expect_err("should have failed")
}

// ---------------------------------------------------------------------------------------------
// Hostile and damaged files
// ---------------------------------------------------------------------------------------------

fn raw_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, FileOptions::<()>::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn manifest_json(kind: &str, version: u32, media: &str) -> String {
    format!(r#"{{"format":"{FORMAT_NAME}","format_version":{version},"kind":"{kind}","media":[{media}]}}"#)
}

fn id_of(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn err_of(bytes: &[u8]) -> String {
    match open(Cursor::new(bytes), None) {
        Ok(_) => panic!("should have been refused"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn files_that_are_not_ours_are_refused_with_a_clear_reason() {
    assert!(err_of(b"just some text").contains("isn't an OpenSanctuary file"));
    assert!(err_of(&raw_zip(&[("hello.txt", b"hi")])).contains("unexpected entry"));
    assert!(err_of(&raw_zip(&[("content.json", b"{}")])).contains("no manifest"));
    let wrong_format = br#"{"format":"something-else","format_version":1,"kind":"song"}"#;
    assert!(err_of(&raw_zip(&[("manifest.json", wrong_format), ("content.json", b"{}")])).contains("isn't an OpenSanctuary file"));
}

#[test]
fn a_newer_format_is_refused_rather_than_guessed_at() {
    let bytes = raw_zip(&[("manifest.json", manifest_json("song", FORMAT_VERSION + 1, "").as_bytes()), ("content.json", b"{}")]);
    let e = open(Cursor::new(&bytes[..]), None).err().unwrap();
    assert!(matches!(e, BundleError::UnsupportedVersion { found, .. } if found == FORMAT_VERSION + 1));
    assert!(e.to_string().contains("newer OpenSanctuary"));
}

#[test]
fn entry_names_that_could_escape_a_folder_are_refused_outright() {
    for evil in ["../evil.txt", "media/../../evil.png", "/etc/passwd", "media/..\\evil.png", "media/x.png", "media/ABCD.png", "themes/x.json"] {
        let bytes = raw_zip(&[("manifest.json", manifest_json("song", 1, "").as_bytes()), ("content.json", br#"{"songs":[]}"#), (evil, b"x")]);
        assert!(err_of(&bytes).contains("unexpected entry"), "{evil} should be refused");
    }
}

#[test]
fn a_kind_mismatch_is_named() {
    let (bytes, _) = {
        let lib = tempfile::tempdir().unwrap();
        let db = Database::in_memory().unwrap();
        db.insert_song(&song("s", "S", None, None, vec![slide("x", None)])).unwrap();
        export_to_vec(|out| export_songs(&db, lib.path(), None, out))
    };
    let e = open(Cursor::new(&bytes[..]), Some(BundleKind::Bible)).err().unwrap();
    assert_eq!(e.to_string(), "that is a song file, not a Bible file");
}

#[test]
fn media_that_lies_about_itself_is_refused_and_nothing_is_kept() {
    let dest = tempfile::tempdir().unwrap();
    let db = Database::in_memory().unwrap();
    let content = br#"{"songs":[{"id":"s","title":"T","author":"","slides":[{"text":"x","header":null,"label":null,"background":null}]}]}"#;

    // 1. Bytes that don't hash to their id.
    let real = PNG.to_vec();
    let id = id_of(&real);
    let entry = format!(r#"{{"id":"{id}","kind":"image","ext":"png","name":"a.png","size":{}}}"#, real.len());
    let tampered = raw_zip(&[("manifest.json", manifest_json("song", 1, &entry).as_bytes()), ("content.json", content), (&format!("media/{id}.png"), b"\x89PNG\r\n\x1a\nTAMPERED")]);
    let e = import(&mut reopen(&tampered), &db, dest.path(), &ImportOptions::default()).err().unwrap();
    assert!(e.to_string().contains("damaged"), "{e}");

    // 2. Script dressed as a picture, with a correct hash and a plausible name.
    let script = b"<html><script>alert(document.domain)</script></html>".to_vec();
    let sid = id_of(&script);
    let entry = format!(r#"{{"id":"{sid}","kind":"image","ext":"png","name":"cute.png","size":{}}}"#, script.len());
    let disguised = raw_zip(&[("manifest.json", manifest_json("song", 1, &entry).as_bytes()), ("content.json", content), (&format!("media/{sid}.png"), &script)]);
    let e = import(&mut reopen(&disguised), &db, dest.path(), &ImportOptions::default()).err().unwrap();
    assert!(e.to_string().contains("isn't the kind of"), "{e}");

    // 3. A video presented as audio.
    let vid = MP4.to_vec();
    let vid_id = id_of(&vid);
    let entry = format!(r#"{{"id":"{vid_id}","kind":"audio","ext":"mp3","name":"song.mp3","size":{}}}"#, vid.len());
    let wrong_kind = raw_zip(&[("manifest.json", manifest_json("song", 1, &entry).as_bytes()), ("content.json", content), (&format!("media/{vid_id}.mp3"), &vid)]);
    assert!(import(&mut reopen(&wrong_kind), &db, dest.path(), &ImportOptions::default()).is_err());

    // None of it was kept, and nothing reached the library.
    let leftovers: Vec<_> = walk(dest.path());
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    assert!(db.get_songs().unwrap().is_empty());
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn the_media_list_and_the_archive_must_agree() {
    let id = id_of(PNG);
    let entry = format!(r#"{{"id":"{id}","kind":"image","ext":"png","name":"a.png","size":{}}}"#, PNG.len());
    let content: &[u8] = br#"{"songs":[]}"#;
    // Listed but absent.
    let missing = raw_zip(&[("manifest.json", manifest_json("song", 1, &entry).as_bytes()), ("content.json", content)]);
    assert!(err_of(&missing).contains("doesn't contain it"));
    // Present but unlisted: a payload the list doesn't mention.
    let unlisted = raw_zip(&[("manifest.json", manifest_json("song", 1, "").as_bytes()), ("content.json", content), (&format!("media/{id}.png"), PNG)]);
    assert!(err_of(&unlisted).contains("doesn't mention"));
    // A listed id that isn't a hash.
    let bad_id = r#"{"id":"../../x","kind":"image","ext":"png","name":"a.png","size":1}"#;
    assert!(err_of(&raw_zip(&[("manifest.json", manifest_json("song", 1, bad_id).as_bytes()), ("content.json", content)])).contains("invalid entry"));
}

#[test]
fn a_bible_with_absurd_chapters_is_refused_before_anything_is_stored() {
    let db = Database::in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let long = "x".repeat(MAX_VERSE_TEXT + 1);
    let content = serde_json::json!({
        "bible": { "abbreviation": "BAD", "name": "Bad" },
        "chapters": [{ "book": "Genesis", "chapter": 1, "verses": [{ "verse_number": 1, "text": long }] }]
    });
    let bytes = raw_zip(&[("manifest.json", manifest_json("bible", 1, "").as_bytes()), ("content.json", content.to_string().as_bytes())]);
    assert!(import(&mut reopen(&bytes), &db, dir.path(), &ImportOptions::default()).is_err());
    assert!(db.get_installed_bibles().unwrap().is_empty());

    let bad_abbr = serde_json::json!({ "bible": { "abbreviation": "../x" }, "chapters": [{ "book": "A", "chapter": 1, "verses": [{ "verse_number": 1, "text": "t" }] }] });
    let bytes = raw_zip(&[("manifest.json", manifest_json("bible", 1, "").as_bytes()), ("content.json", bad_abbr.to_string().as_bytes())]);
    assert!(import(&mut reopen(&bytes), &db, dir.path(), &ImportOptions::default()).is_err());
}

#[test]
fn restored_names_are_safe_readable_and_stable() {
    let entry = |name: &str| MediaEntry { id: "0123456789abcdef".repeat(4), kind: MediaKind::Image, ext: "png".into(), name: name.into(), size: 1 };
    assert_eq!(restored_name(&entry("Easter Sunrise.png")), "Easter_Sunrise-01234567.png");
    assert_eq!(restored_name(&entry("../../etc/passwd")), "passwd-01234567.png");
    assert_eq!(restored_name(&entry("..")), "media-01234567.png");
    // Whether `\` splits a path depends on the OS, so assert the property that matters on both: whatever
    // the name was, the result is one plain file name.
    for nasty in ["a b/c\\d.png", "..\\..\\x.png", "con:\\nul.png", "a\0b.png", "a\nb.png", "%2e%2e%2fx.png"] {
        let n = restored_name(&entry(nasty));
        assert!(!n.contains(['/', '\\', '\0', '\n', ':', '%']) && safe_file_name(&n) && n.ends_with("-01234567.png"), "{nasty:?} -> {n:?}");
    }
    assert_eq!(restored_name(&entry("Gracia Maravillosa ✝.png")), "Gracia_Maravillosa-01234567.png");
    assert!(restored_name(&entry(&"long".repeat(100))).len() < 70);
}

#[test]
fn extensions_map_to_kinds_and_back() {
    for k in [BundleKind::Song, BundleKind::Bible, BundleKind::Presentation, BundleKind::Service] {
        assert_eq!(BundleKind::from_extension(k.extension()), Some(k));
        assert_eq!(BundleKind::from_extension(&format!(".{}", k.extension().to_uppercase())), Some(k));
    }
    // The ones other programs own are not ours.
    for taken in ["osp", "osb", "osz", "oss", "otz"] {
        assert_eq!(BundleKind::from_extension(taken), None, ".{taken}");
    }
}
