//! Reading a Bible from a file or a link: USFM (what eBible.org publishes), OSIS XML and Zefania XML, loose or
//! in a zip. The operator pastes a link (or hands over a file); this turns the bytes into chapters of verses.
//!
//! The input is untrusted, so: format is decided from the content, not the name; zips are read with caps on
//! entries and sizes; XML is read by a small reader that never expands entities or loads anything (a DOCTYPE
//! is skipped, `&custom;` stays as text); chapters, verses and verse length are capped; and nothing is written
//! anywhere until it has all parsed. See `docs/bible-import.md`.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use rusqlite::Result as DbResult;

use crate::core::models::{ScriptureItem, ScriptureVerse};
use crate::storage::db::Database;
use crate::storage::freeshow_import::BIBLE_BOOKS_META;

/// Largest file accepted, loose or as a zip (the unpacked text may be larger; see `MAX_UNPACKED`).
pub const MAX_INPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_UNPACKED: u64 = 400 * 1024 * 1024;
const MAX_ZIP_ENTRIES: usize = 400;
const MAX_CHAPTERS: usize = 4000;
const MAX_VERSES_PER_CHAPTER: usize = 400;
const MAX_VERSE_BYTES: usize = 16_000;
const MAX_NAME: usize = 200;
const BIBLE_INFO_PREFIX: &str = "bibleInfo_";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibleImportError(pub String);

impl std::fmt::Display for BibleImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for BibleImportError {}

fn bad<T>(msg: impl Into<String>) -> Result<T, BibleImportError> {
    Err(BibleImportError(msg.into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Usfm,
    Osis,
    Zefania,
    Usx,
    OpenSong,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Usfm => "USFM",
            Format::Osis => "OSIS XML",
            Format::Zefania => "Zefania XML",
            Format::Usx => "USX",
            Format::OpenSong => "OpenSong XML",
        }
    }
}

