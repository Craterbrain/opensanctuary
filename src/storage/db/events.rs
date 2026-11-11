use rusqlite::{OptionalExtension, Result};

use crate::core::engine::EngineSnapshot;
use crate::core::models::{Schedule, ShowState};

use super::Database;

impl Database {
    pub fn save_event(&self, envelope: &crate::core::events::EventEnvelope) -> Result<()> {
        let conn = self.conn();
        // Licensed passages never reach the event log with their text (see core::events).
        let payload = serde_json::to_string(&*envelope.event.without_licensed_text()).unwrap_or_default();
        let event_type = match &envelope.event {
            crate::core::events::ShowEvent::ItemWentLive { .. } => "ItemWentLive",
            crate::core::events::ShowEvent::LiveSlideChanged { .. } => "LiveSlideChanged",
            crate::core::events::ShowEvent::ItemStaged { .. } => "ItemStaged",
            crate::core::events::ShowEvent::StagedSlideChanged { .. } => "StagedSlideChanged",
            crate::core::events::ShowEvent::BlackoutToggled { .. } => "BlackoutToggled",
            crate::core::events::ShowEvent::ClearTextToggled { .. } => "ClearTextToggled",
            crate::core::events::ShowEvent::LogoToggled { .. } => "LogoToggled",
            crate::core::events::ShowEvent::AlertChanged { .. } => "AlertChanged",
            crate::core::events::ShowEvent::TransitionChanged { .. } => "TransitionChanged",
            crate::core::events::ShowEvent::ScheduleItemAdded { .. } => "ScheduleItemAdded",
            crate::core::events::ShowEvent::ScheduleItemRemoved { .. } => "ScheduleItemRemoved",
            crate::core::events::ShowEvent::ScheduleItemReplaced { .. } => "ScheduleItemReplaced",
            crate::core::events::ShowEvent::ScheduleReordered { .. } => "ScheduleReordered",
            crate::core::events::ShowEvent::ScheduleSlidesReordered { .. } => "ScheduleSlidesReordered",
            crate::core::events::ShowEvent::ScheduleSlideDuplicated { .. } => "ScheduleSlideDuplicated",
            crate::core::events::ShowEvent::ScheduleSlideRemoved { .. } => "ScheduleSlideRemoved",
            crate::core::events::ShowEvent::LicensedPassageSet { .. } => "LicensedPassageSet",
            crate::core::events::ShowEvent::ScheduleItemExpandToggled { .. } => "ScheduleItemExpandToggled",
            crate::core::events::ShowEvent::ScheduleItemThemeSet { .. } => "ScheduleItemThemeSet",
            crate::core::events::ShowEvent::SlideBackgroundSet { .. } => "SlideBackgroundSet",
            crate::core::events::ShowEvent::GlobalThemeSet { .. } => "GlobalThemeSet",
            // Told apart in the stored type, for the CCLI report (`get_ccli_usage_report`), which reads
            // only these columns. "ScheduleLoaded" rows are from before the distinction.
            crate::core::events::ShowEvent::ScheduleLoaded { replay: true, .. } => "ScheduleReplayed",
            crate::core::events::ShowEvent::ScheduleLoaded { replay: false, .. } => "ScheduleOpened",
            crate::core::events::ShowEvent::ScheduleCleared => "ScheduleCleared",
            crate::core::events::ShowEvent::ScheduleTitleSet { .. } => "ScheduleTitleSet",
            _ => "MediaControlEvent",
        };
        conn.execute(
            "INSERT INTO event_log (sequence_number, event_type, payload, timestamp_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(sequence_number) DO NOTHING",
            // u64 no longer has ToSql in rusqlite 0.40; sequence_number never gets near i64::MAX.
            (envelope.sequence_number as i64, event_type, payload, envelope.timestamp_ms as i64),
        )?;
        Ok(())
    }

