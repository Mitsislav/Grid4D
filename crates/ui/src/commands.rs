//! Structural editor operations, queued by panels and applied once per frame.
//!
//! This is the single choke point for undo/redo: every document change a
//! command makes is applied as [`SceneEdit`]s and recorded as one undo step in
//! [`EditorState::history`]. Selection and view changes are not undo steps, but
//! each step restores the selection it was made with.

use glam::Vec3;
use oc4d_render::OrbitCamera;
use oc4d_scene::{Document, Node, NodeData, NodeId, ObjectKindTag, Scene, SceneEdit, Transform};

use crate::{EditorState, Selection, ViewportPanel, theme::palette};

/// How a click changes the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    /// Plain click.
    Replace,
    /// Ctrl/Cmd + click.
    Toggle,
    /// Shift + click.
    Add,
}

impl SelectMode {
    /// Maps keyboard modifiers to a selection mode, C4D-style.
    #[must_use]
    pub fn from_modifiers(m: egui::Modifiers) -> Self {
        if m.command {
            Self::Toggle
        } else if m.shift {
            Self::Add
        } else {
            Self::Replace
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    /// Replaces the document with an empty one and clears the history.
    NewDocument,
    CreateObject(ObjectKindTag),
    DeleteSelected,
    Select {
        id: NodeId,
        mode: SelectMode,
    },
    SelectAll,
    DeselectAll,
    /// Moves `node` under `new_parent` (last child), keeping its world placement.
    Reparent {
        node: NodeId,
        new_parent: Option<NodeId>,
    },
    Undo,
    Redo,
    /// Records an edit that was already written live (a gizmo drag, a value
    /// field): `before` is the node's data when the interaction started. Nothing
    /// is recorded if the data did not change.
    CommitData {
        id: NodeId,
        before: NodeData,
        label: &'static str,
    },
    FrameScene,
    FrameSelected,
    ResetView,
}

/// FIFO of pending commands. Draining keeps the vector's capacity.
#[derive(Debug, Default)]
pub struct CommandQueue {
    pending: Vec<EditorCommand>,
}

impl CommandQueue {
    pub fn push(&mut self, command: EditorCommand) {
        self.pending.push(command);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Applies every queued command. Returns `true` if anything was applied, so the
/// caller can request a repaint and show the result immediately.
pub fn apply_pending(doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel) -> bool {
    if editor.commands.is_empty() {
        return false;
    }
    let mut pending = std::mem::take(&mut editor.commands.pending);
    for command in pending.drain(..) {
        apply(command, doc, editor, viewport);
    }
    // Hand the (now empty) buffer back so its capacity is reused next frame.
    editor.commands.pending = pending;
    true
}

fn apply(command: EditorCommand, doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel) {
    match command {
        EditorCommand::NewDocument => {
            editor.history.clear(doc);
            *doc = Document::new("Untitled 1");
            editor.selection.clear();
            viewport.reset_view();
        }
        EditorCommand::CreateObject(tag) => {
            let edit = SceneEdit::Insert {
                parent: None,
                index: usize::MAX,
                data: new_object_data(tag, &doc.scene, viewport.camera()),
            };
            commit(doc, editor, "Create Object", vec![edit], |inverses, selection| {
                // The inverse of an insert is the detach of the new node.
                if let Some(id) = inverses.first().and_then(SceneEdit::node) {
                    selection.set_single(id);
                }
            });
        }
        EditorCommand::DeleteSelected => {
            // Detaching a node takes its descendants along, so only the topmost
            // selected nodes are detached.
            let edits = editor
                .selection
                .as_slice()
                .iter()
                .filter(|&&id| !has_selected_ancestor(&doc.scene, &editor.selection, id))
                .map(|&root| SceneEdit::Detach { root })
                .collect();
            commit(doc, editor, "Delete", edits, |_, selection| selection.clear());
        }
        EditorCommand::Select { id, mode } => {
            if doc.scene.contains(id) {
                match mode {
                    SelectMode::Replace => editor.selection.set_single(id),
                    SelectMode::Toggle => editor.selection.toggle(id),
                    SelectMode::Add => editor.selection.add(id),
                }
            }
        }
        EditorCommand::SelectAll => {
            let mut ids = Vec::with_capacity(doc.scene.len());
            let mut stack = Vec::new();
            doc.scene.walk(&mut stack, |id, _, _, _| {
                ids.push(id);
                true
            });
            editor.selection.set_all(ids);
        }
        EditorCommand::DeselectAll => editor.selection.clear(),
        EditorCommand::Reparent { node, new_parent } => {
            let current_parent = doc.scene.get(node).map(Node::parent);
            if current_parent.is_some_and(|parent| parent != new_parent)
                && let Some(transform) = doc.scene.transform_keeping_world(node, new_parent)
            {
                let edit = SceneEdit::Move {
                    id: node,
                    parent: new_parent,
                    index: usize::MAX,
                    transform,
                };
                commit(doc, editor, "Reparent", vec![edit], |_, _| {});
            }
        }
        EditorCommand::Undo => {
            editor.history.undo(doc, &mut editor.selection);
        }
        EditorCommand::Redo => {
            editor.history.redo(doc, &mut editor.selection);
        }
        EditorCommand::CommitData { id, before, label } => {
            if doc.scene.get(id).is_some_and(|node| node.data != before) {
                let selection = editor.selection.clone();
                let inverse = SceneEdit::SetData { id, data: before };
                editor
                    .history
                    .record(doc, label, vec![inverse], selection.clone(), selection);
            }
        }
        EditorCommand::FrameScene => viewport.frame_scene(&doc.scene),
        EditorCommand::FrameSelected => viewport.frame_selection(&doc.scene, &editor.selection),
        EditorCommand::ResetView => viewport.reset_view(),
    }
    editor.selection.retain_existing(&doc.scene);
}

/// Applies `edits` as one undo step named `label`, all or nothing.
///
/// `select` updates the selection from the inverses (e.g. to select a created
/// object); the step remembers the selection before and after.
fn commit(
    doc: &mut Document,
    editor: &mut EditorState,
    label: &'static str,
    edits: Vec<SceneEdit>,
    select: impl FnOnce(&[SceneEdit], &mut Selection),
) {
    if edits.is_empty() {
        return;
    }
    let selection_before = editor.selection.clone();
    match doc.apply_all(edits) {
        Ok(inverses) => {
            select(&inverses, &mut editor.selection);
            editor.selection.retain_existing(&doc.scene);
            let selection_after = editor.selection.clone();
            editor
                .history
                .record(doc, label, inverses, selection_before, selection_after);
        }
        Err(err) => log::warn!("{label} rejected: {err}"),
    }
}

/// Default data for a new object, named uniquely, C4D-style.
fn new_object_data(tag: ObjectKindTag, scene: &Scene, camera: &OrbitCamera) -> NodeData {
    let mut data = NodeData::new(scene.unique_name(tag.label()), tag.default_kind());
    match tag {
        // Like C4D: a new camera is created at the current view.
        ObjectKindTag::Camera => {
            data.transform = Transform::from_position(camera.eye());
            data.transform.rotation = Vec3::new(camera.yaw, -camera.pitch, 0.0);
            data.display_color = [0.3, 0.3, 0.32];
        }
        ObjectKindTag::Light => {
            data.transform.position = Vec3::new(0.0, 400.0, 0.0);
            data.display_color = [1.0, 0.95, 0.85];
        }
        ObjectKindTag::Null
        | ObjectKindTag::Cube
        | ObjectKindTag::Sphere
        | ObjectKindTag::Cylinder
        | ObjectKindTag::Plane => {}
    }
    data
}

fn has_selected_ancestor(scene: &Scene, selection: &Selection, id: NodeId) -> bool {
    let mut current = scene.get(id).and_then(Node::parent);
    while let Some(parent) = current {
        if selection.contains(parent) {
            return true;
        }
        current = scene.get(parent).and_then(Node::parent);
    }
    false
}

/// Icon tint for an object type (shared by Object Manager, menus and toolbar).
#[must_use]
pub fn tag_color(tag: ObjectKindTag) -> egui::Color32 {
    match tag {
        ObjectKindTag::Null => palette::ICON_NULL,
        ObjectKindTag::Camera => palette::ICON_CAMERA,
        ObjectKindTag::Light => palette::ICON_LIGHT,
        ObjectKindTag::Cube | ObjectKindTag::Sphere | ObjectKindTag::Cylinder | ObjectKindTag::Plane => {
            palette::ICON_PRIMITIVE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::History;

    /// A headless viewport: without a wgpu render state it only keeps the camera.
    fn viewport() -> ViewportPanel {
        ViewportPanel::new(None, None, 1)
    }

    fn id_of(doc: &Document, name: &str) -> NodeId {
        doc.scene
            .iter()
            .find(|(_, n)| n.data.name == name)
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no node named {name}"))
    }

    fn run(doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel, commands: &[EditorCommand]) {
        for command in commands {
            editor.commands.push(command.clone());
        }
        assert!(apply_pending(doc, editor, viewport));
        assert!(editor.commands.is_empty());
    }

    #[test]
    fn apply_pending_reports_whether_anything_ran() {
        let (mut doc, mut editor, mut vp) = (Document::default(), EditorState::default(), viewport());
        assert!(!apply_pending(&mut doc, &mut editor, &mut vp));
        editor.commands.push(EditorCommand::DeselectAll);
        assert!(apply_pending(&mut doc, &mut editor, &mut vp));
        assert!(!apply_pending(&mut doc, &mut editor, &mut vp));
    }

    #[test]
    fn create_object_names_uniquely_and_selects_it() {
        let (mut doc, mut editor, mut vp) = (Document::default(), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::CreateObject(ObjectKindTag::Cube),
                EditorCommand::CreateObject(ObjectKindTag::Cube),
            ],
        );
        assert_eq!(doc.scene.len(), 2);
        let second = id_of(&doc, "Cube.1");
        assert!(doc.scene.contains(id_of(&doc, "Cube")));
        assert_eq!(editor.selection.as_slice(), &[second]);
        assert_eq!(
            doc.scene.get(second).unwrap().data.kind,
            ObjectKindTag::Cube.default_kind()
        );
    }

    #[test]
    fn new_camera_is_placed_at_the_current_view() {
        let (mut doc, mut editor, mut vp) = (Document::default(), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Camera)],
        );
        let camera = doc.scene.get(id_of(&doc, "Camera")).unwrap();
        assert!(camera.data.transform.position.abs_diff_eq(vp.camera().eye(), 1e-3));
    }

    #[test]
    fn delete_removes_subtrees_and_clears_selection() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let before = doc.scene.len();
        let (group, child) = (id_of(&doc, "Group"), id_of(&doc, "Box"));
        // Parent and child both selected: the child goes with the parent, no double delete.
        editor.selection.set_all([child, group]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        assert_eq!(doc.scene.len(), before - 3, "Group, Cylinder and Box");
        assert!(!doc.scene.contains(group) && !doc.scene.contains(child));
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn reparent_keeps_world_position_and_rejects_cycles() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let (cube, group, cylinder) = (id_of(&doc, "Cube"), id_of(&doc, "Group"), id_of(&doc, "Cylinder"));
        let world_before = doc.scene.world_matrix(cube).w_axis.truncate();

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::Reparent {
                    node: cube,
                    new_parent: Some(group),
                },
                // Group under its own child: rejected, the hierarchy is unchanged.
                EditorCommand::Reparent {
                    node: group,
                    new_parent: Some(cylinder),
                },
            ],
        );
        assert_eq!(doc.scene.get(cube).unwrap().parent(), Some(group));
        assert_eq!(doc.scene.get(group).unwrap().parent(), None);
        let world_after = doc.scene.world_matrix(cube).w_axis.truncate();
        assert!(world_after.abs_diff_eq(world_before, 1e-3), "{world_after:?}");
    }

    #[test]
    fn select_modes_follow_c4d() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let (cube, sphere) = (id_of(&doc, "Cube"), id_of(&doc, "Sphere"));
        let select = |id, mode| EditorCommand::Select { id, mode };

        run(&mut doc, &mut editor, &mut vp, &[select(cube, SelectMode::Replace)]);
        assert_eq!(editor.selection.as_slice(), &[cube]);

        run(&mut doc, &mut editor, &mut vp, &[select(sphere, SelectMode::Add)]);
        assert_eq!(editor.selection.as_slice(), &[cube, sphere]);
        assert_eq!(editor.selection.active(), Some(sphere));

        run(&mut doc, &mut editor, &mut vp, &[select(cube, SelectMode::Toggle)]);
        assert_eq!(editor.selection.as_slice(), &[sphere]);

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SelectAll]);
        assert_eq!(editor.selection.len(), doc.scene.len());

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeselectAll]);
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn commands_on_deleted_nodes_are_ignored() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let sphere = id_of(&doc, "Sphere");
        editor.selection.set_single(sphere);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::DeleteSelected,
                EditorCommand::Select {
                    id: sphere,
                    mode: SelectMode::Add,
                },
                EditorCommand::Reparent {
                    node: sphere,
                    new_parent: None,
                },
            ],
        );
        assert!(!doc.scene.contains(sphere));
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn new_document_resets_scene_selection_and_view() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        editor.selection.set_single(id_of(&doc, "Cube"));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::FrameSelected]);
        assert_ne!(*vp.camera(), OrbitCamera::default());

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::NewDocument]);
        assert!(doc.scene.is_empty());
        assert!(editor.selection.is_empty());
        assert_eq!(*vp.camera(), OrbitCamera::default());
    }

    // ── Undo / redo ─────────────────────────────────────────────────────────

    /// The user-visible document: hierarchy (pre-order ids, parents, depths) and data.
    fn snapshot(doc: &Document) -> Vec<(NodeId, Option<NodeId>, u32, NodeData)> {
        let mut out = Vec::new();
        doc.scene.walk(&mut Vec::new(), |id, node, _, depth| {
            out.push((id, node.parent(), depth, node.data.clone()));
            true
        });
        out
    }

    fn demo() -> (Document, EditorState, ViewportPanel) {
        (Document::with_demo_scene(), EditorState::default(), viewport())
    }

    #[test]
    fn create_undo_redo_reuses_the_node_and_restores_selection() {
        let (mut doc, mut editor, mut vp) = demo();
        let sphere = id_of(&doc, "Sphere");
        editor.selection.set_single(sphere);
        let before = snapshot(&doc);

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Cube)],
        );
        let cube = id_of(&doc, "Cube.1");
        assert_eq!(editor.selection.as_slice(), &[cube]);
        assert_eq!(editor.history.undo_label(), Some("Create Object"));
        let after = snapshot(&doc);

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(snapshot(&doc), before);
        assert_eq!(editor.selection.as_slice(), &[sphere], "selection from before the step");
        assert_eq!(editor.history.redo_label(), Some("Create Object"));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(snapshot(&doc), after);
        assert_eq!(editor.selection.as_slice(), &[cube], "same NodeId after redo");
    }

    #[test]
    fn delete_is_one_step_and_undo_restores_ids_order_and_selection() {
        let (mut doc, mut editor, mut vp) = demo();
        let (group, boxed, cube) = (id_of(&doc, "Group"), id_of(&doc, "Box"), id_of(&doc, "Cube"));
        // Box is inside Group: it goes with its parent instead of failing the batch.
        editor.selection.set_all([boxed, cube, group]);
        let before = snapshot(&doc);

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        assert_eq!(doc.scene.len(), before.len() - 4, "Cube, Group, Cylinder, Box");
        assert_eq!(editor.history.undo_len(), 1);
        assert_eq!(editor.history.undo_label(), Some("Delete"));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(snapshot(&doc), before);
        assert_eq!(editor.selection.as_slice(), &[boxed, cube, group]);

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert!(!doc.scene.contains(group) && !doc.scene.contains(cube));
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn reparent_undo_restores_parent_order_and_transform() {
        let (mut doc, mut editor, mut vp) = demo();
        let (cube, group) = (id_of(&doc, "Cube"), id_of(&doc, "Group"));
        let before = snapshot(&doc);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Reparent {
                node: cube,
                new_parent: Some(group),
            }],
        );
        assert_eq!(doc.scene.get(cube).unwrap().parent(), Some(group));
        assert_eq!(editor.history.undo_label(), Some("Reparent"));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(snapshot(&doc), before);
    }

    #[test]
    fn rejected_and_no_op_commands_record_nothing() {
        let (mut doc, mut editor, mut vp) = demo();
        let (group, cylinder) = (id_of(&doc, "Group"), id_of(&doc, "Cylinder"));
        let before = snapshot(&doc);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::Undo, // nothing to undo yet
                EditorCommand::Redo,
                EditorCommand::DeleteSelected, // nothing selected
                EditorCommand::Reparent {
                    node: group,
                    new_parent: Some(cylinder), // cycle
                },
                EditorCommand::Reparent {
                    node: cylinder,
                    new_parent: Some(group), // already its parent
                },
                EditorCommand::SelectAll,
                EditorCommand::FrameSelected,
                EditorCommand::DeselectAll,
            ],
        );
        assert_eq!(snapshot(&doc), before);
        assert!(!editor.history.can_undo() && !editor.history.can_redo());
        assert_eq!(editor.history.state_id(), 0);
    }

    #[test]
    fn commit_data_records_live_edits() {
        let (mut doc, mut editor, mut vp) = demo();
        let sphere = id_of(&doc, "Sphere");
        let before = doc.scene.get(sphere).unwrap().data.clone();
        let commit = EditorCommand::CommitData {
            id: sphere,
            before: before.clone(),
            label: "Move",
        };

        run(&mut doc, &mut editor, &mut vp, std::slice::from_ref(&commit));
        assert!(!editor.history.can_undo(), "unchanged data records nothing");

        // A live edit, as the gizmo makes while dragging, then the commit on release.
        doc.scene.get_mut(sphere).unwrap().data.transform.position.x += 25.0;
        let after = doc.scene.get(sphere).unwrap().data.clone();
        run(&mut doc, &mut editor, &mut vp, std::slice::from_ref(&commit));
        assert_eq!(editor.history.undo_label(), Some("Move"));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.get(sphere).unwrap().data, before);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(doc.scene.get(sphere).unwrap().data, after);
    }

    #[test]
    fn mixed_steps_undo_and_redo_exactly() {
        let (mut doc, mut editor, mut vp) = demo();
        let (sphere, group, light) = (id_of(&doc, "Sphere"), id_of(&doc, "Group"), id_of(&doc, "Light"));
        let mut states = vec![snapshot(&doc)];

        let mut step = |doc: &mut Document, editor: &mut EditorState, commands: &[EditorCommand]| {
            run(doc, editor, &mut vp, commands);
            states.push(snapshot(doc));
        };
        step(
            &mut doc,
            &mut editor,
            &[EditorCommand::CreateObject(ObjectKindTag::Null)],
        );
        let null = id_of(&doc, "Null");
        step(
            &mut doc,
            &mut editor,
            &[EditorCommand::Reparent {
                node: sphere,
                new_parent: Some(null),
            }],
        );
        step(
            &mut doc,
            &mut editor,
            &[
                EditorCommand::Select {
                    id: group,
                    mode: SelectMode::Replace,
                },
                EditorCommand::Select {
                    id: light,
                    mode: SelectMode::Add,
                },
                EditorCommand::DeleteSelected,
            ],
        );
        let before = doc.scene.get(null).unwrap().data.clone();
        doc.scene.get_mut(null).unwrap().data.name = "Rig".to_owned();
        step(
            &mut doc,
            &mut editor,
            &[EditorCommand::CommitData {
                id: null,
                before,
                label: "Rename",
            }],
        );
        assert_eq!(editor.history.undo_len(), 4);

        for expected in states.iter().rev().skip(1) {
            run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
            assert_eq!(&snapshot(&doc), expected);
        }
        assert!(!editor.history.can_undo());
        for expected in states.iter().skip(1) {
            run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
            assert_eq!(&snapshot(&doc), expected);
        }
        assert!(!editor.history.can_redo());
    }

    #[test]
    fn a_new_step_discards_redo_and_purges_only_unreachable_nodes() {
        let (mut doc, mut editor, mut vp) = demo();
        let sphere = id_of(&doc, "Sphere");
        editor.selection.set_single(sphere);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::DeleteSelected,                    // undo step 1 can bring Sphere back
                EditorCommand::CreateObject(ObjectKindTag::Cube), // step 2
                EditorCommand::Undo,                              // Cube only reachable by redo
            ],
        );
        assert_eq!(doc.scene.detached_roots().len(), 2);
        let undone_cube = *doc.scene.detached_roots().last().unwrap();

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Plane)],
        );
        assert!(!editor.history.can_redo());
        assert_eq!(
            doc.scene.detached_roots(),
            &[sphere],
            "the cube was purged, the sphere kept"
        );

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert!(doc.scene.contains(sphere), "undo of the delete still works");
        assert!(!doc.scene.contains(undone_cube));
        assert!(doc.scene.detached_roots().iter().all(|&root| root != undone_cube));
    }

    #[test]
    fn the_history_limit_purges_what_falls_off() {
        let (mut doc, mut editor, mut vp) = demo();
        editor.history = History::with_limit(2);
        let ids = ["Cube", "Sphere", "Light"].map(|name| id_of(&doc, name));
        for id in ids {
            editor.selection.set_single(id);
            run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        }
        assert_eq!(editor.history.undo_len(), 2);
        assert_eq!(
            doc.scene.detached_roots(),
            &ids[1..],
            "the oldest delete can no longer be undone, so its node is freed"
        );

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo, EditorCommand::Undo],
        );
        assert!(!doc.scene.contains(ids[0]));
        assert!(doc.scene.contains(ids[1]) && doc.scene.contains(ids[2]));
    }

    #[test]
    fn state_id_identifies_document_states() {
        let (mut doc, mut editor, mut vp) = demo();
        let s0 = editor.history.state_id();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Cube)],
        );
        let s1 = editor.history.state_id();
        assert_ne!(s1, s0);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(editor.history.state_id(), s0);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(editor.history.state_id(), s1);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::CreateObject(ObjectKindTag::Sphere)],
        );
        let s2 = editor.history.state_id();
        assert!(s2 != s0 && s2 != s1, "a different edit is a different state");
    }

    #[test]
    fn new_document_clears_the_history() {
        let (mut doc, mut editor, mut vp) = demo();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::CreateObject(ObjectKindTag::Cube),
                EditorCommand::CreateObject(ObjectKindTag::Cube),
                EditorCommand::Undo,
                EditorCommand::NewDocument,
            ],
        );
        assert!(!editor.history.can_undo() && !editor.history.can_redo());
        assert_eq!(editor.history.state_id(), 0);
        assert!(doc.scene.detached_roots().is_empty());
    }
}
