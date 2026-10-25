#![no_main]

//! OpenSanctuary's own bundle files (.ossong etc.) are zips an operator opens from anywhere, so the
//! reader (`storage::bundle`) is fed arbitrary bytes two ways: as the whole file, and -- much more
//! likely to get past the zip layer and exercise the JSON, validation and import code -- as the
//! manifest and content of an otherwise well-formed archive.

use libfuzzer_sys::fuzz_target;
use os_next::storage::bundle::{self, ImportOptions};
use os_next::storage::Database;
use std::io::{Cursor, Write};
use std::sync::OnceLock;
use tempfile::TempDir;
use zip::write::FileOptions;

// One process-lifetime media directory: imports only write files that match their declared hash and
// content type, under names derived from that hash, so reuse never collides.
fn media_dir() -> &'static std::path::Path {
    static DIR: OnceLock<TempDir> = OnceLock::new();
    DIR.get_or_init(|| TempDir::new().expect("tempdir")).path()
}

fn exercise(archive: &[u8]) {
    let Ok(mut opened) = bundle::open(Cursor::new(archive), None) else { return };
    let db = Database::in_memory().expect("in-memory db");
    let _ = bundle::inspect(&opened, &db, media_dir());
    let _ = bundle::import(&mut opened, &db, media_dir(), &ImportOptions { replace_bible: true });
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else { return };
    if mode & 1 == 0 {
        exercise(rest);
        return;
    }
    // Split the rest in two at a fuzzer-chosen point: manifest.json and content.json.
    let at = rest.first().map(|b| (*b as usize).min(rest.len().saturating_sub(1))).unwrap_or(0);
    let (manifest, content) = rest.split_at(at.min(rest.len()));
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [("manifest.json", manifest), ("content.json", content)] {
        if zip.start_file(name, FileOptions::<()>::default()).is_err() || zip.write_all(bytes).is_err() {
            return;
        }
    }
    if let Ok(done) = zip.finish() {
        exercise(&done.into_inner());
    }
});
