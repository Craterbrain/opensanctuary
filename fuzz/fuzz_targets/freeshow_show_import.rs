#![no_main]

use libfuzzer_sys::fuzz_target;
use os_next::storage::freeshow_show_import::FreeShowShowImporter;
use std::sync::OnceLock;
use tempfile::TempDir;

fn media_dir() -> &'static std::path::Path {
    static DIR: OnceLock<TempDir> = OnceLock::new();
    DIR.get_or_init(|| TempDir::new().expect("tempdir")).path()
}

fuzz_target!(|data: &[u8]| {
    let _ = FreeShowShowImporter::import_show_bytes(data, media_dir());
});
