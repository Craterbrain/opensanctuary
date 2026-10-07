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
fn looks_public_domain(ccli_number: &Option<String>, copyright: &Option<String>) -> bool {
    if ccli_number.is_some() {
        return false;
    }
    match copyright {
        None => true,
        Some(c) => {
            let lower = c.trim().to_lowercase();
            lower.is_empty() || lower.contains("public domain")
        }
    }
}

impl Database {
    /// Queries `event_log` for `ItemWentLive` events in `[start_ms, end_ms]`,
    /// filters to songs, and groups by CCLI number (or title) into one usage
    /// row per song. Only counts songs that actually *went live* -- adding a
    /// song to a schedule and never presenting it doesn't count, since
    /// that's what `ItemWentLive` already means (docs/CCLI_REPORTING.md).
    pub fn get_ccli_usage_report(&self, start_ms: i64, end_ms: i64) -> Result<Vec<CcliUsageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT payload, timestamp_ms FROM event_log \
             WHERE event_type = 'ItemWentLive' AND timestamp_ms >= ?1 AND timestamp_ms <= ?2 \
             ORDER BY timestamp_ms ASC",
        )?;
        let rows = stmt.query_map((start_ms, end_ms), |row| {
            let payload: String = row.get(0)?;
            let ts: i64 = row.get(1)?;
            Ok((payload, ts))
        })?;

        let mut grouped: HashMap<String, CcliUsageRow> = HashMap::new();

        for (payload, ts) in rows.flatten() {
            let event: crate::core::events::ShowEvent = match serde_json::from_str(&payload) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let item = match event {
                crate::core::events::ShowEvent::ItemWentLive { item: Some(item), .. } => item,
                _ => continue,
            };
            if item.item_type != "song" {
                continue;
            }

            let metadata = item.slides.iter().find_map(|s| s.ccli_metadata.clone());
            let ccli_number = metadata.as_ref().and_then(|m| m.ccli_number.clone());
            let copyright = metadata.as_ref().and_then(|m| m.copyright.clone());
            let author = metadata
                .as_ref()
                .and_then(|m| m.author.clone())
                .filter(|a| !a.trim().is_empty())
                .unwrap_or_else(|| item.author_or_ref.clone());
            let is_public_domain = looks_public_domain(&ccli_number, &copyright);

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
        assert!(looks_public_domain(&None, &None));
        assert!(looks_public_domain(&None, &Some("".to_string())));
        assert!(looks_public_domain(&None, &Some("Public Domain".to_string())));
        assert!(looks_public_domain(&None, &Some("  public domain  ".to_string())));
        assert!(!looks_public_domain(&None, &Some("© 2020 Example Music".to_string())));
        assert!(!looks_public_domain(&Some("1234567".to_string()), &None));
        assert!(!looks_public_domain(
            &Some("1234567".to_string()),
            &Some("Public Domain".to_string())
        ));
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
        let tmp_db_file = format!("/tmp/test_ccli_report_{}.db", uuid::Uuid::new_v4());
        let db = Database::new(&tmp_db_file).expect("must open test db");

        let copyrighted = song_item(
            "item-copyrighted",
            "Copyrighted Song",
            Some(CcliMetadata {
                song_title: Some("Copyrighted Song".to_string()),
                author: Some("Real Author".to_string()),
                copyright: Some("© 2020 Example Music".to_string()),
                ccli_number: Some("1234567".to_string()),
            }),
        );
        let pd_song = song_item("item-pd", "Amazing Grace", None);
        let non_song = ScheduleItem {
            item_type: "presentation".to_string(),
            ..song_item("item-slides", "Welcome Slide", None)
        };

        // Two uses of the copyrighted song inside the window, one outside it
        // (must not be counted), plus one PD use and one non-song use (must
        // be excluded entirely -- CCLI only cares about songs).
        db.save_event(&went_live(1, 1_000, copyrighted.clone())).expect("save 1");
        db.save_event(&went_live(2, 2_000, copyrighted.clone())).expect("save 2");
        db.save_event(&went_live(3, 999_999, copyrighted.clone())).expect("save 3 (out of range)");
        db.save_event(&went_live(4, 1_500, pd_song.clone())).expect("save 4");
        db.save_event(&went_live(5, 1_500, non_song)).expect("save 5");

        let rows = db.get_ccli_usage_report(0, 10_000).expect("report must succeed");

        let copyrighted_row = rows.iter().find(|r| r.ccli_number.as_deref() == Some("1234567")).expect("copyrighted song must appear");
        assert_eq!(copyrighted_row.use_count, 2, "out-of-range use must not be counted");
        assert_eq!(copyrighted_row.first_used_ms, 1_000);
        assert_eq!(copyrighted_row.last_used_ms, 2_000);
        assert_eq!(copyrighted_row.author, "Real Author");
        assert!(!copyrighted_row.is_public_domain);

        let pd_row = rows.iter().find(|r| r.title == "Amazing Grace").expect("PD song must appear");
        assert_eq!(pd_row.use_count, 1);
        assert!(pd_row.is_public_domain);
        assert_eq!(pd_row.author, "Fallback Author", "falls back to author_or_ref with no CcliMetadata");

        assert!(
            !rows.iter().any(|r| r.title == "Welcome Slide"),
            "non-song items must never appear in a CCLI report"
        );
    }
}
