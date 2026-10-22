//! The library database, Bible/scripture storage and lookup, data-directory layout, media folder sync.

use super::*;

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
        slides: vec![Slide { text: "Bless the Lord O my soul".to_string(), ..Default::default() }],
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
fn test_asset_graph_resolves_asset_uris_under_its_root() {
    let tmp = tempfile::tempdir().unwrap();
    let graph = AssetGraph::new(tmp.path());
    assert_eq!(graph.resolve_uri("asset://videos/worship.mp4"), tmp.path().join("videos/worship.mp4"));
}

#[test]
fn test_a_fresh_library_is_seeded_with_default_themes() {
    // What a brand-new install gets (Database::new seeds; in_memory deliberately doesn't). In a temp dir:
    // this used to open "library.db" in the working directory, creating or modifying a real library in the
    // repo root on every run -- and passing only because of whatever that file already held.
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::new(tmp.path().join("library.db")).unwrap();
    assert!(!db.get_themes().unwrap().is_empty());
    // Reopening an existing library doesn't seed a second copy.
    let count = db.get_themes().unwrap().len();
    drop(db);
    let reopened = Database::new(tmp.path().join("library.db")).unwrap();
    assert_eq!(reopened.get_themes().unwrap().len(), count);
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
        slides: vec![Slide { text: "Every blessing".to_string(), ..Default::default() }],
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
        slides: vec![Slide { text: "Amazing grace how sweet the sound".to_string(), ..Default::default() }],
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
        slides: vec![Slide { text: "Bless the Lord O my soul".to_string(), ..Default::default() }],
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

#[test]
fn test_ccli_report_counts_a_song_once_per_time_it_is_put_up() {
    use os_next::models::CcliMetadata;
    // Drive the real engine through a short service and save its events exactly as the command pipeline
    // does, then read the report: it's what a church sends to CCLI, so every count has to be a real use.
    let db = Database::in_memory().unwrap();
    let engine = ShowEngine::new(Arc::new(EventLog::new(1000)));
    let run = |cmd: ShowCommand| for env in engine.execute_command(cmd) { db.save_event(&env).unwrap(); };
    let song = |id: &str, title: &str, ccli: &str| ScheduleItem {
        item_type: "song".into(),
        slides: ["v1", "c1", "v2"].iter().map(|t| Slide {
            text: t.to_string(),
            ccli_metadata: Some(CcliMetadata { song_title: Some(title.into()), ccli_number: Some(ccli.into()), copyright: Some("© Publisher".into()), ..Default::default() }),
            ..Default::default()
        }).collect(),
        ..item(id, title)
    };
    run(ShowCommand::AddToSchedule(song("a", "Song A", "1111111")));
    run(ShowCommand::AddToSchedule(song("b", "Song B", "2222222")));

    run(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(0) }); // Song A goes up
    run(ShowCommand::NextSlide);
    run(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(2) }); // operator clicks a later slide of the same song
    run(ShowCommand::ToggleBlackout);
    run(ShowCommand::Undo); // ...and undoes the blackout
    run(ShowCommand::NextItem); // Song B goes up
    run(ShowCommand::Undo); // ...undone straight away: back on Song A, which isn't a new use either
    run(ShowCommand::Redo); // and redone: Song B wasn't put up a second time

    let report = db.get_ccli_usage_report(0, i64::MAX).unwrap();
    let uses = |t: &str| report.iter().find(|r| r.title == t).map(|r| r.use_count);
    assert_eq!(uses("Song A"), Some(1), "jumping within the live song, or Undo/Redo, isn't another use");
    assert_eq!(uses("Song B"), Some(1));

    // Genuinely going back to a song later in the service is a second use.
    run(ShowCommand::PrevItem);
    let report = db.get_ccli_usage_report(0, i64::MAX).unwrap();
    assert_eq!(report.iter().find(|r| r.title == "Song A").map(|r| r.use_count), Some(2));
}

