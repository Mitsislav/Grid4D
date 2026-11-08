//! File-level tests: the golden fixture pins the on-disk format, older fixtures
//! must keep loading, and save/load go through the real filesystem.
//!
//! Version 5 is current and grows (additively) until Grid4D 1.2 ships: its
//! golden files (`demo_v5.g4d`, `full_v5.g4d`) are written again with
//! `GRID4D_UPDATE_FIXTURES=1 cargo test -p grid4d-io --test format` when a
//! step adds data, and frozen with the release. Older fixtures
//! (`demo_v1.oc4d`, `demo_v2.g4d`, the `_v3` files frozen with 1.0, the `_v4`
//! files frozen with 1.1) are never regenerated.
//!
//! The golden archive is compared entry by entry, decompressed: the compressed
//! bytes may legitimately change with the deflate library.

#![allow(clippy::unwrap_used, reason = "test code")]

use std::{
    fs,
    io::{Cursor, Read as _},
    path::{Path, PathBuf},
};

use glam::{Quat, Vec3};
use grid4d_scene::{
    Axis, Body, CachedBody, ComponentState, Components, Curve, Document, Interpolation, Key, Layer, Material,
    MeshHandle, Node, ObjectKind, ObjectKindTag, ObjectParam, Override, Param, PluginObject, PluginValue, SceneEdit,
    SimulationCache, Tag, Take, Tangents, Track,
};
use grid4d_scene::{Channel, ColorSpace, Texture, TextureSlot, TextureSource};

/// Written by the current format version (an archive).
const GOLDEN: &str = "tests/fixtures/demo_v5.g4d";
/// Everything the current version can hold (see `full_document`).
const FULL: &str = "tests/fixtures/full_v5.g4d";
/// `golden_document` as Grid4D 1.1 wrote it (format version 4, frozen).
const DEMO_V4: &str = "tests/fixtures/demo_v4.g4d";
/// Everything version 4 can hold (see `full_v4_document`), frozen with 1.1.
const FULL_V4: &str = "tests/fixtures/full_v4.g4d";
/// `golden_document` as Grid4D 1.0 wrote it (format version 3, frozen).
const DEMO_V3: &str = "tests/fixtures/demo_v3.g4d";
/// Everything version 3 can hold (see `full_v3_document`), frozen with 1.0.
const FULL_V3: &str = "tests/fixtures/full_v3.g4d";
/// Version 3 as first released in step 3.6, before component selections were
/// added (additively): must keep loading.
const DEMO_V3_INITIAL: &str = "tests/fixtures/demo_v3_initial.g4d";
/// Written by format version 2 (JSON, before polygon objects); kept to test migration.
const DEMO_V2: &str = "tests/fixtures/demo_v2.g4d";
/// Written by format version 1 (before segment counts); kept to test migration.
const DEMO_V1: &str = "tests/fixtures/demo_v1.oc4d";

fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// A fresh, empty directory under the system temp dir, unique to this test.
fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grid4d-io-{}-{test}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn outline(doc: &Document) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    doc.scene.walk(&mut Vec::new(), |_, node, _, depth| {
        out.push((depth, node.data.name.clone()));
        true
    });
    out
}

const DEMO_OUTLINE: [(u32, &str); 7] = [
    (0, "Cube"),
    (0, "Sphere"),
    (0, "Group"),
    (1, "Cylinder"),
    (1, "Box"),
    (0, "Light"),
    (0, "Camera"),
];

/// The demo scene with its "Box" made editable — every kind of object, a mesh
/// with UVs — and some of its components selected and hidden.
fn golden_document() -> Document {
    golden_with(grid4d_geom::primitives::cube(Vec3::splat(80.0), [1, 2, 1]))
}

/// `golden_document` as Grid4D 1.0 made it: its cube had no UVs.
fn golden_v3_document() -> Document {
    golden_with(without_uvs(grid4d_geom::primitives::cube(Vec3::splat(80.0), [1, 2, 1])))
}

fn without_uvs(mut mesh: grid4d_geom::PolyMesh) -> grid4d_geom::PolyMesh {
    mesh.set_uvs(None).unwrap();
    mesh
}

