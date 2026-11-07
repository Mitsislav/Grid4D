//! Long random sequences of editor commands on the demo scene, with undo and
//! redo mixed in, the way a user would click — through
//! [`commands::apply_pending`], as the application applies them. Checked
//! after every command and at the end:
//!
//! * nothing panics;
//! * every selected object and the linked camera are objects of the
//!   document; a selected tag is there when it is read;
//! * a history state always shows the same document: reaching a state again
//!   by undo or redo gives exactly the document recorded for it, so nothing
//!   changes the document without an undo step (temporary values, which a
//!   change of frame drops, aside);
//! * undoing everything gives back the start, redoing everything the end,
//!   and undoing again the start once more.

use std::collections::HashMap;

use glam::Vec3;
use grid4d_geom::Component;
use grid4d_scene::{
    Document, LayerId, Layers, Materials, NodeData, NodeId, ObjectKind, ObjectKindTag, RenderSettings, SceneEdit,
    SimulationSettings, TagKind, Takes, TimeSettings,
};

use crate::commands::{self, EditorCommand, SelectMode, SplineOp};
use crate::editor::{CurveKey, EditMode, EditorState, Tool};
use crate::keys::CurveOp;
use crate::panels::ViewportPanel;

/// The test's own small random stream (xorshift), so a seed always gives the
/// same sequence.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self((seed ^ 0x9E37_79B9_7F4A_7C15).max(1))
    }

    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % u64::try_from(n.max(1)).unwrap()).unwrap()
    }

    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        let t = f32::from(u16::try_from(self.below(65_536)).unwrap()) / 65_535.0;
        lo + (hi - lo) * t
    }

    fn vec(&mut self, size: f32) -> Vec3 {
        Vec3::new(
            self.range(-size, size),
            self.range(-size, size),
            self.range(-size, size),
        )
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> Option<T> {
        (!items.is_empty()).then(|| items[self.below(items.len())])
    }

    fn index(&mut self, count: usize) -> u32 {
        u32::try_from(self.below(count)).unwrap()
    }
}

/// Everything about the document an undo step may change.
#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    name: String,
    time: TimeSettings,
    materials: Materials,
    layers: Layers,
    takes: Takes,
    render: RenderSettings,
    simulation: SimulationSettings,
    /// Pre-order: id, parent, depth, data.
    nodes: Vec<(NodeId, Option<NodeId>, u32, NodeData)>,
}

fn snapshot(doc: &Document) -> Snapshot {
    let mut nodes = Vec::new();
    doc.scene.walk(&mut Vec::new(), |id, node, _, depth| {
        // Temporary values are dropped when the frame changes, by design
        // not an undo step.
        let mut data = node.data.clone();
        data.unkeyed = None;
        nodes.push((id, node.parent(), depth, data));
        true
    });
    Snapshot {
        name: doc.name.clone(),
        time: doc.time.settings(),
        materials: doc.scene.materials().clone(),
        layers: doc.scene.layers().clone(),
        takes: doc.takes.clone(),
        render: doc.render.clone(),
        simulation: doc.simulation,
        nodes,
    }
}

/// The active object's mesh sizes when it is a polygon object: points,
/// edges, polygons.
fn polygon_counts(doc: &Document, id: NodeId) -> Option<[usize; 3]> {
    let ObjectKind::Polygon { mesh, .. } = &doc.scene.get(id)?.data.kind else {
        return None;
    };
    Some([
        mesh.mesh().point_count(),
        mesh.topology().edge_count(),
        mesh.mesh().face_count(),
    ])
}

const SCRIPTS: [&str; 3] = [
    "let group = create(\"Null\");\nfor i in 0..3 {\n    let cube = create(\"Cube\", group);\n    cube.position = [i * 120, 0, 0];\n}",
    "let sphere = find(\"Sphere\");\nif sphere != () {\n    sphere.position = [0, 150, 0];\n    sphere.scale = [1, 1.5, 1];\n}",
    "let x = find(\"Nothing here\");\nx.position = [1, 2, 3];",
];

