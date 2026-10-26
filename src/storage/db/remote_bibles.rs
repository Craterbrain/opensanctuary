//! Translations a plugin offers without storing them: YouVersion's NIV, NASB, ... The console lists them in
//! the Scriptures sidebar with the plugin's tag; the Bible text is fetched only when a passage is added to the
//! schedule (see `licensed.rs`, and `docs/LICENSED_SCRIPTURE.md`). What is stored here is metadata only:
//! which Bibles the operator chose, never any text.

use rusqlite::Result;
use serde::{Deserialize, Serialize};

use super::Database;

const KEY_PREFIX: &str = "remoteBibles:";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteBible {
    /// The plugin that serves it (set by the host, never by the plugin's say-so).
    #[serde(default)]
    pub source: String,
    pub bible_id: i64,
    pub abbreviation: String,
    pub title: String,
    #[serde(default)]
    pub language: String,
    /// Shown beside the name in the sidebar: "YouVersion".
    pub tag: String,
    #[serde(default)]
    pub copyright: String,
    #[serde(default)]
    pub link: String,
    /// The publisher's id, for the plugin's own rules.
    #[serde(default)]
    pub organization: String,
    /// What the publisher lets be shown at once: chapters, or (when non-zero) this many verses whichever is greater.
    #[serde(default)]
    pub max_chapters: u32,
    #[serde(default)]
    pub max_verses: u32,
}

/// The short notice printed on every slide: "NIV \u{b7} \u{a9} Biblica, Inc". The version name and the rights holder,
/// read from the publisher's own copyright text; just the version name if it can't be read. (The same rule the
/// YouVersion plugin uses, so a passage looked up either way carries the same notice.)
pub fn licensed_tag_for(b: &RemoteBible) -> String {
    let name = b.abbreviation.trim();
    let text = b.copyright.trim();
    let lower = text.to_lowercase();
    let holder = if lower.starts_with("public domain") {
        Some("Public Domain".to_string())
    } else {
        // after "(c)" / "\u{a9}" / "copyright", skip years and "by"
        let start = ["\u{a9}", "(c)", "copyright"].iter().filter_map(|m| lower.find(m).map(|i| i + m.len())).min();
        start.and_then(|i| {
            let rest = text.get(i..)?;
            let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{a9}');
            let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == ',' || c == '-' || c == '\u{2013}' || c.is_whitespace());
            let rest = rest.strip_prefix("by ").or_else(|| rest.strip_prefix("By ")).unwrap_or(rest);
            let end = rest.find(|c: char| matches!(c, '.' | '\u{ae}' | '\u{2122}' | '\n' | ';')).unwrap_or(rest.len());
            let h = rest[..end].split_whitespace().collect::<Vec<_>>().join(" ");
            let h = h.trim_end_matches([',', ' ']).to_string();
            (h.len() >= 2 && h.len() <= 60).then_some(h)
        })
    };
    let tag = match holder {
        Some(h) if h == "Public Domain" => format!("{name} \u{b7} {h}"),
        Some(h) => format!("{name} \u{b7} \u{a9} {h}"),
        None => name.to_string(),
    };
    if tag.chars().count() > 60 { tag.chars().take(59).collect::<String>() + "\u{2026}" } else { tag }
}

pub const MAX_REMOTE_BIBLES: usize = 200;

impl RemoteBible {
    /// Checks what a plugin sent. Returns a cleaned copy with `source` set.
    pub fn checked(mut self, source: &str) -> std::result::Result<Self, String> {
        let short = |s: &str, max: usize| s.len() <= max && !s.chars().any(|c| c.is_control());
        self.abbreviation = self.abbreviation.trim().to_string();
        self.title = self.title.trim().to_string();
        self.tag = self.tag.trim().to_string();
        if self.bible_id <= 0 || self.abbreviation.is_empty() || !short(&self.abbreviation, 40) {
            return Err("each translation needs a bible_id and an abbreviation (40 characters at most)".into());
        }
        if self.title.is_empty() || !short(&self.title, 200) || self.tag.is_empty() || !short(&self.tag, 24) {
            return Err("each translation needs a title (200) and a tag (24 characters at most)".into());
        }
        if !short(&self.language, 40) || !short(&self.organization, 80) || self.copyright.len() > 4000 || self.link.len() > 300 {
            return Err("a translation field is too long".into());
        }
        if !self.link.is_empty() && !self.link.starts_with("https://") {
            return Err("a translation's link must be an https address".into());
        }
        if self.max_chapters > 100 || self.max_verses > 1000 {
            return Err("a translation's limits are out of range".into());
        }
        self.source = source.to_string();
        Ok(self)
    }
}

impl Database {
    /// Replaces the set of translations `plugin_id` offers.
    pub fn set_remote_bibles(&self, plugin_id: &str, bibles: &[RemoteBible]) -> Result<()> {
        let key = format!("{KEY_PREFIX}{plugin_id}");
        if bibles.is_empty() {
            return self.delete_setting(&key);
        }
        let json = serde_json::to_string(bibles).unwrap_or_else(|_| "[]".to_string());
        self.set_setting(&key, &json)
    }