fn golden_with(mesh: grid4d_geom::PolyMesh) -> Document {
    let mut doc = Document::with_demo_scene();
    let id = doc.scene.iter().find(|(_, n)| n.data.name == "Box").unwrap().0;
    let mut state = ComponentState::default();
    state.selected.points.extend([0, 1, 2]);
    state.selected.edges.insert_range(0..4);
    state.selected.polygons.insert(1);
    state.hidden.polygons.insert(5);
    doc.scene.get_mut(id).unwrap().data.kind = ObjectKind::Polygon {
        mesh: MeshHandle::new(mesh),
        components: Components::new(state),
    };
    doc
}

/// Everything a version 5 file can hold (see `docs/FILE_FORMAT.md`): version
/// 4's, and what Phase 16 adds as it arrives — the light-transport switches
/// off and an area light with every area field set (16.2).
/// The kinds of object version 5 adds ([`full_document`] has them).
const VERSION_5_KINDS: [ObjectKindTag; 2] = [ObjectKindTag::Sky, ObjectKindTag::Environment];

fn full_document() -> (Document, SimulationCache) {
    let (mut doc, cache) = full_v4_document();
    doc.render.clamp_indirect = false;
    doc.render.emissive_lights = false;
    doc.scene.insert(Node::new(
        "Area Light",
        ObjectKind::Light {
            color: [1.0, 0.9, 0.8],
            intensity: 2.5,
            light_type: grid4d_scene::LightType::Area,
            details: grid4d_scene::LightDetails {
                area_shape: grid4d_scene::AreaShape::Disc,
                area_size: [80.0, 40.0],
                area_visible: true,
                two_sided: true,
                ..grid4d_scene::LightDetails::default()
            },
            shadow: grid4d_scene::LightShadow::Area,
            // Step 16.4: it lights only the cube and the sphere, in a group.
            linking: grid4d_scene::Linking {
                mode: grid4d_scene::LinkMode::Include,
                objects: ["Cube", "Sphere"]
                    .map(|name| doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0)
                    .to_vec(),
            },
            group: "Key".to_owned(),
        },
    ));
    // Step 16.3: a Sky with its sun placed by angles (its elevation keyed)
    // and an Environment showing the embedded texture.
    let mut sky = Node::new(
        "Sky",
        ObjectKind::Sky(grid4d_scene::Sky {
            placement: grid4d_scene::SunPlacement::Angles,
            elevation: 0.5,
            azimuth: 2.0,
            turbidity: 4.5,
            intensity: 1.5,
            sun: true,
            sun_size: 3.0,
            sun_intensity: 0.75,
            visible: false,
            group: "Daylight".to_owned(),
        }),
    );
    sky.data.tracks.push(Track {
        param: Param::Object(ObjectParam::Elevation),
        curve: Curve::from_keys([Key::new(0.0, 0.5), Key::new(10.0, 0.25)]),
    });
    doc.scene.insert(sky);
    let checker = doc.scene.textures().ids()[1];
    doc.scene.insert(Node::new(
        "Environment",
        ObjectKind::Environment(grid4d_scene::Environment {
            texture: Some(checker),
            intensity: 2.0,
            rotation: -1.0,
            visible: true,
            group: String::new(),
        }),
    ));
    (doc, cache)
}

