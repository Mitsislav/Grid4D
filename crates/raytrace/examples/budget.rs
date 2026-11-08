//! The render-time budget: reference scenes rendered at fixed settings, timed,
//! and compared with a 4096-sample reference of themselves (see
//! `docs/performance.md` ▸ *Render time*). Phase 16 measures here before and
//! after its changes.
//!
//! ```text
//! cargo run --release -p grid4d-raytrace --example budget -- [scene …] [--pictures <folder>]
//! ```
//!
//! Scenes: `demo`, `studio`, `interior`, `dense` (all when none is named).
//! With `--pictures`, each scene's reference and 16-sample pictures are saved
//! there as PNG, to look at.
//!
//! For each scene: the time to evaluate it and build its triangles and BVH; a 1280 × 720 render at 16
//! samples (the size of the phase's acceptance shot); and at 320 × 180 the
//! time and the error of 16, 64 and 256 samples against the reference (a
//! different seed, 4096 samples), as the root mean square of the 8-bit sRGB
//! values (0 – 255; lower is cleaner).

use std::{
    error::Error,
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use glam::{Vec3, camera::rh};
use grid4d_eval::EvaluatedScene;
use grid4d_raytrace::{CameraProjection, Image, Pictures, RenderCamera, RenderOptions, RenderScene, Texturing, render};
use grid4d_scene::{
    Document, Falloff, LightDetails, LightShadow, LightType, Material, MaterialTag, Node, ObjectKind, Tag, Transform,
};

/// A reference scene: its document, camera and bounces.
struct Scene {
    name: &'static str,
    doc: Document,
    camera: RenderCamera,
    bounces: u32,
}

fn looking(eye: Vec3, target: Vec3, fov_y_deg: f32) -> RenderCamera {
    RenderCamera {
        world: rh::view::look_at_mat4(eye, target, Vec3::Y).inverse(),
        projection: CameraProjection::Perspective {
            fov_y: fov_y_deg.to_radians(),
        },
        lens: None,
        end: None,
    }
}

/// Adds `kind`, placed by `transform`, with `material` (named after it).
fn add(doc: &mut Document, kind: ObjectKind, transform: Transform, material: Material) {
    let id = doc.scene.materials_mut().insert(material);
    let mut node = Node::new(material_name(doc, id), kind).with_transform(transform);
    node.data.tags = vec![Tag::Material(MaterialTag::new(Some(id)))];
    doc.scene.insert(node);
}

fn material_name(doc: &Document, id: grid4d_scene::MaterialId) -> String {
    doc.scene
        .materials()
        .get(id)
        .map_or_else(|| "Object".to_owned(), |m| m.name.clone())
}

fn material(name: &str, color: [f32; 3], roughness: f32, edit: impl FnOnce(&mut Material)) -> Material {
    let mut material = Material::new(name);
    material.color = color;
    material.roughness = roughness;
    edit(&mut material);
    material
}

fn light(kind: LightType, intensity: f32, shadow: LightShadow, details: LightDetails) -> ObjectKind {
    ObjectKind::Light {
        color: [1.0, 0.96, 0.9],
        intensity,
        light_type: kind,
        details,
        shadow,
        linking: grid4d_scene::Linking::default(),
        group: String::new(),
    }
}

fn at(position: Vec3) -> Transform {
    Transform::from_position(position)
}

/// The demo scene through its camera (1.1's first reference picture).
fn demo() -> Scene {
    let doc = Document::with_demo_scene();
    let (id, fov) = doc
        .scene
        .iter()
        .find_map(|(id, node)| match node.data.kind {
            ObjectKind::Camera { fov_y_deg, .. } => Some((id, fov_y_deg)),
            _ => None,
        })
        .unwrap_or_default();
    let camera = RenderCamera {
        world: doc.scene.world_matrix(id),
        projection: CameraProjection::Perspective {
            fov_y: fov.to_radians(),
        },
        lens: None,
        end: None,
    };
    Scene {
        name: "demo",
        doc,
        camera,
        bounces: 2,
    }
}

/// A product on a sweep: glass, polished and brushed metal, a coated
/// plastic, a soft key light, a glowing panel (lights only through bounces in
/// 1.1).
fn studio() -> Scene {
    let mut doc = Document::new("studio");
    let grey = |name| material(name, [0.6, 0.6, 0.62], 0.7, |_| {});
    add(
        &mut doc,
        ObjectKind::Plane {
            width: 1200.0,
            depth: 1200.0,
            width_segments: 1,
            depth_segments: 1,
        },
        at(Vec3::ZERO),
        grey("Floor"),
    );
    add(
        &mut doc,
        ObjectKind::Plane {
            width: 1200.0,
            depth: 600.0,
            width_segments: 1,
            depth_segments: 1,
        },
        Transform::from_position(Vec3::new(0.0, 300.0, -300.0)).with_rotation_deg(Vec3::new(0.0, 90.0, 0.0)),
        grey("Backdrop"),
    );
    add(
        &mut doc,
        ObjectKind::Sphere {
            radius: 60.0,
            segments: 64,
        },
        at(Vec3::new(-110.0, 60.0, 40.0)),
        material("Glass", [0.95, 0.97, 1.0], 0.02, |m| m.transmission = 1.0),
    );
    add(
        &mut doc,
        ObjectKind::Torus {
            ring_radius: 55.0,
            pipe_radius: 18.0,
            ring_segments: 64,
            pipe_segments: 24,
        },
        Transform::from_position(Vec3::new(30.0, 18.0, 70.0)),
        material("Brushed", [0.9, 0.7, 0.45], 0.35, |m| m.metallic = 1.0),
    );
    add(
        &mut doc,
        ObjectKind::Cylinder {
            radius: 35.0,
            height: 160.0,
            rotation_segments: 64,
            height_segments: 1,
        },
        at(Vec3::new(140.0, 80.0, -40.0)),
        material("Chrome", [0.95, 0.95, 0.95], 0.05, |m| m.metallic = 1.0),
    );
    add(
        &mut doc,
        ObjectKind::Cube {
            size: Vec3::new(90.0, 90.0, 90.0),
            segments: [1, 1, 1],
        },
        Transform::from_position(Vec3::new(-10.0, 45.0, -110.0)).with_rotation_deg(Vec3::new(25.0, 0.0, 0.0)),
        material("Coated", [0.7, 0.08, 0.05], 0.6, |m| m.clearcoat = 1.0),
    );
    add(
        &mut doc,
        ObjectKind::Plane {
            width: 200.0,
            depth: 80.0,
            width_segments: 1,
            depth_segments: 1,
        },
        Transform::from_position(Vec3::new(-250.0, 260.0, 150.0)).with_rotation_deg(Vec3::new(-60.0, 0.0, -110.0)),
        material("Panel", [0.0; 3], 1.0, |m| {
            m.emission = [1.0, 0.95, 0.85];
            m.emission_strength = 4.0;
        }),
    );
    let mut details = LightDetails {
        outer_angle_deg: 50.0,
        inner_angle_deg: 20.0,
        ..LightDetails::default()
    };
    details.falloff = Falloff::None;
    doc.scene.insert(
        Node::new("Key", light(LightType::Spot, 1.4, LightShadow::Area, details)).with_transform(
            Transform::from_position(Vec3::new(260.0, 420.0, 320.0)).with_rotation_deg(Vec3::new(38.0, -45.0, 0.0)),
        ),
    );
    Scene {
        name: "studio",
        doc,
        camera: looking(Vec3::new(40.0, 170.0, 520.0), Vec3::new(10.0, 50.0, 0.0), 35.0),
        bounces: 4,
    }
}

/// A closed room lit by one small soft lamp: almost all light is bounced.
fn interior() -> Scene {
    let mut doc = Document::new("interior");
    add(
        &mut doc,
        ObjectKind::Cube {
            size: Vec3::new(800.0, 300.0, 800.0),
            segments: [1, 1, 1],
        },
        at(Vec3::new(0.0, 150.0, 0.0)),
        material("Walls", [0.8, 0.78, 0.72], 0.9, |_| {}),
    );
    add(
        &mut doc,
        ObjectKind::Cube {
            size: Vec3::new(200.0, 80.0, 100.0),
            segments: [1, 1, 1],
        },
        at(Vec3::new(-120.0, 40.0, -200.0)),
        material("Sofa", [0.15, 0.3, 0.55], 0.8, |_| {}),
    );
    add(
        &mut doc,
        ObjectKind::Cylinder {
            radius: 60.0,
            height: 70.0,
            rotation_segments: 48,
            height_segments: 1,
        },
        at(Vec3::new(120.0, 35.0, 20.0)),
        material("Table", [0.55, 0.35, 0.2], 0.4, |_| {}),
    );
    add(
        &mut doc,
        ObjectKind::Sphere {
            radius: 25.0,
            segments: 32,
        },
        at(Vec3::new(120.0, 95.0, 20.0)),
        material("Vase", [0.9, 0.85, 0.2], 0.25, |m| m.metallic = 1.0),
    );
    let details = LightDetails {
        falloff: Falloff::InverseSquare,
        radius: 250.0,
        ..LightDetails::default()
    };
    doc.scene.insert(
        Node::new("Lamp", light(LightType::Omni, 0.5, LightShadow::Area, details))
            .with_transform(at(Vec3::new(-200.0, 260.0, 150.0))),
    );
    Scene {
        name: "interior",
        doc,
        camera: looking(Vec3::new(330.0, 170.0, 330.0), Vec3::new(-60.0, 70.0, -120.0), 60.0),
        bounces: 4,
    }
}

/// One sphere of about a million triangles on a floor in the sun.
fn dense() -> Scene {
    let mut doc = Document::new("dense");
    add(
        &mut doc,
        ObjectKind::Plane {
            width: 1000.0,
            depth: 1000.0,
            width_segments: 1,
            depth_segments: 1,
        },
        at(Vec3::ZERO),
        material("Floor", [0.5, 0.5, 0.5], 0.8, |_| {}),
    );
    add(
        &mut doc,
        ObjectKind::Sphere {
            radius: 100.0,
            segments: 1000,
        },
        at(Vec3::new(0.0, 100.0, 0.0)),
        material("Ball", [0.7, 0.4, 0.3], 0.4, |_| {}),
    );
    doc.scene.insert(
        Node::new(
            "Sun",
            light(
                LightType::Infinite,
                1.2,
                LightShadow::ShadowMaps,
                LightDetails::default(),
            ),
        )
        .with_transform(Transform::default().with_rotation_deg(Vec3::new(30.0, -50.0, 0.0))),
    );
    Scene {
        name: "dense",
        doc,
        camera: looking(Vec3::new(180.0, 200.0, 320.0), Vec3::new(0.0, 80.0, 0.0), 40.0),
        bounces: 2,
    }
}

fn build(scene: &Scene, aspect: f32) -> RenderScene {
    let mut evaluated = EvaluatedScene::for_render();
    evaluated.update(&scene.doc.scene);
    let pictures = Pictures::new();
    let texturing = Texturing {
        materials: scene.doc.scene.materials(),
        pictures: &pictures,
        camera: scene.camera,
        aspect,
        cameras: &[],
    };
    RenderScene::textured(&evaluated, &texturing)
}

fn options(scene: &Scene, width: u32, height: u32, samples: u32, seed: u32) -> RenderOptions {
    RenderOptions {
        width,
        height,
        samples,
        bounces: scene.bounces,
        seed,
        auto_light: true,
        clamp_indirect: true,
        emissive_lights: true,
    }
}

/// The best of `runs` timings of `f`, and its last result.
fn timed<T>(runs: u32, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut best = Duration::MAX;
    let mut last = None;
    for _ in 0..runs.max(1) {
        let started = Instant::now();
        let result = f();
        best = best.min(started.elapsed());
        last = Some(result);
    }
    (best, last.unwrap_or_else(f))
}

/// Root mean square difference of two pictures' 8-bit sRGB values.
fn rmse(a: &Image, b: &Image) -> f64 {
    let (a, b) = (a.to_rgb8(), b.to_rgb8());
    let sum: f64 = a
        .iter()
        .zip(&b)
        .map(|(x, y)| {
            let d = f64::from(*x) - f64::from(*y);
            d * d
        })
        .sum();
    (sum / a.len().max(1) as f64).sqrt()
}

fn save(image: &Image, path: &Path) -> Result<(), Box<dyn Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), image.width, image.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&image.to_rgb8())?;
    Ok(())
}

