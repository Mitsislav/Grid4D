//! Writes the scenes the performance budget is measured on
//! (`docs/performance.md`):
//!
//! * `stress-10k.g4d` — 10 000 primitives of eight shapes in groups of 100,
//!   one in ten moving over frames 0–90, and a camera circling them
//!   (look through it and press Play to measure "orbiting");
//! * `stress-1m.g4d` — one polygon sphere of about a million triangles;
//! * `stress-50mb.g4d` — polygon objects with uneven points (so the archive
//!   cannot compress them away) until the file reaches 50 MB;
//! * `stress-10k-textured.g4d` and `stress-1m-textured.g4d` — the first two
//!   in textured materials: eight materials with a 1024-pixel colour picture
//!   and a normal picture (embedded), on Box projections for the primitives
//!   and on the sphere's UVs.
//!
//! `cargo run --release --example stress -- [folder]` (the current folder by
//! default).

use std::f32::consts::TAU;
use std::path::PathBuf;

use glam::Vec3;
use grid4d_geom::primitives;
use grid4d_scene::{
    Axis, Channel, ColorSpace, Curve, Document, Key, Material, MaterialId, MaterialTag, MeshHandle, Node, NodeData,
    ObjectKind, ObjectKindTag, Param, Projection, Tag, Texture, TextureSlot, TextureSource, Track,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    std::fs::create_dir_all(&folder)?;
    for (name, doc) in [
        ("stress-10k.g4d", ten_thousand()?),
        ("stress-1m.g4d", million_triangles()),
        ("stress-50mb.g4d", fifty_megabytes()?),
        ("stress-10k-textured.g4d", textured(ten_thousand()?, Projection::Box)?),
        ("stress-1m-textured.g4d", textured(million_triangles(), Projection::Uv)?),
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
    let mut camera = NodeData::new(
        "Orbit Camera",
        ObjectKind::Camera {
            fov_y_deg: 45.0,
            lens: grid4d_scene::Lens::default(),
        },
    );
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

/// A `side` × `side` PNG made by `texel(x, y)`.
fn picture(side: u32, texel: impl Fn(u32, u32) -> [u8; 3]) -> Result<Vec<u8>, grid4d_io::IoError> {
    let mut pixels = Vec::with_capacity(usize::try_from(side * side * 3).unwrap_or(0));
    for y in 0..side {
        for x in 0..side {
            pixels.extend(texel(x, y));
        }
    }
    grid4d_io::png_bytes(side, side, grid4d_io::Channels::Rgb, &pixels)
}

/// `doc` with eight textured materials (a colour checker of its own hue and a
/// normal picture of bumps, 1024 pixels, embedded), one on each object (by
/// turn) through a Material tag with `projection`.
fn textured(mut doc: Document, projection: Projection) -> Result<Document, grid4d_io::IoError> {
    let embed = |doc: &mut Document, name: &str, bytes: Vec<u8>, color_space| {
        doc.scene.textures_mut().insert(Texture {
            name: name.to_owned(),
            source: TextureSource::Embedded {
                file_name: format!("{name}.png"),
                bytes: bytes.into(),
            },
            color_space,
        })
    };
    let bumps = picture(1024, |x, y| {
        let wave = |v: u32| (f32::from(u16::try_from(v % 64).unwrap_or(0)) / 64.0 * TAU).sin();
        let (nx, ny) = (wave(x) * 0.4, wave(y) * 0.4);
        let byte = |c: f32| (c.mul_add(0.5, 0.5) * 255.0) as u8;
        [byte(nx), byte(ny), byte((1.0 - nx * nx - ny * ny).max(0.0).sqrt())]
    })?;
    let normal = embed(&mut doc, "Bumps", bumps, ColorSpace::Linear);
    let mut materials: Vec<MaterialId> = Vec::new();
    for k in 0..8u8 {
        let hue = [k & 1, (k >> 1) & 1, (k >> 2) & 1].map(|on| if on == 1 { 220 } else { 60 });
        let checker = picture(1024, |x, y| if (x / 128 + y / 128) % 2 == 0 { hue } else { [235; 3] })?;
        let colour = embed(&mut doc, &format!("Checker {k}"), checker, ColorSpace::Srgb);
        let mut material = Material::new(format!("Textured {k}"));
        *material.map_mut(Channel::BaseColor) = Some(TextureSlot::new(colour));
        *material.map_mut(Channel::Normal) = Some(TextureSlot::new(normal));
        materials.push(doc.scene.materials_mut().insert(material));
    }
    let ids: Vec<_> = doc.scene.iter().map(|(id, _)| id).collect();
    let mut turn = 0usize;
    for id in ids {
        let Some(node) = doc.scene.get_mut(id) else { continue };
        if matches!(
            node.data.kind,
            ObjectKind::Null | ObjectKind::Camera { .. } | ObjectKind::Light { .. }
        ) {
            continue;
        }
        let mut tag = MaterialTag::new(materials.get(turn % materials.len()).copied());
        tag.projection = projection;
        node.data.tags.push(Tag::Material(tag));
        turn += 1;
    }
    Ok(doc)
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