/// Everything a version 4 file can hold, which 1.1 froze (`full_v4.g4d`; never
/// change it): version 3's, and what Phase 15 added — its mesh has UVs (15.2);
/// a linked and an embedded texture (15.3); material channels and Material
/// tags' projections (15.4); seams (15.7b); a texture's picture channel (15.8).
fn full_v4_document() -> (Document, SimulationCache) {
    let (mut doc, cache) = full_v3_document();
    let textures = doc.scene.textures_mut();
    textures.insert(Texture::linked(PathBuf::from("maps/wood.jpg")));
    let pixels = [255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
    let png = grid4d_io::png_bytes(2, 2, grid4d_io::Channels::Rgba, &pixels).unwrap();
    textures.insert(Texture {
        name: "Checker".to_owned(),
        source: TextureSource::Embedded {
            file_name: "checker.png".to_owned(),
            bytes: png.into(),
        },
        color_space: ColorSpace::Linear,
    });
    let id = doc.scene.iter().find(|(_, n)| n.data.name == "Mesh").unwrap().0;
    let node = doc.scene.get_mut(id).unwrap();
    let ObjectKind::Polygon { mesh, components } = &mut node.data.kind else {
        unreachable!("the Mesh is a polygon object")
    };
    *mesh = MeshHandle::new(grid4d_geom::primitives::cube(Vec3::splat(80.0), [1, 2, 1]));
    // UV seams around one side (15.7b).
    components.make_mut().seams.insert_range(4..8);

    // A material with every channel away from its default and textures on
    // two channels (15.4).
    let (wood, checker) = {
        let ids = doc.scene.textures().ids();
        (ids[0], ids[1])
    };
    let mut glass = Material::new("Glass");
    glass.specular = 0.7;
    glass.normal_strength = 1.5;
    glass.bump_height = 0.25;
    glass.emission = [0.1, 0.2, 0.3];
    glass.emission_strength = 4.0;
    glass.opacity = 0.9;
    glass.opacity_threshold = 0.25;
    glass.transmission = 1.0;
    glass.ior = 1.33;
    glass.clearcoat = 0.5;
    glass.clearcoat_roughness = 0.1;
    glass.two_sided = true;
    *glass.map_mut(Channel::BaseColor) = Some(TextureSlot::new(wood));
    *glass.map_mut(Channel::Roughness) = Some(TextureSlot {
        tiling: [4.0, 2.0],
        offset: [0.5, 0.25],
        range: [0.2, 0.8],
        strength: 0.75,
        // Its green channel, as glTF packs roughness (15.8).
        source: grid4d_scene::PictureChannel::Green,
        ..TextureSlot::new(checker)
    });
    doc.scene.materials_mut().insert(glass);

    // A second Material tag over the first, every projection field set (15.4).
    let red = doc.scene.materials().ids()[0];
    let sphere = doc.scene.iter().find(|(_, n)| n.data.name == "Sphere").unwrap().0;
    let tagged = doc
        .scene
        .iter()
        .find(|(_, n)| n.data.tags.iter().any(|t| matches!(t, Tag::Material(_))))
        .unwrap()
        .0;
    let tags = &mut doc.scene.get_mut(tagged).unwrap().data.tags;
    let at = tags.iter().position(|t| matches!(t, Tag::Material(_))).unwrap() + 1;
    tags.insert(
        at,
        Tag::Material(grid4d_scene::MaterialTag {
            material: Some(red),
            projection: grid4d_scene::Projection::Box,
            offset: [0.25, -0.5],
            tiling: [2.0, 3.0],
            rotation: 0.5,
            placement: Some(grid4d_scene::Placement {
                position: Vec3::new(1.0, 2.0, 3.0),
                rotation: Vec3::new(0.25, 0.0, 0.0),
                size: Vec3::new(4.0, 5.0, 6.0),
            }),
            camera: Some(sphere),
        }),
    );
    (doc, cache)
}

/// Everything a version 3 file can hold, which 1.0 froze (`full_v3.g4d`; never
/// change it): every kind of object at its defaults, a mesh with
/// selected and hidden components, a plugin object with every kind of value
/// (no plugin needed), a hierarchy, every tag, tracks (transform, an object
/// parameter, a plugin value; every interpolation and manual tangents),
/// materials, layers, takes (one active with Main's values beside it, one
/// with its own overrides), changed render and simulation settings, and a
/// saved simulation with a clone.
fn full_v3_document() -> (Document, SimulationCache) {
    let mut doc = Document::new("Full");
    let mut red = Material::new("Red");
    red.color = [0.8, 0.1, 0.1];
    red.roughness = 0.25;
    red.metallic = 1.0;
    red.emission = [0.5, 0.0, 0.0];
    let red = doc.scene.materials_mut().insert(red);
    let mut props = Layer::new("Props", Layer::COLORS[2]);
    props.switches.locked = true;
    props.switches.render = false;
    let props = doc.scene.layers_mut().insert(props);

    // Every kind version 3 has, at its defaults, inside a group.
    let group = doc.scene.insert(Node::new("Everything", ObjectKind::Null));
    for tag in ObjectKindTag::ALL.into_iter().filter(|t| !VERSION_5_KINDS.contains(t)) {
        doc.scene
            .insert_child(group, Node::new(tag.label(), tag.default_kind()))
            .unwrap();
    }
    let id = |doc: &Document, name: &str| doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0;

    // A mesh with flagged components.
    let mut state = ComponentState::default();
    state.selected.points.extend([0, 1, 2]);
    state.selected.edges.insert_range(0..4);
    state.hidden.polygons.insert(5);
    doc.scene.insert(Node::new(
        "Mesh",
        ObjectKind::Polygon {
            mesh: MeshHandle::new(without_uvs(grid4d_geom::primitives::cube(Vec3::splat(80.0), [1, 2, 1]))),
            components: Components::new(state),
        },
    ));

    // A plugin object with every kind of value.
    let mut gear = PluginObject::new("example.gear");
    gear.set("Radius", PluginValue::Float(120.0));
    gear.set("Teeth", PluginValue::Int(16));
    gear.set("Hole", PluginValue::Bool(true));
    gear.set("Shape", PluginValue::Choice(2));
    gear.set("Axis", PluginValue::Vector(Vec3::new(0.0, 1.0, 0.0)));
    gear.set("Tint", PluginValue::Color([0.2, 0.4, 0.6]));
    let mut gear = Node::new("Gear", ObjectKind::Plugin(gear));
    gear.data.tracks.push(Track {
        param: Param::Object(ObjectParam::Plugin(0)),
        curve: Curve::from_keys([Key::new(0.0, 120.0), Key::new(30.0, 60.0)]),
    });
    doc.scene.insert(gear);

    // A cube with every tag, on a layer, animated.
    let (cube, sphere) = (id(&doc, "Cube"), id(&doc, "Sphere"));
    let mut data = doc.scene.get(cube).unwrap().data.clone();
    data.layer = Some(props);
    data.transform.rotation = Vec3::new(0.5, 0.25, 0.0);
    data.tags = vec![
        Tag::Phong {
            angle_limit: true,
            angle: 0.6,
        },
        Tag::Protection {
            position: true,
            scale: false,
            rotation: true,
        },
        Tag::Target {
            target: Some(sphere),
            up: None,
            pitch: true,
        },
        Tag::Material(grid4d_scene::MaterialTag::new(Some(red))),
        Tag::Dynamics(Body::rigid()),
    ];
    data.tracks = vec![
        Track {
            param: Param::Position(Axis::X),
            curve: Curve::from_keys([
                Key {
                    interpolation: Interpolation::Linear,
                    ..Key::new(0.0, 0.0)
                },
                Key {
                    tangents: Tangents::Manual {
                        left: (4.0, -10.0),
                        right: (6.0, 15.0),
                    },
                    ..Key::new(12.5, 100.0)
                },
                Key {
                    interpolation: Interpolation::Step,
                    ..Key::new(24.0, 50.0)
                },
            ]),
        },
        Track {
            param: Param::Object(ObjectParam::Size(Axis::Y)),
            curve: Curve::from_keys([Key::new(0.0, 200.0), Key::new(10.0, 300.0)]),
        },
    ];
    doc.scene.get_mut(cube).unwrap().data = data;

    // Settings away from their defaults.
    doc.render.width = 640;
    doc.render.height = 360;
    doc.render.samples = 16;
    doc.simulation.enabled = true;
    doc.simulation.gravity = Vec3::new(0.0, -490.0, 0.0);

    // Takes: "Night" active (the cube's colour and size overridden), "Day"
    // with an override of its own.
    let night = doc.takes.insert(Take::new("Night"));
    let day = doc.takes.insert(Take::new("Day"));
    doc.takes.render_take = Some(day);
    doc.apply(SceneEdit::SwitchTake(Some(night))).unwrap();
    let mut data = doc.scene.get(cube).unwrap().data.clone();
    data.display_color = [0.1, 0.1, 0.4];
    data.kind = ObjectKind::Cube {
        size: Vec3::splat(150.0),
        segments: [2, 2, 2],
    };
    let inverse = doc.apply(SceneEdit::SetData { id: cube, data }).unwrap();
    doc.record_in_take(&[inverse]).unwrap();
    let mut moved = doc.scene.get(sphere).unwrap().data.transform;
    moved.position.y = 75.0;
    doc.takes
        .get_mut(day)
        .unwrap()
        .objects
        .insert(sphere, vec![Override::Transform(moved)]);

    // A saved simulation: the cube and a Cloner's clone, two frames.
    let cloner = id(&doc, "Cloner");
    let cache = SimulationCache {
        start: 0,
        bodies: vec![
            CachedBody {
                node: cube,
                clone: None,
                scale: Vec3::ONE,
            },
            CachedBody {
                node: cloner,
                clone: Some(2),
                scale: Vec3::new(1.0, 2.0, 1.0),
            },
        ],
        poses: (0..4)
            .map(|i| {
                let i = f32::from(u8::try_from(i).unwrap());
                (Vec3::new(i, 10.0 - i, 0.0), Quat::from_rotation_y(i * 0.25))
            })
            .collect(),
    };
    (doc, cache)
}

/// An archive's entries, in order, decompressed.
fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut entry = archive.by_index(i).unwrap();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            (entry.name().unwrap().into_owned(), data)
        })
        .collect()
}