/// A Bible read from a file: its details and its chapters (every verse of every chapter), version unset.
#[derive(Debug, Clone)]
pub struct ParsedBible {
    pub format: Format,
    pub abbreviation: String,
    pub name: String,
    pub language: String,
    pub license: String,
    pub chapters: Vec<ParsedChapter>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedChapter {
    pub book: String,
    pub chapter: u32,
    pub verses: Vec<ScriptureVerse>,
}

impl ParsedBible {
    pub fn verse_count(&self) -> usize {
        self.chapters.iter().map(|c| c.verses.len()).sum()
    }
}

// ---------------------------------------------------------------------------------------------------------
// Books
// ---------------------------------------------------------------------------------------------------------

/// USFM code and name of the 66 books, in the order of `BIBLE_BOOKS_META`, then the deuterocanon.
const USFM_66: [&str; 66] = [
    "GEN", "EXO", "LEV", "NUM", "DEU", "JOS", "JDG", "RUT", "1SA", "2SA", "1KI", "2KI", "1CH", "2CH", "EZR", "NEH", "EST", "JOB", "PSA", "PRO",
    "ECC", "SNG", "ISA", "JER", "LAM", "EZK", "DAN", "HOS", "JOL", "AMO", "OBA", "JON", "MIC", "NAM", "HAB", "ZEP", "HAG", "ZEC", "MAL", "MAT",
    "MRK", "LUK", "JHN", "ACT", "ROM", "1CO", "2CO", "GAL", "EPH", "PHP", "COL", "1TH", "2TH", "1TI", "2TI", "TIT", "PHM", "HEB", "JAS", "1PE",
    "2PE", "1JN", "2JN", "3JN", "JUD", "REV",
];

const DEUTERO: [(&str, &str); 21] = [
    ("TOB", "Tobit"), ("JDT", "Judith"), ("ESG", "Esther (Greek)"), ("WIS", "Wisdom of Solomon"), ("SIR", "Sirach"), ("BAR", "Baruch"),
    ("LJE", "Letter of Jeremiah"), ("S3Y", "Prayer of Azariah"), ("SUS", "Susanna"), ("BEL", "Bel and the Dragon"), ("1MA", "1 Maccabees"),
    ("2MA", "2 Maccabees"), ("3MA", "3 Maccabees"), ("4MA", "4 Maccabees"), ("1ES", "1 Esdras"), ("2ES", "2 Esdras"), ("MAN", "Prayer of Manasseh"),
    ("PS2", "Psalm 151"), ("DAG", "Additions to Daniel"), ("ODA", "Odes"), ("PSS", "Psalms of Solomon"),
];

/// OSIS book ids that differ from the USFM code.
const OSIS_TO_USFM: [(&str, &str); 52] = [
    ("Gen", "GEN"), ("Exod", "EXO"), ("Lev", "LEV"), ("Num", "NUM"), ("Deut", "DEU"), ("Josh", "JOS"), ("Judg", "JDG"), ("Ruth", "RUT"),
    ("1Sam", "1SA"), ("2Sam", "2SA"), ("1Kgs", "1KI"), ("2Kgs", "2KI"), ("1Chr", "1CH"), ("2Chr", "2CH"), ("Ezra", "EZR"), ("Neh", "NEH"),
    ("Esth", "EST"), ("Job", "JOB"), ("Ps", "PSA"), ("Prov", "PRO"), ("Eccl", "ECC"), ("Song", "SNG"), ("Isa", "ISA"), ("Jer", "JER"),
    ("Lam", "LAM"), ("Ezek", "EZK"), ("Dan", "DAN"), ("Hos", "HOS"), ("Joel", "JOL"), ("Amos", "AMO"), ("Obad", "OBA"), ("Jonah", "JON"),
    ("Mic", "MIC"), ("Nah", "NAM"), ("Hab", "HAB"), ("Zeph", "ZEP"), ("Hag", "HAG"), ("Zech", "ZEC"), ("Mal", "MAL"), ("Matt", "MAT"),
    ("Mark", "MRK"), ("Luke", "LUK"), ("John", "JHN"), ("Acts", "ACT"), ("Rom", "ROM"), ("1Cor", "1CO"), ("2Cor", "2CO"), ("Gal", "GAL"),
    ("Eph", "EPH"), ("Phil", "PHP"), ("Col", "COL"), ("1Thess", "1TH"),
];
const OSIS_TO_USFM_2: [(&str, &str); 23] = [
    ("2Thess", "2TH"), ("1Tim", "1TI"), ("2Tim", "2TI"), ("Titus", "TIT"), ("Phlm", "PHM"), ("Heb", "HEB"), ("Jas", "JAS"), ("1Pet", "1PE"),
    ("2Pet", "2PE"), ("1John", "1JN"), ("2John", "2JN"), ("3John", "3JN"), ("Jude", "JUD"), ("Rev", "REV"), ("EpJer", "LJE"), ("1Macc", "1MA"),
    ("2Macc", "2MA"), ("3Macc", "3MA"), ("4Macc", "4MA"), ("1Esd", "1ES"), ("2Esd", "2ES"), ("PrMan", "MAN"), ("AddEsth", "ESG"),
];

/// Index into the canonical order (the 66, then the deuterocanon) and the name of a USFM book code.
fn book_by_usfm(code: &str) -> Option<(usize, String)> {
    let code = code.trim().to_ascii_uppercase();
    if let Some(i) = USFM_66.iter().position(|c| *c == code) {
        return Some((i, BIBLE_BOOKS_META[i].name.to_string()));
    }
    DEUTERO.iter().position(|(c, _)| *c == code).map(|i| (66 + i, DEUTERO[i].1.to_string()))
}

fn book_by_osis(id: &str) -> Option<(usize, String)> {
    let id = id.trim();
    let usfm = OSIS_TO_USFM
        .iter()
        .chain(OSIS_TO_USFM_2.iter())
        .find(|(o, _)| o.eq_ignore_ascii_case(id))
        .map(|(_, u)| *u)
        .unwrap_or(id);
    book_by_usfm(usfm)
}

/// Zefania numbers books 1..66 in the canonical order; others are found by name.
fn book_by_zefania(number: Option<u32>, name: &str) -> Option<(usize, String)> {
    if let Some(n) = number.filter(|n| (1..=66).contains(n)) {
        let i = (n - 1) as usize;
        return Some((i, BIBLE_BOOKS_META[i].name.to_string()));
    }
    let lower = name.trim().to_lowercase();
    if let Some(i) = BIBLE_BOOKS_META.iter().position(|m| m.name.to_lowercase() == lower || m.aliases.iter().any(|a| *a == lower)) {
        return Some((i, BIBLE_BOOKS_META[i].name.to_string()));
    }
    DEUTERO.iter().position(|(_, n)| n.to_lowercase() == lower).map(|i| (66 + i, DEUTERO[i].1.to_string()))
}

// ---------------------------------------------------------------------------------------------------------
// Collecting verses
// ---------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Collector {
    /// (book index, chapter) -> verse number -> text, so output is in canonical order whatever the file's.
    chapters: BTreeMap<(usize, u32), BTreeMap<u32, String>>,
    names: BTreeMap<usize, String>,
}

impl Collector {
    fn add(&mut self, book: usize, name: &str, chapter: u32, verse: u32, text: &str) -> Result<(), BibleImportError> {
        let text = tidy(text);
        if text.is_empty() || chapter == 0 || verse == 0 || verse > 9999 || chapter > 9999 {
            return Ok(());
        }
        if text.len() > MAX_VERSE_BYTES {
            return bad(format!("{name} {chapter}:{verse} is longer than {MAX_VERSE_BYTES} bytes"));
        }
        self.names.entry(book).or_insert_with(|| name.to_string());
        let ch = self.chapters.entry((book, chapter)).or_default();
        if ch.len() >= MAX_VERSES_PER_CHAPTER && !ch.contains_key(&verse) {
            return bad(format!("{name} {chapter} has more than {MAX_VERSES_PER_CHAPTER} verses"));
        }
        let slot = ch.entry(verse).or_default();
        if !slot.is_empty() {
            slot.push(' ');
        }
        slot.push_str(&text);
        if slot.len() > MAX_VERSE_BYTES {
            return bad(format!("{name} {chapter}:{verse} is longer than {MAX_VERSE_BYTES} bytes"));
        }
        if self.chapters.len() > MAX_CHAPTERS {
            return bad(format!("more than {MAX_CHAPTERS} chapters"));
        }
        Ok(())
    }

