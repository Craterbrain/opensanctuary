//! Import and export: EasyWorship (.ews/.ewsx/.ewpx), OpenLP, FreeShow, PowerPoint, OpenDocument; media sniffing and fuzz regressions.

use super::*;

// ---------------------------------------------------------------------------------------------
// ODP export: the write side. Validated two ways -- round-tripped through this app's own
// importer, and (if `soffice` is on PATH) handed to real LibreOffice to open/convert, which is a
// much stronger check than anything this app's own (necessarily lenient) reader could give.
// ---------------------------------------------------------------------------------------------

fn sample_presentation_for_export(media_root: &std::path::Path, image_bytes: &[u8]) -> os_next::core::models::Presentation {
    use os_next::core::models::*;
    std::fs::create_dir_all(media_root.join("images")).unwrap();
    std::fs::write(media_root.join("images/photo.png"), image_bytes).unwrap();

    let text_block = SlideElement::TextBlock {
        id: "el_1".to_string(),
        transform: ElementTransform { x: 0.1, y: 0.1, w: 0.5, h: 0.2, z_index: 0, ..Default::default() },
        block: TextBlock {
            runs: vec![
                TextRun { text: "Plain ".to_string(), ..Default::default() },
                TextRun { text: "Bold Red".to_string(), bold: true, italic: true, underline: true, strike: true, color: "#ff00aa".to_string(), font_family: "Georgia".to_string(), font_size_pt: 40.0, ..Default::default() },
                TextRun { text: "\n".to_string(), ..Default::default() },
                TextRun { text: "Second line".to_string(), ..Default::default() },
            ],
            paragraph_style: TextParagraphStyle { align: "right".to_string(), ..Default::default() },
            effects: ElementEffects::default(),
            autofit: false,
        },
    };
    let image = SlideElement::Image {
        id: "el_2".to_string(),
        transform: ElementTransform { x: 0.2, y: 0.3, w: 0.3, h: 0.3, z_index: 1, ..Default::default() },
        file_path: "/media/images/photo.png".to_string(),
        crop: None,
        mask_shape: None,
        alt_text: None,
    };
    // A dropped reference (the file doesn't exist) must not break export, just skip that element.
    let missing_image = SlideElement::Image {
        id: "el_3".to_string(),
        transform: ElementTransform::default(),
        file_path: "/media/images/does_not_exist.png".to_string(),
        crop: None,
        mask_shape: None,
        alt_text: None,
    };

    let shape_rect = SlideElement::Shape {
        id: "el_5".to_string(),
        transform: ElementTransform { x: 0.05, y: 0.1, w: 0.2, h: 0.15, z_index: 1, ..Default::default() },
        shape_kind: "rect".to_string(),
        fill_color: "#aabbcc".to_string(),
        stroke_color: Some("#112233".to_string()),
        stroke_width: 4.0,
    };
    let shape_triangle = SlideElement::Shape {
        id: "el_6".to_string(),
        transform: ElementTransform { x: 0.3, y: 0.1, w: 0.2, h: 0.15, z_index: 2, ..Default::default() },
        shape_kind: "triangle".to_string(),
        fill_color: "#00ff00".to_string(),
        stroke_color: None,
        stroke_width: 0.0,
    };
    let line_arrow = SlideElement::Line {
        id: "el_7".to_string(),
        transform: ElementTransform { x: 0.05, y: 0.4, w: 0.3, h: 0.1, z_index: 3, ..Default::default() },
        line_kind: "straight".to_string(),
        color: "#ff8800".to_string(),
        stroke_width: 2.0,
        start_arrow: false,
        end_arrow: true,
    };
    let line_elbow = SlideElement::Line {
        id: "el_8".to_string(),
        transform: ElementTransform { x: 0.4, y: 0.4, w: 0.3, h: 0.2, z_index: 4, ..Default::default() },
        line_kind: "elbow".to_string(),
        color: "#00aaff".to_string(),
        stroke_width: 1.5,
        start_arrow: false,
        end_arrow: false,
    };
    let table = SlideElement::Table {
        id: "el_9".to_string(),
        transform: ElementTransform { x: 0.05, y: 0.6, w: 0.4, h: 0.3, z_index: 5, ..Default::default() },
        rows: 2,
        cols: 2,
        cells: vec![vec!["R0C0".to_string(), "R0C1".to_string()], vec!["R1C0".to_string(), "R1C1".to_string()]],
    };
    let grouped_a = SlideElement::Shape {
        id: "el_10a".to_string(),
        transform: ElementTransform { x: 0.6, y: 0.1, w: 0.1, h: 0.1, z_index: 0, ..Default::default() },
        shape_kind: "ellipse".to_string(),
        fill_color: "#ff00ff".to_string(),
        stroke_color: None,
        stroke_width: 0.0,
    };
    let grouped_b = SlideElement::Shape {
        id: "el_10b".to_string(),
        transform: ElementTransform { x: 0.75, y: 0.15, w: 0.1, h: 0.1, z_index: 0, ..Default::default() },
        shape_kind: "ellipse".to_string(),
        fill_color: "#ffff00".to_string(),
        stroke_color: None,
        stroke_width: 0.0,
    };
    let group = SlideElement::Group {
        id: "el_10".to_string(),
        transform: ElementTransform { x: 0.6, y: 0.1, w: 0.25, h: 0.15, z_index: 6, ..Default::default() },
        children: vec![grouped_a, grouped_b],
    };

    Presentation {
        id: "pres_export_test".to_string(),
        title: "Export <Test> & \"Quotes\"".to_string(),
        author: "Jason".to_string(),
        theme_name: None,
        slides: vec![
            Slide { label: Some("Slide 1".to_string()), background_v2: Some(SlideBackground::Solid("#123456".to_string())), elements: vec![text_block, image, missing_image], slide_document_version: 1, ..Default::default() },
            Slide {
                label: Some("Slide 2".to_string()),
                elements: vec![
                    SlideElement::TextBlock {
                        id: "el_4".to_string(),
                        transform: ElementTransform { z_index: 0, ..Default::default() },
                        block: TextBlock { runs: vec![TextRun { text: "Second slide".to_string(), ..Default::default() }], paragraph_style: TextParagraphStyle::default(), effects: ElementEffects::default(), autofit: false },
                    },
                    shape_rect,
                    shape_triangle,
                    line_arrow,
                    line_elbow,
                    table,
                    group,
                ],
                slide_document_version: 1,
                ..Default::default()
            },
        ],
    }
}

/// Builds a single-slide .pptx with no presentation.xml/rels (exercises the numeric
/// slideN.xml fallback ordering, already proven correct elsewhere) so callers can focus
/// each test on one slide's own XML content.
fn build_single_slide_pptx(slide_xml: &str) -> Vec<u8> {
    use std::io::{Cursor, Write};
    use zip::write::FileOptions;
    use zip::ZipWriter;

    let cursor = Cursor::new(Vec::new());
    let mut zip_writer = ZipWriter::new(cursor);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip_writer.start_file("ppt/slides/slide1.xml", options).unwrap();
    zip_writer.write_all(slide_xml.as_bytes()).unwrap();
    zip_writer.finish().unwrap().into_inner()
}

/// Builds a minimal in-memory `.pptx` (a zip archive with the handful of OOXML parts
/// PptxImporter actually reads) with slides deliberately out of filename order, to
/// prove presentation order is resolved via presentation.xml's sldIdLst + rels rather
/// than by sorting slideN.xml filenames.
fn build_test_pptx_bytes() -> Vec<u8> {
    use std::io::{Cursor, Write};
    use zip::write::FileOptions;
    use zip::ZipWriter;

    let cursor = Cursor::new(Vec::new());
    let mut zip_writer = ZipWriter::new(cursor);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let write_part = |zw: &mut ZipWriter<Cursor<Vec<u8>>>, name: &str, content: &str| {
        zw.start_file(name, options).unwrap();
        zw.write_all(content.as_bytes()).unwrap();
    };

    write_part(&mut zip_writer, "docProps/core.xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <dc:title>Test Sermon Deck</dc:title>
  <dc:creator>Test Author</dc:creator>
</cp:coreProperties>"#);

    // sldIdLst references rId2 then rId3, in that order — the visual/presentation order.
    write_part(&mut zip_writer, "ppt/presentation.xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst>
    <p:sldId id="256" r:id="rId2"/>
    <p:sldId id="257" r:id="rId3"/>
  </p:sldIdLst>
</p:presentation>"#);

    // rId2 resolves to slide2.xml and rId3 to slide1.xml — filename numbering is the
    // OPPOSITE of presentation order, so a naive numeric-filename sort would fail this test.
    write_part(&mut zip_writer, "ppt/_rels/presentation.xml.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#);

    write_part(&mut zip_writer, "ppt/slides/slide2.xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree><p:sp><p:txBody>
    <a:p><a:r><a:t>This is slide one content, with &amp; special &lt;chars&gt;</a:t></a:r></a:p>
    <a:p><a:r><a:t>Second line</a:t></a:r></a:p>
  </p:txBody></p:sp></p:spTree></p:cSld>
</p:sld>"#);

    write_part(&mut zip_writer, "ppt/slides/slide1.xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree><p:sp><p:txBody>
    <a:p><a:r><a:t>This is slide two content</a:t></a:r></a:p>
  </p:txBody></p:sp></p:spTree></p:cSld>
</p:sld>"#);

    zip_writer.finish().unwrap().into_inner()
}

#[test]
fn test_ewsx_schedule_roundtrip() {
    use std::io::{Read, Write};
    let tmp = tempfile::tempdir().unwrap();
    let archive_path = tmp.path().join("service.ewsx");

    let schedule = Schedule {
        id: "sched_01".to_string(),
        title: "Sunday Morning Service".to_string(),
        items: vec![
            ScheduleItem {
                item_type: "song".to_string(),
                author_or_ref: "Chris Tomlin".to_string(),
                slides: vec![
                    Slide { text: "The splendor of the King".to_string(), header: Some("Verse 1".to_string()), label: Some("V1".to_string()), background: None, notes: None, tag: Some("V1".to_string()), ..Default::default() },
                    Slide { text: "How great is our God".to_string(), header: Some("Chorus".to_string()), label: Some("C1".to_string()), background: None, notes: None, tag: Some("C1".to_string()), ..Default::default() },
                ],
                background: Some("clouds.jpg".to_string()),
                is_expanded: true,
                arrangement: vec![
                    ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
                    ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
                    ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
                ],
                ..item("item_01", "How Great Is Our God")
            }
        ],
        selected_item_index: Some(0),
        is_modified: false,
        schedule_version: 0,
    };

    // Save to EWSX
    EwsxManager::save_schedule_to_ewsx(&schedule, &archive_path).expect("EWSX save should succeed");
    assert!(archive_path.exists());

    // Load from EWSX
    let loaded = EwsxManager::load_schedule_from_ewsx(&archive_path).expect("EWSX load should succeed");
    assert_eq!(loaded.title, "Sunday Morning Service");
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].title, "How Great Is Our God");
    assert_eq!(loaded.items[0].slides.len(), 2);
    assert_eq!(loaded.items[0].arrangement.len(), 3);
    assert_eq!(loaded.items[0].arrangement[0].section_id, "V1");
    assert_eq!(loaded.items[0].arrangement[1].section_id, "C1");
    assert_eq!(loaded.items[0].arrangement[2].section_id, "V1");

    // Test PlayOrder and Verses table import from .ewpx
    let ewpx_path = tmp.path().join("song_with_playorder.ewpx");
    {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let conn = rusqlite::Connection::open(db_file.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE Verses (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                Type INTEGER,
                Number INTEGER,
                Words TEXT
            );
            CREATE TABLE PlayOrder (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                POrder INTEGER,
                Type INTEGER,
                Number INTEGER
            );
            INSERT INTO Verses (Type, Number, Words) VALUES (1, 1, 'Verse 1 lyrics');
            INSERT INTO Verses (Type, Number, Words) VALUES (2, 1, 'Chorus 1 lyrics');
            INSERT INTO Verses (Type, Number, Words) VALUES (3, 1, 'Bridge 1 lyrics');
            INSERT INTO Verses (Type, Number, Words) VALUES (4, 1, 'Ending 1 lyrics');
            INSERT INTO Verses (Type, Number, Words) VALUES (5, 1, 'Pre-Chorus 1 lyrics');
            -- Insert out of order to verify sorting by POrder
            INSERT INTO PlayOrder (POrder, Type, Number) VALUES (3, 1, 1); -- V1
            INSERT INTO PlayOrder (POrder, Type, Number) VALUES (1, 1, 1); -- V1
            INSERT INTO PlayOrder (POrder, Type, Number) VALUES (2, 5, 1); -- P1
            INSERT INTO PlayOrder (POrder, Type, Number) VALUES (4, 2, 1); -- C1
            INSERT INTO PlayOrder (POrder, Type, Number) VALUES (5, 3, 1); -- B1
            INSERT INTO PlayOrder (POrder, Type, Number) VALUES (6, 4, 1); -- E1
            "
        ).unwrap();
        drop(conn);

        let mut db_bytes = Vec::new();
        std::fs::File::open(db_file.path()).unwrap().read_to_end(&mut db_bytes).unwrap();

        let zip_f = std::fs::File::create(&ewpx_path).unwrap();
        let mut zip = zip::ZipWriter::new(zip_f);
        let opts = zip::write::FileOptions::default();
        zip.start_file("main.db", opts).unwrap();
        zip.write_all(&db_bytes).unwrap();
        zip.finish().unwrap();
    }

    let ewpx_loaded = EwsxManager::load_schedule_from_ewsx(&ewpx_path).expect("EWPX import should succeed");
    assert_eq!(ewpx_loaded.items.len(), 1);
    let item = &ewpx_loaded.items[0];
    assert_eq!(item.slides.len(), 5);
    assert_eq!(item.slides[0].tag.as_deref(), Some("V1"));
    assert_eq!(item.slides[1].tag.as_deref(), Some("C1"));
    assert_eq!(item.slides[2].tag.as_deref(), Some("B1"));
    assert_eq!(item.slides[3].tag.as_deref(), Some("E1"));
    assert_eq!(item.slides[4].tag.as_deref(), Some("P1"));

    // Verify PlayOrder sorted by POrder produces arrangement in play sequence
    assert_eq!(item.arrangement.len(), 6);
    assert_eq!(item.arrangement[0].section_id, "V1");
    assert_eq!(item.arrangement[0].source_slide_index, 0);
    assert_eq!(item.arrangement[1].section_id, "P1");
    assert_eq!(item.arrangement[1].source_slide_index, 4);
    assert_eq!(item.arrangement[2].section_id, "V1");
    assert_eq!(item.arrangement[2].source_slide_index, 0);
    assert_eq!(item.arrangement[3].section_id, "C1");
    assert_eq!(item.arrangement[3].source_slide_index, 1);
    assert_eq!(item.arrangement[4].section_id, "B1");
    assert_eq!(item.arrangement[4].source_slide_index, 2);
    assert_eq!(item.arrangement[5].section_id, "E1");
    assert_eq!(item.arrangement[5].source_slide_index, 3);

    // Verify roundtrip preservation: export imported .ewpx to .ewsx and reload
    let ewsx_roundtrip_path = tmp.path().join("roundtrip_export.ewsx");
    EwsxManager::save_schedule_to_ewsx(&ewpx_loaded, &ewsx_roundtrip_path).expect("Export to ewsx must succeed");
    let reloaded = EwsxManager::load_schedule_from_ewsx(&ewsx_roundtrip_path).expect("Reload from ewsx must succeed");
    assert_eq!(reloaded.items[0].slides.len(), 5);
    assert_eq!(reloaded.items[0].arrangement.len(), 6);
    for (orig, reload) in item.arrangement.iter().zip(reloaded.items[0].arrangement.iter()) {
        assert_eq!(orig.section_id, reload.section_id);
        assert_eq!(orig.source_slide_index, reload.source_slide_index);
    }

    if std::path::Path::new("LCspring.ewpx").exists() {
        let lc = EwsxManager::load_schedule_from_ewsx("LCspring.ewpx").expect("LCspring.ewpx must load");
        assert!(!lc.items.is_empty());
        for it in &lc.items {
            assert_eq!(it.arrangement.len(), it.slides.len());
        }
    }
}

#[test]
fn test_ewsx_roundtrip_preserves_rich_text_formatting_and_background() {
    use os_next::core::models::{
        ElementEffects, ElementTransform, SlideBackground, SlideElement, TextBlock,
        TextParagraphStyle, TextRun,
    };

    let tmp = tempfile::tempdir().unwrap();
    let archive_path = tmp.path().join("formatted.ewsx");

    let formatted_run = TextRun {
        text: "Bold red Georgia".to_string(),
        bold: true,
        color: "#ff0000".to_string(),
        font_family: "Georgia".to_string(),
        font_size_pt: 48.0,
        ..Default::default()
    };
    let plain_run = TextRun {
        text: " and plain".to_string(),
        ..Default::default()
    };

    let schedule = Schedule {
        id: "sched_fmt".to_string(),
        title: "Formatted Service".to_string(),
        items: vec![ScheduleItem {
            item_type: "song".to_string(),
            author_or_ref: "Test Author".to_string(),
            slides: vec![Slide {
                text: "Bold red Georgia and plain".to_string(),
                header: Some("Verse 1".to_string()),
                label: Some("V1".to_string()),
                background: None,
                notes: None,
                tag: Some("V1".to_string()),
                elements: vec![SlideElement::TextBlock {
                    id: "el_1".to_string(),
                    transform: ElementTransform::default(),
                    block: TextBlock {
                        runs: vec![formatted_run, plain_run],
                        paragraph_style: TextParagraphStyle { align: "left".to_string(), ..Default::default() },
                        effects: ElementEffects::default(),
                        autofit: true,
                    },
                }],
                background_v2: Some(SlideBackground::Image { file_path: "sunset.jpg".to_string(), opacity: 1.0 }),
                slide_document_version: 1,
                ..Default::default()
            }],
            arrangement: vec![ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None }],
            ..item("item_fmt", "Formatted Song")
        }],
        selected_item_index: Some(0),
        is_modified: false,
        schedule_version: 0,
    };

    EwsxManager::save_schedule_to_ewsx(&schedule, &archive_path).expect("EWSX save should succeed");

    // `load_schedule_from_ewsx` prefers the zip's manifest.json (a plain JSON dump of the
    // Schedule struct) when present, which would trivially round-trip these fields via
    // serde and never actually exercise the RTF/SQLite reconstruction this test is for.
    // Extract main.db's raw bytes and feed them to load_schedule_from_bytes directly —
    // its "starts with the SQLite magic header" branch routes straight into
    // parse_ewsx_sqlite, bypassing the zip/manifest path entirely.
    let zip_bytes = std::fs::read(&archive_path).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).unwrap();
    let mut db_bytes = Vec::new();
    std::io::Read::read_to_end(&mut archive.by_name("main.db").unwrap(), &mut db_bytes).unwrap();
    assert!(db_bytes.starts_with(b"SQLite format 3\0"), "main.db should be a real SQLite file");

    let loaded = EwsxManager::load_schedule_from_bytes(&db_bytes, "Formatted Service").expect("SQLite parse should succeed");

    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].slides.len(), 1);
    let slide = &loaded.items[0].slides[0];

    assert_eq!(slide.text, "Bold red Georgia and plain");

    // Background: written via resource_image + element.background_resource_id, resolved
    // back positionally through slide.order_index.
    match &slide.background_v2 {
        Some(SlideBackground::Image { file_path, .. }) => assert_eq!(file_path, "sunset.jpg"),
        other => panic!("expected an Image background, got {:?}", other),
    }
    assert_eq!(slide.background.as_deref(), Some("sunset.jpg"));

    // Rich formatting: written as real RTF control words, parsed back into TextRuns.
    let block = slide.elements.iter().find_map(|el| match el {
        SlideElement::TextBlock { block, .. } => Some(block),
        _ => None,
    }).expect("slide should have a TextBlock element");
    assert_eq!(block.paragraph_style.align, "left");
    assert_eq!(block.runs.len(), 2);
    assert_eq!(block.runs[0].text, "Bold red Georgia");
    assert!(block.runs[0].bold);
    assert_eq!(block.runs[0].color, "#ff0000");
    assert_eq!(block.runs[0].font_family, "Georgia");
    assert_eq!(block.runs[0].font_size_pt, 48.0);
    assert_eq!(block.runs[1].text, " and plain");
    assert!(!block.runs[1].bold);
}

