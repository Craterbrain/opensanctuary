use os_next::core::models::{ScriptureItem, ScriptureVerse};
use std::sync::Arc;
use os_next::core::commands::ShowCommand;
use os_next::core::engine::ShowEngine;
use os_next::core::event_log::EventLog;
use os_next::core::events::ShowEvent;
use os_next::core::models::{ArrangementEntry, Presentation, Schedule, ScheduleItem, Slide, SlideBackground, SlideElement, Song, Theme, ToScheduleItem};
use os_next::media::asset_graph::AssetGraph;
use os_next::media::hardware::{HardwareProber, PrecisionClock};
use os_next::render::compositor::LayerCompositor;
use os_next::storage::db::Database;
use os_next::storage::ewsx::EwsxManager;
use os_next::storage::freeshow_import::parse_scripture_reference;
use os_next::storage::freeshow_show_import::FreeShowShowImporter;
use os_next::storage::openlp_import::OpenLPImporter;
use os_next::storage::pptx_import::PptxImporter;
use os_next::render::pipeline::RenderTargetDimensions;

#[test]
fn test_event_sourced_engine_and_sequence_numbering() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log.clone());

    let snap0 = engine.snapshot();
    assert_eq!(snap0.sequence_number, 0);
    assert!(!snap0.state.is_blackout);

    // 1. Execute ToggleBlackout command
    let envs1 = engine.execute_command(ShowCommand::ToggleBlackout);
    assert_eq!(envs1.len(), 1);
    assert_eq!(envs1[0].sequence_number, 1);
    assert!(matches!(envs1[0].event, ShowEvent::BlackoutToggled { is_blackout: true }));

    let snap1 = engine.snapshot();
    assert_eq!(snap1.sequence_number, 1);
    assert!(snap1.state.is_blackout);

    // 2. Execute ToggleBlackout again
    let envs2 = engine.execute_command(ShowCommand::ToggleBlackout);
    assert_eq!(envs2.len(), 1);
    assert_eq!(envs2[0].sequence_number, 2);
    assert!(!engine.snapshot().state.is_blackout);

    // 3. Test event log catchup
    let catchup = event_log.get_events_since(0);
    assert_eq!(catchup.len(), 2);
    assert_eq!(catchup[0].sequence_number, 1);
    assert_eq!(catchup[1].sequence_number, 2);
}

#[test]
fn test_show_engine_schedule_and_live_navigation() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let song = Song {
        id: "song_123".to_string(),
        title: "Amazing Grace".to_string(),
        author: "John Newton".to_string(), alternate_title: None,
        copyright: None,
        ccli_number: None,
        slides: vec![
            Slide { text: "Verse 1".to_string(), header: None, label: Some("V1".to_string()), background: None, notes: None, tag: None, ..Default::default() },
            Slide { text: "Verse 2".to_string(), header: None, label: Some("V2".to_string()), background: None, notes: None, tag: None, ..Default::default() },
            Slide { text: "Chorus".to_string(), header: None, label: Some("C1".to_string()), background: None, notes: None, tag: None, ..Default::default() },
        ],
        theme_name: None,
    };

    // Add to schedule
    engine.execute_command(ShowCommand::AddToSchedule(song.to_schedule_item()));
    let snap = engine.snapshot();
    assert_eq!(snap.schedule.items.len(), 1);
    assert_eq!(snap.schedule.items[0].title, "Amazing Grace");

    // Go Live on item 0, slide 0
    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(0) });
    let snap_live = engine.snapshot();
    assert!(snap_live.state.live_item.is_some());
    assert_eq!(snap_live.state.live_slide_index, 0);

    // Next Slide
    engine.execute_command(ShowCommand::NextSlide);
    assert_eq!(engine.snapshot().state.live_slide_index, 1);

    // Jump to Section "C" (Chorus)
    engine.execute_command(ShowCommand::JumpSection("C".to_string()));
    assert_eq!(engine.snapshot().state.live_slide_index, 2);

    // Prev Slide
    engine.execute_command(ShowCommand::PrevSlide);
    assert_eq!(engine.snapshot().state.live_slide_index, 1);
}

#[test]
fn test_schedule_reordering_and_slide_manipulation() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let item1 = ScheduleItem {
        id: "item1".to_string(),
        title: "Song 1".to_string(),
        item_type: "song".to_string(),
        author_or_ref: "Author 1".to_string(),
        slides: vec![
            Slide { text: "Slide 1".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() },
            Slide { text: "Slide 2".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() },
        ],
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false, subtitle: None, notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    };

    let item2 = ScheduleItem {
        id: "item2".to_string(),
        title: "Song 2".to_string(),
        item_type: "song".to_string(),
        author_or_ref: "Author 2".to_string(),
        slides: vec![Slide { text: "Only Slide".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() }],
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false, subtitle: None, notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    };

    engine.execute_command(ShowCommand::AddToSchedule(item1));
    engine.execute_command(ShowCommand::AddToSchedule(item2));
    assert_eq!(engine.snapshot().schedule.items.len(), 2);
    assert_eq!(engine.snapshot().schedule.items[0].title, "Song 1");

    // Reorder Schedule (0 -> 1)
    engine.execute_command(ShowCommand::ReorderSchedule { from: 0, to: 1 });
    assert_eq!(engine.snapshot().schedule.items[0].title, "Song 2");
    assert_eq!(engine.snapshot().schedule.items[1].title, "Song 1");

    // Duplicate slide 0 in Item 1 (now at index 1)
    engine.execute_command(ShowCommand::DuplicateItemSlide { item_index: 1, slide_index: 0 });
    assert_eq!(engine.snapshot().schedule.items[1].slides.len(), 3);

    // Remove slide 1 in Item 1
    engine.execute_command(ShowCommand::RemoveItemSlide { item_index: 1, slide_index: 1 });
    assert_eq!(engine.snapshot().schedule.items[1].slides.len(), 2);

    // Reorder child slides within item 1 (Song 1) (from: 0, to: 1)
    // Item 1 slides are currently [Slide 1, Slide 1 (dup), Slide 2] -> after removing slide 1, it has [Slide 1, Slide 2]
    assert_eq!(engine.snapshot().schedule.items[1].slides[0].text, "Slide 1");
    assert_eq!(engine.snapshot().schedule.items[1].slides[1].text, "Slide 2");
    engine.execute_command(ShowCommand::ReorderItemSlides { item_index: Some(1), from: 0, to: 1 });
    assert_eq!(engine.snapshot().schedule.items[1].slides[0].text, "Slide 2");
    assert_eq!(engine.snapshot().schedule.items[1].slides[1].text, "Slide 1");

    // Remove slide from Item 1 until only 1 slide remains
    engine.execute_command(ShowCommand::RemoveItemSlide { item_index: 1, slide_index: 0 });
    assert_eq!(engine.snapshot().schedule.items[1].slides.len(), 1);

    // Removing the last remaining slide of Item 1 should remove Item 1 from the schedule
    assert_eq!(engine.snapshot().schedule.items.len(), 2);
    engine.execute_command(ShowCommand::RemoveItemSlide { item_index: 1, slide_index: 0 });
    assert_eq!(engine.snapshot().schedule.items.len(), 1);
    assert_eq!(engine.snapshot().schedule.items[0].title, "Song 2");
}

#[test]
fn test_database_crud_and_search() {
    let db = Database::in_memory().expect("In-memory SQLite database should create");

    // 1. Insert and Search Song
    let song = Song {
        id: "song_test_1".to_string(),
        title: "10,000 Reasons".to_string(),
        author: "Matt Redman".to_string(), alternate_title: None,
        copyright: Some("2011".to_string()),
        ccli_number: Some("6016351".to_string()),
        slides: vec![Slide { text: "Bless the Lord O my soul".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() }],
        theme_name: Some("Midnight".to_string()),
    };
    db.insert_song(&song).unwrap();

    let fetched = db.get_songs().unwrap();
    assert_eq!(fetched.len(), 1);
    assert_eq!(fetched[0].title, "10,000 Reasons");

    let search_res = db.search_songs("Redman").unwrap();
    assert_eq!(search_res.len(), 1);

    // 2. Insert and Retrieve Theme
    let theme = Theme {
        id: Some("theme_01".to_string()),
        name: "Midnight Glory".to_string(),
        background: "#0a192f".to_string(),
        font_family: "Inter".to_string(),
        font_size: "48px".to_string(),
        font_color: "#ffffff".to_string(),
        alignment: "center".to_string(),
        text_shadow: "2px 2px 4px #000".to_string(),
        line_height: "1.2".to_string(),
        letter_spacing: "0px".to_string(),
        opacity: "1.0".to_string(),
        margin_top: "5%".to_string(),
        margin_bottom: "5%".to_string(),
        margin_left: "5%".to_string(),
        margin_right: "5%".to_string(),
        vertical_align: "center".to_string(),
        ..Default::default()
    };
    db.insert_theme(&theme).unwrap();

    let themes = db.get_themes().unwrap();
    assert_eq!(themes.len(), 1);
    assert_eq!(themes[0].name, "Midnight Glory");

    // 3. Key-Value Settings
    db.set_setting("churchName", "Grace Church").unwrap();
    let settings = db.get_settings().unwrap();
    assert_eq!(settings.get("churchName").unwrap(), "Grace Church");
}

#[test]
fn test_slide_template_crud() {
    use os_next::core::models::{SlideTemplate, SlideElement, ElementTransform, SlideBackground, TextBlock, TextRun, TextParagraphStyle};

    let db = Database::in_memory().expect("In-memory SQLite database should create");

    let template = SlideTemplate {
        id: "tmpl_1".to_string(),
        name: "Lower Third Announcement".to_string(),
        category: Some("Announcements".to_string()),
        elements: vec![SlideElement::TextBlock {
            id: "tb-1".to_string(),
            transform: ElementTransform { x: 0.05, y: 0.7, w: 0.9, h: 0.2, rotation_deg: 0.0, z_index: 1, locked: false, opacity: 1.0 },
            block: TextBlock {
                runs: vec![TextRun { text: "Announcement Title".to_string(), ..Default::default() }],
                paragraph_style: TextParagraphStyle::default(),
                effects: Default::default(),
                autofit: false,
            },
        }],
        background: Some(SlideBackground::Solid("#101820".to_string())),
        thumbnail_data_url: None,
    };

    db.insert_slide_template(&template).unwrap();

    let templates = db.get_slide_templates().unwrap();
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].name, "Lower Third Announcement");
    assert_eq!(templates[0].category.as_deref(), Some("Announcements"));
    assert_eq!(templates[0].elements.len(), 1);
    assert_eq!(templates[0].background, Some(SlideBackground::Solid("#101820".to_string())));

    // Overwrite via INSERT OR REPLACE
    let mut updated = template.clone();
    updated.name = "Lower Third Announcement (v2)".to_string();
    db.insert_slide_template(&updated).unwrap();
    let templates_after_update = db.get_slide_templates().unwrap();
    assert_eq!(templates_after_update.len(), 1);
    assert_eq!(templates_after_update[0].name, "Lower Third Announcement (v2)");

    db.delete_slide_template("tmpl_1").unwrap();
    let templates_after_delete = db.get_slide_templates().unwrap();
    assert_eq!(templates_after_delete.len(), 0);
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
                id: "item_01".to_string(),
                title: "How Great Is Our God".to_string(),
                item_type: "song".to_string(),
                author_or_ref: "Chris Tomlin".to_string(),
                slides: vec![
                    Slide { text: "The splendor of the King".to_string(), header: Some("Verse 1".to_string()), label: Some("V1".to_string()), background: None, notes: None, tag: Some("V1".to_string()), ..Default::default() },
                    Slide { text: "How great is our God".to_string(), header: Some("Chorus".to_string()), label: Some("C1".to_string()), background: None, notes: None, tag: Some("C1".to_string()), ..Default::default() },
                ],
                background: Some("clouds.jpg".to_string()),
                theme_name: None,
                is_expanded: true,
                is_section_header: false, subtitle: None, notes: None,
                arrangement: vec![
                    ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
                    ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
                    ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
                ],
                default_slide_duration_seconds: None,
                slideshow_loop: false,
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
            id: "item_fmt".to_string(),
            title: "Formatted Song".to_string(),
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
            background: None, theme_name: None,
            is_expanded: false,
            is_section_header: false,
            subtitle: None,
            notes: None,
            arrangement: vec![ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None }],
            default_slide_duration_seconds: None,
            slideshow_loop: false,
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
fn test_scripture_reference_parsing() {
    let parsed1 = parse_scripture_reference("John 3:16").unwrap();
    assert_eq!(parsed1.book, "John");
    assert_eq!(parsed1.chapter, Some(3));
    assert_eq!(parsed1.verse_start, Some(16));
    assert_eq!(parsed1.verse_end, Some(16));

    let parsed2 = parse_scripture_reference("2 Corinthians 5:17-21 KJV").unwrap();
    assert_eq!(parsed2.book, "2 Corinthians");
    assert_eq!(parsed2.chapter, Some(5));
    assert_eq!(parsed2.verse_start, Some(17));
    assert_eq!(parsed2.verse_end, Some(21));
    assert_eq!(parsed2.version, Some("KJV".to_string()));

    let parsed_nkjv = parse_scripture_reference("Romans 8:28 NKJV").unwrap();
    assert_eq!(parsed_nkjv.book, "Romans");
    assert_eq!(parsed_nkjv.chapter, Some(8));
    assert_eq!(parsed_nkjv.verse_start, Some(28));
    assert_eq!(parsed_nkjv.verse_end, Some(28));
    assert_eq!(parsed_nkjv.version, Some("NKJV".to_string()));

    let parsed_nrsv = parse_scripture_reference("Philippians 4:13 (NRSV)").unwrap();
    assert_eq!(parsed_nrsv.book, "Philippians");
    assert_eq!(parsed_nrsv.chapter, Some(4));
    assert_eq!(parsed_nrsv.verse_start, Some(13));
    assert_eq!(parsed_nrsv.version, Some("NRSV".to_string()));

    let parsed_rvr = parse_scripture_reference("Genesis 1:1 RVR1960").unwrap();
    assert_eq!(parsed_rvr.book, "Genesis");
    assert_eq!(parsed_rvr.chapter, Some(1));
    assert_eq!(parsed_rvr.verse_start, Some(1));
    assert_eq!(parsed_rvr.version, Some("RVR1960".to_string()));

    let parsed_csb = parse_scripture_reference("Ephesians 2:8-9 CSB17").unwrap();
    assert_eq!(parsed_csb.book, "Ephesians");
    assert_eq!(parsed_csb.chapter, Some(2));
    assert_eq!(parsed_csb.verse_start, Some(8));
    assert_eq!(parsed_csb.verse_end, Some(9));
    assert_eq!(parsed_csb.version, Some("CSB17".to_string()));

    // Test common book misspellings
    let m1 = parse_scripture_reference("Dueteronomy 6:4-5").unwrap();
    assert_eq!(m1.book, "Deuteronomy");
    assert_eq!(m1.chapter, Some(6));
    assert_eq!(m1.verse_start, Some(4));
    assert_eq!(m1.verse_end, Some(5));

    let m2 = parse_scripture_reference("1 Chronicals 16:1-2").unwrap();
    assert_eq!(m2.book, "1 Chronicles");
    assert_eq!(m2.chapter, Some(16));

    let m3 = parse_scripture_reference("Philipians 4:13").unwrap();
    assert_eq!(m3.book, "Philippians");
    assert_eq!(m3.chapter, Some(4));
    assert_eq!(m3.verse_start, Some(13));

    let m4 = parse_scripture_reference("Ezekial 37:1-14").unwrap();
    assert_eq!(m4.book, "Ezekiel");

    let m5 = parse_scripture_reference("Pslam 23:1").unwrap();
    assert_eq!(m5.book, "Psalms");

    let m6 = parse_scripture_reference("Revalation 21:4").unwrap();
    assert_eq!(m6.book, "Revelation");

    let m7 = parse_scripture_reference("Ecclesiasties 3:1").unwrap();
    assert_eq!(m7.book, "Ecclesiastes");

    // Test abbreviation misspellings & variants
    let a1 = parse_scripture_reference("Duet 6:4").unwrap();
    assert_eq!(a1.book, "Deuteronomy");

    let a2 = parse_scripture_reference("Prvb 3:5-6").unwrap();
    assert_eq!(a2.book, "Proverbs");

    let a3 = parse_scripture_reference("Habk 2:2").unwrap();
    assert_eq!(a3.book, "Habakkuk");

    let a4 = parse_scripture_reference("1 Thes 5:16-18").unwrap();
    assert_eq!(a4.book, "1 Thessalonians");

    // Test version typo suffix
    let v_typo = parse_scripture_reference("John 1:1 (NKVJ)").unwrap();
    assert_eq!(v_typo.book, "John");
    assert_eq!(v_typo.version, Some("NKVJ".to_string()));
}

#[test]
fn test_asset_graph_and_hardware_prober() {
    let tmp = tempfile::tempdir().unwrap();
    let graph = AssetGraph::new(tmp.path());

    // Resolve URI
    let path = graph.resolve_uri("asset://videos/worship.mp4");
    assert_eq!(path, tmp.path().join("videos/worship.mp4"));

    // Hardware Prober
    let caps = HardwareProber::probe();
    assert!(!caps.os.is_empty());
    assert!(!caps.supported_decoders.is_empty());

    // Precision Clock
    let clock = PrecisionClock::new();
    assert_eq!(clock.current_time_ms(), 0);
    clock.start();
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert!(clock.current_time_ms() >= 10);
}

#[test]
fn test_layer_compositor_solid_and_stage_generation() {
    let compositor = LayerCompositor::new(RenderTargetDimensions { width: 1280, height: 720 });
    let solid = compositor.render_solid_color(0, 0, 0, 255);
    assert_eq!(solid.width, 1280);
    assert_eq!(solid.height, 720);
    assert_eq!(solid.rgba_data.len(), 1280 * 720 * 4);

    let slide1 = Slide { text: "Current Lyric Line".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() };
    let slide2 = Slide { text: "Next Lyric Line".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() };

    let (curr, next, clock) = compositor.compose_stage_slide(Some(&slide1), Some(&slide2), "10:30 AM");
    assert_eq!(curr, "Current Lyric Line");
    assert_eq!(next, "Next Lyric Line");
    assert_eq!(clock, "10:30 AM");
}

#[test]
fn test_get_themes_against_library_db() {
    let db = Database::new("library.db").unwrap();
    let themes = db.get_themes().unwrap();
    println!("Loaded {} themes from library.db", themes.len());
    assert!(!themes.is_empty());
}

#[test]
fn test_get_by_id_methods() {
    let db = Database::in_memory().expect("In-memory SQLite database should create");

    let song = Song {
        id: "song_by_id_1".to_string(),
        title: "Blessed Be Your Name".to_string(),
        author: "Matt Redman".to_string(),
        alternate_title: None,
        copyright: None,
        ccli_number: None,
        slides: vec![Slide { text: "Every blessing".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() }],
        theme_name: None,
    };
    db.insert_song(&song).unwrap();

    let fetched = db.get_song_by_id("song_by_id_1").unwrap();
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().title, "Blessed Be Your Name");

    let non_existent = db.get_song_by_id("not_real").unwrap();
    assert!(non_existent.is_none());
}

