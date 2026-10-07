use serde::{Deserialize, Serialize};

pub fn normalize_language_name(lang: &str) -> String {
    let lower = lang.trim().to_lowercase();
    if lower.contains("eng") {
        "English".to_string()
    } else if lower.contains("spa") || lower.contains("esp") {
        "Spanish".to_string()
    } else if lower.contains("fra") || lower.contains("fre") {
        "French".to_string()
    } else if lower.contains("deu") || lower.contains("ger") {
        "German".to_string()
    } else if lower.contains("ukr") {
        "Ukrainian".to_string()
    } else if lower.contains("rus") {
        "Russian".to_string()
    } else if lower.contains("por") {
        "Portuguese".to_string()
    } else if lower.contains("chi") || lower.contains("zho") || lower.contains("cmn") {
        "Chinese".to_string()
    } else if lower.contains("lat") {
        "Latin".to_string()
    } else if lower.contains("grc") || lower.contains("ell") || lower.contains("greek") {
        "Greek".to_string()
    } else if lower.contains("heb") || lower.contains("hebrew") {
        "Hebrew".to_string()
    } else if lower.contains("ita") || lower.contains("italian") {
        "Italian".to_string()
    } else if lower.contains("kor") || lower.contains("korean") {
        "Korean".to_string()
    } else if lower.contains("tgl") || lower.contains("fil") || lower.contains("tagalog") {
        "Tagalog".to_string()
    } else if lower.contains("swe") || lower.contains("swedish") {
        "Swedish".to_string()
    } else if lower.contains("nor") || lower.contains("norwegian") {
        "Norwegian".to_string()
    } else if lower.contains("dan") || lower.contains("danish") {
        "Danish".to_string()
    } else if lower.contains("nld") || lower.contains("dut") || lower.contains("dutch") {
        "Dutch".to_string()
    } else if lower.contains("afr") || lower.contains("afrikaans") {
        "Afrikaans".to_string()
    } else if lower.contains("ara") || lower.contains("arabic") {
        "Arabic".to_string()
    } else if lower.contains("vie") || lower.contains("vietnamese") {
        "Vietnamese".to_string()
    } else if lower.contains("hin") || lower.contains("hindi") {
        "Hindi".to_string()
    } else if lower.contains("ces") || lower.contains("cze") || lower.contains("czech") {
        "Czech".to_string()
    } else if lower.contains("pol") || lower.contains("polish") {
        "Polish".to_string()
    } else if lower.contains("ron") || lower.contains("rum") || lower.contains("romanian") {
        "Romanian".to_string()
    } else if lower.contains("hun") || lower.contains("hungarian") {
        "Hungarian".to_string()
    } else if lower.is_empty() {
        "English".to_string()
    } else {
        let mut c = lang.chars();
        match c.next() {
            None => "Other".to_string(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        }
    }
}


#[derive(Debug, Deserialize)]
struct BollsVerse {
    book: u32,
    chapter: u32,
    verse: u32,
    text: String,
}

pub static CANONICAL_BOOKS: &[&str] = &[
    "Genesis", "Exodus", "Leviticus", "Numbers", "Deuteronomy",
    "Joshua", "Judges", "Ruth", "1 Samuel", "2 Samuel",
    "1 Kings", "2 Kings", "1 Chronicles", "2 Chronicles", "Ezra",
    "Nehemiah", "Esther", "Job", "Psalms", "Proverbs",
    "Ecclesiastes", "Song of Solomon", "Isaiah", "Jeremiah", "Lamentations",
    "Ezekiel", "Daniel", "Hosea", "Joel", "Amos",
    "Obadiah", "Jonah", "Micah", "Nahum", "Habakkuk",
    "Zephaniah", "Haggai", "Zechariah", "Malachi",
    "Matthew", "Mark", "Luke", "John", "Acts",
    "Romans", "1 Corinthians", "2 Corinthians", "Galatians", "Ephesians",
    "Philippians", "Colossians", "1 Thessalonians", "2 Thessalonians",
    "1 Timothy", "2 Timothy", "Titus", "Philemon", "Hebrews",
    "James", "1 Peter", "2 Peter", "1 John", "2 John",
    "3 John", "Jude", "Revelation"
];

use crate::core::models::{ScriptureItem, ScriptureVerse};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnlineBibleCatalogItem {
    pub id: String,
    pub abbreviation: Option<String>,
    pub name: String,
    pub language: Option<String>,
    pub source: Option<String>,
    pub source_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BibleApiResponse {
    reference: String,
    verses: Vec<BibleApiVerse>,
    translation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BibleApiVerse {
    book_name: String,
    chapter: u32,
    verse: u32,
    text: String,
}

pub struct BibleBookMeta {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub chapters: u32,
}

pub static BIBLE_BOOKS_META: &[BibleBookMeta] = &[
    BibleBookMeta { name: "Genesis", aliases: &["genesis", "gen", "ge", "gn", "genisis", "geneses", "genises", "genisus", "gns"], chapters: 50 },
    BibleBookMeta { name: "Exodus", aliases: &["exodus", "exod", "exo", "ex", "exodous", "exodes", "exudus", "exodis", "exd"], chapters: 40 },
    BibleBookMeta { name: "Leviticus", aliases: &["leviticus", "lev", "le", "lv", "levitacus", "leviticos", "levitcus", "lebiticus", "lvt"], chapters: 27 },
    BibleBookMeta { name: "Numbers", aliases: &["numbers", "num", "nu", "nm", "nb", "numers", "numbes", "number", "nbr"], chapters: 36 },
    BibleBookMeta { name: "Deuteronomy", aliases: &["deuteronomy", "deut", "deu", "dt", "de", "deuteronmy", "dueteronomy", "deutronomy", "deuteronemy", "dueteronmy", "deuteronamy", "deuteromony", "duet", "dtr"], chapters: 34 },
    BibleBookMeta { name: "Joshua", aliases: &["joshua", "josh", "jos", "jsh", "joshu", "joshau", "joshuaa"], chapters: 24 },
    BibleBookMeta { name: "Judges", aliases: &["judges", "judg", "jdg", "jg", "jugdes", "judjes", "judge", "jdgs"], chapters: 21 },
    BibleBookMeta { name: "Ruth", aliases: &["ruth", "rth", "ru", "rut", "rhut"], chapters: 4 },
    BibleBookMeta { name: "1 Samuel", aliases: &["1 samuel", "1samuel", "1 sam", "1sam", "1 s", "1s", "1sa", "1 samual", "1samual", "1 saml", "1saml", "1 sm", "1sm"], chapters: 31 },
    BibleBookMeta { name: "2 Samuel", aliases: &["2 samuel", "2samuel", "2 sam", "2sam", "2 s", "2s", "2sa", "2 samual", "2samual", "2 saml", "2saml", "2 sm", "2sm"], chapters: 24 },
    BibleBookMeta { name: "1 Kings", aliases: &["1 kings", "1kings", "1 kgs", "1kgs", "1 ki", "1ki", "1 k", "1k", "1 king", "1king", "1 kng", "1kng", "1 knigs", "1knigs"], chapters: 22 },
    BibleBookMeta { name: "2 Kings", aliases: &["2 kings", "2kings", "2 kgs", "2kgs", "2 ki", "2ki", "2 k", "2k", "2 king", "2king", "2 kng", "2kng", "2 knigs", "2knigs"], chapters: 25 },
    BibleBookMeta { name: "1 Chronicles", aliases: &["1 chronicles", "1chronicles", "1 chron", "1chron", "1 chr", "1chr", "1 ch", "1ch", "1 chronicals", "1chronicals", "1 chronicle", "1chronicle", "1 cronicles", "1cronicles", "1 cornicles", "1cornicles", "1 chro", "1chro"], chapters: 29 },
    BibleBookMeta { name: "2 Chronicles", aliases: &["2 chronicles", "2chronicles", "2 chron", "2chron", "2 chr", "2chr", "2 ch", "2ch", "2 chronicals", "2chronicals", "2 chronicle", "2chronicle", "2 cronicles", "2cronicles", "2 cornicles", "2cornicles", "2 chro", "2chro"], chapters: 36 },
    BibleBookMeta { name: "Ezra", aliases: &["ezra", "ezr", "esra", "ezrah", "ezar"], chapters: 10 },
    BibleBookMeta { name: "Nehemiah", aliases: &["nehemiah", "neh", "ne", "nehemia", "nehemya", "nehamiah", "nehimiah", "nehamia", "nhm"], chapters: 13 },
    BibleBookMeta { name: "Esther", aliases: &["esther", "esth", "es", "ester", "est", "hesther"], chapters: 10 },
    BibleBookMeta { name: "Job", aliases: &["job", "jb", "jobe"], chapters: 42 },
    BibleBookMeta { name: "Psalms", aliases: &["psalms", "psalm", "psa", "ps", "pslam", "pslams", "psams", "psam", "salms", "salm", "psm", "psms", "pss"], chapters: 150 },
    BibleBookMeta { name: "Proverbs", aliases: &["proverbs", "prov", "pro", "pr", "proverb", "proberbs", "prv", "prvb", "prvbs"], chapters: 31 },
    BibleBookMeta { name: "Ecclesiastes", aliases: &["ecclesiastes", "eccl", "ecc", "ec", "ecclesiasties", "ecclisiastes", "eclesiastes", "ecclesiastis", "ecclesiates", "eccleciastes", "eccles", "eccls"], chapters: 12 },
    BibleBookMeta { name: "Song of Solomon", aliases: &["song of solomon", "song of songs", "song", "sos", "canticles", "canticle", "song of soloman", "songs of solomon", "songs of soloman", "song of solmon", "solomon", "soloman"], chapters: 8 },
    BibleBookMeta { name: "Isaiah", aliases: &["isaiah", "isa", "is", "isiah", "isaias", "issiah", "isaha", "iasiah"], chapters: 66 },
    BibleBookMeta { name: "Jeremiah", aliases: &["jeremiah", "jer", "jr", "jeramiah", "jerimiah", "jeremia", "jeramyah", "jrm"], chapters: 52 },
    BibleBookMeta { name: "Lamentations", aliases: &["lamentations", "lam", "la", "lamentation", "lamentaions", "lementations", "lamatations", "lamn"], chapters: 5 },
    BibleBookMeta { name: "Ezekiel", aliases: &["ezekiel", "ezek", "eze", "ezk", "ezekial", "eziekiel", "ezekel", "ezequiel", "ezkl"], chapters: 48 },
    BibleBookMeta { name: "Daniel", aliases: &["daniel", "dan", "da", "dn", "danial", "daneil", "dnl"], chapters: 12 },
    BibleBookMeta { name: "Hosea", aliases: &["hosea", "hos", "ho", "hoseah", "hosia", "hoshea", "hsa"], chapters: 14 },
    BibleBookMeta { name: "Joel", aliases: &["joel", "joe", "jl", "jole"], chapters: 3 },
    BibleBookMeta { name: "Amos", aliases: &["amos", "am", "amoz", "amoss"], chapters: 9 },
    BibleBookMeta { name: "Obadiah", aliases: &["obadiah", "obad", "ob", "obediah", "obadya", "obadia", "obd"], chapters: 1 },
    BibleBookMeta { name: "Jonah", aliases: &["jonah", "jon", "jnh", "jona", "jonahh"], chapters: 4 },
    BibleBookMeta { name: "Micah", aliases: &["micah", "mic", "mc", "mica", "micaha", "micha"], chapters: 7 },
    BibleBookMeta { name: "Nahum", aliases: &["nahum", "nah", "na", "naham", "nahom"], chapters: 3 },
    BibleBookMeta { name: "Habakkuk", aliases: &["habakkuk", "hab", "hb", "habakuk", "habbakuk", "habakkuc", "habbakkuk", "habacuc", "habk", "hbk"], chapters: 3 },
    BibleBookMeta { name: "Zephaniah", aliases: &["zephaniah", "zeph", "zep", "zp", "zephania", "zephanya", "zepheniah", "zph"], chapters: 3 },
    BibleBookMeta { name: "Haggai", aliases: &["haggai", "hag", "hg", "hagai", "hagaii", "haggia"], chapters: 2 },
    BibleBookMeta { name: "Zechariah", aliases: &["zechariah", "zech", "zec", "zc", "zecharaiah", "zecharia", "zachariah", "zacharias", "zachary", "zch"], chapters: 14 },
    BibleBookMeta { name: "Malachi", aliases: &["malachi", "mal", "ml", "malakai", "malachai", "malachy", "malaki", "mlc"], chapters: 4 },
    BibleBookMeta { name: "Matthew", aliases: &["matthew", "matt", "mat", "mt", "mathew", "matthw", "matthews", "mathews", "mtw"], chapters: 28 },
    BibleBookMeta { name: "Mark", aliases: &["mark", "mrk", "mk", "marck", "marcus"], chapters: 16 },
    BibleBookMeta { name: "Luke", aliases: &["luke", "luk", "lk", "lukas", "luc"], chapters: 24 },
    BibleBookMeta { name: "John", aliases: &["john", "jhn", "jn", "j", "jonh", "jhon", "johan"], chapters: 21 },
    BibleBookMeta { name: "Acts", aliases: &["acts", "act", "ac", "actes", "ats"], chapters: 28 },
    BibleBookMeta { name: "Romans", aliases: &["romans", "roman", "rom", "ro", "rm", "romens", "romas", "rmn"], chapters: 16 },
    BibleBookMeta { name: "1 Corinthians", aliases: &["1 corinthians", "1corinthians", "1 cor", "1cor", "1 co", "1co", "1 corinthian", "1corinthian", "1 corinthan", "1corinthan", "1 corintians", "1corintians", "1 cr", "1cr"], chapters: 16 },
    BibleBookMeta { name: "2 Corinthians", aliases: &["2 corinthians", "2corinthians", "2 cor", "2cor", "2 co", "2co", "2 corinthian", "2corinthian", "2 corinthan", "2corinthan", "2 corintians", "2corintians", "2 cr", "2cr"], chapters: 13 },
    BibleBookMeta { name: "Galatians", aliases: &["galatians", "gal", "ga", "galation", "galations", "galatian", "galatins", "gallatians", "glt"], chapters: 6 },
    BibleBookMeta { name: "Ephesians", aliases: &["ephesians", "ephes", "eph", "ep", "ephesian", "ephesiens", "ephecians", "ephessians", "epesians", "ephs"], chapters: 6 },
    BibleBookMeta { name: "Philippians", aliases: &["philippians", "phil", "php", "pp", "philipians", "phillippians", "phillipians", "philipian", "philippian", "phillipian", "phlp"], chapters: 4 },
    BibleBookMeta { name: "Colossians", aliases: &["colossians", "col", "co", "collosians", "colosians", "collossians", "colossian", "collosian", "colosian", "cls", "cols"], chapters: 4 },
    BibleBookMeta { name: "1 Thessalonians", aliases: &["1 thessalonians", "1thessalonians", "1 thess", "1thess", "1 th", "1th", "1 thesalonians", "1thesalonians", "1 thessalonian", "1thessalonian", "1 theselonians", "1 thes", "1thes", "1 thss", "1thss"], chapters: 5 },
    BibleBookMeta { name: "2 Thessalonians", aliases: &["2 thessalonians", "2thessalonians", "2 thess", "2thess", "2 th", "2th", "2 thesalonians", "2thesalonians", "2 thessalonian", "2thessalonian", "2 theselonians", "2 thes", "2thes", "2 thss", "2thss"], chapters: 3 },
    BibleBookMeta { name: "1 Timothy", aliases: &["1 timothy", "1timothy", "1 tim", "1tim", "1 ti", "1ti", "1 timothey", "1timothey", "1 timothi", "1timothi", "1 tm", "1tm", "1 tmt", "1tmt"], chapters: 6 },
    BibleBookMeta { name: "2 Timothy", aliases: &["2 timothy", "2timothy", "2 tim", "2tim", "2 ti", "2ti", "2 timothey", "2timothey", "2 timothi", "2timothi", "2 tm", "2tm", "2 tmt", "2tmt"], chapters: 4 },
    BibleBookMeta { name: "Titus", aliases: &["titus", "tit", "ti", "titos", "titas", "tts"], chapters: 3 },
    BibleBookMeta { name: "Philemon", aliases: &["philemon", "philem", "phm", "pm", "philemone", "philimon", "filemon", "phlm"], chapters: 1 },
    BibleBookMeta { name: "Hebrews", aliases: &["hebrews", "hebrew", "heb", "he", "hebrws", "hebreus", "hbr"], chapters: 13 },
    BibleBookMeta { name: "James", aliases: &["james", "jas", "jm", "ja", "jams", "jaems", "jms"], chapters: 5 },
    BibleBookMeta { name: "1 Peter", aliases: &["1 peter", "1peter", "1 pet", "1pet", "1 pt", "1pt", "1 pe", "1pe", "1 petre", "1petre", "1 peeter", "1peeter", "1 ptr", "1ptr"], chapters: 5 },
    BibleBookMeta { name: "2 Peter", aliases: &["2 peter", "2peter", "2 pet", "2pet", "2 pt", "2pt", "2 pe", "2pe", "2 petre", "2petre", "2 peeter", "2peeter", "2 ptr", "2ptr"], chapters: 3 },
    BibleBookMeta { name: "1 John", aliases: &["1 john", "1john", "1 jhn", "1jhn", "1 jn", "1jn", "1 j", "1j", "1 jonh", "1jonh", "1 jhon", "1jhon"], chapters: 5 },
    BibleBookMeta { name: "2 John", aliases: &["2 john", "2john", "2 jhn", "2jhn", "2 jn", "2jn", "2 j", "2j", "2 jonh", "2jonh", "2 jhon", "2jhon"], chapters: 1 },
    BibleBookMeta { name: "3 John", aliases: &["3 john", "3john", "3 jhn", "3jhn", "3 jn", "3jn", "3 j", "3j", "3 jonh", "3jonh", "3 jhon", "3jhon"], chapters: 1 },
    BibleBookMeta { name: "Jude", aliases: &["jude", "jud", "jd", "judas"], chapters: 1 },
    BibleBookMeta { name: "Revelation", aliases: &["revelation", "revelations", "rev", "re", "revalation", "revalations", "revilation", "revilations", "revelatoin", "rvl", "rvltn"], chapters: 22 },
];

pub fn find_book_meta(raw: &str) -> Option<&'static BibleBookMeta> {
    let clean = raw.trim().to_lowercase().replace('.', "");
    // Try exact alias or name match
    for meta in BIBLE_BOOKS_META {
        if meta.name.to_lowercase() == clean || meta.aliases.iter().any(|a| *a == clean) {
            return Some(meta);
        }
    }
    // Try prefix matching if raw has numbers / chapter suffix (e.g. "John 3:16" -> "John")
    for meta in BIBLE_BOOKS_META {
        for alias in meta.aliases {
            if let Some(rest) = clean.strip_prefix(alias) {
                if rest.is_empty() || rest.starts_with(' ') || rest.starts_with(':') || rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    return Some(meta);
                }
            }
        }
    }
    None
}

pub fn find_book_meta_exact(raw: &str) -> Option<&'static BibleBookMeta> {
    let clean = raw.trim().to_lowercase().replace('.', "");
    BIBLE_BOOKS_META.iter().find(|meta| meta.name.to_lowercase() == clean || meta.aliases.iter().any(|a| *a == clean))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedScriptureRef {
    pub is_reference: bool,
    pub raw_query: String,
    pub book: String,
    pub chapter: Option<u32>,
    pub verse_start: Option<u32>,
    pub verse_end: Option<u32>,
    pub has_specific_verse: bool,
    pub version: Option<String>,
    pub formatted: String,
}

pub static KNOWN_BIBLE_VERSIONS: &[&str] = &[
    // Major English Translations & Modern Standards
    "kjv", "nkjv", "esv", "niv", "nlt", "nasb", "nasb95", "nasb2020",
    "csb", "csb17", "hcsb", "msg", "amp", "ampc", "web", "asv", "bsb", "bbe",
    "nrsv", "nrsvue", "rsv", "net", "cev", "gnt", "gnb", "nirv", "icb",
    "mev", "tlv", "ylt", "dby", "darby", "drb", "douay", "kj21", "leb",
    "tpt", "voice", "isv", "nab", "nabre", "njb", "cjb", "wbt", "erv",
    "ncv", "ehv",
    // Common Version Typos & Transpositions
    "nkvj", "kvj", "evs", "ntl", "nsab", "cbs", "nrsvu", "mgs",
    // Revision Years & Historical Suffixes
    "kjv1900", "niv84", "niv2011",
    // Spanish Translations
    "rvr", "rvr1960", "rvr1909", "rvr1995", "rvr60", "rvr09", "rva",
    "nvi", "dhh", "lbla", "nbla", "tla", "pdt",
    // French Translations
    "lsg", "bds", "pdv", "nbs", "segond",
    // German Translations
    "lut", "lut1912", "elb", "elberfelder", "sch", "sch2000", "ngu", "hfa",
    // Portuguese Translations
    "alm", "almeida", "arc", "ara", "ntlh",
    // Italian Translations
    "cei", "riveduta", "luzzi",
    // Latin & Classical
    "cle", "vulg", "vulgata", "vulgate", "lxxe", "septuagint",
    // Russian, Ukrainian & Slavic
    "synod", "rst", "ubio", "ogienko", "ubg",
    // World Languages (Asian, Tagalog, Romanian, Arabic)
    "cuv", "cuvs", "cuvt", "tag", "krv", "jlb", "svd", "cornilescu",
];

pub fn parse_scripture_reference(input: &str) -> Option<ParsedScriptureRef> {
    let clean = input.trim();
    if clean.is_empty() {
        return None;
    }

    let mut working_str = clean.to_string();
    let mut detected_version = None;

    let tokens: Vec<&str> = working_str.split_whitespace().collect();
    if tokens.len() > 1 {
        let last_token = tokens.last().unwrap().trim_matches(|c| c == '(' || c == ')' || c == '[' || c == ']').to_lowercase();
        if KNOWN_BIBLE_VERSIONS.contains(&last_token.as_str()) {
            detected_version = Some(last_token.to_uppercase());
            let without_last = tokens[..tokens.len() - 1].join(" ");
            working_str = without_last;
        }
    }

    let lower_working = working_str.to_lowercase().replace('.', "");
    let mut matched_meta = None;
    let mut matched_alias_len = 0;

    // First try exact whole string match as a book
    for meta in BIBLE_BOOKS_META {
        if meta.name.to_lowercase() == lower_working || meta.aliases.iter().any(|a| *a == lower_working) {
            return Some(ParsedScriptureRef {
                is_reference: true,
                raw_query: clean.to_string(),
                book: meta.name.to_string(),
                chapter: None,
                verse_start: None,
                verse_end: None,
                has_specific_verse: false,
                version: detected_version,
                formatted: meta.name.to_string(),
            });
        }
    }

    // Next find the longest matching book alias prefix
    for meta in BIBLE_BOOKS_META {
        for alias in meta.aliases {
            if let Some(rest) = lower_working.strip_prefix(alias) {
                if (rest.is_empty() || rest.starts_with(' ') || rest.starts_with(':') || rest.chars().next().is_some_and(|c| c.is_ascii_digit()))
                    && alias.len() > matched_alias_len
                {
                    matched_alias_len = alias.len();
                    matched_meta = Some(meta);
                }
            }
        }
    }

    let meta = matched_meta?;
    let rest_raw = if matched_alias_len <= working_str.len() && working_str.is_char_boundary(matched_alias_len) {
        &working_str[matched_alias_len..]
    } else {
        working_str
            .char_indices()
            .find(|&(idx, _)| idx >= matched_alias_len)
            .map(|(idx, _)| &working_str[idx..])
            .unwrap_or("")
    };
    let rest = rest_raw.trim().trim_start_matches(':').trim();

    if rest.is_empty() {
        return Some(ParsedScriptureRef {
            is_reference: true,
            raw_query: clean.to_string(),
            book: meta.name.to_string(),
            chapter: None,
            verse_start: None,
            verse_end: None,
            has_specific_verse: false,
            version: detected_version,
            formatted: meta.name.to_string(),
        });
    }

    let mut chapter = None;
    let mut verse_start = None;
    let mut verse_end = None;
    let mut has_specific_verse = false;

    if let Some(colon_pos) = rest.find(':') {
        let ch_str = rest[..colon_pos].trim();
        let v_str = rest[colon_pos + 1..].trim();
        if let Ok(ch) = ch_str.parse::<u32>() {
            chapter = Some(ch);
        }
        if let Some((dash_pos, dash)) = v_str.char_indices().find(|&(_, c)| c == '-' || c == '–') {
            let vs_str = v_str[..dash_pos].trim();
            let ve_str = v_str[dash_pos + dash.len_utf8()..].trim();
            if let Ok(vs) = vs_str.parse::<u32>() {
                verse_start = Some(vs);
                has_specific_verse = true;
            }
            if let Ok(ve) = ve_str.parse::<u32>() {
                verse_end = Some(ve);
            } else {
                verse_end = verse_start;
            }
        } else if let Ok(vs) = v_str.parse::<u32>() {
            verse_start = Some(vs);
            verse_end = Some(vs);
            has_specific_verse = true;
        }
    } else {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() >= 2 {
            if let Ok(ch) = parts[0].parse::<u32>() {
                chapter = Some(ch);
            }
            let v_part = parts[1];
            if let Some((dash_pos, dash)) = v_part.char_indices().find(|&(_, c)| c == '-' || c == '–') {
                let vs_str = v_part[..dash_pos].trim();
                let ve_str = v_part[dash_pos + dash.len_utf8()..].trim();
                if let Ok(vs) = vs_str.parse::<u32>() {
                    verse_start = Some(vs);
                    has_specific_verse = true;
                }
                if let Ok(ve) = ve_str.parse::<u32>() {
                    verse_end = Some(ve);
                } else {
                    verse_end = verse_start;
                }
            } else if let Ok(vs) = v_part.parse::<u32>() {
                verse_start = Some(vs);
                verse_end = Some(vs);
                has_specific_verse = true;
            }
        } else if let Ok(ch) = rest.parse::<u32>() {
            chapter = Some(ch);
        }
    }

    let mut formatted = meta.name.to_string();
    if let Some(ch) = chapter {
        formatted.push_str(&format!(" {}", ch));
        if let Some(vs) = verse_start {
            formatted.push_str(&format!(":{}", vs));
            if let Some(ve) = verse_end {
                if ve != vs {
                    formatted.push_str(&format!("-{}", ve));
                }
            }
        }
    }

    Some(ParsedScriptureRef {
        is_reference: true,
        raw_query: clean.to_string(),
        book: meta.name.to_string(),
        chapter,
        verse_start,
        verse_end,
        has_specific_verse,
        version: detected_version,
        formatted,
    })
}

pub struct FreeShowImporter;

impl FreeShowImporter {
    /// Parse FreeShow `.fsb` JSON format into ScriptureItems
    pub fn import_fsb_content(json_str: &str) -> Result<Vec<ScriptureItem>, Box<dyn std::error::Error>> {
        let root: serde_json::Value = serde_json::from_str(json_str)?;

        // FreeShow .fsb is typically [id, bibleObj] or { name, books }
        let bible_obj = if let Some(arr) = root.as_array() {
            arr.get(1).unwrap_or(&root)
        } else {
            &root
        };

        let version_name = bible_obj
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("FreeShow Bible")
            .to_string();

        let mut scriptures = Vec::new();

        if let Some(books) = bible_obj.get("books").and_then(|b| b.as_array()) {
            for book in books {
                let book_name = book
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("Book")
                    .to_string();

                if let Some(chapters) = book.get("chapters").and_then(|c| c.as_array()) {
                    for chapter in chapters {
                        let chapter_num = chapter
                            .get("number")
                            .and_then(|n| n.as_u64())
                            .unwrap_or(1) as u32;

                        let mut verses = Vec::new();
                        if let Some(verse_list) = chapter.get("verses").and_then(|v| v.as_array()) {
                            for v in verse_list {
                                let v_num = v.get("number").and_then(|n| n.as_u64()).unwrap_or(1) as u32;
                                let v_text = v
                                    .get("text")
                                    .or_else(|| v.get("value"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                verses.push(ScriptureVerse {
                                    verse_number: v_num,
                                    text: v_text,
                                });
                            }
                        }

                        if !verses.is_empty() {
                            let first = verses.first().map(|v| v.verse_number).unwrap_or(1);
                            let last = verses.last().map(|v| v.verse_number).unwrap_or(1);

                            let mut item = ScriptureItem::new(&book_name, chapter_num, first, last, &version_name);
                            item.id = format!(
                                "scrip_{}",
                                uuid::Uuid::new_v5(
                                    &uuid::Uuid::NAMESPACE_OID,
                                    format!("{}:{}:{}-{}:{}", book_name, chapter_num, first, last, version_name).as_bytes()
                                )
                            );
                            item.verses = verses;
                            scriptures.push(item);
                        }
                    }
                }
            }
        }

        Ok(scriptures)
    }

    /// Fetch all chapters for an entire book from the online Bible API
    pub async fn fetch_online_book(
        meta: &BibleBookMeta,
        translation: Option<&str>,
    ) -> Result<Vec<ScriptureItem>, Box<dyn std::error::Error + Send + Sync>> {
        let trans = translation.unwrap_or("kjv").to_lowercase();
        let client = reqwest::Client::builder()
            .user_agent("OpenSanctuary/1.0 (Linux; Worship Presentation Engine; https://github.com/opensanctuary)")
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let mut fetch_tasks = Vec::new();
        for ch in 1..=meta.chapters {
            let client_ref = client.clone();
            let book_name = meta.name.to_string();
            let trans_str = trans.clone();

            fetch_tasks.push(tokio::spawn(async move {
                let query = match (book_name.as_str(), ch) {
                    ("Obadiah", 1) => "Obadiah+1:1-21".to_string(),
                    ("Philemon", 1) => "Philemon+1:1-25".to_string(),
                    ("2 John", 1) => "2+John+1:1-13".to_string(),
                    ("3 John", 1) => "3+John+1:1-14".to_string(),
                    ("Jude", 1) => "Jude+1:1-25".to_string(),
                    _ => format!("{}+{}", book_name.replace(' ', "+"), ch),
                };
                let url = format!("https://bible-api.com/{}?translation={}", query, trans_str);
                let resp = client_ref.get(&url).send().await.ok()?;
                if !resp.status().is_success() {
                    return None;
                }
                let api_data: BibleApiResponse = resp.json().await.ok()?;
                if api_data.verses.is_empty() {
                    return None;
                }
                let first_verse = api_data.verses.first()?.verse;
                let last_verse = api_data.verses.last()?.verse;
                let version_name = api_data
                    .translation_id
                    .as_deref()
                    .map(|t| t.to_uppercase())
                    .unwrap_or_else(|| trans_str.to_uppercase());

                let mut item = ScriptureItem::new(&book_name, ch, first_verse, last_verse, &version_name);
                item.reference = format!("{} {}", book_name, ch);
                item.id = format!(
                    "scrip_{}_{}_{}",
                    book_name.to_lowercase().replace(' ', "_"),
                    ch,
                    version_name.to_lowercase()
                );
                item.verses = api_data
                    .verses
                    .into_iter()
                    .map(|v| ScriptureVerse {
                        verse_number: v.verse,
                        text: v.text.trim().to_string(),
                    })
                    .collect();

                Some(item)
            }));
        }

        let mut results = Vec::new();
        for task in fetch_tasks {
            if let Ok(Some(item)) = task.await {
                results.push(item);
            }
        }

        results.sort_by_key(|it| it.chapter);
        if results.is_empty() {
            return Err(format!("Could not fetch chapters for book {}", meta.name).into());
        }

        Ok(results)
    }

    /// Fetch online Bible passage from API, automatically importing whole book if detected
    pub async fn fetch_online_passage(
        query: &str,
        translation: Option<&str>,
    ) -> Result<Vec<ScriptureItem>, Box<dyn std::error::Error + Send + Sync>> {
        let trans = translation.unwrap_or("kjv").to_lowercase();

        // 1. Check if the query matches an exact known Bible book to import the entire book
        if let Some(meta) = find_book_meta_exact(query) {
            if let Ok(book_items) = Self::fetch_online_book(meta, Some(&trans)).await {
                if !book_items.is_empty() {
                    return Ok(book_items);
                }
            }
        }

        // 2. Fallback to direct reference query
        let encoded_query = urlencoding::encode(query.trim());
        let url = format!("https://bible-api.com/{}?translation={}", encoded_query, trans);

        let client = reqwest::Client::builder()
            .user_agent("OpenSanctuary/1.0 (Linux; Worship Presentation Engine; https://github.com/opensanctuary)")
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let resp = client.get(&url).send().await?.error_for_status()?;
        let api_data: BibleApiResponse = resp.json().await?;

        if api_data.verses.is_empty() {
            return Err("No verses found for reference".into());
        }

        let first_verse = &api_data.verses[0];
        let last_verse = api_data.verses.last().unwrap();
        let book_name = first_verse.book_name.clone();
        let chapter = first_verse.chapter;
        let start_v = first_verse.verse;
        let end_v = last_verse.verse;
        let version_name = api_data
            .translation_id
            .as_deref()
            .map(|t| t.to_uppercase())
            .unwrap_or_else(|| trans.to_uppercase());

        let mut item = ScriptureItem::new(&book_name, chapter, start_v, end_v, &version_name);
        item.reference = api_data.reference;
        item.id = format!(
            "scrip_{}",
            uuid::Uuid::new_v5(
                &uuid::Uuid::NAMESPACE_OID,
                format!("{}:{}:{}-{}:{}", book_name, chapter, start_v, end_v, version_name).as_bytes()
            )
        );

        item.verses = api_data
            .verses
            .into_iter()
            .map(|v| ScriptureVerse {
                verse_number: v.verse,
                text: v.text.trim().to_string(),
            })
            .collect();

        Ok(vec![item])
    }

    /// Download the ENTIRE Bible (all 66 books, 1,189 chapters, ~31,000 verses) for a given translation
    pub async fn download_full_bible(translation: &str) -> Result<Vec<ScriptureItem>, Box<dyn std::error::Error + Send + Sync>> {
        let trans_upper = translation.trim().to_uppercase();
        let url = format!("https://bolls.life/static/translations/{}.json", trans_upper);

        // A full translation is a single ~10-15MB JSON file (not fetched
        // chapter-by-chapter), so this is one slow request rather than many
        // fast ones -- but on a slow connection it's genuinely slow: observed
        // ~46s for KJV (12.3MB) at ~260KB/s. 30s was tight enough to fail
        // outright rather than just being unhurried, which matters a lot
        // more now that this is first-time setup's one-click "get started"
        // action -- a new user's very first action failing with a generic
        // error is a bad first impression. 120s covers a much slower
        // connection without leaving a hung request truly unbounded.
        let client = reqwest::Client::builder()
            .user_agent("OpenSanctuary/1.0 (Church Presentation Engine; https://github.com/opensanctuary)")
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let resp = client.get(&url).send().await?.error_for_status()?;
        let verses: Vec<BollsVerse> = resp.json().await?;

        if verses.is_empty() {
            return Err(format!("Downloaded Bible translation '{}' was empty", trans_upper).into());
        }

        // Clean Strong's numbers and HTML tags (e.g. <S>7225</S>, <i>...</i>, <br>)
        fn clean_verse_text(raw: &str) -> String {
            let mut result = String::with_capacity(raw.len());
            let mut in_tag = false;
            for c in raw.chars() {
                if c == '<' {
                    in_tag = true;
                } else if c == '>' {
                    in_tag = false;
                } else if !in_tag {
                    result.push(c);
                }
            }
            result.trim().to_string()
        }

        // Group verses by (book_number, chapter)
        use std::collections::BTreeMap;
        let mut chapters_map: BTreeMap<(u32, u32), Vec<ScriptureVerse>> = BTreeMap::new();

        for v in verses {
            if v.book >= 1 && v.book <= 66 {
                let clean_text = clean_verse_text(&v.text);
                chapters_map.entry((v.book, v.chapter))
                    .or_default()
                    .push(ScriptureVerse {
                        verse_number: v.verse,
                        text: clean_text,
                    });
            }
        }

        let mut items = Vec::with_capacity(chapters_map.len());
        for ((book_num, chapter_num), verse_list) in chapters_map {
            if verse_list.is_empty() {
                continue;
            }
            let book_name = CANONICAL_BOOKS[(book_num - 1) as usize];
            let start_v = verse_list.first().map(|v| v.verse_number).unwrap_or(1);
            let end_v = verse_list.last().map(|v| v.verse_number).unwrap_or(start_v);

            let mut item = ScriptureItem::new(book_name, chapter_num, start_v, end_v, &trans_upper);
            item.reference = format!("{} {}", book_name, chapter_num);
            item.id = format!("scrip_{}_{}_{}", trans_upper.to_lowercase(), book_num, chapter_num);
            item.verses = verse_list;
            items.push(item);
        }

        Ok(items)
    }

    /// Fetch online Bible catalog combining ChurchApps, OpenLP Community, and Bolls.life with smart deduplication
    pub async fn fetch_churchapps_catalog() -> Result<Vec<OnlineBibleCatalogItem>, Box<dyn std::error::Error + Send + Sync>> {
        let client = reqwest::Client::builder()
            .user_agent("OpenSanctuary/1.0 (Church Presentation Engine; https://github.com/opensanctuary)")
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        use std::collections::BTreeMap;
        let mut map: BTreeMap<(String, String), OnlineBibleCatalogItem> = BTreeMap::new();

        // Helper to insert or merge items
        let mut insert_or_merge = |mut item: OnlineBibleCatalogItem, source_label: &str| {
            let abbr = item.abbreviation.as_deref().unwrap_or(&item.id).trim().to_uppercase();
            let raw_lang = item.language.as_deref().unwrap_or("English");
            let norm_lang = normalize_language_name(raw_lang);
            let key = (abbr.clone(), norm_lang.to_lowercase());

            item.abbreviation = Some(abbr);
            item.language = Some(norm_lang);

            if let Some(existing) = map.get_mut(&key) {
                // Merge source badges
                let cur_src = existing.source.as_deref().unwrap_or("");
                if !cur_src.contains(source_label) {
                    if cur_src.is_empty() {
                        existing.source = Some(source_label.to_string());
                    } else {
                        existing.source = Some(format!("{} • {}", cur_src, source_label));
                    }
                }
                // Keep longer / more informative title
                if item.name.len() > existing.name.len() {
                    existing.name = item.name;
                }
            } else {
                item.source = Some(source_label.to_string());
                map.insert(key, item);
            }
        };

        // 1. Curated OpenLP Community & Core Public Domain Bibles
        let openlp_curated = vec![
            ("KJV", "King James Version (Authorized 1769)", "English"),
            ("NKJV", "New King James Version (1982)", "English"),
            ("ESV", "English Standard Version (2001, 2016)", "English"),
            ("NIV", "New International Version (1984)", "English"),
            ("NLT", "New Living Translation (2015)", "English"),
            ("CSB17", "Christian Standard Bible (2017)", "English"),
            ("NASB", "New American Standard Bible (1995)", "English"),
            ("ASV", "American Standard Version (1901)", "English"),
            ("BSB", "Berean Standard Bible", "English"),
            ("WEB", "World English Bible", "English"),
            ("YLT", "Young's Literal Translation (1898)", "English"),
            ("DBY", "Darby Translation (1890)", "English"),
            ("DRB", "Douay-Rheims Bible (1899)", "English"),
            ("MSG", "The Message (2002)", "English"),
            ("AMP", "Amplified Bible (2015)", "English"),
            ("RVR1960", "Reina-Valera 1960", "Spanish"),
            ("LSG", "Louis Segond 1910", "French"),
            ("LUT", "Lutherbibel 1912", "German"),
            ("CLE", "Biblia Sacra Vulgata (Clementine Latin)", "Latin"),
            ("ALM", "João Ferreira de Almeida Revista e Corrigida", "Portuguese"),
            ("SYNOD", "Синодальный перевод (Russian Synodal 1876)", "Russian"),
            ("UBIO", "Біблія в пер. Івана Огієнка (1962)", "Ukrainian"),
            ("CUV", "和合本 (Chinese Union Version)", "Chinese"),
            ("TAG", "Ang Biblia (1905)", "Tagalog"),
            ("LXXE", "English Septuagint Bible (Brenton 1851)", "Greek"),
        ];

        for (abbr, name, lang) in openlp_curated {
            insert_or_merge(OnlineBibleCatalogItem {
                id: abbr.to_string(),
                abbreviation: Some(abbr.to_string()),
                name: name.to_string(),
                language: Some(lang.to_string()),
                source: Some("OpenLP Community".to_string()),
                source_key: Some(abbr.to_string()),
            }, "OpenLP Community");
        }

        // 2. Fetch Bolls.life Global Catalog (150+ full downloadable translations across 31 languages)
        if let Ok(resp) = client.get("https://bolls.life/static/bolls/app/views/languages.json").send().await {
            if let Ok(langs) = resp.json::<Vec<serde_json::Value>>().await {
                for l in langs {
                    let lang_name = l.get("language").and_then(|v| v.as_str()).unwrap_or("English").to_string();
                    if let Some(translations) = l.get("translations").and_then(|v| v.as_array()) {
                        for t in translations {
                            let short_name = t.get("short_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let full_name = t.get("full_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            if !short_name.is_empty() {
                                insert_or_merge(OnlineBibleCatalogItem {
                                    id: short_name.clone(),
                                    abbreviation: Some(short_name.clone()),
                                    name: if full_name.is_empty() { short_name.clone() } else { full_name },
                                    language: Some(lang_name.clone()),
                                    source: Some("Bolls.life".to_string()),
                                    source_key: Some(short_name),
                                }, "Bolls.life");
                            }
                        }
                    }
                }
            }
        }

        // 3. Fetch ChurchApps Catalog
        if let Ok(resp) = client.get("https://api.churchapps.org/content/bibles").send().await {
            if let Ok(ca_items) = resp.json::<Vec<OnlineBibleCatalogItem>>().await {
                for item in ca_items {
                    insert_or_merge(item, "ChurchApps");
                }
            }
        }

        // Collect and sort: English first, then alphabetical by Language, then by Name
        let mut list: Vec<OnlineBibleCatalogItem> = map.into_values().collect();
        list.sort_by(|a, b| {
            let a_lang = a.language.as_deref().unwrap_or("English");
            let b_lang = b.language.as_deref().unwrap_or("English");
            
            let a_is_eng = a_lang == "English";
            let b_is_eng = b_lang == "English";
            
            if a_is_eng != b_is_eng {
                return b_is_eng.cmp(&a_is_eng);
            }
            if a_lang != b_lang {
                return a_lang.cmp(b_lang);
            }
            a.name.cmp(&b.name)
        });

        Ok(list)
    }
}
