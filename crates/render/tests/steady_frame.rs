//! "Zero allocations in a steady frame": once warm, a frame of the viewport's
//! CPU side — evaluation (animated scenes evaluate every frame), extraction
//! and the component overlay — must not allocate, on a scene of the
//! performance budget's size (10 000 objects). Counted per thread by a
//! counting global allocator (`allocation-counter`), linked into this test
//! binary only.

#![allow(clippy::unwrap_used, reason = "test code")]

use glam::Vec3;
use grid4d_eval::EvaluatedScene;
use grid4d_geom::Component;
use grid4d_render::{ComponentHit, FrameData, OrbitCamera};
use grid4d_scene::{Axis, Curve, Document, Key, Node, NodeData, NodeId, ObjectKind, Param, Track};

/// `count` primitives of every shape in groups of 100, every `animated`-th
/// one moving and turning over frames 0–90, one polygon object, and a Sky
/// whose sun rises over the frames (an Environment under it, unused). The
/// groups take turns at a plain material, a textured one on the objects' UVs
/// and the textured one through a Box projection.
fn scene(count: usize, animated: usize) -> (Document, NodeId) {
    use grid4d_scene::{Channel, Material, MaterialTag, Projection, Tag, Texture, TextureSlot};
    let mut doc = Document::with_demo_scene();
    let wood = doc.scene.textures_mut().insert(Texture::linked("wood.png".into()));
    let plain = doc.scene.materials_mut().insert(Material::new("Plain"));
    let mut oak = Material::new("Oak");
    *oak.map_mut(Channel::BaseColor) = Some(TextureSlot::new(wood));
    let oak = doc.scene.materials_mut().insert(oak);
    let tags = [
        MaterialTag::new(Some(plain)),
        MaterialTag::new(Some(oak)),
        MaterialTag {
            projection: Projection::Box,
            ..MaterialTag::new(Some(oak))
        },
    ];
    let kinds = [
        grid4d_scene::ObjectKindTag::Cube,
        grid4d_scene::ObjectKindTag::Sphere,
        grid4d_scene::ObjectKindTag::Cylinder,
        grid4d_scene::ObjectKindTag::Cone,
        grid4d_scene::ObjectKindTag::Torus,
        grid4d_scene::ObjectKindTag::Capsule,
        grid4d_scene::ObjectKindTag::Plane,
        grid4d_scene::ObjectKindTag::Disc,
    ];
    let mut group = None;
    for k in 0..count {
        if k % 100 == 0 {
            let mut node = Node::new("Group", ObjectKind::Null);
            node.data.tags = vec![Tag::Material(tags[(k / 100) % tags.len()])];
            group = Some(doc.scene.insert(node));
        }
        let mut data = NodeData::new("Object", kinds[k % kinds.len()].default_kind());
        let (x, z) = (k % 100, k / 100);
        let place = |i: usize| f32::from(u16::try_from(i).unwrap()) * 250.0;
        data.transform.position = Vec3::new(place(x), 0.0, place(z));
        data.transform.scale = Vec3::splat(0.5);
        if k % animated == 0 {
            data.tracks.push(track(Param::Position(Axis::Y), 0.0, 300.0));
            data.tracks.push(track(Param::Rotation(Axis::X), 0.0, 3.0));
        }
        let parent = group.unwrap();
        doc.scene.insert_child(parent, Node::from_data(data)).unwrap();
    }
    let sphere = ObjectKind::Sphere {
        radius: 100.0,
        segments: 48,
    };
    let mesh = grid4d_eval::editable_mesh(&sphere).unwrap();
    let polygon = doc.scene.insert(Node::new(
        "Mesh",
        ObjectKind::polygon(grid4d_scene::MeshHandle::new(mesh)),
    ));
    let mut sky = Node::new(
        "Sky",
        ObjectKind::Sky(grid4d_scene::Sky {
            placement: grid4d_scene::SunPlacement::Angles,
            ..grid4d_scene::Sky::default()
        }),
    );
    sky.data
        .tracks
        .push(track(Param::Object(grid4d_scene::ObjectParam::Elevation), 0.1, 1.2));
    doc.scene.insert(sky);
    doc.scene.insert(Node::new(
        "Environment",
        ObjectKind::Environment(grid4d_scene::Environment::default()),
    ));
    (doc, polygon)
}

