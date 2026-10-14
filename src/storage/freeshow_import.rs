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
}
