//! A mesh of a million triangles (the performance budget's single mesh):
//! building its topology and BVH, triangulating it for the viewport, and one
//! Catmull-Clark level that makes a mesh of that size (`cargo bench -p
//! grid4d-geom`).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use grid4d_geom::subdivision::{Boundary, SubdivisionOptions, subdivide};
use grid4d_geom::{Bvh, PolyMesh, Smoothing, Topology, TriMesh, Triangulator, primitives};

/// 708 × 708 quads: 1 002 528 triangles.
fn million_triangles() -> PolyMesh {
    primitives::plane(1000.0, 1000.0, 708, 708)
}

fn mesh(c: &mut Criterion) {
    let big = million_triangles();
    let mut group = c.benchmark_group("1M triangles");
    group.sample_size(10);
    group.bench_function("topology", |b| b.iter(|| black_box(Topology::build(&big))));
    group.bench_function("bvh", |b| b.iter(|| black_box(Bvh::build(&big))));
    let mut triangulator = Triangulator::new();
    let mut out = TriMesh::default();
    group.bench_function("triangulate (Phong 80°)", |b| {
        b.iter(|| {
            triangulator.triangulate(&big, Smoothing::default(), &mut out);
            black_box(out.triangle_count())
        });
    });
    // A quarter of the quads, subdivided once: a million triangles again.
    let cage = primitives::plane(1000.0, 1000.0, 354, 354);
    let options = SubdivisionOptions {
        levels: 1,
        linear_pre: 0,
        boundary: Boundary::default(),
    };
    group.bench_function("subdivide 250k quads to 1M", |b| {
        b.iter(|| black_box(subdivide(&cage, &options)))
    });
    group.finish();
}

criterion_group!(benches, mesh);
criterion_main!(benches);