/// A track of `param` from `from` at frame 0 to `to` at frame 90.
fn track(param: Param, from: f32, to: f32) -> Track {
    Track {
        param,
        curve: Curve::from_keys([Key::new(0.0, from), Key::new(90.0, to)]),
    }
}

/// One frame of a view, as the viewport draws it.
struct View {
    evaluated: EvaluatedScene,
    frame: FrameData,
    camera: OrbitCamera,
}

impl View {
    fn new() -> Self {
        Self {
            evaluated: EvaluatedScene::new(),
            frame: FrameData::new(),
            camera: OrbitCamera::default(),
        }
    }

    fn draw(
        &mut self,
        doc: &Document,
        at: f64,
        selected: &dyn Fn(NodeId) -> bool,
        target: Option<(NodeId, Component)>,
        hover: &[ComponentHit],
    ) {
        self.evaluated.set_frame(at);
        self.evaluated.update(&doc.scene);
        self.frame.set_camera(&self.camera, 16.0 / 9.0);
        self.frame
            .extract_with_materials(&self.evaluated, Some(doc.scene.materials()), selected);
        self.frame.extract_components(&self.evaluated, target, hover);
    }
}

#[test]
fn a_steady_frame_allocates_nothing() {
    let (doc, polygon) = scene(10_000, 10);
    let mut view = View::new();
    let none = |_: NodeId| false;
    let some = |id: NodeId| id == polygon;
    let target = Some((polygon, Component::Point));
    let hover = [ComponentHit::Point(7)];

    // Warm up: every frame once, both selections, the overlay built.
    for at in 0..=90 {
        view.draw(&doc, f64::from(at), &none, None, &[]);
    }
    for at in 0..=90 {
        view.draw(&doc, f64::from(at), &some, target, &hover);
    }

    assert!(view.frame.instance_count() > 10_000, "{}", view.frame.instance_count());
    let counted = allocation_counter::measure(|| {
        std::hint::black_box(Vec::<u8>::with_capacity(16));
    });
    assert_eq!(counted.count_total, 1, "the counter counts");

    // The animated objects do move from frame to frame.
    let height = |view: &View| -> f32 { view.evaluated.nodes().iter().map(|n| n.world.w_axis.y).sum() };
    view.draw(&doc, 10.0, &none, None, &[]);
    let early = height(&view);
    view.draw(&doc, 80.0, &none, None, &[]);
    assert!(height(&view) > early + 1000.0, "the scene is animated");

    // Playing: the animated objects evaluate again every frame.
    let playing = allocation_counter::measure(|| {
        for at in 0..=90 {
            view.draw(&doc, f64::from(at), &some, target, &hover);
        }
    });
    assert_eq!(playing.count_total, 0, "playing allocated: {playing:?}");

    // Orbiting a still frame: the camera moves, nothing evaluates.
    let orbiting = allocation_counter::measure(|| {
        for step in 0..100u8 {
            view.camera.yaw = f32::from(step) * 0.05;
            view.draw(&doc, 45.0, &some, target, &hover);
        }
    });
    assert_eq!(orbiting.count_total, 0, "orbiting allocated: {orbiting:?}");

    // Selection changing (Select All, Deselect All): only what is drawn
    // highlighted changes.
    let selecting = allocation_counter::measure(|| {
        for k in 0..20 {
            let selected: &dyn Fn(NodeId) -> bool = if k % 2 == 0 { &|_| true } else { &none };
            view.draw(&doc, 45.0, selected, target, &hover);
        }
    });
    assert_eq!(selecting.count_total, 0, "selecting allocated: {selecting:?}");
}
