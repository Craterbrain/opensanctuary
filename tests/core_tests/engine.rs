//! The show engine: commands -> events -> state, live/stage navigation, arrangements, the slide editor, undo/redo, schedule edits.

use super::*;

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
        item_type: "song".to_string(),
        author_or_ref: "Author 1".to_string(),
        slides: vec![
            Slide { text: "Slide 1".to_string(), ..Default::default() },
            Slide { text: "Slide 2".to_string(), ..Default::default() },
        ],
        ..item("item1", "Song 1")
    };

    let item2 = ScheduleItem {
        item_type: "song".to_string(),
        author_or_ref: "Author 2".to_string(),
        slides: vec![Slide { text: "Only Slide".to_string(), ..Default::default() }],
        ..item("item2", "Song 2")
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
        item_type: "song".to_string(),
        author_or_ref: "Author".to_string(),
        slides: vec![
            Slide { text: "Slide 1".to_string(), header: None, label: Some("V1".to_string()), background: None, notes: None, tag: Some("V1".to_string()), ..Default::default() },
            Slide { text: "Slide 2".to_string(), header: None, label: Some("C1".to_string()), background: None, notes: None, tag: Some("C1".to_string()), ..Default::default() },
        ],
        arrangement: vec![
            ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
            ArrangementEntry { section_id: "C1".to_string(), source_slide_index: 1, background_override: Some("custom_bg.jpg".to_string()) },
            ArrangementEntry { section_id: "V1".to_string(), source_slide_index: 0, background_override: None },
        ],
        ..item("sched_1", "Test Arrangement Song")
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
        item_type: "song".to_string(),
        author_or_ref: "Author".to_string(),
        slides: vec![initial_slide],
        ..item("item_batch", "Batch Song")
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
        item_type: "song".to_string(),
        author_or_ref: "Author".to_string(),
        slides: vec![initial_slide],
        ..item("item_bg", "BG Song")
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
        item_type: "song".to_string(),
        author_or_ref: "Author".to_string(),
        slides: vec![slide],
        ..item("item_reorder", "Reorder Song")
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
        item_type: "song".to_string(),
        author_or_ref: "Author".to_string(),
        slides: vec![slide],
        ..item("item_group", "Group Song")
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
        slides: vec![
            Slide { text: "Welcome".to_string(), header: None, label: None, background: None, notes: None, tag: Some("V1".to_string()), ..Default::default() },
            Slide { text: "Please stand".to_string(), header: None, label: None, background: None, notes: None, tag: Some("V2".to_string()), ..Default::default() },
        ],
        ..item("item1", "Announcement Part 1")
    };
    let item2 = ScheduleItem {
        slides: vec![
            Slide { text: "Offering".to_string(), header: None, label: None, background: None, notes: None, tag: Some("V1".to_string()), ..Default::default() },
        ],
        ..item("item2", "Announcement Part 2")
    };
    let item3 = ScheduleItem {
        item_type: "song".to_string(),
        slides: vec![Slide { text: "Amen".to_string(), ..Default::default() }],
        ..item("item3", "Closing Song")
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
    engine2.execute_command(ShowCommand::AddToSchedule(ScheduleItem { slides: vec![Slide { text: "A1".to_string(), ..Default::default() }], ..item("a", "A") }));
    engine2.execute_command(ShowCommand::AddToSchedule(ScheduleItem { slides: vec![Slide { text: "B1".to_string(), ..Default::default() }], ..item("b", "B") }));
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
        item_type: "header".to_string(),
        is_expanded: true,
        is_section_header: true,
        ..item("hdr", "Header")
    };
    let song = ScheduleItem {
        item_type: "song".to_string(),
        slides: vec![Slide { text: "Only".to_string(), ..Default::default() }],
        ..item("song", "Song")
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

#[test]
fn test_out_of_range_go_live_and_stage_are_no_ops() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);
    engine.execute_command(ShowCommand::AddToSchedule(simple_item("a", "A", &["a1", "a2"])));
    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(1) });
    engine.execute_command(ShowCommand::StageItem { item_index: Some(0), slide_index: Some(0) });

    // A stale index (e.g. a paired remote that hasn't seen an item removal yet)
    // must not blank the live output or clear the staged item.
    let events = engine.execute_command(ShowCommand::GoLive { item_index: Some(99), slide_index: Some(0) });
    assert!(events.is_empty(), "out-of-range GoLive should emit nothing");
    let events = engine.execute_command(ShowCommand::StageItem { item_index: Some(1), slide_index: Some(0) });
    assert!(events.is_empty(), "out-of-range StageItem should emit nothing");

    let snap = engine.snapshot();
    assert_eq!(snap.state.live_item.as_ref().map(|i| i.id.as_str()), Some("a"));
    assert_eq!(snap.state.live_slide_index, 1);
    assert_eq!(snap.state.staged_item.as_ref().map(|i| i.id.as_str()), Some("a"));
}