#[test]
fn test_generate_and_load_pristine_sample_ewsx() {
    let schedule = Schedule {
        id: "sample_sunday_service".to_string(),
        title: "Sunday Morning Worship".to_string(),
        items: vec![
            ScheduleItem {
                item_type: "song".to_string(),
                author_or_ref: "John Newton".to_string(),
                slides: vec![
                    Slide { text: "Amazing grace, how sweet the sound\nThat saved a wretch like me".to_string(), header: Some("Verse 1".to_string()), label: Some("V1".to_string()), background: None, notes: None, tag: None, ..Default::default() },
                    Slide { text: "I once was lost, but now am found\nWas blind, but now I see".to_string(), header: Some("Verse 2".to_string()), label: Some("V2".to_string()), background: None, notes: None, tag: None, ..Default::default() },
                ],
                is_expanded: true,
                subtitle: Some("John Newton".to_string()),
                ..item("item_01", "Amazing Grace")
            },
            ScheduleItem {
                item_type: "scripture".to_string(),
                author_or_ref: "Psalm 103:1-5".to_string(),
                slides: vec![
                    Slide { text: "Bless the LORD, O my soul".to_string(), header: Some("Psalm 103:1".to_string()), label: Some("V1".to_string()), background: None, notes: None, tag: None, ..Default::default() },
                ],
                ..item("item_02", "Psalm 103:1-5")
            }
        ],
        selected_item_index: Some(0),
        is_modified: false,
        schedule_version: 0,
    };

    // Regression-tests the save/load round trip only -- this used to write
    // straight to "test1.ewsx" / "web/test1.ewsx", which are NOT test
    // fixtures: they're the real, shipped "Load EasyWorship sample
    // schedule" demo file (see web/src/app_ui.ts's fetch('/test1.ewsx')),
    // committed to the repo. Running `cargo test` was silently overwriting
    // that real content with this test's placeholder schedule on every run
    // (visible as a recurring, unexplained diff on both files). A tempdir
    // gives this test the same coverage without touching real project
    // files; regenerating the actual shipped sample (if that's ever
    // genuinely wanted) should be its own explicit, manually-run step, not
    // a side effect of the test suite.
    let test_dir = tempfile::tempdir().unwrap();
    let ewsx_path = test_dir.path().join("test1.ewsx");
    EwsxManager::save_schedule_to_ewsx(&schedule, &ewsx_path).expect("Failed to write test1.ewsx");

    // Verify loading back from bytes
    let bytes = std::fs::read(&ewsx_path).unwrap();
    let loaded = EwsxManager::load_schedule_from_bytes(&bytes, "test1").expect("Failed to load test1.ewsx from bytes");
    assert_eq!(loaded.title, "Sunday Morning Worship");
    assert_eq!(loaded.items.len(), 2);
    assert_eq!(loaded.items[0].title, "Amazing Grace");
}

#[test]
fn test_pptx_import_resolves_presentation_order_and_metadata() {
    let bytes = build_test_pptx_bytes();
    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "fallback_stem", media_dir.path())
        .expect("Should parse the constructed .pptx");

    assert_eq!(pres.title, "Test Sermon Deck");
    assert_eq!(pres.author, "Test Author");
    assert_eq!(pres.slides.len(), 2);

    // slide2.xml must come FIRST (per sldIdLst/rels order), proving the importer does
    // not fall back to sorting by slideN.xml filename number.
    assert_eq!(pres.slides[0].text, "This is slide one content, with & special <chars>\nSecond line");
    assert_eq!(pres.slides[1].text, "This is slide two content");
}

#[test]
fn test_pptx_import_falls_back_to_file_stem_when_title_missing() {
    use std::io::{Cursor, Write};
    use zip::write::FileOptions;
    use zip::ZipWriter;

    // No docProps/core.xml, and no presentation.xml/rels either — exercises both the
    // missing-title fallback and the numeric-filename-ordering fallback together.
    let cursor = Cursor::new(Vec::new());
    let mut zip_writer = ZipWriter::new(cursor);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip_writer.start_file("ppt/slides/slide1.xml", options).unwrap();
    zip_writer.write_all(br#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Only slide</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#).unwrap();
    let bytes = zip_writer.finish().unwrap().into_inner();

    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "My Sermon Notes", media_dir.path())
        .expect("Should parse a minimal .pptx with no core.xml");
    assert_eq!(pres.title, "My Sermon Notes");
    assert_eq!(pres.slides.len(), 1);
    assert_eq!(pres.slides[0].text, "Only slide");
}

#[test]
fn test_pptx_import_extracts_run_formatting_and_alignment() {
    use os_next::core::models::SlideElement;

    let slide_xml = r##"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree><p:sp><p:txBody>
    <a:p>
      <a:pPr algn="r"/>
      <a:r>
        <a:rPr b="1" i="1" u="sng" strike="sngStrike" sz="4800">
          <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
          <a:latin typeface="Georgia"/>
        </a:rPr>
        <a:t>Fancy</a:t>
      </a:r>
      <a:r>
        <a:t> plain</a:t>
      </a:r>
    </a:p>
  </p:txBody></p:sp></p:spTree></p:cSld>
</p:sld>"##;

    let bytes = build_single_slide_pptx(slide_xml);
    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "stem", media_dir.path()).expect("should parse");

    assert_eq!(pres.slides.len(), 1);
    let slide = &pres.slides[0];
    assert_eq!(slide.text, "Fancy plain");

    let block = slide.elements.iter().find_map(|el| match el {
        SlideElement::TextBlock { block, .. } => Some(block),
        _ => None,
    }).expect("slide should have a TextBlock element");
    assert_eq!(block.paragraph_style.align, "right");
    assert_eq!(block.runs.len(), 2);

    let fancy = &block.runs[0];
    assert_eq!(fancy.text, "Fancy");
    assert!(fancy.bold);
    assert!(fancy.italic);
    assert!(fancy.underline);
    assert!(fancy.strike);
    assert_eq!(fancy.color, "#ff0000");
    assert_eq!(fancy.font_family, "Georgia");
    assert_eq!(fancy.font_size_pt, 48.0);

    let plain = &block.runs[1];
    assert_eq!(plain.text, " plain");
    assert!(!plain.bold && !plain.italic && !plain.underline && !plain.strike);
}

#[test]
fn test_pptx_import_extracts_solid_background() {
    let slide_xml = r##"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:bg><p:bgPr><a:solidFill><a:srgbClr val="1F1C18"/></a:solidFill></p:bgPr></p:bg>
    <p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Hello</a:t></a:r></a:p></p:txBody></p:sp></p:spTree>
  </p:cSld>
</p:sld>"##;

    let bytes = build_single_slide_pptx(slide_xml);
    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "stem", media_dir.path()).expect("should parse");

    let slide = &pres.slides[0];
    assert_eq!(slide.background.as_deref(), Some("#1f1c18"));
    match &slide.background_v2 {
        Some(os_next::core::models::SlideBackground::Solid(c)) => assert_eq!(c, "#1f1c18"),
        other => panic!("expected a Solid background, got {:?}", other),
    }
}

#[test]
fn test_pptx_import_extracts_picture_background_and_saves_media_file() {
    use std::io::Write;
    use zip::write::FileOptions;

    let slide_xml = r##"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:cSld>
    <p:bg><p:bgPr><a:blipFill><a:blip r:embed="rId1"/><a:stretch><a:fillRect/></a:stretch></a:blipFill></p:bgPr></p:bg>
    <p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Hello</a:t></a:r></a:p></p:txBody></p:sp></p:spTree>
  </p:cSld>
</p:sld>"##;

    let fake_png_bytes: &[u8] = b"\x89PNG\r\n\x1a\nfake-image-bytes-for-testing";

    let cursor = std::io::Cursor::new(Vec::new());
    let mut zip_writer = zip::ZipWriter::new(cursor);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip_writer.start_file("ppt/slides/slide1.xml", options).unwrap();
    zip_writer.write_all(slide_xml.as_bytes()).unwrap();
    zip_writer.start_file("ppt/slides/_rels/slide1.xml.rels", options).unwrap();
    zip_writer.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"#).unwrap();
    zip_writer.start_file("ppt/media/image1.png", options).unwrap();
    zip_writer.write_all(fake_png_bytes).unwrap();
    let bytes = zip_writer.finish().unwrap().into_inner();

    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "stem", media_dir.path()).expect("should parse");

    let slide = &pres.slides[0];
    let file_path = match &slide.background_v2 {
        Some(os_next::core::models::SlideBackground::Image { file_path, .. }) => file_path.clone(),
        other => panic!("expected an Image background, got {:?}", other),
    };
    assert!(file_path.starts_with("/media/images/pptx_"), "unexpected path: {}", file_path);
    assert!(file_path.ends_with(".png"), "unexpected extension: {}", file_path);

    // The embedded image's actual bytes must have been extracted onto disk, not just
    // referenced by a path that doesn't exist anywhere.
    let saved_name = file_path.trim_start_matches("/media/images/");
    let saved_bytes = std::fs::read(media_dir.path().join(saved_name)).expect("extracted image file should exist");
    assert_eq!(saved_bytes, fake_png_bytes);
}

#[test]
fn test_pptx_import_emits_one_positioned_textblock_per_shape() {
    use os_next::core::models::SlideElement;

    // Two text shapes, each with its own explicit <a:xfrm> position -- the exact
    // "side-by-side columns" / "caption + body" case docs/IMPORT_EXPORT_NOTES.md
    // says used to get flattened into one full-bleed text block. No
    // presentation.xml here, so the importer's default 16:9 (12192000x6858000 EMU)
    // slide size applies -- these EMU values were chosen to divide out evenly
    // against it.
    let slide_xml = r##"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:sp>
      <p:spPr><a:xfrm><a:off x="1219200" y="685800"/><a:ext cx="3048000" cy="1371600"/></a:xfrm></p:spPr>
      <p:txBody><a:p><a:r><a:t>Left column</a:t></a:r></a:p></p:txBody>
    </p:sp>
    <p:sp>
      <p:spPr><a:xfrm><a:off x="6096000" y="3429000"/><a:ext cx="3048000" cy="1371600"/></a:xfrm></p:spPr>
      <p:txBody><a:p><a:r><a:t>Right column</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"##;

    let bytes = build_single_slide_pptx(slide_xml);
    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "stem", media_dir.path()).expect("should parse");

    let slide = &pres.slides[0];
    assert_eq!(slide.elements.len(), 2, "each shape must become its own element, not one flattened block");

    let text_blocks: Vec<_> = slide.elements.iter().filter_map(|el| match el {
        SlideElement::TextBlock { transform, block, .. } => Some((transform, block)),
        _ => None,
    }).collect();
    assert_eq!(text_blocks.len(), 2);

    let (t0, b0) = text_blocks[0];
    assert_eq!(b0.runs[0].text, "Left column");
    assert!((t0.x - 0.1).abs() < 1e-9, "x was {}", t0.x);
    assert!((t0.y - 0.1).abs() < 1e-9, "y was {}", t0.y);
    assert!((t0.w - 0.25).abs() < 1e-9, "w was {}", t0.w);
    assert!((t0.h - 0.2).abs() < 1e-9, "h was {}", t0.h);
    assert_eq!(t0.z_index, 0);

    let (t1, b1) = text_blocks[1];
    assert_eq!(b1.runs[0].text, "Right column");
    assert!((t1.x - 0.5).abs() < 1e-9, "x was {}", t1.x);
    assert!((t1.y - 0.5).abs() < 1e-9, "y was {}", t1.y);
    assert_eq!(t1.z_index, 1);

    // Distinct positions -- not both defaulted to the same full-bleed box.
    assert_ne!(t0.x, t1.x);
    assert_ne!(t0.y, t1.y);

    // Legacy flat `text` stays correct too, joining both shapes.
    assert_eq!(slide.text, "Left column\n\nRight column");
}

