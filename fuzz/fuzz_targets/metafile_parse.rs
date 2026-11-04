#![no_main]

use libfuzzer_sys::fuzz_target;
use os_next::storage::{emf, wmf};

// The WMF and EMF readers walk counted binary records; a zip-level pptx fuzzer rarely reaches
// them with anything well-formed enough to get past the headers.
fuzz_target!(|data: &[u8]| {
    let _ = emf::parse(data);
    let _ = wmf::parse(data);
});
