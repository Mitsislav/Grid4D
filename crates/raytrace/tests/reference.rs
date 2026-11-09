//! Reference-image tests: small renders compared with committed pictures
//! within a tolerance (floating-point details differ a little between
//! machines) — the demo scene through its camera, and a scene of textured
//! materials (pictures through UV and Box projections, a layer cut out
//! by its Opacity, a bump map, glass and a clear coat), a scene lit by an
//! area light of each shape, a daylight scene under a Sky, a product lit by
//! an Environment picture, depth of field through a six-bladed aperture and
//! a spinning wheel blurred by its motion.
//!
//! After an intended change to the renderer, write new references with
//! `GRID4D_UPDATE_REFERENCE=1 cargo test -p grid4d-raytrace --test reference`
//! and look at them before committing.

#![allow(clippy::unwrap_used, reason = "test code")]

use std::{fs::File, io::BufWriter, sync::Arc};

use glam::Vec3;
use grid4d_eval::EvaluatedScene;
use grid4d_raytrace::{
    CameraProjection, Image, Picture, Pictures, RenderCamera, RenderOptions, RenderScene, Texels, Texturing, ThinLens,
    render,
};
use grid4d_scene::{
    AreaShape, Channel, Document, Environment, LightDetails, LightShadow, LightType, Material, MaterialTag, Node,
    ObjectKind, Projection, Sky, SunPlacement, Tag, Texture, TextureId, TextureSlot, Transform,
};

const DEMO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/demo_reference.png");
const TEXTURED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/textured_reference.png");
const AREAS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/area_reference.png");
const DAYLIGHT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/daylight_reference.png");
const PRODUCT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/environment_reference.png");
const FOCUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/depth_of_field_reference.png"
);
const WHEEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/motion_blur_reference.png");
const WAX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/subsurface_reference.png");

fn demo() -> Image {
    let doc = Document::with_demo_scene();
    let mut evaluated = EvaluatedScene::for_render();
    evaluated.update(&doc.scene);
    let (id, fov) = doc
        .scene
        .iter()
        .find_map(|(id, node)| match node.data.kind {
            ObjectKind::Camera { fov_y_deg, .. } => Some((id, fov_y_deg)),
            _ => None,
        })
        .expect("the demo scene has a camera");
    let camera = RenderCamera {
        world: doc.scene.world_matrix(id),
        projection: CameraProjection::Perspective {
            fov_y: fov.to_radians(),
        },
        lens: None,
        end: None,
    };
    let options = RenderOptions {
        width: 96,
        height: 54,
        samples: 8,
        bounces: 1,
        seed: 0,
        auto_light: true,
        clamp_indirect: true,
        emissive_lights: true,
        adaptive: None,
        region: None,
    };
    render(&RenderScene::new(&evaluated), &camera, &options)
}

