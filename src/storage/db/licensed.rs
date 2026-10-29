//! Licensed Bible passages (YouVersion's NIV, NASB, ...): text a publisher lets the app show, not keep.
//!
//! Kept apart from the Bibles the user owns (`bibles/<VERSION>.db`):
//!  * the verses are sealed with AES-256-GCM before they touch the database, so opening `library.db`
//!    with a SQLite browser shows nothing readable (the key lives in the OS keyring, or, where there is
//!    none, in an owner-only file beside the database);
//!  * every passage has an expiry, taken from the `Cache-Control` the publisher's API sent with it;
//!    expired passages are deleted at start-up and again every hour (`PRAGMA secure_delete` zeroes the
//!    freed pages), and an expired passage is not served;
//!  * nothing about this table is exported: exports carry the reference only (see `LicensedMeta`).
//!
//! This makes recovering the text from disk difficult, not impossible: the app must be able to show it.

use std::path::Path;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};

use crate::core::models::{ScriptureItem, ScriptureVerse};
use crate::storage::keyring::KeyringService;

use super::Database;

const KEY_SERVICE: &str = "OpenSanctuary:System";
const KEY_ACCOUNT: &str = "licensed-cache-key";
const KEY_FILE: &str = ".licensed-cache.key";
const SEAL_PREFIX: &[u8] = b"os1";
/// Same switch the TLS key uses (src/network/tls.rs): tests and headless boxes keep secrets in a file.
const ENV_FILE_STORE: &str = "OS_NEXT_TLS_KEY_STORE";

/// What the host knows about a licensed passage besides its (sealed) text. The reference, version and
/// notice are not licensed text and are what an export carries so the passage can be looked up again.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LicensedMeta {
    pub id: String,
    /// The plugin that supplied it and is asked to look it up again.
    pub source: String,
    pub bible_id: i64,
    /// `JHN.3.16-18`, the reference in the publisher's own form.
    pub usfm: String,
    /// Short tag shown on every slide, e.g. "NIV" (the licenses allow a tag that links to the publisher).
    pub tag: String,
    /// The publisher's full copyright and trademark notice.
    pub copyright: String,
    pub link: String,
    pub expires_at_ms: i64,
}

fn oops(msg: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(msg.into())))
}

fn encode_key(k: &[u8; 32]) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_key(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn random_key() -> Result<[u8; 32]> {
    let mut k = [0u8; 32];
    SystemRandom::new().fill(&mut k).map_err(|_| oops("no random numbers available"))?;
    Ok(k)
}

#[cfg(unix)]
fn write_private(path: &Path, data: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(data.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &Path, data: &str) -> std::io::Result<()> {
    std::fs::write(path, data)
}

/// Seals `plain` for row `id`: `os1 || nonce(12) || ciphertext || tag`. The id is authenticated, so a
/// sealed blob can't be moved to another row.
pub(super) fn seal(key: &[u8; 32], id: &str, plain: &[u8]) -> Result<Vec<u8>> {
    let mut nonce = [0u8; 12];
    SystemRandom::new().fill(&mut nonce).map_err(|_| oops("no random numbers available"))?;
    let k = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|_| oops("bad key"))?);
    let mut buf = plain.to_vec();
    k.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(id.as_bytes()), &mut buf)
        .map_err(|_| oops("could not seal"))?;
    let mut out = Vec::with_capacity(SEAL_PREFIX.len() + 12 + buf.len());
    out.extend_from_slice(SEAL_PREFIX);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&buf);
    Ok(out)
}

pub(super) fn open(key: &[u8; 32], id: &str, sealed: &[u8]) -> Option<Vec<u8>> {
    let rest = sealed.strip_prefix(SEAL_PREFIX)?;
    if rest.len() < 12 + 16 {
        return None;
    }
    let (nonce, ct) = rest.split_at(12);
    let k = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
    let mut buf = ct.to_vec();
    let plain = k.open_in_place(Nonce::try_assume_unique_for_key(nonce).ok()?, Aad::from(id.as_bytes()), &mut buf).ok()?;
    Some(plain.to_vec())
}

