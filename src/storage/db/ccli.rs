use rusqlite::Result;
use serde::Serialize;
use std::collections::HashMap;

use super::Database;

/// One song's usage within a report window (docs/CCLI_REPORTING.md). Grouped
/// by `ccli_number` when present, falling back to a lowercased title for
/// songs missing one -- same as the design doc specifies, so a song with no
/// CCLI number isn't silently dropped, just grouped more loosely.
#[derive(Debug, Clone, Serialize)]
pub struct CcliUsageRow {
    pub title: String,
    pub author: String,
    pub ccli_number: Option<String>,
    pub use_count: u32,
    pub first_used_ms: i64,
    pub last_used_ms: i64,
    pub is_public_domain: bool,
}

/// Public-domain heuristic from docs/CCLI_REPORTING.md: no CCLI number, and
/// the copyright string is either empty/absent or contains "public domain".
/// Deliberately a heuristic, not authoritative -- the report UI surfaces
/// this as a visible, overridable section rather than a silent filter.
fn looks_public_domain(ccli_number: &Option<String>, copyright: &Option<String>, override_pd: Option<bool>) -> bool {
    if override_pd == Some(true) {
        return true;
    }
    if ccli_number.is_some() {
        return false;
    }
    match copyright {
        None => true,
        Some(c) => {
            let lower = c.trim().to_lowercase();
            lower.is_empty() || crate::core::models::copyright_text_is_public_domain(&lower)
        }
    }
}

