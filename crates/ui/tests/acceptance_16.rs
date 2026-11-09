//! Phase 16's acceptance, as one run through the editor (see
//! `docs/IMPLEMENTATION_PLAN.md`, Phase 16 ▸ Acceptance), at a small size so
//! it runs as a test: a document from 1.1 opens with every new setting at
//! 1.1's behaviour; a product shot is lit by a panorama and a large
//! rectangle light, a glass bottle stands in front with depth of field on
//! it, a wheel spins with motion blur; rendered with few samples, adaptive
//! sampling and the denoiser it comes within a set error of a many-sample
//! render; its passes land in one multi-layer OpenEXR; a queue of two takes
//! renders unattended; a daylight scene shows its sky; the document is
//! saved as version 5 and opens the same; 1.0 and 1.1 files open.

#![allow(clippy::unwrap_used, reason = "test code")]

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use glam::Vec3;
use grid4d_render::OrbitCamera;
use grid4d_scene::{
    AreaShape, Axis, ColorSpace, Curve, Denoise, Document, Environment, FrameRange, ImageFormat, Interpolation, Key,
    Lens, LightDetails, LightShadow, LightType, Linking, Material, MaterialTag, Node, NodeId, ObjectKind, Param, Pass,
    Sky, Tag, Take, Texture, TextureSource, Track, Transform,
};
use grid4d_ui::{
    EditorCommand, EditorState, ViewportPanel,
    render_queue::{JobStatus, JobTake},
};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../io/tests/fixtures");

fn folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grid4d-acceptance-16-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Editor {
    doc: Document,
    editor: EditorState,
    viewport: ViewportPanel,
}

impl Editor {
    fn new(doc: Document) -> Self {
        Self {
            doc,
            editor: EditorState::default(),
            viewport: ViewportPanel::new(None),
        }
    }

    fn run(&mut self, commands: impl IntoIterator<Item = EditorCommand>) {
        for command in commands {
            self.editor.commands.push(command);
            self.frame();
        }
    }

    fn frame(&mut self) {
        grid4d_ui::commands::apply_pending(&mut self.doc, &mut self.editor, &mut self.viewport);
    }

    /// Runs frames until no render and no queue job runs.
    fn wait(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(300);
        self.frame();
        while self.editor.renders.running_number().is_some() || self.editor.render_queue.is_running() {
            assert!(Instant::now() < deadline, "the renders did not end");
            std::thread::sleep(Duration::from_millis(5));
            self.frame();
        }
        self.frame();
    }

    /// Renders the document through camera object `camera` (Shift+R as the
    /// view looking through it does); returns the picture's sRGB bytes.
    fn render(&mut self, camera: NodeId, path: Option<&Path>) -> Vec<u8> {
        let pictures = grid4d_ui::textures::decode_all(self.doc.scene.textures()).0;
        let replay = self.editor.simulation.replay(&self.doc);
        let number =
            self.editor
                .renders
                .start(&self.doc, pictures, &OrbitCamera::default(), Some(camera), path, replay);
        self.wait();
        let picture = self.editor.renders.picture(number).unwrap();
        assert_eq!(picture.state, grid4d_ui::rendering::RenderState::Finished);
        picture.rgb.clone()
    }
}

/// Root mean square difference of two sRGB pictures, 0 – 255.
fn rmse(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let sum: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| {
            let d = f64::from(*x) - f64::from(*y);
            d * d
        })
        .sum();
    (sum / a.len().max(1) as f64).sqrt()
}

/// A small panorama: a dim grey dome, a darker floor, one bright window.
fn panorama() -> Vec<u8> {
    let (w, h) = (64u32, 32u32);
    let pixels: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let window = (20..28).contains(&x) && (6..12).contains(&y);
            let v = if window {
                40.0
            } else if y < h / 2 {
                0.4
            } else {
                0.1
            };
            [v, v * 0.95, v * 0.9, 1.0]
        })
        .collect();
    grid4d_io::exr_bytes(w, h, grid4d_io::Channels::Rgb, &pixels).unwrap()
}