/// A `size` × `size` picture made by `texel(x, y)`.
fn picture(size: u32, srgb: bool, texel: impl Fn(u32, u32) -> [u8; 4]) -> Arc<Picture> {
    let bytes: Vec<u8> = (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .flat_map(|(x, y)| texel(x, y))
        .collect();
    let texels = if srgb {
        Texels::Srgb8(bytes.into())
    } else {
        Texels::Linear8(bytes.into())
    };
    Arc::new(Picture::new(size, size, texels).expect("a whole picture"))
}

/// Adds a texture named `name` with `picture` and returns its slot.
fn add(doc: &mut Document, pictures: &mut Pictures, name: &str, picture: Arc<Picture>) -> (TextureId, TextureSlot) {
    let id = doc.scene.textures_mut().insert(Texture::linked(name.into()));
    pictures.insert(id, picture);
    (id, TextureSlot::new(id))
}

fn tagged(kind: ObjectKind, position: Vec3, tags: Vec<MaterialTag>) -> Node {
    let mut node = Node::new("Object", kind).with_position(position);
    node.data.tags = tags.into_iter().map(Tag::Material).collect();
    node
}

/// A floor of checked tiles, a planked box with a round label over it, a
/// bumpy coated ball and a glass block, in one sun with soft shadows.
fn textured() -> Image {
    let mut doc = Document::new("textured");
    let mut pictures = Pictures::new();
    let checker = picture(8, true, |x, y| {
        if (x + y) % 2 == 0 {
            [200, 190, 170, 255]
        } else {
            [70, 80, 95, 255]
        }
    });
    let planks = picture(16, true, |x, y| {
        let shade = u8::try_from(140 + (y / 4) * 20 + (x % 5) * 3).unwrap_or(255);
        [shade, shade / 2 + 30, shade / 4 + 10, 255]
    });
    let disc = picture(16, false, |x, y| {
        let (dx, dy) = (
            f32::from(u8::try_from(x).unwrap_or(0)) - 7.5,
            f32::from(u8::try_from(y).unwrap_or(0)) - 7.5,
        );
        let inside = dx * dx + dy * dy < 36.0;
        [if inside { 255 } else { 0 }, 0, 0, 255]
    });
    let ridges = picture(16, false, |x, _| {
        let v = u8::try_from((x % 4) * 80).unwrap_or(255);
        [v, v, v, 255]
    });

    // The floor: tiles tiled four times across.
    let (_, mut slot) = add(&mut doc, &mut pictures, "checker.png", checker);
    slot.tiling = [4.0, 4.0];
    let mut floor = Material::new("Floor");
    floor.roughness = 0.8;
    *floor.map_mut(Channel::BaseColor) = Some(slot);
    let floor = doc.scene.materials_mut().insert(floor);
    doc.scene.insert(tagged(
        ObjectKind::Plane {
            width: 800.0,
            depth: 800.0,
            width_segments: 1,
            depth_segments: 1,
        },
        Vec3::ZERO,
        vec![MaterialTag::new(Some(floor))],
    ));

    // A box in planks (Box projection) with a red label on each side (its UVs,
    // cut out round).
    let (_, slot) = add(&mut doc, &mut pictures, "planks.png", planks);
    let mut wood = Material::new("Wood");
    wood.roughness = 0.6;
    *wood.map_mut(Channel::BaseColor) = Some(slot);
    let wood = doc.scene.materials_mut().insert(wood);
    let (_, slot) = add(&mut doc, &mut pictures, "disc.png", disc);
    let mut label = Material::new("Label");
    label.color = [0.8, 0.05, 0.03];
    label.roughness = 0.3;
    *label.map_mut(Channel::Opacity) = Some(slot);
    let label = doc.scene.materials_mut().insert(label);
    let mut planked = MaterialTag::new(Some(wood));
    planked.projection = Projection::Box;
    let stuck = MaterialTag::new(Some(label));
    doc.scene.insert(tagged(
        ObjectKind::Cube {
            size: Vec3::new(160.0, 120.0, 120.0),
            segments: [1, 1, 1],
        },
        Vec3::new(-170.0, 60.0, 0.0),
        vec![planked, stuck],
    ));

    // A ball with ridges (Bump) under a clear coat.
    let (_, mut slot) = add(&mut doc, &mut pictures, "ridges.png", ridges);
    slot.tiling = [3.0, 1.0];
    let mut ball = Material::new("Ball");
    ball.color = [0.05, 0.25, 0.6];
    ball.roughness = 0.7;
    ball.bump_height = 6.0;
    ball.clearcoat = 1.0;
    *ball.map_mut(Channel::Bump) = Some(slot);
    let ball = doc.scene.materials_mut().insert(ball);
    doc.scene.insert(tagged(
        ObjectKind::Sphere {
            radius: 70.0,
            segments: 32,
        },
        Vec3::new(60.0, 70.0, 40.0),
        vec![MaterialTag::new(Some(ball))],
    ));

    // A block of tinted glass.
    let mut glass = Material::new("Glass");
    glass.color = [0.75, 0.95, 0.85];
    glass.roughness = 0.05;
    glass.transmission = 1.0;
    let glass = doc.scene.materials_mut().insert(glass);
    doc.scene.insert(tagged(
        ObjectKind::Cube {
            size: Vec3::new(80.0, 160.0, 80.0),
            segments: [1, 1, 1],
        },
        Vec3::new(220.0, 80.0, -60.0),
        vec![MaterialTag::new(Some(glass))],
    ));

    let sun = ObjectKind::Light {
        color: [1.0; 3],
        intensity: 1.2,
        light_type: LightType::Infinite,
        details: LightDetails::default(),
        shadow: LightShadow::Raytraced,
        linking: grid4d_scene::Linking::default(),
        group: String::new(),
    };
    doc.scene.insert(
        Node::new("Sun", sun).with_transform(Transform::default().with_rotation_deg(Vec3::new(30.0, -55.0, 0.0))),
    );

    let mut evaluated = EvaluatedScene::for_render();
    evaluated.update(&doc.scene);
    let camera = RenderCamera::framing(Vec3::new(-180.0, 0.0, -60.0), Vec3::new(200.0, 120.0, 60.0), 0.7);
    let options = RenderOptions {
        width: 160,
        height: 90,
        samples: 16,
        bounces: 3,
        seed: 0,
        auto_light: false,
        clamp_indirect: true,
        emissive_lights: true,
        adaptive: None,
        region: None,
    };
    let texturing = Texturing {
        materials: doc.scene.materials(),
        pictures: &pictures,
        camera,
        aspect: 160.0 / 90.0,
        cameras: &[],
    };
    render(&RenderScene::textured(&evaluated, &texturing), &camera, &options)
}

/// A grey floor with a cube and a ball, lit by an area light of each shape
/// (a soft rectangle above, a disc from the side, a visible sphere, a
/// visible tube) and a glowing panel, with light bouncing between them.
fn areas() -> Image {
    let (scene, camera, options) = areas_view(96, 54, 32);
    render(&scene, &camera, &options)
}

/// The area-light scene, its camera and options at `width` × `height`.
fn areas_view(width: u32, height: u32, samples: u32) -> (RenderScene, RenderCamera, RenderOptions) {
    let mut doc = Document::new("areas");
    let mut grey = Material::new("Grey");
    (grey.color, grey.roughness) = ([0.6, 0.6, 0.6], 0.7);
    let grey = doc.scene.materials_mut().insert(grey);
    let mut floor = Node::new(
        "Floor",
        ObjectKind::Plane {
            width: 800.0,
            depth: 800.0,
            width_segments: 1,
            depth_segments: 1,
        },
    );
    floor.data.tags = vec![Tag::Material(MaterialTag::new(Some(grey)))];
    doc.scene.insert(floor);
    let mut red = Material::new("Red");
    (red.color, red.roughness) = ([0.7, 0.1, 0.08], 0.35);
    let red = doc.scene.materials_mut().insert(red);
    let mut cube = Node::new(
        "Cube",
        ObjectKind::Cube {
            size: Vec3::splat(100.0),
            segments: [1, 1, 1],
        },
    )
    .with_position(Vec3::new(-80.0, 50.0, 0.0));
    cube.data.tags = vec![Tag::Material(MaterialTag::new(Some(red)))];
    doc.scene.insert(cube);
    let mut gold = Material::new("Gold");
    (gold.color, gold.roughness, gold.metallic) = ([0.9, 0.7, 0.35], 0.25, 1.0);
    let gold = doc.scene.materials_mut().insert(gold);
    let mut ball = Node::new(
        "Ball",
        ObjectKind::Sphere {
            radius: 50.0,
            segments: 32,
        },
    )
    .with_position(Vec3::new(90.0, 50.0, 20.0));
    ball.data.tags = vec![Tag::Material(MaterialTag::new(Some(gold)))];
    doc.scene.insert(ball);
    let area = |shape, size, visible, intensity| ObjectKind::Light {
        color: [1.0, 0.95, 0.9],
        intensity,
        light_type: LightType::Area,
        details: LightDetails {
            area_shape: shape,
            area_size: size,
            area_visible: visible,
            ..LightDetails::default()
        },
        shadow: LightShadow::Area,
        linking: grid4d_scene::Linking::default(),
        group: String::new(),
    };
    let at = |position: Vec3, hpb: Vec3| Transform::from_position(position).with_rotation_deg(hpb);
    for (shape, size, visible, intensity, place) in [
        (
            AreaShape::Rectangle,
            [200.0, 120.0],
            false,
            0.4,
            at(Vec3::new(0.0, 400.0, 100.0), Vec3::new(0.0, -90.0, 0.0)),
        ),
        (
            AreaShape::Disc,
            [80.0, 80.0],
            false,
            0.3,
            at(Vec3::new(-350.0, 120.0, 0.0), Vec3::new(-90.0, 0.0, 0.0)),
        ),
        (
            AreaShape::Sphere,
            [30.0, 30.0],
            true,
            0.05,
            at(Vec3::new(220.0, 60.0, -120.0), Vec3::ZERO),
        ),
        (
            AreaShape::Tube,
            [8.0, 160.0],
            true,
            0.05,
            at(Vec3::new(0.0, 15.0, -200.0), Vec3::new(90.0, 0.0, 0.0)),
        ),
    ] {
        doc.scene
            .insert(Node::new("Area", area(shape, size, visible, intensity)).with_transform(place));
    }
    let mut eval = EvaluatedScene::for_render();
    eval.update(&doc.scene);
    let camera = RenderCamera::framing(Vec3::new(-200.0, 0.0, -200.0), Vec3::new(240.0, 120.0, 120.0), 0.75);
    let options = RenderOptions {
        width,
        height,
        samples,
        bounces: 2,
        seed: 0,
        auto_light: false,
        clamp_indirect: true,
        emissive_lights: true,
        adaptive: None,
        region: None,
    };
    let pictures = Pictures::new();
    let texturing = Texturing {
        materials: doc.scene.materials(),
        pictures: &pictures,
        camera,
        aspect: f32::from(u16::try_from(width).unwrap()) / f32::from(u16::try_from(height).unwrap()),
        cameras: &[],
    };
    (RenderScene::textured(&eval, &texturing), camera, options)
}

/// A grey floor and three shapes; `material(name, colour, roughness,
/// metallic)` adds a material.
fn still_life(doc: &mut Document) {
    let add = |doc: &mut Document, name: &str, color: [f32; 3], roughness: f32, metallic: f32| {
        let mut m = Material::new(name);
        (m.color, m.roughness, m.metallic) = (color, roughness, metallic);
        doc.scene.materials_mut().insert(m)
    };
    let grey = add(doc, "Grey", [0.55, 0.55, 0.55], 0.8, 0.0);
    let red = add(doc, "Red", [0.75, 0.08, 0.06], 0.3, 0.0);
    let chrome = add(doc, "Chrome", [0.95, 0.95, 0.95], 0.08, 1.0);
    let gold = add(doc, "Gold", [0.9, 0.7, 0.35], 0.3, 1.0);
    let tag = |m| vec![MaterialTag::new(Some(m))];
    doc.scene.insert(tagged(
        ObjectKind::Plane {
            width: 1200.0,
            depth: 1200.0,
            width_segments: 1,
            depth_segments: 1,
        },
        Vec3::ZERO,
        tag(grey),
    ));
    doc.scene.insert(tagged(
        ObjectKind::Cube {
            size: Vec3::splat(100.0),
            segments: [1, 1, 1],
        },
        Vec3::new(-110.0, 50.0, 0.0),
        tag(red),
    ));
    doc.scene.insert(tagged(
        ObjectKind::Sphere {
            radius: 55.0,
            segments: 32,
        },
        Vec3::new(20.0, 55.0, 60.0),
        tag(chrome),
    ));
    doc.scene.insert(tagged(
        ObjectKind::Torus {
            ring_radius: 45.0,
            pipe_radius: 15.0,
            ring_segments: 32,
            pipe_segments: 12,
        },
        Vec3::new(130.0, 15.0, -30.0),
        tag(gold),
    ));
}

/// Renders `doc` from a camera above the front left, with `pictures`, at
/// the size and samples of the reference pictures.
fn still_life_render(doc: &Document, pictures: &Pictures) -> Image {
    still_life_at(doc, pictures, 96, 54, 32)
}

/// Renders `doc` from a camera above the front left, with `pictures`.
fn still_life_at(doc: &Document, pictures: &Pictures, width: u32, height: u32, samples: u32) -> Image {
    let (scene, camera, options) = still_life_view(doc, pictures, width, height, samples);
    render(&scene, &camera, &options)
}

/// The still life's render scene, camera and options.
fn still_life_view(
    doc: &Document,
    pictures: &Pictures,
    width: u32,
    height: u32,
    samples: u32,
) -> (RenderScene, RenderCamera, RenderOptions) {
    let mut eval = EvaluatedScene::for_render();
    eval.update(&doc.scene);
    // Low and close: the shapes, the floor's edge far away and the sky.
    let eye = Vec3::new(-260.0, 110.0, -330.0);
    let camera = RenderCamera {
        world: glam::camera::rh::view::look_at_mat4(eye, Vec3::new(10.0, 40.0, 0.0), Vec3::Y).inverse(),
        projection: CameraProjection::Perspective {
            fov_y: 40f32.to_radians(),
        },
        lens: None,
        end: None,
    };
    let options = RenderOptions {
        width,
        height,
        samples,
        bounces: 2,
        seed: 0,
        auto_light: true,
        clamp_indirect: true,
        emissive_lights: true,
        adaptive: None,
        region: None,
    };
    let texturing = Texturing {
        materials: doc.scene.materials(),
        pictures,
        camera,
        aspect: f32::from(u16::try_from(width).unwrap()) / f32::from(u16::try_from(height).unwrap()),
        cameras: &[],
    };
    (RenderScene::textured(&eval, &texturing), camera, options)
}

/// The still life under a clear afternoon Sky, the sun 35° up in the
/// south-west.
fn daylight() -> Image {
    let (doc, pictures) = daylight_scene();
    still_life_render(&doc, &pictures)
}

fn daylight_scene() -> (Document, Pictures) {
    let mut doc = Document::new("daylight");
    still_life(&mut doc);
    doc.scene.insert(Node::new(
        "Sky",
        ObjectKind::Sky(Sky {
            placement: SunPlacement::Angles,
            elevation: 35f32.to_radians(),
            azimuth: 220f32.to_radians(),
            turbidity: 3.0,
            ..Sky::default()
        }),
    ));
    (doc, Pictures::new())
}

/// A made-up studio panorama (128 × 64, half floats): a soft grey dome, a
/// darker floor, and one bright softbox high on one side.
fn studio_panorama() -> Arc<Picture> {
    let (w, h) = (128u32, 64u32);
    let mut values = Vec::with_capacity(usize::try_from(w * h * 4).unwrap());
    for y in 0..h {
        for x in 0..w {
            let v = (f32::from(u16::try_from(y).unwrap()) + 0.5) / 64.0;
            let u = (f32::from(u16::try_from(x).unwrap()) + 0.5) / 128.0;
            let mut rgb = if v < 0.5 {
                [0.35 + 0.25 * v, 0.37 + 0.25 * v, 0.42 + 0.2 * v]
            } else {
                [0.12, 0.11, 0.1]
            };
            if (0.62..0.72).contains(&u) && (0.18..0.32).contains(&v) {
                rgb = [16.0, 15.5, 15.0];
            }
            values.extend([rgb[0], rgb[1], rgb[2], 1.0].map(half::f16::from_f32));
        }
    }
    Arc::new(Picture::new(w, h, Texels::Half(values.into())).unwrap())
}

/// The still life lit by the panorama, turned a little.
fn environment_lit() -> Image {
    let (doc, pictures) = environment_scene();
    still_life_render(&doc, &pictures)
}

fn environment_scene() -> (Document, Pictures) {
    let mut doc = Document::new("studio");
    still_life(&mut doc);
    let mut pictures = Pictures::new();
    let (texture, _) = add(&mut doc, &mut pictures, "studio.exr", studio_panorama());
    doc.scene.insert(Node::new(
        "Environment",
        ObjectKind::Environment(Environment {
            texture: Some(texture),
            intensity: 1.0,
            rotation: 0.6,
            visible: true,
            group: String::new(),
        }),
    ));
    (doc, pictures)
}

/// The manual's picture of the two (`render-sky-environment.png`, before
/// it is reduced to 256 colours), larger and smoother than the references:
///
/// `GRID4D_MANUAL_PICTURES=<folder> cargo test --release -p grid4d-raytrace --test reference -- --ignored`
#[test]
#[ignore = "renders a picture for the manual"]
fn sky_and_environment_for_the_manual() {
    let dir = std::env::var_os("GRID4D_MANUAL_PICTURES").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let (sky, empty) = daylight_scene();
    let (studio, pictures) = environment_scene();
    let (w, h) = (480u32, 270u32);
    let left = still_life_at(&sky, &empty, w, h, 256).to_rgba8();
    let right = still_life_at(&studio, &pictures, w, h, 256).to_rgba8();
    let row = usize::try_from(w * 4).unwrap();
    let mut both = Vec::with_capacity(row * 2 * usize::try_from(h).unwrap());
    for (a, b) in left.chunks(row).zip(right.chunks(row)) {
        both.extend_from_slice(a);
        both.extend_from_slice(b);
    }
    let path = dir.join("render-sky-environment.png");
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(&path).unwrap()), w * 2, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&both).unwrap();
    println!("{}", path.display());
}