#[test]
fn test_item_navigation_skips_section_headers() {
    let event_log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(event_log);
    engine.execute_command(ShowCommand::AddToSchedule(simple_item("a", "A", &["a1", "a2"])));
    engine.execute_command(ShowCommand::AddScheduleHeader { title: "Sermon".to_string(), index: None });
    engine.execute_command(ShowCommand::AddScheduleHeader { title: "Response".to_string(), index: None });
    engine.execute_command(ShowCommand::AddToSchedule(simple_item("b", "B", &["b1", "b2"])));
    engine.execute_command(ShowCommand::AddScheduleHeader { title: "End".to_string(), index: None });
    let live_id = |e: &ShowEngine| e.snapshot().state.live_item.map(|i| i.id);

    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(0) });
    engine.execute_command(ShowCommand::NextItem);
    assert_eq!(live_id(&engine).as_deref(), Some("b"), "NextItem steps over both headers");

    // Nothing but a header after B: NextItem / NextSlide off the end stay on B.
    engine.execute_command(ShowCommand::NextItem);
    assert_eq!(live_id(&engine).as_deref(), Some("b"));
    engine.execute_command(ShowCommand::JumpSlide(1));
    engine.execute_command(ShowCommand::NextSlide);
    assert_eq!(live_id(&engine).as_deref(), Some("b"));
    assert_eq!(engine.snapshot().state.live_slide_index, 1);

    engine.execute_command(ShowCommand::PrevItem);
    assert_eq!(live_id(&engine).as_deref(), Some("a"), "PrevItem steps over both headers");

    // NextSlide past A's last slide auto-advances to B, not the header in between;
    // PrevSlide from B's first slide goes back to A's last slide.
    engine.execute_command(ShowCommand::JumpSlide(1));
    engine.execute_command(ShowCommand::NextSlide);
    assert_eq!(live_id(&engine).as_deref(), Some("b"));
    assert_eq!(engine.snapshot().state.live_slide_index, 0);
    engine.execute_command(ShowCommand::PrevSlide);
    assert_eq!(live_id(&engine).as_deref(), Some("a"));
    assert_eq!(engine.snapshot().state.live_slide_index, 1);
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

// ---------------------------------------------------------------------------------------------
// The slide editor's commands, each one end to end: command -> event -> state, then Undo restores
// the slide exactly and Redo re-applies the same result. (Before this, only the batch path and
// group/z-order had tests; every direct element command was unexercised.)
// ---------------------------------------------------------------------------------------------

fn text_el(id: &str, text: &str, z: i32) -> SlideElement {
    use os_next::models::{ElementTransform, TextBlock, TextParagraphStyle, TextRun};
    SlideElement::TextBlock {
        id: id.to_string(),
        transform: ElementTransform { x: 0.1, y: 0.1 * z as f64, w: 0.8, h: 0.2, rotation_deg: 0.0, opacity: 1.0, z_index: z, locked: false },
        block: TextBlock {
            runs: vec![TextRun { text: text.to_string(), ..Default::default() }],
            paragraph_style: TextParagraphStyle::default(),
            effects: Default::default(),
            autofit: true,
        },
    }
}

