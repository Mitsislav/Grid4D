//! File-level tests: the golden fixture pins the on-disk format, and save/load
//! go through the real filesystem.
//!
//! Regenerate the fixture after an *intentional* format change (which also needs
//! a new format version and a migration) with:
//! `OC4D_UPDATE_FIXTURES=1 cargo test -p oc4d-io --test format`

#![allow(clippy::unwrap_used, reason = "test code")]

use std::{
    fs,
    path::{Path, PathBuf},
};

use oc4d_scene::Document;

const DEMO_V1: &str = "tests/fixtures/demo_v1.oc4d";

fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// A fresh, empty directory under the system temp dir, unique to this test.
fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oc4d-io-{}-{test}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_demo_scene_matches_the_golden_file() {
    let text = oc4d_io::to_string(&Document::with_demo_scene()).unwrap();
    let path = fixture_path(DEMO_V1);
    if std::env::var_os("OC4D_UPDATE_FIXTURES").is_some() {
        fs::write(&path, &text).unwrap();
    }
    let golden = fs::read_to_string(&path).unwrap();
    assert_eq!(
        text, golden,
        "the version 1 format changed; see the note at the top of this file"
    );
}

#[test]
fn the_golden_file_loads() {
    let doc = oc4d_io::load(&fixture_path(DEMO_V1)).unwrap();
    let names: Vec<(u32, String)> = {
        let mut out = Vec::new();
        doc.scene.walk(&mut Vec::new(), |_, node, _, depth| {
            out.push((depth, node.data.name.clone()));
            true
        });
        out
    };
    let expected = [
        (0, "Cube"),
        (0, "Sphere"),
        (0, "Group"),
        (1, "Cylinder"),
        (1, "Box"),
        (0, "Light"),
        (0, "Camera"),
    ];
    assert_eq!(names, expected.map(|(d, n)| (d, n.to_owned())));
    assert_eq!(doc.time.fps(), 30);
}

#[test]
fn save_and_load_through_the_filesystem() {
    let dir = temp_dir("save_load");
    let path = dir.join(format!("shot.{}", oc4d_io::EXTENSION));

    let mut doc = Document::with_demo_scene();
    oc4d_io::save(&doc, &path).unwrap();
    // Saving again replaces the file in place.
    doc.name = "Second save".to_owned();
    oc4d_io::save(&doc, &path).unwrap();

    let loaded = oc4d_io::load(&path).unwrap();
    assert_eq!(loaded.name, "Second save");
    assert_eq!(loaded.scene.len(), doc.scene.len());
    let leftovers: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        leftovers,
        [format!("shot.{}", oc4d_io::EXTENSION)],
        "no temporary file left behind"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn loading_a_missing_file_is_an_io_error() {
    let missing = temp_dir("missing").join("nothing-here.oc4d");
    assert!(matches!(oc4d_io::load(&missing), Err(oc4d_io::IoError::Io(_))));
}