#[test]
fn test_generate_and_load_pristine_sample_ewsx() {
    let schedule = Schedule {
        id: "sample_sunday_service".to_string(),
        title: "Sunday Morning Worship".to_string(),
        items: vec![
            ScheduleItem {
                id: "item_01".to_string(),
                title: "Amazing Grace".to_string(),
                item_type: "song".to_string(),
                author_or_ref: "John Newton".to_string(),
                slides: vec![
                    Slide { text: "Amazing grace, how sweet the sound\nThat saved a wretch like me".to_string(), header: Some("Verse 1".to_string()), label: Some("V1".to_string()), background: None, notes: None, tag: None, ..Default::default() },
                    Slide { text: "I once was lost, but now am found\nWas blind, but now I see".to_string(), header: Some("Verse 2".to_string()), label: Some("V2".to_string()), background: None, notes: None, tag: None, ..Default::default() },
                ],
                background: None, theme_name: None,
                is_expanded: true,
                is_section_header: false,
                subtitle: Some("John Newton".to_string()),
                notes: None,
                arrangement: Vec::new(),
                default_slide_duration_seconds: None,
                slideshow_loop: false,
            },
            ScheduleItem {
                id: "item_02".to_string(),
                title: "Psalm 103:1-5".to_string(),
                item_type: "scripture".to_string(),
                author_or_ref: "Psalm 103:1-5".to_string(),
                slides: vec![
                    Slide { text: "Bless the LORD, O my soul".to_string(), header: Some("Psalm 103:1".to_string()), label: Some("V1".to_string()), background: None, notes: None, tag: None, ..Default::default() },
                ],
                background: None, theme_name: None,
                is_expanded: false,
                is_section_header: false,
                subtitle: None,
                notes: None,
                arrangement: Vec::new(),
                default_slide_duration_seconds: None,
                slideshow_loop: false,
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
fn test_scriptures_batch_insert_and_installed_bibles() {
    let db = Database::in_memory().expect("In-memory SQLite database should create");

    let mut scriptures = Vec::new();
    for chapter in 1..=5 {
        let mut item = ScriptureItem::new("Genesis", chapter, 1, 10, "KJV");
        item.id = format!("scrip_kjv_gen_{}", chapter);
        item.verses = vec![
            ScriptureVerse { verse_number: 1, text: "In the beginning".to_string() },
            ScriptureVerse { verse_number: 2, text: "And the earth was without form".to_string() },
        ];
        scriptures.push(item);
    }

    db.insert_scriptures_batch(&scriptures).expect("Batch insert should succeed");

    let bibles = db.get_installed_bibles().expect("Should fetch installed bibles");
    assert_eq!(bibles.len(), 1);
    assert_eq!(bibles[0].id, "KJV");
    assert_eq!(bibles[0].verse_count, 5);
}

#[tokio::test]
async fn test_fetch_churchapps_catalog_live() {
    let items = os_next::storage::freeshow_import::FreeShowImporter::fetch_churchapps_catalog().await.unwrap();
    println!("Catalog items returned: {}", items.len());
    assert!(!items.is_empty());
}

#[test]
fn test_stage_item_preview_click_preserves_item() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let song = Song {
        id: "song_preview_test".to_string(),
        title: "Blessed Assurance".to_string(),
        author: "Fanny Crosby".to_string(),
        alternate_title: None,
        copyright: None,
        ccli_number: None,
        slides: vec![
            Slide { text: "Verse 1: Blessed assurance, Jesus is mine!".to_string(), header: None, label: Some("V1".to_string()), background: None, notes: None, tag: None, ..Default::default() },
            Slide { text: "Verse 2: Perfect submission, perfect delight!".to_string(), header: None, label: Some("V2".to_string()), background: None, notes: None, tag: None, ..Default::default() },
            Slide { text: "Chorus: This is my story, this is my song!".to_string(), header: None, label: Some("C1".to_string()), background: None, notes: None, tag: None, ..Default::default() },
        ],
        theme_name: None,
    };

    // Add item to schedule
    engine.execute_command(ShowCommand::AddToSchedule(song.to_schedule_item()));
    assert_eq!(engine.snapshot().schedule.items.len(), 1);

    // 1. Initial stage of item 0, slide 0
    engine.execute_command(ShowCommand::StageItem { item_index: Some(0), slide_index: Some(0) });
    let snap1 = engine.snapshot();
    assert!(snap1.state.staged_item.is_some(), "Item should be staged");
    assert_eq!(snap1.state.staged_item.unwrap().title, "Blessed Assurance");
    assert_eq!(snap1.state.staged_slide_index, 0);

    // 2. Click verse 2 in Preview Deck (item_index is None, slide_index is Some(1))
    // Previously, this wiped staged_item to None!
    engine.execute_command(ShowCommand::StageItem { item_index: None, slide_index: Some(1) });
    let snap2 = engine.snapshot();
    assert!(snap2.state.staged_item.is_some(), "Item must NOT disappear when clicking a verse card in preview deck!");
    assert_eq!(snap2.state.staged_item.unwrap().title, "Blessed Assurance");
    assert_eq!(snap2.state.staged_slide_index, 1, "Slide index should advance to 1 (Verse 2)");

    // 3. Click verse 3 (Chorus) in Preview Deck (slide_index 2)
    engine.execute_command(ShowCommand::StageItem { item_index: None, slide_index: Some(2) });
    let snap3 = engine.snapshot();
    assert!(snap3.state.staged_item.is_some(), "Item must remain staged");
    assert_eq!(snap3.state.staged_slide_index, 2, "Slide index should advance to 2 (Chorus)");

    // 4. Reorder slides (move Chorus from 2 to 0)
    engine.execute_command(ShowCommand::ReorderItemSlides { item_index: Some(0), from: 2, to: 0 });
    let snap4 = engine.snapshot();
    // Schedule item must have Chorus first
    assert_eq!(snap4.schedule.items[0].slides[0].label.as_deref(), Some("C1"));
    // Staged item MUST also have Chorus first!
    let staged_slides = &snap4.state.staged_item.expect("staged_item must be Some").slides;
    assert_eq!(staged_slides[0].label.as_deref(), Some("C1"), "Staged item slides must be synchronized after reorder!");
    assert_eq!(staged_slides[1].label.as_deref(), Some("V1"));
    assert_eq!(staged_slides[2].label.as_deref(), Some("V2"));
    // Staged slide index was 2 (Chorus), Chorus is now at 0, so staged_slide_index should follow to 0!
    assert_eq!(snap4.state.staged_slide_index, 0, "Staged slide index should follow moved slide!");

    // 5. Test Reorder with item_index: None (e.g. ad-hoc item staged in preview)
    engine.execute_command(ShowCommand::ReorderItemSlides { item_index: None, from: 0, to: 1 });
    let snap5 = engine.snapshot();
    let s5_slides = &snap5.state.staged_item.expect("staged_item must be Some").slides;
    assert_eq!(s5_slides[0].label.as_deref(), Some("V1"));
    assert_eq!(s5_slides[1].label.as_deref(), Some("C1"));
    assert_eq!(snap5.state.staged_slide_index, 1, "Staged slide index should follow Chorus from 0 to 1");
}

#[test]
fn test_event_log_persistence_resumption_and_idempotence() {
    let tmp_db_file = format!("/tmp/test_event_log_{}.db", uuid::Uuid::new_v4());
    let db = os_next::storage::Database::new(&tmp_db_file).expect("Must open test db");

    // 1. Initial sequence should be 0 on empty db
    let initial_seq = db.get_max_event_sequence().expect("Must read max seq");
    assert_eq!(initial_seq, 0);

    // 2. Start EventLog at sequence 0 and execute a command
    let event_log1 = Arc::new(EventLog::with_initial_sequence(100, initial_seq));
    let engine1 = ShowEngine::new(event_log1.clone());
    let events1 = engine1.execute_command(ShowCommand::ToggleBlackout);
    assert_eq!(events1[0].sequence_number, 1);

    // Save event to db
    db.save_event(&events1[0]).expect("Must save event 1");

    // Execute second command
    let events2 = engine1.execute_command(ShowCommand::ToggleLogo);
    assert_eq!(events2[0].sequence_number, 2);
    db.save_event(&events2[0]).expect("Must save event 2");

    // 3. Test IDEMPOTENCE (saving event 1 or 2 again should NOT fail with UNIQUE constraint)
    db.save_event(&events1[0]).expect("Saving duplicate sequence must succeed via UPSERT");
    db.save_event(&events2[0]).expect("Saving duplicate sequence must succeed via UPSERT");

    // 4. Verify max sequence in db is now 2
    let max_seq = db.get_max_event_sequence().expect("Must read updated max seq");
    assert_eq!(max_seq, 2);

    // 5. Simulate server restart: resume EventLog from max_seq (2)
    let event_log2 = Arc::new(EventLog::with_initial_sequence(100, max_seq));
    let engine2 = ShowEngine::new(event_log2.clone());

    // Next event MUST be sequence #3 (no collision with #1 or #2!)
    let events3 = engine2.execute_command(ShowCommand::ToggleClearText);
    assert_eq!(events3[0].sequence_number, 3);
    db.save_event(&events3[0]).expect("Must save event 3 cleanly");

    // 6. Test restore_from_events
    let all_saved = db.load_events().expect("Must load events from db");
    assert_eq!(all_saved.len(), 3);
    let engine3 = ShowEngine::new(Arc::new(EventLog::new(100)));
    engine3.restore_from_events(all_saved);
    let snap3 = engine3.snapshot();
    assert_eq!(snap3.sequence_number, 3);
    assert!(snap3.state.is_blackout);
    assert!(snap3.state.is_logo);
    assert!(snap3.state.is_clear_text);

    let _ = std::fs::remove_file(&tmp_db_file);
}

#[test]
fn test_backend_plugin_manager_lifecycle_and_interception() {
    use os_next::core::plugins::{OsPlugin, PluginManager};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct TestDmxPlugin {
        state_changes: AtomicUsize,
        block_next: AtomicBool,
    }

    impl OsPlugin for TestDmxPlugin {
        fn name(&self) -> &'static str {
            "DMX Lighting & PTZ Controller"
        }

        fn on_state_change(&self, _state: &os_next::core::models::ShowState) {
            self.state_changes.fetch_add(1, Ordering::SeqCst);
        }

        fn on_command(&self, cmd: &str) -> bool {
            if self.block_next.load(Ordering::SeqCst) && (cmd.contains("NextSlide") || cmd.contains("ToggleClearText")) {
                return false; // Intercept & block command
            }
            true
        }
    }

    let plugin = Arc::new(TestDmxPlugin {
        state_changes: AtomicUsize::new(0),
        block_next: AtomicBool::new(true),
    });

    struct PluginWrapper(Arc<TestDmxPlugin>);
    impl OsPlugin for PluginWrapper {
        fn name(&self) -> &'static str { self.0.name() }
        fn on_state_change(&self, state: &os_next::core::models::ShowState) { self.0.on_state_change(state) }
        fn on_command(&self, cmd: &str) -> bool { self.0.on_command(cmd) }
    }

    let mut pm = PluginManager::new();
    pm.register_plugin(Box::new(PluginWrapper(plugin.clone())));
    assert_eq!(pm.plugin_count(), 1);

    // 1. Safety-critical commands bypass any plugin veto
    assert!(pm.check_command("ToggleBlackout"));
    assert!(pm.check_command("ToggleClearText"), "Safety-critical command must bypass plugin veto!");

    // 2. Normal command interception
    assert!(!pm.check_command("NextSlide"), "Normal command should be vetoed by plugin");

    plugin.block_next.store(false, Ordering::SeqCst);
    assert!(pm.check_command("NextSlide"), "Command allowed after block flag cleared");

    // 3. Panic isolation at FFI boundary
    struct PanickingPlugin;
    impl OsPlugin for PanickingPlugin {
        fn name(&self) -> &'static str { "Panicking Broken Plugin" }
        fn on_state_change(&self, _: &os_next::core::models::ShowState) { panic!("Simulated FFI panic in state change"); }
        fn on_command(&self, _: &str) -> bool { panic!("Simulated FFI panic in command hook"); }
    }
    pm.register_plugin(Box::new(PanickingPlugin));

    // Host must NOT panic when broken plugin panics
    assert!(pm.check_command("NextSlide"), "Panicking plugin must be isolated without aborting host");
    let state = os_next::core::models::ShowState::default();
    pm.notify_state_change(&state); // Must not panic
    assert_eq!(plugin.state_changes.load(Ordering::SeqCst), 1);
}

#[test]
fn test_plugin_manager_collects_resources_across_categories_and_isolates_panics() {
    use os_next::core::plugins::{OsPlugin, PluginManager};

    struct SongLibraryPlugin;
    impl OsPlugin for SongLibraryPlugin {
        fn name(&self) -> &'static str { "External Song Library" }
        fn on_state_change(&self, _state: &os_next::core::models::ShowState) {}
        fn on_command(&self, _cmd: &str) -> bool { true }
        fn provide_resources(&self, category: &str) -> Vec<serde_json::Value> {
            if category == "songs" {
                vec![serde_json::json!({"id": "ext_1", "title": "Amazing Grace (External)"})]
            } else {
                Vec::new()
            }
        }
    }

    struct BrokenResourcePlugin;
    impl OsPlugin for BrokenResourcePlugin {
        fn name(&self) -> &'static str { "Broken Resource Plugin" }
        fn on_state_change(&self, _state: &os_next::core::models::ShowState) {}
        fn on_command(&self, _cmd: &str) -> bool { true }
        fn provide_resources(&self, _category: &str) -> Vec<serde_json::Value> {
            panic!("Simulated failure while enumerating resources");
        }
    }

    let mut pm = PluginManager::new();
    pm.register_plugin(Box::new(SongLibraryPlugin));
    pm.register_plugin(Box::new(BrokenResourcePlugin));

    let songs = pm.collect_resources("songs");
    assert_eq!(songs.len(), 1, "the broken plugin must not suppress the working one's contribution");
    assert_eq!(songs[0]["title"], "Amazing Grace (External)");

    // A category no plugin provides yields nothing, not an error.
    assert!(pm.collect_resources("themes").is_empty());

    // A plugin that doesn't override provide_resources contributes nothing by default,
    // and never panics — proving the trait's default implementation is safe to inherit.
    struct MinimalPlugin;
    impl OsPlugin for MinimalPlugin {
        fn name(&self) -> &'static str { "Minimal Plugin" }
        fn on_state_change(&self, _state: &os_next::core::models::ShowState) {}
        fn on_command(&self, _cmd: &str) -> bool { true }
    }
    let mut pm2 = PluginManager::new();
    pm2.register_plugin(Box::new(MinimalPlugin));
    assert!(pm2.collect_resources("songs").is_empty());
}

#[test]
fn test_engine_undo_and_redo_macro_compensation() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log.clone());

    // 1. Initial State: Empty schedule
    assert_eq!(engine.snapshot().schedule.items.len(), 0);

    // 2. Add Item 1
    let song1 = Song {
        id: "song_1".to_string(),
        title: "Song One".to_string(),
        author: "Author".to_string(),
        alternate_title: None, copyright: None, ccli_number: None, slides: vec![], theme_name: None,
    };
    engine.execute_command(ShowCommand::AddToSchedule(song1.to_schedule_item()));
    assert_eq!(engine.snapshot().schedule.items.len(), 1);

    // 3. Add Item 2
    let song2 = Song {
        id: "song_2".to_string(),
        title: "Song Two".to_string(),
        author: "Author".to_string(),
        alternate_title: None, copyright: None, ccli_number: None, slides: vec![], theme_name: None,
    };
    engine.execute_command(ShowCommand::AddToSchedule(song2.to_schedule_item()));
    assert_eq!(engine.snapshot().schedule.items.len(), 2);

    // 4. Toggle Blackout
    engine.execute_command(ShowCommand::ToggleBlackout);
    assert!(engine.snapshot().state.is_blackout);

    // 5. Undo Blackout: Blackout should be reverted!
    let undo1_events = engine.execute_command(ShowCommand::Undo);
    assert!(!undo1_events.is_empty());
    assert!(!engine.snapshot().state.is_blackout, "Blackout must be false after undo");
    assert_eq!(engine.snapshot().schedule.items.len(), 2, "Schedule items preserved");

    // 6. Redo Blackout: Blackout should be restored!
    let redo1_events = engine.execute_command(ShowCommand::Redo);
    assert!(!redo1_events.is_empty());
    assert!(engine.snapshot().state.is_blackout, "Blackout must be true after redo");

    // 7. Undo twice: (a) Blackout undone, (b) Item 2 undone
    engine.execute_command(ShowCommand::Undo); // undid blackout
    engine.execute_command(ShowCommand::Undo); // undid AddToSchedule(song2)
    assert_eq!(engine.snapshot().schedule.items.len(), 1, "Only Song 1 should remain in schedule");
    assert_eq!(engine.snapshot().schedule.items[0].title, "Song One");

    // 8. Redo: Restores Item 2!
    engine.execute_command(ShowCommand::Redo);
    assert_eq!(engine.snapshot().schedule.items.len(), 2, "Song 2 restored via Redo");
    assert_eq!(engine.snapshot().schedule.items[1].title, "Song Two");

    // 9. Verify event log monotonicity
    let snap = engine.snapshot();
    assert!(snap.sequence_number >= 7);
}

#[test]
fn test_graceful_storage_degradation_on_sqlite_failure() {
    let tmp_dir = std::env::temp_dir();
    let tmp_db_file = tmp_dir.join(format!("test_degradation_{}.db", uuid::Uuid::new_v4()));
    let db = Database::new(tmp_db_file.to_str().unwrap()).expect("Must init temp db");

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    // Set up broadcast channel simulating the server broadcaster (tokio::sync::broadcast::channel(512))
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(16);

    // 1. Initial baseline command succeeds and persists to SQLite cleanly
    let events1 = engine.execute_command(ShowCommand::ToggleBlackout);
    assert_eq!(events1.len(), 1);
    assert_eq!(events1[0].sequence_number, 1);
    for env in &events1 {
        db.save_event(env).expect("First event must save to SQLite cleanly");
    }
    let snap1 = engine.snapshot();
    let snap1_json = serde_json::to_string(&snap1).unwrap();
    tx.send(snap1_json).expect("Broadcast channel sends snapshot #1");
    let received1 = rx.try_recv().expect("Connected display receives snapshot #1");
    assert!(received1.contains("\"sequence_number\":1"));
    assert!(received1.contains("\"is_blackout\":true"));

    // 2. Simulate catastrophic SQLite storage failure (e.g. disk full, read-only filesystem, table dropped)
    db.execute_raw("DROP TABLE event_log").expect("Drop event_log table to simulate fatal SQLite storage I/O failure");

    // 3. Dispatch subsequent command during active storage failure
    // Emulates routes.rs and ws.rs execution pipeline:
    //   let events = app.engine.execute_command(cmd);
    //   for env in &events {
    //       if let Err(e) = app.db.save_event(env) {
    //           tracing::warn!("Failed to persist event log entry #{}: {}", env.sequence_number, e);
    //       }
    //   }
    let events2 = engine.execute_command(ShowCommand::ToggleClearText);
    assert!(!events2.is_empty(), "Engine MUST plan and reduce events in memory regardless of disk health");
    assert_eq!(events2[0].sequence_number, 2, "Monotonic sequence continues in memory without gap");

    let mut storage_error_logged = false;
    for env in &events2 {
        if let Err(e) = db.save_event(env) {
            storage_error_logged = true;
            // Actionable warning logged; error is not allowed to panic or terminate execution
            assert!(e.to_string().contains("no such table") || e.to_string().contains("SQL"));
        }
    }
    assert!(storage_error_logged, "SQLite save_event MUST fail under simulated failure");

    // 4. Invariant: In-memory presentation state continues uninterrupted
    let snap2 = engine.snapshot();
    assert_eq!(snap2.sequence_number, 2, "In-memory sequence counter advances to 2");
    assert!(snap2.state.is_blackout, "Prior blackout state retained in RAM");
    assert!(snap2.state.is_clear_text, "New clear text state applied in RAM");

    // 5. Invariant: Real-time broadcast channel continues distributing snapshots to live screens
    let snap2_json = serde_json::to_string(&snap2).unwrap();
    let broadcast_result = tx.send(snap2_json);
    assert!(broadcast_result.is_ok(), "Broadcast channel MUST continue emitting snapshots to sanctuary displays");

    let received2 = rx.try_recv().expect("Client displays MUST receive live broadcast during storage failure");
    assert!(received2.contains("\"protocol_version\":2"), "Protocol version present in broadcast");
    assert!(received2.contains("\"sequence_number\":2"));
    assert!(received2.contains("\"is_clear_text\":true"));

    // 6. Invariant: Subsequent live presentation commands continue working without halt
    let events3 = engine.execute_command(ShowCommand::ToggleLogo);
    assert_eq!(events3[0].sequence_number, 3);
    let snap3 = engine.snapshot();
    assert!(snap3.state.is_logo);
    assert_eq!(snap3.sequence_number, 3);

    let _ = std::fs::remove_file(&tmp_db_file);
}

#[test]
fn test_media_transport_commands_plan_events_and_sanitize_inputs() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    // 1. MediaScheduledStart: Sanitize negative or infinite PTS
    let ev1 = engine.execute_command(ShowCommand::MediaScheduledStart { target_pts: -10.0, start_at_epoch_ms: 1789000000 });
    assert_eq!(ev1.len(), 1);
    if let ShowEvent::MediaScheduledStart { target_pts, start_at_epoch_ms } = &ev1[0].event {
        assert_eq!(*target_pts, 0.0, "Negative target_pts must clamp to 0.0");
        assert_eq!(*start_at_epoch_ms, 1789000000);
    } else {
        panic!("Expected ShowEvent::MediaScheduledStart");
    }

    let ev1_valid = engine.execute_command(ShowCommand::MediaScheduledStart { target_pts: 42.5, start_at_epoch_ms: 1789001000 });
    if let ShowEvent::MediaScheduledStart { target_pts, .. } = &ev1_valid[0].event {
        assert_eq!(*target_pts, 42.5);
    } else {
        panic!("Expected ShowEvent::MediaScheduledStart");
    }

    // 2. MediaPause: Sanitize negative or non-finite time
    let ev2 = engine.execute_command(ShowCommand::MediaPause { current_time: -5.0 });
    if let ShowEvent::MediaPaused { current_time } = &ev2[0].event {
        assert_eq!(*current_time, 0.0, "Negative pause current_time must clamp to 0.0");
    } else {
        panic!("Expected ShowEvent::MediaPaused");
    }

    // 3. MediaSetLoop
    let ev3 = engine.execute_command(ShowCommand::MediaSetLoop(true));
    if let ShowEvent::MediaLoopSet { is_looping } = &ev3[0].event {
        assert!(*is_looping);
    } else {
        panic!("Expected ShowEvent::MediaLoopSet");
    }

    // 4. MediaSetMute
    let ev4 = engine.execute_command(ShowCommand::MediaSetMute(true));
    if let ShowEvent::MediaMuteSet { is_muted } = &ev4[0].event {
        assert!(*is_muted);
    } else {
        panic!("Expected ShowEvent::MediaMuteSet");
    }

    // 5. MediaSetVolume: Clamping between 0.0 and 1.0
    let ev5_high = engine.execute_command(ShowCommand::MediaSetVolume(2.5));
    if let ShowEvent::MediaVolumeSet { volume } = &ev5_high[0].event {
        assert_eq!(*volume, 1.0, "Volume > 1.0 must clamp to 1.0");
    } else {
        panic!("Expected ShowEvent::MediaVolumeSet");
    }

    let ev5_low = engine.execute_command(ShowCommand::MediaSetVolume(-0.5));
    if let ShowEvent::MediaVolumeSet { volume } = &ev5_low[0].event {
        assert_eq!(*volume, 0.0, "Volume < 0.0 must clamp to 0.0");
    } else {
        panic!("Expected ShowEvent::MediaVolumeSet");
    }

    let ev5_normal = engine.execute_command(ShowCommand::MediaSetVolume(0.75));
    if let ShowEvent::MediaVolumeSet { volume } = &ev5_normal[0].event {
        assert_eq!(*volume, 0.75);
    } else {
        panic!("Expected ShowEvent::MediaVolumeSet");
    }

    // 6. MediaPreroll: Sanitize negative PTS
    let ev6 = engine.execute_command(ShowCommand::MediaPreroll { target_pts: -2.0 });
    if let ShowEvent::MediaPrerolled { target_pts } = &ev6[0].event {
        assert_eq!(*target_pts, 0.0, "Negative preroll PTS must clamp to 0.0");
    } else {
        panic!("Expected ShowEvent::MediaPrerolled");
    }

    // 7. Verify all events advanced the monotonic sequence number
    let snap = engine.snapshot();
    assert_eq!(snap.sequence_number, 9, "9 media commands produced 9 sequence entries");
    // Verify media commands did not corrupt schedule
    assert_eq!(snap.schedule.items.len(), 0);
}