#[test]
fn test_opening_a_pre_split_library_migrates_songs_scriptures_and_themes() {
    // An install from before songs/ and bibles/ were split out (and before the flat-CSS Theme model) kept
    // everything in one library.db. Opening it must move each kind of content to where it lives now,
    // convert the old theme columns, and leave nothing to re-migrate the next time.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("library.db");
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(r#"
            CREATE TABLE songs (id TEXT PRIMARY KEY, title TEXT, author TEXT, copyright TEXT, ccli_number TEXT, slides_json TEXT, theme_name TEXT);
            INSERT INTO songs VALUES ('pd1', 'Amazing Grace', 'John Newton', 'Public Domain', NULL, '[{"text":"Amazing grace","header":null,"label":"V1","background":null}]', NULL);
            INSERT INTO songs VALUES ('cr1', 'Modern Song', 'Writer', '© 2020 Publisher', '7654321', '[{"text":"Line one","header":null,"label":"V1","background":null}]', NULL);
            CREATE TABLE scriptures (id TEXT PRIMARY KEY, book TEXT, chapter INTEGER, verse_start INTEGER, verse_end INTEGER, version TEXT, reference TEXT, verses_json TEXT, theme_name TEXT);
            INSERT INTO scriptures VALUES ('j3', 'John', 3, 16, 16, 'KJV', 'John 3:16', '[{"verse_number":16,"text":"For God so loved the world"}]', NULL);
            CREATE TABLE themes (name TEXT PRIMARY KEY, background_value TEXT, font_family TEXT, font_size INTEGER, font_color TEXT, alignment TEXT,
                                 shadow INTEGER, shadow_color TEXT, margin_top INTEGER, margin_bottom INTEGER, margin_left TEXT, margin_right INTEGER, vertical_alignment TEXT);
            INSERT INTO themes VALUES ('Legacy Blue', '#123456', 'Georgia', 48, '#ffffff', 'center', 1, '#111111', 20, 20, '10%', 20, 'top');
        "#).unwrap();
    }

    let db = Database::new(&path).unwrap();
    let songs = db.get_songs().unwrap();
    let titles: Vec<&str> = songs.iter().map(|s| s.title.as_str()).collect();
    assert!(titles.contains(&"Amazing Grace") && titles.contains(&"Modern Song"), "songs after migration: {titles:?}");
    assert_eq!(songs.iter().find(|s| s.id == "cr1").unwrap().ccli_number.as_deref(), Some("7654321"));
    assert_eq!(songs.iter().find(|s| s.id == "pd1").unwrap().slides[0].text, "Amazing grace");
    let john = db.get_scriptures().unwrap().into_iter().find(|s| s.book == "John" && s.chapter == 3).expect("John 3 migrated");
    assert_eq!(john.verses[0].text, "For God so loved the world");
    assert!(db.get_installed_bibles().unwrap().iter().any(|b| b.id.eq_ignore_ascii_case("KJV")));

    let legacy = db.get_themes().unwrap().into_iter().find(|t| t.name == "Legacy Blue").expect("legacy theme kept");
    assert_eq!(legacy.background, "#123456");
    assert_eq!(legacy.font_size, "48px", "integer px sizes become CSS lengths");
    assert_eq!(legacy.text_shadow, "2px 2px 8px #111111", "the old shadow flag + colour become a text-shadow");
    assert_eq!((legacy.margin_top.as_str(), legacy.margin_left.as_str()), ("20px", "10%"));
    assert_eq!(legacy.vertical_align, "top");

    let tables = |c: &rusqlite::Connection| -> Vec<String> {
        c.prepare("SELECT name FROM sqlite_master WHERE type='table'").unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect()
    };
    let main = rusqlite::Connection::open(&path).unwrap();
    let t = tables(&main);
    assert!(!t.contains(&"songs".to_string()) && !t.contains(&"scriptures".to_string()), "legacy tables left behind: {t:?}");
    assert!(t.contains(&"themes_legacy_backup".to_string()), "the pre-migration themes are kept as a backup");
    drop(main);

    // Reopening is a no-op: same songs, same themes, no duplicates.
    let (n_songs, n_themes) = (songs.len(), db.get_themes().unwrap().len());
    drop(db);
    let again = Database::new(&path).unwrap();
    assert_eq!((again.get_songs().unwrap().len(), again.get_themes().unwrap().len()), (n_songs, n_themes));
}

#[test]
fn test_a_legacy_song_that_cant_be_migrated_keeps_the_legacy_table() {
    // Regression: the migration ignored read and insert failures and then dropped the legacy table anyway,
    // silently deleting whatever hadn't made it across.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("library.db");
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(r#"
            CREATE TABLE songs (id TEXT PRIMARY KEY, title TEXT, author TEXT, copyright TEXT, ccli_number TEXT, slides_json TEXT, theme_name TEXT);
            INSERT INTO songs VALUES ('ok', 'Readable', 'A', 'Public Domain', NULL, '[{"text":"x","header":null,"label":null,"background":null}]', NULL);
            INSERT INTO songs VALUES ('bad', 'Unreadable', 'B', 'Public Domain', NULL, NULL, NULL);
        "#).unwrap();
    }
    let db = Database::new(&path).unwrap();
    assert!(db.get_songs().unwrap().iter().any(|s| s.id == "ok"), "the readable song still migrates");
    drop(db);
    let c = rusqlite::Connection::open(&path).unwrap();
    let kept: i64 = c.query_row("SELECT COUNT(*) FROM songs", [], |r| r.get(0)).unwrap();
    assert_eq!(kept, 2, "the legacy table (and the row that couldn't move) must still be there");
}