    pub fn get_max_event_sequence(&self) -> Result<u64> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT MAX(COALESCE((SELECT MAX(sequence_number) FROM event_log), 0), \
                        COALESCE((SELECT sequence_number FROM engine_snapshot WHERE id = 1), 0))",
        )?;
        let max_seq: i64 = stmt.query_row([], |row| row.get(0))?;
        Ok(max_seq.max(0) as u64)
    }

    /// The events to replay on top of `load_engine_snapshot` (all of them, when there is none). Older
    /// rows a compaction kept for the CCLI report are already in the snapshot and are left out.
    pub fn load_events(&self) -> Result<Vec<crate::core::events::EventEnvelope>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT sequence_number, payload, timestamp_ms FROM event_log \
             WHERE sequence_number > COALESCE((SELECT sequence_number FROM engine_snapshot WHERE id = 1), 0) \
             ORDER BY sequence_number ASC",
        )?;
        let event_iter = stmt.query_map([], |row| {
            // u64 no longer has FromSql in rusqlite 0.40; see the insert above.
            let seq: u64 = row.get::<_, i64>(0)? as u64;
            let payload_str: String = row.get(1)?;
            let ts: i64 = row.get(2)?;
            match serde_json::from_str::<crate::core::events::ShowEvent>(&payload_str) {
                Ok(event) => Ok(Some(crate::core::events::EventEnvelope {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    sequence_number: seq,
                    event,
                    timestamp_ms: ts as u64,
                })),
                Err(e) => {
                    tracing::warn!("Skipping unparseable event #{} from event_log: {}", seq, e);
                    Ok(None)
                }
            }
        })?;

        let mut events = Vec::new();
        for env in event_iter.flatten().flatten() {
            events.push(env);
        }
        Ok(events)
    }

    pub fn load_engine_snapshot(&self) -> Result<Option<EngineSnapshot>> {
        let conn = self.conn();
        let row = conn
            .query_row("SELECT sequence_number, state, schedule FROM engine_snapshot WHERE id = 1", [], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .optional()?;
        let Some((seq, state, schedule)) = row else { return Ok(None) };
        match (serde_json::from_str(&state), serde_json::from_str(&schedule)) {
            (Ok(state), Ok(schedule)) => Ok(Some(EngineSnapshot { sequence_number: seq.max(0) as u64, state, schedule })),
            (state, schedule) => {
                // The events it stands for are gone, so this can't be rebuilt; start from a clean engine.
                tracing::warn!(
                    "engine snapshot #{} is unreadable ({:?} / {:?}); starting from its later events only",
                    seq,
                    state.err(),
                    schedule.err()
                );
                Ok(None)
            }
        }
    }

    /// Keeps the event log from growing for ever: stores `snapshot` (the engine as it stood after event
    /// `snapshot.sequence_number`) as where a replay starts, and drops the events it already holds -- all
    /// but what the CCLI report reads (`get_ccli_usage_report`): every `ItemWentLive`, and a replay marker
    /// right before one, its schedule emptied (the marker's type is all the report reads). Returns how
    /// many rows went.
    pub fn compact_event_log(&self, snapshot: &EngineSnapshot) -> Result<usize> {
        let seq = snapshot.sequence_number as i64;
        // Licensed text is never stored (see core::events).
        let state = ShowState {
            live_item: snapshot.state.live_item.as_ref().map(|i| i.without_licensed_text()),
            staged_item: snapshot.state.staged_item.as_ref().map(|i| i.without_licensed_text()),
            ..snapshot.state.clone()
        };
        let state_json = serde_json::to_string(&state).unwrap_or_default();
        let schedule_json = serde_json::to_string(&snapshot.schedule.without_licensed_text()).unwrap_or_default();
        let marker = |replay: bool| {
            serde_json::to_string(&crate::core::events::ShowEvent::ScheduleLoaded { schedule: Schedule::default(), replay }).unwrap_or_default()
        };

        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO engine_snapshot (id, sequence_number, state, schedule) VALUES (1, ?1, ?2, ?3)",
            (seq, state_json, schedule_json),
        )?;
        let removed = tx.execute(
            "DELETE FROM event_log WHERE sequence_number <= ?1 AND event_type <> 'ItemWentLive' \
             AND NOT (event_type IN ('ScheduleReplayed', 'ScheduleLoaded') \
                      AND sequence_number + 1 IN (SELECT sequence_number FROM event_log WHERE event_type = 'ItemWentLive'))",
            [seq],
        )?;
        tx.execute("UPDATE event_log SET payload = ?2 WHERE sequence_number <= ?1 AND event_type = 'ScheduleReplayed'", (seq, marker(true)))?;
        tx.execute("UPDATE event_log SET payload = ?2 WHERE sequence_number <= ?1 AND event_type = 'ScheduleLoaded'", (seq, marker(false)))?;
        tx.commit()?;
        Ok(removed)
    }
}
