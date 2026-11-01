//! Saving and loading a `.g4d` of about 50 MB (the performance budget's
//! file: under 2 s to open), in memory, without the disk (`cargo bench -p
//! grid4d-io`).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use glam::Vec3;
use grid4d_geom::primitives;
use grid4d_scene::{Document, MeshHandle, Node, ObjectKind};

/// Polygon objects of 250 000 quads with slightly uneven points (so the
/// archive does not compress them away) until the file reaches 50 MB.
fn fifty_megabytes() -> (Document, Vec<u8>) {
    let mut doc = Document::new("50 MB");
    let mut seed = 0x9E37_79B9_u32;
    loop {
        let mut mesh = primitives::plane(1000.0, 1000.0, 500, 500);
        for point in mesh.points_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *point += Vec3::Y * f32::from(u16::try_from(seed >> 16).unwrap_or(0)) * 1e-4;
        }
        doc.scene
            .insert(Node::new("Mesh", ObjectKind::polygon(MeshHandle::new(mesh))));
        let Ok(bytes) = grid4d_io::to_bytes(&doc) else {
            panic!("the document saves");
        };
        if bytes.len() >= 50 << 20 {
            return (doc, bytes);
        }
    }
}

fn files(c: &mut Criterion) {
    let (doc, bytes) = fifty_megabytes();
    let mut group = c.benchmark_group(format!("{} MB .g4d", bytes.len() >> 20));
    group.sample_size(10);
    group.bench_function("load", |b| b.iter(|| black_box(grid4d_io::from_bytes(&bytes))));
    group.bench_function("save", |b| b.iter(|| black_box(grid4d_io::to_bytes(&doc))));
    group.finish();
}

criterion_group!(benches, files);
criterion_main!(benches);