impl Database {
    /// Counts, per song, the times it was *put up* (docs/CCLI_REPORTING.md) in `[start_ms, end_ms]`,
    /// grouped by CCLI number (or title). Adding a song to a schedule and never presenting it doesn't
    /// count. Neither does every `ItemWentLive` event: the engine also emits one when the operator jumps
    /// to another slide of the song already live, and when Undo/Redo replays the log (a `ScheduleLoaded`
    /// immediately followed by `ItemWentLive` in the same command) -- counting those over-reported every
    /// song a church used. So a use is the live item *changing to* that song, outside a replay; the scan
    /// starts before the window so the song already up when it opens isn't counted again.
    pub fn get_ccli_usage_report(&self, start_ms: i64, end_ms: i64) -> Result<Vec<CcliUsageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT sequence_number, event_type, CASE WHEN event_type = 'ItemWentLive' THEN payload END, timestamp_ms \
             FROM event_log \
             WHERE event_type IN ('ItemWentLive', 'ScheduleLoaded') AND timestamp_ms <= ?1 \
             ORDER BY sequence_number ASC",
        )?;
        let rows = stmt.query_map([end_ms], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, i64>(3)?))
        })?;

        let mut uses: Vec<(crate::core::models::ScheduleItem, i64)> = Vec::new();
        let mut live_id: Option<String> = None;
        let mut last_load_seq: Option<i64> = None;
        for (seq, event_type, payload, ts) in rows.flatten() {
            if event_type == "ScheduleLoaded" {
                last_load_seq = Some(seq);
                continue;
            }
            let item = match payload.and_then(|p| serde_json::from_str::<crate::core::events::ShowEvent>(&p).ok()) {
                Some(crate::core::events::ShowEvent::ItemWentLive { item, .. }) => item,
                _ => continue,
            };
            let id = item.as_ref().map(|i| i.id.clone());
            let replay = last_load_seq == Some(seq - 1);
            if let Some(item) = item {
                if !replay && id != live_id && ts >= start_ms && item.item_type == "song" {
                    uses.push((item, ts));
                }
            }
            live_id = id;
        }

        let mut grouped: HashMap<String, CcliUsageRow> = HashMap::new();

        for (item, ts) in uses {
            let metadata = item.slides.iter().find_map(|s| s.ccli_metadata.clone());
            let ccli_number = metadata.as_ref().and_then(|m| m.ccli_number.clone());
            let copyright = metadata.as_ref().and_then(|m| m.copyright.clone());
            let author = metadata
                .as_ref()
                .and_then(|m| m.author.clone())
                .filter(|a| !a.trim().is_empty())
                .unwrap_or_else(|| item.author_or_ref.clone());
            let is_public_domain = looks_public_domain(&ccli_number, &copyright, metadata.as_ref().and_then(|m| m.public_domain));

            let key = ccli_number.clone().unwrap_or_else(|| item.title.to_lowercase());

            grouped
                .entry(key)
                .and_modify(|r| {
                    r.use_count += 1;
                    r.first_used_ms = r.first_used_ms.min(ts);
                    r.last_used_ms = r.last_used_ms.max(ts);
                })
                .or_insert(CcliUsageRow {
                    title: item.title.clone(),
                    author,
                    ccli_number,
                    use_count: 1,
                    first_used_ms: ts,
                    last_used_ms: ts,
                    is_public_domain,
                });
        }

        let mut result: Vec<CcliUsageRow> = grouped.into_values().collect();
        result.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::events::{EventEnvelope, ShowEvent};
    use crate::core::models::{CcliMetadata, ScheduleItem, Slide};

    #[test]
    fn public_domain_heuristic_matches_docs_ccli_reporting() {
        assert!(looks_public_domain(&None, &None, None));
        assert!(looks_public_domain(&None, &Some("".to_string()), None));
        assert!(looks_public_domain(&None, &Some("Public Domain".to_string()), None));
        assert!(looks_public_domain(&None, &Some("  public domain  ".to_string()), None));
        assert!(!looks_public_domain(&None, &Some("© 2020 Example Music".to_string()), None));
        assert!(!looks_public_domain(&Some("1234567".to_string()), &None, None));
        assert!(!looks_public_domain(
            &Some("1234567".to_string()),
            &Some("Public Domain".to_string()),
            None
        ));
        // The operator's override wins over the text and a CCLI number; a modern arrangement of a public-domain hymn is not public domain.
        assert!(looks_public_domain(&Some("1234567".to_string()), &Some("© 2020 Example Music".to_string()), Some(true)));
        assert!(!looks_public_domain(&None, &Some("Arr. © 2001 Example Music, based on a public domain hymn".to_string()), None));
        assert!(!looks_public_domain(&None, &Some("Public Domain; arrangement 1998 Smith".to_string()), None));
    }

    fn song_item(id: &str, title: &str, ccli_metadata: Option<CcliMetadata>) -> ScheduleItem {
        ScheduleItem {
            id: id.to_string(),
            title: title.to_string(),
            subtitle: None,
            notes: None,
            item_type: "song".to_string(),
            author_or_ref: "Fallback Author".to_string(),
            slides: vec![Slide { text: "V1".to_string(), header: None, label: None, background: None, notes: None, tag: None, ccli_metadata, ..Default::default() }],
            background: None,
            theme_name: None,
            is_expanded: false,
            is_section_header: false,
            arrangement: Vec::new(),
            default_slide_duration_seconds: None,
            slideshow_loop: false,
            licensed: None,
        }
    }

    fn went_live(seq: u64, ts: i64, item: ScheduleItem) -> EventEnvelope {
        EventEnvelope {
            sequence_number: seq,
            event_id: format!("evt-{}", seq),
            timestamp_ms: ts as u64,
            event: ShowEvent::ItemWentLive { item: Some(item), slide_index: 0 },
        }
    }

    #[test]
    fn get_ccli_usage_report_groups_counts_and_splits_public_domain() {
        let db = Database::in_memory().expect("must open test db");

        let copyrighted = song_item(
            "item-copyrighted",
            "Copyrighted Song",
            Some(CcliMetadata {
                song_title: Some("Copyrighted Song".to_string()),
                author: Some("Real Author".to_string()),
                copyright: Some("© 2020 Example Music".to_string()),
                ccli_number: Some("1234567".to_string()),
                public_domain: None,
            }),
        );
        let pd_song = song_item("item-pd", "Amazing Grace", None);
        let non_song = ScheduleItem {
            item_type: "presentation".to_string(),
            ..song_item("item-slides", "Welcome Slide", None)
        };

        // The copyrighted song is put up twice inside the window (with another song in between) and once
        // after it (must not be counted), plus one PD use and one non-song use (excluded entirely -- CCLI
        // only cares about songs). Each is a change of what's live: going live again on the song that's
        // already up isn't another use (see test_ccli_report_counts_a_song_once_per_time_it_is_put_up).
        db.save_event(&went_live(1, 1_000, copyrighted.clone())).expect("save 1");
        db.save_event(&went_live(2, 1_500, pd_song.clone())).expect("save 2");
        db.save_event(&went_live(3, 1_600, non_song)).expect("save 3");
        db.save_event(&went_live(4, 2_000, copyrighted.clone())).expect("save 4");
        db.save_event(&went_live(5, 2_500, pd_song.clone())).expect("save 5");
        db.save_event(&went_live(6, 999_999, copyrighted.clone())).expect("save 6 (out of range)");

        let rows = db.get_ccli_usage_report(0, 10_000).expect("report must succeed");

        let copyrighted_row = rows.iter().find(|r| r.ccli_number.as_deref() == Some("1234567")).expect("copyrighted song must appear");
        assert_eq!(copyrighted_row.use_count, 2, "out-of-range use must not be counted");
        assert_eq!(copyrighted_row.first_used_ms, 1_000);
        assert_eq!(copyrighted_row.last_used_ms, 2_000);
        assert_eq!(copyrighted_row.author, "Real Author");
        assert!(!copyrighted_row.is_public_domain);

        let pd_row = rows.iter().find(|r| r.title == "Amazing Grace").expect("PD song must appear");
        assert_eq!(pd_row.use_count, 2);
        assert!(pd_row.is_public_domain);
        assert_eq!(pd_row.author, "Fallback Author", "falls back to author_or_ref with no CcliMetadata");

        assert!(
            !rows.iter().any(|r| r.title == "Welcome Slide"),
            "non-song items must never appear in a CCLI report"
        );

        // A window that opens while a song is already up, with only a jump within it inside the window,
        // has no new use of it (the scan starts before the window for exactly this).
        db.save_event(&went_live(7, 20_000, copyrighted.clone())).expect("save 7");
        db.save_event(&went_live(8, 30_000, copyrighted.clone())).expect("save 8 (same song, later slide)");
        assert!(db.get_ccli_usage_report(25_000, 40_000).unwrap().is_empty());
    }
}