#[test]
fn test_media_playback_state_reflects_transport_transitions() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    // No dedicated media playing yet.
    assert!(engine.snapshot().state.media_playback.is_none());

    // Scheduling a start populates media_playback as playing, at the given target position,
    // with a future start anchor for every client to align to.
    engine.execute_command(ShowCommand::MediaScheduledStart { target_pts: 12.5, start_at_epoch_ms: 1789002000 });
    let playback = engine.snapshot().state.media_playback.expect("media_playback must be populated after MediaScheduledStart");
    assert!(playback.is_playing);
    assert_eq!(playback.current_time, 12.5);
    assert_eq!(playback.start_at_epoch_ms, Some(1789002000));
    let version_after_start = playback.sync_version;

    // Pausing flips is_playing off, records the paused position, and clears the start anchor
    // (there is nothing left to "start at" once paused).
    engine.execute_command(ShowCommand::MediaPause { current_time: 20.0 });
    let playback = engine.snapshot().state.media_playback.expect("media_playback must survive a pause");
    assert!(!playback.is_playing);
    assert_eq!(playback.current_time, 20.0);
    assert_eq!(playback.start_at_epoch_ms, None);
    assert!(playback.sync_version > version_after_start, "sync_version must advance so clients can detect the change");

    // Loop/mute/volume commands update just their field without disturbing the rest.
    engine.execute_command(ShowCommand::MediaSetLoop(true));
    engine.execute_command(ShowCommand::MediaSetMute(true));
    engine.execute_command(ShowCommand::MediaSetVolume(0.4));
    let playback = engine.snapshot().state.media_playback.unwrap();
    assert!(playback.is_looping);
    assert!(playback.is_muted);
    assert_eq!(playback.volume, 0.4);
    assert_eq!(playback.current_time, 20.0, "unrelated fields must not reset on a loop/mute/volume change");

    // Prerolling holds at a position without playing — distinct from a scheduled start.
    engine.execute_command(ShowCommand::MediaPreroll { target_pts: 0.0 });
    let playback = engine.snapshot().state.media_playback.unwrap();
    assert!(!playback.is_playing);
    assert_eq!(playback.current_time, 0.0);

    // A new item going live invalidates any in-flight dedicated media state — it described
    // the item that just left, and would otherwise mislead clients about the new item.
    engine.execute_command(ShowCommand::GoLive { item_index: None, slide_index: None });
    assert!(engine.snapshot().state.media_playback.is_none(), "media_playback must clear when the live item changes");
}

/// `schedule.schedule_version` is what `src/api/ws.rs`'s `broadcast_snapshot` uses to decide
/// whether the (potentially large) `Schedule` needs to be re-sent to every connected client,
/// or whether every client already has the current one. It must increment on events that
/// actually mutate the schedule, and must NOT increment on events that only touch
/// `ShowState` — otherwise every blackout toggle would force a full schedule re-broadcast,
/// defeating the entire point.
#[test]
fn test_schedule_version_increments_only_on_schedule_mutating_events() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    assert_eq!(engine.snapshot().schedule.schedule_version, 0);

    let mut song = Song::new("Amazing Grace", "John Newton");
    song.add_slide("V1", "Verse 1", "Amazing grace how sweet the sound");
    engine.execute_command(ShowCommand::AddToSchedule(song.to_schedule_item()));
    let version_after_add = engine.snapshot().schedule.schedule_version;
    assert_eq!(version_after_add, 1, "adding a schedule item must increment schedule_version");

    // A run of ShowState-only commands must leave schedule_version untouched.
    engine.execute_command(ShowCommand::ToggleBlackout);
    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(0) });
    engine.execute_command(ShowCommand::ToggleLogo);
    engine.execute_command(ShowCommand::MediaScheduledStart { target_pts: 0.0, start_at_epoch_ms: 1789000000 });
    assert_eq!(
        engine.snapshot().schedule.schedule_version,
        version_after_add,
        "blackout/live-navigation/logo/media-transport commands must not bump schedule_version"
    );

    // A second schedule mutation increments it again.
    engine.execute_command(ShowCommand::RemoveFromSchedule(0));
    assert_eq!(engine.snapshot().schedule.schedule_version, version_after_add + 1, "removing a schedule item must increment schedule_version");
}

#[tokio::test]
async fn test_broadcast_channel_buffer_capacity_and_lag_eviction() {
    // Exact capacity configured in main.rs:80
    let (tx, mut fast_rx) = tokio::sync::broadcast::channel::<String>(512);
    let mut slow_rx = tx.subscribe();

    // Broadcast 512 messages
    for i in 0..512 {
        let msg = format!("snapshot_{}", i);
        let send_res = tx.send(msg);
        assert!(send_res.is_ok(), "Broadcast send within capacity must succeed");
    }

    // Fast receiver reads all 512 messages without lag
    for i in 0..512 {
        let msg = fast_rx.recv().await.expect("Fast receiver must read smoothly");
        assert_eq!(msg, format!("snapshot_{}", i));
    }

    // Broadcast 1 additional message (513th), overflowing slow_rx's 512-buffer
    let send_513 = tx.send("snapshot_512".to_string());
    assert!(send_513.is_ok());

    // Slow receiver attempts to read: must encounter RecvError::Lagged(1)
    match slow_rx.recv().await {
        Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
            assert!(dropped >= 1, "Must report at least 1 dropped message on overflow");
        }
        other => panic!("Expected RecvError::Lagged on slow subscriber, got: {:?}", other),
    }

    // After catching up past the lag, slow receiver resumes reading from the oldest retained message
    let oldest_retained = slow_rx.recv().await.expect("Slow receiver must resume reading after lag recovery");
    assert_eq!(oldest_retained, "snapshot_1", "Tokio broadcast drops oldest item (0) and resumes at oldest retained (1)");

    // Draining the remaining 511 messages leads to the latest broadcast message
    let mut last_msg = oldest_retained;
    while let Ok(msg) = slow_rx.try_recv() {
        last_msg = msg;
    }
    assert_eq!(last_msg, "snapshot_512", "Drained slow receiver arrives at latest broadcast frame");
}

#[tokio::test]
async fn test_shorthand_catalog_item_expansion_and_route_handlers() {
    use os_next::core::models::ToScheduleItem;

    let db = os_next::storage::Database::in_memory().unwrap();
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log.clone());

    // 1. Seed Song into DB
    let song = os_next::models::Song {
        id: "song_101".to_string(),
        title: "Be Thou My Vision".to_string(),
        alternate_title: None,
        author: "Eleanor Hull".to_string(),
        copyright: Some("Public Domain".to_string()),
        ccli_number: Some("12345".to_string()),
        slides: vec![os_next::models::Slide {
            text: "Be Thou my vision, O Lord of my heart".to_string(),
            header: None,
            label: Some("V1".to_string()),
            background: None,
            notes: None,
            tag: None,
            ..Default::default()
        }],
        theme_name: None,
    };
    db.insert_song(&song).unwrap();

    // 2. Seed Scripture into DB
    let scripture = os_next::models::ScriptureItem {
        id: "scripture_202".to_string(),
        book: "Psalm".to_string(),
        chapter: 121,
        verse_start: 1,
        verse_end: 2,
        version: "KJV".to_string(),
        reference: "Psalm 121:1-2".to_string(),
        verses: vec![os_next::models::ScriptureVerse {
            verse_number: 1,
            text: "I will lift up mine eyes unto the hills".to_string(),
        }],
        theme_name: None,
    };
    db.insert_scripture(&scripture).unwrap();

    // 3. Seed Media into DB
    let media = os_next::models::MediaItem {
        id: "media_303".to_string(),
        name: "Sunrise Loop".to_string(),
        media_type: "video".to_string(),
        file_path: "/media/videos/sunrise.mp4".to_string(),
        duration_seconds: Some(15.0),
        thumbnail_path: None,
        loop_playback: true,
    };
    db.insert_media(&media).unwrap();

    // 4. Seed Presentation into DB
    let presentation = os_next::models::Presentation {
        id: "pres_404".to_string(),
        title: "Welcome Slides".to_string(),
        author: "Church Admin".to_string(),
        slides: vec![os_next::models::Slide {
            text: "Announcements: Fellowship lunch at noon".to_string(),
            header: Some("Welcome".to_string()),
            label: Some("Slide 1".to_string()),
            background: None,
            notes: None,
            tag: None,
            ..Default::default()
        }],
        theme_name: None,
    };
    db.insert_presentation(&presentation).unwrap();

    // Simulate server-side shorthand resolution: match item_type and item_id
    let types_and_ids = vec![
        ("song", "song_101", "song"),
        ("scripture", "scripture_202", "scripture"),
        ("media", "media_303", "media"),
        ("presentation", "pres_404", "presentation"),
    ];

    for (item_type, item_id, expected_type) in types_and_ids {
        let sched_item = match item_type {
            "song" => db.get_song_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
            "scripture" => db.get_scripture_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
            "media" => db.get_media_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
            "presentation" => db.get_presentation_by_id(item_id).ok().flatten().map(|i| i.to_schedule_item()),
            _ => None,
        };

        assert!(sched_item.is_some(), "Shorthand resolution for {} must succeed", item_type);
        let item = sched_item.unwrap();
        assert_eq!(item.item_type, expected_type);
        engine.execute_command(ShowCommand::AddToSchedule(item));
    }

    // Verify all 4 items were added to active engine schedule
    let snap = engine.snapshot();
    assert_eq!(snap.schedule.items.len(), 4);
    assert_eq!(snap.schedule.items[0].title, "Be Thou My Vision");
    assert_eq!(snap.schedule.items[1].title, "Psalm 121:1-2");
    assert_eq!(snap.schedule.items[2].title, "Sunrise Loop");
    assert_eq!(snap.schedule.items[3].title, "Welcome Slides");

    // Negative case: unknown item_id returns None gracefully
    let invalid = db.get_song_by_id("nonexistent_id").ok().flatten();
    assert!(invalid.is_none(), "Nonexistent ID must return None without panicking");
}

#[test]
fn test_split_database_architecture_and_folder_autocreation() {
    let test_dir = std::env::temp_dir().join(format!("os_next_split_db_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&test_dir).expect("Create temp test dir");

    let db_path = test_dir.join("library.db");
    let bibles_dir = test_dir.join("bibles");
    let songs_dir = test_dir.join("songs");

    assert!(!bibles_dir.exists(), "bibles dir must not exist prior to db init");
    assert!(!songs_dir.exists(), "songs dir must not exist prior to db init");

    // 1. Initialize Database - server should automatically create expected folders
    let db = Database::new(&db_path).expect("Database::new should succeed");
    assert!(bibles_dir.exists(), "Server must auto-create bibles/ directory");
    assert!(songs_dir.exists(), "Server must auto-create songs/ directory");

    // 2. Insert Public Domain Song
    let pd_song = Song {
        id: "song_pd_01".to_string(),
        title: "Amazing Grace".to_string(),
        author: "John Newton (1779)".to_string(),
        alternate_title: None,
        copyright: Some("Public Domain".to_string()),
        ccli_number: None,
        slides: vec![Slide { text: "Amazing grace how sweet the sound".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() }],
        theme_name: None,
    };
    db.insert_song(&pd_song).expect("Insert PD song");

    // 3. Insert Copyrighted Song
    let cr_song = Song {
        id: "song_cr_01".to_string(),
        title: "10,000 Reasons".to_string(),
        author: "Matt Redman".to_string(),
        alternate_title: None,
        copyright: Some("2011 Thankyou Music / Said And Done Music".to_string()),
        ccli_number: Some("6016351".to_string()),
        slides: vec![Slide { text: "Bless the Lord O my soul".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() }],
        theme_name: None,
    };
    db.insert_song(&cr_song).expect("Insert CR song");

    // 4. Verify separate database files exist in songs/
    let pd_db_file = songs_dir.join("public_domain.db");
    let cr_db_file = songs_dir.join("copyrighted.db");
    assert!(pd_db_file.exists(), "songs/public_domain.db must exist on disk");
    assert!(cr_db_file.exists(), "songs/copyrighted.db must exist on disk");

    // Verify physical separation by opening individual SQLite connections directly
    let pd_raw_conn = rusqlite::Connection::open(&pd_db_file).unwrap();
    let pd_count: usize = pd_raw_conn.query_row("SELECT COUNT(*) FROM songs WHERE id = 'song_pd_01'", [], |r| r.get(0)).unwrap();
    let pd_cr_count: usize = pd_raw_conn.query_row("SELECT COUNT(*) FROM songs WHERE id = 'song_cr_01'", [], |r| r.get(0)).unwrap();
    assert_eq!(pd_count, 1, "public_domain.db must contain the public domain song");
    assert_eq!(pd_cr_count, 0, "public_domain.db must NOT contain the copyrighted song");

    let cr_raw_conn = rusqlite::Connection::open(&cr_db_file).unwrap();
    let cr_count: usize = cr_raw_conn.query_row("SELECT COUNT(*) FROM songs WHERE id = 'song_cr_01'", [], |r| r.get(0)).unwrap();
    let cr_pd_count: usize = cr_raw_conn.query_row("SELECT COUNT(*) FROM songs WHERE id = 'song_pd_01'", [], |r| r.get(0)).unwrap();
    assert_eq!(cr_count, 1, "copyrighted.db must contain the copyrighted song");
    assert_eq!(cr_pd_count, 0, "copyrighted.db must NOT contain the public domain song");

    // 5. Verify unified querying across both split databases
    let all_songs = db.get_songs().expect("Fetch all songs");
    assert_eq!(all_songs.len(), 2, "get_songs must merge songs from both databases");

    let fetched_pd = db.get_song_by_id("song_pd_01").unwrap();
    assert!(fetched_pd.is_some());
    assert_eq!(fetched_pd.unwrap().title, "Amazing Grace");

    let fetched_cr = db.get_song_by_id("song_cr_01").unwrap();
    assert!(fetched_cr.is_some());
    assert_eq!(fetched_cr.unwrap().title, "10,000 Reasons");

    // 6. Insert Scriptures for two different Bibles (KJV and ASV)
    let kjv_items = vec![
        ScriptureItem {
            id: "scrip_kjv_gen_1".to_string(),
            book: "Genesis".to_string(),
            chapter: 1,
            verse_start: 1,
            verse_end: 1,
            version: "KJV".to_string(),
            reference: "Genesis 1:1 (KJV)".to_string(),
            verses: vec![ScriptureVerse { verse_number: 1, text: "In the beginning God created the heaven and the earth.".to_string() }],
            theme_name: None,
        },
    ];
    let asv_items = vec![
        ScriptureItem {
            id: "scrip_asv_gen_1".to_string(),
            book: "Genesis".to_string(),
            chapter: 1,
            verse_start: 1,
            verse_end: 1,
            version: "ASV".to_string(),
            reference: "Genesis 1:1 (ASV)".to_string(),
            verses: vec![ScriptureVerse { verse_number: 1, text: "In the beginning God created the heavens and the earth.".to_string() }],
            theme_name: None,
        },
    ];

    db.insert_scriptures_batch(&kjv_items).expect("Insert KJV batch");
    db.insert_scriptures_batch(&asv_items).expect("Insert ASV batch");

    // 7. Verify per-Bible database files exist in bibles/
    let kjv_db_file = bibles_dir.join("KJV.db");
    let asv_db_file = bibles_dir.join("ASV.db");
    assert!(kjv_db_file.exists(), "bibles/KJV.db must exist on disk");
    assert!(asv_db_file.exists(), "bibles/ASV.db must exist on disk");

    // 8. Verify installed Bibles discovery
    let bibles = db.get_installed_bibles().expect("Get installed bibles");
    assert_eq!(bibles.len(), 2, "Must discover both KJV and ASV installed databases");
    let bible_ids: Vec<String> = bibles.iter().map(|b| b.id.clone()).collect();
    assert!(bible_ids.contains(&"KJV".to_string()));
    assert!(bible_ids.contains(&"ASV".to_string()));

    // 9. Verify unified scripture querying across per-Bible databases
    let all_scrip = db.get_scriptures().expect("Get scriptures");
    assert_eq!(all_scrip.len(), 2, "Must query across all Bible databases");

    let searched = db.search_scriptures("Genesis").expect("Search scriptures");
    assert_eq!(searched.len(), 2, "Search must return matches from all installed Bibles");

    let scrip_by_id = db.get_scripture_by_id("scrip_kjv_gen_1").expect("Get scripture by id");
    assert!(scrip_by_id.is_some());
    assert_eq!(scrip_by_id.unwrap().version, "KJV");

    // Cleanup
    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_set_arrangement_roundtrip() {
    use os_next::engine::{effective_arrangement, resolve_slide_at};
    use os_next::models::ArrangementEntry;

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let mut song = Song::new("Amazing Grace", "John Newton");
    song.add_slide("V1", "Verse 1", "Amazing grace how sweet the sound");
    song.add_slide("V2", "Verse 2", "'Twas grace that taught my heart to fear");
    song.add_slide("C1", "Chorus", "My chains are gone, I've been set free");

    engine.execute_command(ShowCommand::AddToSchedule(song.to_schedule_item()));
    let snap = engine.snapshot();
    assert_eq!(snap.schedule.items.len(), 1);

    // Initial item has identity arrangement, effective_arrangement maps 1:1
    let item = &snap.schedule.items[0];
    assert_eq!(item.arrangement.len(), 3);
    let eff = effective_arrangement(item);
    assert_eq!(eff.len(), 3);
    assert_eq!(eff[0].section_id, "V1");
    assert_eq!(eff[1].section_id, "V2");
    assert_eq!(eff[2].section_id, "C1");

    // Set custom arrangement: V1 -> C1 -> V2 -> C1
    let new_arr = vec![
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 2, background_override: None },
        ArrangementEntry { section_id: "V2".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 2, background_override: None },
    ];

    engine.execute_command(ShowCommand::SetArrangement {
        item_index: Some(0),
        arrangement: new_arr.clone(),
    });

    let snap_after = engine.snapshot();
    let updated_item = &snap_after.schedule.items[0];
    assert_eq!(updated_item.arrangement.len(), 4);
    let eff_after = effective_arrangement(updated_item);
    assert_eq!(eff_after.len(), 4);
    assert_eq!(eff_after[0].section_id, "V1");
    assert_eq!(eff_after[1].section_id, "C1");
    assert_eq!(eff_after[2].section_id, "V2");
    assert_eq!(eff_after[3].section_id, "C1");

    // Verify resolve_slide_at maps play order to master slide content
    let slide_play_1 = resolve_slide_at(updated_item, 1).unwrap();
    let slide_play_3 = resolve_slide_at(updated_item, 3).unwrap();
    assert_eq!(slide_play_1.text, "My chains are gone, I've been set free");
    assert_eq!(slide_play_3.text, "My chains are gone, I've been set free");
    // Master slides are untouched (only 3 unique slides)
    assert_eq!(updated_item.slides.len(), 3);
}

/// Exercises the exact resolution path `GET /api/display-state` uses
/// (`effective_arrangement`/`resolve_slide_at`/`resolve_background_at` against
/// `state.live_item`/`state.live_slide_index`) so a native polling client (Roku) sees
/// current/next slide text and background in arrangement play order, not raw
/// `item.slides` array order.
#[test]
fn test_display_state_resolution_follows_arrangement_not_raw_slide_order() {
    use os_next::engine::{effective_arrangement, resolve_background_at, resolve_slide_at};
    use os_next::models::ArrangementEntry;

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let mut song = Song::new("Amazing Grace", "John Newton");
    song.add_slide("V1", "Verse 1", "Amazing grace how sweet the sound");
    song.add_slide("V2", "Verse 2", "'Twas grace that taught my heart to fear");
    song.add_slide("C1", "Chorus", "My chains are gone, I've been set free");
    engine.execute_command(ShowCommand::AddToSchedule(song.to_schedule_item()));

    // Reorder play order to V1 -> C1 -> V2 -> C1, so play position 1 (raw slide index 2)
    // comes immediately before play position 2 (raw slide index 1) — the opposite of
    // their order in `item.slides`. If a client-facing endpoint read `item.slides`
    // directly instead of going through the arrangement, this would catch it.
    let new_arr = vec![
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 2, background_override: None },
        ArrangementEntry { section_id: "V2".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 2, background_override: None },
    ];
    engine.execute_command(ShowCommand::SetArrangement { item_index: Some(0), arrangement: new_arr });

    // Give the chorus's first play-position occurrence a per-play background override —
    // display-state should surface this over the master slide's own background.
    engine.execute_command(ShowCommand::SetSlideBackground {
        item_index: Some(0),
        slide_index: 1,
        background: "url('/media/images/cross.jpg')".to_string(),
    });

    // Go live at play position 1 (the first Chorus occurrence).
    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(1) });

    let snap = engine.snapshot();
    let live_item = snap.state.live_item.as_ref().expect("item must be live");
    let play_pos = snap.state.live_slide_index;
    assert_eq!(play_pos, 1);

    let slide_count = effective_arrangement(live_item).len();
    assert_eq!(slide_count, 4, "slide_count must reflect play-order length, not raw slide count");

    let current = resolve_slide_at(live_item, play_pos).expect("current slide must resolve");
    assert_eq!(current.text, "My chains are gone, I've been set free", "current slide must be the Chorus, not raw index 1 (Verse 2)");

    let next = resolve_slide_at(live_item, play_pos + 1).expect("next slide must resolve");
    assert_eq!(next.text, "'Twas grace that taught my heart to fear", "next slide must be Verse 2, per arrangement order");

    let background = resolve_background_at(live_item, play_pos);
    assert_eq!(background, Some("url('/media/images/cross.jpg')"), "per-play background override must take precedence over the master slide's own background");
}

