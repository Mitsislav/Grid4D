//! Property tests: random documents go through save and load and come back
//! the same — saved again, they give the same bytes. Every kind of object with
//! random parameters, a random hierarchy, transforms, switches, tags,
//! animation, materials, layers and plugin values (see `docs/FILE_FORMAT.md`);
//! the same documents labelled version 4 (Grid4D 1.1) and, without what
//! version 4 added, version 3 (Grid4D 1.0) read the same.

#![allow(clippy::unwrap_used, reason = "test code")]

use glam::Vec3;
use grid4d_scene::{
    Axis, Curve, Document, Interpolation, Key, Layer, Material, Node, NodeId, ObjectKind, ObjectKindTag, Param,
    ParamUnit, PluginObject, PluginValue, Tag, Track,
};
use proptest::prelude::*;

/// An object as the strategy makes it, before it is placed in a document.
#[derive(Clone, Debug)]
struct RandomObject {
    name: String,
    kind: ObjectKind,
    position: Vec3,
    rotation: Vec3,
    scale: Vec3,
    color: [f32; 3],
    switches: [bool; 3],
    /// Chooses the parent among the objects before it (or none).
    parent: u32,
    phong: Option<f32>,
    protection: Option<[bool; 3]>,
    material: Option<u32>,
    layer: Option<u32>,
    keys: Vec<(f64, f32, u8)>,
}