    fn finish(self) -> Vec<ParsedChapter> {
        let names = self.names;
        self.chapters
            .into_iter()
            .map(|((book, chapter), verses)| ParsedChapter {
                book: names.get(&book).cloned().unwrap_or_default(),
                chapter,
                verses: verses.into_iter().map(|(n, t)| ScriptureVerse { verse_number: n, text: t }).collect(),
            })
            .collect()
    }
}

/// Collapses whitespace, drops control characters.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() || c == '\u{a0}' || c == '\u{feff}' {
            space = true;
        } else if !c.is_control() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------
// USFM
// ---------------------------------------------------------------------------------------------------------

/// Markers whose whole line is not verse text (headings, titles, notes, introductions, running headers).
fn usfm_skip_line(marker: &str) -> bool {
    let m = marker.trim_end_matches(|c: char| c.is_ascii_digit());
    matches!(
        m,
        "id" | "ide" | "usfm" | "sts" | "rem" | "h" | "toc" | "mt" | "mte" | "imt" | "imte" | "is" | "ip" | "ipi" | "im" | "imi" | "ipq" | "imq" | "ipr"
            | "iq" | "ili" | "iot" | "io" | "iex" | "ib" | "ie" | "s" | "ms" | "mr" | "r" | "sr" | "d" | "sp" | "cl" | "cd" | "cp" | "ca" | "va"
            | "vp" | "restore" | "periph" | "k" | "lit" | "esb" | "esbe" | "cat" | "b"
    )
}

/// USFM inline text with footnotes, cross references, figures, word attributes and other markers removed.
fn clean_usfm_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let Some(c) = rest.chars().next() else { break };
        if c != '\\' {
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        // A marker: `\name`, `\+name`, `\name*`, `\*`.
        let body = &rest[1..];
        if body.starts_with('*') {
            i += 2;
            continue;
        }
        let name_end = body.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '+' || ch == '-')).unwrap_or(body.len());
        let plus = body.starts_with('+');
        let name = body[..name_end].trim_start_matches('+');
        // A nested character style closes with `\+w*`, a plain one with `\w*`.
        let closing = |n: &str| if plus { format!("\\+{n}*") } else { format!("\\{n}*") };
        let after = &body[name_end..];
        let consumed_star = after.starts_with('*');
        let after_marker = if consumed_star { &after[1..] } else { after };
        let advance = |tail: &str| s.len() - tail.len();
        match name {
            // Spans that are not scripture text: skip to their closing marker (or the end of the line).
            "f" | "fe" | "ef" | "x" | "ex" | "fig" | "fm" | "xo" | "xt" if !consumed_star => {
                let close = closing(name);
                i = match after_marker.find(&close) {
                    Some(p) => advance(&after_marker[p + close.len()..]),
                    None => s.len(),
                };
            }
            // Word-level attributes: keep the word, drop `|lemma="..."`.
            "w" | "wg" | "wh" | "wa" | "rb" | "jmp" | "ztext" if !consumed_star => {
                let close = closing(name);
                let (inner, tail) = match after_marker.find(&close) {
                    Some(p) => (&after_marker[..p], &after_marker[p + close.len()..]),
                    None => (after_marker, ""),
                };
                let inner = inner.strip_prefix(' ').unwrap_or(inner);
                out.push_str(&clean_usfm_inline(inner.split('|').next().unwrap_or("")));
                i = advance(tail);
            }
            // Milestones (`\zaln-s | ... \*`, `\k-s ... \*`): nothing to keep.
            n if n.ends_with("-s") || n.ends_with("-e") => {
                i = match after_marker.find("\\*") {
                    Some(p) => advance(&after_marker[p + 2..]),
                    None => advance(after_marker),
                };
            }
            _ => {
                // Any other marker (`\nd`, `\add`, `\wj`, `\p`, ...): drop the marker, keep the text.
                i = advance(after_marker);
                if after_marker.starts_with(' ') && !consumed_star {
                    i += 1;
                }
            }
        }
    }
    out
}

/// One USFM book (or several books concatenated: each `\id` starts one) into `c`.
fn parse_usfm(text: &str, c: &mut Collector) -> Result<usize, BibleImportError> {
    let mut book: Option<(usize, String)> = None;
    let (mut chapter, mut verse) = (0u32, 0u32);
    let mut buf = String::new();
    let mut books = 0usize;
    let flush = |c: &mut Collector, book: &Option<(usize, String)>, chapter: u32, verse: u32, buf: &mut String| -> Result<(), BibleImportError> {
        if let Some((i, name)) = book {
            if verse > 0 {
                c.add(*i, name, chapter, verse, &clean_usfm_inline(buf))?;
            }
        }
        buf.clear();
        Ok(())
    };
    for raw in text.lines() {
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('\\') {
            let end = rest.find(|ch: char| ch.is_whitespace()).unwrap_or(rest.len());
            let marker = &rest[..end];
            let tail = rest[end..].trim_start();
            match marker {
                "id" => {
                    flush(c, &book, chapter, verse, &mut buf)?;
                    book = tail.split_whitespace().next().and_then(book_by_usfm);
                    if book.is_some() {
                        books += 1;
                    }
                    (chapter, verse) = (0, 0);
                }
                "c" => {
                    flush(c, &book, chapter, verse, &mut buf)?;
                    chapter = tail.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
                    verse = 0;
                }
                "v" => {
                    flush(c, &book, chapter, verse, &mut buf)?;
                    let mut parts = tail.splitn(2, char::is_whitespace);
                    let num = parts.next().unwrap_or("");
                    verse = num.chars().take_while(|ch| ch.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
                    buf.push_str(parts.next().unwrap_or(""));
                }
                m if usfm_skip_line(m) => {}
                _ => {
                    // A paragraph or poetry marker, or an inline style starting the line: the line is text.
                    if verse > 0 {
                        let text = if marker.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic()) && !is_inline_start(marker) { tail } else { line };
                        buf.push(' ');
                        buf.push_str(text);
                    }
                }
            }
        } else if verse > 0 {
            buf.push(' ');
            buf.push_str(line);
        }
    }
    flush(c, &book, chapter, verse, &mut buf)?;
    Ok(books)
}