#[test]
fn test_pptx_import_emits_positioned_image_element_from_inline_picture() {
    use os_next::core::models::SlideElement;
    use std::io::Write;
    use zip::write::FileOptions;

    // A text shape plus an inline (body-level) picture -- the "positioned logo" /
    // "caption over a photo" case. Inline pictures (<p:pic>, as opposed to a
    // slide-level <p:bg> fill) were never imported at all before this rewrite.
    let slide_xml = r##"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:cSld><p:spTree>
    <p:sp>
      <p:txBody><a:p><a:r><a:t>Caption text</a:t></a:r></a:p></p:txBody>
    </p:sp>
    <p:pic>
      <p:nvPicPr><p:cNvPr id="3" name="Logo"/></p:nvPicPr>
      <p:blipFill><a:blip r:embed="rId1"/></p:blipFill>
      <p:spPr><a:xfrm><a:off x="9144000" y="457200"/><a:ext cx="1524000" cy="1524000"/></a:xfrm></p:spPr>
    </p:pic>
  </p:spTree></p:cSld>
</p:sld>"##;

    let fake_png_bytes: &[u8] = b"\x89PNG\r\n\x1a\nfake-inline-logo-bytes";

    let cursor = std::io::Cursor::new(Vec::new());
    let mut zip_writer = zip::ZipWriter::new(cursor);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip_writer.start_file("ppt/slides/slide1.xml", options).unwrap();
    zip_writer.write_all(slide_xml.as_bytes()).unwrap();
    zip_writer.start_file("ppt/slides/_rels/slide1.xml.rels", options).unwrap();
    zip_writer.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/logo.png"/>
</Relationships>"#).unwrap();
    zip_writer.start_file("ppt/media/logo.png", options).unwrap();
    zip_writer.write_all(fake_png_bytes).unwrap();
    let bytes = zip_writer.finish().unwrap().into_inner();

    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "stem", media_dir.path()).expect("should parse");

    let slide = &pres.slides[0];
    assert_eq!(slide.elements.len(), 2, "both the text shape and the inline picture must become elements");

    let image = slide.elements.iter().find_map(|el| match el {
        SlideElement::Image { transform, file_path, .. } => Some((transform, file_path.clone())),
        _ => None,
    }).expect("an Image element must be present");
    let (transform, file_path) = image;

    assert!(file_path.starts_with("/media/images/pptx_"), "unexpected path: {}", file_path);
    assert!(file_path.ends_with(".png"), "unexpected extension: {}", file_path);
    let saved_name = file_path.trim_start_matches("/media/images/");
    let saved_bytes = std::fs::read(media_dir.path().join(saved_name)).expect("extracted logo file should exist");
    assert_eq!(saved_bytes, fake_png_bytes);

    // Positioned where the <a:xfrm> actually said (top-right corner), not the
    // default full-bleed box.
    assert!((transform.x - 0.75).abs() < 1e-9, "x was {}", transform.x);
    assert!(transform.w < 0.2, "a small logo must not default to a full-bleed box; w was {}", transform.w);
    assert_eq!(transform.z_index, 1, "the picture is the second element to successfully resolve");

    // The text shape's own element is unaffected by the picture's presence.
    assert_eq!(slide.text, "Caption text");
}

#[test]
fn test_openlp_import_songs_sqlite() {
    let fixture_path = std::path::Path::new("tests/fixtures/openlp/songs-2.4.6.sqlite");
    assert!(fixture_path.exists(), "songs fixture must exist");

    let songs = OpenLPImporter::import_songs_db(fixture_path).expect("Should parse songs.sqlite");
    assert!(!songs.is_empty(), "Songs list should not be empty");
    println!("Imported {} songs from songs-2.4.6.sqlite", songs.len());

    let db = Database::in_memory().expect("In-memory SQLite database should create");
    for song in &songs {
        db.insert_song(song).expect("Should insert song into database");
    }

    let loaded = db.get_songs().expect("Should get songs from db");
    assert_eq!(loaded.len(), songs.len());

    // Check first song has slides and valid id
    let first = &songs[0];
    assert!(!first.title.is_empty());
    assert!(!first.slides.is_empty());
    assert!(!first.id.is_empty());
    assert_eq!(first.theme_name.as_deref(), Some("Moss on tree"));
    assert_eq!(loaded[0].theme_name.as_deref(), Some("Moss on tree"));
}

#[test]
fn test_openlp_import_bible_sqlite() {
    let fixture_path = std::path::Path::new("tests/fixtures/openlp/bible-tests.sqlite");
    assert!(fixture_path.exists(), "bible fixture must exist");

    let scriptures = OpenLPImporter::import_bible_db(fixture_path).expect("Should parse bible sqlite");
    assert!(!scriptures.is_empty(), "Scriptures should not be empty");
    println!("Imported {} scripture chapters from bible-tests.sqlite", scriptures.len());

    let db = Database::in_memory().expect("In-memory SQLite database should create");
    for scrip in &scriptures {
        db.insert_scripture(scrip).expect("Should insert scripture into db");
    }

    // Verify verse tags were cleaned
    for item in &scriptures {
        for v in &item.verses {
            assert!(!v.text.contains("{su}"), "Verse text should not contain OpenLP {{su}} tag");
            assert!(!v.text.contains("{/su}"), "Verse text should not contain OpenLP {{/su}} tag");
        }
    }
}

#[test]
fn test_openlp_import_service_item_json() {
    let fixture_path = std::path::Path::new("tests/fixtures/openlp/serviceitem-song.osj");
    assert!(fixture_path.exists(), "serviceitem json fixture must exist");

    let json_str = std::fs::read_to_string(fixture_path).expect("Should read serviceitem json");
    let schedule = OpenLPImporter::import_openlp_service_json(&json_str).expect("Should parse serviceitem json");

    assert!(!schedule.items.is_empty(), "Schedule should contain items");
    let item = &schedule.items[0];
    assert_eq!(item.title, "Amazing Grace");
    assert!(!item.slides.is_empty(), "Item should have slides");
    println!("Imported service item '{}' with {} slides", item.title, item.slides.len());
}

#[test]
fn test_openlp_service_zip_osz_loading() {
    let fixture_path = std::path::Path::new("tests/fixtures/openlp/test.osz");
    assert!(fixture_path.exists(), "test.osz fixture must exist");

    let bytes = std::fs::read(fixture_path).expect("Should read test.osz bytes");
    let schedule = EwsxManager::load_schedule_from_bytes(&bytes, "test.osz").expect("Should load schedule from .osz zip archive");

    assert!(!schedule.items.is_empty(), "Schedule should have items");
    println!("Loaded schedule '{}' from test.osz with {} items", schedule.title, schedule.items.len());
    for item in &schedule.items {
        println!("  Item: {} ({} slides)", item.title, item.slides.len());
    }
}

#[test]
fn test_openlp_detect_and_import_local() {
    let db = Database::in_memory().expect("In-memory SQLite database should create");
    let result = OpenLPImporter::detect_and_import_local(&db, None).expect("Should detect and import local");
    println!("Detect and import local result: {:?}", result);
    if !OpenLPImporter::find_local_openlp_dirs().is_empty() {
        assert!(result.success);
        assert!(result.songs_count > 0 || result.bibles_count > 0);
    }
}

#[test]
fn test_openlp_theme_xml_resolution_and_backgrounds() {
    use std::io::Cursor;
    use std::io::Write;
    use zip::write::FileOptions;
    use os_next::storage::openlp_import::extract_xml_tag_content;

    // 1. Verify extract_xml_tag_content helper
    let sample_xml = "<theme><name>TestTheme</name><color>#ff1122</color></theme>";
    assert_eq!(extract_xml_tag_content(sample_xml, "name").as_deref(), Some("TestTheme"));
    assert_eq!(extract_xml_tag_content(sample_xml, "color").as_deref(), Some("#ff1122"));
    assert_eq!(extract_xml_tag_content(sample_xml, "nonexistent"), None);

    // 2. Solid color theme XML
    let solid_xml = r#"<theme version="2.0">
  <name>SolidBlue</name>
  <background type="solid">
    <color>#003366</color>
  </background>
</theme>"#;
    let solid_bg = OpenLPImporter::parse_openlp_theme_xml(solid_xml, None, None);
    match solid_bg {
        Some(SlideBackground::Solid(c)) => assert_eq!(c, "#003366"),
        other => panic!("Expected Solid background, got {:?}", other),
    }

    // 3. Gradient theme XML
    let gradient_xml = r#"<theme version="2.0">
  <name>GradSun</name>
  <background type="gradient">
    <startColor>#ff0000</startColor>
    <endColor>#0000ff</endColor>
    <direction>horizontal</direction>
  </background>
</theme>"#;
    let grad_bg = OpenLPImporter::parse_openlp_theme_xml(gradient_xml, None, None);
    match grad_bg {
        Some(SlideBackground::Gradient { kind, stops, angle_deg }) => {
            assert_eq!(kind, "linear");
            assert_eq!(stops.len(), 2);
            assert_eq!(stops[0].color, "#ff0000");
            assert_eq!(stops[1].color, "#0000ff");
            assert_eq!(angle_deg, Some(90.0));
        }
        other => panic!("Expected Gradient background, got {:?}", other),
    }

    // 4. Image theme XML in a directory
    let temp_theme_dir = tempfile::tempdir().unwrap();
    let img_path = temp_theme_dir.path().join("forest.jpg");
    // Real magic bytes: importers now sniff content type instead of trusting the extension.
    std::fs::write(&img_path, b"\xFF\xD8\xFF\xE0fake-jpeg-bytes").unwrap();

    let img_xml = r#"<theme version="2.0">
  <name>ForestTheme</name>
  <background type="image">
    <filename>forest.jpg</filename>
  </background>
</theme>"#;
    let media_dir = tempfile::tempdir().unwrap();
    let img_bg = OpenLPImporter::parse_openlp_theme_xml(img_xml, Some(temp_theme_dir.path()), Some(media_dir.path()));
    match img_bg {
        Some(SlideBackground::Image { file_path, opacity }) => {
            assert_eq!(opacity, 1.0);
            assert!(file_path.starts_with("/media/images/openlp_"), "unexpected file path: {}", file_path);
            let saved_name = file_path.trim_start_matches("/media/images/");
            assert!(media_dir.path().join(saved_name).exists(), "Extracted media file should exist");
        }
        other => panic!("Expected Image background, got {:?}", other),
    }

    // 5. OpenLP .otz zip package
    let otz_cursor = Cursor::new(Vec::new());
    let mut zip_writer = zip::ZipWriter::new(otz_cursor);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let otz_xml = r#"<theme version="2.0">
  <name>Moss on tree</name>
  <background type="image">
    <filename>climbing-moss.jpeg</filename>
  </background>
</theme>"#;
    zip_writer.start_file("Moss on tree/Moss on tree.xml", options).unwrap();
    zip_writer.write_all(otz_xml.as_bytes()).unwrap();
    zip_writer.start_file("Moss on tree/climbing-moss.jpeg", options).unwrap();
    zip_writer.write_all(b"\xFF\xD8\xFF\xE0fake-climbing-moss-bytes").unwrap();
    let otz_bytes = zip_writer.finish().unwrap().into_inner();

    let otz_bg = OpenLPImporter::parse_openlp_otz(Cursor::new(otz_bytes), Some(media_dir.path()));
    match otz_bg {
        Some(SlideBackground::Image { file_path, opacity }) => {
            assert_eq!(opacity, 1.0);
            assert!(file_path.starts_with("/media/images/openlp_"));
            assert!(file_path.ends_with(".jpg"), "extension comes from the sniffed content type");
            let saved_name = file_path.trim_start_matches("/media/images/");
            let content = std::fs::read(media_dir.path().join(saved_name)).unwrap();
            assert_eq!(content, b"\xFF\xD8\xFF\xE0fake-climbing-moss-bytes");
        }
        other => panic!("Expected Image background from .otz, got {:?}", other),
    }

    // 6. import_songs_db_with_media with theme resolution on disk
    let songs_dir = tempfile::tempdir().unwrap();
    let orig_db = std::path::Path::new("tests/fixtures/openlp/songs-2.4.6.sqlite");
    let copied_db = songs_dir.path().join("songs.sqlite");
    std::fs::copy(orig_db, &copied_db).unwrap();

    let themes_dir = songs_dir.path().join("themes").join("Moss on tree");
    std::fs::create_dir_all(&themes_dir).unwrap();
    std::fs::write(
        themes_dir.join("Moss on tree.xml"),
        r#"<theme version="2.0">
  <name>Moss on tree</name>
  <background type="solid">
    <color>#2e5c1e</color>
  </background>
</theme>"#,
    ).unwrap();

    let songs = OpenLPImporter::import_songs_db_with_media(&copied_db, Some(media_dir.path()))
        .expect("Should import songs and resolve theme");
    assert!(!songs.is_empty());
    let song = &songs[0];
    assert_eq!(song.theme_name.as_deref(), Some("Moss on tree"));
    for slide in &song.slides {
        assert_eq!(slide.background.as_deref(), Some("#2e5c1e"));
        match &slide.background_v2 {
            Some(SlideBackground::Solid(c)) => assert_eq!(c, "#2e5c1e"),
            other => panic!("Expected Solid slide background, got {:?}", other),
        }
    }
}

#[test]
fn test_openlp_service_theme_wiring() {
    let media_dir = tempfile::tempdir().unwrap();
    let themes_dir = media_dir.path().join("themes");
    std::fs::create_dir_all(&themes_dir).unwrap();
    std::fs::write(
        themes_dir.join("Sunset.xml"),
        r#"<theme version="2.0">
  <name>Sunset</name>
  <background type="gradient">
    <startColor>#ff7e5f</startColor>
    <endColor>#feb47b</endColor>
    <direction>vertical</direction>
  </background>
</theme>"#,
    ).unwrap();

    let service_json = r#"[
  {
    "serviceitem": {
      "header": {
        "title": "Holy Holy Holy",
        "theme": "Sunset",
        "plugin": "songs"
      },
      "data": [
        {
          "raw_slide": "Holy, Holy, Holy! Lord God Almighty!",
          "verseTag": "V1"
        },
        {
          "raw_slide": "Early in the morning our song shall rise to Thee;",
          "verseTag": "V2"
        }
      ]
    }
  }
]"#;

    let schedule = OpenLPImporter::import_openlp_service_json_with_media(service_json, Some(media_dir.path()))
        .expect("Should import service JSON with theme");
    assert_eq!(schedule.items.len(), 1);
    let item = &schedule.items[0];
    assert_eq!(item.title, "Holy Holy Holy");
    assert!(item.background.is_some(), "ScheduleItem background must be set from theme");
    assert!(item.background.as_ref().unwrap().contains("linear-gradient"));
    assert_eq!(item.slides.len(), 2);

    for slide in &item.slides {
        assert_eq!(slide.background, item.background);
        match &slide.background_v2 {
            Some(SlideBackground::Gradient { stops, angle_deg, .. }) => {
                assert_eq!(stops.len(), 2);
                assert_eq!(stops[0].color, "#ff7e5f");
                assert_eq!(stops[1].color, "#feb47b");
                assert_eq!(*angle_deg, Some(180.0));
            }
            other => panic!("Expected Gradient background on slide, got {:?}", other),
        }
    }
}