fn ms(d: Duration) -> String {
    if d.as_secs_f64() >= 10.0 {
        format!("{:.1} s", d.as_secs_f64())
    } else {
        format!("{:.0} ms", d.as_secs_f64() * 1000.0)
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut names = Vec::new();
    let mut pictures: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--pictures" {
            pictures = args.next().map(PathBuf::from);
        } else {
            names.push(arg);
        }
    }
    if let Some(folder) = &pictures {
        std::fs::create_dir_all(folder)?;
    }
    let scenes = [demo, studio, interior, dense];
    println!("| Scene | Triangles | Build | 1280 × 720, 16 samples | 16 samples | 64 | 256 | 4096 (reference) |");
    println!("|-------|-----------|-------|------------------------|------------|----|-----|------------------|");
    for make in scenes {
        let scene = make();
        if !names.is_empty() && !names.iter().any(|n| n == scene.name) {
            continue;
        }
        let (built, wide) = timed(3, || build(&scene, 16.0 / 9.0));
        let (full, _) = timed(2, || render(&wide, &scene.camera, &options(&scene, 1280, 720, 16, 0)));
        let (reference_time, reference) =
            timed(1, || render(&wide, &scene.camera, &options(&scene, 320, 180, 4096, 1)));
        let mut cells = Vec::new();
        for samples in [16, 64, 256] {
            let (time, image) = timed(2, || {
                render(&wide, &scene.camera, &options(&scene, 320, 180, samples, 0))
            });
            cells.push(format!("{} / {:.2}", ms(time), rmse(&image, &reference)));
            if samples == 16
                && let Some(folder) = &pictures
            {
                save(&image, &folder.join(format!("{}-16.png", scene.name)))?;
            }
        }
        if let Some(folder) = &pictures {
            save(&reference, &folder.join(format!("{}-4096.png", scene.name)))?;
        }
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            scene.name,
            wide.triangle_count(),
            ms(built),
            ms(full),
            cells[0],
            cells[1],
            cells[2],
            ms(reference_time),
        );
    }
    Ok(())
}
