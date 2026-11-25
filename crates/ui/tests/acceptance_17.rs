//! Phase 17's acceptance, as one run through the editor (see
//! `docs/IMPLEMENTATION_PLAN.md`, Phase 17 ▸ Acceptance), at a small size so
//! it runs as a test. A product shell:
//! - a box bevelled with a custom profile, two cylinders subtracted by a
//!   live Boolean (a cutter moved: the result follows and stays closed);
//! - half a handle completed by a Symmetry (welded: closed), its top
//!   extruded along a spline;
//! - a box creased under a Subdivision Surface (its corners stay sharp);
//! - a dent pushed into a panel with soft selection;
//! - an imported scan reduced to 10 % keeping its outline and UVs;
//! - a label wrapped round a bottle, a decal projected onto the shell;
//! - weighted normals on the shell.
//!
//! Every step undoes and redoes, the scene renders, it is saved as version 6
//! and opens the same, and every 1.x file opens.

#![allow(clippy::unwrap_used, reason = "test code")]

use std::{
    f32::consts::{FRAC_PI_2, PI, TAU},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use glam::Vec3;
use grid4d_geom::{
    BitSet, Component, PolyMesh, Topology,
    deformers::{ProjectToSurface, Wrap},
    ops::{BevelProfile, BevelShape, ProfilePreset},
    primitives,
    soft::{SoftScratch, SoftSelection, soft_weights},
};
use grid4d_render::OrbitCamera;
use grid4d_scene::{
    Document, MeshHandle, Node, NodeData, NodeId, ObjectKind, ObjectKindTag, Reduce, SceneEdit, Tag, Tags, Transform,
};
use grid4d_ui::{EditMode, EditorCommand, EditorState, SelectMode, Tool, ViewportPanel};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../io/tests/fixtures");

fn folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grid4d-acceptance-17-{}", std::process::id()));
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

    /// Adds `data` under `parent` (one undo step) and returns its id.
    fn insert(&mut self, parent: Option<NodeId>, data: NodeData) -> NodeId {
        let name = data.name.clone();
        self.run([EditorCommand::Edit {
            edit: SceneEdit::Insert {
                parent,
                index: usize::MAX,
                data,
            },
            label: "Create Object",
        }]);
        self.id(&name)
    }

    fn id(&self, name: &str) -> NodeId {
        self.doc
            .scene
            .iter()
            .find(|(_, n)| n.data.name == name)
            .unwrap_or_else(|| panic!("{name}"))
            .0
    }

    fn mesh(&self, id: NodeId) -> MeshHandle {
        match &self.doc.scene.get(id).unwrap().data.kind {
            ObjectKind::Polygon { mesh, .. } => mesh.clone(),
            other => panic!("not a polygon object: {}", other.label()),
        }
    }

    /// Selects every component of `id` in `mode` (Select All).
    fn select_all(&mut self, id: NodeId, mode: EditMode) {
        self.editor.selection.set_single(id);
        self.editor.mode = mode;
        self.run([EditorCommand::SelectAll]);
        self.editor.mode = EditMode::Model;
    }

    /// Applies the current modelling tool to `id` in `mode` (its Apply
    /// button).
    fn apply(&mut self, id: NodeId, tool: Tool, mode: EditMode) {
        self.editor.selection.set_single(id);
        (self.editor.tool, self.editor.mode) = (tool, mode);
        self.run([EditorCommand::ApplyMeshTool]);
        self.editor.mode = EditMode::Model;
    }

    /// Runs frames until no render runs.
    fn wait(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(300);
        self.frame();
        while self.editor.renders.running_number().is_some() {
            assert!(Instant::now() < deadline, "the render did not end");
            std::thread::sleep(Duration::from_millis(5));
            self.frame();
        }
        self.frame();
    }
}

fn node(name: &str, kind: ObjectKind, position: Vec3) -> NodeData {
    Node::new(name, kind).with_position(position).data
}

/// Every edge has two polygons.
fn closed(mesh: &PolyMesh) -> bool {
    let topology = Topology::build(mesh);
    mesh.face_count() > 0 && (0..topology.edge_count()).all(|e| topology.edge_faces(e).len() == 2)
}

