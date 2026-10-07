#![no_main]

use libfuzzer_sys::fuzz_target;
use os_next::storage::openlp_import::OpenLPImporter;

// Both theme-background parsers take `theme_dir`/`media_dir` only to resolve
// an on-disk image once the JSON/XML itself points at one; passing `None`
// for both exercises every parsing branch except that final filesystem
// touch, which is exactly the part driven by untrusted file content.
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = OpenLPImporter::parse_openlp_theme_json(s, None, None);
        let _ = OpenLPImporter::parse_openlp_theme_xml(s, None, None);
    }
});
