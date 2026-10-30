//! A thousand cubes from a Cloner falling into a box: the cost of building
//! the simulation and of simulating a frame (`cargo bench -p grid4d-sim`).

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use glam::Vec3;
use grid4d_geom::cloner::{Cloner, GridArray, Layout};
use grid4d_scene::{Body, CollisionShape, Document, Node, NodeData, ObjectKind, Tag};
use grid4d_sim::Simulation;

/// A 10 × 10 × 10 grid of 30 cm cubes over a floor with four walls.
fn scene() -> Document {
    let mut doc = Document::new("bench");
    let wall = |size: Vec3, at: Vec3| {
        let mut data = NodeData::new(
            "Wall",
            ObjectKind::Cube {
                size,
                segments: [1, 1, 1],
            },
        );
        data.transform.position = at;
        data.tags.push(Tag::Dynamics(Body::collider()));
        Node::from_data(data)
    };
    doc.scene.insert(wall(Vec3::new(800.0, 20.0, 800.0), Vec3::Y * -10.0));
    for (size, at) in [
        (Vec3::new(20.0, 300.0, 800.0), Vec3::new(-400.0, 150.0, 0.0)),
        (Vec3::new(20.0, 300.0, 800.0), Vec3::new(400.0, 150.0, 0.0)),
        (Vec3::new(800.0, 300.0, 20.0), Vec3::new(0.0, 150.0, -400.0)),
        (Vec3::new(800.0, 300.0, 20.0), Vec3::new(0.0, 150.0, 400.0)),
    ] {
        doc.scene.insert(wall(size, at));
    }
    let mut cloner = NodeData::new(
        "Cloner",
        ObjectKind::cloner(Cloner {
            layout: Layout::Grid(GridArray {
                count: [10, 10, 10],
                size: Vec3::splat(40.0),
                ..GridArray::default()
            }),
            ..Cloner::default()
        }),
    );
    cloner.transform.position.y = 400.0;
    cloner.tags.push(Tag::Dynamics(Body {
        shape: CollisionShape::Box,
        ..Body::rigid()
    }));
    let cloner = doc.scene.insert(Node::from_data(cloner));
    let cube = ObjectKind::Cube {
        size: Vec3::splat(30.0),
        segments: [1, 1, 1],
    };
    let _ = doc.scene.insert_child(cloner, Node::new("Cube", cube));
    doc
}

fn falling_cubes(c: &mut Criterion) {
    let doc = scene();
    c.bench_function("build 1000 bodies", |b| b.iter(|| black_box(Simulation::of(&doc))));
    let mut group = c.benchmark_group("simulate");
    group.sample_size(10);
    // A frame while the cubes fall and pile up (frames 20 – 30).
    group.bench_function("a frame of 1000 cubes", |b| {
        b.iter_batched(
            || {
                let mut simulation = Simulation::of(&doc);
                simulation.step_to(20);
                simulation
            },
            |mut simulation| {
                simulation.step();
                simulation
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

criterion_group!(benches, falling_cubes);
criterion_main!(benches);