#[test]
fn test_set_arrangement_participates_in_undo() {
    use os_next::models::ArrangementEntry;

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let mut song = Song::new("Cornerstone", "Hillsong");
    song.add_slide("V1", "Verse 1", "My hope is built on nothing less");
    song.add_slide("C1", "Chorus", "Christ alone, cornerstone");

    engine.execute_command(ShowCommand::AddToSchedule(song.to_schedule_item()));
    assert_eq!(engine.snapshot().schedule.items[0].arrangement.len(), 2);

    let arr = vec![
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
    ];

    engine.execute_command(ShowCommand::SetArrangement {
        item_index: Some(0),
        arrangement: arr,
    });
    assert_eq!(engine.snapshot().schedule.items[0].arrangement.len(), 3);

    // Undo restores previous arrangement
    engine.execute_command(ShowCommand::Undo);
    assert_eq!(engine.snapshot().schedule.items[0].arrangement.len(), 2);

    // Redo reapplies the arrangement
    engine.execute_command(ShowCommand::Redo);
    assert_eq!(engine.snapshot().schedule.items[0].arrangement.len(), 3);
}

#[test]
fn test_jump_section_within_arrangement() {
    use os_next::models::ArrangementEntry;

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let mut song = Song::new("Way Maker", "Sinach");
    song.add_slide("V1", "Verse 1", "You are here moving in our midst");
    song.add_slide("C1", "Chorus", "Way maker, miracle worker");
    song.add_slide("V2", "Verse 2", "You are here touching every heart");
    song.add_slide("B1", "Bridge", "Even when I don't see it, You're working");

    engine.execute_command(ShowCommand::AddToSchedule(song.to_schedule_item()));

    // Custom play order: V1 (0) -> C1 (1) -> V2 (2) -> C1 (3) -> B1 (4) -> C1 (5)
    let arr = vec![
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "V2".to_string(), source_slide_index: 2, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "B1".to_string(), source_slide_index: 3, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
    ];
    engine.execute_command(ShowCommand::SetArrangement {
        item_index: Some(0),
        arrangement: arr,
    });

    // Go live at position 0 (V1)
    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(0) });
    assert_eq!(engine.snapshot().state.live_slide_index, 0);

    // JumpSection("C") jumps to the first Chorus at play position 1
    engine.execute_command(ShowCommand::JumpSection("C".to_string()));
    assert_eq!(engine.snapshot().state.live_slide_index, 1);

    // JumpSection("C") again cycles forward to the NEXT Chorus occurrence
    // (play position 3), not back to the same one — repeated presses of a
    // letter step through every occurrence of that section in sequence.
    engine.execute_command(ShowCommand::JumpSection("C".to_string()));
    assert_eq!(engine.snapshot().state.live_slide_index, 3);

    // ...and a third press cycles to the last Chorus occurrence (position 5)
    engine.execute_command(ShowCommand::JumpSection("C".to_string()));
    assert_eq!(engine.snapshot().state.live_slide_index, 5);

    // A fourth press wraps back around to the first Chorus occurrence
    engine.execute_command(ShowCommand::JumpSection("C".to_string()));
    assert_eq!(engine.snapshot().state.live_slide_index, 1);

    // JumpSection("B") jumps to Bridge at play position 4
    engine.execute_command(ShowCommand::JumpSection("B".to_string()));
    assert_eq!(engine.snapshot().state.live_slide_index, 4);

    // JumpSection("V") resumes the Verse cycle from where GoLive left it
    // (Verse 1, position 0) and advances to the NEXT verse (Verse 2, play
    // position 2) — jumping away to Chorus/Bridge and back doesn't reset
    // the Verse cursor to the first verse.
    engine.execute_command(ShowCommand::JumpSection("V".to_string()));
    assert_eq!(engine.snapshot().state.live_slide_index, 2);

    // Pressing "V" again wraps back around to Verse 1
    engine.execute_command(ShowCommand::JumpSection("V".to_string()));
    assert_eq!(engine.snapshot().state.live_slide_index, 0);
}

#[test]
fn test_next_slide_walks_arrangement_not_master_slides() {
    use os_next::models::ArrangementEntry;

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let mut song = Song::new("Goodness of God", "Bethel");
    song.add_slide("V1", "Verse 1", "I love You Lord");
    song.add_slide("C1", "Chorus", "All my life You have been faithful");

    engine.execute_command(ShowCommand::AddToSchedule(song.to_schedule_item()));

    // Arrangement has 3 entries over 2 master slides: V1 -> C1 -> C1
    let arr = vec![
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
    ];
    engine.execute_command(ShowCommand::SetArrangement {
        item_index: Some(0),
        arrangement: arr,
    });

    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(0) });
    assert_eq!(engine.snapshot().state.live_slide_index, 0);

    engine.execute_command(ShowCommand::NextSlide);
    assert_eq!(engine.snapshot().state.live_slide_index, 1);

    // Walks to second chorus play position (2), beyond master slides len (2)
    engine.execute_command(ShowCommand::NextSlide);
    assert_eq!(engine.snapshot().state.live_slide_index, 2);
}

#[test]
fn test_background_override_applies_only_to_one_play() {
    use os_next::models::ArrangementEntry;

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let mut song = Song::new("Build My Life", "Housefires");
    song.add_slide("V1", "Verse 1", "Worthy of every song");
    song.add_slide("C1", "Chorus", "Holy there is no one like You");

    engine.execute_command(ShowCommand::AddToSchedule(song.to_schedule_item()));

    let arr = vec![
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
    ];
    engine.execute_command(ShowCommand::SetArrangement {
        item_index: Some(0),
        arrangement: arr,
    });

    // Set background override on play position 2 (second chorus)
    engine.execute_command(ShowCommand::SetSlideBackground {
        item_index: Some(0),
        slide_index: 2,
        background: "golden_sunset.mp4".to_string(),
    });

    let item = &engine.snapshot().schedule.items[0];
    let arr = &item.arrangement;
    assert_eq!(arr[0].background_override, None);
    assert_eq!(arr[1].background_override, None);
    assert_eq!(arr[2].background_override, Some("golden_sunset.mp4".to_string()));

    // Master slide background is unmutated
    assert_eq!(item.slides[1].background, None);
}

#[test]
fn test_arrangement_shrink_clamps_live_index() {
    use os_next::models::ArrangementEntry;

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let mut song = Song::new("King of Kings", "Brooke Ligertwood");
    song.add_slide("V1", "Verse 1", "In the darkness we were waiting");
    song.add_slide("C1", "Chorus", "Praise the Father, praise the Son");

    engine.execute_command(ShowCommand::AddToSchedule(song.to_schedule_item()));

    let initial_arr = vec![
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
    ];
    engine.execute_command(ShowCommand::SetArrangement {
        item_index: Some(0),
        arrangement: initial_arr,
    });

    // Go live on play position 3
    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(3) });
    assert_eq!(engine.snapshot().state.live_slide_index, 3);

    // Shrink arrangement to 2 entries
    let shortened_arr = vec![
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
    ];
    engine.execute_command(ShowCommand::SetArrangement {
        item_index: Some(0),
        arrangement: shortened_arr,
    });

    // live_slide_index must be clamped to new_len - 1 = 1
    assert_eq!(engine.snapshot().state.live_slide_index, 1);
}

#[test]
fn test_schedule_item_with_arrangement_serializes_and_reloads() {
    use os_next::models::{ArrangementEntry, ScheduleItem, Slide};

    let item = ScheduleItem {
        id: "sched_1".to_string(),
        title: "Test Arrangement Song".to_string(),
        subtitle: None,
        notes: None,
        item_type: "song".to_string(),
        author_or_ref: "Author".to_string(),
        slides: vec![
            Slide { text: "Slide 1".to_string(), header: None, label: Some("V1".to_string()), background: None, notes: None, tag: Some("V1".to_string()), ..Default::default() },
            Slide { text: "Slide 2".to_string(), header: None, label: Some("C1".to_string()), background: None, notes: None, tag: Some("C1".to_string()), ..Default::default() },
        ],
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false,
        arrangement: vec![
            ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
            ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: Some("custom_bg.jpg".to_string()) },
            ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ],
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    };

    let serialized = serde_json::to_string(&item).expect("Serialization failed");
    let deserialized: ScheduleItem = serde_json::from_str(&serialized).expect("Deserialization failed");
    assert_eq!(item, deserialized);
    assert_eq!(deserialized.arrangement.len(), 3);
    assert_eq!(deserialized.arrangement[1].background_override, Some("custom_bg.jpg".to_string()));

    // Verify backward compatibility: JSON missing arrangement defaults to empty Vec
    let legacy_json = r#"{
        "id": "legacy_1",
        "title": "Legacy Item",
        "item_type": "song",
        "author_or_ref": "Legacy Author",
        "slides": []
    }"#;
    let legacy_item: ScheduleItem = serde_json::from_str(legacy_json).expect("Legacy deserialization failed");
    assert!(legacy_item.arrangement.is_empty());
}

#[test]
fn test_reconcile_arrangement_preserves_order_on_verse_add() {
    use os_next::engine::reconcile_arrangement;
    use os_next::models::{ArrangementEntry, Song, ToScheduleItem};

    let mut song = Song::new("Amazing Grace", "John Newton");
    song.add_slide("V1", "Verse 1", "Amazing grace how sweet the sound");
    song.add_slide("C1", "Chorus", "My chains are gone, I've been set free");

    let mut item = song.to_schedule_item();
    // Custom arrangement with repeated verse and chorus: V1 -> C1 -> V1
    item.arrangement = vec![
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
    ];

    // Mutate item.slides: add a new verse V2
    let mut v2_song = Song::new("Amazing Grace", "John Newton");
    v2_song.add_slide("V2", "Verse 2", "'Twas grace that taught my heart to fear");
    item.slides.push(v2_song.slides[0].clone());

    // Reconcile
    reconcile_arrangement(&mut item);

    // Existing entries retain their relative order (V1 -> C1 -> V1)
    assert_eq!(item.arrangement.len(), 4);
    assert_eq!(item.arrangement[0].section_id, "V1");
    assert_eq!(item.arrangement[0].source_slide_index, 0);
    assert_eq!(item.arrangement[1].section_id, "C1");
    assert_eq!(item.arrangement[1].source_slide_index, 1);
    assert_eq!(item.arrangement[2].section_id, "V1");
    assert_eq!(item.arrangement[2].source_slide_index, 0);

    // New master slide V2 is appended to the end
    assert_eq!(item.arrangement[3].section_id, "V2");
    assert_eq!(item.arrangement[3].source_slide_index, 2);
}

#[test]
fn test_reconcile_arrangement_drops_deleted_verse() {
    use os_next::engine::reconcile_arrangement;
    use os_next::models::{ArrangementEntry, Song, ToScheduleItem};

    let mut song = Song::new("Way Maker", "Sinach");
    song.add_slide("V1", "Verse 1", "You are here moving in our midst");
    song.add_slide("C1", "Chorus", "Way maker, miracle worker");
    song.add_slide("V2", "Verse 2", "You are here touching every heart");

    let mut item = song.to_schedule_item();
    // Custom arrangement: V1 -> C1 -> V2 -> C1
    item.arrangement = vec![
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "V2".to_string(), source_slide_index: 2, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
    ];

    // Delete Chorus (C1) at index 1 from master slides
    item.slides.remove(1);
    // master slides are now [V1 (index 0), V2 (index 1)]

    // Reconcile
    reconcile_arrangement(&mut item);

    // Both C1 entries must be dropped; V1 and V2 remain in original relative order
    assert_eq!(item.arrangement.len(), 2);
    assert_eq!(item.arrangement[0].section_id, "V1");
    assert_eq!(item.arrangement[0].source_slide_index, 0);
    assert_eq!(item.arrangement[1].section_id, "V2");
    assert_eq!(item.arrangement[1].source_slide_index, 1); // remapped from 2 to 1!
}

#[test]
fn test_reconcile_arrangement_remaps_indices_on_insert() {
    use os_next::engine::reconcile_arrangement;
    use os_next::models::{ArrangementEntry, Slide, Song, ToScheduleItem};

    let mut song = Song::new("Cornerstone", "Hillsong");
    song.add_slide("V1", "Verse 1", "My hope is built on nothing less");
    song.add_slide("C1", "Chorus", "Christ alone, cornerstone");

    let mut item = song.to_schedule_item();
    item.arrangement = vec![
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: None },
        ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
    ];

    // Insert an Intro ("I1") at index 0 of master slides
    let intro_slide = Slide {
        text: "[Intro Instrumental]".to_string(),
        header: Some("Intro".to_string()),
        label: Some("I1".to_string()),
        background: None,
        notes: None,
        tag: Some("I1".to_string()),
        ..Default::default()
    };
    item.slides.insert(0, intro_slide);
    // master slides: [0: I1, 1: V1, 2: C1]

    // Reconcile
    reconcile_arrangement(&mut item);

    // Existing entries retain order; source_slide_index remapped; new master appended at end
    assert_eq!(item.arrangement.len(), 4);
    assert_eq!(item.arrangement[0].section_id, "V1");
    assert_eq!(item.arrangement[0].source_slide_index, 1); // remapped 0 -> 1
    assert_eq!(item.arrangement[1].section_id, "C1");
    assert_eq!(item.arrangement[1].source_slide_index, 2); // remapped 1 -> 2
    assert_eq!(item.arrangement[2].section_id, "V1");
    assert_eq!(item.arrangement[2].source_slide_index, 1); // remapped 0 -> 1
    assert_eq!(item.arrangement[3].section_id, "I1");
    assert_eq!(item.arrangement[3].source_slide_index, 0); // new slide appended with source 0
}