#[test]
fn test_freeshow_show_import_minimal_and_runs() {
    let media_dir = tempfile::tempdir().unwrap();

    // FreeShow show JSON in array wrapper format: [ id, show_object ]
    let json_str = r#"[
  "show_test_001",
  {
    "name": "Great Is Thy Faithfulness",
    "meta": {
      "artist": "Thomas Chisholm",
      "CCLI": "18944"
    },
    "settings": {
      "activeLayout": "layout_main"
    },
    "layouts": {
      "layout_main": {
        "slides": [
          { "id": "slide_v1" },
          { "id": "slide_c1" }
        ]
      }
    },
    "slides": {
      "slide_v1": {
        "group": "Verse 1",
        "items": [
          {
            "type": "text",
            "lines": [
              {
                "align": "center",
                "text": [
                  { "value": "Great is Thy " },
                  {
                    "value": "faithfulness",
                    "style": "font-weight: bold; font-style: italic; color: #ff9900; text-decoration: underline line-through; font-family: Inter; letter-spacing: 2px;"
                  },
                  { "value": ", O God my Father," }
                ]
              },
              {
                "align": "center",
                "text": [
                  { "value": "There is no shadow of turning with Thee." }
                ]
              }
            ]
          }
        ]
      },
      "slide_c1": {
        "group": "Chorus",
        "items": [
          {
            "type": "text",
            "lines": [
              {
                "align": "right",
                "text": [
                  { "value": "Great is Thy faithfulness! Morning by morning new mercies I see;" }
                ]
              }
            ]
          }
        ]
      }
    }
  }
]"#;

    let schedule = FreeShowShowImporter::import_show_bytes(json_str.as_bytes(), media_dir.path())
        .expect("FreeShow import should parse show successfully");

    assert_eq!(schedule.items.len(), 1);
    let item = &schedule.items[0];
    assert_eq!(item.title, "Great Is Thy Faithfulness");
    assert_eq!(item.subtitle.as_deref(), Some("Thomas Chisholm"));
    assert_eq!(item.item_type, "song");
    assert_eq!(item.slides.len(), 2);

    // Slide 1 (Verse 1)
    let slide_1 = &item.slides[0];
    assert_eq!(slide_1.label.as_deref(), Some("Verse 1"));
    assert_eq!(slide_1.header.as_deref(), Some("V1"));
    assert_eq!(slide_1.elements.len(), 1);

    if let SlideElement::TextBlock { block, .. } = &slide_1.elements[0] {
        assert_eq!(block.paragraph_style.align, "center");
        assert_eq!(block.runs.len(), 5);

        // Run 0: "Great is Thy "
        assert_eq!(block.runs[0].text, "Great is Thy ");
        assert!(!block.runs[0].bold);
        assert!(!block.runs[0].italic);

        // Run 1: "faithfulness" formatted
        assert_eq!(block.runs[1].text, "faithfulness");
        assert!(block.runs[1].bold);
        assert!(block.runs[1].italic);
        assert!(block.runs[1].underline);
        assert!(block.runs[1].strike);
        assert_eq!(block.runs[1].color, "#ff9900");
        assert_eq!(block.runs[1].font_family, "Inter");
        assert_eq!(block.runs[1].letter_spacing_px, 2.0);

        // Run 2: ", O God my Father,"
        assert_eq!(block.runs[2].text, ", O God my Father,");

        // Run 3: "\n" neutral paragraph break
        assert_eq!(block.runs[3].text, "\n");

        // Run 4: "There is no shadow of turning with Thee."
        assert_eq!(block.runs[4].text, "There is no shadow of turning with Thee.");
    } else {
        panic!("Expected TextBlock element");
    }

    // Slide 2 (Chorus)
    let slide_2 = &item.slides[1];
    assert_eq!(slide_2.label.as_deref(), Some("Chorus"));
    assert_eq!(slide_2.header.as_deref(), Some("C1"));
    if let SlideElement::TextBlock { block, .. } = &slide_2.elements[0] {
        assert_eq!(block.paragraph_style.align, "right");
        assert_eq!(block.runs[0].text, "Great is Thy faithfulness! Morning by morning new mercies I see;");
    } else {
        panic!("Expected TextBlock element");
    }

    // Arrangement verification
    assert_eq!(item.arrangement.len(), 2);
    assert_eq!(item.arrangement[0].section_id, "V1");
    assert_eq!(item.arrangement[0].source_slide_index, 0);
    assert_eq!(item.arrangement[1].section_id, "C1");
    assert_eq!(item.arrangement[1].source_slide_index, 1);
}

#[test]
fn test_freeshow_show_import_backgrounds_and_media() {
    let media_dir = tempfile::tempdir().unwrap();

    let json_str = r##"{
  "name": "Background Showcase",
  "settings": {
    "activeLayout": "l1"
  },
  "layouts": {
    "l1": {
      "slides": [
        { "id": "s_solid" },
        { "id": "s_media" }
      ]
    }
  },
  "slides": {
    "s_solid": {
      "color": "#1a2b3c",
      "items": [
        {
          "type": "text",
          "lines": [{ "text": [{ "value": "Solid Slide" }] }]
        }
      ]
    },
    "s_media": {
      "settings": {
        "backgroundImage": "media_bg_1"
      },
      "items": [
        {
          "type": "text",
          "lines": [{ "text": [{ "value": "Media Slide" }] }]
        }
      ]
    }
  },
  "media": {
    "media_bg_1": {
      "name": "bg.png",
      "base64": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
    }
  }
}"##;

    let schedule = FreeShowShowImporter::import_show_bytes(json_str.as_bytes(), media_dir.path())
        .expect("FreeShow import should succeed");

    assert_eq!(schedule.items.len(), 1);
    let item = &schedule.items[0];
    assert_eq!(item.slides.len(), 2);

    // Slide 0: Solid background
    let s0 = &item.slides[0];
    assert_eq!(s0.background.as_deref(), Some("#1a2b3c"));
    match &s0.background_v2 {
        Some(SlideBackground::Solid(c)) => assert_eq!(c, "#1a2b3c"),
        other => panic!("Expected Solid background, got {:?}", other),
    }

    // Slide 1: Media background extracted to disk
    let s1 = &item.slides[1];
    assert!(s1.background.is_some());
    match &s1.background_v2 {
        Some(SlideBackground::Image { file_path, opacity }) => {
            assert_eq!(*opacity, 1.0);
            assert!(file_path.starts_with("/media/images/freeshow_"), "unexpected path: {}", file_path);
            assert!(file_path.ends_with(".png"), "unexpected extension: {}", file_path);
            let filename = file_path.trim_start_matches("/media/images/");
            let extracted_path = media_dir.path().join(filename);
            assert!(extracted_path.exists(), "Extracted media file must exist on disk");
            let bytes = std::fs::read(&extracted_path).unwrap();
            assert!(!bytes.is_empty(), "Extracted media file must not be empty");
        }
        other => panic!("Expected Image background from media, got {:?}", other),
    }
}

#[test]
fn test_freeshow_schedule_dispatcher() {
    use std::io::Write;
    use zip::write::FileOptions;

    let show_json = r#"{
  "name": "Dispatcher Test",
  "settings": { "activeLayout": "l1" },
  "layouts": { "l1": { "slides": [{ "id": "s1" }] } },
  "slides": {
    "s1": {
      "items": [{ "type": "text", "lines": [{ "text": [{ "value": "Dispatcher slide" }] }] }]
    }
  }
}"#;

    // 1. Direct .show bytes
    let sched1 = EwsxManager::load_schedule_from_bytes(show_json.as_bytes(), "test.show")
        .expect("EwsxManager should dispatch .show files");
    assert_eq!(sched1.items.len(), 1);
    assert_eq!(sched1.items[0].title, "Dispatcher Test");

    // 2. Direct .project bytes
    let sched2 = EwsxManager::load_schedule_from_bytes(show_json.as_bytes(), "test.project")
        .expect("EwsxManager should dispatch .project files");
    assert_eq!(sched2.items.len(), 1);
    assert_eq!(sched2.items[0].title, "Dispatcher Test");

    // 3. Zip archive containing a .show file
    let cursor = std::io::Cursor::new(Vec::new());
    let mut zip_writer = zip::ZipWriter::new(cursor);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip_writer.start_file("deck.show", options).unwrap();
    zip_writer.write_all(show_json.as_bytes()).unwrap();
    let zip_bytes = zip_writer.finish().unwrap().into_inner();

    let sched3 = EwsxManager::load_schedule_from_bytes(&zip_bytes, "packaged.zip")
        .expect("EwsxManager should dispatch zip archives containing .show files");
    assert_eq!(sched3.items.len(), 1);
    assert_eq!(sched3.items[0].title, "Dispatcher Test");
}

#[tokio::test]
async fn test_freeshow_import_http_route() {
    use base64::Engine;

    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_http.db");
    let db = Database::new(&db_path).expect("Database::new");
    let web_dir = test_dir.path().join("web");
    std::fs::create_dir_all(web_dir.join("media").join("images")).unwrap();

    let event_log = Arc::new(EventLog::with_initial_sequence(2000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let asset_graph = Arc::new(AssetGraph::new(&web_dir));
    let (tx, _rx) = tokio::sync::broadcast::channel(512);

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        asset_graph,
        web_dir: web_dir.clone(),
        media_dir: web_dir.clone(),
        ..base_app_state(db.clone())
    };

    let router = os_next::api::create_router(app_state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let raw_show = r#"[
  "http_show_1",
  {
    "name": "Live HTTP Test Song",
    "meta": { "author": "John Wesley" },
    "settings": { "activeLayout": "l1" },
    "layouts": { "l1": { "slides": [{ "id": "s1" }] } },
    "slides": {
      "s1": {
        "group": "Verse 1",
        "items": [{ "type": "text", "lines": [{ "text": [{ "value": "O for a thousand tongues to sing" }] }] }]
      }
    }
  }
]"#;

    let b64 = base64::engine::general_purpose::STANDARD.encode(raw_show.as_bytes());
    let payload = serde_json::json!({
        "file_data_base64": b64
    });

    let client = reqwest::Client::new();
    let resp = client.post(format!("http://{}/api/import/freeshow-show", addr))
        .header("x-host-token", "test_host_session_token")
        .json(&payload)
        .send()
        .await
        .expect("HTTP request should succeed");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let body_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body_json["success"], true);
    assert_eq!(body_json["title"], "Live HTTP Test Song");
    assert_eq!(body_json["type"], "song");
    assert_eq!(body_json["slide_count"], 1);

    // Verify it actually saved into the SQLite database
    let songs = db.get_songs().expect("Songs should be retrieved from db");
    assert!(songs.iter().any(|s| s.title == "Live HTTP Test Song"));
}

#[tokio::test]
async fn test_lcspring_ewpx_presentation_bullets_and_transforms() {
    use std::path::PathBuf;
    use os_next::core::models::SlideElement;

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ewpx_path = manifest_dir.join("LCspring.ewpx");
    if !ewpx_path.exists() {
        return;
    }

    let schedule = EwsxManager::load_schedule_from_ewsx(&ewpx_path).expect("Should load LCspring.ewpx");
    assert!(!schedule.items.is_empty(), "Schedule should contain items");

    // Verify presentation items have autofit: false and custom transforms
    let presentation_items: Vec<_> = schedule.items.iter().filter(|it| it.item_type == "presentation").collect();
    assert!(!presentation_items.is_empty(), "Should contain presentation items");

    let mut found_bullet = false;
    let mut found_custom_bullet = false;
    let mut found_custom_transform = false;

    for item in &presentation_items {
        for slide in &item.slides {
            for el in &slide.elements {
                if let SlideElement::TextBlock { transform, block, .. } = el {
                    // Rule: Autosizing should only apply to songs and scripture
                    assert!(!block.autofit, "Presentation text blocks must have autofit = false");

                    // Bounding boxes must not be stretched to default when real coordinates exist
                    if (transform.w - 0.2775).abs() < 0.01 && (transform.y - 0.7002).abs() < 0.01 {
                        found_custom_transform = true;
                    }

                    // Font sizes should not be at 73pt template ceiling
                    for run in &block.runs {
                        assert!(run.font_size_pt <= 68.0, "Presentation fonts should be scaled below 70pt ceiling, got {}", run.font_size_pt);
                    }
                }
            }

            if slide.text.contains("• TODAY - Sacrificial Giving") {
                found_bullet = true;
            }
            if slide.text.contains("✱ Please see the Pastor") {
                found_custom_bullet = true;
            }
        }
    }

    assert!(found_bullet, "Should have parsed standard bullet point (•)");
    assert!(found_custom_bullet, "Should have parsed custom glyph bullet (✱)");
    assert!(found_custom_transform, "Should have preserved custom non-default element transform coordinates");
}

// Inputs found by `fuzz/` (see docs/FUZZING.md) that used to panic; each must
// now return normally. Fixtures are the raw libFuzzer crash artifacts.
#[test]
fn test_fuzz_regressions_do_not_panic() {
    let ewsx: &[u8] = include_bytes!("../fixtures/fuzz_regressions/ewsx_import.bin");
    let _ = EwsxManager::load_schedule_from_bytes(ewsx, "fuzz");

    let theme = String::from_utf8_lossy(include_bytes!("../fixtures/fuzz_regressions/openlp_theme.bin")).into_owned();
    let _ = OpenLPImporter::parse_openlp_theme_xml(&theme, None, None);

    let rtf = String::from_utf8_lossy(include_bytes!("../fixtures/fuzz_regressions/rtf_parse.bin")).into_owned();
    let _ = os_next::storage::rtf::parse_rtf(&rtf);

    let scripture = String::from_utf8_lossy(include_bytes!("../fixtures/fuzz_regressions/scripture_ref.bin")).into_owned();
    let _ = parse_scripture_reference(&scripture);
    let _ = parse_scripture_reference("John 3 16–18");
    let _ = parse_scripture_reference("John 3:16–18");
}

// Importers copy media referenced by untrusted files into the web-served
// media dir. Active content (HTML/SVG) and non-media files (a local path such
// as /etc/passwd) must be refused rather than copied under a servable name.
#[test]
fn test_importers_refuse_non_media_files_for_served_media_dir() {
    let theme_dir = tempfile::tempdir().unwrap();
    std::fs::write(theme_dir.path().join("evil.html"), b"<script>alert(1)</script>").unwrap();
    std::fs::write(theme_dir.path().join("evil.jpg"), b"<svg onload=alert(1)/>").unwrap();
    let media_dir = tempfile::tempdir().unwrap();

    for name in ["evil.html", "evil.jpg"] {
        let xml = format!(
            r#"<theme version="2.0"><name>T</name><background type="image"><filename>{}</filename></background></theme>"#,
            name
        );
        let bg = OpenLPImporter::parse_openlp_theme_xml(&xml, Some(theme_dir.path()), Some(media_dir.path()));
        assert!(bg.is_none(), "{} must not be imported as a background", name);
    }
    assert_eq!(std::fs::read_dir(media_dir.path()).unwrap().count(), 0, "nothing should be copied");
}

#[test]
fn test_sniff_audio_ext_recognizes_real_audio_and_rejects_impostors() {
    use os_next::storage::media_sniff::sniff_audio_ext;
    assert_eq!(sniff_audio_ext(b"ID3\x03\x00\x00\x00\x00\x00\x00"), Some("mp3"));
    // A bare mp3 stream (no ID3 tag) is only recognized by a real, frame-chaining MPEG bitstream
    // (src/storage/media_sniff.rs's own tests cover this in depth); 4 bytes of sync pattern alone is not enough.
    assert_eq!(sniff_audio_ext(&[0xFF, 0xFB, 0x90, 0x00]), None, "a lone header with nothing real following it must not pass");
    assert_eq!(sniff_audio_ext(b"RIFF\0\0\0\0WAVEfmt "), Some("wav"));
    assert_eq!(sniff_audio_ext(b"fLaC\0\0\0\"xxxx"), Some("flac"));
    assert_eq!(sniff_audio_ext(b"OggS\0\x02\0\0\0\0\0\0\0\0OpusHead\x01\x02"), Some("opus"));
    assert_eq!(sniff_audio_ext(b"OggS\0\x02\0\0\0\0\0\0\0\0\x01vorbis"), Some("ogg"));
    assert_eq!(sniff_audio_ext(b"\0\0\0\x18ftypM4A \0\0\0\0"), Some("m4a"));
    assert_eq!(sniff_audio_ext(b"<html><script>alert(1)</script></html>"), None);
    assert_eq!(sniff_audio_ext(b"GIF89a"), None, "an image is not audio, even if someone names it x.mp3");
    assert_eq!(sniff_audio_ext(b""), None);
}

// ---------------------------------------------------------------------------------------------
// ODP import (testdata/odp/fixture.odp: a real presentation built and saved by actual
// LibreOffice via pyuno, not hand-authored XML -- see testdata/odp/README.md)
// ---------------------------------------------------------------------------------------------

