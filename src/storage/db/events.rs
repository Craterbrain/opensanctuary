use rusqlite::Result;

use super::Database;

impl Database {
    pub fn save_event(&self, envelope: &crate::core::events::EventEnvelope) -> Result<()> {
        let conn = self.conn();
        let payload = serde_json::to_string(&envelope.event).unwrap_or_default();
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
            crate::core::events::ShowEvent::ScheduleReordered { .. } => "ScheduleReordered",
            crate::core::events::ShowEvent::ScheduleSlidesReordered { .. } => "ScheduleSlidesReordered",
            crate::core::events::ShowEvent::ScheduleSlideDuplicated { .. } => "ScheduleSlideDuplicated",
            crate::core::events::ShowEvent::ScheduleSlideRemoved { .. } => "ScheduleSlideRemoved",
            crate::core::events::ShowEvent::ScheduleItemExpandToggled { .. } => "ScheduleItemExpandToggled",
            crate::core::events::ShowEvent::ScheduleItemThemeSet { .. } => "ScheduleItemThemeSet",
            crate::core::events::ShowEvent::SlideBackgroundSet { .. } => "SlideBackgroundSet",
            crate::core::events::ShowEvent::GlobalThemeSet { .. } => "GlobalThemeSet",
            crate::core::events::ShowEvent::ScheduleLoaded { .. } => "ScheduleLoaded",
            crate::core::events::ShowEvent::ScheduleCleared => "ScheduleCleared",
            crate::core::events::ShowEvent::ScheduleTitleSet { .. } => "ScheduleTitleSet",
            _ => "MediaControlEvent",
        };
        conn.execute(
            "INSERT INTO event_log (sequence_number, event_type, payload, timestamp_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(sequence_number) DO NOTHING",
            (envelope.sequence_number, event_type, payload, envelope.timestamp_ms as i64),
        )?;
        Ok(())
    }

    pub fn get_max_event_sequence(&self) -> Result<u64> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT COALESCE(MAX(sequence_number), 0) FROM event_log")?;
        let max_seq: i64 = stmt.query_row([], |row| row.get(0))?;
        Ok(max_seq.max(0) as u64)
    }

    pub fn load_events(&self) -> Result<Vec<crate::core::events::EventEnvelope>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT sequence_number, payload, timestamp_ms FROM event_log ORDER BY sequence_number ASC")?;
        let event_iter = stmt.query_map([], |row| {
            let seq: u64 = row.get(0)?;
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
}