#[test]
fn test_bible_passage_query_and_routes() {
    let test_dir = std::env::temp_dir().join(format!("test_bible_passage_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&test_dir).unwrap();

    let db_path = test_dir.join("library.db");
    let bibles_dir = test_dir.join("bibles");
    let db = Database::new_with_dirs(&db_path, &bibles_dir, test_dir.join("songs")).unwrap();

    // 1. Seed two chapters into a test KJV database
    let mut kjv_items = Vec::new();
    let mut gen1 = ScriptureItem::new("Genesis", 1, 1, 2, "KJV");
    gen1.verses = vec![
        ScriptureVerse { verse_number: 1, text: "In the beginning God created the heaven and the earth.".to_string() },
        ScriptureVerse { verse_number: 2, text: "And the earth was without form, and void.".to_string() },
    ];
    kjv_items.push(gen1);

    let mut john3 = ScriptureItem::new("John", 3, 1, 3, "KJV");
    john3.verses = vec![
        ScriptureVerse { verse_number: 1, text: "There was a man of the Pharisees, named Nicodemus.".to_string() },
        ScriptureVerse { verse_number: 2, text: "The same came to Jesus by night.".to_string() },
        ScriptureVerse { verse_number: 3, text: "Jesus answered and said unto him...".to_string() },
    ];
    kjv_items.push(john3);

    db.insert_scriptures_batch(&kjv_items).expect("Batch insert KJV");

    // 2. Query passage with exact case
    let passage = db.get_passage("KJV", "John", 3).expect("Query John 3").expect("Should find passage");
    assert_eq!(passage.book, "John");
    assert_eq!(passage.chapter, 3);
    assert_eq!(passage.verses.len(), 3);
    assert_eq!(passage.verses[0].verse_number, 1);
    assert_eq!(passage.verses[0].text, "There was a man of the Pharisees, named Nicodemus.");

    // 3. Query passage with case-insensitive book and version
    let passage_ci = db.get_passage("kjv", "john", 3).expect("Query john 3 lowercase").expect("Should find passage");
    assert_eq!(passage_ci.book, "John");
    assert_eq!(passage_ci.chapter, 3);
    assert_eq!(passage_ci.verses.len(), 3);

    // 4. Query non-existent book or chapter returns None
    let not_found = db.get_passage("KJV", "Revelation", 22).expect("Query missing chapter");
    assert!(not_found.is_none());

    // Cleanup
    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_bible_delete_installed_translation() {
    let test_dir = std::env::temp_dir().join(format!("test_bible_delete_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&test_dir).unwrap();

    let db_path = test_dir.join("library.db");
    let bibles_dir = test_dir.join("bibles");
    let db = Database::new_with_dirs(&db_path, &bibles_dir, test_dir.join("songs")).unwrap();

    // 1. Seed a translation
    let mut item = ScriptureItem::new("Romans", 8, 28, 28, "ESV");
    item.verses = vec![ScriptureVerse { verse_number: 28, text: "And we know that in all things God works for the good...".to_string() }];
    db.insert_scriptures_batch(&[item]).expect("Insert ESV");

    let installed_before = db.get_installed_bibles().expect("Get installed bibles");
    assert!(installed_before.iter().any(|b| b.id == "ESV"));

    // Verify file exists on disk
    let esv_db_path = bibles_dir.join("ESV.db");
    assert!(esv_db_path.exists());

    // 2. Delete installed translation
    let deleted = db.delete_installed_bible("ESV").expect("Delete ESV");
    assert!(deleted);

    // Verify file deleted from disk and no longer in installed bibles
    assert!(!esv_db_path.exists());
    let installed_after = db.get_installed_bibles().expect("Get installed bibles after delete");
    assert!(!installed_after.iter().any(|b| b.id == "ESV"));

    // Cleanup
    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_parallel_display_slide_generation() {
    // Tests parallel dual-column scripture slide formatting
    let p_label = "KJV";
    let s_label = "ASV";

    let p_verse = "In the beginning God created the heaven and the earth.";
    let s_verse = "In the beginning God created the heavens and the earth.";

    let slide_content = format!("[{}]\n1. {}\n|||\n[{}]\n1. {}", p_label, p_verse, s_label, s_verse);
    assert!(slide_content.contains("|||"));

    let parts: Vec<&str> = slide_content.split("|||").map(|s| s.trim()).collect();
    assert_eq!(parts.len(), 2);
    assert!(parts[0].starts_with("[KJV]"));
    assert!(parts[1].starts_with("[ASV]"));

    // Create a presentation item representing the dual translation
    let slide = Slide {
        text: slide_content.clone(),
        header: Some("Genesis 1:1".to_string()),
        label: Some("1".to_string()),
        background: None,
        notes: None,
        tag: Some("V1".to_string()),
        ..Default::default()
    };

    let pres = Presentation {
        id: "pres_dual_gen_1".to_string(),
        title: "Genesis 1:1 [KJV | ASV]".to_string(),
        author: "Dual: KJV + ASV".to_string(),
        slides: vec![slide],
        theme_name: None,
    };

    let sched_item = pres.to_schedule_item();
    assert_eq!(sched_item.item_type, "presentation");
    assert_eq!(sched_item.slides.len(), 1);
    assert_eq!(sched_item.slides[0].text, slide_content);
    assert_eq!(sched_item.arrangement.len(), 1);
    assert_eq!(sched_item.arrangement[0].section_id, "V1");
}

#[test]
fn test_default_bible_setting_persistence_and_metadata_migration() {
    let test_dir = std::env::temp_dir().join(format!("test_bible_default_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&test_dir).unwrap();

    let db_path = test_dir.join("library.db");
    let bibles_dir = test_dir.join("bibles");
    let db = Database::new_with_dirs(&db_path, &bibles_dir, test_dir.join("songs")).unwrap();

    // 1. Seed two Bible translations: KJV and ESV
    let mut kjv = ScriptureItem::new("Genesis", 1, 1, 1, "KJV");
    kjv.verses = vec![ScriptureVerse { verse_number: 1, text: "In the beginning...".to_string() }];
    let mut esv = ScriptureItem::new("Genesis", 1, 1, 1, "ESV");
    esv.verses = vec![ScriptureVerse { verse_number: 1, text: "In the beginning...".to_string() }];
    db.insert_scriptures_batch(&[kjv, esv]).expect("Insert bibles");

    // 2. Simulate legacy app_metadata table existing with older setting
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS app_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO app_metadata (key, value) VALUES ('defaultBibleVersion', 'KJV')",
            [],
        ).unwrap();
    }

    // 3. User sets new default bible via settings API: ESV
    db.set_setting("defaultBibleVersion", "ESV").expect("Save defaultBibleVersion");

    // 4. Verify get_settings() returns ESV (canonical settings overrides legacy app_metadata)
    let settings = db.get_settings().expect("Get settings");
    assert_eq!(settings.get("defaultBibleVersion").map(|s| s.as_str()), Some("ESV"));

    // 5. Verify get_installed_bibles() marks ESV with is_default: true, and KJV with is_default: false
    let bibles = db.get_installed_bibles().expect("Get installed bibles");
    let esv_info = bibles.iter().find(|b| b.id == "ESV").expect("ESV found");
    assert!(esv_info.is_default, "ESV should be marked default");
    let kjv_info = bibles.iter().find(|b| b.id == "KJV").expect("KJV found");
    assert!(!kjv_info.is_default, "KJV should not be marked default");

    // 6. Case-insensitive matching: set lowercase 'kjv'
    db.set_setting("defaultBibleVersion", "kjv").expect("Save lowercase kjv");
    let bibles_case = db.get_installed_bibles().expect("Get installed bibles after update");
    let kjv_case = bibles_case.iter().find(|b| b.id == "KJV").expect("KJV found");
    assert!(kjv_case.is_default, "KJV should be marked default even when set with lowercase 'kjv'");
    let esv_case = bibles_case.iter().find(|b| b.id == "ESV").expect("ESV found");
    assert!(!esv_case.is_default, "ESV should no longer be default");

    // Cleanup
    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_legacy_slide_synthesis() {
    use os_next::models::{Slide, SlideElement};
    let mut slide = Slide {
        text: "Line 1\nLine 2".to_string(),
        header: Some("Header".to_string()),
        label: Some("V1".to_string()),
        background: Some("ocean.jpg".to_string()),
        notes: Some("Speaker note".to_string()),
        tag: Some("V1".to_string()),
        ..Default::default()
    };
    assert!(slide.elements.is_empty());
    slide.synthesize_v2_if_needed();
    assert_eq!(slide.elements.len(), 1);
    match &slide.elements[0] {
        SlideElement::TextBlock { id: _, transform, block } => {
            assert_eq!(block.runs.len(), 1);
            assert_eq!(block.runs[0].text, "Line 1\nLine 2");
            assert_eq!(transform.x, 0.05);
            assert_eq!(transform.y, 0.07);
            assert_eq!(transform.w, 0.90);
            assert_eq!(transform.h, 0.86);
        }
        _ => panic!("Expected TextBlock element"),
    }
    assert_eq!(slide.speaker_notes.as_deref(), Some("Speaker note"));
    assert!(slide.background_v2.is_some());

    // Project text test
    slide.text = "".to_string();
    slide.project_text_from_elements();
    assert_eq!(slide.text, "Line 1\nLine 2");
}

#[test]
fn test_element_transform_normalized() {
    use os_next::models::ElementTransform;
    let raw = ElementTransform {
        x: -0.5,
        y: 1.5,
        w: 1.8,
        h: -0.2,
        rotation_deg: 360.0,
        opacity: 2.0,
        z_index: -5,
        locked: false,
    };
    let norm = raw.normalized();
    assert_eq!(norm.x, 0.0);
    assert_eq!(norm.y, 1.0);
    assert_eq!(norm.w, 1.0);
    assert_eq!(norm.h, 0.0);
    assert_eq!(norm.opacity, 1.0);
}

#[test]
fn test_batch_slide_edit_single_undo() {
    use os_next::core::commands::{ShowCommand, SlideEditOp};
    use os_next::core::engine::ShowEngine;
    use os_next::models::{ElementTransform, ScheduleItem, Slide, SlideElement, TextBlock, TextParagraphStyle, TextRun};

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let initial_slide = Slide { text: "Original Lyric Line".to_string(), ..Default::default() };
    let item = ScheduleItem {
        id: "item_batch".to_string(),
        title: "Batch Song".to_string(),
        item_type: "song".to_string(),
        author_or_ref: "Author".to_string(),
        slides: vec![initial_slide],
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false,
        subtitle: None,
        notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    };
    engine.execute_command(ShowCommand::AddToSchedule(item));

    let new_element = SlideElement::TextBlock {
        id: "new_el_1".to_string(),
        transform: ElementTransform {
            x: 0.1,
            y: 0.1,
            w: 0.8,
            h: 0.2,
            rotation_deg: 0.0,
            opacity: 1.0,
            z_index: 2,
            locked: false,
        },
        block: TextBlock {
            runs: vec![TextRun {
                text: "Updated Element Content".to_string(),
                bold: true,
                ..Default::default()
            }],
            paragraph_style: TextParagraphStyle::default(),
            effects: Default::default(),
            autofit: true,
        },
    };

    let ops = vec![
        SlideEditOp::add_element(new_element),
        SlideEditOp::set_speaker_notes("Preach here"),
    ];

    let events = engine.execute_command(ShowCommand::BatchSlideEdit {
        item_index: 0,
        slide_index: 0,
        ops,
    });
    assert_eq!(events.len(), 1, "BatchSlideEdit must produce exactly one atomic ShowEvent");

    let snap_after = engine.snapshot();
    let edited_slide = &snap_after.schedule.items[0].slides[0];
    assert_eq!(edited_slide.elements.len(), 2);
    assert_eq!(edited_slide.speaker_notes.as_deref(), Some("Preach here"));

    // Single undo rolls back the entire batch atomically
    let undo_events = engine.execute_command(ShowCommand::Undo);
    assert!(!undo_events.is_empty(), "Undo must succeed");

    let snap_undone = engine.snapshot();
    let restored_slide = &snap_undone.schedule.items[0].slides[0];
    assert_eq!(restored_slide.elements.len(), 1);
    assert_eq!(restored_slide.speaker_notes, None);
}

#[test]
fn test_batch_slide_edit_set_background_and_transition() {
    use os_next::core::commands::{ShowCommand, SlideEditOp};
    use os_next::core::engine::ShowEngine;
    use os_next::models::{ScheduleItem, Slide, SlideBackground, SlideTransition};

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let initial_slide = Slide { text: "Background Test".to_string(), ..Default::default() };
    let item = ScheduleItem {
        id: "item_bg".to_string(),
        title: "BG Song".to_string(),
        item_type: "song".to_string(),
        author_or_ref: "Author".to_string(),
        slides: vec![initial_slide],
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false,
        subtitle: None,
        notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    };
    engine.execute_command(ShowCommand::AddToSchedule(item));

    let background = SlideBackground::Solid("#112233".to_string());
    let transition = SlideTransition { kind: "dissolve".to_string(), duration_ms: 600 };

    let ops = vec![
        SlideEditOp::set_background(background.clone()),
        SlideEditOp::set_transition(transition.clone()),
    ];

    let events = engine.execute_command(ShowCommand::BatchSlideEdit {
        item_index: 0,
        slide_index: 0,
        ops,
    });
    assert_eq!(events.len(), 1, "BatchSlideEdit must produce exactly one atomic ShowEvent");

    let snap_after = engine.snapshot();
    let edited_slide = &snap_after.schedule.items[0].slides[0];
    assert_eq!(edited_slide.background_v2, Some(background));
    assert_eq!(edited_slide.background.as_deref(), Some("#112233"));
    assert_eq!(edited_slide.transition, Some(transition));

    // Single undo rolls back the entire batch atomically
    let undo_events = engine.execute_command(ShowCommand::Undo);
    assert!(!undo_events.is_empty(), "Undo must succeed");

    let snap_undone = engine.snapshot();
    let restored_slide = &snap_undone.schedule.items[0].slides[0];
    assert_eq!(restored_slide.background_v2, None);
    assert_eq!(restored_slide.transition, None);
}

#[test]
fn test_z_order_dense_after_reorder() {
    use os_next::core::commands::ShowCommand;
    use os_next::core::engine::ShowEngine;
    use os_next::models::{ElementTransform, ScheduleItem, Slide, SlideElement, TextBlock, TextParagraphStyle, TextRun};

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let make_tb = |id: &str, z: i32| -> SlideElement {
        SlideElement::TextBlock {
            id: id.to_string(),
            transform: ElementTransform {
                x: 0.1,
                y: 0.1,
                w: 0.5,
                h: 0.5,
                rotation_deg: 0.0,
                opacity: 1.0,
                z_index: z,
                locked: false,
            },
            block: TextBlock {
                runs: vec![TextRun {
                    text: id.to_string(),
                    ..Default::default()
                }],
                paragraph_style: TextParagraphStyle::default(),
                effects: Default::default(),
                autofit: true,
            },
        }
    };

    let mut slide = Slide::default();
    slide.elements = vec![make_tb("el_a", 0), make_tb("el_b", 1), make_tb("el_c", 2)];
    slide.slide_document_version = 2;

    let item = ScheduleItem {
        id: "item_reorder".to_string(),
        title: "Reorder Song".to_string(),
        item_type: "song".to_string(),
        author_or_ref: "Author".to_string(),
        slides: vec![slide],
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false,
        subtitle: None,
        notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    };
    engine.execute_command(ShowCommand::AddToSchedule(item));

    // Reorder: Move el_c (which was z=2) to bottom (z=-1), so el_c=0, el_a=1, el_b=2
    engine.execute_command(ShowCommand::ReorderSlideElements {
        item_index: 0,
        slide_index: 0,
        element_id: "el_c".to_string(),
        to_z: -1,
    });

    let snap = engine.snapshot();
    let reordered_slide = &snap.schedule.items[0].slides[0];
    assert_eq!(reordered_slide.elements.len(), 3);
    assert_eq!(reordered_slide.elements[0].id(), "el_c");
    assert_eq!(reordered_slide.elements[0].transform().z_index, 0);
    assert_eq!(reordered_slide.elements[1].id(), "el_a");
    assert_eq!(reordered_slide.elements[1].transform().z_index, 1);
    assert_eq!(reordered_slide.elements[2].id(), "el_b");
    assert_eq!(reordered_slide.elements[2].transform().z_index, 2);
}

#[test]
fn test_group_ungroup_roundtrip() {
    use os_next::core::commands::ShowCommand;
    use os_next::core::engine::ShowEngine;
    use os_next::models::{ElementTransform, ScheduleItem, Slide, SlideElement, TextBlock, TextParagraphStyle, TextRun};

    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let tb1 = SlideElement::TextBlock {
        id: "el_1".to_string(),
        transform: ElementTransform {
            x: 0.1,
            y: 0.2,
            w: 0.3,
            h: 0.2,
            rotation_deg: 0.0,
            opacity: 1.0,
            z_index: 0,
            locked: false,
        },
        block: TextBlock {
            runs: vec![TextRun {
                text: "Hello".to_string(),
                ..Default::default()
            }],
            paragraph_style: TextParagraphStyle::default(),
            effects: Default::default(),
            autofit: true,
        },
    };

    let tb2 = SlideElement::TextBlock {
        id: "el_2".to_string(),
        transform: ElementTransform {
            x: 0.2,
            y: 0.3,
            w: 0.4,
            h: 0.3,
            rotation_deg: 0.0,
            opacity: 1.0,
            z_index: 1,
            locked: false,
        },
        block: TextBlock {
            runs: vec![TextRun {
                text: "World".to_string(),
                ..Default::default()
            }],
            paragraph_style: TextParagraphStyle::default(),
            effects: Default::default(),
            autofit: true,
        },
    };

    let mut slide = Slide::default();
    slide.elements = vec![tb1, tb2];
    slide.slide_document_version = 2;

    let item = ScheduleItem {
        id: "item_group".to_string(),
        title: "Group Song".to_string(),
        item_type: "song".to_string(),
        author_or_ref: "Author".to_string(),
        slides: vec![slide],
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false,
        subtitle: None,
        notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    };
    engine.execute_command(ShowCommand::AddToSchedule(item));

    // Group elements
    engine.execute_command(ShowCommand::GroupSlideElements {
        item_index: 0,
        slide_index: 0,
        element_ids: vec!["el_1".to_string(), "el_2".to_string()],
    });

    let snap_grouped = engine.snapshot();
    let grouped_slide = &snap_grouped.schedule.items[0].slides[0];
    assert_eq!(grouped_slide.elements.len(), 1, "Should now be 1 group element");
    let group_id = match &grouped_slide.elements[0] {
        SlideElement::Group { id, transform, children } => {
            assert_eq!(children.len(), 2);
            assert_eq!(transform.x, 0.1);
            assert_eq!(transform.y, 0.2);
            // min_x=0.1, max_x=0.6 -> width = 0.5
            // min_y=0.2, max_y=0.6 -> height = 0.4
            assert!((transform.w - 0.5).abs() < 1e-5);
            assert!((transform.h - 0.4).abs() < 1e-5);
            id.clone()
        }
        _ => panic!("Expected Group element"),
    };

    // Ungroup elements
    engine.execute_command(ShowCommand::UngroupSlideElements {
        item_index: 0,
        slide_index: 0,
        group_id,
    });

    let snap_ungrouped = engine.snapshot();
    let ungrouped_slide = &snap_ungrouped.schedule.items[0].slides[0];
    assert_eq!(ungrouped_slide.elements.len(), 2, "Should restore 2 child elements");
    assert_eq!(ungrouped_slide.elements[0].id(), "el_1");
    assert_eq!(ungrouped_slide.elements[1].id(), "el_2");
    assert!((ungrouped_slide.elements[0].transform().x - 0.1).abs() < 1e-5);
    assert!((ungrouped_slide.elements[1].transform().x - 0.2).abs() < 1e-5);
}

#[test]
fn test_merge_schedule_items_combines_slides_and_undoes_cleanly() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let item1 = ScheduleItem {
        id: "item1".to_string(),
        title: "Announcement Part 1".to_string(),
        item_type: "presentation".to_string(),
        author_or_ref: String::new(),
        slides: vec![
            Slide { text: "Welcome".to_string(), header: None, label: None, background: None, notes: None, tag: Some("V1".to_string()), ..Default::default() },
            Slide { text: "Please stand".to_string(), header: None, label: None, background: None, notes: None, tag: Some("V2".to_string()), ..Default::default() },
        ],
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false, subtitle: None, notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    };
    let item2 = ScheduleItem {
        id: "item2".to_string(),
        title: "Announcement Part 2".to_string(),
        item_type: "presentation".to_string(),
        author_or_ref: String::new(),
        slides: vec![
            Slide { text: "Offering".to_string(), header: None, label: None, background: None, notes: None, tag: Some("V1".to_string()), ..Default::default() },
        ],
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false, subtitle: None, notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    };
    let item3 = ScheduleItem {
        id: "item3".to_string(),
        title: "Closing Song".to_string(),
        item_type: "song".to_string(),
        author_or_ref: String::new(),
        slides: vec![Slide { text: "Amen".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() }],
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false, subtitle: None, notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    };

    engine.execute_command(ShowCommand::AddToSchedule(item1));
    engine.execute_command(ShowCommand::AddToSchedule(item2));
    engine.execute_command(ShowCommand::AddToSchedule(item3));
    assert_eq!(engine.snapshot().schedule.items.len(), 3);

    engine.execute_command(ShowCommand::MergeScheduleItems { first_item_index: 0, second_item_index: 1 });

    let snap = engine.snapshot();
    assert_eq!(snap.schedule.items.len(), 2, "the two merged items collapse into one");
    let merged = &snap.schedule.items[0];
    assert_eq!(merged.item_type, "presentation");
    assert_eq!(merged.title, "Announcement Part 1", "keeps the earlier item's title");
    assert_eq!(
        merged.slides.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
        vec!["Welcome", "Please stand", "Offering"],
        "slides concatenate in schedule order"
    );
    assert_eq!(merged.arrangement.len(), 3, "gets a fresh identity arrangement");
    assert_eq!(snap.schedule.items[1].title, "Closing Song", "untouched item shifts down by one");

    // Merging is order-independent: passing the later index first still keeps
    // schedule order in the concatenated result and the surviving item's position.
    let event_log2 = Arc::new(EventLog::new(100));
    let engine2 = ShowEngine::new(event_log2);
    engine2.execute_command(ShowCommand::AddToSchedule(ScheduleItem {
        id: "a".to_string(), title: "A".to_string(), item_type: "presentation".to_string(), author_or_ref: String::new(),
        slides: vec![Slide { text: "A1".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() }],
        background: None, theme_name: None, is_expanded: false, is_section_header: false, subtitle: None, notes: None, arrangement: Vec::new(),
        default_slide_duration_seconds: None, slideshow_loop: false,
    }));
    engine2.execute_command(ShowCommand::AddToSchedule(ScheduleItem {
        id: "b".to_string(), title: "B".to_string(), item_type: "presentation".to_string(), author_or_ref: String::new(),
        slides: vec![Slide { text: "B1".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() }],
        background: None, theme_name: None, is_expanded: false, is_section_header: false, subtitle: None, notes: None, arrangement: Vec::new(),
        default_slide_duration_seconds: None, slideshow_loop: false,
    }));
    engine2.execute_command(ShowCommand::MergeScheduleItems { first_item_index: 1, second_item_index: 0 });
    let snap2 = engine2.snapshot();
    assert_eq!(snap2.schedule.items.len(), 1);
    assert_eq!(
        snap2.schedule.items[0].slides.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
        vec!["A1", "B1"],
        "concatenation always follows schedule position, not argument order"
    );

    // Undo restores both original items exactly.
    engine.execute_command(ShowCommand::Undo);
    let restored = engine.snapshot();
    assert_eq!(restored.schedule.items.len(), 3);
    assert_eq!(restored.schedule.items[0].title, "Announcement Part 1");
    assert_eq!(restored.schedule.items[1].title, "Announcement Part 2");
    assert_eq!(restored.schedule.items[2].title, "Closing Song");
}

#[test]
fn test_merge_schedule_items_rejects_section_headers_and_self_merge() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    let header = ScheduleItem {
        id: "hdr".to_string(), title: "Header".to_string(), item_type: "header".to_string(), author_or_ref: String::new(),
        slides: Vec::new(), background: None, theme_name: None, is_expanded: true, is_section_header: true, subtitle: None, notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None, slideshow_loop: false,
    };
    let song = ScheduleItem {
        id: "song".to_string(), title: "Song".to_string(), item_type: "song".to_string(), author_or_ref: String::new(),
        slides: vec![Slide { text: "Only".to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() }],
        background: None, theme_name: None, is_expanded: false, is_section_header: false, subtitle: None, notes: None, arrangement: Vec::new(),
        default_slide_duration_seconds: None, slideshow_loop: false,
    };
    engine.execute_command(ShowCommand::AddToSchedule(header));
    engine.execute_command(ShowCommand::AddToSchedule(song));
    assert_eq!(engine.snapshot().schedule.items.len(), 2);

    // Merging a section header is a no-op.
    engine.execute_command(ShowCommand::MergeScheduleItems { first_item_index: 0, second_item_index: 1 });
    assert_eq!(engine.snapshot().schedule.items.len(), 2, "section headers can't be merged");

    // Merging an item with itself is a no-op.
    engine.execute_command(ShowCommand::MergeScheduleItems { first_item_index: 1, second_item_index: 1 });
    assert_eq!(engine.snapshot().schedule.items.len(), 2, "self-merge is a no-op");
}

fn simple_item(id: &str, title: &str, texts: &[&str]) -> ScheduleItem {
    ScheduleItem {
        id: id.to_string(),
        title: title.to_string(),
        item_type: "presentation".to_string(),
        author_or_ref: String::new(),
        slides: texts.iter().map(|t| Slide { text: t.to_string(), header: None, label: None, background: None, notes: None, tag: None, ..Default::default() }).collect(),
        background: None, theme_name: None,
        is_expanded: false,
        is_section_header: false, subtitle: None, notes: None,
        arrangement: Vec::new(),
        default_slide_duration_seconds: None,
        slideshow_loop: false,
    }
}

#[test]
fn test_slide_and_presentation_duration_and_loop_commands() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);
    engine.execute_command(ShowCommand::AddToSchedule(simple_item("p1", "Slideshow", &["Slide 1", "Slide 2", "Slide 3"])));

    // Set a presentation-level default duration.
    engine.execute_command(ShowCommand::SetPresentationDefaultDuration { item_index: 0, duration_seconds: Some(6.0) });
    let snap = engine.snapshot();
    assert_eq!(snap.schedule.items[0].default_slide_duration_seconds, Some(6.0));
    // No per-slide override yet: all slides inherit the default (None override).
    assert!(snap.schedule.items[0].slides.iter().all(|s| s.duration_seconds.is_none()));

    // Override just the second slide.
    engine.execute_command(ShowCommand::SetSlideDuration { item_index: 0, slide_index: 1, duration_seconds: Some(2.5) });
    let snap = engine.snapshot();
    assert_eq!(snap.schedule.items[0].slides[0].duration_seconds, None, "untouched slide keeps no override");
    assert_eq!(snap.schedule.items[0].slides[1].duration_seconds, Some(2.5));
    assert_eq!(snap.schedule.items[0].slides[2].duration_seconds, None);

    // Clear the per-slide override.
    engine.execute_command(ShowCommand::SetSlideDuration { item_index: 0, slide_index: 1, duration_seconds: None });
    assert_eq!(engine.snapshot().schedule.items[0].slides[1].duration_seconds, None);

    // Enable looping.
    assert_eq!(engine.snapshot().schedule.items[0].slideshow_loop, false);
    engine.execute_command(ShowCommand::SetPresentationLoop { item_index: 0, loop_enabled: true });
    assert_eq!(engine.snapshot().schedule.items[0].slideshow_loop, true);

    // Out-of-bounds indices are silently ignored, not a panic or corruption.
    let before = engine.snapshot().schedule.items[0].clone();
    engine.execute_command(ShowCommand::SetSlideDuration { item_index: 0, slide_index: 99, duration_seconds: Some(1.0) });
    engine.execute_command(ShowCommand::SetPresentationDefaultDuration { item_index: 5, duration_seconds: Some(1.0) });
    engine.execute_command(ShowCommand::SetPresentationLoop { item_index: 5, loop_enabled: true });
    assert_eq!(engine.snapshot().schedule.items[0], before, "out-of-bounds commands must be no-ops");
}

#[test]
fn test_slide_duration_resyncs_live_and_staged_item_copies() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);
    engine.execute_command(ShowCommand::AddToSchedule(simple_item("p1", "Slideshow", &["Slide 1", "Slide 2"])));
    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(0) });
    engine.execute_command(ShowCommand::StageItem { item_index: Some(0), slide_index: Some(0) });

    engine.execute_command(ShowCommand::SetPresentationDefaultDuration { item_index: 0, duration_seconds: Some(4.0) });
    engine.execute_command(ShowCommand::SetPresentationLoop { item_index: 0, loop_enabled: true });

    let snap = engine.snapshot();
    let live = snap.state.live_item.expect("live item should be set");
    let staged = snap.state.staged_item.expect("staged item should be set");
    assert_eq!(live.default_slide_duration_seconds, Some(4.0), "live copy must resync on duration change");
    assert!(live.slideshow_loop, "live copy must resync on loop change");
    assert_eq!(staged.default_slide_duration_seconds, Some(4.0), "staged copy must resync too");
    assert!(staged.slideshow_loop);
}

