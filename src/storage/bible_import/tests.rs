use super::*;
use std::io::Write;

const USFM_GEN: &str = "\\id GEN Test Bible\n\\h Genesis\n\\mt1 Genesis\n\\c 1\n\\s1 A heading that is not text\n\\p\n\\v 1 In the \\w beginning|strong=\"H7225\"\\w* God created the heavens and the earth.\\f + \\fr 1.1 \\ft a note\\f*\n\\v 2 The earth was formless\n\\q1 and void,\n\\q2 and darkness \\add was\\add* over the deep.\n\\c 2\n\\p\n\\v 1 Thus the heavens were finished.\n";
const USFM_JHN: &str = "\\id JHN\n\\c 3\n\\p\n\\v 16 For God so loved the world.\n\\v 17-18 For God did not send his Son.\n";

fn osis() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<osis xmlns="http://www.bibletechnologies.net/2003/OSIS/namespace"><osisText osisIDWork="TST" xml:lang="en">
<header><work osisWork="TST"><title>Test Bible</title><rights>Public Domain</rights></work></header>
<div type="book" osisID="Matt"><chapter osisID="Matt.1"><verse sID="Matt.1.1" osisID="Matt.1.1"/>The book of <name>Jesus</name><verse eID="Matt.1.1"/> <verse sID="Matt.1.2" osisID="Matt.1.2"/>Abraham begat Isaac.<verse eID="Matt.1.2"/></chapter></div>
<div type="book" osisID="Gen"><chapter osisID="Gen.1"><title>Not a verse</title><verse osisID="Gen.1.1">In the <w lemma="strong:H7225">beginning</w> God created.<note type="study">skip <w>me</w></note></verse><verse osisID="Gen.1.2">The earth was <transChange type="added">without</transChange> form.</verse></chapter></div>
</osisText></osis>"#
        .to_string()
}

fn zefania() -> String {
    r#"<?xml version="1.0"?><XMLBIBLE biblename="Zef Bible" type="x"><INFORMATION><title>Zef</title><identifier>ZEF</identifier><language>ENG</language><rights>Free</rights></INFORMATION>
<BIBLEBOOK bnumber="43" bname="John"><CHAPTER cnumber="3"><VERS vnumber="16">For God so loved</VERS></CHAPTER></BIBLEBOOK>
<BIBLEBOOK bnumber="1" bname="Genesis"><CHAPTER cnumber="1"><VERS vnumber="1">In the beginning<NOTE>skip this</NOTE> God</VERS><VERS vnumber="2">Formless &amp; void</VERS></CHAPTER></BIBLEBOOK></XMLBIBLE>"#
        .to_string()
}

fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in files {
        w.start_file(*name, zip::write::FileOptions::<()>::default()).unwrap();
        w.write_all(text.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn verse<'a>(b: &'a ParsedBible, book: &str, chapter: u32, verse: u32) -> Option<&'a str> {
    b.chapters.iter().find(|c| c.book == book && c.chapter == chapter)?.verses.iter().find(|v| v.verse_number == verse).map(|v| v.text.as_str())
}

#[test]
fn usfm_footnotes_headings_poetry_and_word_attributes() {
    let b = parse_bible(USFM_GEN.as_bytes(), "kjv-test.usfm").unwrap();
    assert_eq!(b.format, Format::Usfm);
    assert_eq!(b.abbreviation, "KJV-TEST");
    assert_eq!(verse(&b, "Genesis", 1, 1), Some("In the beginning God created the heavens and the earth."));
    assert_eq!(verse(&b, "Genesis", 1, 2), Some("The earth was formless and void, and darkness was over the deep."));
    assert_eq!(verse(&b, "Genesis", 2, 1), Some("Thus the heavens were finished."));
    assert_eq!(b.chapters.len(), 2);
    assert!(!b.chapters.iter().flat_map(|c| &c.verses).any(|v| v.text.contains("heading") || v.text.contains("note") || v.text.contains('\\')));
}

#[test]
fn usfm_nested_word_markup_and_footnotes_inside_words_of_jesus_like_ebible_publishes() {
    // eBible's "with Strong's" files: `\wj` around `\+w word|strong="G1"\+w*`, with a footnote in the middle.
    let usfm = "\\id JHN\n\\c 3\n\\p\n\\v 16 \\wj \\+w For|strong=\"G1063\"\\+w* \\+w God|strong=\"G2316\"\\+w* \\+w so|strong=\"G3779\"\\+w* loved the world, born\\wj*\\f + \\fr 3:16 \\ft The Greek word \\+w x|strong=\"G1\"\\+w*.\\f* \\wj \\+w Son|strong=\"G5207\"\\+w*, \\+w that|strong=\"G2443\"\\+w* whoever believes. \\wj*\n\\v 17 Don\u{2019}\\+w t|strong=\"G3588\"\\+w* send.\n";
    let b = parse_bible(usfm.as_bytes(), "w.usfm").unwrap();
    assert_eq!(verse(&b, "John", 3, 16), Some("For God so loved the world, born Son, that whoever believes."));
    assert_eq!(verse(&b, "John", 3, 17), Some("Don\u{2019}t send."));
    assert!(!b.chapters.iter().flat_map(|c| &c.verses).any(|v| v.text.contains("strong") || v.text.contains('\\') || v.text.contains("Greek word")));
}

