//! Writes the scenes the performance budget is measured on
//! (`docs/performance.md`):
//!
//! * `stress-10k.g4d` — 10 000 primitives of eight shapes in groups of 100,
//!   one in ten moving over frames 0–90, and a camera circling them
//!   (look through it and press Play to measure "orbiting");
//! * `stress-1m.g4d` — one polygon sphere of about a million triangles;
//! * `stress-50mb.g4d` — polygon objects with uneven points (so the archive
//!   cannot compress them away) until the file reaches 50 MB.
//!
//! `cargo run --release --example stress -- [folder]` (the current folder by
//! default).

use std::f32::consts::TAU;
use std::path::PathBuf;

use glam::Vec3;
use grid4d_geom::primitives;
use grid4d_scene::{Axis, Curve, Document, Key, MeshHandle, Node, NodeData, ObjectKind, ObjectKindTag, Param, Track};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    std::fs::create_dir_all(&folder)?;
    for (name, doc) in [
        ("stress-10k.g4d", ten_thousand()?),
        ("stress-1m.g4d", million_triangles()),
        ("stress-50mb.g4d", fifty_megabytes()?),
    ] {
        let path = folder.join(name);
        grid4d_io::save(&doc, &path)?;
        let size = std::fs::metadata(&path)?.len();
        println!("{} ({} MB)", path.display(), size.div_ceil(1 << 20));
    }
    Ok(())
}

/// A light, so the scenes are lit as a user's would be.
fn lit(name: &str) -> Document {
    let mut doc = Document::new(name);
    let mut light = NodeData::new("Light", ObjectKindTag::Light.default_kind());
    light.transform.position = Vec3::new(-2000.0, 3000.0, 2000.0);
    doc.scene.insert(Node::from_data(light));
    doc
}

fn ten_thousand() -> Result<Document, grid4d_scene::SceneError> {
    let mut doc = lit("10 000 objects");
    let kinds = [
        ObjectKindTag::Cube,
        ObjectKindTag::Sphere,
        ObjectKindTag::Cylinder,
        ObjectKindTag::Cone,
        ObjectKindTag::Torus,
        ObjectKindTag::Capsule,
        ObjectKindTag::Plane,
        ObjectKindTag::Disc,
    ];
    let place = |i: usize| (f32::from(u16::try_from(i).unwrap_or(0)) - 49.5) * 150.0;
    let mut group = None;
    for k in 0..10_000 {
        if k % 100 == 0 {
            group = Some(
                doc.scene
                    .insert(Node::new(format!("Row {}", k / 100), ObjectKind::Null)),
            );
        }
        let mut data = NodeData::new(format!("Object {k}"), kinds[k % kinds.len()].default_kind());
        data.transform.position = Vec3::new(place(k % 100), 0.0, place(k / 100));
        data.transform.scale = Vec3::splat(0.4);
        if k % 10 == 0 {
            data.tracks.push(track(Param::Position(Axis::Y), 0.0, 200.0));
        }
        if let Some(parent) = group {
            doc.scene.insert_child(parent, Node::from_data(data))?;
        }
    }
    // A camera on a turning rig: playing frames 0–90 circles the field once.
    let mut rig = NodeData::new("Camera Rig", ObjectKind::Null);
    rig.tracks.push(track(Param::Rotation(Axis::X), 0.0, TAU));
    let rig = doc.scene.insert(Node::from_data(rig));
    let mut camera = NodeData::new("Orbit Camera", ObjectKind::Camera { fov_y_deg: 45.0 });
    camera.transform.position = Vec3::new(0.0, 4000.0, 11_000.0);
    camera.transform.rotation = Vec3::new(0.0, -0.35, 0.0);
    doc.scene.insert_child(rig, Node::from_data(camera))?;
    Ok(doc)
}

/// A track of `param` from `from` at frame 0 to `to` at frame 90.
fn track(param: Param, from: f32, to: f32) -> Track {
    Track {
        param,
        curve: Curve::from_keys([Key::new(0.0, from), Key::new(90.0, to)]),
    }
}

fn million_triangles() -> Document {
    let mut doc = lit("1M triangles");
    // 1000 segments around: 1000 × 500 faces, about a million triangles.
    let mesh = primitives::sphere(500.0, 1000);
    doc.scene
        .insert(Node::new("Sphere", ObjectKind::polygon(MeshHandle::new(mesh))));
    doc
}

fn fifty_megabytes() -> Result<Document, grid4d_io::IoError> {
    let mut doc = lit("50 MB");
    let mut seed = 0x9E37_79B9_u32;
    for k in 0.. {
        let mut mesh = primitives::plane(1000.0, 1000.0, 500, 500);
        for point in mesh.points_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *point += Vec3::Y * f32::from(u16::try_from(seed >> 16).unwrap_or(0)) * 1e-3;
        }
        let mut data = NodeData::new(format!("Mesh {k}"), ObjectKind::polygon(MeshHandle::new(mesh)));
        data.transform.position.x = f32::from(u16::try_from(k).unwrap_or(0)) * 1100.0;
        doc.scene.insert(Node::from_data(data));
        if grid4d_io::to_bytes(&doc)?.len() >= 50 << 20 {
            break;
        }
    }
    Ok(doc)
}