#[test]
fn test_set_item_theme_overrides_existing_per_slide_backgrounds() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);
    let mut item = simple_item("song1", "A New Commandment", &["Verse 1", "Verse 2"]);
    // Slides already carry their own background — this is what shadowed the
    // item-level theme change in resolveSlideBackground's cascade and made
    // "apply theme" appear to do nothing.
    for slide in item.slides.iter_mut() {
        slide.background = Some("old-background.jpg".to_string());
    }
    engine.execute_command(ShowCommand::AddToSchedule(item));
    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(0) });
    engine.execute_command(ShowCommand::StageItem { item_index: Some(0), slide_index: Some(0) });

    engine.execute_command(ShowCommand::SetItemTheme {
        item_index: 0,
        theme_name: "Sunset Horizon".to_string(),
        background: Some("linear-gradient(160deg, #2d1b4e, #c74b50, #f5a25d)".to_string()),
    });

    let snap = engine.snapshot();
    let item = &snap.schedule.items[0];
    assert_eq!(item.background.as_deref(), Some("linear-gradient(160deg, #2d1b4e, #c74b50, #f5a25d)"));
    for slide in &item.slides {
        assert_eq!(
            slide.background.as_deref(),
            Some("linear-gradient(160deg, #2d1b4e, #c74b50, #f5a25d)"),
            "every slide's own background must be overwritten too, or the item-level theme is shadowed"
        );
    }

    let live = snap.state.live_item.expect("live item should be set");
    let staged = snap.state.staged_item.expect("staged item should be set");
    assert!(live.slides.iter().all(|s| s.background.as_deref() == Some("linear-gradient(160deg, #2d1b4e, #c74b50, #f5a25d)")), "live copy must resync");
    assert!(staged.slides.iter().all(|s| s.background.as_deref() == Some("linear-gradient(160deg, #2d1b4e, #c74b50, #f5a25d)")), "staged copy must resync too");
}

#[test]
fn test_merge_schedule_items_rejects_non_adjacent() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);
    for (i, name) in ["A", "B", "C"].iter().enumerate() {
        engine.execute_command(ShowCommand::AddToSchedule(simple_item(name, name, &[&format!("{}1", name)])));
        let _ = i;
    }
    assert_eq!(engine.snapshot().schedule.items.len(), 3);

    // Indices 0 and 2 are not adjacent (item "B" sits between them) — must be rejected
    // rather than silently swallowing "B".
    engine.execute_command(ShowCommand::MergeScheduleItems { first_item_index: 0, second_item_index: 2 });
    let snap = engine.snapshot();
    assert_eq!(snap.schedule.items.len(), 3, "non-adjacent merge is rejected");
    assert_eq!(snap.schedule.items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), vec!["A", "B", "C"]);
}

#[test]
fn test_merge_schedule_items_shifts_selected_index_correctly() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);
    for name in ["A", "B", "C", "D", "E"] {
        engine.execute_command(ShowCommand::AddToSchedule(simple_item(name, name, &[&format!("{}1", name)])));
    }
    assert_eq!(engine.snapshot().schedule.items.len(), 5);

    // Select D (index 3), then merge A and B (indices 0, 1) — unrelated to D's position.
    {
        let mut schedule = engine.snapshot().schedule;
        schedule.selected_item_index = Some(3);
        engine.execute_command(ShowCommand::LoadSchedule(schedule));
    }
    engine.execute_command(ShowCommand::MergeScheduleItems { first_item_index: 0, second_item_index: 1 });

    let snap = engine.snapshot();
    assert_eq!(
        snap.schedule.items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
        vec!["A", "C", "D", "E"],
        "A+B collapse into one item titled A, C/D/E shift down by one"
    );
    assert_eq!(snap.state.selected_item_index, Some(2), "selection must follow D to its new index, not drift to C");
}

#[test]
fn test_merge_schedule_items_preserves_background_overrides() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);
    engine.execute_command(ShowCommand::AddToSchedule(simple_item("first", "First", &["A1", "A2"])));
    engine.execute_command(ShowCommand::AddToSchedule(simple_item("second", "Second", &["B1"])));

    // Give the first item a custom arrangement with a per-position background override.
    let custom_arrangement = vec![
        ArrangementEntry { section_id: "S1".to_string(), source_slide_index: 0, background_override: Some("red.jpg".to_string()) },
        ArrangementEntry { section_id: "S2".to_string(), source_slide_index: 1, background_override: None },
    ];
    engine.execute_command(ShowCommand::SetArrangement { item_index: Some(0), arrangement: custom_arrangement });

    engine.execute_command(ShowCommand::MergeScheduleItems { first_item_index: 0, second_item_index: 1 });

    let snap = engine.snapshot();
    assert_eq!(snap.schedule.items.len(), 1);
    let merged = &snap.schedule.items[0];
    assert_eq!(merged.arrangement.len(), 3);
    assert_eq!(merged.arrangement[0].background_override.as_deref(), Some("red.jpg"), "the override on the first slide must survive the merge");
    assert_eq!(merged.arrangement[1].background_override, None);
    assert_eq!(merged.arrangement[2].background_override, None);
}

#[test]
fn test_schedule_replace_vs_append_mode() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);

    // Initial schedule with item A
    engine.execute_command(ShowCommand::AddToSchedule(simple_item("item_a", "Item A", &["Slide A1"])));
    assert_eq!(engine.snapshot().schedule.items.len(), 1);

    // Incoming schedule with item B
    let incoming = os_next::core::models::Schedule {
        id: "incoming_sched".to_string(),
        title: "Incoming Schedule".to_string(),
        items: vec![simple_item("item_b", "Item B", &["Slide B1"])],
        selected_item_index: None,
        is_modified: false,
        schedule_version: 0,
    };

    // Case 1: Append mode adds items without dropping existing items
    for item in incoming.items.clone() {
        engine.execute_command(ShowCommand::AddToSchedule(item));
    }
    let snap_appended = engine.snapshot();
    assert_eq!(snap_appended.schedule.items.len(), 2);
    assert_eq!(snap_appended.schedule.items[0].title, "Item A");
    assert_eq!(snap_appended.schedule.items[1].title, "Item B");

    // Case 2: Replace mode replaces the entire schedule
    engine.execute_command(ShowCommand::LoadSchedule(incoming));
    let snap_replaced = engine.snapshot();
    assert_eq!(snap_replaced.schedule.items.len(), 1);
    assert_eq!(snap_replaced.schedule.items[0].title, "Item B");
    assert_eq!(snap_replaced.schedule.title, "Incoming Schedule");
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
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: web_dir.clone(),
        media_dir: web_dir.clone(),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: "test_host_session_token".to_string(),
        server_port: 8080,
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
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

#[test]
fn test_sync_media_folder_autopopulates_images() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_media_sync.db");
    let db = Database::new(&db_path).expect("Database::new");

    let media_images_dir = test_dir.path().join("media").join("images");
    std::fs::create_dir_all(&media_images_dir).unwrap();

    // Create two dummy image files
    std::fs::write(media_images_dir.join("majestic-canyon-sunset.jpg"), b"fake_jpg_bytes").unwrap();
    std::fs::write(media_images_dir.join("golden_wheat_field.png"), b"fake_png_bytes").unwrap();
    std::fs::write(media_images_dir.join("notes.txt"), b"not an image").unwrap();

    let added = db.sync_media_folder(&media_images_dir).expect("sync_media_folder should succeed");
    assert_eq!(added, 2, "Both images should be discovered and added");

    let media = db.get_media().expect("get_media");
    assert_eq!(media.len(), 2);

    let canyon = media.iter().find(|m| m.file_path == "/media/images/majestic-canyon-sunset.jpg").unwrap();
    assert_eq!(canyon.name, "Majestic Canyon Sunset");
    assert_eq!(canyon.media_type, "Image");

    let wheat = media.iter().find(|m| m.file_path == "/media/images/golden_wheat_field.png").unwrap();
    assert_eq!(wheat.name, "Golden Wheat Field");
    assert_eq!(wheat.media_type, "Image");

    // Second sync should find 0 new items (idempotent)
    let added_second = db.sync_media_folder(&media_images_dir).expect("second sync");
    assert_eq!(added_second, 0, "No duplicate items should be added");
}

#[test]
fn test_sync_media_videos_folder_autopopulates_videos() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_video_sync.db");
    let db = Database::new(&db_path).expect("Database::new");

    let media_videos_dir = test_dir.path().join("media").join("videos");
    std::fs::create_dir_all(&media_videos_dir).unwrap();

    // Create sample video files and a non-video file
    std::fs::write(media_videos_dir.join("worship-motion-background.mp4"), b"fake_mp4_bytes").unwrap();
    std::fs::write(media_videos_dir.join("ambient_clouds_loop.webm"), b"fake_webm_bytes").unwrap();
    std::fs::write(media_videos_dir.join("readme.md"), b"not a video").unwrap();

    let added = db.sync_media_videos_folder(&media_videos_dir).expect("sync_media_videos_folder should succeed");
    assert_eq!(added, 2, "Both video files should be discovered and added");

    let media = db.get_media().expect("get_media");
    assert_eq!(media.len(), 2);

    let motion = media.iter().find(|m| m.file_path == "/media/videos/worship-motion-background.mp4").unwrap();
    assert_eq!(motion.name, "Worship Motion Background");
    assert_eq!(motion.media_type, "Video");
    assert!(motion.loop_playback, "Video background should have loop_playback enabled");

    let clouds = media.iter().find(|m| m.file_path == "/media/videos/ambient_clouds_loop.webm").unwrap();
    assert_eq!(clouds.name, "Ambient Clouds Loop");
    assert_eq!(clouds.media_type, "Video");

    // Second sync should be idempotent
    let added_second = db.sync_media_videos_folder(&media_videos_dir).expect("second sync");
    assert_eq!(added_second, 0, "No duplicate video items should be added");
}

#[tokio::test]
async fn test_keyring_metadata_ownership_persistence() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_keyring.db");
    let db = Database::new(&db_path).expect("Database::new");

    // Initially unrecorded
    let owner = db.get_keyring_owner("OpenSanctuary:Plugin:obs", "api_key").unwrap();
    assert_eq!(owner, None);

    // Record owner for plugin
    db.record_keyring_owner("OpenSanctuary:Plugin:obs", "api_key", "plugin:obs").unwrap();
    let owner = db.get_keyring_owner("OpenSanctuary:Plugin:obs", "api_key").unwrap();
    assert_eq!(owner, Some("plugin:obs".to_string()));

    // Record host owner for host secret
    db.record_keyring_owner("OpenSanctuary:Pexels", "api_key", "host").unwrap();
    let host_owner = db.get_keyring_owner("OpenSanctuary:Pexels", "api_key").unwrap();
    assert_eq!(host_owner, Some("host".to_string()));

    // Delete metadata
    let deleted = db.delete_keyring_metadata("OpenSanctuary:Plugin:obs", "api_key").unwrap();
    assert!(deleted);
    let owner_after = db.get_keyring_owner("OpenSanctuary:Plugin:obs", "api_key").unwrap();
    assert_eq!(owner_after, None);

    // Deleting non-existent returns false
    let deleted_again = db.delete_keyring_metadata("OpenSanctuary:Plugin:obs", "api_key").unwrap();
    assert!(!deleted_again);
}

#[tokio::test]
async fn test_app_state_plugin_token_lifecycle() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_tokens.db");
    let db = Database::new(&db_path).expect("Database::new");
    let event_log = Arc::new(EventLog::new(100));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _) = tokio::sync::broadcast::channel(16);

    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let app = os_next::api::ws::AppState {
        engine,
        db,
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        tx,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: "test_host_token_abc".to_string(),
        server_port: 8080,
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    // Issue tokens for different plugins
    let token_obs = app.issue_plugin_token("obs_remote").await;
    let token_pco = app.issue_plugin_token("planning_center").await;

    assert!(token_obs.starts_with("plg_"));
    assert!(token_pco.starts_with("plg_"));
    assert_ne!(token_obs, token_pco);

    // Resolve tokens
    let resolved_obs = app.resolve_plugin_token(&token_obs).await;
    assert_eq!(resolved_obs, Some("obs_remote".to_string()));

    let resolved_pco = app.resolve_plugin_token(&token_pco).await;
    assert_eq!(resolved_pco, Some("planning_center".to_string()));

    // Unknown token returns None
    let resolved_unknown = app.resolve_plugin_token("plg_nonexistent").await;
    assert_eq!(resolved_unknown, None);
}