#[test]
fn usfm_books_in_one_zip_come_out_in_canonical_order_and_verse_ranges_use_their_first_number() {
    let z = zip_of(&[("43-JHNtest.usfm", USFM_JHN), ("02-GENtest.usfm", USFM_GEN), ("README.md", "not a bible"), ("images/x.png", "\u{89}PNG")]);
    let b = parse_bible(&z, "test_bible.zip").unwrap();
    assert_eq!(b.format, Format::Usfm);
    assert_eq!(b.chapters.iter().map(|c| (c.book.as_str(), c.chapter)).collect::<Vec<_>>(), [("Genesis", 1), ("Genesis", 2), ("John", 3)]);
    assert_eq!(verse(&b, "John", 3, 17), Some("For God did not send his Son."));
    assert_eq!(b.abbreviation, "TEST_BIBLE");
}

#[test]
fn osis_container_and_milestone_verses_notes_and_details() {
    let b = parse_bible(osis().as_bytes(), "whatever.xml").unwrap();
    assert_eq!(b.format, Format::Osis);
    assert_eq!((b.abbreviation.as_str(), b.name.as_str(), b.language.as_str(), b.license.as_str()), ("TST", "Test Bible", "en", "Public Domain"));
    assert_eq!(verse(&b, "Genesis", 1, 1), Some("In the beginning God created."));
    assert_eq!(verse(&b, "Genesis", 1, 2), Some("The earth was without form."));
    assert_eq!(verse(&b, "Matthew", 1, 1), Some("The book of Jesus"));
    assert_eq!(verse(&b, "Matthew", 1, 2), Some("Abraham begat Isaac."));
    assert_eq!(b.chapters[0].book, "Genesis", "canonical order, not the file's");
    assert!(!b.chapters.iter().flat_map(|c| &c.verses).any(|v| v.text.contains("Not a verse") || v.text.contains("skip")));
}

#[test]
fn zefania_details_notes_and_numeric_books() {
    let b = parse_bible(zefania().as_bytes(), "x.xml").unwrap();
    assert_eq!(b.format, Format::Zefania);
    assert_eq!((b.abbreviation.as_str(), b.name.as_str(), b.language.as_str()), ("ZEF", "Zef Bible", "ENG"));
    assert_eq!(verse(&b, "Genesis", 1, 1), Some("In the beginning God"));
    assert_eq!(verse(&b, "Genesis", 1, 2), Some("Formless & void"));
    assert_eq!(verse(&b, "John", 3, 16), Some("For God so loved"));
    assert_eq!(b.chapters[0].book, "Genesis");
}

#[test]
fn the_format_comes_from_the_content_and_utf16_and_a_bom_are_read() {
    let mut utf16 = vec![0xff, 0xfe];
    for u in USFM_JHN.encode_utf16() {
        utf16.extend_from_slice(&u.to_le_bytes());
    }
    assert_eq!(parse_bible(&utf16, "a.bin").unwrap().format, Format::Usfm);
    let mut bom = vec![0xef, 0xbb, 0xbf];
    bom.extend_from_slice(zefania().as_bytes());
    assert_eq!(parse_bible(&bom, "noext").unwrap().format, Format::Zefania);
    // A zip holding one XML Bible.
    assert_eq!(parse_bible(&zip_of(&[("bible.xml", &osis())]), "b.zip").unwrap().format, Format::Osis);
}

