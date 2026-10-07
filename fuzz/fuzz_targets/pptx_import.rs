#![no_main]

use libfuzzer_sys::fuzz_target;
use os_next::storage::pptx_import::PptxImporter;
use std::sync::OnceLock;
use tempfile::TempDir;

// One process-lifetime media directory, reused across iterations, instead of
// a fresh tempdir per call -- `extract_picture_element` writes copied images
// out with uuid-derived names, so reuse never collides; it just means a long
// fuzz run accumulates files here and the directory should be cleared
// between runs.
fn media_dir() -> &'static std::path::Path {
    static DIR: OnceLock<TempDir> = OnceLock::new();
    DIR.get_or_init(|| TempDir::new().expect("tempdir")).path()
}

fuzz_target!(|data: &[u8]| {
    let _ = PptxImporter::import_pptx_bytes(data, "fuzz", media_dir());
});