    pub fn get_remote_bibles(&self, plugin_id: &str) -> Result<Vec<RemoteBible>> {
        Ok(self
            .get_setting(&format!("{KEY_PREFIX}{plugin_id}"))?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default())
    }

    /// Every plugin's translations (the sidebar).
    pub fn all_remote_bibles(&self) -> Result<Vec<RemoteBible>> {
        let mut out = Vec::new();
        for (key, value) in self.get_settings()? {
            if let Some(plugin) = key.strip_prefix(KEY_PREFIX) {
                if let Ok(list) = serde_json::from_str::<Vec<RemoteBible>>(&value) {
                    out.extend(list.into_iter().map(|mut b| {
                        b.source = plugin.to_string();
                        b
                    }));
                }
            }
        }
        out.sort_by(|a, b| a.abbreviation.cmp(&b.abbreviation).then(a.bible_id.cmp(&b.bible_id)));
        Ok(out)
    }

    /// One translation, for turning a reference into a schedule item.
    pub fn find_remote_bible(&self, source: &str, bible_id: i64) -> Result<Option<RemoteBible>> {
        Ok(self.get_remote_bibles(source)?.into_iter().find(|b| b.bible_id == bible_id).map(|mut b| {
            b.source = source.to_string();
            b
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bible(id: i64, abbr: &str) -> RemoteBible {
        RemoteBible {
            source: "spoofed".into(), bible_id: id, abbreviation: abbr.into(), title: format!("{abbr} Bible"), language: "en".into(),
            tag: "YouVersion".into(), copyright: "(c)".into(), link: "https://www.bible.com/versions/1".into(), organization: "org".into(),
            max_chapters: 2, max_verses: 25,
        }
    }

    #[test]
    fn stored_per_plugin_and_never_holds_text() {
        let db = Database::in_memory().unwrap();
        db.set_remote_bibles("youversion", &[bible(111, "NIV"), bible(3034, "BSB")]).unwrap();
        db.set_remote_bibles("other", &[bible(5, "ZZZ")]).unwrap();
        assert_eq!(db.get_remote_bibles("youversion").unwrap().len(), 2);
        let all = db.all_remote_bibles().unwrap();
        assert_eq!(all.iter().map(|b| (b.abbreviation.as_str(), b.source.as_str())).collect::<Vec<_>>(), [("BSB", "youversion"), ("NIV", "youversion"), ("ZZZ", "other")]);
        assert_eq!(db.find_remote_bible("youversion", 111).unwrap().unwrap().abbreviation, "NIV");
        assert!(db.find_remote_bible("other", 111).unwrap().is_none(), "another plugin's set is not searched");
        db.set_remote_bibles("youversion", &[]).unwrap();
        assert!(db.get_remote_bibles("youversion").unwrap().is_empty());
        assert_eq!(db.all_remote_bibles().unwrap().len(), 1);
    }

    #[test]
    fn the_notice_is_the_version_and_the_rights_holder() {
        let b = |abbr: &str, c: &str| RemoteBible { abbreviation: abbr.into(), copyright: c.into(), ..bible(1, abbr) };
        assert_eq!(licensed_tag_for(&b("NIV", "The Holy Bible, New International Version\u{ae} NIV\u{ae} Copyright \u{a9} 1973, 1978, 1984, 2011 by Biblica, Inc.\u{ae} Used by Permission.")), "NIV \u{b7} \u{a9} Biblica, Inc");
        assert_eq!(licensed_tag_for(&b("NASB2020", "New American Standard Bible\u{ae} Copyright \u{a9} 1960, 1971, 1977, 1995, 2020 by The Lockman Foundation. All rights reserved")), "NASB2020 \u{b7} \u{a9} The Lockman Foundation");
        assert_eq!(licensed_tag_for(&b("BSB", "Public Domain")), "BSB \u{b7} Public Domain");
        assert_eq!(licensed_tag_for(&b("X", "")), "X");
        assert!(licensed_tag_for(&b("X", "(c) ")).chars().count() <= 60);
    }

    #[test]
    fn what_a_plugin_sends_is_checked_and_the_source_is_the_hosts() {
        let ok = bible(111, "NIV").checked("youversion").unwrap();
        assert_eq!(ok.source, "youversion", "the plugin cannot name another plugin");
        for bad in [
            RemoteBible { bible_id: 0, ..bible(1, "X") },
            RemoteBible { abbreviation: " ".into(), ..bible(1, "X") },
            RemoteBible { tag: "".into(), ..bible(1, "X") },
            RemoteBible { tag: "x".repeat(25), ..bible(1, "X") },
            RemoteBible { link: "http://example.com".into(), ..bible(1, "X") },
            RemoteBible { title: "a\nb".into(), ..bible(1, "X") },
            RemoteBible { max_chapters: 500, ..bible(1, "X") },
        ] {
            assert!(bad.checked("youversion").is_err());
        }
    }
}