fn vector(range: std::ops::Range<f32>) -> impl Strategy<Value = Vec3> {
    (range.clone(), range.clone(), range).prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

fn colour() -> impl Strategy<Value = [f32; 3]> {
    (0.0f32..=1.0, 0.0f32..=1.0, 0.0f32..=1.0).prop_map(|(r, g, b)| [r, g, b])
}

/// The kinds of object version 4 has (version 5's get a strategy of their own).
fn version_4_kinds() -> Vec<ObjectKindTag> {
    ObjectKindTag::ALL
        .into_iter()
        .filter(|t| !matches!(t, ObjectKindTag::Sky | ObjectKindTag::Environment))
        .collect()
}

/// A kind of object (one version 4 has) at its defaults, its keyable
/// parameters set at random (within what the object accepts), or a plugin
/// object with random values.
fn kind() -> impl Strategy<Value = ObjectKind> {
    let builtin = (
        prop::sample::select(version_4_kinds()),
        prop::collection::vec(0.0f32..1.0, 8),
    )
        .prop_map(|(tag, values)| {
            let mut kind = tag.default_kind();
            for (param, v) in kind.params().into_iter().zip(values) {
                let value = match param.unit() {
                    ParamUnit::Fraction => v,
                    ParamUnit::Radians => v * 3.0,
                    ParamUnit::Plain => 1.0 + v * 400.0,
                };
                kind.set_param(param, value);
            }
            kind
        });
    let plugin = (
        "[a-z]{1,8}\\.[a-z]{1,8}",
        -1000.0f32..1000.0,
        any::<i32>(),
        any::<bool>(),
        0u32..10,
        vector(-100.0..100.0),
        colour(),
    )
        .prop_map(|(type_id, float, int, flag, choice, vector, color)| {
            let mut object = PluginObject::new(type_id);
            object.set("Float", PluginValue::Float(float));
            object.set("Int", PluginValue::Int(int));
            object.set("Bool", PluginValue::Bool(flag));
            object.set("Choice", PluginValue::Choice(choice));
            object.set("Vector", PluginValue::Vector(vector));
            object.set("Color", PluginValue::Color(color));
            ObjectKind::Plugin(object)
        });
    prop_oneof![4 => builtin, 1 => plugin]
}

fn object() -> impl Strategy<Value = RandomObject> {
    (
        (
            "[A-Za-z0-9 ._-]{0,12}",
            kind(),
            vector(-1000.0..1000.0),
            vector(-6.0..6.0),
            vector(0.1..10.0),
        ),
        (colour(), any::<[bool; 3]>(), any::<u32>()),
        (
            prop::option::of(0.0f32..std::f32::consts::PI),
            prop::option::of(any::<[bool; 3]>()),
            prop::option::of(any::<u32>()),
            prop::option::of(any::<u32>()),
        ),
        prop::collection::vec((0.0f64..200.0, -500.0f32..500.0, 0u8..3), 0..5),
    )
        .prop_map(
            |(
                (name, kind, position, rotation, scale),
                (color, switches, parent),
                (phong, protection, material, layer),
                keys,
            )| RandomObject {
                name,
                kind,
                position,
                rotation,
                scale,
                color,
                switches,
                parent,
                phong,
                protection,
                material,
                layer,
                keys,
            },
        )
}

/// One of `list`, chosen by `n` (none without `n` or an empty list).
fn pick<T: Copy>(list: &[T], n: Option<u32>) -> Option<T> {
    n.and_then(|n| list.get(usize::try_from(n).ok()? % list.len().max(1)).copied())
}

/// Builds the document: the materials and layers, then the objects, each
/// under one of those before it or at the top.
fn document(objects: Vec<RandomObject>, materials: Vec<([f32; 3], f32)>, layers: Vec<[f32; 3]>) -> Document {
    let mut doc = Document::new("random");
    let materials: Vec<_> = materials
        .into_iter()
        .enumerate()
        .map(|(i, (color, roughness))| {
            let mut material = Material::new(format!("Material {i}"));
            material.color = color;
            material.roughness = roughness;
            doc.scene.materials_mut().insert(material)
        })
        .collect();
    let layers: Vec<_> = layers
        .into_iter()
        .enumerate()
        .map(|(i, color)| doc.scene.layers_mut().insert(Layer::new(format!("Layer {i}"), color)))
        .collect();
    let mut ids: Vec<NodeId> = Vec::new();
    for object in objects {
        let mut node = Node::new(object.name, object.kind);
        let data = &mut node.data;
        data.transform.position = object.position;
        data.transform.rotation = object.rotation;
        data.transform.scale = object.scale;
        data.display_color = object.color;
        [data.visible_in_editor, data.visible_in_render, data.enabled] = object.switches;
        if let Some(angle) = object.phong {
            data.tags.push(Tag::Phong {
                angle_limit: true,
                angle,
            });
        }
        if let Some([position, scale, rotation]) = object.protection {
            data.tags.push(Tag::Protection {
                position,
                scale,
                rotation,
            });
        }
        if let Some(material) = pick(&materials, object.material) {
            data.tags
                .push(Tag::Material(grid4d_scene::MaterialTag::new(Some(material))));
        }
        data.layer = pick(&layers, object.layer);
        if !object.keys.is_empty() {
            let interpolation = |i: u8| match i {
                0 => Interpolation::Spline,
                1 => Interpolation::Linear,
                _ => Interpolation::Step,
            };
            let keys = object.keys.iter().map(|&(frame, value, i)| Key {
                interpolation: interpolation(i),
                ..Key::new(frame.round(), value)
            });
            data.tracks.push(Track {
                param: Param::Position(Axis::X),
                curve: Curve::from_keys(keys),
            });
            // An object parameter too, when the object has one (a plugin
            // object's first value, `ObjectParam::Plugin(0)`).
            if let Some(&param) = data.kind.params().first() {
                let value = data.kind.param(param).unwrap_or(0.0);
                data.tracks.push(Track {
                    param: Param::Object(param),
                    curve: Curve::from_keys([Key::new(0.0, value), Key::new(10.0, value)]),
                });
            }
        }
        let parent = (!ids.is_empty() && object.parent % 3 != 0)
            .then(|| ids[usize::try_from(object.parent).unwrap() % ids.len()]);
        let id = match parent {
            Some(parent) => doc.scene.insert_child(parent, node).unwrap(),
            None => doc.scene.insert(node),
        };
        ids.push(id);
    }
    doc
}

// 256 documents a run (`PROPTEST_CASES` changes it).
proptest! {
    #[test]
    fn random_documents_save_load_and_save_to_the_same_bytes(
        objects in prop::collection::vec(object(), 0..24),
        materials in prop::collection::vec((colour(), 0.0f32..=1.0), 0..3),
        layers in prop::collection::vec(colour(), 0..3),
        textures in prop::collection::vec(
            ("[a-z]{1,8}", prop::option::of(prop::collection::vec(any::<u8>(), 0..32)), any::<bool>()),
            0..4,
        ),
        seams in prop::collection::vec(prop::collection::vec(any::<bool>(), 24), 0..3),
    ) {
        let mut doc = document(objects, materials, layers);
        // UV seams (version 4): cubes whose corners are flagged at random.
        for flags in seams {
            let mut state = grid4d_scene::ComponentState::default();
            state.seams.extend(flags.iter().enumerate().filter(|(_, f)| **f).map(|(c, _)| c));
            state.selected.edges.extend(flags.iter().enumerate().filter(|(_, f)| !**f).map(|(c, _)| c));
            doc.scene.insert(Node::new(
                "Seamed",
                ObjectKind::Polygon {
                    mesh: grid4d_scene::MeshHandle::new(grid4d_geom::primitives::cube(Vec3::splat(10.0), [1, 1, 1])),
                    components: grid4d_scene::Components::new(state),
                },
            ));
        }
        // Textures (version 4): linked by a relative path, or embedded.
        for (name, embedded, linear) in textures {
            let source = match embedded {
                Some(bytes) => grid4d_scene::TextureSource::Embedded {
                    file_name: format!("{name}.png"),
                    bytes: bytes.into(),
                },
                None => grid4d_scene::TextureSource::Linked(format!("maps/{name}.jpg").into()),
            };
            let color_space = if linear { grid4d_scene::ColorSpace::Linear } else { grid4d_scene::ColorSpace::Srgb };
            doc.scene.textures_mut().insert(grid4d_scene::Texture { name, source, color_space });
        }
        // Material channels (version 4): values away from the defaults, and a
        // texture on a channel when there is one.
        let textures: Vec<grid4d_scene::TextureId> = doc.scene.textures().ids().to_vec();
        let ids: Vec<grid4d_scene::MaterialId> = doc.scene.materials().ids().to_vec();
        for (k, id) in ids.into_iter().enumerate() {
            let material = doc.scene.materials_mut().get_mut(id).unwrap();
            let f = f32::from(u8::try_from(k % 10).unwrap()) / 10.0;
            (material.specular, material.transmission, material.clearcoat) = (f, 1.0 - f, f * 0.5);
            material.ior = 1.0 + f;
            material.two_sided = k % 2 == 0;
            if let Some(&texture) = textures.get(k % textures.len().max(1)) {
                let channel = grid4d_scene::Channel::ALL[k % grid4d_scene::Channel::ALL.len()];
                *material.map_mut(channel) = Some(grid4d_scene::TextureSlot {
                    tiling: [1.0 + f, 2.0],
                    source: grid4d_scene::PictureChannel::ALL[k % 4],
                    ..grid4d_scene::TextureSlot::new(texture)
                });
            }
        }
        // Material tags' projections (version 4), and a second tag on some.
        let objects: Vec<NodeId> = doc.scene.iter().map(|(id, _)| id).collect();
        for (k, &id) in objects.iter().enumerate() {
            let camera = objects.first().copied();
            let data = &mut doc.scene.get_mut(id).unwrap().data;
            let f = f32::from(u8::try_from(k % 7).unwrap());
            for tag in &mut data.tags {
                if let Tag::Material(tag) = tag {
                    tag.projection = grid4d_scene::Projection::ALL[k % 6];
                    tag.offset = [f * 0.1, -f];
                    tag.tiling = [1.0 + f, 0.5];
                    tag.rotation = f * 0.2;
                    tag.camera = camera.filter(|_| k % 2 == 0);
                    tag.placement = (k % 3 == 0).then_some(grid4d_scene::Placement {
                        position: Vec3::splat(f),
                        rotation: Vec3::new(0.0, f * 0.1, 0.0),
                        size: Vec3::new(1.0, 2.0, f + 1.0),
                    });
                }
            }
            if k % 3 == 1 && data.tags.iter().any(|t| matches!(t, Tag::Material(_))) {
                data.tags.push(Tag::Material(grid4d_scene::MaterialTag::new(None)));
            }
        }
        let bytes = grid4d_io::to_bytes(&doc).unwrap();
        let loaded = grid4d_io::from_bytes(&bytes).unwrap();
        prop_assert_eq!(loaded.scene.len(), doc.scene.len());
        let names = |doc: &Document| {
            let mut out = Vec::new();
            doc.scene.walk(&mut Vec::new(), |_, node, _, depth| {
                out.push((depth, node.data.name.clone()));
                true
            });
            out
        };
        prop_assert_eq!(names(&loaded), names(&doc));
        let again = grid4d_io::to_bytes(&loaded).unwrap();
        prop_assert!(again == bytes, "saved again, the file changed");
        // Written as version 4 (Grid4D 1.1) would have: the same document.
        // Everything above is version 4's; Phase 16's additions get
        // strategies of their own, left out of this check.
        let as_v4 = grid4d_io::from_bytes(&labelled(&bytes, 4)).unwrap();
        prop_assert!(grid4d_io::to_bytes(&as_v4).unwrap() == bytes, "version 4 read differently");
    }

    /// What version 5 adds round-trips; labelled version 4, a file reads as
    /// Grid4D 1.1 read it (the additions dropped, at their defaults).
    #[test]
    fn version_5_additions_round_trip_and_a_version_4_file_drops_them(
        objects in prop::collection::vec(object(), 0..8),
        clamp_indirect in any::<bool>(),
        emissive_lights in any::<bool>(),
        area in prop::option::of((0usize..4, 1.0f32..500.0, 1.0f32..500.0, any::<bool>(), any::<bool>())),
        sky in prop::option::of((any::<bool>(), -0.17f32..1.57, 0.0f32..6.0, 1.7f32..10.0, 0.0f32..5.0, any::<bool>(), 0.1f32..20.0, any::<bool>())),
        environment in prop::option::of((any::<bool>(), 0.0f32..5.0, -6.0f32..6.0, any::<bool>())),
        linked in prop::option::of((any::<bool>(), "[A-Za-z]{0,8}")),
    ) {
        let mut doc = document(objects, Vec::new(), Vec::new());
        // Step 16.2: the light-transport switches, an area light.
        doc.render.clamp_indirect = clamp_indirect;
        doc.render.emissive_lights = emissive_lights;
        if let Some((shape, x, y, visible, two_sided)) = area {
            doc.scene.insert(Node::new(
                "Area",
                ObjectKind::Light {
                    color: [1.0; 3],
                    intensity: 1.0,
                    light_type: grid4d_scene::LightType::Area,
                    details: grid4d_scene::LightDetails {
                        area_shape: grid4d_scene::AreaShape::ALL[shape],
                        area_size: [x, y],
                        area_visible: visible,
                        two_sided,
                        ..grid4d_scene::LightDetails::default()
                    },
                    shadow: grid4d_scene::LightShadow::Area,
                    linking: grid4d_scene::Linking::default(),
                    group: String::new(),
                },
            ));
        }
        // Step 16.4: a light linked to the objects so far, in a group.
        if let Some((include, group)) = &linked {
            let objects: Vec<_> = doc.scene.iter().map(|(id, _)| id).take(3).collect();
            let mut light = ObjectKind::light([1.0; 3], 1.0);
            if let ObjectKind::Light { linking, group: own, .. } = &mut light {
                linking.mode = if *include { grid4d_scene::LinkMode::Include } else { grid4d_scene::LinkMode::Exclude };
                linking.objects = objects;
                own.clone_from(group);
            }
            doc.scene.insert(Node::new("Linked", light));
        }
        // Step 16.3: a Sky, an Environment (with a texture or none).
        if let Some((angles, elevation, azimuth, turbidity, intensity, sun, sun_size, visible)) = sky {
            doc.scene.insert(Node::new(
                "Sky",
                ObjectKind::Sky(grid4d_scene::Sky {
                    placement: if angles { grid4d_scene::SunPlacement::Angles } else { grid4d_scene::SunPlacement::FromObject },
                    elevation,
                    azimuth,
                    turbidity,
                    intensity,
                    sun,
                    sun_size,
                    sun_intensity: intensity * 0.5,
                    visible,
                    group: String::new(),
                }),
            ));
        }
        if let Some((textured, intensity, rotation, visible)) = environment {
            let texture = textured.then(|| {
                doc.scene
                    .textures_mut()
                    .insert(grid4d_scene::Texture::linked(std::path::PathBuf::from("sky.hdr")))
            });
            doc.scene.insert(Node::new(
                "Environment",
                ObjectKind::Environment(grid4d_scene::Environment { texture, intensity, rotation, visible, group: "Fill".to_owned() }),
            ));
        }
        let bytes = grid4d_io::to_bytes(&doc).unwrap();
        let loaded = grid4d_io::from_bytes(&bytes).unwrap();
        prop_assert_eq!(&loaded.render, &doc.render);
        prop_assert!(grid4d_io::to_bytes(&loaded).unwrap() == bytes);
        let as_v4 = grid4d_io::from_bytes(&labelled(&bytes, 4));
        if area.is_some() || sky.is_some() || environment.is_some() {
            // Grid4D 1.1 refused the Area light type and these kinds of
            // object: so does a version 4 file.
            prop_assert!(as_v4.is_err());
        } else {
            let mut expected = doc.render;
            (expected.clamp_indirect, expected.emissive_lights) = (true, true);
            let as_v4 = as_v4.unwrap();
            prop_assert_eq!(&as_v4.render, &expected);
            // Lights light everything, in no group.
            for (_, node) in as_v4.scene.iter() {
                if let ObjectKind::Light { linking, group, .. } = &node.data.kind {
                    prop_assert!(linking.is_open() && group.is_empty());
                }
            }
        }
    }

    /// Written as version 3 (Grid4D 1.0) would have: the same document comes
    /// back. Every document the strategies make holds only what version 3
    /// holds; Phase 15's additions get strategies of their own.
    #[test]
    fn random_version_3_documents_read_as_the_same_document(
        objects in prop::collection::vec(object(), 0..12),
        materials in prop::collection::vec((colour(), 0.0f32..=1.0), 0..3),
        layers in prop::collection::vec(colour(), 0..3),
    ) {
        let doc = document(objects, materials, layers);
        let bytes = grid4d_io::to_bytes(&doc).unwrap();
        let loaded = grid4d_io::from_bytes(&labelled(&bytes, 3)).unwrap();
        prop_assert!(grid4d_io::to_bytes(&loaded).unwrap() == bytes, "version 3 read differently");
    }
}

/// The archive `bytes` with its `document.json` labelled `version`.
fn labelled(bytes: &[u8], version: u32) -> Vec<u8> {
    use std::io::{Cursor, Read as _, Write as _};

    let current = format!("\"version\": {}", grid4d_io::FORMAT_VERSION);
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().unwrap().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        if name == "document.json" {
            let text = String::from_utf8(data).unwrap();
            assert!(text.contains(&current));
            data = text
                .replacen(&current, &format!("\"version\": {version}"), 1)
                .into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(&data).unwrap();
    }
    out.finish().unwrap().into_inner()
}
