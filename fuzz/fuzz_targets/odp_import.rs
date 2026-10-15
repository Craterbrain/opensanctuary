#![no_main]

use libfuzzer_sys::fuzz_target;
use os_next::storage::odp_import::OdpImporter;
use std::sync::OnceLock;
use tempfile::TempDir;

// Same reasoning as pptx_import.rs's fuzz target: one process-lifetime media directory reused
// across iterations (write_sniffed_media's uuid-derived names never collide), cleared between runs.
fn media_dir() -> &'static std::path::Path {
    static DIR: OnceLock<TempDir> = OnceLock::new();
    DIR.get_or_init(|| TempDir::new().expect("tempdir")).path()
}

fuzz_target!(|data: &[u8]| {
    let _ = OdpImporter::import_odp_bytes(data, "fuzz", media_dir());
});