/// A row of glowing beads going back from 150 to 900 cm and a grey ball at
/// 400 cm, seen through a lens focused on the ball with six blades: the near
/// and far beads blur into hexagons.
fn depth_of_field_at(width: u32, height: u32, samples: u32) -> Image {
    let mut doc = Document::new("focus");
    let mut glow = Material::new("Glow");
    (glow.color, glow.emission) = ([0.0; 3], [60.0, 48.0, 30.0]);
    let glow = doc.scene.materials_mut().insert(glow);
    for k in 0..8u8 {
        let z = -150.0 - f32::from(k) * 110.0;
        let x = -60.0 + f32::from(k) * 22.0;
        doc.scene.insert(tagged(
            ObjectKind::Sphere {
                radius: 1.2,
                segments: 12,
            },
            Vec3::new(x * 0.6, 5.0 + f32::from(k % 3) * 8.0, z),
            vec![MaterialTag::new(Some(glow))],
        ));
    }
    let mut grey = Material::new("Grey");
    (grey.color, grey.roughness) = ([0.6, 0.6, 0.6], 0.5);
    let grey = doc.scene.materials_mut().insert(grey);
    // A dark wall far behind.
    let mut wall = Material::new("Wall");
    wall.color = [0.04, 0.04, 0.05];
    let wall = doc.scene.materials_mut().insert(wall);
    let mut back = Node::new(
        "Wall",
        ObjectKind::Plane {
            width: 3000.0,
            depth: 3000.0,
            width_segments: 1,
            depth_segments: 1,
        },
    )
    .with_transform(
        Transform::from_position(Vec3::new(0.0, 0.0, -1400.0)).with_rotation_deg(Vec3::new(0.0, 90.0, 0.0)),
    );
    back.data.tags = vec![Tag::Material(MaterialTag::new(Some(wall)))];
    doc.scene.insert(back);
    doc.scene.insert(tagged(
        ObjectKind::Sphere {
            radius: 40.0,
            segments: 32,
        },
        Vec3::new(75.0, -20.0, -400.0),
        vec![MaterialTag::new(Some(grey))],
    ));
    let mut eval = EvaluatedScene::for_render();
    eval.update(&doc.scene);
    let camera = RenderCamera {
        lens: Some(ThinLens {
            radius: 10.0,
            focus_distance: 400.0,
            blades: 6,
            rotation: 0.3,
        }),
        ..RenderCamera::still(
            glam::Mat4::IDENTITY,
            CameraProjection::Perspective {
                fov_y: 30f32.to_radians(),
            },
        )
    };
    let options = RenderOptions {
        width,
        height,
        samples,
        bounces: 1,
        seed: 0,
        auto_light: true,
        clamp_indirect: true,
        emissive_lights: true,
        adaptive: None,
        region: None,
    };
    render(&RenderScene::new(&eval), &camera, &options)
}