impl Database {
    pub(super) fn init_licensed_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS licensed_passages (
                id TEXT PRIMARY KEY,
                version TEXT NOT NULL,
                book TEXT NOT NULL,
                chapter INTEGER NOT NULL,
                verse_start INTEGER NOT NULL,
                verse_end INTEGER NOT NULL,
                reference TEXT NOT NULL,
                source TEXT NOT NULL,
                bible_id INTEGER NOT NULL,
                usfm TEXT NOT NULL,
                tag TEXT NOT NULL,
                copyright TEXT NOT NULL,
                link TEXT NOT NULL,
                expires_at_ms INTEGER NOT NULL,
                sealed BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_licensed_expiry ON licensed_passages(expires_at_ms);",
        )
    }

    /// The key that seals licensed text: OS keyring first, then an owner-only file beside the database.
    fn licensed_key(&self) -> Result<[u8; 32]> {
        if let Some(k) = self.licensed_key.get() {
            return Ok(*k);
        }
        let key = self.load_or_create_licensed_key()?;
        let _ = self.licensed_key.set(key);
        Ok(key)
    }

    fn load_or_create_licensed_key(&self) -> Result<[u8; 32]> {
        if self.is_in_memory {
            return random_key();
        }
        if std::env::var(ENV_FILE_STORE).as_deref() != Ok("file") {
            match KeyringService::get_secret(KEY_SERVICE, KEY_ACCOUNT) {
                Ok(Some(s)) => {
                    if let Some(k) = decode_key(&s) {
                        return Ok(k);
                    }
                }
                Ok(None) => {
                    let k = random_key()?;
                    if KeyringService::set_secret(KEY_SERVICE, KEY_ACCOUNT, &encode_key(&k)).is_ok() {
                        return Ok(k);
                    }
                }
                Err(_) => {}
            }
        }
        let dir = self.bibles_dir.as_deref().and_then(|d| d.parent()).unwrap_or_else(|| Path::new("."));
        let path = dir.join(KEY_FILE);
        if let Ok(s) = std::fs::read_to_string(&path) {
            if let Some(k) = decode_key(&s) {
                return Ok(k);
            }
        }
        let k = random_key()?;
        write_private(&path, &encode_key(&k)).map_err(|e| oops(format!("could not store the cache key: {e}")))?;
        Ok(k)
    }

    /// Saves (or replaces) a licensed passage. `item.id` must already be set.
    pub fn insert_licensed_scripture(&self, item: &ScriptureItem, meta: &LicensedMeta) -> Result<()> {
        let key = self.licensed_key()?;
        let verses = serde_json::to_vec(&item.verses).map_err(|e| oops(e.to_string()))?;
        let sealed = seal(&key, &item.id, &verses)?;
        let conn = self.main_conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO licensed_passages
             (id, version, book, chapter, verse_start, verse_end, reference, source, bible_id, usfm, tag, copyright, link, expires_at_ms, sealed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                item.id, item.version, item.book, item.chapter, item.verse_start, item.verse_end, item.reference,
                meta.source, meta.bible_id, meta.usfm, meta.tag, meta.copyright, meta.link, meta.expires_at_ms, sealed
            ],
        )?;
        Ok(())
    }

    fn licensed_row(&self, row: &rusqlite::Row, key: &[u8; 32]) -> rusqlite::Result<Option<(ScriptureItem, LicensedMeta)>> {
        let id: String = row.get(0)?;
        let sealed: Vec<u8> = row.get(14)?;
        let Some(plain) = open(key, &id, &sealed) else { return Ok(None) };
        let verses: Vec<ScriptureVerse> = serde_json::from_slice(&plain).unwrap_or_default();
        let item = ScriptureItem {
            id: id.clone(),
            version: row.get(1)?,
            book: row.get(2)?,
            chapter: row.get(3)?,
            verse_start: row.get(4)?,
            verse_end: row.get(5)?,
            reference: row.get(6)?,
            verses,
            theme_name: None,
        };
        let meta = LicensedMeta {
            id,
            source: row.get(7)?,
            bible_id: row.get(8)?,
            usfm: row.get(9)?,
            tag: row.get(10)?,
            copyright: row.get(11)?,
            link: row.get(12)?,
            expires_at_ms: row.get(13)?,
        };
        Ok(Some((item, meta)))
    }

    const LICENSED_COLS: &'static str =
        "id, version, book, chapter, verse_start, verse_end, reference, source, bible_id, usfm, tag, copyright, link, expires_at_ms, sealed";

    /// A licensed passage that has not expired, with its text.
    pub fn get_licensed_scripture(&self, id: &str, now_ms: i64) -> Result<Option<(ScriptureItem, LicensedMeta)>> {
        let key = self.licensed_key()?;
        let conn = self.main_conn.lock().unwrap_or_else(|e| e.into_inner());
        let sql = format!("SELECT {} FROM licensed_passages WHERE id = ?1 AND expires_at_ms > ?2", Self::LICENSED_COLS);
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut rows = stmt.query(params![id, now_ms])?;
        match rows.next()? {
            Some(row) => self.licensed_row(row, &key),
            None => Ok(None),
        }
    }

    /// Every licensed passage that has not expired.
    pub fn list_licensed_scriptures(&self, now_ms: i64) -> Result<Vec<(ScriptureItem, LicensedMeta)>> {
        let key = self.licensed_key()?;
        let conn = self.main_conn.lock().unwrap_or_else(|e| e.into_inner());
        let sql = format!("SELECT {} FROM licensed_passages WHERE expires_at_ms > ?1 ORDER BY reference ASC", Self::LICENSED_COLS);
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![now_ms], |row| self.licensed_row(row, &key))?;
        Ok(rows.flatten().flatten().collect())
    }

    /// What is known about a licensed passage without opening its text (an export or a schedule item
    /// carries this, never the verses).
    pub fn licensed_meta(&self, id: &str) -> Result<Option<LicensedMeta>> {
        let conn = self.main_conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare_cached(
            "SELECT source, bible_id, usfm, tag, copyright, link, expires_at_ms FROM licensed_passages WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        Ok(match rows.next()? {
            Some(r) => Some(LicensedMeta {
                id: id.to_string(),
                source: r.get(0)?,
                bible_id: r.get(1)?,
                usfm: r.get(2)?,
                tag: r.get(3)?,
                copyright: r.get(4)?,
                link: r.get(5)?,
                expires_at_ms: r.get(6)?,
            }),
            None => None,
        })
    }

    /// The publisher said these are good for another while: renew the lifetime, keep the text as it is.
    pub fn refresh_licensed_ttl(&self, ids: &[String], expires_at_ms: i64) -> Result<usize> {
        let conn = self.main_conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut n = 0;
        for id in ids {
            n += conn.execute("UPDATE licensed_passages SET expires_at_ms = ?1 WHERE id = ?2", params![expires_at_ms, id])?;
        }
        Ok(n)
    }

    pub fn delete_licensed_scripture(&self, id: &str) -> Result<()> {
        let conn = self.main_conn.lock().unwrap_or_else(|e| e.into_inner());
        let _ = conn.execute_batch("PRAGMA secure_delete = ON;");
        conn.execute("DELETE FROM licensed_passages WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Deletes every kept passage, expired or not, for "delete them now" in Settings. Returns how many went.
    pub fn purge_all_licensed(&self) -> Result<usize> {
        let conn = self.main_conn.lock().unwrap_or_else(|e| e.into_inner());
        let _ = conn.execute_batch("PRAGMA secure_delete = ON;");
        conn.execute("DELETE FROM licensed_passages", [])
    }

    /// Deletes every expired passage. Returns how many went.
    pub fn purge_expired_licensed(&self, now_ms: i64) -> Result<usize> {
        let conn = self.main_conn.lock().unwrap_or_else(|e| e.into_inner());
        // Zero the freed pages so the text isn't left lying in the file.
        let _ = conn.execute_batch("PRAGMA secure_delete = ON;");
        conn.execute("DELETE FROM licensed_passages WHERE expires_at_ms <= ?1", params![now_ms])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str) -> ScriptureItem {
        ScriptureItem {
            id: id.into(), book: "John".into(), chapter: 3, verse_start: 16, verse_end: 17, version: "NIV".into(),
            reference: "John 3".into(),
            verses: vec![ScriptureVerse { verse_number: 16, text: "SECRET-LICENSED-TEXT-16".into() }, ScriptureVerse { verse_number: 17, text: "SECRET-LICENSED-TEXT-17".into() }],
            theme_name: None,
        }
    }
    fn meta(id: &str, expires: i64) -> LicensedMeta {
        LicensedMeta { id: id.into(), source: "youversion".into(), bible_id: 111, usfm: "JHN.3.16-17".into(), tag: "NIV".into(),
            copyright: "(c) Biblica".into(), link: "https://www.biblica.com".into(), expires_at_ms: expires }
    }

    #[test]
    fn sealing_round_trips_and_is_bound_to_the_row() {
        let key = random_key().unwrap();
        let sealed = seal(&key, "a", b"hello world").unwrap();
        assert!(!sealed.windows(5).any(|w| w == b"hello"), "the text must not be readable in the blob");
        assert_eq!(open(&key, "a", &sealed).as_deref(), Some(&b"hello world"[..]));
        assert_eq!(open(&key, "b", &sealed), None, "moved to another row");
        assert_eq!(open(&random_key().unwrap(), "a", &sealed), None, "wrong key");
        let mut bad = sealed.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(open(&key, "a", &bad), None, "tampered");
        assert_ne!(seal(&key, "a", b"hello world").unwrap(), sealed, "fresh nonce each time");
    }

    #[test]
    fn key_text_round_trip() {
        let k = random_key().unwrap();
        assert_eq!(decode_key(&encode_key(&k)), Some(k));
        assert_eq!(decode_key("zz"), None);
        assert_eq!(decode_key(&"g".repeat(64)), None);
    }

    #[test]
    fn the_operator_can_delete_every_kept_passage_at_once() {
        let db = Database::in_memory().unwrap();
        db.insert_licensed_scripture(&item("p1"), &meta("p1", 1_000)).unwrap();
        db.insert_licensed_scripture(&item("p2"), &meta("p2", 9_000_000_000_000)).unwrap();
        assert_eq!(db.purge_all_licensed().unwrap(), 2, "one expired, one still good: both go");
        assert!(db.licensed_meta("p1").unwrap().is_none() && db.licensed_meta("p2").unwrap().is_none());
        assert_eq!(db.purge_all_licensed().unwrap(), 0);
    }

    #[test]
    fn stored_text_is_sealed_expires_and_is_purged() {
        let db = Database::in_memory().unwrap();
        db.insert_licensed_scripture(&item("p1"), &meta("p1", 1_000)).unwrap();
        db.insert_licensed_scripture(&item("p2"), &meta("p2", 9_000)).unwrap();

        // Raw rows hold no readable verses.
        {
            let conn = db.main_conn.lock().unwrap();
            let blobs: Vec<Vec<u8>> = conn.prepare("SELECT sealed FROM licensed_passages").unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect();
            assert_eq!(blobs.len(), 2);
            assert!(blobs.iter().all(|b| !b.windows(6).any(|w| w == b"SECRET")));
        }

        let (got, m) = db.get_licensed_scripture("p1", 500).unwrap().unwrap();
        assert_eq!(got.verses[0].text, "SECRET-LICENSED-TEXT-16");
        assert_eq!((m.tag.as_str(), m.bible_id, m.usfm.as_str()), ("NIV", 111, "JHN.3.16-17"));
        assert!(db.get_licensed_scripture("p1", 1_000).unwrap().is_none(), "expired is not served");
        assert_eq!(db.list_licensed_scriptures(5_000).unwrap().len(), 1);

        // Renewing keeps the text and moves the expiry.
        assert_eq!(db.refresh_licensed_ttl(&["p1".into()], 7_000).unwrap(), 1);
        assert_eq!(db.get_licensed_scripture("p1", 5_000).unwrap().unwrap().0.verses[1].text, "SECRET-LICENSED-TEXT-17");

        assert_eq!(db.purge_expired_licensed(8_000).unwrap(), 1, "p1 (renewed to 7000) is now expired");

        assert!(db.licensed_meta("p1").unwrap().is_none());
        assert!(db.licensed_meta("p2").unwrap().is_some());
        db.delete_licensed_scripture("p2").unwrap();
        assert!(db.list_licensed_scriptures(0).unwrap().is_empty());
    }
}