#[test]
fn things_that_are_not_bibles_are_refused_plainly() {
    for (name, bytes) in [
        ("empty", Vec::new()),
        ("text", b"Hello, world".to_vec()),
        ("json", br#"{"a":1}"#.to_vec()),
        ("other xml", b"<?xml version=\"1.0\"?><feed><entry>x</entry></feed>".to_vec()),
        ("usfm without verses", b"\\id GEN\n\\c 1\n".to_vec()),
        ("bad zip", b"PK\x03\x04garbage".to_vec()),
        ("zip of nothing useful", zip_of(&[("a.png", "x")])),
    ] {
        let e = parse_bible(&bytes, "f").unwrap_err();
        assert!(!e.0.is_empty(), "{name}");
    }
}

#[test]
fn hostile_xml_is_read_as_text_not_expanded() {
    // A "billion laughs" DOCTYPE: skipped, never expanded; the entity reference stays literal text.
    let evil = r#"<?xml version="1.0"?><!DOCTYPE lol [<!ENTITY lol "lollollollollollol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">]>
<XMLBIBLE biblename="E"><BIBLEBOOK bnumber="1"><CHAPTER cnumber="1"><VERS vnumber="1">&lol2; &#65; &#xZZ; &amp;amp; end</VERS></CHAPTER></BIBLEBOOK></XMLBIBLE>"#;
    let b = parse_bible(evil.as_bytes(), "e.xml").unwrap();
    assert_eq!(verse(&b, "Genesis", 1, 1), Some("&lol2; A &#xZZ; &amp; end"));
    // Truncated, unbalanced and deeply nested input must not hang or panic.
    let deep = format!("<XMLBIBLE><BIBLEBOOK bnumber=\"1\"><CHAPTER cnumber=\"1\"><VERS vnumber=\"1\">{}x", "<b>".repeat(50_000));
    let b = parse_bible(deep.as_bytes(), "d.xml").unwrap_err();
    assert!(!b.0.is_empty());
    assert!(parse_bible(b"<osis><osisText><div type=\"book\" osisID=\"Gen\"><chapter osisID=\"Gen.1\"><verse osisID=\"Gen.1.1\">half", "t.xml").is_ok());
}

#[test]
fn limits_are_enforced() {
    let long = "x".repeat(MAX_VERSE_BYTES + 1);
    let usfm = format!("\\id GEN\n\\c 1\n\\v 1 {long}\n");
    assert!(parse_bible(usfm.as_bytes(), "l.usfm").unwrap_err().0.contains("longer than"));
    let many: String = (1..=MAX_VERSES_PER_CHAPTER + 1).map(|v| format!("\\v {v} word\n")).collect();
    assert!(parse_bible(format!("\\id GEN\n\\c 1\n{many}").as_bytes(), "m.usfm").unwrap_err().0.contains("more than"));
    let files: Vec<(String, String)> = (0..MAX_ZIP_ENTRIES + 1).map(|i| (format!("f{i}.usfm"), USFM_JHN.to_string())).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert!(parse_bible(&zip_of(&refs), "z.zip").unwrap_err().0.contains("more than"));
    assert!(parse_bible(&vec![b'a'; MAX_INPUT_BYTES + 1], "big").unwrap_err().0.contains("larger than"));
}

#[test]
fn a_default_abbreviation_comes_from_the_file_name_without_the_format() {
    for (file, want) in [("engwebp_usfm.zip", "ENGWEBP"), ("eng-web_usfm.zip", "ENG-WEB"), ("KJV.osis.xml", "KJV"), ("My Bible.zip", "MYBIBLE"), ("_usfm.zip", "_USFM"), ("", "")] {
        assert_eq!(name_hint(file), want, "{file}");
    }
}

#[test]
fn abbreviations_are_cleaned() {
    assert_eq!(clean_abbreviation("  kjv 1769!  "), "KJV1769");
    assert_eq!(clean_abbreviation("../../etc"), "ETC");
    assert_eq!(clean_abbreviation(&"a".repeat(80)).len(), 40);
    assert_eq!(clean_abbreviation("???"), "");
}

#[test]
fn installing_writes_the_chapters_the_way_the_plugin_host_does_and_replace_is_explicit() {
    let db = Database::in_memory().unwrap();
    let b = parse_bible(USFM_GEN.as_bytes(), "x.usfm").unwrap();
    let got = install(&db, &b, "tst", false, "https://example.org/b.zip").unwrap();
    assert_eq!((got.abbreviation.as_str(), got.chapters, got.verses), ("TST", 2, 3));
    assert!(is_installed(&db, "TST").unwrap());
    let rows = db.get_scriptures().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.version == "TST" && r.id.starts_with("scrip_")));
    let again = install(&db, &b, "TST", false, "").unwrap_err();
    assert!(again.0.contains("already installed"));
    // Replace: the old chapters go, so a smaller Bible leaves nothing of the old one behind.
    let smaller = parse_bible(USFM_JHN.as_bytes(), "j.usfm").unwrap();
    install(&db, &smaller, "TST", true, "").unwrap();
    let rows = db.get_scriptures().unwrap();
    assert_eq!(rows.iter().map(|r| r.book.as_str()).collect::<Vec<_>>(), ["John"]);
    assert!(install(&db, &b, "???", false, "").is_err());
    let info = db.get_setting("bibleInfo_TST").unwrap().unwrap();
    assert!(info.contains("\"abbreviation\":\"TST\""));
}
