//! Extraction (what to draw, batched by shape) at the performance budget's
//! size, and building the component overlay of a million-triangle mesh
//! (`cargo bench -p grid4d-render`). The GPU side is measured by hand with
//! View ▸ Frame Time (`docs/performance.md`).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use glam::Vec3;
use grid4d_eval::EvaluatedScene;
use grid4d_geom::{Component, primitives};
use grid4d_render::{FrameData, OrbitCamera};
use grid4d_scene::{Document, MeshHandle, Node, NodeData, NodeId, ObjectKind, ObjectKindTag};

/// 10 000 primitives in groups of 100.
fn objects(count: usize) -> Document {
    let mut doc = Document::with_demo_scene();
    let kinds = [
        ObjectKindTag::Cube,
        ObjectKindTag::Sphere,
        ObjectKindTag::Cylinder,
        ObjectKindTag::Torus,
    ];
    let mut group = None;
    for k in 0..count {
        if k % 100 == 0 {
            group = Some(doc.scene.insert(Node::new("Group", ObjectKind::Null)));
        }
        let mut data = NodeData::new("Object", kinds[k % kinds.len()].default_kind());
        let place = |i: usize| f32::from(u16::try_from(i).unwrap_or(u16::MAX)) * 250.0;
        data.transform.position = Vec3::new(place(k % 100), 0.0, place(k / 100));
        if let Some(parent) = group {
            let _ = doc.scene.insert_child(parent, Node::from_data(data));
        }
    }
    doc
}

fn extraction(c: &mut Criterion) {
    let doc = objects(10_000);
    let mut evaluated = EvaluatedScene::new();
    evaluated.update(&doc.scene);
    let mut frame = FrameData::new();
    let mut camera = OrbitCamera::default();
    frame.extract(&evaluated, |_| false);
    c.bench_function("extract 10k objects (orbiting)", |b| {
        b.iter(|| {
            camera.yaw += 0.01;
            frame.set_camera(&camera, 16.0 / 9.0);
            frame.extract(&evaluated, |_| false);
            black_box(frame.instance_count())
        });
    });
    let mut all = false;
    c.bench_function("extract 10k objects (selection toggled)", |b| {
        b.iter(|| {
            all = !all;
            frame.extract(&evaluated, |_| all);
            black_box(frame.instance_count())
        });
    });
}

fn components(c: &mut Criterion) {
    let mut doc = Document::new("mesh");
    let mesh: NodeId = doc.scene.insert(Node::new(
        "Mesh",
        ObjectKind::polygon(MeshHandle::new(primitives::plane(1000.0, 1000.0, 708, 708))),
    ));
    let mut evaluated = EvaluatedScene::new();
    evaluated.update(&doc.scene);
    let mut frame = FrameData::new();
    frame.extract(&evaluated, |_| false);
    let mut group = c.benchmark_group("1M triangles");
    group.sample_size(10);
    for (name, component) in [
        ("points overlay", Component::Point),
        ("edges overlay", Component::Edge),
        ("polygons overlay", Component::Polygon),
    ] {
        group.bench_function(name, |b| {
            b.iter(|| {
                // Built again when the mode changes.
                frame.extract_components(&evaluated, None, &[]);
                frame.extract_components(&evaluated, Some((mesh, component)), &[]);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, extraction, components);
criterion_main!(benches);
