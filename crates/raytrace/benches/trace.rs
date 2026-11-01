//! The path tracer on the demo scene: building its BVH, and a small render
//! (`cargo bench -p grid4d-raytrace`).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use grid4d_eval::EvaluatedScene;
use grid4d_raytrace::{RenderCamera, RenderOptions, RenderScene, render};
use grid4d_scene::Document;

fn trace(c: &mut Criterion) {
    let doc = Document::with_demo_scene();
    let mut evaluated = EvaluatedScene::for_render();
    evaluated.update(&doc.scene);
    c.bench_function("demo scene: build", |b| {
        b.iter(|| black_box(RenderScene::new(&evaluated)))
    });

    let scene = RenderScene::new(&evaluated);
    let Some((min, max)) = scene.bounds() else {
        return;
    };
    let camera = RenderCamera::framing(min, max, 40f32.to_radians());
    let options = RenderOptions {
        width: 320,
        height: 180,
        samples: 4,
        ..RenderOptions::from(&doc.render)
    };
    let mut group = c.benchmark_group("demo scene");
    group.sample_size(10);
    group.bench_function("render 320 × 180, 4 samples", |b| {
        b.iter(|| black_box(render(&scene, &camera, &options)));
    });
    group.finish();
}

criterion_group!(benches, trace);
criterion_main!(benches);