fn material(doc: &mut Document, name: &str, edit: impl FnOnce(&mut Material)) -> grid4d_scene::MaterialId {
    let mut m = Material::new(name);
    edit(&mut m);
    doc.scene.materials_mut().insert(m)
}

fn tagged(doc: &mut Document, mut node: Node, material: grid4d_scene::MaterialId) -> NodeId {
    node.data.tags = vec![Tag::Material(MaterialTag::new(Some(material)))];
    doc.scene.insert(node)
}

/// The product shot: a floor, a label (a box) behind a glass bottle, a
/// spinning wheel, a panorama and a large rectangle light; a camera focused
/// on the bottle. Returns the document and the camera.
fn product_shot() -> (Document, NodeId) {
    let mut doc = Document::new("Product");
    let panorama = doc.scene.textures_mut().insert(Texture {
        name: "Studio".to_owned(),
        source: TextureSource::Embedded {
            file_name: "studio.exr".to_owned(),
            bytes: panorama().into(),
        },
        color_space: ColorSpace::Linear,
    });
    doc.scene.insert(Node::new(
        "Environment",
        ObjectKind::Environment(Environment {
            texture: Some(panorama),
            intensity: 1.0,
            rotation: 0.0,
            visible: true,
            group: "Studio".to_owned(),
        }),
    ));
    let floor = material(&mut doc, "Floor", |m| m.color = [0.6, 0.6, 0.6]);
    tagged(
        &mut doc,
        Node::new(
            "Floor",
            ObjectKind::Plane {
                width: 800.0,
                depth: 800.0,
                width_segments: 1,
                depth_segments: 1,
            },
        ),
        floor,
    );
    let label = material(&mut doc, "Label", |m| (m.color, m.roughness) = ([0.8, 0.15, 0.1], 0.4));
    tagged(
        &mut doc,
        Node::new(
            "Label",
            ObjectKind::Cube {
                size: Vec3::new(60.0, 80.0, 10.0),
                segments: [1, 1, 1],
            },
        )
        .with_position(Vec3::new(0.0, 40.0, 60.0)),
        label,
    );
    let glass = material(&mut doc, "Glass", |m| {
        (m.color, m.transmission, m.roughness, m.ior) = ([0.9, 1.0, 0.95], 1.0, 0.05, 1.5);
    });
    let bottle = tagged(
        &mut doc,
        Node::new(
            "Bottle",
            ObjectKind::Cylinder {
                radius: 20.0,
                height: 90.0,
                rotation_segments: 32,
                height_segments: 1,
            },
        )
        .with_position(Vec3::new(-30.0, 45.0, -40.0)),
        glass,
    );
    // A wheel turning a quarter turn a frame about its axle (Z).
    let dark = material(&mut doc, "Rubber", |m| (m.color, m.roughness) = ([0.1, 0.1, 0.1], 0.7));
    let mut wheel = Node::new(
        "Wheel",
        ObjectKind::Torus {
            ring_radius: 40.0,
            pipe_radius: 8.0,
            ring_segments: 32,
            pipe_segments: 12,
        },
    )
    .with_transform(
        Transform::from_position(Vec3::new(110.0, 48.0, 20.0)).with_rotation_deg(Vec3::new(0.0, 90.0, 0.0)),
    );
    wheel.data.tracks.push(Track {
        param: Param::Rotation(Axis::Z),
        curve: Curve::from_keys([
            Key {
                interpolation: Interpolation::Linear,
                ..Key::new(0.0, 0.0)
            },
            Key {
                interpolation: Interpolation::Linear,
                ..Key::new(10.0, std::f32::consts::TAU * 2.5)
            },
        ]),
    });
    tagged(&mut doc, wheel, dark);
    // A large rectangle light above the front, facing down.
    doc.scene.insert(
        Node::new(
            "Softbox",
            ObjectKind::Light {
                color: [1.0, 0.97, 0.92],
                intensity: 2.0,
                light_type: LightType::Area,
                details: LightDetails {
                    area_shape: AreaShape::Rectangle,
                    area_size: [200.0, 200.0],
                    ..LightDetails::default()
                },
                shadow: LightShadow::Area,
                linking: Linking::default(),
                group: "Key".to_owned(),
            },
        )
        .with_transform(
            Transform::from_position(Vec3::new(0.0, 300.0, -100.0)).with_rotation_deg(Vec3::new(0.0, -70.0, 0.0)),
        ),
    );
    // The camera, focused on the bottle, its shutter open a whole frame.
    let look = glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 90.0, -400.0), Vec3::new(10.0, 45.0, 0.0), Vec3::Y);
    let camera = doc.scene.insert(
        Node::new(
            "Camera",
            ObjectKind::Camera {
                fov_y_deg: 35.0,
                lens: Lens {
                    depth_of_field: true,
                    focus_object: Some(bottle),
                    f_stop: 2.0,
                    shutter: 1.0,
                    ..Lens::default()
                },
            },
        )
        .with_transform(Transform::from_matrix(&look.inverse())),
    );
    doc.time.set_frame(2);
    doc.render.frames = FrameRange::Current;
    doc.render.width = 96;
    doc.render.height = 54;
    doc.render.motion_blur = true;
    doc.render.bounces = 3;
    (doc, camera)
}