#[test]
fn test_odp_import_against_a_real_libreoffice_fixture() {
    let tmp = tempfile::tempdir().unwrap();
    let media_dir = tmp.path().join("media/images");
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/odp/fixture.odp")).unwrap();
    let pres = os_next::storage::OdpImporter::import_odp_bytes(&bytes, "My Deck", &media_dir).expect("real LibreOffice output must import");

    assert_eq!(pres.title, "My Deck", "no dc:title in this fixture's meta.xml, so the file stem is used");
    assert_eq!(pres.slides.len(), 2);

    // Slide 1: one text box, two paragraphs. First paragraph: a plain run then a styled run
    // (LibreOffice's own editor merged "World" and " plain" into one run sharing the same
    // direct formatting once both were typed under the same character attributes -- a real
    // editing-model behavior, not an importer bug; see testdata/odp/README.md).
    let s1 = &pres.slides[0];
    assert_eq!(s1.elements.len(), 1, "{:?}", s1.elements.iter().map(|e| e.id()).collect::<Vec<_>>());
    let os_next::core::models::SlideElement::TextBlock { transform, block, .. } = &s1.elements[0] else { panic!("expected a TextBlock") };
    // "Hello " (plain), "World plain" (styled -- LibreOffice's own editor merged "World" and
    // " plain" into one run under the same direct formatting once both were typed with the same
    // character attributes active; a real editing-model behavior, not an importer bug), "\n"
    // (the paragraph break), "Second paragraph left" (styled the same way, independently, in its
    // own paragraph -- see testdata/odp/README.md).
    assert_eq!(block.runs.len(), 4, "{:?}", block.runs);
    assert_eq!(block.runs[0].text, "Hello ");
    assert!(!block.runs[0].bold && !block.runs[0].italic);
    let styled = &block.runs[1];
    assert_eq!(styled.text, "World plain");
    assert!(styled.bold && styled.italic, "{:?}", styled);
    assert_eq!(styled.color.to_lowercase(), "#ff0000");
    assert_eq!(styled.font_family, "Liberation Sans");
    assert!((styled.font_size_pt - 28.0).abs() < 0.01);
    assert_eq!(block.runs[2].text, "\n");
    let styled2 = &block.runs[3];
    assert_eq!(styled2.text, "Second paragraph left");
    assert!(styled2.bold && styled2.italic && styled2.color.eq_ignore_ascii_case("#ff0000"));
    assert_eq!(block.paragraph_style.align, "right", "the first paragraph (text-align=right) sets the shape's alignment");
    // Position: svg:x=2cm y=1.5cm w=16cm h=3cm on a 28cm x 15.75cm page.
    assert!((transform.x - 2.0 / 28.0).abs() < 0.001, "{}", transform.x);
    assert!((transform.y - 1.5 / 15.75).abs() < 0.001, "{}", transform.y);
    assert!((transform.w - 16.0 / 28.0).abs() < 0.001, "{}", transform.w);
    assert!((transform.h - 3.0 / 15.75).abs() < 0.001, "{}", transform.h);
    assert_eq!(transform.z_index, 0);
    assert!(s1.text.contains("Hello World plain"), "{}", s1.text);

    // Slide 2: the green rectangle (a bare `draw:rect`, not a `draw:frame`), the image, and the
    // second text box all come through now, each at its own position, document order preserved.
    let s2 = &pres.slides[1];
    assert_eq!(s2.elements.len(), 3, "{:?}", s2.elements.iter().map(|e| e.id()).collect::<Vec<_>>());

    let os_next::core::models::SlideElement::Shape { transform: shp_t, shape_kind, fill_color, .. } = &s2.elements[0] else { panic!("expected the rectangle first (document order)") };
    assert_eq!(shape_kind, "rect");
    assert_eq!(fill_color.to_lowercase(), "#00aa00");
    assert_eq!(shp_t.z_index, 0);
    assert!((shp_t.x - 1.0 / 28.0).abs() < 0.001, "{}", shp_t.x);
    assert!((shp_t.w - 5.0 / 28.0).abs() < 0.001, "{}", shp_t.w);

    let os_next::core::models::SlideElement::Image { transform: img_t, file_path, .. } = &s2.elements[1] else { panic!("expected the image second (document order)") };
    assert!(file_path.starts_with("/media/images/odp_"), "{file_path}");
    assert_eq!(img_t.z_index, 1);
    assert!((img_t.x - 3.0 / 28.0).abs() < 0.001);
    let saved = media_dir.join(file_path.trim_start_matches("/media/images/"));
    assert!(saved.is_file(), "the embedded PNG must actually be written to media_dir");
    assert_eq!(std::fs::metadata(&saved).unwrap().len() > 0, true);

    let os_next::core::models::SlideElement::TextBlock { transform: t2, block: b2, .. } = &s2.elements[2] else { panic!("expected the second text box third") };
    assert_eq!(b2.runs[0].text, "Second slide title");
    assert_eq!(t2.z_index, 2);
}

#[test]
fn test_odp_import_backgrounds_notes_bare_text_and_error_cases() {
    let tmp = tempfile::tempdir().unwrap();
    let media_dir = tmp.path().join("media/images");

    let content_xml = r##"<?xml version="1.0"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0">
<office:automatic-styles>
  <style:style style:name="dp1" style:family="drawing-page">
    <style:drawing-page-properties draw:fill="solid" draw:fill-color="#336699"/>
  </style:style>
  <style:style style:name="dp2" style:family="drawing-page">
    <style:drawing-page-properties draw:fill="bitmap" draw:fill-image-name="bg1"/>
  </style:style>
  <draw:fill-image draw:name="bg1" xlink:href="Pictures/bg.png"/>
</office:automatic-styles>
<office:body><office:presentation>
  <draw:page draw:name="page1" draw:style-name="dp1" draw:master-page-name="Default">
    <draw:frame svg:x="1cm" svg:y="1cm" svg:width="10cm" svg:height="2cm">
      <draw:text-box><text:p>Bare paragraph, no span</text:p></draw:text-box>
    </draw:frame>
    <presentation:notes draw:style-name="dp1">
      <draw:page-thumbnail svg:x="0cm" svg:y="0cm" svg:width="1cm" svg:height="1cm" presentation:class="page"/>
      <draw:frame presentation:class="notes"><draw:text-box><text:p>Speaker note line one</text:p><text:p>line two</text:p></draw:text-box></draw:frame>
    </presentation:notes>
  </draw:page>
  <draw:page draw:name="page2" draw:style-name="dp2" draw:master-page-name="Default">
    <draw:frame svg:x="1cm" svg:y="1cm" svg:width="5cm" svg:height="2cm"><draw:text-box><text:p>Image bg slide</text:p></draw:text-box></draw:frame>
  </draw:page>
</office:presentation></office:body>
</office:document-content>"##;
    let styles_xml = r##"<?xml version="1.0"?>
<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0">
  <office:master-styles><style:master-page style:name="Default" style:page-layout-name="PM1"/></office:master-styles>
  <office:automatic-styles>
    <style:page-layout style:name="PM1"><style:page-layout-properties fo:page-width="28cm" fo:page-height="15.75cm"/></style:page-layout>
  </office:automatic-styles>
</office:document-styles>"##;
    let png = [0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0,0,0,0];
    let zip_bytes = make_zip(&[
        ("mimetype", b"application/vnd.oasis.opendocument.presentation"),
        ("content.xml", content_xml.as_bytes()),
        ("styles.xml", styles_xml.as_bytes()),
        ("Pictures/bg.png", &png),
    ]);
    let pres = os_next::storage::OdpImporter::import_odp_bytes(&zip_bytes, "Stem", &media_dir).unwrap();
    assert_eq!(pres.title, "Stem", "no meta.xml at all: falls back to the file stem");
    assert_eq!(pres.slides.len(), 2);

    let s1 = &pres.slides[0];
    assert_eq!(s1.background.as_deref(), Some("#336699"));
    assert_eq!(s1.speaker_notes.as_deref(), Some("Speaker note line one\nline two"));
    let os_next::core::models::SlideElement::TextBlock { block, .. } = &s1.elements[0] else { panic!() };
    assert_eq!(block.runs.len(), 1);
    assert_eq!(block.runs[0].text, "Bare paragraph, no span");
    assert_eq!(block.runs[0].color, "#ffffff", "no style at all: the model's own default, not something this importer invented");

    let s2 = &pres.slides[1];
    match s2.background_v2.as_ref().unwrap() {
        os_next::core::models::SlideBackground::Image { file_path, .. } => {
            assert!(media_dir.join(file_path.trim_start_matches("/media/images/")).is_file());
        }
        other => panic!("expected an Image background: {other:?}"),
    }

    // Error cases: never panics, always a readable Err.
    assert!(os_next::storage::OdpImporter::import_odp_bytes(b"not a zip", "x", &media_dir).is_err());
    let wrong_mime = make_zip(&[("mimetype", b"application/zip"), ("content.xml", b"<office:document-content/>")]);
    assert!(os_next::storage::OdpImporter::import_odp_bytes(&wrong_mime, "x", &media_dir).is_err());
    let no_pages = make_zip(&[
        ("mimetype", b"application/vnd.oasis.opendocument.presentation"),
        ("content.xml", b"<office:document-content><office:body><office:presentation/></office:body></office:document-content>"),
    ]);
    assert!(os_next::storage::OdpImporter::import_odp_bytes(&no_pages, "x", &media_dir).is_err());
}

#[test]
fn test_odp_import_reconstructs_shapes_lines_tables_and_groups() {
    let tmp = tempfile::tempdir().unwrap();
    let media_dir = tmp.path().join("media/images");

    let content_xml = r##"<?xml version="1.0"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0">
<office:automatic-styles>
  <style:style style:name="grEllipse" style:family="graphic"><style:graphic-properties draw:fill-color="#ff0000" draw:stroke="none"/></style:style>
  <style:style style:name="grTri" style:family="graphic"><style:graphic-properties draw:fill-color="#00ff00" draw:stroke="none"/></style:style>
  <style:style style:name="grLine" style:family="graphic"><style:graphic-properties draw:stroke="solid" svg:stroke-color="#0000ff" svg:stroke-width="0.05cm" draw:marker-end="Arrow"/></style:style>
  <style:style style:name="grElbow" style:family="graphic"><style:graphic-properties draw:stroke="solid" svg:stroke-color="#123456" svg:stroke-width="0.03cm"/></style:style>
  <style:style style:name="grCurve" style:family="graphic"><style:graphic-properties draw:stroke="solid" svg:stroke-color="#654321" svg:stroke-width="0.03cm"/></style:style>
  <style:style style:name="grRect" style:family="graphic"><style:graphic-properties draw:fill-color="#abcdef"/></style:style>
</office:automatic-styles>
<office:body><office:presentation>
  <draw:page draw:name="page1" draw:style-name="dp1" draw:master-page-name="Default">
    <draw:ellipse draw:style-name="grEllipse" svg:x="1cm" svg:y="1cm" svg:width="4cm" svg:height="3cm"/>
    <draw:custom-shape draw:style-name="grTri" svg:x="6cm" svg:y="1cm" svg:width="4cm" svg:height="3cm">
      <draw:enhanced-geometry svg:viewBox="0 0 21600 21600" draw:type="isosceles-triangle" draw:enhanced-path="M 10800 0 L 21600 21600 0 21600 Z N"/>
    </draw:custom-shape>
    <draw:line draw:style-name="grLine" svg:x1="1cm" svg:y1="6cm" svg:x2="6cm" svg:y2="7cm"/>
    <draw:polyline draw:style-name="grElbow" svg:x="8cm" svg:y="6cm" svg:width="4cm" svg:height="3cm" svg:viewBox="0 0 100 100" draw:points="5,5 50,5 50,95 95,95"/>
    <draw:path draw:style-name="grCurve" svg:x="13cm" svg:y="6cm" svg:width="4cm" svg:height="3cm" svg:viewBox="0 0 100 100" svg:d="M 5 5 C 35 65 65 80 95 50"/>
    <draw:g draw:style-name="grp1">
      <draw:rect draw:style-name="grRect" svg:x="20cm" svg:y="1cm" svg:width="2cm" svg:height="1cm"/>
      <draw:rect draw:style-name="grRect" svg:x="23cm" svg:y="2cm" svg:width="2cm" svg:height="1cm"/>
    </draw:g>
  </draw:page>
</office:presentation></office:body>
</office:document-content>"##;
    let styles_xml = r##"<?xml version="1.0"?>
<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0">
  <office:master-styles><style:master-page style:name="Default" style:page-layout-name="PM1"/></office:master-styles>
  <office:automatic-styles>
    <style:page-layout style:name="PM1"><style:page-layout-properties fo:page-width="28cm" fo:page-height="15.75cm"/></style:page-layout>
  </office:automatic-styles>
</office:document-styles>"##;
    let zip_bytes = make_zip(&[
        ("mimetype", b"application/vnd.oasis.opendocument.presentation"),
        ("content.xml", content_xml.as_bytes()),
        ("styles.xml", styles_xml.as_bytes()),
    ]);
    let pres = os_next::storage::OdpImporter::import_odp_bytes(&zip_bytes, "Stem", &media_dir).unwrap();
    let els = &pres.slides[0].elements;
    assert_eq!(els.len(), 6, "{:?}", els.iter().map(|e| e.id()).collect::<Vec<_>>());

    use os_next::core::models::SlideElement;
    let SlideElement::Shape { shape_kind, fill_color, .. } = &els[0] else { panic!("expected ellipse: {:?}", els[0]) };
    assert_eq!(shape_kind, "ellipse");
    assert_eq!(fill_color.to_lowercase(), "#ff0000");

    let SlideElement::Shape { shape_kind, fill_color, .. } = &els[1] else { panic!("expected a custom shape (triangle): {:?}", els[1]) };
    assert_eq!(shape_kind, "triangle", "ODF's 'isosceles-triangle' preset name maps to this app's 'triangle'");
    assert_eq!(fill_color.to_lowercase(), "#00ff00");

    let SlideElement::Line { line_kind: _, color, end_arrow, start_arrow, .. } = &els[2] else { panic!("expected a straight line: {:?}", els[2]) };
    assert_eq!(color.to_lowercase(), "#0000ff");
    assert!(*end_arrow, "draw:marker-end means an arrowhead regardless of line_kind");
    assert!(!*start_arrow);

    let SlideElement::Line { line_kind, color, .. } = &els[3] else { panic!("expected an elbow polyline: {:?}", els[3]) };
    assert_eq!(line_kind, "elbow", "4 points -> elbow, not straight");
    assert_eq!(color.to_lowercase(), "#123456");

    let SlideElement::Line { line_kind, color, .. } = &els[4] else { panic!("expected a curved path: {:?}", els[4]) };
    assert_eq!(line_kind, "curved");
    assert_eq!(color.to_lowercase(), "#654321");

    let SlideElement::Group { children, .. } = &els[5] else { panic!("expected a group: {:?}", els[5]) };
    assert_eq!(children.len(), 2);
    for child in children {
        let SlideElement::Shape { fill_color, .. } = child else { panic!("expected rects inside the group: {:?}", child) };
        assert_eq!(fill_color.to_lowercase(), "#abcdef");
    }
}

/// `draw:polygon`/`draw:regular-polygon`/`draw:caption`/`draw:a` are all real, spec-defined ODF
/// elements (ISO/IEC 26300, confirmed against the actual OASIS schema, not LibreOffice's own UI
/// conventions -- see docs/IMPORT_EXPORT_NOTES.md), distinct from anything `draw:custom-shape`'s
/// preset-name vocabulary covers. `draw:polygon` in particular was a real silent-drop gap before
/// this test was added: it was simply absent from the page-shape dispatcher's candidate list, so
/// a file using one (rather than the functionally-equivalent `draw:custom-shape`) lost that shape
/// entirely.
#[test]
fn test_odp_import_reconstructs_spec_elements_pptx_gallery_doesnt_produce() {
    let tmp = tempfile::tempdir().unwrap();
    let media_dir = tmp.path().join("media/images");

    let content_xml = r##"<?xml version="1.0"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  xmlns:xlink="http://www.w3.org/1999/xlink"
  xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0">
<office:automatic-styles>
  <style:style style:name="grPoly" style:family="graphic"><style:graphic-properties draw:fill-color="#223344" draw:stroke="none"/></style:style>
  <style:style style:name="grHex" style:family="graphic"><style:graphic-properties draw:fill-color="#556677" draw:stroke="none"/></style:style>
  <style:style style:name="grStar" style:family="graphic"><style:graphic-properties draw:fill-color="#887766" draw:stroke="none"/></style:style>
  <style:style style:name="grCaption" style:family="graphic"><style:graphic-properties draw:fill-color="#aabbcc" draw:stroke="none"/></style:style>
  <style:style style:name="grLinked" style:family="graphic"><style:graphic-properties draw:fill-color="#112233" draw:stroke="none"/></style:style>
</office:automatic-styles>
<office:body><office:presentation>
  <draw:page draw:name="page1" draw:style-name="dp1" draw:master-page-name="Default">
    <draw:polygon draw:style-name="grPoly" svg:x="1cm" svg:y="1cm" svg:width="4cm" svg:height="3cm" svg:viewBox="0 0 4000 3000" draw:points="0,0 2000,500 4000,0 2000,3000"/>
    <draw:regular-polygon draw:style-name="grHex" svg:x="6cm" svg:y="1cm" svg:width="4cm" svg:height="3cm" draw:corners="6"/>
    <draw:regular-polygon draw:style-name="grStar" svg:x="11cm" svg:y="1cm" svg:width="4cm" svg:height="3cm" draw:corners="6" draw:concave="true"/>
    <draw:caption draw:style-name="grCaption" svg:x="1cm" svg:y="6cm" svg:width="4cm" svg:height="3cm" draw:caption-point-x="0cm" draw:caption-point-y="0cm"/>
    <draw:a xlink:href="https://example.org/">
      <draw:rect draw:style-name="grLinked" svg:x="6cm" svg:y="6cm" svg:width="4cm" svg:height="3cm"/>
    </draw:a>
  </draw:page>
</office:presentation></office:body>
</office:document-content>"##;
    let styles_xml = r##"<?xml version="1.0"?>
<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0">
  <office:master-styles><style:master-page style:name="Default" style:page-layout-name="PM1"/></office:master-styles>
  <office:automatic-styles>
    <style:page-layout style:name="PM1"><style:page-layout-properties fo:page-width="28cm" fo:page-height="15.75cm"/></style:page-layout>
  </office:automatic-styles>
</office:document-styles>"##;
    let zip_bytes = make_zip(&[
        ("mimetype", b"application/vnd.oasis.opendocument.presentation"),
        ("content.xml", content_xml.as_bytes()),
        ("styles.xml", styles_xml.as_bytes()),
    ]);
    let pres = os_next::storage::OdpImporter::import_odp_bytes(&zip_bytes, "Stem", &media_dir).unwrap();
    let els = &pres.slides[0].elements;
    assert_eq!(els.len(), 5, "{:?}", els.iter().map(|e| e.id()).collect::<Vec<_>>());

    use os_next::core::models::SlideElement;
    let SlideElement::Shape { shape_kind, fill_color, .. } = &els[0] else { panic!("expected the free-form polygon: {:?}", els[0]) };
    assert_eq!(shape_kind, "rect", "no point-list field on Shape to preserve the exact outline, so it falls back to a plain rect -- position/color still preserved, not dropped");
    assert_eq!(fill_color.to_lowercase(), "#223344");
    assert!((els[0].transform().x - 1.0 / 28.0).abs() < 0.001, "{}", els[0].transform().x);

    let SlideElement::Shape { shape_kind, fill_color, .. } = &els[1] else { panic!("expected the regular hexagon: {:?}", els[1]) };
    assert_eq!(shape_kind, "hexagon", "draw:corners=\"6\" maps to this app's hexagon glyph");
    assert_eq!(fill_color.to_lowercase(), "#556677");

    let SlideElement::Shape { shape_kind, fill_color, .. } = &els[2] else { panic!("expected the concave (star-like) regular polygon: {:?}", els[2]) };
    assert_eq!(shape_kind, "star", "draw:concave=\"true\" always maps to this app's one star glyph, regardless of corner count");
    assert_eq!(fill_color.to_lowercase(), "#887766");

    let SlideElement::Shape { shape_kind, fill_color, .. } = &els[3] else { panic!("expected the caption: {:?}", els[3]) };
    assert_eq!(shape_kind, "callout", "draw:caption reuses this app's existing callout glyph");
    assert_eq!(fill_color.to_lowercase(), "#aabbcc");

    let SlideElement::Shape { shape_kind, fill_color, .. } = &els[4] else { panic!("expected the hyperlink-wrapped rect, unwrapped in place: {:?}", els[4]) };
    assert_eq!(shape_kind, "rect");
    assert_eq!(fill_color.to_lowercase(), "#112233");
    assert_eq!(els[4].transform().z_index, 4, "unwrapped content continues the page's own z-order sequence, not a restarted-at-0 local one");
}