/// A wheel (a tyre and four spokes) turning 20° over the shutter, seen from
/// above, lit by the automatic light: the spokes smear into fans.
fn spinning_wheel_at(width: u32, height: u32, samples: u32) -> Image {
    let moment = |turn: f32| {
        let mut doc = Document::new("wheel");
        let mut dark = Material::new("Dark");
        (dark.color, dark.roughness) = ([0.08, 0.08, 0.09], 0.6);
        let dark = doc.scene.materials_mut().insert(dark);
        let mut red = Material::new("Red");
        (red.color, red.roughness) = ([0.75, 0.1, 0.08], 0.4);
        let red = doc.scene.materials_mut().insert(red);
        let mut floor = Material::new("Floor");
        floor.color = [0.7, 0.7, 0.7];
        let floor = doc.scene.materials_mut().insert(floor);
        doc.scene.insert(tagged(
            ObjectKind::Plane {
                width: 600.0,
                depth: 600.0,
                width_segments: 1,
                depth_segments: 1,
            },
            Vec3::ZERO,
            vec![MaterialTag::new(Some(floor))],
        ));
        let mut wheel = Node::new("Wheel", ObjectKind::Null).with_transform(
            Transform::from_position(Vec3::new(0.0, 30.0, 0.0)).with_rotation_deg(Vec3::new(turn, 0.0, 0.0)),
        );
        wheel.data.tags.clear();
        let wheel = doc.scene.insert(wheel);
        let mut tyre = Node::new(
            "Tyre",
            ObjectKind::Torus {
                ring_radius: 110.0,
                pipe_radius: 14.0,
                ring_segments: 48,
                pipe_segments: 12,
            },
        );
        tyre.data.tags = vec![Tag::Material(MaterialTag::new(Some(dark)))];
        doc.scene.insert_child(wheel, tyre).unwrap();
        for k in 0..4u8 {
            let mut spoke = Node::new(
                "Spoke",
                ObjectKind::Cube {
                    size: Vec3::new(200.0, 10.0, 16.0),
                    segments: [1, 1, 1],
                },
            )
            .with_transform(Transform::default().with_rotation_deg(Vec3::new(
                f32::from(k) * 45.0,
                0.0,
                0.0,
            )));
            spoke.data.tags = vec![Tag::Material(MaterialTag::new(Some(red)))];
            doc.scene.insert_child(wheel, spoke).unwrap();
        }
        let mut eval = EvaluatedScene::for_render();
        eval.update(&doc.scene);
        RenderScene::new(&eval)
    };
    let moments: Vec<RenderScene> = (0..5u8).map(|k| moment(f32::from(k) * 5.0)).collect();
    let (scene, still) = RenderScene::moving(moments, 2);
    assert_eq!(still, 0);
    let look = glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 330.0, 190.0), Vec3::new(0.0, 20.0, 0.0), Vec3::Y);
    let camera = RenderCamera::still(look.inverse(), CameraProjection::Perspective { fov_y: 0.75 });
    let options = RenderOptions {
        width,
        height,
        samples,
        bounces: 1,
        seed: 0,
        auto_light: true,
        clamp_indirect: true,
        emissive_lights: true,
        adaptive: None,
        region: None,
    };
    render(&scene, &camera, &options)
}

