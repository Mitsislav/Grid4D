//! File-level tests: the golden fixture pins the on-disk format, older fixtures
//! must keep loading, and save/load go through the real filesystem.
//!
//! Regenerate the current golden file after an *intentional* format change (which
//! also needs a new format version and a migration) with:
//! `GRID4D_UPDATE_FIXTURES=1 cargo test -p grid4d-io --test format`
//! Older fixtures (`demo_v1.oc4d`, `demo_v2.g4d`) are never regenerated.
//!
//! The golden archive is compared entry by entry, decompressed: the compressed
//! bytes may legitimately change with the deflate library.

#![allow(clippy::unwrap_used, reason = "test code")]

use std::{
    fs,
    io::{Cursor, Read as _},
    path::{Path, PathBuf},
};

use glam::Vec3;
use grid4d_scene::{Document, MeshHandle, ObjectKind};

/// Written by the current format version (an archive).
const GOLDEN: &str = "tests/fixtures/demo_v3.g4d";
/// Written by format version 2 (JSON, before polygon objects); kept to test migration.
const DEMO_V2: &str = "tests/fixtures/demo_v2.g4d";
/// Written by format version 1 (before segment counts); kept to test migration.
const DEMO_V1: &str = "tests/fixtures/demo_v1.oc4d";

fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// A fresh, empty directory under the system temp dir, unique to this test.
fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grid4d-io-{}-{test}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn outline(doc: &Document) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    doc.scene.walk(&mut Vec::new(), |_, node, _, depth| {
        out.push((depth, node.data.name.clone()));
        true
    });
    out
}

const DEMO_OUTLINE: [(u32, &str); 7] = [
    (0, "Cube"),
    (0, "Sphere"),
    (0, "Group"),
    (1, "Cylinder"),
    (1, "Box"),
    (0, "Light"),
    (0, "Camera"),
];

/// The demo scene with its "Box" made editable: every kind of object, and a mesh.
fn golden_document() -> Document {
    let mut doc = Document::with_demo_scene();
    let id = doc.scene.iter().find(|(_, n)| n.data.name == "Box").unwrap().0;
    let mesh = grid4d_geom::primitives::cube(Vec3::splat(80.0), [1, 2, 1]);
    doc.scene.get_mut(id).unwrap().data.kind = ObjectKind::Polygon {
        mesh: MeshHandle::new(mesh),
    };
    doc
}

/// An archive's entries, in order, decompressed.
fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
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

#[test]
fn the_golden_document_matches_the_golden_file() {
    let bytes = grid4d_io::to_bytes(&golden_document()).unwrap();
    let path = fixture_path(GOLDEN);
    if std::env::var_os("GRID4D_UPDATE_FIXTURES").is_some() {
        fs::write(&path, &bytes).unwrap();
    }
    let (written, golden) = (entries(&bytes), entries(&fs::read(&path).unwrap()));
    let names = |e: &[(String, Vec<u8>)]| e.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    assert_eq!(names(&written), names(&golden));
    // The JSON as text, so a failure shows a readable diff.
    let text = |e: &[(String, Vec<u8>)]| String::from_utf8(e[0].1.clone()).unwrap();
    assert_eq!(
        text(&written),
        text(&golden),
        "the current file format changed; see the note at the top of this file"
    );
    assert_eq!(written, golden, "a mesh file changed");
}

#[test]
fn the_golden_file_loads() {
    let doc = grid4d_io::load(&fixture_path(GOLDEN)).unwrap();
    assert_eq!(outline(&doc), DEMO_OUTLINE.map(|(d, n)| (d, n.to_owned())));
    assert_eq!(doc.time.fps(), 30);
    let reference = golden_document();
    let kind = |doc: &Document| {
        let node = doc.scene.iter().find(|(_, n)| n.data.name == "Box").unwrap().1;
        node.data.kind.clone()
    };
    assert_eq!(kind(&doc), kind(&reference), "the mesh, point for point");
}

#[test]
fn the_version_2_fixture_still_loads() {
    let doc = grid4d_io::load(&fixture_path(DEMO_V2)).unwrap();
    assert_eq!(outline(&doc), DEMO_OUTLINE.map(|(d, n)| (d, n.to_owned())));
    // Migrating then saving gives exactly what the current version writes.
    assert_eq!(
        grid4d_io::to_bytes(&doc).unwrap(),
        grid4d_io::to_bytes(&Document::with_demo_scene()).unwrap()
    );
}

#[test]
fn the_version_1_fixture_still_loads() {
    let doc = grid4d_io::load(&fixture_path(DEMO_V1)).unwrap();
    assert_eq!(outline(&doc), DEMO_OUTLINE.map(|(d, n)| (d, n.to_owned())));
    let sphere = doc.scene.iter().find(|(_, n)| n.data.name == "Sphere").unwrap().1;
    assert_eq!(
        sphere.data.kind,
        ObjectKind::Sphere {
            radius: 80.0,
            segments: 24
        },
        "migrated with default segments"
    );
    // Migrating then saving gives exactly what the current version writes.
    assert_eq!(
        grid4d_io::to_bytes(&doc).unwrap(),
        grid4d_io::to_bytes(&Document::with_demo_scene()).unwrap()
    );
}

#[test]
fn save_and_load_through_the_filesystem() {
    let dir = temp_dir("save_load");
    let path = dir.join(format!("shot.{}", grid4d_io::EXTENSION));

    let mut doc = Document::with_demo_scene();
    grid4d_io::save(&doc, &path).unwrap();
    // Saving again replaces the file in place.
    doc.name = "Second save".to_owned();
    grid4d_io::save(&doc, &path).unwrap();

    let loaded = grid4d_io::load(&path).unwrap();
    assert_eq!(loaded.name, "Second save");
    assert_eq!(loaded.scene.len(), doc.scene.len());
    let leftovers: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        leftovers,
        [format!("shot.{}", grid4d_io::EXTENSION)],
        "no temporary file left behind"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn loading_a_missing_file_is_an_io_error() {
    let missing = temp_dir("missing").join("nothing-here.g4d");
    assert!(matches!(grid4d_io::load(&missing), Err(grid4d_io::IoError::Io(_))));
}