#[test]
fn the_golden_document_matches_the_golden_file() {
    let bytes = grid4d_io::to_bytes(&golden_document()).unwrap();
    let path = fixture_path(GOLDEN);
    if std::env::var_os("GRID4D_UPDATE_FIXTURES").is_some() {
        fs::write(&path, &bytes).unwrap();
    }
    let (written, golden) = (entries(&bytes), entries(&fs::read(&path).unwrap()));
    let names = |e: &[(String, Vec<u8>)]| e.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    assert_eq!(names(&written), names(&golden));
    // The JSON as text, so a failure shows a readable diff.
    let text = |e: &[(String, Vec<u8>)]| String::from_utf8(e[0].1.clone()).unwrap();
    assert_eq!(
        text(&written),
        text(&golden),
        "the current file format changed; see the note at the top of this file"
    );
    assert_eq!(written, golden, "a mesh file changed");
}

/// Compares an archive with a fixture entry by entry (the JSON as text, for a
/// readable diff).
fn assert_same_entries(bytes: &[u8], fixture: &[u8]) {
    let (written, golden) = (entries(bytes), entries(fixture));
    let names = |e: &[(String, Vec<u8>)]| e.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    assert_eq!(names(&written), names(&golden));
    let text = |e: &[(String, Vec<u8>)]| String::from_utf8(e[0].1.clone()).unwrap();
    assert_eq!(
        text(&written),
        text(&golden),
        "the current file format changed; see the note at the top of this file"
    );
    for ((name, a), (_, b)) in written.iter().zip(&golden).skip(1) {
        assert!(a == b, "{name} changed");
    }
}