/// Compares `image` with the picture at `path` (or writes it there when asked).
fn matches_reference(image: &Image, path: &str) {
    let bytes = image.to_rgba8();
    if std::env::var_os("GRID4D_UPDATE_REFERENCE").is_some() {
        let mut encoder = png::Encoder::new(BufWriter::new(File::create(path).unwrap()), image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&bytes).unwrap();
        return;
    }
    let decoder = png::Decoder::new(std::io::BufReader::new(
        File::open(path).expect("the reference picture"),
    ));
    let mut reader = decoder.read_info().unwrap();
    let mut reference = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut reference).unwrap();
    assert_eq!((info.width, info.height), (image.width, image.height));
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let reference = &reference[..info.buffer_size()];

    let differences: Vec<u8> = bytes.iter().zip(reference).map(|(a, b)| a.abs_diff(*b)).collect();
    let mean = differences.iter().map(|&d| f64::from(d)).sum::<f64>() / differences.len() as f64;
    let large = differences.iter().filter(|&&d| d > 32).count();
    assert!(mean <= 2.0, "mean difference {mean:.2} of 255");
    assert!(
        large * 100 <= differences.len(),
        "{large} of {} values differ by more than 32",
        differences.len()
    );
}

#[test]
fn the_demo_scene_matches_its_reference() {
    matches_reference(&demo(), DEMO);
}