/// A song whose first slide holds two text elements, `e1` and `e2`; runs `make(&that slide)` and checks
/// it as described above. `check` asserts the specific change on the edited slide.
fn assert_slide_edit_round_trips(name: &str, make: impl Fn(&Slide) -> ShowCommand, check: impl Fn(&Slide)) {
    let engine = ShowEngine::new(Arc::new(EventLog::new(100)));
    let slide = Slide { text: "Line one".into(), elements: vec![text_el("e1", "Line one", 1), text_el("e2", "Line two", 2)], ..Default::default() };
    engine.execute_command(ShowCommand::AddToSchedule(ScheduleItem { item_type: "song".into(), slides: vec![slide], ..item("s", "Song") }));
    let slide_now = |e: &ShowEngine| e.snapshot().schedule.items[0].slides[0].clone();
    let before = slide_now(&engine);

    let events = engine.execute_command(make(&before));
    assert!(!events.is_empty(), "{name}: emitted no event");
    let after = slide_now(&engine);
    assert_ne!(after, before, "{name}: changed nothing");
    check(&after);

    engine.execute_command(ShowCommand::Undo);
    assert_eq!(slide_now(&engine), before, "{name}: Undo didn't restore the slide exactly");
    engine.execute_command(ShowCommand::Redo);
    assert_eq!(slide_now(&engine), after, "{name}: Redo didn't re-apply the same edit");
}

#[test]
fn test_every_slide_editor_command_applies_and_undoes_exactly() {
    use os_next::core::commands::SlideEditOp;
    use os_next::models::{CcliMetadata, ElementEffects, ElementTransform, SlideTransition, TextRun};
    let moved = ElementTransform { x: 0.25, y: 0.5, w: 0.4, h: 0.1, rotation_deg: 15.0, opacity: 0.5, z_index: 1, locked: false };
    let edited_runs = vec![TextRun { text: "Edited".into(), bold: true, ..Default::default() }];
    let effects = ElementEffects { blend_mode: "multiply".into(), ..Default::default() };
    let ccli = CcliMetadata { song_title: Some("Song".into()), ccli_number: Some("1234567".into()), ..Default::default() };
    let el = |s: &Slide, id: &str| s.elements.iter().find(|e| e.id() == id).cloned();
    let runs_of = |e: Option<SlideElement>| match e { Some(SlideElement::TextBlock { block, .. }) => block.runs, _ => vec![] };
    let transform_of = |e: Option<SlideElement>| match e { Some(SlideElement::TextBlock { transform, .. }) => Some(transform), _ => None };
    let effects_of = |e: Option<SlideElement>| match e { Some(SlideElement::TextBlock { block, .. }) => Some(block.effects), _ => None };

    // Direct commands.
    assert_slide_edit_round_trips("AddSlideElement",
        |_| ShowCommand::AddSlideElement { item_index: 0, slide_index: 0, element: text_el("e3", "Added", 3), at_index: None },
        |s| assert_eq!(runs_of(el(s, "e3"))[0].text, "Added"));
    assert_slide_edit_round_trips("RemoveSlideElement",
        |_| ShowCommand::RemoveSlideElement { item_index: 0, slide_index: 0, element_id: "e1".into() },
        |s| { assert!(el(s, "e1").is_none()); assert!(el(s, "e2").is_some()); });
    assert_slide_edit_round_trips("UpdateSlideElementTransform",
        |_| ShowCommand::UpdateSlideElementTransform { item_index: 0, slide_index: 0, element_id: "e1".into(), transform: moved.clone() },
        |s| assert_eq!(transform_of(el(s, "e1")).map(|t| (t.x, t.y, t.rotation_deg)), Some((0.25, 0.5, 15.0))));
    assert_slide_edit_round_trips("UpdateTextBlockContent",
        |_| ShowCommand::UpdateTextBlockContent { item_index: 0, slide_index: 0, element_id: "e1".into(), runs: edited_runs.clone(), paragraph_style: None },
        |s| { let r = runs_of(el(s, "e1")); assert_eq!((r[0].text.as_str(), r[0].bold), ("Edited", true)); });
    assert_slide_edit_round_trips("UpdateElementEffects",
        |_| ShowCommand::UpdateElementEffects { item_index: 0, slide_index: 0, element_id: "e2".into(), effects: effects.clone() },
        |s| assert_eq!(effects_of(el(s, "e2")).map(|e| e.blend_mode), Some("multiply".to_string())));
    assert_slide_edit_round_trips("ReorderSlideElements",
        |_| ShowCommand::ReorderSlideElements { item_index: 0, slide_index: 0, element_id: "e1".into(), to_z: 9 },
        |s| { let z = |id| transform_of(el(s, id)).unwrap().z_index; assert!(z("e1") > z("e2"), "e1 should now be on top"); });
    assert_slide_edit_round_trips("GroupSlideElements",
        |_| ShowCommand::GroupSlideElements { item_index: 0, slide_index: 0, element_ids: vec!["e1".into(), "e2".into()] },
        |s| { assert_eq!(s.elements.len(), 1); assert!(matches!(&s.elements[0], SlideElement::Group { children, .. } if children.len() == 2)); });
    assert_slide_edit_round_trips("SetSlideBackgroundV2",
        |_| ShowCommand::SetSlideBackgroundV2 { item_index: Some(0), slide_index: 0, background: SlideBackground::Solid("#123456".into()) },
        |s| assert_eq!(s.background_v2, Some(SlideBackground::Solid("#123456".into()))));
    assert_slide_edit_round_trips("SetSlideTransition",
        |_| ShowCommand::SetSlideTransition { item_index: Some(0), slide_index: 0, transition: SlideTransition { kind: "crossfade".into(), duration_ms: 400 } },
        |s| assert_eq!(s.transition.as_ref().map(|t| (t.kind.as_str(), t.duration_ms)), Some(("crossfade", 400))));
    assert_slide_edit_round_trips("SetSlideSpeakerNotes",
        |_| ShowCommand::SetSlideSpeakerNotes { item_index: 0, slide_index: 0, notes: "Pause here".into() },
        |s| assert_eq!(s.speaker_notes.as_deref(), Some("Pause here")));
    assert_slide_edit_round_trips("SetSlideCcliMetadata",
        |_| ShowCommand::SetSlideCcliMetadata { item_index: 0, slide_index: 0, metadata: ccli.clone() },
        |s| assert_eq!(s.ccli_metadata.as_ref().and_then(|m| m.ccli_number.as_deref()), Some("1234567")));

    // The same edits through BatchSlideEdit (the editor's "Apply"), one op per batch.
    let batch = |ops: Vec<SlideEditOp>| move |_: &Slide| ShowCommand::BatchSlideEdit { item_index: 0, slide_index: 0, ops: ops.clone() };
    assert_slide_edit_round_trips("batch RemoveElement", batch(vec![SlideEditOp::remove_element("e2")]), |s| assert!(el(s, "e2").is_none()));
    assert_slide_edit_round_trips("batch UpdateTransform", batch(vec![SlideEditOp::update_transform("e2", moved.clone())]),
        |s| assert_eq!(transform_of(el(s, "e2")).map(|t| t.x), Some(0.25)));
    assert_slide_edit_round_trips("batch UpdateTextBlockContent", batch(vec![SlideEditOp::update_text_block_content("e2", edited_runs.clone(), None)]),
        |s| assert_eq!(runs_of(el(s, "e2"))[0].text, "Edited"));
    assert_slide_edit_round_trips("batch UpdateElementEffects", batch(vec![SlideEditOp::update_element_effects("e1", effects.clone())]),
        |s| assert_eq!(effects_of(el(s, "e1")).map(|e| e.blend_mode), Some("multiply".to_string())));
    assert_slide_edit_round_trips("batch ReorderElements", batch(vec![SlideEditOp::reorder_elements("e1", 9)]),
        |s| { let z = |id| transform_of(el(s, id)).unwrap().z_index; assert!(z("e1") > z("e2")); });
    assert_slide_edit_round_trips("batch GroupElements", batch(vec![SlideEditOp::group_elements(vec!["e1".into(), "e2".into()])]),
        |s| assert!(matches!(&s.elements[0], SlideElement::Group { .. })));
}