#[tokio::test]
async fn test_network_api_routes_and_conflict_detection() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_network.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(2000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let bound_port = addr.port();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: "test_host_session_token".to_string(),
        server_port: bound_port,
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // 1. GET /api/network/info
    let resp = client.get(format!("http://{}/api/network/info", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let info_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(info_json["success"], true);
    assert_eq!(info_json["port"], bound_port);
    assert!(info_json["remote_url"].as_str().unwrap().contains(&format!(":{}", bound_port)));
    assert!(info_json["mdns_url"].as_str().unwrap().contains(".local:"));

    // 2. GET /api/network/interfaces
    let resp = client.get(format!("http://{}/api/network/interfaces", addr))
        .header("x-host-token", "test_host_session_token")
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ifaces_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(ifaces_json["success"], true);
    assert!(ifaces_json["interfaces"].is_array());
    assert_eq!(ifaces_json["port"], bound_port);

    // 3. POST /api/network/interfaces
    let save_payload = serde_json::json!({
        "enabled_interfaces": ["eth0", "wlan0"],
        "broadcast_all": true
    });
    let resp = client.post(format!("http://{}/api/network/interfaces", addr))
        .header("x-host-token", "test_host_session_token")
        .json(&save_payload)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let save_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(save_json["success"], true);

    // Verify settings saved in db
    let settings = db.get_settings().unwrap();
    assert_eq!(settings.get("networkEnabledInterfaces").map(|s| s.as_str()), Some("eth0,wlan0"));
    assert_eq!(settings.get("networkBroadcastAll").map(|s| s.as_str()), Some("true"));

    // 4. GET /api/network/check-port?port=<bound_port>
    // Should identify as in_use_by_current = true and available = true (since it belongs to this server)
    let resp = client.get(format!("http://{}/api/network/check-port?port={}", addr, bound_port))
        .header("x-host-token", "test_host_session_token")
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let port_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(port_json["success"], true);
    assert_eq!(port_json["available"], true);
    assert_eq!(port_json["in_use_by_current"], true);

    // 5. GET /api/network/check-hostname?name=MySanctuary_Host
    let resp = client.get(format!("http://{}/api/network/check-hostname?name=MySanctuary_Host", addr))
        .header("x-host-token", "test_host_session_token")
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let hn_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(hn_json["success"], true);
    assert_eq!(hn_json["sanitized_hostname"], "mysanctuary-host");

    // 6. POST /api/network/broadcast-option12
    let bcast_payload = serde_json::json!({
        "hostname": "test-bcast-host"
    });
    let resp = client.post(format!("http://{}/api/network/broadcast-option12", addr))
        .header("x-host-token", "test_host_session_token")
        .json(&bcast_payload)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let bcast_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(bcast_json["success"], true);
    assert_eq!(bcast_json["hostname"], "test-bcast-host");
}

/// Regression test: the hostname/port conflict check was only ever wired
/// into the read-only
/// GET /api/network/check-hostname and check-port endpoints — the actual
/// write paths (POST /api/settings, POST /api/network/dedicated-mac/toggle)
/// saved whatever they were given regardless of what the check would have
/// said. This proves the write paths now hold a flagged value back (while
/// still saving the rest of the same request) unless explicitly overridden.
#[tokio::test]
async fn test_conflicting_network_settings_are_held_back_unless_confirmed() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_conflict.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(2100, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let bound_port = addr.port();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: "test_host_session_conflict".to_string(),
        server_port: bound_port,
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // --- Port conflict: deterministic, no network dependency ---
    // Occupy a real port with a plain TCP listener so check_port_conflict
    // has a genuine collision to find, exactly like
    // test_check_port_conflict_detects_occupied_port in src/network/mod.rs.
    let occupied_listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let occupied_port = occupied_listener.local_addr().unwrap().port();

    let resp = client.post(format!("http://{}/api/settings", addr))
        .header("x-host-token", "test_host_session_conflict")
        .json(&serde_json::json!({
            "networkPort": occupied_port.to_string(),
            "churchName": "Held Back Church",
        }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["_conflicts"]["networkPort"].is_object(), "expected networkPort to be flagged: {body}");

    let settings_after = db.get_settings().unwrap();
    // The conflicting key was held back entirely (this is a fresh db, so it
    // was never set to anything at all — not the occupied value, not
    // anything else)...
    assert!(settings_after.get("networkPort").is_none());
    // ...but an unrelated key in the SAME request still saved normally.
    assert_eq!(settings_after.get("churchName").map(|s| s.as_str()), Some("Held Back Church"));

    // Resubmitting with _confirmOverrides saves it anyway.
    let resp = client.post(format!("http://{}/api/settings", addr))
        .header("x-host-token", "test_host_session_conflict")
        .json(&serde_json::json!({
            "networkPort": occupied_port.to_string(),
            "_confirmOverrides": "networkPort",
        }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.get("_conflicts").is_none());
    let settings_forced = db.get_settings().unwrap();
    assert_eq!(settings_forced.get("networkPort").map(|s| s.as_str()), Some(occupied_port.to_string().as_str()));
    drop(occupied_listener);

    // A non-conflicting port saves through with no _conflicts at all. Bind
    // and immediately drop a fresh ephemeral listener to get a port number
    // that's very likely free, rather than guessing via arithmetic.
    let free_port = {
        let probe = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    let resp = client.post(format!("http://{}/api/settings", addr))
        .header("x-host-token", "test_host_session_conflict")
        .json(&serde_json::json!({ "networkPort": free_port.to_string() }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.get("_conflicts").is_none());

    // Deliberately NOT testing POST /api/network/dedicated-mac/toggle's new
    // hostname-conflict guard here with `enable: true`, even with a clean
    // (non-conflicting) hostname. That handler falls through to a real
    // `pkexec` invocation (src/network/linux.rs) whenever there's no
    // conflict — confirmed the hard way while writing this test: with no
    // polkit auth agent registered in this sandbox, `pkexec` blocks
    // indefinitely instead of failing fast, which hung this exact test
    // until the child process was killed by hand. That's a pre-existing
    // risk this fix didn't introduce, just newly reachable by testing the
    // handler at all — an automated test still shouldn't invoke it.
    //
    // The conflict branch itself WAS verified live, not just by inspection:
    // this dev machine's avahi-daemon already sees a real device on the LAN
    // (`avahi-browse -rp -t _uscan._tcp` → an Epson printer answering as
    // `EPSON5490D4.local` / 192.168.1.118). Hitting this handler with that
    // exact hostname returned `{"success":false,"conflict":true,
    // "conflicting_ip":"192.168.1.118",...}` and did not touch pkexec at
    // all — the same request against `/api/settings` held `networkHostname`
    // back in `_conflicts` too. Not reproduced here as an automated
    // assertion because it depends on a specific device on this specific
    // LAN, which won't exist on another machine or in CI — but the
    // identical enforcement pattern (check, then hold back unless forced)
    // is proven end-to-end and automatically above via ports, which are
    // fully reproducible without any external device.
}

#[tokio::test]
async fn test_client_pairing_lifecycle_and_zero_auth_displays() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_pairing.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(3000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: "test_host_session_token".to_string(),
        server_port: addr.port(),
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        // with_connect_info: GET /api/internal/host-token needs the real
        // peer address to enforce its loopback-only check (see below).
        let _ = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await;
    });

    let client = reqwest::Client::new();
    let host_token = "test_host_session_token";

    // 0. CRITICAL AUTH INVARIANT: minting a pairing session with no host
    // token (or the wrong one) is rejected. Before this fix, any LAN
    // client could mint its own session and self-pair a rogue device.
    let sess_noauth_resp = client.post(format!("http://{}/api/pairing/session", addr))
        .send().await.unwrap();
    assert_eq!(sess_noauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let sess_badauth_resp = client.post(format!("http://{}/api/pairing/session", addr))
        .header("x-host-token", "wrong_token")
        .send().await.unwrap();
    assert_eq!(sess_badauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 1. POST /api/pairing/session (with the real host token) - generate ephemeral pairing session
    let sess_resp = client.post(format!("http://{}/api/pairing/session", addr))
        .header("x-host-token", host_token)
        .send().await.unwrap();
    assert_eq!(sess_resp.status(), reqwest::StatusCode::OK);
    let sess_json: serde_json::Value = sess_resp.json().await.unwrap();
    let session_token = sess_json["session_token"].as_str().unwrap().to_string();
    assert!(session_token.starts_with("pair_sess_"));
    assert_eq!(sess_json["expires_in"], 300);

    // 2. Reject authorization with invalid session token
    let bad_auth_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .json(&serde_json::json!({
            "session_token": "pair_sess_invalid_token",
            "device_id": "tv-sanctuary-1",
            "name": "Sanctuary Main TV",
            "platform": "android-tv"
        }))
        .send().await.unwrap();
    assert_eq!(bad_auth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let bad_auth_json: serde_json::Value = bad_auth_resp.json().await.unwrap();
    assert_eq!(bad_auth_json["error"], "Invalid or expired pairing session token");

    // 3. Status check before pairing reports false
    let status_before = client.get(format!("http://{}/api/pairing/status?device_id=tv-sanctuary-1", addr))
        .send().await.unwrap();
    assert_eq!(status_before.status(), reqwest::StatusCode::OK);
    let status_before_json: serde_json::Value = status_before.json().await.unwrap();
    assert_eq!(status_before_json["paired"], false);

    // 4. Authorize with valid session token
    let auth_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .json(&serde_json::json!({
            "session_token": session_token,
            "device_id": "tv-sanctuary-1",
            "name": "Sanctuary Main TV",
            "platform": "android-tv"
        }))
        .send().await.unwrap();
    assert_eq!(auth_resp.status(), reqwest::StatusCode::OK);
    let auth_json: serde_json::Value = auth_resp.json().await.unwrap();
    assert_eq!(auth_json["success"], true);
    assert_eq!(auth_json["device_id"], "tv-sanctuary-1");
    assert_eq!(auth_json["name"], "Sanctuary Main TV");
    let dev_token = auth_json["token"].as_str().unwrap().to_string();
    assert!(dev_token.starts_with("dev_tok_"));

    // 5. The same session token authorizes a SECOND device too — sessions
    // are scoped by TTL, not single-use, so a technician can pair every TV
    // in the building from one console-minted session within its 5-minute
    // window instead of the phone needing to mint its own sessions (which
    // would require giving the phone console-level trust it was never meant
    // to have). See `validate_pairing_session` in src/api/ws.rs.
    let second_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .json(&serde_json::json!({
            "session_token": session_token,
            "device_id": "tv-sanctuary-2",
            "name": "Sanctuary Side TV",
            "platform": "roku"
        }))
        .send().await.unwrap();
    assert_eq!(second_resp.status(), reqwest::StatusCode::OK);
    let second_json: serde_json::Value = second_resp.json().await.unwrap();
    assert_eq!(second_json["success"], true);
    assert_eq!(second_json["device_id"], "tv-sanctuary-2");

    // A session-token holder can't take over an already-paired device id;
    // the console (host token) can.
    let hijack_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .json(&serde_json::json!({
            "session_token": session_token,
            "device_id": "tv-sanctuary-1",
            "name": "Attacker",
            "platform": "android-tv"
        }))
        .send().await.unwrap();
    assert_eq!(hijack_resp.status(), reqwest::StatusCode::CONFLICT);
    let repair_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .header("x-host-token", host_token)
        .json(&serde_json::json!({
            "session_token": session_token,
            "device_id": "tv-sanctuary-2",
            "name": "Sanctuary Side TV",
            "platform": "roku"
        }))
        .send().await.unwrap();
    assert_eq!(repair_resp.status(), reqwest::StatusCode::OK);

    // An unknown/garbage session token still fails, same as before.
    let bad_reuse_resp = client.post(format!("http://{}/api/pairing/authorize", addr))
        .json(&serde_json::json!({
            "session_token": "pair_sess_totally_made_up",
            "device_id": "tv-sanctuary-3",
            "name": "Lobby TV",
            "platform": "roku"
        }))
        .send().await.unwrap();
    assert_eq!(bad_reuse_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 6. Status check after pairing reports true with token
    let status_after = client.get(format!("http://{}/api/pairing/status?device_id=tv-sanctuary-1", addr))
        .send().await.unwrap();
    assert_eq!(status_after.status(), reqwest::StatusCode::OK);
    let status_after_json: serde_json::Value = status_after.json().await.unwrap();
    assert_eq!(status_after_json["paired"], true);
    assert_eq!(status_after_json["device_id"], "tv-sanctuary-1");
    assert_eq!(status_after_json["token"], dev_token);

    // 6b. A SECOND status poll for the same, already-paired device_id no
    // longer includes the token -- it was already delivered once above.
    // Before this fix, any LAN caller could read an already-paired device's
    // permanent bearer token forever just by naming its device_id.
    let status_again = client.get(format!("http://{}/api/pairing/status?device_id=tv-sanctuary-1", addr))
        .send().await.unwrap();
    let status_again_json: serde_json::Value = status_again.json().await.unwrap();
    assert_eq!(status_again_json["paired"], true);
    assert_eq!(status_again_json["token"], serde_json::Value::Null);

    // 7. Verify token endpoint
    let verify_resp = client.post(format!("http://{}/api/pairing/verify", addr))
        .json(&serde_json::json!({ "token": dev_token }))
        .send().await.unwrap();
    assert_eq!(verify_resp.status(), reqwest::StatusCode::OK);
    let verify_json: serde_json::Value = verify_resp.json().await.unwrap();
    assert_eq!(verify_json["valid"], true);
    assert_eq!(verify_json["device_id"], "tv-sanctuary-1");

    // 8. GET /api/pairing/devices lists both paired devices — but only for
    // a caller with the host token; a plain LAN client gets nothing.
    // Before this fix, every paired device's permanent bearer token was
    // readable by anyone.
    let list_noauth_resp = client.get(format!("http://{}/api/pairing/devices", addr))
        .send().await.unwrap();
    assert_eq!(list_noauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let list_resp = client.get(format!("http://{}/api/pairing/devices", addr))
        .header("x-host-token", host_token)
        .send().await.unwrap();
    assert_eq!(list_resp.status(), reqwest::StatusCode::OK);
    let list_json: Vec<serde_json::Value> = list_resp.json().await.unwrap();
    assert_eq!(list_json.len(), 2);
    assert!(list_json.iter().any(|d| d["id"] == "tv-sanctuary-1" && d["name"] == "Sanctuary Main TV" && d["platform"] == "android-tv"));
    assert!(list_json.iter().any(|d| d["id"] == "tv-sanctuary-2"));

    // 9. DELETE /api/pairing/devices/:id unpairs the device — same gate.
    // A plain LAN client can no longer revoke a live display mid-service.
    let del_noauth_resp = client.delete(format!("http://{}/api/pairing/devices/tv-sanctuary-1", addr))
        .send().await.unwrap();
    assert_eq!(del_noauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let del_resp = client.delete(format!("http://{}/api/pairing/devices/tv-sanctuary-1", addr))
        .header("x-host-token", host_token)
        .send().await.unwrap();
    assert_eq!(del_resp.status(), reqwest::StatusCode::OK);
    let del_json: serde_json::Value = del_resp.json().await.unwrap();
    assert_eq!(del_json["success"], true);

    // Verify status returns false after revocation
    let status_revoked = client.get(format!("http://{}/api/pairing/status?device_id=tv-sanctuary-1", addr))
        .send().await.unwrap();
    let status_revoked_json: serde_json::Value = status_revoked.json().await.unwrap();
    assert_eq!(status_revoked_json["paired"], false);

    // 10. CRITICAL ZERO-AUTH INVARIANT:
    // Verify /api/state and public display endpoints require NO auth headers or tokens
    let state_resp = client.get(format!("http://{}/api/state", addr))
        .send().await.unwrap();
    assert_eq!(state_resp.status(), reqwest::StatusCode::OK);

    let remote_resp = client.get(format!("http://{}/remote", addr))
        .send().await.unwrap();
    // /remote redirects or renders 200 OK without requiring auth
    assert!(remote_resp.status().is_success() || remote_resp.status().is_redirection());

    // 11. GET /api/server-info no longer carries host_token — this route is
    // deliberately zero-auth (any LAN client reads it), which used to mean
    // anyone could read the host token from it and get unrestricted keyring
    // access.
    let server_info_resp = client.get(format!("http://{}/api/server-info", addr))
        .send().await.unwrap();
    assert_eq!(server_info_resp.status(), reqwest::StatusCode::OK);
    let server_info_json: serde_json::Value = server_info_resp.json().await.unwrap();
    assert!(server_info_json.get("host_token").is_none());
    assert!(server_info_json.get("instance_id").is_some());

    // 12. GET /api/internal/host-token: this test's own reqwest client
    // connects over loopback (127.0.0.1), so it's the one caller allowed to
    // read the host token this way — proving the intended "browser tab on
    // this machine" path works, not just that the network path is closed.
    let host_token_resp = client.get(format!("http://{}/api/internal/host-token", addr))
        .send().await.unwrap();
    assert_eq!(host_token_resp.status(), reqwest::StatusCode::OK);
    let host_token_json: serde_json::Value = host_token_resp.json().await.unwrap();
    assert_eq!(host_token_json["host_token"], host_token);

    // 12b. A reverse proxy on this same machine (Caddy, see docs/TUNNELS.md)
    // makes every real request's TCP peer loopback -- this test's own
    // reqwest client already IS that loopback peer, so it can simulate one
    // directly by adding its own X-Forwarded-For header. A real, non-
    // loopback original client reported that way must be rejected...
    let proxied_real_client_resp = client.get(format!("http://{}/api/internal/host-token", addr))
        .header("x-forwarded-for", "203.0.113.7")
        .send().await.unwrap();
    assert_eq!(proxied_real_client_resp.status(), reqwest::StatusCode::FORBIDDEN, "a proxy reporting a real, non-loopback original client must not get the host token");

    // ...while one reporting a loopback original client (e.g. a browser tab
    // on this same machine going through the proxy) still succeeds.
    let proxied_loopback_client_resp = client.get(format!("http://{}/api/internal/host-token", addr))
        .header("x-forwarded-for", "127.0.0.1")
        .send().await.unwrap();
    assert_eq!(proxied_loopback_client_resp.status(), reqwest::StatusCode::OK);

    // 13. POST /api/pairing/remote-session requires the host token too —
    // otherwise any LAN client could mint itself a live remote-control
    // token without ever touching the console or scanning a QR.
    let remote_sess_noauth_resp = client.post(format!("http://{}/api/pairing/remote-session", addr))
        .send().await.unwrap();
    assert_eq!(remote_sess_noauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let remote_sess_resp = client.post(format!("http://{}/api/pairing/remote-session", addr))
        .header("x-host-token", host_token)
        .send().await.unwrap();
    assert_eq!(remote_sess_resp.status(), reqwest::StatusCode::OK);
    let remote_sess_json: serde_json::Value = remote_sess_resp.json().await.unwrap();
    assert_eq!(remote_sess_json["success"], true);
    assert!(remote_sess_json["token"].as_str().unwrap().starts_with("dev_tok_"));
}

/// POST /api/keyring/plugin/token used to mint a valid, unrestricted token for
/// any plugin_name a caller asserted — including one that was never actually
/// installed, e.g. "planning_center" — letting anyone read that plugin's
/// vault the moment any *real* plugin's secret was ever stored under that
/// name.
#[tokio::test]
async fn test_plugin_token_requires_a_real_installed_plugin() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_plugin_token.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(3000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        // The real repo web dir — "hello_world" is a genuinely installed
        // plugin there (web/plugins/hello_world.js), loaded by main.ts.
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: "test_host_session_plugin".to_string(),
        server_port: addr.port(),
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // Minting a plugin token is console-only: an unauthenticated LAN caller is refused
    // even for a real installed plugin (it would unlock that plugin's keyring secrets).
    let anon_resp = client.post(format!("http://{}/api/keyring/plugin/token", addr))
        .json(&serde_json::json!({ "plugin_name": "hello_world" }))
        .send().await.unwrap();
    assert_eq!(anon_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // A made-up plugin name that was never installed is rejected.
    let fake_resp = client.post(format!("http://{}/api/keyring/plugin/token", addr))
        .header("x-host-token", "test_host_session_plugin")
        .json(&serde_json::json!({ "plugin_name": "planning_center" }))
        .send().await.unwrap();
    assert_eq!(fake_resp.status(), reqwest::StatusCode::NOT_FOUND);

    // A real, installed plugin still gets a token.
    let real_resp = client.post(format!("http://{}/api/keyring/plugin/token", addr))
        .header("x-host-token", "test_host_session_plugin")
        .json(&serde_json::json!({ "plugin_name": "hello_world" }))
        .send().await.unwrap();
    assert_eq!(real_resp.status(), reqwest::StatusCode::OK);
    let real_json: serde_json::Value = real_resp.json().await.unwrap();
    assert_eq!(real_json["success"], true);
    assert!(real_json["token"].as_str().unwrap().starts_with("plg_"));
}

/// `POST /api/command` and `/ws` used to execute any command from any caller
/// with no authentication at all, unless it carried a paired-device token
/// (a pre-existing console-auth gap). Both now require `host_session_token`
/// for callers with no device token -- this proves the gate actually blocks
/// execution (not just
/// returns an error status) on both entry points.
#[tokio::test]
async fn test_command_endpoint_requires_host_token_for_unpaired_callers() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_command_host_token.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(5000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host_token = "test_host_session_command".to_string();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: host_token.clone(),
        server_port: addr.port(),
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // POST /api/internal/verify-host-token: no loopback restriction (unlike
    // GET /api/internal/host-token), just a yes/no against the real token --
    // this is how a remote console confirms a token it was handed
    // out-of-band (printed to the server's terminal, or pasted manually).
    let resp = client.post(format!("http://{}/api/internal/verify-host-token", addr))
        .json(&serde_json::json!({ "token": host_token }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["valid"], true);
    let resp = client.post(format!("http://{}/api/internal/verify-host-token", addr))
        .json(&serde_json::json!({ "token": "not-the-real-token" }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["valid"], false);

    // HTTP: no token at all -- rejected, state unchanged.
    let resp = client.post(format!("http://{}/api/command", addr))
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["state"]["state"]["is_blackout"], false);

    // HTTP: wrong token -- also rejected.
    let resp = client.post(format!("http://{}/api/command", addr))
        .header("x-host-token", "not-the-real-token")
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["state"]["state"]["is_blackout"], false);

    // HTTP: the real host token but no console_session_id -- still rejected
    // (a distinct "missing x-console-session-id" reason, not a state change).
    let resp = client.post(format!("http://{}/api/command", addr))
        .header("x-host-token", &host_token)
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["state"]["state"]["is_blackout"], false);

    // HTTP: the real host token + a session id -- executes.
    let resp = client.post(format!("http://{}/api/command", addr))
        .header("x-host-token", &host_token)
        .header("x-console-session-id", "session-a")
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["state"]["state"]["is_blackout"], true);

    // WS: a message with no device_token and no (or a wrong) host_token
    // must not execute either -- connect, send both bad variants, then
    // confirm via a fresh HTTP snapshot that blackout is still on from the
    // HTTP toggle above (i.e. an unauthenticated WS ToggleBlackout did not
    // flip it back off).
    use futures_util::{SinkExt, StreamExt};
    let (mut ws_stream, _) = tokio_tungstenite::connect_async(format!("ws://{}/ws", addr)).await.unwrap();
    let _initial_snapshot = ws_stream.next().await; // server sends one on connect

    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout" }).to_string().into(),
    )).await.unwrap();
    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "host_token": "not-the-real-token", "console_session_id": "session-a" }).to_string().into(),
    )).await.unwrap();

    // Give the server a moment to (not) process those, then verify via HTTP
    // that state didn't change -- a broadcast never arrives on this socket
    // either way since a rejected message never calls broadcast_snapshot.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], true, "unauthenticated/wrong-token WS commands must not execute");

    // WS: the real host token but no console_session_id -- also rejected.
    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "host_token": host_token }).to_string().into(),
    )).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], true, "a WS command missing console_session_id must not execute");

    // WS: the real host token + the same session id already holding the
    // lock from the HTTP call above -- executes (toggles back off).
    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "host_token": host_token, "console_session_id": "session-a" }).to_string().into(),
    )).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], false, "a WS command with the real host token and matching session id must execute");
}