#[test]
fn textured_materials_match_their_reference() {
    matches_reference(&textured(), TEXTURED);
}

#[test]
fn area_lights_match_their_reference() {
    matches_reference(&areas(), AREAS);
}

#[test]
fn depth_of_field_matches_its_reference() {
    matches_reference(&depth_of_field_at(96, 54, 64), FOCUS);
}

/// A wax ball beside a matte one of the same colour on a grey floor, lit
/// from behind and low, with a dim overcast sky: the wax glows at its edge
/// and its shadow side is lit from inside.
fn wax_scene() -> Document {
    let mut doc = Document::new("wax");
    let mut grey = Material::new("Grey");
    (grey.color, grey.roughness) = ([0.4, 0.4, 0.4], 0.9);
    let grey = doc.scene.materials_mut().insert(grey);
    let colour = [0.9, 0.7, 0.45];
    let mut wax = Material::new("Wax");
    (wax.color, wax.roughness, wax.subsurface) = (colour, 0.4, 1.0);
    wax.subsurface_radius = [3.0, 1.5, 0.8];
    wax.subsurface_scale = 2.0;
    let wax = doc.scene.materials_mut().insert(wax);
    let mut matte = Material::new("Matte");
    (matte.color, matte.roughness) = (colour, 0.4);
    let matte = doc.scene.materials_mut().insert(matte);
    doc.scene.insert(tagged(
        ObjectKind::Plane {
            width: 1200.0,
            depth: 1200.0,
            width_segments: 1,
            depth_segments: 1,
        },
        Vec3::ZERO,
        vec![MaterialTag::new(Some(grey))],
    ));
    let ball = ObjectKind::Sphere {
        radius: 45.0,
        segments: 48,
    };
    doc.scene.insert(tagged(
        ball.clone(),
        Vec3::new(-80.0, 45.0, 20.0),
        vec![MaterialTag::new(Some(wax))],
    ));
    doc.scene.insert(tagged(
        ball,
        Vec3::new(60.0, 45.0, 60.0),
        vec![MaterialTag::new(Some(matte))],
    ));
    // Behind the balls, low, shining towards the camera.
    let place =
        glam::camera::rh::view::look_at_mat4(Vec3::new(300.0, 120.0, 500.0), Vec3::new(0.0, 40.0, 0.0), Vec3::Y)
            .inverse();
    doc.scene.insert(
        Node::new(
            "Back Light",
            ObjectKind::Light {
                color: [1.0, 0.95, 0.9],
                intensity: 3.0,
                light_type: LightType::Infinite,
                details: LightDetails::default(),
                shadow: LightShadow::Raytraced,
                linking: grid4d_scene::Linking::default(),
                group: String::new(),
            },
        )
        .with_transform(Transform::from_matrix(&place)),
    );
    doc.scene.insert(Node::new(
        "Sky",
        ObjectKind::Sky(Sky {
            sun: false,
            intensity: 0.25,
            ..Sky::default()
        }),
    ));
    doc
}

