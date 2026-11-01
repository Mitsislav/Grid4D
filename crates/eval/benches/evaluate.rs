//! Evaluation at the performance budget's size: 10 000 objects after an
//! edit and while playing, and the generators' work — a Subdivision Surface,
//! an Extrude of a text and a Cloner of a thousand cubes (`cargo bench -p
//! grid4d-eval`).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use glam::Vec3;
use grid4d_eval::{EvaluatedScene, generator_mesh};
use grid4d_geom::cloner::{Cloner, GridArray, Layout};
use grid4d_scene::{Axis, Curve, Document, Key, Node, NodeData, NodeId, ObjectKind, ObjectKindTag, Param, Track};

/// `count` primitives in groups of 100, every `animated`-th one moving over
/// frames 0–90.
fn objects(count: usize, animated: usize) -> (Document, NodeId) {
    let mut doc = Document::new("bench");
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
    let mut group = None;
    let mut last = None;
    for k in 0..count {
        if k % 100 == 0 {
            group = Some(doc.scene.insert(Node::new("Group", ObjectKind::Null)));
        }
        let mut data = NodeData::new("Object", kinds[k % kinds.len()].default_kind());
        let place = |i: usize| f32::from(u16::try_from(i).unwrap_or(u16::MAX)) * 250.0;
        data.transform.position = Vec3::new(place(k % 100), 0.0, place(k / 100));
        if animated > 0 && k % animated == 0 {
            data.tracks.push(Track {
                param: Param::Position(Axis::Y),
                curve: Curve::from_keys([Key::new(0.0, 0.0), Key::new(90.0, 300.0)]),
            });
        }
        let Some(parent) = group else { continue };
        last = doc.scene.insert_child(parent, Node::from_data(data)).ok();
    }
    (doc, last.unwrap_or_default())
}

fn evaluation(c: &mut Criterion) {
    let mut group = c.benchmark_group("10k objects");
    let (mut doc, edited) = objects(10_000, 0);
    let mut evaluated = EvaluatedScene::new();
    evaluated.update(&doc.scene);
    group.bench_function("evaluate after an edit", |b| {
        b.iter(|| {
            if let Some(node) = doc.scene.get_mut(edited) {
                node.data.transform.position.x += 1.0;
            }
            black_box(evaluated.update(&doc.scene))
        });
    });
    let (doc, _) = objects(10_000, 10);
    let mut evaluated = EvaluatedScene::new();
    let mut frame = 0;
    group.bench_function("evaluate the next frame (1 000 animated)", |b| {
        b.iter(|| {
            frame = (frame + 1) % 91;
            evaluated.set_frame(f64::from(frame));
            black_box(evaluated.update(&doc.scene))
        });
    });
    group.finish();
}

fn generators(c: &mut Criterion) {
    let mut group = c.benchmark_group("generators");

    // A Subdivision Surface, three levels over a cube of 6 × 6 × 6 segments.
    let mut doc = Document::new("subdivision");
    let mut kind = ObjectKindTag::SubdivisionSurface.default_kind();
    if let ObjectKind::SubdivisionSurface { viewport_levels, .. } = &mut kind {
        *viewport_levels = 3;
    }
    let sds = doc.scene.insert(Node::new("Subdivision Surface", kind));
    let cage = ObjectKind::Cube {
        size: Vec3::splat(200.0),
        segments: [6, 6, 6],
    };
    let _ = doc.scene.insert_child(sds, Node::new("Cube", cage));
    group.bench_function("subdivision surface (3 levels)", |b| {
        b.iter(|| black_box(generator_mesh(&doc.scene, sds, 0.0)));
    });

    // An Extrude of a text.
    let mut doc = Document::new("extrude");
    let extrude = doc
        .scene
        .insert(Node::new("Extrude", ObjectKindTag::Extrude.default_kind()));
    let mut text = ObjectKindTag::Text.default_kind();
    if let ObjectKind::Text(text) = &mut text {
        *text = grid4d_geom::spline::Text::new("Grid4D extrudes text");
    }
    let _ = doc.scene.insert_child(extrude, Node::new("Text", text));
    group.bench_function("extrude a text", |b| {
        b.iter(|| black_box(generator_mesh(&doc.scene, extrude, 0.0)));
    });

    // A Cloner of 10 × 10 × 10 cubes, evaluated after an edit.
    let mut doc = Document::new("cloner");
    let cloner = doc.scene.insert(Node::new(
        "Cloner",
        ObjectKind::cloner(Cloner {
            layout: Layout::Grid(GridArray {
                count: [10, 10, 10],
                size: Vec3::splat(400.0),
                ..GridArray::default()
            }),
            ..Cloner::default()
        }),
    ));
    let cube = ObjectKind::Cube {
        size: Vec3::splat(30.0),
        segments: [1, 1, 1],
    };
    let _ = doc.scene.insert_child(cloner, Node::new("Cube", cube));
    let mut evaluated = EvaluatedScene::new();
    group.bench_function("cloner of 1 000 (after an edit)", |b| {
        b.iter(|| {
            if let Some(node) = doc.scene.get_mut(cloner) {
                node.data.transform.position.x += 1.0;
            }
            evaluated.update(&doc.scene);
            black_box(evaluated.instances().len())
        });
    });
    group.finish();
}

criterion_group!(benches, evaluation, generators);
criterion_main!(benches);