/// One random command for the current state, like a click somewhere in the
/// interface. Editor state a click would change first (the mode, the tool,
/// the timeline's key selection) is changed here.
#[expect(clippy::too_many_lines, reason = "one arm per kind of command")]
fn random_command(rng: &mut Rng, doc: &mut Document, editor: &mut EditorState) -> EditorCommand {
    let ids: Vec<NodeId> = doc.scene.iter().map(|(id, _)| id).collect();
    let any = |rng: &mut Rng| rng.pick(&ids);
    let active = editor.selection.active();
    let tag = |rng: &mut Rng| ObjectKindTag::ALL[rng.below(ObjectKindTag::ALL.len())];
    let materials = doc.scene.materials().ids().to_vec();
    let layers: Vec<LayerId> = doc.scene.layers().ids().to_vec();
    let takes = doc.takes.ids().to_vec();
    let mode = |rng: &mut Rng| match rng.below(4) {
        0 => SelectMode::Replace,
        1 => SelectMode::Toggle,
        2 => SelectMode::Add,
        _ => SelectMode::Remove,
    };

    match rng.below(53) {
        0..=2 => EditorCommand::CreateObject(tag(rng)),
        3 => EditorCommand::CreateObject(ObjectKindTag::Cube),
        4 => EditorCommand::CreateChild(tag(rng)),
        5 => EditorCommand::CreatePlugin {
            type_id: if rng.chance(2) { "grid4d.gear" } else { "grid4d.wave" }.to_owned(),
            child: rng.chance(2),
        },
        6..=8 => match any(rng) {
            Some(id) => EditorCommand::Select { id, mode: mode(rng) },
            None => EditorCommand::SelectAll,
        },
        9 => {
            if rng.chance(3) {
                EditorCommand::DeselectAll
            } else {
                EditorCommand::SelectAll
            }
        }
        10 => EditorCommand::DeleteSelected,
        11 => match any(rng) {
            Some(node) => EditorCommand::Reparent {
                node,
                new_parent: if rng.chance(3) { None } else { any(rng) },
            },
            None => EditorCommand::NewMaterial,
        },
        12 => EditorCommand::MakeEditable,
        13 => EditorCommand::CurrentStateToObject,
        14 => {
            let generators = ObjectKindTag::GENERATORS;
            EditorCommand::WrapInGenerator(generators[rng.below(generators.len())])
        }
        15 => EditorCommand::ToggleParentGenerator,
        16 => match any(rng) {
            Some(id) => EditorCommand::FitToParent(id),
            None => EditorCommand::ResetView,
        },
        // A rename or a toggle in a panel.
        17 => match any(rng).and_then(|id| Some((id, doc.scene.get(id)?.data.clone()))) {
            Some((id, mut data)) => {
                match rng.below(3) {
                    0 => data.name = format!("Renamed {}", rng.below(100)),
                    1 => data.visible_in_editor = !data.visible_in_editor,
                    _ => data.enabled = !data.enabled,
                }
                EditorCommand::Edit {
                    edit: SceneEdit::SetData { id, data },
                    label: "Rename",
                }
            }
            None => EditorCommand::NewTake,
        },
        // A gizmo drag: written live, then committed.
        18 => match any(rng).and_then(|id| Some((id, doc.scene.get(id)?.data.clone()))) {
            Some((id, before)) => {
                let frame = f64::from(doc.time.frame());
                if let Some(node) = doc.scene.get_mut(id) {
                    let mut shown = node.data.transform_at(frame);
                    shown.position += rng.vec(200.0);
                    shown.scale *= rng.range(0.5, 2.0);
                    node.data.set_transform_at(&shown, frame, editor.animation.autokey);
                }
                EditorCommand::CommitLive {
                    inverse: SceneEdit::SetData { id, data: before },
                    label: "Move",
                }
            }
            None => EditorCommand::FrameScene,
        },
        19 => EditorCommand::RecordKeys {
            at: i32::try_from(rng.below(40)).unwrap(),
        },
        20 => {
            editor.keys.frames = vec![f64::from(u8::try_from(rng.below(40)).unwrap())];
            if rng.chance(2) {
                EditorCommand::MoveKeys {
                    by: i32::try_from(rng.below(20)).unwrap() - 10,
                }
            } else {
                EditorCommand::DeleteKeys
            }
        }
        21 => {
            // Every key of the active object's first track, in the F-Curve editor.
            editor.curves.keys.clear();
            if let Some(node) = active
                && let Some(track) = doc.scene.get(node).and_then(|n| n.data.tracks.first())
            {
                for key in track.curve.keys() {
                    editor.curves.insert(CurveKey {
                        node,
                        param: track.param,
                        frame: key.frame,
                    });
                }
            }
            EditorCommand::EditCurveKeys {
                op: if rng.chance(2) {
                    CurveOp::Delete
                } else {
                    CurveOp::Interpolation(grid4d_scene::Interpolation::Linear)
                },
            }
        }
        22 => match rng.pick(&materials) {
            Some(material) if rng.chance(2) => EditorCommand::AssignMaterial {
                material,
                nodes: editor.selection.as_slice().to_vec(),
                layer: rng.chance(3),
            },
            Some(material) if rng.chance(3) => EditorCommand::DeleteMaterial(material),
            _ => EditorCommand::NewMaterial,
        },
        23 => EditorCommand::AddTag(TagKind::ALL[rng.below(TagKind::ALL.len())].default_tag()),
        24 => match active {
            Some(node) if rng.chance(4) => EditorCommand::BakeMaterialTag(crate::TagRef {
                node,
                kind: TagKind::Material,
                nth: rng.below(2),
            }),
            Some(node) => EditorCommand::DeleteTag(crate::TagRef {
                node,
                kind: TagKind::ALL[rng.below(TagKind::ALL.len())],
                nth: rng.below(2),
            }),
            None => EditorCommand::DeselectAll,
        },
        25 => {
            if layers.is_empty() || rng.chance(3) {
                let mut table = doc.scene.layers().clone();
                table.insert(table.next_layer("Layer"));
                EditorCommand::Edit {
                    edit: SceneEdit::SetLayers(table),
                    label: "New Layer",
                }
            } else if rng.chance(3) {
                EditorCommand::SelectLayerObjects(layers[rng.below(layers.len())])
            } else {
                EditorCommand::AssignLayer {
                    layer: rng.pick(&layers).filter(|_| !rng.chance(4)),
                    objects: editor.selection.as_slice().to_vec(),
                }
            }
        }
        26 => match rng.below(4) {
            0 => EditorCommand::NewTake,
            1 => EditorCommand::SwitchTake(rng.pick(&takes).filter(|_| !rng.chance(3))),
            2 => match rng.pick(&takes) {
                Some(take) => EditorCommand::DeleteTake(take),
                None => EditorCommand::NewTake,
            },
            _ => match rng.pick(&takes) {
                Some(take) => EditorCommand::ClearOverride {
                    take,
                    node: any(rng),
                    group: Some(grid4d_scene::OverrideGroup::Transform),
                },
                None => EditorCommand::SwitchTake(None),
            },
        },
        27 => EditorCommand::RunScript(SCRIPTS[rng.below(SCRIPTS.len())].to_owned()),
        28 => match grid4d_plugin::registry().command("grid4d.scatter") {
            Some(plugin) => EditorCommand::RunPlugin {
                type_id: "grid4d.scatter".to_owned(),
                options: editor.plugin_commands.options_mut(plugin).clone(),
            },
            None => EditorCommand::SelectAll,
        },
        29 => {
            editor.tool = Tool::Pen;
            match rng.below(5) {
                0 => EditorCommand::PenClose,
                1 => EditorCommand::PenFinish,
                _ => EditorCommand::PenPoint {
                    position: rng.vec(300.0),
                    tangent: if rng.chance(2) { Vec3::ZERO } else { rng.vec(50.0) },
                },
            }
        }
        30 => EditorCommand::SplineCommand(SplineOp::ALL[rng.below(SplineOp::ALL.len())]),
        31 => {
            editor.mode = EditMode::ALL[rng.below(EditMode::ALL.len())];
            EditorCommand::ConvertSelection {
                to: EditMode::ALL[rng.below(EditMode::ALL.len())],
                tolerant: rng.chance(2),
            }
        }
        // The component modes, on the active polygon object.
        32..=46 => {
            if editor.mode == EditMode::Model {
                editor.mode = EditMode::ALL[1 + rng.below(3)];
            }
            let component = editor.mode.component().unwrap_or(Component::Point);
            let Some((node, [points, edges, faces])) = active.and_then(|id| Some((id, polygon_counts(doc, id)?)))
            else {
                // Make a polygon object the active one first (or one, if
                // there is none).
                let polygons: Vec<NodeId> = ids
                    .iter()
                    .copied()
                    .filter(|&id| polygon_counts(doc, id).is_some())
                    .collect();
                return match rng.pick(&polygons) {
                    Some(id) => EditorCommand::Select {
                        id,
                        mode: SelectMode::Replace,
                    },
                    None => EditorCommand::MakeEditable,
                };
            };
            let count = match component {
                Component::Point => points,
                Component::Edge => edges,
                Component::Polygon => faces,
            };
            match rng.below(17) {
                0..=3 => EditorCommand::SelectComponents {
                    node,
                    component,
                    hits: (0..1 + rng.below(4)).map(|_| rng.index(count)).collect(),
                    mode: mode(rng),
                },
                4 => EditorCommand::GrowSelection,
                5 => EditorCommand::ShrinkSelection,
                6 => [
                    EditorCommand::InvertSelection,
                    EditorCommand::SelectConnected,
                    EditorCommand::HideSelected,
                    EditorCommand::HideUnselected,
                    EditorCommand::UnhideAll,
                ][rng.below(5)]
                .clone(),
                7 => EditorCommand::MeltSelected,
                8 => EditorCommand::DissolveSelected,
                9 => EditorCommand::ConnectSelected,
                10 => EditorCommand::LoopCut {
                    node,
                    edge: rng.index(edges),
                    fractions: vec![rng.range(0.1, 0.9)],
                },
                11 => EditorCommand::LineCut {
                    node,
                    eye: Vec3::new(0.0, 0.0, 1000.0),
                    from: Vec3::new(rng.range(-0.3, 0.3), -1.0, -3.0).normalize(),
                    to: Vec3::new(rng.range(-0.3, 0.3), 1.0, -3.0).normalize(),
                },
                12 => EditorCommand::Bridge {
                    node,
                    component,
                    from: rng.index(count),
                    to: rng.index(count),
                },
                13 => {
                    if rng.chance(2) {
                        EditorCommand::BridgeSelected
                    } else {
                        EditorCommand::Weld {
                            node,
                            onto: rng.chance(2).then(|| rng.index(points)),
                        }
                    }
                }
                14 => EditorCommand::Optimize,
                15 => {
                    use crate::uv_tools::UvOp;
                    use grid4d_geom::uv::{FlipAxis, Projection};
                    let settings = &mut editor.uv_settings;
                    settings.relax.iterations = 1 + u32::try_from(rng.below(5)).unwrap();
                    settings.relax.pin_borders = rng.chance(2);
                    settings.rotate = rng.chance(2);
                    let op = [
                        UvOp::Unwrap,
                        UvOp::Relax,
                        UvOp::Pack,
                        UvOp::Project(Projection::ALL[rng.below(Projection::ALL.len())]),
                        UvOp::Flip(if rng.chance(2) { FlipAxis::U } else { FlipAxis::V }),
                        UvOp::MarkSeams,
                        UvOp::ClearSeams,
                    ][rng.below(7)];
                    EditorCommand::UvCommand(op)
                }
                _ => {
                    editor.tool = [Tool::Extrude, Tool::Inset, Tool::Bevel][rng.below(3)];
                    let settings = &mut editor.modeling_settings;
                    settings.extrude.offset = rng.range(-50.0, 50.0);
                    settings.inset.offset = rng.range(0.0, 20.0);
                    settings.bevel.offset = rng.range(0.0, 20.0);
                    EditorCommand::ApplyMeshTool
                }
            }
        }
        51 => {
            // The Texture Manager: pictures that need not exist (embedding a
            // missing one is an alert and changes nothing).
            let textures = doc.scene.textures().ids().to_vec();
            match (rng.pick(&textures), rng.below(4)) {
                (Some(id), 0) => EditorCommand::DeleteTexture(id),
                (Some(id), 1) => EditorCommand::EmbedTexture(id),
                (_, 2) => EditorCommand::RemoveUnusedTextures,
                _ => {
                    EditorCommand::AddTextureFiles(vec![std::path::PathBuf::from(format!("maps/{}.png", rng.below(5)))])
                }
            }
        }
        47 => EditorCommand::Undo,
        48 => EditorCommand::Redo,
        // A click on the timeline: temporary values give way to the curves.
        49 | 50 => {
            doc.time.set_frame(i32::try_from(rng.below(60)).unwrap());
            EditorCommand::RecordKeys { at: doc.time.frame() }
        }
        _ => {
            editor.mode = EditMode::Model;
            [
                EditorCommand::FrameSelected,
                EditorCommand::ShowView(None),
                EditorCommand::ShowView(Some(rng.below(4))),
                EditorCommand::ResetView,
            ][rng.below(4)]
            .clone()
        }
    }
}