#[test]
fn test_schedule_title_expand_and_new_schedule() {
    let engine = ShowEngine::new(Arc::new(EventLog::new(100)));
    engine.execute_command(ShowCommand::AddToSchedule(simple_item("a", "A", &["a1"])));
    engine.execute_command(ShowCommand::SetScheduleTitle("Easter Sunday".into()));
    assert_eq!(engine.snapshot().schedule.title, "Easter Sunday");

    let expanded = |e: &ShowEngine| e.snapshot().schedule.items[0].is_expanded;
    let was = expanded(&engine);
    engine.execute_command(ShowCommand::ToggleScheduleItemExpand(0));
    assert_eq!(expanded(&engine), !was);
    engine.execute_command(ShowCommand::ToggleScheduleItemExpand(0));
    assert_eq!(expanded(&engine), was);

    // New schedule: empty, untitled again, nothing live or staged left pointing at the old one.
    engine.execute_command(ShowCommand::GoLive { item_index: Some(0), slide_index: Some(0) });
    engine.execute_command(ShowCommand::NewSchedule);
    let snap = engine.snapshot();
    assert!(snap.schedule.items.is_empty());
    assert_ne!(snap.schedule.title, "Easter Sunday");
    assert_eq!(snap.state.selected_item_index, None);
}