/// Library CRUD-delete, schedule save, settings, and the local-update-apply
/// routes all now require the host token, same as `/api/command`/`/ws` --
/// previously any LAN caller could delete the whole song/media/theme/
/// scripture library, overwrite an arbitrary file via `/api/schedule/save`,
/// or make the console open an arbitrary local file via
/// `/api/updates/apply-local`, with zero authentication. This does not
/// exercise `apply_local_update`'s success path (it would try to actually
/// open a file with the OS's default handler) -- only that an
/// unauthenticated/wrong-token call is rejected before it gets that far.
#[tokio::test]
async fn test_library_mutation_and_admin_routes_require_host_token() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_library_auth.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::new(100));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host_token = "test_host_session_library_auth".to_string();

    let song = os_next::core::models::Song::new("Auth Test Song", "Test Author");
    db.insert_song(&song).unwrap();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: test_dir.path().to_path_buf(),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: host_token.clone(),
        server_port: addr.port(),
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // DELETE /api/songs/:id -- no token, wrong token, then the real one.
    let resp = client.delete(format!("http://{}/api/songs/{}", addr, song.id)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let resp = client.delete(format!("http://{}/api/songs/{}", addr, song.id))
        .header("x-host-token", "wrong")
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(db.get_songs().unwrap().len(), 1, "unauthenticated caller must not be able to delete a song");
    let resp = client.delete(format!("http://{}/api/songs/{}", addr, song.id))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(db.get_songs().unwrap().len(), 0);

    // POST /api/schedule/save -- no token is rejected before any file is written.
    let save_path = test_dir.path().join("unauthorized_save.ewsx");
    let resp = client.post(format!("http://{}/api/schedule/save", addr))
        .json(&serde_json::json!({ "path": save_path.to_str().unwrap() }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(!save_path.exists(), "unauthenticated caller must not be able to write an arbitrary file via schedule save");
    let resp = client.post(format!("http://{}/api/schedule/save", addr))
        .header("x-host-token", &host_token)
        .json(&serde_json::json!({ "path": save_path.to_str().unwrap() }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert!(save_path.exists());

    // POST /api/settings -- no token is rejected.
    let resp = client.post(format!("http://{}/api/settings", addr))
        .json(&serde_json::json!({ "churchName": "Attacker Name" }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_ne!(
        db.get_settings().unwrap().get("churchName").map(|s| s.as_str()),
        Some("Attacker Name"),
        "unauthenticated caller must not be able to change settings"
    );

    // POST /api/updates/apply-local -- no token is rejected before it ever
    // tries to hand the path to the OS's open-with-default-handler.
    let resp = client.post(format!("http://{}/api/updates/apply-local", addr))
        .json(&serde_json::json!({ "path": "/nonexistent/should-not-be-opened", "force": true }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // The automated check/download flow (GET /api/updates/status, POST
    // /api/updates/check, POST /api/updates/download-and-install) needs the
    // same host-token gate -- without it, any LAN caller could trigger a
    // real download-and-install, not just read status.
    let resp = client.get(format!("http://{}/api/updates/status", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let resp = client.post(format!("http://{}/api/updates/check", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let resp = client.post(format!("http://{}/api/updates/download-and-install", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Same gate for the yt-dlp self-updater's status (src/network/ytdlp_updater.rs)
    // -- it's read-only, but still shouldn't leak to an unpaired LAN caller.
    let resp = client.get(format!("http://{}/api/ytdlp-updater/status", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // ADB TV-provisioning routes -- without this, any LAN caller could
    // trigger a real adb install/launch sequence against whatever IP it
    // supplied, or read another operator's in-progress provisioning status.
    let resp = client.get(format!("http://{}/api/tv-provision/status", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let resp = client.post(format!("http://{}/api/tv-provision/start", addr))
        .json(&serde_json::json!({ "ip": "192.0.2.1" }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let resp = client.get(format!("http://{}/api/tv-provision/progress/nonexistent", addr)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

/// Real, authenticated exercise of the ADB provisioning flow's actual
/// `adb` invocation -- targets `127.0.0.1` (loopback, nothing listening on
/// the ADB port in a test environment) rather than mocking `adb` out, so
/// this proves the real subprocess plumbing (spawn, argument construction,
/// the "adb connect exits 0 even on failure" quirk `network::adb::provision`
/// specifically works around) rather than just the HTTP plumbing around it.
/// Doesn't assert *which* failure reason (no `adb` binary vs. connection
/// refused) since that depends on whether this machine has `adb` installed
/// -- only that it fails cleanly and reports a real error, never hangs.
#[tokio::test]
async fn test_tv_provision_start_and_progress_against_unreachable_device() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_tv_provision.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::new(100));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host_token = "test_host_session_tv_provision".to_string();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: host_token.clone(),
        server_port: addr.port(),
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // No bundled APK in this test harness -- resolve_tv_apk_path() finds
    // nothing next to the `cargo test` binary, so this correctly reports
    // apk_available: false rather than crashing.
    let resp = client.get(format!("http://{}/api/tv-provision/status", addr))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["apk_available"], false, "no APK is bundled in the test harness, expected {:?}", body);

    // POST /api/tv-provision/start correctly refuses when there's no APK to
    // install, rather than starting a task that could only ever fail later.
    let resp = client.post(format!("http://{}/api/tv-provision/start", addr))
        .header("x-host-token", &host_token)
        .json(&serde_json::json!({ "ip": "127.0.0.1" }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE, "expected a clean refusal with no APK bundled");
}

/// Real, authenticated end-to-end coverage of the automated update routes
/// against the live public test repo (`crate::network::updater::RELEASES_REPO`)
/// -- `status` starts empty, `check` does a real GitHub call and populates
/// it, and a second `status` read reflects that without hitting the network
/// again.
///
/// `#[ignore]`d -- see `downloads_and_verifies_the_real_test_repo_release`
/// in `src/network/updater.rs` for why: this and that test together were
/// the dominant source of real GitHub API calls during this feature's own
/// development, enough to trip a secondary rate limit mid-session. Run
/// explicitly with `cargo test --test core_tests -- --ignored` when
/// actually verifying the live path.
#[tokio::test]
#[ignore]
async fn test_update_check_and_status_routes_against_real_test_repo() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_update_routes.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::new(100));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host_token = "test_host_session_update_routes".to_string();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: host_token.clone(),
        server_port: addr.port(),
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // Nothing's checked yet -- no background task in this harness (that's
    // only started from `main.rs`), so status starts as null.
    let resp = client.get(format!("http://{}/api/updates/status", addr))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["status"].is_null(), "expected no check to have run yet, got {:?}", body);

    // A real check against the real test repo.
    let resp = client.post(format!("http://{}/api/updates/check", addr))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["status"]["error"].is_null(), "expected a real check to succeed, got {:?}", body);
    assert!(body["status"]["current_version"].is_string());

    // Status now reflects that check, without another network call.
    let resp = client.get(format!("http://{}/api/updates/status", addr))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(!body["status"].is_null(), "expected status to be populated after a check");

    // A second POST /api/updates/check right away must NOT make another
    // real GitHub call -- MIN_SECONDS_BETWEEN_REAL_CHECKS in routes.rs
    // returns the still-fresh cached result instead (`throttled: true`).
    // This is exactly the behavior that keeps an operator mashing "Check
    // for Updates" from hammering GitHub's API.
    let resp = client.post(format!("http://{}/api/updates/check", addr))
        .header("x-host-token", &host_token)
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["throttled"], true, "a second check within the cooldown should be throttled, got {:?}", body);
}

/// "One console at a time, first one connected wins" (docs/CLIENT_PAIRING.md).
/// A second console (different `console_session_id`) presenting the correct
/// host token must still be rejected with the distinct `console_locked`
/// reason while a first console holds the lock -- and once the first one's
/// WS connection closes, the lock frees up for the next one.
#[tokio::test]
async fn test_only_one_console_connected_at_a_time() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_console_lock.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(6000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host_token = "test_host_session_lock".to_string();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: host_token.clone(),
        server_port: addr.port(),
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    use futures_util::{SinkExt, StreamExt};
    let client = reqwest::Client::new();

    // Console A connects over WS and claims the lock with its first command.
    let (mut ws_a, _) = tokio_tungstenite::connect_async(format!("ws://{}/ws", addr)).await.unwrap();
    let _ = ws_a.next().await; // initial snapshot
    ws_a.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "host_token": host_token, "console_session_id": "console-a" }).to_string().into(),
    )).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr)).send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], true, "console A's first command should claim the lock and execute");

    // Console B (different session id, correct host token) is locked out --
    // via HTTP, with the distinct "console_locked" error surfaced.
    let resp = client.post(format!("http://{}/api/command", addr))
        .header("x-host-token", &host_token)
        .header("x-console-session-id", "console-b")
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["error"], "console_locked");
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr)).send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], true, "console B must not have executed while A holds the lock");

    // Console A can keep issuing commands (same session id it already holds
    // the lock for) while B remains locked out.
    ws_a.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "cmd": "ToggleBlackout", "host_token": host_token, "console_session_id": "console-a" }).to_string().into(),
    )).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let state: serde_json::Value = client.get(format!("http://{}/api/state", addr)).send().await.unwrap().json().await.unwrap();
    assert_eq!(state["state"]["is_blackout"], false, "console A should still be able to issue further commands");

    // Console A disconnects -- releasing the lock -- so console B can now claim it.
    drop(ws_a);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let resp = client.post(format!("http://{}/api/command", addr))
        .header("x-host-token", &host_token)
        .header("x-console-session-id", "console-b")
        .json(&serde_json::json!({ "ToggleBlackout": null }))
        .send().await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true, "console B should be able to claim the lock once A disconnects");
}

/// Confirms the router itself behaves correctly across both listeners --
/// zero-auth routes reachable on each, the HTTPS plane accepting the
/// self-signed cert, and `GET /api/network/info` always preferring https://
/// URLs when available (`remote_url`/`mdns_url`/`pairing_url` -- since
/// `src/main.rs`, the cleartext plane is never network-reachable at all, so
/// every URL handed to a non-loopback caller must be https). The actual
/// production loopback-only (127.0.0.1) vs network-wide (0.0.0.0) bind
/// split lives in `src/main.rs`, not exercised here -- this test builds its
/// own listeners directly against the router, both already on loopback by
/// construction, which is the right level for what this test actually
/// checks.
#[tokio::test]
async fn test_split_plane_http_https_listeners_and_zero_auth_displays() {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_split_plane.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();
    let event_log = Arc::new(EventLog::with_initial_sequence(4000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let asset_graph = Arc::new(os_next::media::AssetGraph::new(test_dir.path().join("media")));
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    // 1. Generate TLS certificate
    let interfaces = os_next::network::NetworkInterfaceInfo::discover_all();
    let (cert_pem, key_pem) = os_next::network::tls::get_or_create_tls_certificate(&db, "test-sanctuary", &interfaces).unwrap();
    assert!(cert_pem.contains("BEGIN CERTIFICATE"));
    assert!(key_pem.contains("PRIVATE KEY"));

    // 2. Bind cleartext HTTP listener
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();
    let http_port = http_addr.port();

    // 3. Bind HTTPS listener port
    let https_probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let https_port = https_probe.local_addr().unwrap().port();
    drop(https_probe);

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: "test_host_session_split".to_string(),
        server_port: http_port,
        https_port: Some(https_port),
        https_enabled: true,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);

    // Spawn cleartext HTTP listener
    let http_router = router.clone();
    tokio::spawn(async move {
        let _ = axum::serve(http_listener, http_router).await;
    });

    // Spawn secure HTTPS listener
    let rustls_config = os_next::network::tls::create_rustls_config(&cert_pem, &key_pem).await.unwrap();
    let https_router = router.clone();
    let https_addr = std::net::SocketAddr::from(([127, 0, 0, 1], https_port));
    tokio::spawn(async move {
        let _ = axum_server::bind_rustls(https_addr, rustls_config)
            .serve(https_router.into_make_service())
            .await;
    });

    // Wait briefly for servers to listen
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Standard cleartext HTTP client
    let http_client = reqwest::Client::new();

    // TLS-capable client accepting our self-signed cert
    let cert = reqwest::Certificate::from_pem(cert_pem.as_bytes()).unwrap();
    let https_client = reqwest::Client::builder()
        .add_root_certificate(cert)
        .build()
        .unwrap();

    // 4. Verify cleartext HTTP /api/network/info reports both ports
    let net_info_resp = http_client.get(format!("http://{}/api/network/info", http_addr))
        .send().await.unwrap();
    assert_eq!(net_info_resp.status(), reqwest::StatusCode::OK);
    let net_info: serde_json::Value = net_info_resp.json().await.unwrap();
    assert_eq!(net_info["success"], true);
    assert_eq!(net_info["http_port"], http_port);
    assert_eq!(net_info["https_port"], https_port);
    assert_eq!(net_info["https_enabled"], true);
    assert!(net_info["pairing_url"].as_str().unwrap().starts_with("https://"));
    assert!(net_info["pairing_url"].as_str().unwrap().contains(&format!(":{}", https_port)));
    // remote_url/mdns_url: the cleartext plane is never network-reachable
    // (src/main.rs loopback-bind), so these must prefer https:// too, same
    // as pairing_url already did.
    assert!(net_info["remote_url"].as_str().unwrap().starts_with("https://"));
    assert!(net_info["remote_url"].as_str().unwrap().contains(&format!(":{}", https_port)));
    assert!(net_info["mdns_url"].as_str().unwrap().starts_with("https://"));
    assert!(net_info["mdns_url"].as_str().unwrap().contains(&format!(":{}", https_port)));

    // 5. Zero-Auth AV Plane: /api/state, /remote on HTTP
    let state_resp = http_client.get(format!("http://{}/api/state", http_addr))
        .send().await.unwrap();
    assert_eq!(state_resp.status(), reqwest::StatusCode::OK);

    // 6. Secure HTTPS Plane: /api/state and /api/pairing/session over https://
    let secure_state = https_client.get(format!("https://127.0.0.1:{}/api/state", https_port))
        .send().await.unwrap();
    assert_eq!(secure_state.status(), reqwest::StatusCode::OK);

    let pair_sess = https_client.post(format!("https://127.0.0.1:{}/api/pairing/session", https_port))
        .header("x-host-token", "test_host_session_split")
        .json(&serde_json::json!({ "ttl_seconds": 60 }))
        .send().await.unwrap();
    assert_eq!(pair_sess.status(), reqwest::StatusCode::OK);
    let pair_json: serde_json::Value = pair_sess.json().await.unwrap();
    assert!(pair_json["session_token"].as_str().unwrap().starts_with("pair_sess_"));
    assert_eq!(pair_json["expires_in"], 300);

    // 7. Test POST /api/network/tls/regenerate
    let regen_resp = http_client.post(format!("http://{}/api/network/tls/regenerate", http_addr))
        .header("x-host-token", "test_host_session_split")
        .send().await.unwrap();
    assert_eq!(regen_resp.status(), reqwest::StatusCode::OK);
    let regen_json: serde_json::Value = regen_resp.json().await.unwrap();
    assert_eq!(regen_json["success"], true);
}

#[tokio::test]
async fn test_security_headers_and_cors_policies() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test_sec.db");
    let db = os_next::storage::Database::new(db_path.to_str().unwrap()).unwrap();

    // 1. Verify get_setting / set_setting on Database
    assert_eq!(db.get_setting("non_existent_key").unwrap(), None);
    db.set_setting("test_key", "test_value").unwrap();
    assert_eq!(db.get_setting("test_key").unwrap(), Some("test_value".to_string()));

    let event_log = Arc::new(EventLog::new(100));
    let engine = Arc::new(ShowEngine::new(event_log));
    let asset_graph = Arc::new(AssetGraph::new(temp_dir.path().join("assets")));
    let (tx, _) = tokio::sync::broadcast::channel(100);
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: std::path::PathBuf::from("web"),
        media_dir: std::path::PathBuf::from("web"),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: "test_host_session_sec".to_string(),
        server_port: http_addr.port(),
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);

    tokio::spawn(async move {
        let _ = axum::serve(http_listener, router).await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    let client = reqwest::Client::new();

    // 2. Default Configuration: Balanced CSP, SAMEORIGIN, Permissive CORS
    let resp = client.get(format!("http://{}/api/state", http_addr))
        .header("Origin", "http://any-display.sanctuary.lan:8080")
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let headers = resp.headers();
    assert_eq!(
        headers.get("x-content-type-options").unwrap().to_str().unwrap(),
        "nosniff"
    );
    assert_eq!(
        headers.get("x-frame-options").unwrap().to_str().unwrap(),
        "SAMEORIGIN"
    );
    let csp = headers.get("content-security-policy").unwrap().to_str().unwrap();
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("frame-ancestors 'self'"));
    assert!(csp.contains("media-src 'self' data: blob: https: http:")); // Allows sanctuary HTTP media in balanced mode
    assert!(headers.get("access-control-allow-origin").is_some());

    // 3. Reconfigure to Strict CSP, Deny Framing, and Restricted CORS --
    // through the real POST /api/settings endpoint (host-token gated), not
    // a direct `db.set_setting`: the security headers/CORS predicate now
    // read an in-memory cache that only `post_settings` refreshes, not the
    // database directly (see `AppState::security_header_settings`).
    let reconf_resp = client.post(format!("http://{}/api/settings", http_addr))
        .header("x-host-token", "test_host_session_sec")
        .json(&serde_json::json!({
            "securityCspMode": "strict",
            "securityFrameOptions": "deny",
            "securityCorsMode": "restricted",
        }))
        .send().await.unwrap();
    assert_eq!(reconf_resp.status(), reqwest::StatusCode::OK);

    let resp_strict = client.get(format!("http://{}/api/state", http_addr))
        .header("Origin", "http://192.168.1.100:8080")
        .send().await.unwrap();
    assert_eq!(resp_strict.status(), reqwest::StatusCode::OK);

    let h_strict = resp_strict.headers();
    assert_eq!(
        h_strict.get("x-frame-options").unwrap().to_str().unwrap(),
        "DENY"
    );
    let csp_strict = h_strict.get("content-security-policy").unwrap().to_str().unwrap();
    assert!(csp_strict.contains("frame-ancestors 'none'"));
    assert!(csp_strict.contains("form-action 'self'"));
    // Origin 192.168.1.100 is in private RFC 1918 range -> allowed
    assert_eq!(
        h_strict.get("access-control-allow-origin").unwrap().to_str().unwrap(),
        "http://192.168.1.100:8080"
    );

    // 4. Restricted CORS: Public / external origin should be rejected
    let resp_rejected = client.get(format!("http://{}/api/state", http_addr))
        .header("Origin", "https://malicious-attacker-site.com")
        .send().await.unwrap();
    assert_eq!(resp_rejected.status(), reqwest::StatusCode::OK);
    assert!(resp_rejected.headers().get("access-control-allow-origin").is_none());

    // 5. Reconfigure to Disabled CSP and Disabled Framing (same reasoning
    // as step 3 -- through the real endpoint, not the database directly)
    let disable_resp = client.post(format!("http://{}/api/settings", http_addr))
        .header("x-host-token", "test_host_session_sec")
        .json(&serde_json::json!({
            "securityCspMode": "disabled",
            "securityFrameOptions": "disabled",
        }))
        .send().await.unwrap();
    assert_eq!(disable_resp.status(), reqwest::StatusCode::OK);

    let resp_disabled = client.get(format!("http://{}/api/state", http_addr))
        .send().await.unwrap();
    assert_eq!(resp_disabled.status(), reqwest::StatusCode::OK);
    assert!(resp_disabled.headers().get("content-security-policy").is_none());
    assert!(resp_disabled.headers().get("x-frame-options").is_none());
    assert_eq!(
        resp_disabled.headers().get("x-content-type-options").unwrap().to_str().unwrap(),
        "nosniff"
    );
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

/// Builds a minimal real AppState + router for the check-public-url tests
/// below, bound to an ephemeral loopback port. Mirrors the construction
/// `test_freeshow_import_http_route` already uses elsewhere in this file.
async fn spawn_test_app(host_session_token: &str) -> String {
    let test_dir = tempfile::tempdir().unwrap();
    let db_path = test_dir.path().join("test_public_url.db");
    let db = Database::new(&db_path).expect("Database::new");
    let web_dir = test_dir.path().join("web");
    std::fs::create_dir_all(&web_dir).unwrap();

    let event_log = Arc::new(EventLog::with_initial_sequence(3000, 0));
    let engine = Arc::new(ShowEngine::new(event_log));
    let asset_graph = Arc::new(AssetGraph::new(&web_dir));
    let (tx, _rx) = tokio::sync::broadcast::channel(512);
    let plugin_manager = Arc::new(os_next::core::plugins::PluginManager::new());
    let bible_providers = os_next::storage::BibleProviderRegistry::new_default();
    let media_search = Arc::new(os_next::storage::MediaSearchRegistry::new_default());

    let app_state = os_next::api::ws::AppState {
        tx,
        engine,
        db: db.clone(),
        asset_graph,
        web_dir: web_dir.clone(),
        media_dir: web_dir.clone(),
        data_dir: std::path::PathBuf::from("."),
        skip_first_time_setup: true,
        plugin_manager,
        bible_providers,
        media_search,
        last_broadcast_schedule_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        display_manager: None,
        plugin_tokens: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        host_session_token: host_session_token.to_string(),
        server_port: 8080,
        https_port: None,
        https_enabled: false,
        pairing_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        active_console: Arc::new(std::sync::Mutex::new(None)),
        pending_pairing_token_delivery: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        security_header_settings: Arc::new(std::sync::RwLock::new(Default::default())),
        update_status: Arc::new(std::sync::Mutex::new(None)),
        ytdlp_update_status: Arc::new(std::sync::Mutex::new(None)),
    };

    let router = os_next::api::create_router(app_state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{}", addr)
}

#[tokio::test]
async fn test_check_public_url_requires_host_token() {
    let base = spawn_test_app("real_token").await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/network/check-public-url?url=https://connect.example.org", base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_check_public_url_rejects_non_https_scheme_without_a_network_call() {
    // `url=http://...` (or any malformed URL) must be rejected by
    // validate_https_url before the handler ever attempts an outbound
    // request -- this test would hang/timeout if that ordering regressed,
    // since `example.invalid` resolves to nothing.
    let base = spawn_test_app("real_token").await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/network/check-public-url?url=http://example.invalid", base))
        .header("x-host-token", "real_token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["reachable"], false);
    assert!(body["message"].as_str().unwrap().contains("https://"));
}




// Inputs found by `fuzz/` (see docs/FUZZING.md) that used to panic; each must
// now return normally. Fixtures are the raw libFuzzer crash artifacts.
#[test]
fn test_fuzz_regressions_do_not_panic() {
    let ewsx: &[u8] = include_bytes!("fixtures/fuzz_regressions/ewsx_import.bin");
    let _ = EwsxManager::load_schedule_from_bytes(ewsx, "fuzz");

    let theme = String::from_utf8_lossy(include_bytes!("fixtures/fuzz_regressions/openlp_theme.bin")).into_owned();
    let _ = OpenLPImporter::parse_openlp_theme_xml(&theme, None, None);

    let rtf = String::from_utf8_lossy(include_bytes!("fixtures/fuzz_regressions/rtf_parse.bin")).into_owned();
    let _ = os_next::storage::rtf::parse_rtf(&rtf);

    let scripture = String::from_utf8_lossy(include_bytes!("fixtures/fuzz_regressions/scripture_ref.bin")).into_owned();
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