#[test]
fn everything_version_5_holds_matches_its_golden_file_and_round_trips() {
    let (doc, cache) = full_document();
    let bytes = grid4d_io::to_bytes_with_cache(&doc, Some(&cache)).unwrap();
    let path = fixture_path(FULL);
    if std::env::var_os("GRID4D_UPDATE_FIXTURES").is_some() {
        fs::write(&path, &bytes).unwrap();
    }
    let fixture = fs::read(&path).unwrap();
    assert_same_entries(&bytes, &fixture);
    let json = String::from_utf8(entries(&fixture)[0].1.clone()).unwrap();
    for kind in ObjectKindTag::ALL {
        let name = format!("\"name\": \"{}\"", kind.label());
        assert!(json.contains(&name), "{name} is in the fixture");
    }

    // Loaded and saved again: the same file, so nothing was lost.
    let (loaded, back) = grid4d_io::from_bytes_with_cache(&fixture).unwrap();
    assert_eq!(loaded.scene.len(), doc.scene.len());
    assert_eq!(loaded.takes.ids().len(), 2);
    let again = grid4d_io::to_bytes_with_cache(&loaded, back.as_ref()).unwrap();
    assert_same_entries(&again, &fixture);
}

#[test]
fn the_golden_file_loads() {
    let doc = grid4d_io::load(&fixture_path(GOLDEN)).unwrap();
    assert_eq!(outline(&doc), DEMO_OUTLINE.map(|(d, n)| (d, n.to_owned())));
    assert_eq!(doc.time.fps(), 30);
    let reference = golden_document();
    let kind = |doc: &Document| {
        let node = doc.scene.iter().find(|(_, n)| n.data.name == "Box").unwrap().1;
        node.data.kind.clone()
    };
    assert_eq!(kind(&doc), kind(&reference), "the mesh, point for point");
}

#[test]
fn everything_version_4_held_still_loads_the_same() {
    // Read from the frozen file and saved: exactly what the current version
    // writes for the same document, so version 4's meaning has not moved.
    // From bytes, without a folder: the linked texture's path stays relative.
    let fixture = fs::read(fixture_path(FULL_V4)).unwrap();
    let (loaded, cache) = grid4d_io::from_bytes_with_cache(&fixture).unwrap();
    let (doc, expected_cache) = full_v4_document();
    assert_same_entries(
        &grid4d_io::to_bytes_with_cache(&loaded, cache.as_ref()).unwrap(),
        &grid4d_io::to_bytes_with_cache(&doc, Some(&expected_cache)).unwrap(),
    );
    let json = String::from_utf8(entries(&fixture)[0].1.clone()).unwrap();
    assert!(json.contains("\"version\": 4"), "the frozen file is version 4");
}

