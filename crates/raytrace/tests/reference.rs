//! Reference-image tests: small renders compared with committed pictures
//! within a tolerance (floating-point details differ a little between
//! machines) — the demo scene through its camera, and a scene of textured
//! materials (pictures through UV and Box projections, a layer cut out
//! by its Opacity, a bump map, glass and a clear coat), and a scene lit by an
//! area light of each shape.
//!
//! After an intended change to the renderer, write new references with
//! `GRID4D_UPDATE_REFERENCE=1 cargo test -p grid4d-raytrace --test reference`
//! and look at them before committing.

#![allow(clippy::unwrap_used, reason = "test code")]

use std::{fs::File, io::BufWriter, sync::Arc};

use glam::Vec3;
use grid4d_eval::EvaluatedScene;
use grid4d_raytrace::{
    CameraProjection, Image, Picture, Pictures, RenderCamera, RenderOptions, RenderScene, Texels, Texturing, render,
};
use grid4d_scene::{
    AreaShape, Channel, Document, LightDetails, LightShadow, LightType, Material, MaterialTag, Node, ObjectKind,
    Projection, Tag, Texture, TextureId, TextureSlot, Transform,
};

const DEMO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/demo_reference.png");
const TEXTURED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/textured_reference.png");
const AREAS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/area_reference.png");

fn demo() -> Image {
    let doc = Document::with_demo_scene();
    let mut evaluated = EvaluatedScene::for_render();
    evaluated.update(&doc.scene);
    let (id, fov) = doc
        .scene
        .iter()
        .find_map(|(id, node)| match node.data.kind {
            ObjectKind::Camera { fov_y_deg } => Some((id, fov_y_deg)),
            _ => None,
        })
        .expect("the demo scene has a camera");
    let camera = RenderCamera {
        world: doc.scene.world_matrix(id),
        projection: CameraProjection::Perspective {
            fov_y: fov.to_radians(),
        },
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
        width: 96,
        height: 54,
        samples: 32,
        bounces: 2,
        seed: 0,
        auto_light: false,
        clamp_indirect: true,
        emissive_lights: true,
    };
    let pictures = Pictures::new();
    let texturing = Texturing {
        materials: doc.scene.materials(),
        pictures: &pictures,
        camera,
        aspect: 96.0 / 54.0,
        cameras: &[],
    };
    render(&RenderScene::textured(&eval, &texturing), &camera, &options)
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
