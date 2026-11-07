//! Any bytes given to the `.g4d` loader: it must return a document or an
//! error — never panic, hang or allocate what a file merely claims. The seed
//! corpus (the test fixtures) holds every format version, so the current
//! version 5 reader and the upgrades of versions 3 and 4 to it are exercised.
//!
//! cargo +nightly fuzz run load fuzz/corpus/load crates/io/tests/fixtures

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok((document, cache)) = grid4d_io::from_bytes_with_cache(data) {
        // What loads must save, and load again.
        if let Ok(bytes) = grid4d_io::to_bytes_with_cache(&document, cache.as_ref()) {
            assert!(grid4d_io::from_bytes_with_cache(&bytes).is_ok(), "a saved document loads");
        }
    }
});