#[test]
fn the_version_4_golden_file_still_loads_the_same() {
    let doc = grid4d_io::load(&fixture_path(DEMO_V4)).unwrap();
    assert_eq!(outline(&doc), DEMO_OUTLINE.map(|(d, n)| (d, n.to_owned())));
    assert_same_entries(
        &grid4d_io::to_bytes(&doc).unwrap(),
        &grid4d_io::to_bytes(&golden_document()).unwrap(),
    );
}

#[test]
fn everything_version_3_held_still_loads_the_same() {
    // Read from the frozen file and saved: exactly what the current version
    // writes for the same document, so version 3's meaning has not moved.
    let (loaded, cache) = grid4d_io::load_with_cache(&fixture_path(FULL_V3)).unwrap();
    let (doc, expected_cache) = full_v3_document();
    assert_same_entries(
        &grid4d_io::to_bytes_with_cache(&loaded, cache.as_ref()).unwrap(),
        &grid4d_io::to_bytes_with_cache(&doc, Some(&expected_cache)).unwrap(),
    );
}

#[test]
fn the_version_3_golden_file_still_loads_the_same() {
    let doc = grid4d_io::load(&fixture_path(DEMO_V3)).unwrap();
    assert_eq!(outline(&doc), DEMO_OUTLINE.map(|(d, n)| (d, n.to_owned())));
    assert_same_entries(
        &grid4d_io::to_bytes(&doc).unwrap(),
        &grid4d_io::to_bytes(&golden_v3_document()).unwrap(),
    );
}

#[test]
fn the_first_version_3_fixture_still_loads() {
    let doc = grid4d_io::load(&fixture_path(DEMO_V3_INITIAL)).unwrap();
    assert_eq!(outline(&doc), DEMO_OUTLINE.map(|(d, n)| (d, n.to_owned())));
    let node = doc.scene.iter().find(|(_, n)| n.data.name == "Box").unwrap().1;
    let ObjectKind::Polygon { mesh, components } = &node.data.kind else {
        panic!("the Box is a polygon object");
    };
    assert_eq!(mesh.mesh().point_count(), 12);
    assert_eq!(*components, Components::default(), "nothing selected before 4.1");
}

#[test]
fn the_version_2_fixture_still_loads() {
    let doc = grid4d_io::load(&fixture_path(DEMO_V2)).unwrap();
    assert_eq!(outline(&doc), DEMO_OUTLINE.map(|(d, n)| (d, n.to_owned())));
    // Migrating then saving gives exactly what the current version writes.
    assert_eq!(
        grid4d_io::to_bytes(&doc).unwrap(),
        grid4d_io::to_bytes(&Document::with_demo_scene()).unwrap()
    );
}

#[test]
fn the_version_1_fixture_still_loads() {
    let doc = grid4d_io::load(&fixture_path(DEMO_V1)).unwrap();
    assert_eq!(outline(&doc), DEMO_OUTLINE.map(|(d, n)| (d, n.to_owned())));
    let sphere = doc.scene.iter().find(|(_, n)| n.data.name == "Sphere").unwrap().1;
    assert_eq!(
        sphere.data.kind,
        ObjectKind::Sphere {
            radius: 80.0,
            segments: 24
        },
        "migrated with default segments"
    );
    // Migrating then saving gives exactly what the current version writes.
    assert_eq!(
        grid4d_io::to_bytes(&doc).unwrap(),
        grid4d_io::to_bytes(&Document::with_demo_scene()).unwrap()
    );
}

#[test]
fn save_and_load_through_the_filesystem() {
    let dir = temp_dir("save_load");
    let path = dir.join(format!("shot.{}", grid4d_io::EXTENSION));

    let mut doc = Document::with_demo_scene();
    grid4d_io::save(&doc, &path).unwrap();
    // Saving again replaces the file in place.
    doc.name = "Second save".to_owned();
    grid4d_io::save(&doc, &path).unwrap();

    let loaded = grid4d_io::load(&path).unwrap();
    assert_eq!(loaded.name, "Second save");
    assert_eq!(loaded.scene.len(), doc.scene.len());
    let leftovers: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        leftovers,
        [format!("shot.{}", grid4d_io::EXTENSION)],
        "no temporary file left behind"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn loading_a_missing_file_is_an_io_error() {
    let missing = temp_dir("missing").join("nothing-here.g4d");
    assert!(matches!(grid4d_io::load(&missing), Err(grid4d_io::IoError::Io(_))));
}
