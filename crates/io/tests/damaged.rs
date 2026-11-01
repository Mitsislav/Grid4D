//! Damaged and hostile files: whatever the bytes, the loader returns a
//! document or an error — it never panics, and whatever it loads can be saved
//! and loaded again. A quick, stable companion of the fuzz target in `fuzz/`
//! (which runs in CI on nightly).

#![allow(clippy::unwrap_used, reason = "test code")]

use std::{
    fs,
    io::{Cursor, Read as _, Write as _},
    path::Path,
};

/// A small, seeded random number generator (`SplitMix64`).
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap()).unwrap()
    }

    fn byte(&mut self) -> u8 {
        self.next().to_le_bytes()[0]
    }
}

/// How many damaged copies of each fixture to try: `base`, times
/// `GRID4D_DAMAGE_ROUNDS` when set (for a longer run by hand).
fn rounds(base: usize) -> usize {
    std::env::var("GRID4D_DAMAGE_ROUNDS")
        .ok()
        .and_then(|n| n.parse::<usize>().ok())
        .map_or(base, |n| base * n)
}

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read(&path).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

/// Loads `bytes`; what loads must save and load again.
fn load(bytes: &[u8]) {
    if let Ok((document, cache)) = grid4d_io::from_bytes_with_cache(bytes)
        && let Ok(saved) = grid4d_io::to_bytes_with_cache(&document, cache.as_ref())
    {
        assert!(
            grid4d_io::from_bytes_with_cache(&saved).is_ok(),
            "a saved document loads"
        );
    }
}

#[test]
fn damaged_files_never_panic_the_loader() {
    let mut random = Random(1);
    for (_, fixture) in fixtures() {
        for _ in 0..rounds(300) {
            let mut bytes = fixture.clone();
            match random.below(4) {
                // A few bytes flipped.
                0 => {
                    for _ in 0..=random.below(8) {
                        let at = random.below(bytes.len());
                        bytes[at] ^= random.byte().max(1);
                    }
                }
                // Cut short.
                1 => bytes.truncate(random.below(bytes.len())),
                // Bytes inserted.
                2 => {
                    let at = random.below(bytes.len());
                    let extra: Vec<u8> = (0..=random.below(64)).map(|_| random.byte()).collect();
                    bytes.splice(at..at, extra);
                }
                // A span overwritten.
                _ => {
                    let at = random.below(bytes.len());
                    let end = (at + 1 + random.below(256)).min(bytes.len());
                    for byte in &mut bytes[at..end] {
                        *byte = random.byte();
                    }
                }
            }
            load(&bytes);
        }
    }
}

/// The entries of a zip archive, in order.
fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut entry = archive.by_index(i).unwrap();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            (entry.name().unwrap().into_owned(), data)
        })
        .collect()
}

fn zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        writer
            .start_file(name.as_str(), zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// Where the numbers of a JSON text are (roughly: digit runs with a sign,
/// fraction and exponent).
fn numbers(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = if i > 0 && bytes[i - 1] == b'-' { i - 1 } else { i };
            while i < bytes.len() && (bytes[i].is_ascii_digit() || b".eE+-".contains(&bytes[i])) {
                i += 1;
            }
            out.push((start, i));
        } else {
            i += 1;
        }
    }
    out
}

/// Valid archives whose `document.json` says hostile things: numbers out of
/// any range, indices past the lists, the wrong types, lines gone or doubled.
#[test]
fn hostile_documents_never_panic_the_loader() {
    const HOSTILE: [&str; 14] = [
        "-1",
        "0",
        "-0.0",
        "4294967295",
        "4294967296",
        "18446744073709551615",
        "99999999999999999999",
        "1e39",
        "1e308",
        "-1e308",
        "\"x\"",
        "null",
        "[]",
        "{}",
    ];
    let mut random = Random(7);
    for (name, fixture) in fixtures() {
        if !fixture.starts_with(b"PK") {
            continue;
        }
        let entries = unzip(&fixture);
        let json = String::from_utf8(entries[0].1.clone()).unwrap();
        let spots = numbers(&json);
        let lines: Vec<&str> = json.lines().collect();
        for round in 0..rounds(400) {
            let text = match round % 3 {
                // A number replaced.
                0 => {
                    let (start, end) = spots[random.below(spots.len())];
                    format!(
                        "{}{}{}",
                        &json[..start],
                        HOSTILE[random.below(HOSTILE.len())],
                        &json[end..]
                    )
                }
                // A line gone.
                1 => {
                    let gone = random.below(lines.len());
                    let kept: Vec<&str> = lines
                        .iter()
                        .enumerate()
                        .filter(|&(i, _)| i != gone)
                        .map(|(_, l)| *l)
                        .collect();
                    kept.join("\n")
                }
                // A line doubled.
                _ => {
                    let twice = random.below(lines.len());
                    let mut doubled = lines.clone();
                    doubled.insert(twice, lines[twice]);
                    doubled.join("\n")
                }
            };
            let mut hostile = entries.clone();
            hostile[0].1 = text.into_bytes();
            let bytes = zip(&hostile);
            load(&bytes);
            assert!(!name.is_empty());
        }
    }
}