#[test]
fn test_odp_import_reconstructs_a_table_with_a_merged_cell_padded_out() {
    let tmp = tempfile::tempdir().unwrap();
    let media_dir = tmp.path().join("media/images");
    let content_xml = r##"<?xml version="1.0"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0">
<office:body><office:presentation>
  <draw:page draw:name="page1">
    <draw:frame svg:x="1cm" svg:y="1cm" svg:width="8cm" svg:height="3cm">
      <table:table>
        <table:table-column/><table:table-column/>
        <table:table-row>
          <table:table-cell table:number-columns-spanned="2"><text:p>Spans both</text:p></table:table-cell>
          <table:covered-table-cell/>
        </table:table-row>
        <table:table-row>
          <table:table-cell><text:p>A</text:p></table:table-cell>
          <table:table-cell><text:p>B</text:p></table:table-cell>
        </table:table-row>
      </table:table>
    </draw:frame>
  </draw:page>
</office:presentation></office:body>
</office:document-content>"##;
    let zip_bytes = make_zip(&[
        ("mimetype", b"application/vnd.oasis.opendocument.presentation"),
        ("content.xml", content_xml.as_bytes()),
    ]);
    let pres = os_next::storage::OdpImporter::import_odp_bytes(&zip_bytes, "Stem", &media_dir).unwrap();
    let os_next::core::models::SlideElement::Table { rows, cols, cells, .. } = &pres.slides[0].elements[0] else { panic!() };
    assert_eq!(*rows, 2);
    assert_eq!(*cols, 2);
    assert_eq!(cells[0], vec!["Spans both".to_string(), String::new()], "the covered cell is padded back in as empty, not left short");
    assert_eq!(cells[1], vec!["A".to_string(), "B".to_string()]);
}

#[test]
fn test_odp_export_round_trips_through_this_apps_own_importer() {
    let tmp = tempfile::tempdir().unwrap();
    let png = [0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4, 5, 6, 7, 8];
    let pres = sample_presentation_for_export(tmp.path(), &png);

    let bytes = os_next::storage::OdpExporter::export_presentation(&pres, tmp.path()).unwrap();
    assert!(bytes.len() > 100);

    // It's a real zip whose first entry is the stored, uncompressed mimetype -- the ODF-specific
    // convention a generic sniffer uses, checked directly rather than trusting the writer's intent.
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
    assert_eq!(archive.by_index(0).unwrap().name(), "mimetype");
    assert_eq!(archive.by_index(0).unwrap().compression(), zip::CompressionMethod::Stored);
    let mut mt = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("mimetype").unwrap(), &mut mt).unwrap();
    assert_eq!(mt, "application/vnd.oasis.opendocument.presentation");
    drop(archive);

    let reimport_media = tmp.path().join("reimport");
    let reimported = os_next::storage::OdpImporter::import_odp_bytes(&bytes, "stem", &reimport_media).expect("this app's own export must import cleanly");
    assert_eq!(reimported.title, "Export <Test> & \"Quotes\"", "XML special characters in the title round-trip");
    assert_eq!(reimported.slides.len(), 2);

    let s1 = &reimported.slides[0];
    assert_eq!(s1.background.as_deref(), Some("#123456"));
    // The missing-file image was skipped; the real text block and the real image survive, in order.
    assert_eq!(s1.elements.len(), 2);
    let os_next::core::models::SlideElement::TextBlock { block, transform, .. } = &s1.elements[0] else { panic!() };
    assert_eq!(transform.z_index, 0);
    assert_eq!(block.runs.len(), 4, "{:?}", block.runs);
    assert_eq!(block.runs[0].text, "Plain ");
    assert!(!block.runs[0].bold);
    assert_eq!(block.runs[1].text, "Bold Red");
    assert!(block.runs[1].bold && block.runs[1].italic && block.runs[1].underline && block.runs[1].strike);
    assert_eq!(block.runs[1].color.to_lowercase(), "#ff00aa");
    assert_eq!(block.runs[1].font_family, "Georgia");
    assert!((block.runs[1].font_size_pt - 40.0).abs() < 0.01);
    assert_eq!(block.runs[2].text, "\n");
    assert_eq!(block.runs[3].text, "Second line");
    assert_eq!(block.paragraph_style.align, "right");
    assert!((transform.x - 0.1).abs() < 0.001 && (transform.w - 0.5).abs() < 0.001);

    let os_next::core::models::SlideElement::Image { file_path, transform: img_t, .. } = &s1.elements[1] else { panic!("expected the image") };
    assert_eq!(img_t.z_index, 1);
    assert!((img_t.x - 0.2).abs() < 0.001);
    let saved = reimport_media.join(file_path.trim_start_matches("/media/images/"));
    assert_eq!(std::fs::read(&saved).unwrap(), png);

    let s2 = &reimported.slides[1];
    assert_eq!(s2.elements.len(), 7, "{:?}", s2.elements.iter().map(|e| e.id()).collect::<Vec<_>>());
    let os_next::core::models::SlideElement::TextBlock { block, .. } = &s2.elements[0] else { panic!() };
    assert_eq!(block.runs[0].text, "Second slide");

    let os_next::core::models::SlideElement::Shape { shape_kind, fill_color, stroke_color, stroke_width, .. } = &s2.elements[1] else { panic!("expected the rect shape: {:?}", s2.elements[1]) };
    assert_eq!(shape_kind, "rect");
    assert_eq!(fill_color.to_lowercase(), "#aabbcc");
    assert_eq!(stroke_color.as_deref().map(str::to_lowercase).as_deref(), Some("#112233"));
    assert!((stroke_width - 4.0).abs() < 0.5, "relative stroke width must survive the cm round trip within rounding: {stroke_width}");

    let os_next::core::models::SlideElement::Shape { shape_kind, fill_color, .. } = &s2.elements[2] else { panic!("expected the triangle: {:?}", s2.elements[2]) };
    assert_eq!(shape_kind, "triangle", "this app's own shape_kind strings pass through draw:type unchanged");
    assert_eq!(fill_color.to_lowercase(), "#00ff00");

    let os_next::core::models::SlideElement::Line { color, end_arrow, start_arrow, .. } = &s2.elements[3] else { panic!("expected the arrow line: {:?}", s2.elements[3]) };
    assert_eq!(color.to_lowercase(), "#ff8800");
    assert!(*end_arrow && !*start_arrow);

    let os_next::core::models::SlideElement::Line { line_kind, color, .. } = &s2.elements[4] else { panic!("expected the elbow line: {:?}", s2.elements[4]) };
    assert_eq!(line_kind, "elbow");
    assert_eq!(color.to_lowercase(), "#00aaff");

    let os_next::core::models::SlideElement::Table { rows, cols, cells, .. } = &s2.elements[5] else { panic!("expected the table: {:?}", s2.elements[5]) };
    assert_eq!(*rows, 2);
    assert_eq!(*cols, 2);
    assert_eq!(cells[0], vec!["R0C0".to_string(), "R0C1".to_string()]);
    assert_eq!(cells[1], vec!["R1C0".to_string(), "R1C1".to_string()]);

    let os_next::core::models::SlideElement::Group { children, .. } = &s2.elements[6] else { panic!("expected the group: {:?}", s2.elements[6]) };
    assert_eq!(children.len(), 2);
    let os_next::core::models::SlideElement::Shape { fill_color: c0, .. } = &children[0] else { panic!() };
    let os_next::core::models::SlideElement::Shape { fill_color: c1, .. } = &children[1] else { panic!() };
    assert_eq!([c0.to_lowercase(), c1.to_lowercase()], ["#ff00ff", "#ffff00"]);
}