/// Is `\marker` a character style that can begin a line of verse text (so the line is kept whole)?
fn is_inline_start(marker: &str) -> bool {
    let m = marker.trim_start_matches('+').trim_end_matches('*');
    matches!(m, "w" | "wg" | "wh" | "wa" | "nd" | "add" | "wj" | "tl" | "qt" | "sc" | "bk" | "pn" | "k" | "dc" | "ord" | "sig" | "em" | "bd" | "it" | "bdit" | "no" | "sls" | "ndx" | "rb" | "jmp")
}

// ---------------------------------------------------------------------------------------------------------
// A small XML reader: tags, text, CDATA. No entity expansion beyond the five and numeric references, no DTD.
// ---------------------------------------------------------------------------------------------------------

#[derive(Debug)]
enum Ev {
    Start { name: String, attrs: Vec<(String, String)>, empty: bool },
    End { name: String },
    Text(String),
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        match rest.find(';').filter(|e| *e <= 12) {
            Some(e) => {
                let ent = &rest[1..e];
                let ch = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
                    n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
                    _ => None,
                };
                match ch {
                    Some(c) => {
                        out.push(c);
                        rest = &rest[e + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &rest[1..];
                    }
                }
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn local_name(n: &str) -> String {
    n.rsplit(':').next().unwrap_or(n).to_ascii_lowercase()
}

/// Reads the tags and text of `xml` in order. Comments, processing instructions and a DOCTYPE (with any internal
/// subset) are skipped; CDATA is text. Malformed input ends the stream rather than failing: what was read stands.
fn xml_events(xml: &str) -> Vec<Ev> {
    let mut out = Vec::new();
    let b = xml.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let end = xml[i..].find('<').map(|p| i + p).unwrap_or(b.len());
            out.push(Ev::Text(decode_entities(&xml[i..end])));
            i = end;
            continue;
        }
        let rest = &xml[i..];
        if rest.starts_with("<!--") {
            i = rest.find("-->").map(|p| i + p + 3).unwrap_or(b.len());
        } else if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").map(|p| i + p).unwrap_or(b.len());
            out.push(Ev::Text(xml[(i + 9).min(end)..end].to_string()));
            i = (end + 3).min(b.len());
        } else if rest.starts_with("<?") {
            i = rest.find("?>").map(|p| i + p + 2).unwrap_or(b.len());
        } else if rest.starts_with("<!") {
            // <!DOCTYPE ... [ internal subset ]>: skipped whole, never interpreted.
            let mut depth = 0i32;
            let mut j = i + 2;
            while j < b.len() {
                match b[j] {
                    b'[' => depth += 1,
                    b']' => depth -= 1,
                    b'>' if depth <= 0 => break,
                    _ => {}
                }
                j += 1;
            }
            i = (j + 1).min(b.len());
        } else if let Some(stripped) = rest.strip_prefix("</") {
            let end = stripped.find('>').unwrap_or(stripped.len());
            out.push(Ev::End { name: local_name(stripped[..end].trim()) });
            i += 2 + end + 1;
        } else {
            // A start tag; its end is the first '>' outside quotes.
            let mut j = i + 1;
            let mut quote = 0u8;
            while j < b.len() {
                let ch = b[j];
                if quote != 0 {
                    if ch == quote {
                        quote = 0;
                    }
                } else if ch == b'"' || ch == b'\'' {
                    quote = ch;
                } else if ch == b'>' {
                    break;
                }
                j += 1;
            }
            let inner = &xml[i + 1..j.min(b.len())];
            let empty = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let name_end = inner.find(char::is_whitespace).unwrap_or(inner.len());
            let name = local_name(&inner[..name_end]);
            let attrs = parse_attrs(&inner[name_end..]);
            if !name.is_empty() {
                out.push(Ev::Start { name, attrs, empty });
            }
            i = (j + 1).min(b.len());
        }
    }
    out
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while let Some(eq) = rest.find('=') {
        let name = local_name(rest[..eq].trim());
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let body = &after[1..];
        let Some(end) = body.find(q) else { break };
        if !name.is_empty() {
            out.push((name, decode_entities(&body[..end])));
        }
        rest = body[end + 1..].trim_start();
        if out.len() > 64 {
            break;
        }
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

// ---------------------------------------------------------------------------------------------------------
// OSIS
// ---------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Details {
    name: String,
    abbreviation: String,
    language: String,
    license: String,
}

fn parse_osis(xml: &str, c: &mut Collector, d: &mut Details) -> Result<(), BibleImportError> {
    let mut book: Option<(usize, String)> = None;
    let mut chapter = 0u32;
    let mut verse: Option<u32> = None;
    let mut buf = String::new();
    let mut skip = 0usize;
    let mut in_header = false;
    let mut header_field: Option<&'static str> = None;
    let flush = |c: &mut Collector, book: &Option<(usize, String)>, chapter: u32, verse: &mut Option<u32>, buf: &mut String| -> Result<(), BibleImportError> {
        if let (Some((i, name)), Some(v)) = (book, *verse) {
            c.add(*i, name, chapter, v, buf)?;
        }
        buf.clear();
        *verse = None;
        Ok(())
    };
    for ev in xml_events(xml) {
        match ev {
            Ev::Start { name, attrs, empty } => match name.as_str() {
                "osistext" => {
                    if let Some(w) = attr(&attrs, "osisidwork") {
                        d.abbreviation = w.to_string();
                    }
                    if let Some(l) = attr(&attrs, "lang") {
                        d.language = l.to_string();
                    }
                }
                "header" => in_header = !empty,
                "title" | "identifier" | "language" | "rights" | "description" if in_header && !empty => {
                    header_field = Some(match name.as_str() {
                        "title" => "title",
                        "identifier" => "identifier",
                        "language" => "language",
                        _ => "rights",
                    });
                }
                "div" if attr(&attrs, "type") == Some("book") => {
                    flush(c, &book, chapter, &mut verse, &mut buf)?;
                    book = attr(&attrs, "osisid").and_then(book_by_osis);
                    chapter = 0;
                }
                "chapter" => {
                    flush(c, &book, chapter, &mut verse, &mut buf)?;
                    if let Some(id) = attr(&attrs, "osisid").or_else(|| attr(&attrs, "sid")) {
                        let mut parts = id.split('.');
                        if let Some(b) = parts.next().and_then(book_by_osis) {
                            book = Some(b);
                        }
                        chapter = parts.next().and_then(|n| n.parse().ok()).unwrap_or(chapter);
                    }
                }
                "verse" => {
                    if attr(&attrs, "eid").is_some() {
                        flush(c, &book, chapter, &mut verse, &mut buf)?;
                    } else if let Some(id) = attr(&attrs, "osisid").or_else(|| attr(&attrs, "sid")) {
                        flush(c, &book, chapter, &mut verse, &mut buf)?;
                        // "Gen.1.1", "Gen.1.1 Gen.1.2" or "Gen.1.1-Gen.1.3": the first verse names it.
                        let first = id.split([' ', '-']).next().unwrap_or("");
                        let mut parts = first.split('.');
                        if let Some(b) = parts.next().and_then(book_by_osis) {
                            book = Some(b);
                        }
                        if let Some(ch) = parts.next().and_then(|n| n.parse::<u32>().ok()) {
                            chapter = ch;
                        }
                        verse = parts.next().and_then(|n| n.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok());
                        let _ = empty;
                    }
                }
                "note" | "rdg" | "figure" | "catchword" if !empty => skip += 1,
                "title" if !in_header && !empty => skip += 1,
                "lb" | "l" | "p" | "br" => buf.push(' '),
                _ => {}
            },
            Ev::End { name } => match name.as_str() {
                "header" => in_header = false,
                "title" if !in_header => skip = skip.saturating_sub(1),
                "title" | "identifier" | "language" | "rights" | "description" => header_field = None,
                "verse" => flush(c, &book, chapter, &mut verse, &mut buf)?,
                "note" | "rdg" | "figure" | "catchword" => skip = skip.saturating_sub(1),
                "lg" | "l" | "p" => buf.push(' '),
                _ => {}
            },
            Ev::Text(t) => {
                if let Some(field) = header_field {
                    let t = t.trim();
                    if !t.is_empty() {
                        let slot = match field {
                            "title" => &mut d.name,
                            "identifier" => &mut d.abbreviation,
                            "language" => &mut d.language,
                            _ => &mut d.license,
                        };
                        if slot.is_empty() {
                            *slot = t.chars().take(MAX_NAME).collect();
                        }
                    }
                } else if verse.is_some() && skip == 0 {
                    buf.push_str(&t);
                }
            }
        }
        if buf.len() > MAX_VERSE_BYTES * 4 {
            return bad("a verse is far longer than any real verse");
        }
    }
    flush(c, &book, chapter, &mut verse, &mut buf)
}

// ---------------------------------------------------------------------------------------------------------
// Zefania
// ---------------------------------------------------------------------------------------------------------

fn parse_zefania(xml: &str, c: &mut Collector, d: &mut Details) -> Result<(), BibleImportError> {
    let mut book: Option<(usize, String)> = None;
    let mut chapter = 0u32;
    let mut verse: Option<u32> = None;
    let mut buf = String::new();
    let mut skip = 0usize;
    let mut info_field: Option<&'static str> = None;
    for ev in xml_events(xml) {
        match ev {
            Ev::Start { name, attrs, empty } => match name.as_str() {
                "xmlbible" => {
                    if let Some(n) = attr(&attrs, "biblename") {
                        d.name = n.chars().take(MAX_NAME).collect();
                    }
                }
                "title" | "identifier" | "language" | "rights" | "description" if !empty && verse.is_none() => {
                    info_field = Some(match name.as_str() {
                        "title" => "title",
                        "identifier" => "identifier",
                        "language" => "language",
                        _ => "rights",
                    });
                }
                "biblebook" => {
                    book = book_by_zefania(attr(&attrs, "bnumber").and_then(|n| n.trim().parse().ok()), attr(&attrs, "bname").unwrap_or(""));
                    chapter = 0;
                }
                "chapter" => chapter = attr(&attrs, "cnumber").and_then(|n| n.trim().parse().ok()).unwrap_or(0),
                "vers" => {
                    verse = attr(&attrs, "vnumber").and_then(|n| n.trim().chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok());
                    buf.clear();
                    if empty {
                        verse = None;
                    }
                }
                "note" | "caption" if !empty => skip += 1,
                "br" => buf.push(' '),
                _ => {}
            },
            Ev::End { name } => match name.as_str() {
                "title" | "identifier" | "language" | "rights" | "description" => info_field = None,
                "vers" => {
                    if let (Some((i, bname)), Some(v)) = (&book, verse) {
                        c.add(*i, bname, chapter, v, &buf)?;
                    }
                    verse = None;
                    buf.clear();
                }
                "note" | "caption" => skip = skip.saturating_sub(1),
                _ => {}
            },
            Ev::Text(t) => {
                if let Some(field) = info_field {
                    let t = t.trim();
                    if !t.is_empty() {
                        let slot = match field {
                            "title" => &mut d.name,
                            "identifier" => &mut d.abbreviation,
                            "language" => &mut d.language,
                            _ => &mut d.license,
                        };
                        if slot.is_empty() {
                            *slot = t.chars().take(MAX_NAME).collect();
                        }
                    }
                } else if verse.is_some() && skip == 0 {
                    buf.push_str(&t);
                }
            }
        }
        if buf.len() > MAX_VERSE_BYTES * 4 {
            return bad("a verse is far longer than any real verse");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// USX
// ---------------------------------------------------------------------------------------------------------

/// USX paragraph styles that are not verse text (headings, titles, introductions, running headers).
fn usx_skip_para(style: &str) -> bool {
    usfm_skip_line(style)
}

/// One USX book (USX 2 or 3). A zip holds one file per book, so this adds to `c` and reports whether it found a book.
fn parse_usx(xml: &str, c: &mut Collector) -> Result<bool, BibleImportError> {
    let mut book: Option<(usize, String)> = None;
    let mut chapter = 0u32;
    let mut verse: Option<u32> = None;
    let mut buf = String::new();
    let mut skip = 0usize;
    let mut in_book_tag = false;
    let mut para_skips: Vec<bool> = Vec::new();
    let mut found = false;
    let flush = |c: &mut Collector, book: &Option<(usize, String)>, chapter: u32, verse: &mut Option<u32>, buf: &mut String| -> Result<(), BibleImportError> {
        if let (Some((i, name)), Some(v)) = (book, *verse) {
            c.add(*i, name, chapter, v, buf)?;
        }
        buf.clear();
        *verse = None;
        Ok(())
    };
    for ev in xml_events(xml) {
        match ev {
            Ev::Start { name, attrs, empty } => match name.as_str() {
                "book" => {
                    flush(c, &book, chapter, &mut verse, &mut buf)?;
                    book = attr(&attrs, "code").and_then(book_by_usfm);
                    found |= book.is_some();
                    chapter = 0;
                    in_book_tag = !empty;
                }
                "chapter" => {
                    flush(c, &book, chapter, &mut verse, &mut buf)?;
                    if attr(&attrs, "eid").is_none() {
                        chapter = attr(&attrs, "number").and_then(|n| n.trim().parse().ok()).unwrap_or(0);
                    }
                }
                "verse" => {
                    flush(c, &book, chapter, &mut verse, &mut buf)?;
                    if attr(&attrs, "eid").is_none() {
                        verse = attr(&attrs, "number").and_then(|n| n.trim().chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok());
                    }
                }
                "para" => {
                    let skipped = usx_skip_para(attr(&attrs, "style").unwrap_or(""));
                    if !empty {
                        para_skips.push(skipped);
                        if skipped {
                            skip += 1;
                        }
                    }
                }
                "note" | "figure" if !empty => skip += 1,
                "optbreak" | "ms" | "lb" => buf.push(' '),
                _ => {}
            },
            Ev::End { name } => match name.as_str() {
                "book" => in_book_tag = false,
                "para" => {
                    if para_skips.pop() == Some(true) {
                        skip = skip.saturating_sub(1);
                    }
                    buf.push(' ');
                }
                "note" | "figure" => skip = skip.saturating_sub(1),
                _ => {}
            },
            Ev::Text(t) => {
                // The book element's own text is the book's description, not verse text.
                if !in_book_tag && verse.is_some() && skip == 0 {
                    buf.push_str(&t);
                }
            }
        }
        if buf.len() > MAX_VERSE_BYTES * 4 {
            return bad("a verse is far longer than any real verse");
        }
    }
    flush(c, &book, chapter, &mut verse, &mut buf)?;
    Ok(found)
}

// ---------------------------------------------------------------------------------------------------------
// OpenSong
// ---------------------------------------------------------------------------------------------------------

/// OpenSong XML Bible: `<bible><b n="Genesis"><c n="1"><v n="1">text</v>`.
fn parse_opensong(xml: &str, c: &mut Collector, d: &mut Details) -> Result<(), BibleImportError> {
    let mut book: Option<(usize, String)> = None;
    let mut chapter = 0u32;
    let mut verse: Option<u32> = None;
    let mut buf = String::new();
    for ev in xml_events(xml) {
        match ev {
            Ev::Start { name, attrs, empty } => match name.as_str() {
                "bible" => {
                    if let Some(n) = attr(&attrs, "n").or_else(|| attr(&attrs, "name")) {
                        d.name = n.chars().take(MAX_NAME).collect();
                    }
                }
                "b" => {
                    book = attr(&attrs, "n").and_then(|n| book_by_zefania(None, n));
                    chapter = 0;
                }
                "c" => chapter = attr(&attrs, "n").and_then(|n| n.trim().parse().ok()).unwrap_or(0),
                "v" => {
                    buf.clear();
                    verse = if empty { None } else { attr(&attrs, "n").and_then(|n| n.trim().chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()) };
                }
                _ => {}
            },
            Ev::End { name } => {
                if name == "v" {
                    if let (Some((i, bname)), Some(v)) = (&book, verse) {
                        c.add(*i, bname, chapter, v, &buf)?;
                    }
                    verse = None;
                    buf.clear();
                }
            }
            Ev::Text(t) => {
                if verse.is_some() {
                    buf.push_str(&t);
                }
            }
        }
        if buf.len() > MAX_VERSE_BYTES * 4 {
            return bad("a verse is far longer than any real verse");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------------------------

/// Text from bytes: UTF-8 (with or without a BOM) or UTF-16 with a BOM.
fn decode_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let le = bytes[0] == 0xff;
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|p| if le { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) }).collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes.strip_prefix(&[0xef, 0xbb, 0xbf][..]).unwrap_or(bytes)).into_owned()
}

/// What a piece of text is, by what it says.
fn sniff(text: &str) -> Option<Format> {
    let head: String = text.chars().take(4096).collect();
    let lower = head.to_ascii_lowercase();
    if lower.contains("<osis") {
        return Some(Format::Osis);
    }
    if lower.contains("<xmlbible") {
        return Some(Format::Zefania);
    }
    if lower.contains("<usx") {
        return Some(Format::Usx);
    }
    if lower.contains("<bible") && lower.contains("<b n=") {
        return Some(Format::OpenSong);
    }
    let t = text.trim_start();
    if t.starts_with("\\id ") || t.starts_with("\\ide ") || t.starts_with("\\usfm ") || (t.starts_with('\\') && (t.contains("\n\\c ") || t.contains("\n\\v "))) {
        return Some(Format::Usfm);
    }
    None
}

/// A name for the Bible from a file name: letters and digits of the stem, upper case.
fn name_hint(file_name: &str) -> String {
    let stem = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let stem = stem.split('.').next().unwrap_or(stem);
    let mut s: String = stem.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(40).collect::<String>().to_uppercase();
    // "engwebp_usfm.zip" is the Bible "ENGWEBP"; the format is not part of its name.
    for suffix in ["_USFM", "-USFM", "_USX", "-USX", "_OPENSONG", "-OPENSONG", "_OSIS", "-OSIS", "_ZEFANIA", "-ZEFANIA", "_XML", "-XML", "_SFM"] {
        if let Some(stripped) = s.strip_suffix(suffix).filter(|t| !t.is_empty()) {
            s = stripped.to_string();
            break;
        }
    }
    s
}

/// A valid version name: upper case letters, digits, `-` and `_`, at most 40 characters.
pub fn clean_abbreviation(s: &str) -> String {
    s.trim().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(40).collect::<String>().to_uppercase()
}

/// Reads a Bible from `bytes` (a loose file or a zip). `file_name` is only a hint for the abbreviation.
pub fn parse_bible(bytes: &[u8], file_name: &str) -> Result<ParsedBible, BibleImportError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return bad(format!("that file is larger than {} MB", MAX_INPUT_BYTES >> 20));
    }
    let mut texts: Vec<(String, String)> = Vec::new(); // (entry name, text)
    if bytes.starts_with(b"PK\x03\x04") {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| BibleImportError("that isn't a readable zip file".into()))?;
        if zip.len() > MAX_ZIP_ENTRIES {
            return bad(format!("the zip holds more than {MAX_ZIP_ENTRIES} files"));
        }
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|_| BibleImportError("a file in the zip is unreadable".into()))?;
            if f.is_dir() {
                continue;
            }
            let name = f.name().unwrap_or_default().into_owned();
            let lower = name.to_ascii_lowercase();
            if !(lower.ends_with(".usfm") || lower.ends_with(".sfm") || lower.ends_with(".xml") || lower.ends_with(".txt") || lower.ends_with(".osis") || lower.ends_with(".usx")) {
                continue;
            }
            total += f.size();
            if total > MAX_UNPACKED {
                return bad("the zip unpacks to more text than is reasonable for a Bible");
            }
            let mut buf = Vec::new();
            f.by_ref().take(MAX_UNPACKED).read_to_end(&mut buf).map_err(|_| BibleImportError("a file in the zip is unreadable".into()))?;
            texts.push((name, decode_text(&buf)));
        }
    } else {
        texts.push((file_name.to_string(), decode_text(bytes)));
    }

    let mut details = Details::default();
    let mut collector = Collector::default();
    let mut format: Option<Format> = None;
    // One XML Bible, or any number of USFM books.
    for (name, text) in &texts {
        match sniff(text) {
            Some(Format::Osis) if format.is_none() => {
                parse_osis(text, &mut collector, &mut details)?;
                format = Some(Format::Osis);
                break;
            }
            Some(Format::Zefania) if format.is_none() => {
                parse_zefania(text, &mut collector, &mut details)?;
                format = Some(Format::Zefania);
                break;
            }
            Some(Format::OpenSong) if format.is_none() => {
                parse_opensong(text, &mut collector, &mut details)?;
                format = Some(Format::OpenSong);
                break;
            }
            Some(Format::Usx) if format.is_none() || format == Some(Format::Usx) => {
                if parse_usx(text, &mut collector)? {
                    format = Some(Format::Usx);
                }
            }
            Some(Format::Usfm) => {
                if parse_usfm(text, &mut collector)? > 0 {
                    format = Some(Format::Usfm);
                }
            }
            _ => {
                let _ = name;
            }
        }
    }
    let Some(format) = format else {
        return bad("I couldn't recognise that file as a Bible. Supported: USFM, USX, OSIS XML, Zefania XML and OpenSong XML (loose or in a zip).");
    };
    let chapters = collector.finish();
    if chapters.is_empty() {
        return bad("that file has no verses in a form I can read");
    }
    let hint = name_hint(file_name);
    let mut abbreviation = clean_abbreviation(&details.abbreviation);
    if abbreviation.is_empty() {
        abbreviation = hint.clone();
    }
    if abbreviation.is_empty() {
        abbreviation = "BIBLE".to_string();
    }
    let name = if details.name.trim().is_empty() { abbreviation.clone() } else { tidy(&details.name) };
    Ok(ParsedBible { format, abbreviation, name, language: tidy(&details.language), license: tidy(&details.license), chapters })
}

