#![no_main]

//! A Bible file the operator pastes a link to (or picks) is untrusted: `parse_bible` is fed arbitrary bytes, which
//! cover loose USFM, OSIS and Zefania XML, and zips of them (a zip magic prefix is the fuzzer's to find). Whatever parses
//! is then installed into an in-memory library, so the chapter/verse limits and the writer are exercised too.

use libfuzzer_sys::fuzz_target;
use os_next::storage::bible_import::{install, parse_bible};
use os_next::storage::Database;

fuzz_target!(|data: &[u8]| {
    if let Ok(parsed) = parse_bible(data, "fuzz.xml") {
        let db = Database::in_memory().expect("in-memory db");
        let _ = install(&db, &parsed, &parsed.abbreviation.clone(), true, "fuzz");
    }
});
