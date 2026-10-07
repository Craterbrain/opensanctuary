#![no_main]

use libfuzzer_sys::fuzz_target;
use os_next::storage::ewsx::EwsxManager;

// Covers the EWSX/OSZ/EWPX zip-sniffing + embedded-JSON-Schedule fallback
// path in `load_schedule_from_bytes` -- the entry point used for an
// uploaded/dropped import file of unknown provenance.
fuzz_target!(|data: &[u8]| {
    let _ = EwsxManager::load_schedule_from_bytes(data, "fuzz");
});