#[test]
fn subsurface_scattering_matches_its_reference() {
    matches_reference(&still_life_at(&wax_scene(), &Pictures::new(), 96, 54, 64), WAX);
}

/// The manual's picture of subsurface scattering (`render-subsurface.png`,
/// before it is reduced to 256 colours):
///
/// `GRID4D_MANUAL_PICTURES=<folder> cargo test --release -p grid4d-raytrace --test reference -- --ignored subsurface_for`
#[test]
#[ignore = "renders a picture for the manual"]
fn subsurface_for_the_manual() {
    let dir = std::env::var_os("GRID4D_MANUAL_PICTURES").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    // As the editor renders it for new documents: denoised.
    let doc = wax_scene();
    let (scene, camera, options) = still_life_view(&doc, &Pictures::new(), 640, 360, 256);
    let mut progressive = grid4d_raytrace::Progressive::new(&options).with_denoising();
    progressive.add_samples(
        &scene,
        &camera,
        &options,
        u32::MAX,
        &grid4d_raytrace::RenderControl::default(),
    );
    let mut image = progressive.image();
    grid4d_raytrace::denoise(&mut image, 1.0);
    let path = dir.join("render-subsurface.png");
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(&path).unwrap()), 640, 360);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&image.to_rgb8())
        .unwrap();
    println!("{}", path.display());
}

#[test]
fn a_spinning_wheel_matches_its_reference() {
    matches_reference(&spinning_wheel_at(96, 54, 32), WHEEL);
}

#[test]
fn a_daylight_scene_matches_its_reference() {
    matches_reference(&daylight(), DAYLIGHT);
}

#[test]
fn an_environment_lit_product_matches_its_reference() {
    matches_reference(&environment_lit(), PRODUCT);
}

/// The manual's picture of depth of field and motion blur
/// (`render-lens.png`, before it is reduced to 256 colours):
///
/// `GRID4D_MANUAL_PICTURES=<folder> cargo test --release -p grid4d-raytrace --test reference -- --ignored lens`
#[test]
#[ignore = "renders a picture for the manual"]
fn lens_and_shutter_for_the_manual() {
    let dir = std::env::var_os("GRID4D_MANUAL_PICTURES").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let (w, h) = (480u32, 270u32);
    let left = depth_of_field_at(w, h, 256).to_rgba8();
    let right = spinning_wheel_at(w, h, 128).to_rgba8();
    let row = usize::try_from(w * 4).unwrap();
    let mut both = Vec::with_capacity(row * 2 * usize::try_from(h).unwrap());
    for (a, b) in left.chunks(row).zip(right.chunks(row)) {
        both.extend_from_slice(a);
        both.extend_from_slice(b);
    }
    let path = dir.join("render-lens.png");
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(&path).unwrap()), w * 2, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&both).unwrap();
    println!("{}", path.display());
}

