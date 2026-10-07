#![no_main]

use libfuzzer_sys::fuzz_target;
use os_next::storage::freeshow_import::FreeShowImporter;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = FreeShowImporter::import_fsb_content(s);
    }
});