#[test]
fn test_undoing_an_unrelated_edit_keeps_existing_group_ids() {
    // Regression: the group id was minted randomly while *applying* the grouping event, so every replay
    // of the event log (each Undo/Redo) gave every group a new id.
    let engine = ShowEngine::new(Arc::new(EventLog::new(100)));
    let slide = Slide { elements: vec![text_el("e1", "One", 1), text_el("e2", "Two", 2), text_el("e3", "Three", 3)], ..Default::default() };
    engine.execute_command(ShowCommand::AddToSchedule(ScheduleItem { slides: vec![slide], ..item("s", "S") }));
    engine.execute_command(ShowCommand::GroupSlideElements { item_index: 0, slide_index: 0, element_ids: vec!["e1".into(), "e2".into()] });
    let group_id = |e: &ShowEngine| e.snapshot().schedule.items[0].slides[0].elements.iter()
        .find(|el| matches!(el, SlideElement::Group { .. })).map(|g| g.id().to_string()).unwrap();
    let id = group_id(&engine);

    engine.execute_command(ShowCommand::SetSlideSpeakerNotes { item_index: 0, slide_index: 0, notes: "later edit".into() });
    engine.execute_command(ShowCommand::Undo);
    assert_eq!(group_id(&engine), id, "undoing the notes edit renamed the group");
    // ...so an Ungroup sent with the id the editor already had still finds it.
    engine.execute_command(ShowCommand::UngroupSlideElements { item_index: 0, slide_index: 0, group_id: id });
    assert!(engine.snapshot().schedule.items[0].slides[0].elements.iter().all(|el| !matches!(el, SlideElement::Group { .. })));
}

/// Undo/Redo rebuild the schedule by replaying the event log, and a restart replays it too, so a
/// replay must give back the same schedule: the same element ids (the console edits elements by
/// id, and an edit aimed at an id that changed under it is silently dropped) and the same "unsaved
/// changes" flag. Found by the Qwen soak run (every Undo renamed every auto-made text box).
#[test]
fn test_undo_redo_replay_keeps_element_ids_and_the_unsaved_flag() {
    use os_next::models::TextRun;
    let log = Arc::new(EventLog::new(100));
    let engine = ShowEngine::new(log.clone());
    engine.execute_command(ShowCommand::AddToSchedule(simple_item("a", "A", &["one", "two"])));
    let element_id = engine.snapshot().schedule.items[0].slides[0].elements[0].id().to_string();
    assert!(engine.snapshot().schedule.is_modified);

    let before = engine.snapshot().schedule;
    engine.execute_command(ShowCommand::SetScheduleTitle("Evening".into()));
    engine.execute_command(ShowCommand::Undo);
    let after_undo = engine.snapshot().schedule;
    assert_eq!(after_undo.items, before.items, "Undo must give back the very same items");
    assert!(after_undo.is_modified, "Undo back to an unsaved schedule must leave it unsaved");
    engine.execute_command(ShowCommand::Redo);
    assert_eq!(engine.snapshot().schedule.items, before.items);
    assert!(engine.snapshot().schedule.is_modified);

    // The edit the console sends next still lands on the text box it was showing.
    engine.execute_command(ShowCommand::UpdateTextBlockContent {
        item_index: 0, slide_index: 0, element_id: element_id.clone(),
        runs: vec![TextRun { text: "edited".into(), ..Default::default() }], paragraph_style: None,
    });
    let slide = &engine.snapshot().schedule.items[0].slides[0];
    assert_eq!(slide.elements[0].id(), element_id);
    assert!(format!("{:?}", slide.elements[0]).contains("edited"), "the edit was dropped: {:?}", slide.elements[0]);

    // A restart (replaying the same log into a new engine) lands on the same schedule too.
    let restarted = ShowEngine::new(Arc::new(EventLog::new(100)));
    restarted.restore_from_events(log.all_events());
    assert_eq!(restarted.snapshot().schedule.items, engine.snapshot().schedule.items);
    assert_eq!(restarted.snapshot().schedule.is_modified, engine.snapshot().schedule.is_modified);
}
