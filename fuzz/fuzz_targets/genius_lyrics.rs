#![no_main]

use libfuzzer_sys::fuzz_target;
use os_next::storage::genius_import::{extract_ccli_from_text, parse_genius_lyrics_to_song};

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = parse_genius_lyrics_to_song("fuzz title", "fuzz artist", s);
        let _ = extract_ccli_from_text(s);
    }
});
