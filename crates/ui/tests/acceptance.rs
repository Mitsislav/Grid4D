//! Phase 15's acceptance, as one run through the editor's commands (see
//! `docs/IMPLEMENTATION_PLAN.md`, Phase 15 ▸ Acceptance): a textured glTF
//! model comes in; a cube gets a photo through a Box projection; a capsule
//! is unwrapped from a marked seam and its islands packed; a sphere gets
//! glass and a plane brushed metal with a normal picture; a still is
//! rendered with the pictures; the document is saved as version 4 and opens
//! the same; it is exported to glTF and read back with its materials; a 1.0
//! file opens.

#![allow(clippy::unwrap_used, reason = "test code")]

use std::{path::PathBuf, sync::Arc};

use glam::Vec3;
use grid4d_geom::{Component, uv};
use grid4d_raytrace::{Picture, Pictures, RenderCamera, RenderOptions, RenderScene, Texels, Texturing};
use grid4d_scene::{
    Channel, ColorSpace, Document, Material, MaterialTag, Node, NodeId, ObjectKind, ObjectKindTag, PictureChannel,
    Projection, Tag, Texture, TextureSlot, TextureSource,
};
use grid4d_ui::{EditMode, EditorCommand, EditorState, SelectMode, ViewportPanel, uv_tools::UvOp};

fn folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grid4d-acceptance-15-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 4 × 4 PNG of `texel(x, y)` (RGBA).
fn png(texel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            pixels.extend(texel(x, y));
        }
    }
    grid4d_io::png_bytes(4, 4, grid4d_io::Channels::Rgba, &pixels).unwrap()
}

struct Editor {
    doc: Document,
    editor: EditorState,
    viewport: ViewportPanel,
}

impl Editor {
    fn run(&mut self, commands: impl IntoIterator<Item = EditorCommand>) {
        for command in commands {
            self.editor.commands.push(command);
            grid4d_ui::commands::apply_pending(&mut self.doc, &mut self.editor, &mut self.viewport);
        }
    }

    /// A new object of `tag`, made editable, named `name`.
    fn editable(&mut self, tag: ObjectKindTag, name: &str) -> NodeId {
        self.run([EditorCommand::CreateObject(tag), EditorCommand::MakeEditable]);
        let id = self.editor.selection.active().unwrap();
        self.doc.scene.get_mut(id).unwrap().data.name = name.to_owned();
        id
    }

    /// `material` on object `id` through a tag with `projection`.
    fn give(&mut self, id: NodeId, material: Material, projection: Projection) {
        let material = self.doc.scene.materials_mut().insert(material);
        let mut tag = MaterialTag::new(Some(material));
        tag.projection = projection;
        self.doc.scene.get_mut(id).unwrap().data.tags.push(Tag::Material(tag));
    }

    fn embed(&mut self, name: &str, bytes: Vec<u8>, color_space: ColorSpace) -> grid4d_scene::TextureId {
        self.doc.scene.textures_mut().insert(Texture {
            name: name.to_owned(),
            source: TextureSource::Embedded {
                file_name: format!("{name}.png"),
                bytes: bytes.into(),
            },
            color_space,
        })
    }
}

/// The decoded pictures of `doc`'s embedded and linked textures, for a render.
fn pictures(doc: &Document) -> Pictures {
    let mut pictures = Pictures::new();
    for (id, texture) in doc.scene.textures().iter() {
        let bytes: Arc<[u8]> = match &texture.source {
            TextureSource::Embedded { bytes, .. } => Arc::clone(bytes),
            TextureSource::Linked(path) => std::fs::read(path).unwrap().into(),
        };
        let image = grid4d_io::decode_texture(&bytes, texture.color_space).unwrap();
        let level = &image.levels[0];
        let texels = match (&level.pixels, texture.color_space) {
            (grid4d_io::Pixels::Rgba8(b), ColorSpace::Srgb) => Texels::Srgb8(Arc::clone(b)),
            (grid4d_io::Pixels::Rgba8(b), ColorSpace::Linear) => Texels::Linear8(Arc::clone(b)),
            (grid4d_io::Pixels::RgbaF16(v), _) => Texels::Half(Arc::clone(v)),
        };
        pictures.insert(id, Arc::new(Picture::new(level.width, level.height, texels).unwrap()));
    }
    pictures
}