// ---------------------------------------------------------------------------------------------------------
// Installing
// ---------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct Installed {
    pub abbreviation: String,
    pub name: String,
    pub format: &'static str,
    pub chapters: usize,
    pub verses: usize,
}

/// What a Bible about to be installed amounts to, without installing it.
pub fn describe(parsed: &ParsedBible, abbreviation: &str) -> Installed {
    Installed { abbreviation: abbreviation.to_string(), name: parsed.name.clone(), format: parsed.format.label(), chapters: parsed.chapters.len(), verses: parsed.verse_count() }
}

/// True when a Bible with this abbreviation is already installed.
pub fn is_installed(db: &Database, abbreviation: &str) -> DbResult<bool> {
    Ok(db.get_installed_bibles()?.iter().any(|b| b.id.eq_ignore_ascii_case(abbreviation)))
}

/// Writes `parsed` into the library under `abbreviation` (replacing an installed one of that name when
/// `replace`), with the same chapter ids the plugin host gives, so it is the same rows however it arrived.
pub fn install(db: &Database, parsed: &ParsedBible, abbreviation: &str, replace: bool, source: &str) -> Result<Installed, BibleImportError> {
    let abbr = clean_abbreviation(abbreviation);
    if abbr.is_empty() {
        return bad("the Bible needs an abbreviation (letters and digits)");
    }
    let installed = is_installed(db, &abbr).map_err(|e| BibleImportError(format!("database error: {e}")))?;
    if installed && !replace {
        return bad(format!("{abbr} is already installed"));
    }
    let items: Vec<ScriptureItem> = parsed
        .chapters
        .iter()
        .map(|c| {
            let (start, end) = (c.verses.first().map(|v| v.verse_number).unwrap_or(1), c.verses.last().map(|v| v.verse_number).unwrap_or(1));
            ScriptureItem {
                id: format!("scrip_{}", uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, format!("{}:{}:{}-{}:{}", c.book, c.chapter, start, end, abbr).as_bytes())),
                reference: format!("{} {}", c.book, c.chapter),
                book: c.book.clone(),
                chapter: c.chapter,
                verse_start: start,
                verse_end: end,
                version: abbr.clone(),
                verses: c.verses.clone(),
                theme_name: None,
            }
        })
        .collect();
    let db_err = |e: rusqlite::Error| BibleImportError(format!("database error: {e}"));
    if installed {
        db.delete_installed_bible(&abbr).map_err(db_err)?;
    }
    db.insert_scriptures_batch(&items).map_err(db_err)?;
    let info = serde_json::json!({
        "abbreviation": abbr, "name": parsed.name, "language": parsed.language, "license": parsed.license, "attribution": source,
    });
    db.set_setting(&format!("{BIBLE_INFO_PREFIX}{abbr}"), &info.to_string()).map_err(db_err)?;
    Ok(Installed { abbreviation: abbr, name: parsed.name.clone(), format: parsed.format.label(), chapters: items.len(), verses: parsed.verse_count() })
}

#[cfg(test)]
mod tests;