/// The invariants that hold after every command.
fn check(doc: &Document, editor: &EditorState, states: &mut HashMap<u64, Snapshot>, context: &str) {
    for &id in editor.selection.as_slice() {
        assert!(
            doc.scene.contains(id),
            "{context}: a selected object is not in the document"
        );
    }
    // The selected tag may outlive its object (an undo brings both back); it
    // is only ever read through `selected_tag`, which must check.
    if let Some((tag, index)) = editor.selected_tag(doc) {
        let tags = doc.scene.get(tag.node).map(|node| node.data.tags.as_slice());
        assert!(
            tags.is_some_and(|tags| index < tags.len()),
            "{context}: the selected tag is not there"
        );
    }
    if let Some(camera) = editor.view_camera {
        assert!(doc.scene.contains(camera), "{context}: the linked camera is gone");
    }
    let now = snapshot(doc);
    match states.get(&editor.history.state_id()) {
        Some(seen) => assert!(
            *seen == now,
            "{context}: history state {} shows another document than before",
            editor.history.state_id()
        ),
        None => {
            states.insert(editor.history.state_id(), now);
        }
    }
}

fn apply(doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel, command: EditorCommand) {
    editor.commands.push(command);
    commands::apply_pending(doc, editor, viewport);
}

#[test]
fn random_command_sequences_keep_every_id_valid_and_undo_redo_exactly() {
    crate::test_plugins();
    for seed in 1..=12u64 {
        let mut rng = Rng::new(seed);
        let mut doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        editor.preferences.undo_depth = 10_000;
        let mut viewport = ViewportPanel::new(None);
        let start = snapshot(&doc);
        let mut states = HashMap::new();
        check(&doc, &editor, &mut states, "the start");

        let mut steps = 0;
        for k in 0..400 {
            let command = random_command(&mut rng, &mut doc, &mut editor);
            let context = format!("seed {seed}, command {k}: {command:?}");
            let before = editor.history.state_id();
            apply(&mut doc, &mut editor, &mut viewport, command);
            steps += usize::from(editor.history.state_id() != before);
            check(&doc, &editor, &mut states, &context);
        }
        assert!(steps > 100, "seed {seed}: only {steps} commands changed the document");
        // The sequence may end in undos: the end is the newest state.
        while editor.history.can_redo() {
            apply(&mut doc, &mut editor, &mut viewport, EditorCommand::Redo);
            check(&doc, &editor, &mut states, &format!("seed {seed}: redo to the end"));
        }
        let end = snapshot(&doc);

        let mut undone = 0;
        while editor.history.can_undo() {
            apply(&mut doc, &mut editor, &mut viewport, EditorCommand::Undo);
            check(&doc, &editor, &mut states, &format!("seed {seed}: undo {undone}"));
            undone += 1;
        }
        assert!(
            snapshot(&doc) == start,
            "seed {seed}: undoing everything gives back the start"
        );
        while editor.history.can_redo() {
            apply(&mut doc, &mut editor, &mut viewport, EditorCommand::Redo);
            check(&doc, &editor, &mut states, &format!("seed {seed}: redo"));
        }
        assert!(
            snapshot(&doc) == end,
            "seed {seed}: redoing everything gives back the end"
        );
        while editor.history.can_undo() {
            apply(&mut doc, &mut editor, &mut viewport, EditorCommand::Undo);
        }
        assert!(snapshot(&doc) == start, "seed {seed}: and undoing again the start");
    }
}