fn polygon(mesh: PolyMesh) -> ObjectKind {
    ObjectKind::polygon(MeshHandle::new(mesh))
}

#[test]
fn phase_17_a_product_shell_modelled_saved_and_rendered() {
    let dir = folder();

    // Every 1.x file opens.
    let mut old = Editor::new(Document::new("Start"));
    for file in [
        "full_v5.g4d",
        "full_v4.g4d",
        "full_v3.g4d",
        "demo_v2.g4d",
        "demo_v1.oc4d",
    ] {
        old.run([EditorCommand::OpenFile(Path::new(FIXTURES).join(file))]);
        assert!(old.editor.file.alert.is_none(), "{file}: {:?}", old.editor.file.alert);
    }

    let mut e = Editor::new(Document::new("Shell"));
    let start = grid4d_io::to_bytes(&e.doc).unwrap();

    // 1. A box bevelled with a custom profile (a cove).
    let box_mesh = primitives::cube(Vec3::new(200.0, 100.0, 150.0), [1, 1, 1]);
    let boolean = e.insert(
        None,
        node(
            "Boolean",
            ObjectKind::Boolean(grid4d_scene::Boolean::default()),
            Vec3::ZERO,
        ),
    );
    let shell = e.insert(Some(boolean), node("Shell", polygon(box_mesh), Vec3::ZERO));
    {
        let bevel = &mut e.editor.modeling_settings.bevel;
        (bevel.offset, bevel.shape) = (8.0, BevelShape::Custom);
        bevel.profile = BevelProfile::preset(ProfilePreset::Cove);
    }
    e.select_all(shell, EditMode::Edges);
    e.apply(shell, Tool::Bevel, EditMode::Edges);
    let bevelled = e.mesh(shell);
    assert!(bevelled.mesh().face_count() > 6 * 4, "every edge bevelled");
    assert!(closed(bevelled.mesh()), "the bevelled box is closed");

    // 2. Two cylinders subtracted by the live Boolean.
    let cutters = e.insert(Some(boolean), node("Cutters", ObjectKind::Null, Vec3::ZERO));
    for (name, x) in [("Hole A", -50.0), ("Hole B", 50.0)] {
        let cylinder = ObjectKind::Cylinder {
            radius: 20.0,
            height: 300.0,
            rotation_segments: 24,
            height_segments: 1,
        };
        e.insert(Some(cutters), node(name, cylinder, Vec3::new(x, 0.0, 0.0)));
    }
    let result = grid4d_eval::generator_mesh(&e.doc.scene, boolean, 0.0).unwrap();
    assert!(closed(&result), "the Boolean's result is closed");
    assert!(result.face_count() > bevelled.mesh().face_count());
    // A cutter moved: the result follows, at an interactive pace.
    let mut evaluated = grid4d_eval::EvaluatedScene::new();
    evaluated.update(&e.doc.scene);
    let hole = e.id("Hole A");
    let mut moved = e.doc.scene.get(hole).unwrap().data.clone();
    moved.transform.position.z = 30.0;
    e.run([EditorCommand::Edit {
        edit: SceneEdit::SetData { id: hole, data: moved },
        label: "Move",
    }]);
    let clock = Instant::now();
    evaluated.update(&e.doc.scene);
    let took = clock.elapsed();
    assert!(took < Duration::from_secs(2), "{took:?} (an unoptimised build)");
    let after = grid4d_eval::generator_mesh(&e.doc.scene, boolean, 0.0).unwrap();
    assert_ne!(after.points(), result.points(), "the result follows the cutter");
    assert!(closed(&after), "still closed after the move");

    // 3. Half a handle completed by a Symmetry: welded, closed.
    let mut half = primitives::cube(Vec3::splat(40.0), [2, 2, 2]);
    for p in half.points_mut() {
        p.x += 20.0;
    }
    let kept: Vec<Vec<u32>> = (0..half.face_count())
        .filter(|&f| half.face_center(f).x > 1e-3)
        .map(|f| half.face(f).to_vec())
        .collect();
    let half = PolyMesh::from_faces(half.points().to_vec(), kept).unwrap();
    let symmetry = e.insert(
        None,
        node(
            "Symmetry",
            ObjectKind::Symmetry(grid4d_geom::symmetry::Symmetry::default()),
            Vec3::new(0.0, 120.0, 0.0),
        ),
    );
    let handle = e.insert(Some(symmetry), node("Handle", polygon(half), Vec3::ZERO));
    let whole = grid4d_eval::generator_mesh(&e.doc.scene, symmetry, 0.0).unwrap();
    assert!(closed(&whole), "welded at the plane: no seam");
    // Its top extruded along a spline.
    let mut arc = ObjectKindTag::Arc.default_kind();
    if let ObjectKind::SplinePrimitive(primitive) = &mut arc
        && let grid4d_geom::spline::SplineShape::Arc { radius, .. } = &mut primitive.shape
    {
        *radius = 30.0;
    }
    let path = e.insert(None, node("Path", arc, Vec3::new(30.0, 140.0, 0.0)));
    let top = {
        let mesh = e.mesh(handle);
        let mesh = mesh.mesh();
        (0..mesh.face_count())
            .find(|&f| mesh.face_normal(f).y > 0.9 && mesh.face_center(f).x > 25.0)
            .unwrap()
    };
    let faces = e.mesh(handle).mesh().face_count();
    e.editor.selection.set_single(handle);
    e.editor.mode = EditMode::Polygons;
    e.run([EditorCommand::SelectComponents {
        node: handle,
        component: Component::Polygon,
        hits: vec![u32::try_from(top).unwrap()],
        mode: SelectMode::Replace,
    }]);
    e.editor.modeling_settings.along_spline = Some(path);
    e.apply(handle, Tool::ExtrudeAlongSpline, EditMode::Polygons);
    assert!(e.mesh(handle).mesh().face_count() > faces, "extruded along the path");

    // 4. A box creased under a Subdivision Surface: its corners stay sharp.
    let sds = e.insert(
        None,
        node(
            "Subdivision Surface",
            ObjectKindTag::SubdivisionSurface.default_kind(),
            Vec3::new(400.0, 0.0, 0.0),
        ),
    );
    let cage = e.insert(
        Some(sds),
        node(
            "Cage",
            polygon(primitives::cube(Vec3::splat(100.0), [1, 1, 1])),
            Vec3::ZERO,
        ),
    );
    e.editor.modeling_settings.crease = 1.0;
    e.select_all(cage, EditMode::Edges);
    e.apply(cage, Tool::Crease, EditMode::Edges);
    let smooth = grid4d_eval::generator_mesh(&e.doc.scene, sds, 0.0).unwrap();
    let corner = smooth
        .points()
        .iter()
        .fold(0.0f32, |m, p| m.max(p.x.abs().min(p.y.abs()).min(p.z.abs())));
    assert!((corner - 50.0).abs() < 1e-3, "a corner at the cube's own: {corner}");

    // 5. A dent pushed into a panel with soft selection.
    let panel = e.insert(
        None,
        node(
            "Panel",
            polygon(primitives::plane(200.0, 200.0, 20, 20)),
            Vec3::new(-400.0, 0.0, 0.0),
        ),
    );
    let flat = e.mesh(panel);
    let centre = flat.mesh().points().iter().position(|p| p.length() < 1e-3).unwrap();
    let mut selected = BitSet::new();
    selected.insert(centre);
    let settings = SoftSelection {
        enabled: true,
        radius: 60.0,
        ..SoftSelection::default()
    };
    let mut weights = Vec::new();
    soft_weights(
        flat.mesh().points(),
        &selected,
        &BitSet::new(),
        None,
        &settings,
        &mut SoftScratch::default(),
        &mut weights,
    );
    let dented: Vec<Vec3> = flat
        .mesh()
        .points()
        .iter()
        .zip(&weights)
        .map(|(p, w)| *p - Vec3::Y * 20.0 * w)
        .collect();
    let mut data = e.doc.scene.get(panel).unwrap().data.clone();
    data.kind = ObjectKind::polygon(flat.with_points(&dented));
    e.run([EditorCommand::Edit {
        edit: SceneEdit::SetData { id: panel, data },
        label: "Move",
    }]);
    let dent = e.mesh(panel);
    let depth = |at: Vec3| {
        let points = dent.mesh().points();
        let (i, _) = flat
            .mesh()
            .points()
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.distance(at).total_cmp(&b.1.distance(at)))
            .unwrap();
        -points[i].y
    };
    assert!((depth(Vec3::ZERO) - 20.0).abs() < 1e-4, "the centre goes all the way");
    let (near, far) = (depth(Vec3::new(30.0, 0.0, 0.0)), depth(Vec3::new(90.0, 0.0, 0.0)));
    assert!(
        near > 0.0 && near < 20.0 && far.abs() < 1e-6,
        "a smooth dent: {near}, {far}"
    );

    // 6. An imported scan reduced to 10 %, keeping its outline and UVs.
    let mut scan = primitives::plane(400.0, 400.0, 40, 40);
    for p in scan.points_mut() {
        p.y = 10.0 * (p.x * 0.05).sin() * (p.z * 0.04).cos();
    }
    let scan_file = dir.join("scan.obj");
    let mut scan_doc = Document::new("Scan");
    scan_doc.scene.insert(Node::new("Scan", polygon(scan.clone())));
    grid4d_io::export(&grid4d_ui::file::exportable(&scan_doc), &scan_file).unwrap();
    e.run([EditorCommand::MergeFile(scan_file)]);
    let scanned = e.id("Scan");
    let reduce = e.insert(
        None,
        node(
            "Reduce",
            ObjectKind::Reduce(Reduce {
                keep: 0.1,
                ..Reduce::default()
            }),
            Vec3::ZERO,
        ),
    );
    e.run([EditorCommand::Reparent {
        node: scanned,
        new_parent: Some(reduce),
    }]);
    let lighter = grid4d_eval::generator_mesh(&e.doc.scene, reduce, 0.0).unwrap();
    let triangles = |m: &PolyMesh| (0..m.face_count()).map(|f| m.face(f).len() - 2).sum::<usize>();
    let (full, kept) = (triangles(&scan), triangles(&lighter));
    assert!(kept <= full / 10 + 8, "{kept} of {full} triangles");
    assert!(lighter.uvs().is_some(), "UVs kept");
    let outline = |m: &PolyMesh| {
        let xs = m.points().iter().map(|p| p.x);
        let zs = m.points().iter().map(|p| p.z);
        (
            xs.clone().fold(f32::MAX, f32::min),
            xs.fold(f32::MIN, f32::max),
            zs.fold(f32::MAX, f32::min),
        )
    };
    let (a, b) = (outline(&scan), outline(&lighter));
    assert!(
        (a.0 - b.0).abs() < 1e-2 && (a.1 - b.1).abs() < 1e-2 && (a.2 - b.2).abs() < 1e-2,
        "{a:?} {b:?}"
    );

    // 7. A label wrapped round a bottle.
    let radius = 50.0;
    e.insert(
        None,
        node(
            "Bottle",
            ObjectKind::Cylinder {
                radius,
                height: 260.0,
                rotation_segments: 48,
                height_segments: 1,
            },
            Vec3::new(0.0, 0.0, 400.0),
        ),
    );
    let label = e.insert(
        None,
        node(
            "Label",
            ObjectKind::Plane {
                width: TAU * radius,
                depth: 100.0,
                width_segments: 48,
                depth_segments: 4,
            },
            Vec3::new(0.0, 0.0, 400.0),
        ),
    );
    let mut wrap = node(
        "Wrap",
        ObjectKind::Wrap(Wrap {
            width: TAU * radius,
            height: 100.0,
            radius,
            start: -PI,
            end: PI,
            ..Wrap::default()
        }),
        Vec3::ZERO,
    );
    // Turned so its X and Y lie in the label: the label rolls round Z.
    wrap.transform.rotation.y = FRAC_PI_2;
    e.insert(Some(label), wrap);

    // 8. A decal projected onto the shell.
    let decal = e.insert(
        None,
        node(
            "Decal",
            ObjectKind::Plane {
                width: 60.0,
                depth: 60.0,
                width_segments: 6,
                depth_segments: 6,
            },
            Vec3::new(0.0, 150.0, 0.0),
        ),
    );
    e.insert(
        Some(decal),
        node(
            "Project to Surface",
            ObjectKind::ProjectToSurface {
                project: ProjectToSurface::default(),
                target: Some(shell),
            },
            Vec3::ZERO,
        ),
    );

    // 9. Weighted normals on the shell.
    e.editor.selection.set_single(shell);
    e.run([EditorCommand::AddTag(Tag::Phong {
        angle_limit: true,
        angle: 30f32.to_radians(),
        weighted: true,
    })]);
    let tags = &e.doc.scene.get(shell).unwrap().data.tags;
    assert!(matches!(tags.smoothing(), grid4d_geom::Smoothing::Weighted(_)));

    let check_deformers = |doc: &Document| {
        let mut evaluated = grid4d_eval::EvaluatedScene::new();
        evaluated.update(&doc.scene);
        let wrapped = evaluated.deformed(label).unwrap();
        for p in wrapped.mesh().points() {
            assert!((p.truncate().length() - radius).abs() < 1e-3, "on the bottle: {p}");
        }
        let projected = evaluated.deformed(decal).unwrap();
        let ObjectKind::Polygon { mesh: target, .. } = &doc.scene.get(shell).unwrap().data.kind else {
            panic!("the shell")
        };
        for p in projected.mesh().points() {
            let world = *p + Vec3::new(0.0, 150.0, 0.0);
            let (on, _) = target.nearest(world).unwrap();
            assert!(on.distance(world) < 1e-2, "on the shell: {world}");
        }
    };
    check_deformers(&e.doc);

    // 10. Every step undoes and redoes.
    let end = grid4d_io::to_bytes(&e.doc).unwrap();
    let mut steps = 0;
    while e.editor.history.can_undo() {
        e.run([EditorCommand::Undo]);
        steps += 1;
    }
    assert!(steps >= 15, "{steps} steps");
    assert_eq!(grid4d_io::to_bytes(&e.doc).unwrap(), start, "undone to the start");
    while e.editor.history.can_redo() {
        e.run([EditorCommand::Redo]);
    }
    assert_eq!(grid4d_io::to_bytes(&e.doc).unwrap(), end, "redone to the end");
    check_deformers(&e.doc);

    // 11. Saved as version 6, it opens the same.
    let path = dir.join("shell.g4d");
    grid4d_io::save(&e.doc, &path).unwrap();
    let again = grid4d_io::load(&path).unwrap();
    assert_eq!(grid4d_io::to_bytes(&again).unwrap(), end);
    assert_eq!(grid4d_io::FORMAT_VERSION, 6);

    // 12. The scene renders.
    let look = glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 300.0, -700.0), Vec3::ZERO, Vec3::Y);
    let camera = e.insert(
        None,
        Node::new(
            "Camera",
            ObjectKind::Camera {
                fov_y_deg: 50.0,
                lens: grid4d_scene::Lens::default(),
            },
        )
        .with_transform(Transform::from_matrix(&look.inverse()))
        .data,
    );
    (
        e.doc.render.width,
        e.doc.render.height,
        e.doc.render.samples,
        e.doc.render.save,
    ) = (48, 27, 2, false);
    e.doc.render.adaptive = false;
    let replay = e.editor.simulation.replay(&e.doc);
    let number = e.editor.renders.start(
        &e.doc,
        Default::default(),
        &OrbitCamera::default(),
        Some(camera),
        None,
        replay,
    );
    e.wait();
    let picture = e.editor.renders.picture(number).unwrap();
    assert_eq!(picture.state, grid4d_ui::rendering::RenderState::Finished);
    let (first, rest) = picture.rgb.split_at(3);
    assert!(rest.chunks(3).any(|p| p != first), "something is in the picture");
}