/// The manual's picture of render passes (`render-passes.png`, before it is
/// reduced to 256 colours): the daylight still life and seven of its passes.
///
/// `GRID4D_MANUAL_PICTURES=<folder> cargo test --release -p grid4d-raytrace --test reference -- --ignored passes`
#[test]
#[ignore = "renders a picture for the manual"]
fn passes_for_the_manual() {
    use grid4d_raytrace::{Progressive, RenderControl};
    use grid4d_scene::Pass;
    let dir = std::env::var_os("GRID4D_MANUAL_PICTURES").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let (w, h) = (240u32, 135u32);
    let (doc, pictures) = daylight_scene();
    let (scene, camera, options) = still_life_view(&doc, &pictures, w, h, 128);
    let shown = [
        Pass::DiffuseDirect,
        Pass::SpecularIndirect,
        Pass::Background,
        Pass::Albedo,
        Pass::Depth,
        Pass::Normal,
        Pass::ObjectId,
    ];
    let mut progressive = Progressive::with_passes(&options, shown.into_iter().collect(), &scene);
    progressive.add_samples(&scene, &camera, &options, u32::MAX, &RenderControl::default());
    let image = progressive.image();
    let mut tiles = vec![image.to_rgb8()];
    for pass in &shown {
        tiles.push(image.passes.iter().find(|l| l.name == pass.name()).unwrap().to_rgb8());
    }
    // Four across, two down, a 4-pixel gap.
    let gap = 4usize;
    let (tw, th) = (usize::try_from(w).unwrap(), usize::try_from(h).unwrap());
    let (width, height) = (tw * 4 + gap * 3, th * 2 + gap);
    let mut out = vec![255u8; width * height * 3];
    for (n, tile) in tiles.iter().enumerate() {
        let (x0, y0) = ((n % 4) * (tw + gap), (n / 4) * (th + gap));
        for y in 0..th {
            let row = &tile[y * tw * 3..(y + 1) * tw * 3];
            let start = ((y0 + y) * width + x0) * 3;
            out[start..start + tw * 3].copy_from_slice(row);
        }
    }
    let path = dir.join("render-passes.png");
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(&path).unwrap()),
        u32::try_from(width).unwrap(),
        u32::try_from(height).unwrap(),
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&out).unwrap();
    println!("{}", path.display());
}

/// The manual's picture of adaptive sampling (`render-adaptive.png`, before
/// it is reduced to 256 colours): the daylight still life with adaptive
/// sampling, and where its samples went.
///
/// `GRID4D_MANUAL_PICTURES=<folder> cargo test --release -p grid4d-raytrace --test reference -- --ignored adaptive`
#[test]
#[ignore = "renders a picture for the manual"]
fn adaptive_sampling_for_the_manual() {
    use grid4d_raytrace::{Adaptive, PassLayer};
    let dir = std::env::var_os("GRID4D_MANUAL_PICTURES").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let (w, h) = (360u32, 203u32);
    let (doc, pictures) = daylight_scene();
    let (scene, camera, options) = still_life_view(&doc, &pictures, w, h, 512);
    let options = RenderOptions {
        adaptive: Some(Adaptive {
            min_samples: 16,
            threshold: 0.03,
        }),
        ..options
    };
    let image = render(&scene, &camera, &options);
    let left = image.to_rgb8();
    let right = PassLayer::samples(&image.sample_counts).to_rgb8();
    let row = usize::try_from(w * 3).unwrap();
    let gap = 4usize;
    let width = row * 2 + gap * 3;
    let mut both = Vec::new();
    for (a, b) in left.chunks(row).zip(right.chunks(row)) {
        both.extend_from_slice(a);
        both.extend(std::iter::repeat_n(255u8, gap * 3));
        both.extend_from_slice(b);
    }
    let path = dir.join("render-adaptive.png");
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(&path).unwrap()),
        u32::try_from(width / 3).unwrap(),
        h,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&both).unwrap();
    println!("{}", path.display());
}

/// The manual's picture of the denoiser (`render-denoise.png`, before it is
/// reduced to 256 colours): the area-light scene at 64 samples, denoised,
/// and at 2048.
///
/// `GRID4D_MANUAL_PICTURES=<folder> cargo test --release -p grid4d-raytrace --test reference -- --ignored denoiser`
#[test]
#[ignore = "renders a picture for the manual"]
fn denoiser_for_the_manual() {
    use grid4d_raytrace::{Progressive, RenderControl, denoise};
    let dir = std::env::var_os("GRID4D_MANUAL_PICTURES").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let (w, h) = (320u32, 180u32);
    let (scene, camera, options) = areas_view(w, h, 64);
    let mut progressive = Progressive::new(&options).with_denoising();
    progressive.add_samples(&scene, &camera, &options, u32::MAX, &RenderControl::default());
    let noisy = progressive.image();
    let mut clean = noisy.clone();
    assert!(denoise(&mut clean, 1.0));
    let reference = render(
        &scene,
        &camera,
        &RenderOptions {
            samples: 2048,
            ..options
        },
    );
    let tiles = [noisy.to_rgb8(), clean.to_rgb8(), reference.to_rgb8()];
    let row = usize::try_from(w * 3).unwrap();
    let gap = [255u8; 12];
    let mut out = Vec::new();
    for y in 0..usize::try_from(h).unwrap() {
        for (i, tile) in tiles.iter().enumerate() {
            if i > 0 {
                out.extend_from_slice(&gap);
            }
            out.extend_from_slice(&tile[y * row..(y + 1) * row]);
        }
    }
    let path = dir.join("render-denoise.png");
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(&path).unwrap()), w * 3 + 8, h);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&out).unwrap();
    println!("{}", path.display());
}