#[test]
fn phase_16_a_product_shot_lit_rendered_and_queued() {
    let dir = folder();

    // 1. A 1.1 document opens with every new setting at 1.1's behaviour,
    //    and 1.0's opens too.
    let mut e = Editor::new(Document::new("Start"));
    e.run([EditorCommand::OpenFile(Path::new(FIXTURES).join("full_v4.g4d"))]);
    assert!(e.editor.file.alert.is_none(), "{:?}", e.editor.file.alert);
    let render = &e.doc.render;
    assert!(!render.adaptive && !render.motion_blur && render.passes.is_empty());
    assert_eq!((render.denoise, render.region), (Denoise::Off, None));
    for (_, node) in e.doc.scene.iter() {
        if let ObjectKind::Camera { lens, .. } = &node.data.kind {
            assert_eq!(*lens, Lens::default(), "pinhole cameras, as in 1.1");
        }
    }
    for (_, material) in e.doc.scene.materials().iter() {
        assert!(material.subsurface == 0.0 && !material.displace);
    }
    e.run([EditorCommand::OpenFile(Path::new(FIXTURES).join("full_v3.g4d"))]);
    assert!(e.editor.file.alert.is_none(), "{:?}", e.editor.file.alert);

    // 2. The product shot, rendered with few samples, adaptively and
    //    denoised, against many samples without either.
    let (doc, camera) = product_shot();
    let mut e = Editor::new(doc);
    let quick = {
        let render = &mut e.doc.render;
        (render.samples, render.adaptive, render.min_samples) = (32, true, 8);
        render.denoise = Denoise::Final;
        render.passes = [
            Pass::DiffuseDirect,
            Pass::SpecularDirect,
            Pass::Transmission,
            Pass::Depth,
            Pass::ObjectId,
            Pass::LightGroups,
        ]
        .into_iter()
        .collect();
        (render.format, render.output) = (ImageFormat::Exr, "renders/shot".to_owned());
        let path = dir.join("product.g4d");
        grid4d_io::save(&e.doc, &path).unwrap();
        e.render(camera, Some(&path))
    };
    let noisy = {
        let render = &mut e.doc.render;
        (render.adaptive, render.denoise, render.save) = (false, Denoise::Off, false);
        e.render(camera, None)
    };
    let reference = {
        e.doc.render.samples = 1024;
        e.render(camera, None)
    };
    let (quick, noisy) = (rmse(&quick, &reference), rmse(&noisy, &reference));
    assert!(quick < 6.0, "adaptive and denoised: {quick:.2} from the reference");
    assert!(
        quick < noisy,
        "cleaner than the same samples plain: {quick:.2} against {noisy:.2}"
    );

    // 3. The passes in one multi-layer OpenEXR, the objects named.
    let exr = std::fs::read(dir.join("renders").join("shot.exr")).unwrap();
    let has = |text: &str| exr.windows(text.len()).any(|w| w == text.as_bytes());
    for channel in [
        "DiffuseDirect.R",
        "Transmission.G",
        "Depth.Z",
        "ObjectID.ID",
        "Group.Key.R",
        "Group.Studio.B",
        "Noisy.R",
    ] {
        assert!(has(channel), "{channel} in the file");
    }
    assert!(has("= Bottle"), "the IDs named");

    // 4. Saved as version 5, it opens the same.
    let path = dir.join("product.g4d");
    e.doc.render.samples = 4;
    grid4d_io::save(&e.doc, &path).unwrap();
    let again = grid4d_io::load(&path).unwrap();
    assert_eq!(
        grid4d_io::to_bytes(&again).unwrap(),
        grid4d_io::to_bytes(&e.doc).unwrap()
    );

    // 5. A queue of two takes renders unattended.
    for (name, output) in [("Wide", "renders/wide"), ("Close", "renders/close")] {
        let mut take = Take::new(name);
        let mut settings = e.doc.render.clone();
        (settings.output, settings.format, settings.save) = (output.to_owned(), ImageFormat::Png, true);
        settings.passes = grid4d_scene::Passes::NONE;
        (settings.width, settings.height, settings.samples) = (32, 18, 2);
        take.render = Some(settings);
        take.camera = Some(camera);
        e.doc.takes.insert(take);
    }
    grid4d_io::save(&e.doc, &path).unwrap();
    let queue = &mut e.editor.render_queue;
    for name in ["Wide", "Close"] {
        queue.jobs.push(grid4d_ui::render_queue::QueueJob::new(
            path.clone(),
            JobTake::Named(name.to_owned()),
        ));
    }
    e.run([EditorCommand::StartRenderQueue]);
    e.wait();
    let statuses: Vec<JobStatus> = e.editor.render_queue.jobs.iter().map(|j| j.status).collect();
    assert_eq!(statuses, [JobStatus::Finished, JobStatus::Finished]);
    for file in ["wide.png", "close.png"] {
        assert!(dir.join("renders").join(file).exists(), "{file}");
    }

    // 6. A daylight scene: the Sky surrounds it (the viewport's background
    //    and light), and renders a blue sky above.
    let mut day = Document::new("Daylight");
    day.scene.insert(Node::new("Sky", ObjectKind::Sky(Sky::default())));
    let look = glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 50.0, -300.0), Vec3::new(0.0, 120.0, 0.0), Vec3::Y);
    let camera = day.scene.insert(
        Node::new(
            "Camera",
            ObjectKind::Camera {
                fov_y_deg: 50.0,
                lens: Lens::default(),
            },
        )
        .with_transform(Transform::from_matrix(&look.inverse())),
    );
    (day.render.width, day.render.height, day.render.samples, day.render.save) = (32, 18, 4, false);
    let mut evaluated = grid4d_eval::EvaluatedScene::new();
    evaluated.update(&day.scene);
    assert!(
        evaluated.environment().is_some(),
        "the viewport draws and lights with it"
    );
    let mut e = Editor::new(day);
    let sky = e.render(camera, None);
    let top = &sky[..32 * 3];
    let (r, b): (u32, u32) = top
        .chunks(3)
        .fold((0, 0), |(r, b), p| (r + u32::from(p[0]), b + u32::from(p[2])));
    assert!(b > r, "a blue sky above: red {r}, blue {b}");
}
