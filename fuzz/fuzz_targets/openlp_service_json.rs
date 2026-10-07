#![no_main]

use libfuzzer_sys::fuzz_target;
use os_next::storage::openlp_import::OpenLPImporter;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = OpenLPImporter::import_openlp_service_json(s);
    }
});
