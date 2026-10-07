#![no_main]

use libfuzzer_sys::fuzz_target;
use os_next::storage::openlp_import::OpenLPImporter;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let _ = OpenLPImporter::parse_openlp_otz(Cursor::new(data), None);
});