#[test]
fn test_odp_export_is_a_real_file_real_libreoffice_accepts() {
    if std::process::Command::new("soffice").arg("--version").output().is_err() {
        eprintln!("skipping: no `soffice` on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let png = [0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
    let pres = sample_presentation_for_export(tmp.path(), &png);
    let bytes = os_next::storage::OdpExporter::export_presentation(&pres, tmp.path()).unwrap();
    let odp_path = tmp.path().join("export_test.odp");
    std::fs::write(&odp_path, &bytes).unwrap();

    let profile = tmp.path().join("lo_profile");
    std::fs::create_dir_all(&profile).unwrap();
    let out = std::process::Command::new("soffice")
        .arg("--headless")
        .arg("--invisible")
        .arg("--norestore")
        .arg(format!("-env:UserInstallation=file://{}", profile.display()))
        .arg("--convert-to").arg("pdf")
        .arg("--outdir").arg(tmp.path())
        .arg(&odp_path)
        .output()
        .expect("soffice must run");
    assert!(out.status.success(), "soffice failed: {}", String::from_utf8_lossy(&out.stderr));
    let pdf = tmp.path().join("export_test.pdf");
    assert!(pdf.is_file(), "no PDF produced: {}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let pdf_bytes = std::fs::read(&pdf).unwrap();
    assert!(pdf_bytes.starts_with(b"%PDF"), "not a real PDF");
    assert!(pdf_bytes.len() > 1000, "suspiciously small PDF ({} bytes): real LibreOffice couldn't have rendered real content", pdf_bytes.len());
}

#[tokio::test]
async fn test_export_odp_route_is_console_only_and_downloads_a_real_file() {
    let tmp = tempfile::tempdir().unwrap();
    let db = os_next::storage::Database::new(tmp.path().join("export.db").to_str().unwrap()).unwrap();
    let media_dir = tmp.path().join("media");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app_state = os_next::api::ws::AppState {
        media_dir: media_dir.clone(), host_session_token: "export_host".to_string(), server_port: addr.port(),
        ..base_app_state(db.clone())
    };
    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let h = "export_host";

    let pres = os_next::core::models::Presentation {
        id: "pres_export_route_test".to_string(),
        title: "Route Test".to_string(),
        author: String::new(),
        theme_name: None,
        slides: vec![os_next::core::models::Slide {
            label: Some("Slide 1".to_string()),
            elements: vec![os_next::core::models::SlideElement::TextBlock {
                id: "el_1".to_string(),
                transform: os_next::core::models::ElementTransform::default(),
                block: os_next::core::models::TextBlock {
                    runs: vec![os_next::core::models::TextRun { text: "Hi".to_string(), ..Default::default() }],
                    paragraph_style: Default::default(), effects: Default::default(), autofit: false,
                },
            }],
            slide_document_version: 1,
            ..Default::default()
        }],
    };
    db.insert_presentation(&pres).unwrap();

    // Console-only, whatever the id.
    assert_eq!(c.get(format!("{base}/api/export/odp/{}", pres.id)).send().await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(c.get(format!("{base}/api/export/odp/nope")).header("x-host-token", h).send().await.unwrap().status(), reqwest::StatusCode::NOT_FOUND);

    let r = c.get(format!("{base}/api/export/odp/{}", pres.id)).header("x-host-token", h).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    assert_eq!(r.headers().get("content-type").unwrap(), "application/vnd.oasis.opendocument.presentation");
    assert!(r.headers().get("content-disposition").unwrap().to_str().unwrap().contains("Route Test.odp"));
    let bytes = r.bytes().await.unwrap();
    assert!(bytes.len() > 100);

    // It's genuinely importable -- the route isn't just handing back some placeholder.
    let reimported = os_next::storage::OdpImporter::import_odp_bytes(&bytes, "x", &media_dir.join("images")).unwrap();
    assert_eq!(reimported.title, "Route Test");
    let os_next::core::models::SlideElement::TextBlock { block, .. } = &reimported.slides[0].elements[0] else { panic!() };
    assert_eq!(block.runs[0].text, "Hi");
}

#[test]
fn test_odp_export_omits_this_apps_own_placeholder_color_and_font_so_other_tools_dont_render_it_invisible() {
    use os_next::core::models::*;
    let tmp = tempfile::tempdir().unwrap();
    let never_styled = SlideElement::TextBlock {
        id: "el_1".to_string(),
        transform: ElementTransform { z_index: 0, ..ElementTransform::default() },
        block: TextBlock { runs: vec![TextRun { text: "plain".to_string(), ..Default::default() }], paragraph_style: TextParagraphStyle::default(), effects: ElementEffects::default(), autofit: false },
    };
    let explicitly_white = SlideElement::TextBlock {
        id: "el_2".to_string(),
        transform: ElementTransform { z_index: 1, ..ElementTransform::default() },
        block: TextBlock {
            runs: vec![TextRun { text: "x".to_string(), color: "#ffffff".to_string(), font_family: "Comic Sans MS".to_string(), ..Default::default() }],
            paragraph_style: TextParagraphStyle::default(), effects: ElementEffects::default(), autofit: false,
        },
    };
    let pres = Presentation {
        id: "p".to_string(), title: "t".to_string(), author: String::new(), theme_name: None,
        slides: vec![Slide { elements: vec![never_styled, explicitly_white], slide_document_version: 1, ..Default::default() }],
    };
    let bytes = os_next::storage::OdpExporter::export_presentation(&pres, tmp.path()).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
    let mut content = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("content.xml").unwrap(), &mut content).unwrap();

    // A run never styled beyond this struct's own defaults doesn't force this app's placeholder
    // color/font into the file at all -- so a real ODF tool's own default (black on white) shows,
    // instead of white-on-white (this exact bug was found by hand: import testdata/odp/fixture.odp
    // through the real app, export it back out, and open the result in real LibreOffice -- "Hello "
    // visibly disappeared).
    let t_styles: Vec<&str> = content.match_indices("style:family=\"text\"").map(|(i, _)| {
        let start = content[..i].rfind("<style:style").unwrap();
        let end = content[i..].find("</style:style>").unwrap() + i + "</style:style>".len();
        &content[start..end]
    }).collect();
    assert_eq!(t_styles.len(), 2, "{t_styles:?}");
    assert!(!t_styles[0].contains("fo:color"), "{}", t_styles[0]);
    assert!(!t_styles[0].contains("style:font-name"), "{}", t_styles[0]);
    // ...but a run that happens to ask for white explicitly, alongside a real/different font,
    // keeps its color -- only the exact (default color, default font) pair together is omitted.
    assert!(t_styles[1].contains("fo:color=\"#ffffff\""), "{}", t_styles[1]);
    assert!(t_styles[1].contains("style:font-name=\"Comic Sans MS\""), "{}", t_styles[1]);

    // Re-importing it still reconstructs this app's own white/default-font look, since the model's
    // own Default is exactly those values -- nothing is lost for a document that stays inside this app.
    let reimported = os_next::storage::OdpImporter::import_odp_bytes(&bytes, "x", &tmp.path().join("images")).unwrap();
    let SlideElement::TextBlock { block, .. } = &reimported.slides[0].elements[0] else { panic!() };
    assert_eq!(block.runs[0].color, TextRun::default().color);
    assert_eq!(block.runs[0].font_family, TextRun::default().font_family);
}

/// A real-world ODP, authored by hand in actual LibreOffice Impress using its "Basic Shapes"/
/// "Symbol Shapes"/"Arrows" gallery (not built via `pyuno` like `testdata/odp/fixture.odp`, and
/// not committed to the repo -- a human's own test file, not ours to redistribute), via the same
/// `$OS_TEST_DOCS` convention `docs/workflow.md` already documents for real EasyWorship fixtures:
/// skip quietly if it isn't present on this machine. Its 9 shapes (a callout, a diamond, an
/// MSO-imported blob shape, a trapezoid, a heart, a brace pair, a left-right arrow, a 5-point
/// star, and a "bang"/explosion) all leave fill/stroke *unset on their own style* and inherit
/// from a shared parent style (almost universal real-producer behavior for a shape gallery, not
/// the simpler single-style-no-inheritance case `test_odp_import_reconstructs_shapes_lines_tables_
/// and_groups`'s hand-written XML covers) -- this is what caught `resolve_graphic_style` not
/// walking `style:parent-style-name` before this test was added. It also has a `dr3d:scene`
/// (a real extruded-cube 3D shape, from Impress's "3D Objects" gallery) alongside the 2D shapes,
/// genuinely out of scope (this app's `SlideElement` has no 3D concept at all) and confirmed here
/// to not confuse the page-shape scanner into misparsing anything around it.
#[test]
fn test_odp_import_against_a_real_world_shape_gallery_file() {
    let docs_dir = std::env::var("OS_TEST_DOCS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("Documents/OpenSanctuary-test-docs"));
    let path = docs_dir.join("shape-test.odp");
    if !path.exists() {
        eprintln!("skipping: {} not present (see docs/workflow.md's $OS_TEST_DOCS)", path.display());
        return;
    }
    let bytes = std::fs::read(&path).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let pres = os_next::storage::OdpImporter::import_odp_bytes(&bytes, "shape-test", &tmp.path().join("media")).expect("a real human-authored file must import");

    let els = &pres.slides[0].elements;
    assert_eq!(els.len(), 9, "all 9 gallery shapes must survive, and the dr3d:scene cube must not be mistaken for one: {:?}", els.iter().map(|e| e.id()).collect::<Vec<_>>());

    use os_next::core::models::SlideElement;
    let expected_kinds = ["callout", "diamond", "mso-spt100", "trapezoid", "heart", "brace-pair", "left-right-arrow", "star", "bang"];
    for (el, expected_kind) in els.iter().zip(expected_kinds) {
        let SlideElement::Shape { shape_kind, fill_color, .. } = el else { panic!("expected a Shape: {el:?}") };
        assert_eq!(shape_kind, expected_kind);
        // Every shape here leaves fill unset on its own style and inherits from a shared parent
        // ("standard", #18a303) -- except the brace pair, which really is an explicit outline-only
        // shape (its own style sets draw:fill="none" directly, which must win over the parent's
        // fill rather than being treated as "unset" and inherited through).
        if expected_kind == "brace-pair" {
            assert_eq!(fill_color, "none", "the brace pair's own explicit draw:fill=\"none\" must stop inheritance, not fall through to the parent's fill");
        } else {
            assert_eq!(fill_color.to_lowercase(), "#18a303", "{shape_kind} must inherit the parent style's fill, not come up empty");
        }
    }
}

/// The full round trip for the same real-world file `test_odp_import_against_a_real_world_shape_
/// gallery_file` imports: import it, export this app's own result, and hand *that* to real
/// LibreOffice -- covering something the synthetic `sample_presentation_for_export` data doesn't
/// (every one of its `Shape`s has an explicit literal color; these 9 only have one because
/// `resolve_graphic_style`'s inheritance walk resolved it, and 5 of the 9 are exotic preset types
/// this app doesn't natively render, falling back to `shape_enhanced_path`'s plain-rect glyph).
/// Same `$OS_TEST_DOCS` skip-if-missing convention as that test.
#[test]
fn test_odp_export_of_a_real_world_shape_gallery_file_is_accepted_by_real_libreoffice() {
    if std::process::Command::new("soffice").arg("--version").output().is_err() {
        eprintln!("skipping: no `soffice` on PATH");
        return;
    }
    let docs_dir = std::env::var("OS_TEST_DOCS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("Documents/OpenSanctuary-test-docs"));
    let path = docs_dir.join("shape-test.odp");
    if !path.exists() {
        eprintln!("skipping: {} not present (see docs/workflow.md's $OS_TEST_DOCS)", path.display());
        return;
    }
    let bytes = std::fs::read(&path).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let pres = os_next::storage::OdpImporter::import_odp_bytes(&bytes, "shape-test", &tmp.path().join("media")).unwrap();
    let out_bytes = os_next::storage::OdpExporter::export_presentation(&pres, &tmp.path().join("media")).unwrap();
    let odp_path = tmp.path().join("roundtrip.odp");
    std::fs::write(&odp_path, &out_bytes).unwrap();

    let profile = tmp.path().join("lo_profile");
    std::fs::create_dir_all(&profile).unwrap();
    let out = std::process::Command::new("soffice")
        .arg("--headless")
        .arg("--invisible")
        .arg("--norestore")
        .arg(format!("-env:UserInstallation=file://{}", profile.display()))
        .arg("--convert-to")
        .arg("png")
        .arg("--outdir")
        .arg(tmp.path())
        .arg(&odp_path)
        .output()
        .expect("soffice must run");
    assert!(out.status.success(), "soffice failed: {}", String::from_utf8_lossy(&out.stderr));
    let png = tmp.path().join("roundtrip.png");
    assert!(png.is_file(), "no PNG produced: {}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(std::fs::metadata(&png).unwrap().len() > 1000, "suspiciously small PNG: real LibreOffice couldn't have rendered real shapes");
}

/// Every `shape_kind` this app's own editor (`web/src/editor/shape_library.ts`) can draw, placed
/// on one slide, exported, and checked two ways: (1) re-imported by this app's own importer, so a
/// typo in either `shape_enhanced_path` (export) or `map_custom_shape_type` (import) that broke
/// the round trip for any one kind -- most likely to slip in when adding a new one, since they're
/// hand-kept in sync across three files, not generated from one shared list -- shows up as a
/// mismatch here; (2) handed to real LibreOffice, which must still accept the file and render
/// something (confirmed visually when this test was written: every shape distinct and
/// recognizable, see docs/IMPORT_EXPORT_NOTES.md).
#[test]
fn test_every_known_shape_kind_round_trips_and_is_accepted_by_real_libreoffice() {
    use os_next::core::models::*;
    let kinds = [
        "rect", "rounded_rect", "ellipse", "triangle", "diamond", "star", "callout", "trapezoid",
        "left-right-arrow", "parallelogram", "hexagon", "octagon", "cross", "pentagon", "heart",
        "lightning", "left-arrow", "right-arrow", "up-arrow", "down-arrow", "up-down-arrow",
        "quad-arrow", "chevron", "notched-right-arrow", "striped-right-arrow", "triangle-down", "ring",
    ];
    let mut elements = Vec::new();
    for (i, kind) in kinds.iter().enumerate() {
        let col = i % 6;
        let row = i / 6;
        elements.push(SlideElement::Shape {
            id: format!("el_{i}"),
            transform: ElementTransform { x: 0.02 + col as f64 * 0.16, y: 0.03 + row as f64 * 0.2, w: 0.13, h: 0.15, z_index: i as i32, ..Default::default() },
            shape_kind: kind.to_string(),
            fill_color: "#2a7a2a".to_string(),
            stroke_color: None,
            stroke_width: 0.0,
        });
    }
    let pres = Presentation {
        id: "pres_shapes".to_string(),
        title: "Shapes".to_string(),
        author: String::new(),
        theme_name: None,
        slides: vec![Slide { label: Some("Shapes".to_string()), elements, slide_document_version: 1, ..Default::default() }],
    };
    let tmp = tempfile::tempdir().unwrap();
    let bytes = os_next::storage::OdpExporter::export_presentation(&pres, tmp.path()).unwrap();

    let reimported = os_next::storage::OdpImporter::import_odp_bytes(&bytes, "shapes", &tmp.path().join("media")).unwrap();
    assert_eq!(reimported.slides[0].elements.len(), kinds.len(), "{:?}", reimported.slides[0].elements.iter().map(|e| e.id()).collect::<Vec<_>>());
    for (el, expected_kind) in reimported.slides[0].elements.iter().zip(kinds) {
        let SlideElement::Shape { shape_kind, fill_color, .. } = el else { panic!("expected a Shape: {el:?}") };
        assert_eq!(shape_kind, expected_kind, "round trip must reconstruct the exact original shape_kind");
        assert_eq!(fill_color.to_lowercase(), "#2a7a2a");
    }

    if std::process::Command::new("soffice").arg("--version").output().is_err() {
        eprintln!("skipping the real-LibreOffice half: no `soffice` on PATH");
        return;
    }
    let odp_path = tmp.path().join("all_shapes.odp");
    std::fs::write(&odp_path, &bytes).unwrap();
    let profile = tmp.path().join("lo_profile");
    std::fs::create_dir_all(&profile).unwrap();
    let out = std::process::Command::new("soffice")
        .arg("--headless")
        .arg("--invisible")
        .arg("--norestore")
        .arg(format!("-env:UserInstallation=file://{}", profile.display()))
        .arg("--convert-to")
        .arg("png")
        .arg("--outdir")
        .arg(tmp.path())
        .arg(&odp_path)
        .output()
        .expect("soffice must run");
    assert!(out.status.success(), "soffice failed: {}", String::from_utf8_lossy(&out.stderr));
    let png = tmp.path().join("all_shapes.png");
    assert!(png.is_file(), "no PNG produced: {}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(std::fs::metadata(&png).unwrap().len() > 2000, "suspiciously small PNG: real LibreOffice couldn't have rendered 27 real shapes");
}

/// A `Shape` with `shape_kind: "ring"` must come out as *unfilled* -- the one case where this
/// app's own `fill_color` becomes a stroke, not a fill (see `render_element`'s doc comment on why:
/// real LibreOffice renders two opposite-wound `draw:enhanced-path` subpaths as a solid union, not
/// a hole with a nonzero fill rule, confirmed by actually rendering one). The round-trip-identity
/// test above would not have caught a regression back to a filled ring (fill_color/shape_kind
/// would still match after re-import even if the exported XML made it render solid), so this
/// checks the XML directly instead of only the re-imported model.
#[test]
fn test_ring_shape_exports_as_unfilled_not_a_solid_blob() {
    use os_next::core::models::*;
    let pres = Presentation {
        id: "p".to_string(),
        title: "t".to_string(),
        author: String::new(),
        theme_name: None,
        slides: vec![Slide {
            label: Some("S".to_string()),
            elements: vec![SlideElement::Shape {
                id: "e1".to_string(),
                transform: ElementTransform { x: 0.1, y: 0.1, w: 0.3, h: 0.3, z_index: 0, ..Default::default() },
                shape_kind: "ring".to_string(),
                fill_color: "#cc2222".to_string(),
                stroke_color: None,
                stroke_width: 0.0,
            }],
            slide_document_version: 1,
            ..Default::default()
        }],
    };
    let tmp = tempfile::tempdir().unwrap();
    let bytes = os_next::storage::OdpExporter::export_presentation(&pres, tmp.path()).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
    let mut content = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("content.xml").unwrap(), &mut content).unwrap();
    assert!(content.contains(r#"draw:fill="none""#), "a ring's graphic style must be unfilled: {content}");
    assert!(content.contains(r##"svg:stroke-color="#cc2222""##), "the ring's color must come through as the stroke color: {content}");
    assert!(content.contains(r#"draw:type="ring""#), "draw:type must still identify this as a ring for re-import: {content}");

    // And the re-imported color must come from that stroke, round-tripping the original fill_color.
    let reimported = os_next::storage::OdpImporter::import_odp_bytes(&bytes, "ring", &tmp.path().join("media")).unwrap();
    let SlideElement::Shape { fill_color, .. } = &reimported.slides[0].elements[0] else { panic!() };
    assert_eq!(fill_color.to_lowercase(), "#cc2222");
}

/// A real-world `.pptx` produced by actual LibreOffice Impress (see
/// `testdata/pptx/README.md`), not hand-authored XML -- confirms `Table`/`Group`
/// reconstruction against genuine OOXML, including the two structural facts that
/// file's README calls out (a table's `<p:xfrm>` isn't nested in `<p:spPr>`; a
/// group's `<a:off>`/`<a:ext>` matching its own `<a:chOff>`/`<a:chExt>` in the
/// untouched-group case).
#[test]
fn test_pptx_import_against_a_real_libreoffice_fixture() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/pptx/fixture.pptx")).unwrap();
    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "stem", media_dir.path()).expect("real LibreOffice output must import");

    let els = &pres.slides[0].elements;
    assert_eq!(els.len(), 4, "{:?}", els.iter().map(|e| e.id()).collect::<Vec<_>>());

    use os_next::core::models::SlideElement;
    // The two title/subtitle placeholder boxes come through as plain, unfilled rects
    // (no text of their own in this fixture, but a real <a:prstGeom prst="rect">).
    assert!(matches!(&els[0], SlideElement::Shape { shape_kind, .. } if shape_kind == "rect"));
    assert!(matches!(&els[1], SlideElement::Shape { shape_kind, .. } if shape_kind == "rect"));

    let SlideElement::Table { rows, cols, cells, transform, .. } = &els[2] else { panic!("expected the table: {:?}", els[2]) };
    assert_eq!(*rows, 2);
    assert_eq!(*cols, 2);
    assert_eq!(cells[0], vec!["R0C0".to_string(), "R0C1".to_string()]);
    assert_eq!(cells[1], vec!["R1C0".to_string(), "R1C1".to_string()]);
    assert!(transform.x > 0.0 && transform.x < 1.0, "{}", transform.x);

    let SlideElement::Group { children, .. } = &els[3] else { panic!("expected the group: {:?}", els[3]) };
    assert_eq!(children.len(), 2, "both rectangles in the group must come through");
    for child in children {
        assert!(matches!(child, SlideElement::Shape { shape_kind, .. } if shape_kind == "rect"));
    }
    // The two children must be at genuinely different positions (the group's own
    // off/ext == chOff/chExt in this fixture, so this is really testing that the
    // GroupRemap identity case didn't collapse both children onto the same spot).
    assert_ne!(children[0].transform().x, children[1].transform().x);
}

/// Hand-authored XML for the shapes/lines/connectors this app's own editor can
/// render, covering exactly the cases `test_pptx_import_against_a_real_libreoffice_
/// fixture` doesn't (that fixture has no connectors, no callouts, no explicit
/// fill/stroke colors, and no nested group) -- attribute names and structure
/// (`a:prstGeom`, `a:solidFill`/`a:srgbClr`, `a:ln`/`a:headEnd`/`a:tailEnd`,
/// `p:cxnSp`) are the same ones confirmed against real LibreOffice output above and
/// in `docs/IMPORT_EXPORT_NOTES.md`, not invented for this test alone.
#[test]
fn test_pptx_import_reconstructs_shapes_lines_connectors_and_nested_groups() {
    use os_next::core::models::SlideElement;

    let slide_xml = r##"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:sp>
      <p:spPr><a:xfrm><a:off x="100000" y="100000"/><a:ext cx="500000" cy="400000"/></a:xfrm>
        <a:prstGeom prst="diamond"><a:avLst/></a:prstGeom>
        <a:solidFill><a:srgbClr val="FF00AA"/></a:solidFill>
        <a:ln w="25400"><a:solidFill><a:srgbClr val="112233"/></a:solidFill></a:ln>
      </p:spPr>
    </p:sp>
    <p:sp>
      <p:spPr><a:xfrm><a:off x="700000" y="100000"/><a:ext cx="500000" cy="400000"/></a:xfrm>
        <a:prstGeom prst="wedgeRectCallout"><a:avLst/></a:prstGeom>
        <a:solidFill><a:srgbClr val="00AA55"/></a:solidFill>
      </p:spPr>
    </p:sp>
    <p:sp>
      <p:spPr><a:xfrm><a:off x="100000" y="600000"/><a:ext cx="500000" cy="100000"/></a:xfrm>
        <a:prstGeom prst="line"><a:avLst/></a:prstGeom>
        <a:ln w="38160"><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill>
          <a:headEnd type="none"/><a:tailEnd type="triangle"/>
        </a:ln>
      </p:spPr>
    </p:sp>
    <p:cxnSp>
      <p:spPr><a:xfrm><a:off x="700000" y="600000"/><a:ext cx="500000" cy="300000"/></a:xfrm>
        <a:prstGeom prst="bentConnector3"><a:avLst/></a:prstGeom>
        <a:ln w="12700"><a:solidFill><a:srgbClr val="654321"/></a:solidFill></a:ln>
      </p:spPr>
    </p:cxnSp>
    <p:grpSp>
      <p:grpSpPr><a:xfrm>
        <a:off x="1500000" y="100000"/><a:ext cx="800000" cy="800000"/>
        <a:chOff x="0" y="0"/><a:chExt cx="400000" cy="400000"/>
      </a:xfrm></p:grpSpPr>
      <p:sp>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="200000" cy="200000"/></a:xfrm>
          <a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom>
          <a:solidFill><a:srgbClr val="AABBCC"/></a:solidFill>
        </p:spPr>
      </p:sp>
      <p:grpSp>
        <p:grpSpPr><a:xfrm>
          <a:off x="200000" y="200000"/><a:ext cx="200000" cy="200000"/>
          <a:chOff x="200000" y="200000"/><a:chExt cx="200000" cy="200000"/>
        </a:xfrm></p:grpSpPr>
        <p:sp>
          <p:spPr><a:xfrm><a:off x="200000" y="200000"/><a:ext cx="100000" cy="100000"/></a:xfrm>
            <a:prstGeom prst="star5"><a:avLst/></a:prstGeom>
            <a:solidFill><a:srgbClr val="FFEE00"/></a:solidFill>
          </p:spPr>
        </p:sp>
      </p:grpSp>
    </p:grpSp>
  </p:spTree></p:cSld>
</p:sld>"##;

    let bytes = build_single_slide_pptx(slide_xml);
    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "stem", media_dir.path()).expect("should parse");
    let els = &pres.slides[0].elements;
    assert_eq!(els.len(), 5, "{:?}", els.iter().map(|e| e.id()).collect::<Vec<_>>());

    let SlideElement::Shape { shape_kind, fill_color, stroke_color, stroke_width, .. } = &els[0] else { panic!("expected the diamond: {:?}", els[0]) };
    assert_eq!(shape_kind, "diamond");
    assert_eq!(fill_color.to_lowercase(), "#ff00aa");
    assert_eq!(stroke_color.as_deref().map(str::to_lowercase).as_deref(), Some("#112233"));
    assert!(*stroke_width > 0.0);

    let SlideElement::Shape { shape_kind, fill_color, .. } = &els[1] else { panic!("expected the callout: {:?}", els[1]) };
    assert_eq!(shape_kind, "callout", "OOXML's wedgeRectCallout maps to this app's callout glyph");
    assert_eq!(fill_color.to_lowercase(), "#00aa55");

    let SlideElement::Line { line_kind, color, start_arrow, end_arrow, .. } = &els[2] else { panic!("expected the plain-sp line: {:?}", els[2]) };
    assert_eq!(line_kind, "straight");
    assert_eq!(color.to_lowercase(), "#0000ff");
    assert!(!*start_arrow, "headEnd type=\"none\" means no start arrow");
    assert!(*end_arrow, "tailEnd type=\"triangle\" means an end arrow");

    let SlideElement::Line { line_kind, color, .. } = &els[3] else { panic!("expected the bent connector: {:?}", els[3]) };
    assert_eq!(line_kind, "elbow", "bentConnectorN maps to this app's elbow line_kind");
    assert_eq!(color.to_lowercase(), "#654321");

    let SlideElement::Group { children, .. } = &els[4] else { panic!("expected the outer group: {:?}", els[4]) };
    assert_eq!(children.len(), 2, "the ellipse and the nested group's own child, both unwrapped to this level");
    let SlideElement::Shape { shape_kind, fill_color, transform: ellipse_t, .. } = &children[0] else { panic!("expected the ellipse: {:?}", children[0]) };
    assert_eq!(shape_kind, "ellipse");
    assert_eq!(fill_color.to_lowercase(), "#aabbcc");
    // Outer group: off=(1500000,100000) ext=(800000,800000) chOff=(0,0) chExt=(400000,400000)
    // -> a uniform 2x scale. Ellipse's own local off/ext=(0,0)/(200000,200000) remap to
    // absolute EMU (1500000,100000)/(400000,400000), normalized against the default
    // 12192000x6858000 EMU slide size (no presentation.xml in this test).
    assert!((ellipse_t.x - 0.123_031_496).abs() < 1e-6, "{}", ellipse_t.x);
    assert!((ellipse_t.y - 0.014_581_511).abs() < 1e-6, "{}", ellipse_t.y);
    assert!((ellipse_t.w - 0.032_808_399).abs() < 1e-6, "{}", ellipse_t.w);
    assert!((ellipse_t.h - 0.058_326_043).abs() < 1e-6, "{}", ellipse_t.h);

    let SlideElement::Group { children: inner_children, .. } = &children[1] else { panic!("expected the nested group: {:?}", children[1]) };
    assert_eq!(inner_children.len(), 1);
    let SlideElement::Shape { shape_kind, fill_color, transform: star_t, .. } = &inner_children[0] else { panic!("expected the star: {:?}", inner_children[0]) };
    assert_eq!(shape_kind, "star");
    assert_eq!(fill_color.to_lowercase(), "#ffee00");
    // This is the number that actually exercises the nested-group fix: the inner
    // group's own off=(200000,200000)/ext=(200000,200000) must FIRST be remapped
    // through the OUTER group's 2x transform (-> absolute off=(1900000,500000),
    // ext=(400000,400000)) before its own chOff/chExt=(200000,200000)/(200000,200000)
    // (read raw, un-remapped) are used to build ITS remap -- giving a second 2x
    // scale on top. The star's local off/ext=(200000,200000)/(100000,100000) then
    // remap to absolute EMU (1900000,500000)/(200000,200000). If the inner group's
    // off/ext were used un-remapped instead (the bug this test exists to catch),
    // the star would land at a visibly different, wrong position.
    assert!((star_t.x - 0.155_839_895).abs() < 1e-6, "{}", star_t.x);
    assert!((star_t.y - 0.072_907_553).abs() < 1e-6, "{}", star_t.y);
    assert!((star_t.w - 0.016_404_199).abs() < 1e-6, "{}", star_t.w);
    assert!((star_t.h - 0.029_163_021).abs() < 1e-6, "{}", star_t.h);
}

/// A theme-colored shape (`<p:style><a:fillRef idx="2"><a:schemeClr val="accent1"/>`,
/// no explicit `<a:solidFill>` at all in `<p:spPr>` itself) -- the single most common
/// real-world gap this importer has, per its own module doc: a shape inserted via
/// the authoring tool's UI without manually overriding its fill very often looks
/// exactly like this. Must not crash, and must come through with this app's own
/// "nothing resolved" fallback (`"none"`), not a guessed color.
#[test]
fn test_pptx_import_theme_colored_shape_has_no_crash_and_no_guessed_color() {
    use os_next::core::models::SlideElement;
    let slide_xml = r##"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:sp>
      <p:spPr><a:xfrm><a:off x="100000" y="100000"/><a:ext cx="500000" cy="400000"/></a:xfrm>
        <a:prstGeom prst="hexagon"><a:avLst/></a:prstGeom>
      </p:spPr>
      <p:style>
        <a:lnRef idx="1"><a:schemeClr val="accent1"/></a:lnRef>
        <a:fillRef idx="2"><a:schemeClr val="accent1"/></a:fillRef>
        <a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef>
        <a:fontRef idx="minor"><a:schemeClr val="tx1"/></a:fontRef>
      </p:style>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"##;
    let bytes = build_single_slide_pptx(slide_xml);
    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "stem", media_dir.path()).expect("should parse, not crash");
    let SlideElement::Shape { shape_kind, fill_color, stroke_color, .. } = &pres.slides[0].elements[0] else { panic!() };
    assert_eq!(shape_kind, "hexagon");
    assert_eq!(fill_color, "none", "a theme-only fill must not be guessed at");
    assert!(stroke_color.is_none());
}

/// A `<p:sp>` with `prst="line"` that *also* carries real text -- legal OOXML
/// (nothing stops a line-preset autoshape from having a `<p:txBody>`), and real
/// text must always win over the line/shape distinction, the same priority this
/// importer already used for text vs. non-text shapes before `Line` existed at
/// all. A version of this importer once checked `prst=="line"` unconditionally
/// before ever looking at the shape's text, silently dropping it for exactly this
/// case -- this test exists to keep that from coming back.
#[test]
fn test_pptx_import_prefers_real_text_over_line_preset_geometry() {
    use os_next::core::models::SlideElement;
    let slide_xml = r##"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:sp>
      <p:spPr><a:xfrm><a:off x="100000" y="100000"/><a:ext cx="500000" cy="100000"/></a:xfrm>
        <a:prstGeom prst="line"><a:avLst/></a:prstGeom>
      </p:spPr>
      <p:txBody><a:p><a:r><a:t>Caption on a line shape</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"##;
    let bytes = build_single_slide_pptx(slide_xml);
    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "stem", media_dir.path()).expect("should parse");
    let SlideElement::TextBlock { block, .. } = &pres.slides[0].elements[0] else { panic!("expected a TextBlock, text must not be dropped: {:?}", pres.slides[0].elements[0]) };
    assert_eq!(block.runs[0].text, "Caption on a line shape");
}

/// A table with a horizontally-merged cell (`gridSpan`/`hMerge`, OOXML's equivalent
/// of ODF's `table:number-columns-spanned`/`table:covered-table-cell`) -- the
/// covered continuation cell must not be counted as a real cell, and every real
/// cell must land in its actual column, not just get appended and the row padded
/// at the end. Two rows on purpose: a merge at the row's *trailing* edge (where
/// appending-and-padding would accidentally look correct) and one *not* at the
/// trailing edge (where it wouldn't -- a real cell after the merge would otherwise
/// shift into the wrong column entirely instead of landing after the covered
/// slot). Both must place every cell by its own real column position.
#[test]
fn test_pptx_import_table_with_merged_cells_placed_by_real_column_not_appended() {
    use os_next::core::models::SlideElement;
    let slide_xml = r##"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:graphicFrame>
      <p:xfrm><a:off x="100000" y="100000"/><a:ext cx="3000000" cy="1000000"/></p:xfrm>
      <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
        <a:tbl>
          <a:tblGrid><a:gridCol w="1000000"/><a:gridCol w="1000000"/><a:gridCol w="1000000"/></a:tblGrid>
          <a:tr h="500000">
            <a:tc gridSpan="2"><a:txBody><a:p><a:r><a:t>AB</a:t></a:r></a:p></a:txBody></a:tc>
            <a:tc hMerge="1"><a:txBody><a:p/></a:txBody></a:tc>
            <a:tc><a:txBody><a:p><a:r><a:t>C</a:t></a:r></a:p></a:txBody></a:tc>
          </a:tr>
          <a:tr h="500000">
            <a:tc><a:txBody><a:p><a:r><a:t>A</a:t></a:r></a:p></a:txBody></a:tc>
            <a:tc gridSpan="2"><a:txBody><a:p><a:r><a:t>BC</a:t></a:r></a:p></a:txBody></a:tc>
            <a:tc hMerge="1"><a:txBody><a:p/></a:txBody></a:tc>
          </a:tr>
        </a:tbl>
      </a:graphicData></a:graphic>
    </p:graphicFrame>
  </p:spTree></p:cSld>
</p:sld>"##;
    let bytes = build_single_slide_pptx(slide_xml);
    let media_dir = tempfile::tempdir().unwrap();
    let pres = PptxImporter::import_pptx_bytes(&bytes, "stem", media_dir.path()).expect("should parse");
    let SlideElement::Table { rows, cols, cells, .. } = &pres.slides[0].elements[0] else { panic!() };
    assert_eq!(*rows, 2);
    assert_eq!(*cols, 3);
    // Merge NOT at the trailing edge: a naive "append real cells, pad the end"
    // approach would produce ["AB", "C", ""] (C shifted into the covered slot's
    // column instead of its own) -- this is the case that actually distinguishes
    // correct placement from that bug, unlike the row below.
    assert_eq!(cells[0], vec!["AB".to_string(), String::new(), "C".to_string()], "merge NOT at the trailing edge");
    // Contrast case: merge AT the trailing edge, where append-and-pad happens to
    // produce the same result as correct positional placement -- kept to show why
    // the row above is the one that actually matters, not because this one alone
    // would have caught anything.
    assert_eq!(cells[1], vec!["A".to_string(), "BC".to_string(), String::new()], "merge at the trailing edge");
}

#[test]
fn test_ewsx_export_reimports_through_easyworships_own_files_not_just_our_manifest() {
    use std::io::{Read, Write};
    // An exported .ewsx carries our manifest.json (lossless) *and* EasyWorship's main.db and Schedule.xml.
    // The loader takes the manifest first, so the plain round-trip test never touches the EasyWorship-native
    // paths: strip it (and then main.db) and the import must still recover the content from EW's own files.
    let schedule = Schedule {
        id: "s".into(),
        title: "Sunday".into(),
        items: vec![
            ScheduleItem {
                item_type: "song".into(),
                author_or_ref: "John Newton".into(),
                slides: vec![
                    Slide { text: "Amazing grace how sweet the sound".into(), tag: Some("V1".into()), label: Some("Verse 1".into()), ..Default::default() },
                    Slide { text: "Twas grace that taught my heart".into(), tag: Some("V2".into()), label: Some("Verse 2".into()), ..Default::default() },
                ],
                ..item("song1", "Amazing Grace")
            },
            simple_item("ann", "Announcements", &["Welcome", "Coffee after"]),
        ],
        selected_item_index: None,
        is_modified: false,
        schedule_version: 0,
    };
    let bytes = EwsxManager::save_schedule_to_ewsx_bytes(&schedule).unwrap();
    let names = |b: &[u8]| { let z = zip::ZipArchive::new(std::io::Cursor::new(b)).unwrap(); z.file_names().map(String::from).collect::<Vec<_>>() };
    assert!(names(&bytes).iter().any(|n| n == "main.db"), "an .ewsx must carry EasyWorship's main.db: {:?}", names(&bytes));

    let without = |b: &[u8], drop: &[&str]| -> Vec<u8> {
        let mut src = zip::ZipArchive::new(std::io::Cursor::new(b)).unwrap();
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for i in 0..src.len() {
            let mut f = src.by_index(i).unwrap();
            if drop.contains(&f.name()) { continue; }
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            out.start_file(f.name().to_string(), zip::write::FileOptions::default()).unwrap();
            out.write_all(&data).unwrap();
        }
        out.finish().unwrap().into_inner()
    };
    let texts = |s: &Schedule, title: &str| s.items.iter().find(|i| i.title == title).map(|i| i.slides.iter().map(|sl| sl.text.trim().to_string()).collect::<Vec<_>>());

    // EasyWorship's database alone.
    let from_db = EwsxManager::load_schedule_from_bytes(&without(&bytes, &["manifest.json", "schedule.json"]), "Sunday").expect("main.db import");
    assert_eq!(texts(&from_db, "Amazing Grace"), Some(vec!["Amazing grace how sweet the sound".to_string(), "Twas grace that taught my heart".to_string()]));
    assert_eq!(texts(&from_db, "Announcements").map(|t| t.len()), Some(2));

    // Its Schedule.xml alone (what older EasyWorship versions read).
    let only_xml = without(&bytes, &["manifest.json", "schedule.json", "main.db"]);
    assert_eq!(names(&only_xml), vec!["Schedule.xml".to_string()]);
    let from_xml = EwsxManager::load_schedule_from_bytes(&only_xml, "Sunday").expect("Schedule.xml import");
    assert!(from_xml.items.iter().any(|i| i.title == "Amazing Grace"), "items: {:?}", from_xml.items.iter().map(|i| &i.title).collect::<Vec<_>>());
}
