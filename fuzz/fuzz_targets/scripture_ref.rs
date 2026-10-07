#![no_main]

use libfuzzer_sys::fuzz_target;
use os_next::storage::freeshow_import::parse_scripture_reference;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = parse_scripture_reference(s);
    }
});