#[test]
fn phase_15_from_a_textured_model_to_a_render_and_back() {
    let dir = folder();
    let mut e = Editor {
        doc: Document::new("Acceptance"),
        editor: EditorState::default(),
        viewport: ViewportPanel::new(None),
    };

    // 1. A textured glTF model comes in, with its picture.
    let mut model = Document::new("Model");
    let wood = model.scene.textures_mut().insert(Texture {
        name: "Wood".to_owned(),
        source: TextureSource::Embedded {
            file_name: "wood.png".to_owned(),
            bytes: png(|x, _| [120 + 30 * u8::try_from(x).unwrap(), 70, 30, 255]).into(),
        },
        color_space: ColorSpace::Srgb,
    });
    let mut planks = Material::new("Planks");
    *planks.map_mut(Channel::BaseColor) = Some(TextureSlot::new(wood));
    let planks = model.scene.materials_mut().insert(planks);
    let mut board = Node::new("Board", ObjectKindTag::Plane.default_kind());
    board.data.tags = vec![Tag::Material(MaterialTag::new(Some(planks)))];
    model.scene.insert(board);
    let glb = dir.join("model.glb");
    grid4d_io::export(&grid4d_ui::file::exportable(&model), &glb).unwrap();
    e.run([EditorCommand::MergeFile(glb)]);
    let (_, merged) = e
        .doc
        .scene
        .materials()
        .iter()
        .find(|(_, m)| m.name == "Planks")
        .unwrap();
    let slot = merged
        .map(Channel::BaseColor)
        .expect("the model's picture came with it");
    assert!(e.doc.scene.textures().contains(slot.texture));

    // 2. A cube with a photo through a Box projection (a picture linked next to the document).
    let photo = dir.join("photo.png");
    std::fs::write(&photo, png(|x, y| [40 * u8::try_from(x + y).unwrap(), 90, 160, 255])).unwrap();
    let cube = e.editable(ObjectKindTag::Cube, "Crate");
    let picture = e.doc.scene.textures_mut().insert(Texture::linked(photo));
    let mut photographed = Material::new("Photo");
    *photographed.map_mut(Channel::BaseColor) = Some(TextureSlot::new(picture));
    e.give(cube, photographed, Projection::Box);

    // 3. A capsule unwrapped from a seam marked along one side, then packed.
    let capsule = e.editable(ObjectKindTag::Capsule, "Figure");
    let seam: Vec<u32> = {
        let ObjectKind::Polygon { mesh, .. } = &e.doc.scene.get(capsule).unwrap().data.kind else {
            panic!("editable")
        };
        let points = mesh.mesh().points();
        mesh.topology()
            .edges()
            .iter()
            .enumerate()
            .filter(|(_, [a, b])| {
                let (pa, pb) = (points[*a as usize], points[*b as usize]);
                // One line from pole to pole: points on the +X side of the XY plane.
                pa.z.abs() < 1e-3 && pb.z.abs() < 1e-3 && pa.x >= -1e-3 && pb.x >= -1e-3 && pa != pb
            })
            .map(|(e, _)| u32::try_from(e).unwrap())
            .collect()
    };
    assert!(seam.len() > 5, "a seam from pole to pole: {}", seam.len());
    e.editor.mode = EditMode::Edges;
    e.run([
        EditorCommand::SelectComponents {
            node: capsule,
            component: Component::Edge,
            hits: seam,
            mode: SelectMode::Replace,
        },
        EditorCommand::UvCommand(UvOp::MarkSeams),
        EditorCommand::DeselectAll,
    ]);
    e.editor.mode = EditMode::Polygons;
    e.run([
        EditorCommand::UvCommand(UvOp::Unwrap),
        EditorCommand::UvCommand(UvOp::Pack),
    ]);
    {
        let ObjectKind::Polygon { mesh, components } = &e.doc.scene.get(capsule).unwrap().data.kind else {
            panic!("editable")
        };
        assert!(!components.state().seams.is_empty(), "the seams are kept");
        assert_eq!(uv::flipped_faces(mesh.mesh()), 0);
        assert_eq!(
            uv::layout(mesh.mesh(), mesh.topology()).island_count,
            1,
            "one piece, cut along the seam"
        );
        assert!(
            mesh.mesh()
                .uvs()
                .unwrap()
                .iter()
                .all(|uv| (-1e-4..=1.0001).contains(&uv.x) && (-1e-4..=1.0001).contains(&uv.y))
        );
    }

    // 4. Glass on a sphere, brushed metal (a normal picture) on a plane.
    let sphere = e.editable(ObjectKindTag::Sphere, "Glass Ball");
    let mut glass = Material::new("Glass");
    (glass.color, glass.roughness, glass.transmission) = ([0.9, 1.0, 0.95], 0.05, 1.0);
    e.give(sphere, glass, Projection::Uv);
    let floor = e.editable(ObjectKindTag::Plane, "Floor");
    let lines = e.embed(
        "Brushed",
        png(|x, _| {
            if x % 2 == 0 {
                [150, 128, 230, 255]
            } else {
                [106, 128, 230, 255]
            }
        }),
        ColorSpace::Linear,
    );
    let mut metal = Material::new("Brushed Metal");
    (metal.color, metal.metallic, metal.roughness) = ([0.8; 3], 1.0, 0.35);
    *metal.map_mut(Channel::Normal) = Some(TextureSlot {
        tiling: [20.0, 1.0],
        ..TextureSlot::new(lines)
    });
    *metal.map_mut(Channel::Roughness) = Some(TextureSlot {
        source: PictureChannel::Green,
        range: [0.2, 0.5],
        ..TextureSlot::new(lines)
    });
    e.give(floor, metal, Projection::Uv);
    for (id, at) in [
        (cube, Vec3::new(-250.0, 100.0, 0.0)),
        (capsule, Vec3::new(0.0, 100.0, -150.0)),
        (sphere, Vec3::new(250.0, 100.0, 0.0)),
    ] {
        e.doc.scene.get_mut(id).unwrap().data.transform.position = at;
    }
    e.doc.scene.get_mut(floor).unwrap().data.transform.scale = Vec3::splat(4.0);

    // 5. A still with the pictures.
    let mut evaluated = grid4d_eval::EvaluatedScene::for_render();
    evaluated.update(&e.doc.scene);
    let camera = RenderCamera::framing(Vec3::new(-400.0, 0.0, -300.0), Vec3::new(400.0, 250.0, 300.0), 0.7);
    let pictures = pictures(&e.doc);
    assert_eq!(pictures.len(), e.doc.scene.textures().len(), "every picture decodes");
    let texturing = Texturing {
        materials: e.doc.scene.materials(),
        pictures: &pictures,
        camera,
        aspect: 1.5,
        cameras: &[],
    };
    let options = RenderOptions {
        width: 48,
        height: 32,
        samples: 2,
        bounces: 2,
        seed: 0,
        auto_light: true,
    };
    let image = grid4d_raytrace::render(&RenderScene::textured(&evaluated, &texturing), &camera, &options);
    let covered = image.pixels.iter().filter(|p| p[3] > 0.5).count();
    assert!(
        covered > image.pixels.len() / 3,
        "the scene fills the picture: {covered}"
    );
    assert!(image.pixels.iter().all(|p| p.iter().all(|v| v.is_finite())));

    // 6. Saved as version 4, it opens the same.
    let saved = dir.join("scene.g4d");
    grid4d_io::save(&e.doc, &saved).unwrap();
    assert_eq!(grid4d_io::FORMAT_VERSION, 4);
    let reopened = grid4d_io::load(&saved).unwrap();
    assert_eq!(
        grid4d_io::to_bytes(&reopened).unwrap(),
        grid4d_io::to_bytes(&e.doc).unwrap()
    );

    // 7. Exported to glTF and read back with its materials and pictures.
    let exported = dir.join("scene.glb");
    grid4d_io::export(&grid4d_ui::file::exportable(&e.doc), &exported).unwrap();
    let back = grid4d_io::import(&exported).unwrap();
    for name in ["Planks", "Photo", "Glass", "Brushed Metal"] {
        assert!(back.materials.iter().any(|(_, m)| m.name == name), "{name} comes back");
    }
    let (_, glass_back) = back.materials.iter().find(|(_, m)| m.name == "Glass").unwrap();
    assert!((glass_back.transmission - 1.0).abs() < 1e-6);
    let (_, metal_back) = back.materials.iter().find(|(_, m)| m.name == "Brushed Metal").unwrap();
    assert!(metal_back.map(Channel::Normal).is_some(), "the normal picture travels");
    assert!(back.textures.len() >= 3);

    // 8. A Grid4D 1.0 file opens.
    let old = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../io/tests/fixtures/demo_v3.g4d");
    let doc = grid4d_io::load(&old).unwrap();
    assert!(doc.scene.len() > 3);
    let _ = std::fs::remove_dir_all(&dir);
}
