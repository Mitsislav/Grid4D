//! Structural editor operations, queued by panels and applied once per frame.
//!
//! This is the single choke point for undo/redo: every document change a
//! command makes is applied as [`SceneEdit`]s and recorded as one undo step in
//! [`EditorState::history`]. Selection and view changes are not undo steps, but
//! each step restores the selection it was made with.

use glam::Vec3;
use grid4d_geom::{
    BitSet, Component, MeshError, PolyMesh, Topology, ops,
    selection::{self, corners_from_edges, edges_from_corners},
};
use grid4d_render::OrbitCamera;
use grid4d_scene::{
    ComponentState, Document, MeshHandle, Node, NodeData, NodeId, ObjectKind, ObjectKindTag, Scene, SceneEdit,
    Transform,
};

use crate::{
    EditMode, EditorState, Selection, ViewportPanel,
    file::{self, FileAction, UnsavedChoice},
    theme::palette,
};

/// How a click changes the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    /// Plain click.
    Replace,
    /// Ctrl/Cmd + click.
    Toggle,
    /// Shift + click.
    Add,
    /// Ctrl/Cmd + click in a component mode (C4D removes there, rather than toggling).
    Remove,
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

    /// The same for components: Shift adds, Ctrl removes.
    #[must_use]
    pub fn for_components(m: egui::Modifiers) -> Self {
        match Self::from_modifiers(m) {
            Self::Toggle => Self::Remove,
            other => other,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    /// Replaces the document with an empty one (asks to save unsaved changes first).
    NewDocument,
    /// Asks for a document to open (asks to save unsaved changes first).
    Open,
    /// Opens this document: File ▸ Open Recent, or a file dropped on the window
    /// (asks to save unsaved changes first).
    OpenFile(std::path::PathBuf),
    /// Saves to the current file, asking for a path if there is none yet.
    Save,
    /// Asks for a path and saves there.
    SaveAs,
    /// Closes the window (asks to save unsaved changes first).
    Quit,
    /// The answer to a pending "Save changes?" prompt.
    AnswerUnsaved(UnsavedChoice),
    CreateObject(ObjectKindTag),
    DeleteSelected,
    /// Turns the selected primitives into polygon objects that look the same
    /// (C4D's "Make Editable", `C`); other objects are left alone.
    MakeEditable,
    /// Component modes: merges the selected points, edges or polygons into the
    /// polygons around them (Melt, U~Z / Alt+Backspace). One undo step.
    MeltSelected,
    /// Like Melt; for edges also removes the points left between two edges
    /// (Dissolve, M~N / Ctrl+Backspace). One undo step.
    DissolveSelected,
    /// Joins the selected points, or the middles of the selected edges, that share
    /// a polygon (Connect Points/Edges, M~M). One undo step.
    ConnectSelected,
    /// Loop/Path Cut on polygon object `node` across unique edge `edge` at
    /// `fractions` along it (from the edge's first point). One undo step.
    LoopCut {
        node: NodeId,
        edge: u32,
        fractions: Vec<f32>,
    },
    /// Line Cut on polygon object `node`: the plane through `eye` and the world
    /// directions `from` and `to` (the rays through the line's ends), with the Line
    /// Cut tool's options. One undo step.
    LineCut {
        node: NodeId,
        eye: Vec3,
        from: Vec3,
        to: Vec3,
    },
    /// Runs the current modelling tool (Extrude, Inset) once with its options on
    /// the active polygon object's selection in the current mode (its Apply
    /// button). One undo step if anything changed.
    ApplyMeshTool,
    /// A click, frame or loop in a component mode on polygon object `node`:
    /// `hits` are the picked points, unique edges or polygons — none for empty
    /// space, which clears the selection on a plain click. One undo step if
    /// anything changed.
    SelectComponents {
        node: NodeId,
        component: Component,
        hits: Vec<u32>,
        mode: SelectMode,
    },
    /// Select ▸ Grow Selection (neighbours of the selection, current mode).
    GrowSelection,
    /// Select ▸ Shrink Selection (drops the selection's border).
    ShrinkSelection,
    /// Select ▸ Invert (every visible component not selected).
    InvertSelection,
    /// Select ▸ Select Connected (everything reachable from the selection).
    SelectConnected,
    /// Select ▸ Hide Selected / Hide Unselected / Unhide All.
    HideSelected,
    HideUnselected,
    UnhideAll,
    /// Switches to mode `to`, converting the current mode's component selection
    /// of the active object (Shift+click on a mode: `tolerant`; Ctrl+click:
    /// strict). One undo step if the selection changed.
    ConvertSelection {
        to: EditMode,
        tolerant: bool,
    },
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
    /// Applies one discrete document change (a rename, a visibility toggle) as
    /// an undo step.
    Edit {
        edit: SceneEdit,
        label: &'static str,
    },
    /// Records a change that was already written live while the user interacted
    /// (a gizmo drag, a value field): `inverse` restores the state from before
    /// the interaction, e.g. `SetData` with the node's old data. Nothing is
    /// recorded if the live value equals it, i.e. nothing changed.
    CommitLive {
        inverse: SceneEdit,
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

    /// The queued commands, oldest first.
    #[cfg(test)]
    pub(crate) fn pending(&self) -> &[EditorCommand] {
        &self.pending
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
        EditorCommand::NewDocument => file::request(FileAction::New, doc, editor, viewport),
        EditorCommand::Open => file::request(FileAction::Open, doc, editor, viewport),
        EditorCommand::OpenFile(path) => file::request(FileAction::OpenPath(path), doc, editor, viewport),
        EditorCommand::Save => {
            file::save(doc, editor, false);
        }
        EditorCommand::SaveAs => {
            file::save(doc, editor, true);
        }
        EditorCommand::Quit => file::request(FileAction::Quit, doc, editor, viewport),
        EditorCommand::AnswerUnsaved(choice) => file::answer(choice, doc, editor, viewport),
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
        // Deleting components comes with step 4.5; objects stay put meanwhile.
        EditorCommand::DeleteSelected if editor.mode != EditMode::Model => {
            let label = match editor.mode {
                EditMode::Points => "Delete Points",
                EditMode::Edges => "Delete Edges",
                EditMode::Polygons | EditMode::Model => "Delete Polygons",
            };
            rebuild_selection(doc, editor, label, ops::delete, false);
        }
        EditorCommand::MeltSelected => rebuild_selection(doc, editor, "Melt", ops::melt, false),
        EditorCommand::DissolveSelected => rebuild_selection(doc, editor, "Dissolve", ops::dissolve, false),
        EditorCommand::ConnectSelected => rebuild_selection(doc, editor, "Connect", ops::connect, true),
        EditorCommand::LoopCut { node, edge, fractions } => {
            let select_cuts = editor.modeling_settings.loop_cut.select_cuts;
            let data = doc.scene.get(node).and_then(|n| {
                let ObjectKind::Polygon { mesh, .. } = &n.data.kind else {
                    return None;
                };
                let rebuilt = crate::modeling::loop_cut(mesh, idx(edge), &fractions)?;
                crate::modeling::rebuilt_data(&n.data, rebuilt, select_cuts)
            });
            if let Some(data) = data {
                commit(
                    doc,
                    editor,
                    "Loop Cut",
                    vec![SceneEdit::SetData { id: node, data }],
                    |_, _| {},
                );
            }
        }
        EditorCommand::LineCut { node, eye, from, to } => {
            let settings = editor.modeling_settings.line_cut;
            let world = doc.scene.world_matrix(node);
            let data = doc.scene.get(node).and_then(|n| {
                let ObjectKind::Polygon { mesh, .. } = &n.data.kind else {
                    return None;
                };
                // The cut runs in the object's space.
                let local = world.inverse();
                let cut = ops::LineCut {
                    eye: local.transform_point3(eye),
                    from: local.transform_vector3(from),
                    to: local.transform_vector3(to),
                    infinite: settings.infinite,
                    connect: settings.connect,
                };
                let visible = |x: Vec3| {
                    !settings.visible_only || {
                        let to_x = x - cut.eye;
                        let distance = to_x.length();
                        // Seen if nothing is hit clearly before the crossing.
                        mesh.raycast(cut.eye, to_x / distance.max(f32::EPSILON))
                            .is_none_or(|hit| hit.t >= distance * (1.0 - 1e-4) - 1e-4)
                    }
                };
                let rebuilt = ops::line_cut(mesh.mesh(), mesh.topology(), &cut, visible).ok()?;
                crate::modeling::rebuilt_data(&n.data, rebuilt, settings.select_cuts)
            });
            if let Some(data) = data {
                commit(
                    doc,
                    editor,
                    "Line Cut",
                    vec![SceneEdit::SetData { id: node, data }],
                    |_, _| {},
                );
            }
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
        EditorCommand::MakeEditable => {
            let edits = editor
                .selection
                .as_slice()
                .iter()
                .filter_map(|&id| {
                    let data = &doc.scene.get(id)?.data;
                    let mesh = grid4d_eval::editable_mesh(&data.kind)?;
                    let mut data = data.clone();
                    data.kind = ObjectKind::polygon(MeshHandle::new(mesh));
                    Some(SceneEdit::SetData { id, data })
                })
                .collect();
            commit(doc, editor, "Make Editable", edits, |_, _| {});
        }
        EditorCommand::ApplyMeshTool => {
            let tool = editor.tool;
            let mut settings = editor.modeling_settings;
            let offset = crate::modeling::offset_mut(tool, &mut settings).map_or(0.0, |o| *o);
            if let Some((id, component)) = component_target(doc, editor)
                && let Some(data) = doc
                    .scene
                    .get(id)
                    .and_then(|n| crate::modeling::apply(&n.data, tool, component, &settings, offset))
            {
                commit(
                    doc,
                    editor,
                    tool.label(),
                    vec![SceneEdit::SetData { id, data }],
                    |_, _| {},
                );
            }
        }
        EditorCommand::Select { id, mode } => {
            if doc.scene.contains(id) {
                match mode {
                    SelectMode::Replace => editor.selection.set_single(id),
                    SelectMode::Toggle => editor.selection.toggle(id),
                    SelectMode::Add => editor.selection.add(id),
                    SelectMode::Remove => editor.selection.remove(id),
                }
            }
        }
        EditorCommand::SelectComponents {
            node,
            component,
            hits,
            mode,
        } => edit_components(doc, editor, node, select_label(component), |state, mesh| {
            let picked: BitSet = hits.iter().map(|&i| idx(i)).collect();
            // An edge is stored as all of its corners; hidden components stay unselected.
            let mut picked = stored(picked, component, mesh);
            picked.remove_all(&hidden_mask(state, mesh, component));
            let indices: Vec<usize> = picked.iter().collect();
            let bits = state.selected.get_mut(component);
            match mode {
                SelectMode::Replace => {
                    bits.clear();
                    bits.extend(indices);
                }
                SelectMode::Add => bits.extend(indices),
                SelectMode::Remove => {
                    for i in indices {
                        bits.remove(i);
                    }
                }
                SelectMode::Toggle => {
                    let on = indices.iter().any(|&i| bits.contains(i));
                    for i in indices {
                        if on {
                            bits.remove(i);
                        } else {
                            bits.insert(i);
                        }
                    }
                }
            }
        }),
        EditorCommand::GrowSelection => {
            let edges_only = editor.selection_settings.grow_edges_only;
            in_component_mode(doc, editor, "Grow Selection", |state, mesh, component| {
                let grown = selection::grow(
                    mesh.mesh(),
                    mesh.topology(),
                    state.selected.get(component),
                    component,
                    edges_only,
                );
                set_selected(state, mesh, component, grown);
            });
        }
        EditorCommand::ShrinkSelection => {
            let edges_only = editor.selection_settings.grow_edges_only;
            in_component_mode(doc, editor, "Shrink Selection", |state, mesh, component| {
                let shrunk = selection::shrink(
                    mesh.mesh(),
                    mesh.topology(),
                    state.selected.get(component),
                    component,
                    edges_only,
                );
                set_selected(state, mesh, component, shrunk);
            });
        }
        EditorCommand::InvertSelection => {
            in_component_mode(doc, editor, "Invert Selection", |state, mesh, component| {
                let hidden = hidden_mask(state, mesh, component);
                let inverted = selection::invert(
                    mesh.mesh(),
                    mesh.topology(),
                    state.selected.get(component),
                    component,
                    &hidden,
                );
                set_selected(state, mesh, component, inverted);
            })
        }
        EditorCommand::SelectConnected => {
            in_component_mode(doc, editor, "Select Connected", |state, mesh, component| {
                let reached =
                    selection::connected(mesh.mesh(), mesh.topology(), state.selected.get(component), component);
                set_selected(state, mesh, component, reached);
            })
        }
        EditorCommand::HideSelected => in_component_mode(doc, editor, "Hide Selected", |state, mesh, component| {
            let selected = stored(
                elements(state.selected.get(component), component, mesh),
                component,
                mesh,
            );
            state.hidden.get_mut(component).extend(selected.iter());
            state.selected.get_mut(component).clear();
        }),
        EditorCommand::HideUnselected => in_component_mode(doc, editor, "Hide Unselected", |state, mesh, component| {
            let hidden = hidden_mask(state, mesh, component);
            let others = selection::invert(
                mesh.mesh(),
                mesh.topology(),
                state.selected.get(component),
                component,
                &hidden,
            );
            state.hidden.get_mut(component).extend(others.iter());
        }),
        EditorCommand::UnhideAll => in_component_mode(doc, editor, "Unhide All", |state, _, _| {
            state.hidden = grid4d_geom::ComponentSet::default();
        }),
        EditorCommand::ConvertSelection { to, tolerant } => {
            if let (Some(from), Some(target), Some(id)) =
                (editor.mode.component(), to.component(), editor.selection.active())
                && from != target
            {
                edit_components(doc, editor, id, "Convert Selection", |state, mesh| {
                    let mut converted =
                        selection::convert(mesh.mesh(), mesh.topology(), &state.selected, from, target, tolerant);
                    converted.remove_all(state.hidden.get(target)); // hidden components stay unselected
                    *state.selected.get_mut(target) = converted;
                });
            }
            editor.mode = to;
        }
        EditorCommand::SelectAll if editor.mode != EditMode::Model => {
            in_component_mode(doc, editor, "Select All", |state, mesh, component| {
                let count = match component {
                    Component::Point => mesh.mesh().point_count(),
                    Component::Edge => mesh.mesh().corner_count(),
                    Component::Polygon => mesh.mesh().face_count(),
                };
                let mut all = BitSet::new();
                all.insert_range(0..count);
                set_selected(state, mesh, component, all);
            });
        }
        EditorCommand::DeselectAll if editor.mode != EditMode::Model => {
            if let (Some(component), Some(id)) = (editor.mode.component(), editor.selection.active()) {
                edit_components(doc, editor, id, "Deselect All", |state, _| {
                    state.selected.get_mut(component).clear();
                });
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
        EditorCommand::Edit { edit, label } => commit(doc, editor, label, vec![edit], |_, _| {}),
        EditorCommand::CommitLive { inverse, label } => {
            if !is_current_state(doc, &inverse) {
                let selection = editor.selection.clone();
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

/// The active object and the current mode's component kind, when the mode is a
/// component mode and the active object a polygon object.
#[must_use]
pub fn component_target(doc: &Document, editor: &EditorState) -> Option<(NodeId, Component)> {
    let component = editor.mode.component()?;
    let id = editor.selection.active()?;
    matches!(doc.scene.get(id)?.data.kind, ObjectKind::Polygon { .. }).then_some((id, component))
}

/// Runs a rebuilding operation (Delete, Melt, Dissolve, Connect) on the active
/// polygon object's selection in the current component mode, carrying its
/// selection and hidden flags across: one undo step named `label` if the mesh
/// changed. With `select_cuts` the new edges become the edge selection.
fn rebuild_selection(
    doc: &mut Document,
    editor: &mut EditorState,
    label: &'static str,
    operation: impl FnOnce(&PolyMesh, &Topology, Component, &BitSet) -> Result<ops::Rebuilt, MeshError>,
    select_cuts: bool,
) {
    let Some((id, component)) = component_target(doc, editor) else {
        return;
    };
    let data = doc.scene.get(id).and_then(|n| {
        let ObjectKind::Polygon { mesh, components } = &n.data.kind else {
            return None;
        };
        let topology = mesh.topology();
        let selected = match component {
            Component::Point => components.selected().points.clone(),
            Component::Edge => edges_from_corners(&components.selected().edges, topology),
            Component::Polygon => components.selected().polygons.clone(),
        };
        if selected.is_empty() {
            return None;
        }
        let rebuilt = operation(mesh.mesh(), topology, component, &selected).ok()?;
        crate::modeling::rebuilt_data(&n.data, rebuilt, select_cuts)
    });
    if let Some(data) = data {
        commit(doc, editor, label, vec![SceneEdit::SetData { id, data }], |_, _| {});
    }
}

/// Runs `change` on the active polygon object's components in the current
/// component mode, as one undo step named `label` (nothing outside such a mode).
fn in_component_mode(
    doc: &mut Document,
    editor: &mut EditorState,
    label: &'static str,
    change: impl FnOnce(&mut ComponentState, &MeshHandle, Component),
) {
    if let Some((id, component)) = component_target(doc, editor) {
        edit_components(doc, editor, id, label, |state, mesh| change(state, mesh, component));
    }
}

/// Every stored flag that hides a component of kind `component`: hidden ones,
/// the points/edges of hidden polygons, and every corner of a hidden edge.
fn hidden_mask(state: &ComponentState, mesh: &MeshHandle, component: Component) -> BitSet {
    let closure = selection::hidden_closure(mesh.mesh(), mesh.topology(), &state.hidden);
    stored(elements(closure.get(component), component, mesh), component, mesh)
}

/// Replaces the selection of `component` with `bits` (stored form), minus hidden ones.
fn set_selected(state: &mut ComponentState, mesh: &MeshHandle, component: Component, mut bits: BitSet) {
    bits.remove_all(&hidden_mask(state, mesh, component));
    *state.selected.get_mut(component) = bits;
}

/// A stored selection as elements: edges as unique edges, the rest as is.
fn elements(bits: &BitSet, component: Component, mesh: &MeshHandle) -> BitSet {
    match component {
        Component::Edge => edges_from_corners(bits, mesh.topology()),
        Component::Point | Component::Polygon => bits.clone(),
    }
}

/// Elements in stored form: an edge as all of its corners.
fn stored(bits: BitSet, component: Component, mesh: &MeshHandle) -> BitSet {
    match component {
        Component::Edge => corners_from_edges(&bits, mesh.topology()),
        Component::Point | Component::Polygon => bits,
    }
}

/// Changes the components of polygon object `id` with `change` as one undo step
/// named `label`; records nothing if they end up unchanged.
fn edit_components(
    doc: &mut Document,
    editor: &mut EditorState,
    id: NodeId,
    label: &'static str,
    change: impl FnOnce(&mut ComponentState, &MeshHandle),
) {
    let Some(node) = doc.scene.get(id) else { return };
    let ObjectKind::Polygon { mesh, components } = &node.data.kind else {
        return;
    };
    let mut updated = components.clone();
    change(updated.make_mut(), mesh);
    if updated == *components {
        return;
    }
    let mut data = node.data.clone();
    data.kind = ObjectKind::Polygon {
        mesh: mesh.clone(),
        components: updated,
    };
    commit(doc, editor, label, vec![SceneEdit::SetData { id, data }], |_, _| {});
}

const fn select_label(component: Component) -> &'static str {
    match component {
        Component::Point => "Select Points",
        Component::Edge => "Select Edges",
        Component::Polygon => "Select Polygons",
    }
}

/// `u32` index → `usize` (lossless on every supported platform).
fn idx(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

/// `true` if [`EditorCommand::MakeEditable`] has a primitive to convert in
/// `selection` (enables the menu entry and the palette button).
#[must_use]
pub fn can_make_editable(doc: &Document, selection: &Selection) -> bool {
    selection.as_slice().iter().any(|&id| {
        doc.scene
            .get(id)
            .is_some_and(|node| ObjectKindTag::PRIMITIVES.contains(&node.data.kind.tag()))
    })
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
        | ObjectKindTag::Plane
        | ObjectKindTag::Cone
        | ObjectKindTag::Torus
        | ObjectKindTag::Disc
        | ObjectKindTag::Capsule
        | ObjectKindTag::Polygon => {}
    }
    data
}

/// `true` if applying the value edit `edit` would change nothing, i.e. a live
/// interaction ended where it started. A missing node counts as unchanged, so a
/// commit for an object deleted mid-drag records nothing.
fn is_current_state(doc: &Document, edit: &SceneEdit) -> bool {
    match edit {
        SceneEdit::SetData { id, data } => doc.scene.get(*id).is_none_or(|node| node.data == *data),
        SceneEdit::SetTime(settings) => doc.time.settings() == *settings,
        SceneEdit::SetName(name) => doc.name == *name,
        // Structural changes are never made live; record them as given.
        SceneEdit::Insert { .. } | SceneEdit::Detach { .. } | SceneEdit::Attach { .. } | SceneEdit::Move { .. } => {
            false
        }
    }
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
        ObjectKindTag::Cube
        | ObjectKindTag::Sphere
        | ObjectKindTag::Cylinder
        | ObjectKindTag::Plane
        | ObjectKindTag::Cone
        | ObjectKindTag::Torus
        | ObjectKindTag::Disc
        | ObjectKindTag::Capsule
        | ObjectKindTag::Polygon => palette::ICON_PRIMITIVE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::History;

    /// A headless viewport: without a wgpu render state it only keeps the camera.
    fn viewport() -> ViewportPanel {
        ViewportPanel::new(None)
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
    fn apply_mesh_tool_is_one_undo_step_in_a_component_mode() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let cube = id_of(&doc, "Cube");
        editor.selection.set_single(cube);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        let faces = |doc: &Document| match &doc.scene.get(cube).unwrap().data.kind {
            ObjectKind::Polygon { mesh, .. } => mesh.mesh().face_count(),
            _ => 0,
        };
        editor.tool = crate::Tool::Extrude;
        let steps = editor.history.undo_len();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ApplyMeshTool]);
        assert_eq!(
            (faces(&doc), editor.history.undo_len()),
            (6, steps),
            "Model mode: nothing"
        );

        editor.mode = EditMode::Polygons;
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ApplyMeshTool]);
        assert_eq!(faces(&doc), 6 + 6 * 4, "every face is its own piece at 90°");
        assert_eq!(editor.history.undo_label(), Some("Extrude"));
        assert_eq!(editor.history.undo_len(), steps + 1);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(faces(&doc), 6);
    }

    fn mesh_of(doc: &Document, id: NodeId) -> MeshHandle {
        let ObjectKind::Polygon { mesh, .. } = &doc.scene.get(id).unwrap().data.kind else {
            panic!("not a polygon object")
        };
        mesh.clone()
    }

    fn select_in(
        doc: &mut Document,
        editor: &mut EditorState,
        vp: &mut ViewportPanel,
        id: NodeId,
        component: Component,
        hits: Vec<u32>,
    ) {
        run(
            doc,
            editor,
            vp,
            &[EditorCommand::SelectComponents {
                node: id,
                component,
                hits,
                mode: SelectMode::Replace,
            }],
        );
    }

    #[test]
    fn deleting_components_rebuilds_the_mesh_as_one_step() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Points);
        let before = mesh_of(&doc, id);
        select_in(&mut doc, &mut editor, &mut vp, id, Component::Point, vec![0]);
        let steps = editor.history.undo_len();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        let after = mesh_of(&doc, id);
        assert_eq!((after.mesh().point_count(), after.mesh().face_count()), (7, 3));
        assert!(
            selected(&doc, id, Component::Point).is_empty(),
            "the deleted point was the selection"
        );
        assert_eq!(editor.history.undo_len(), steps + 1);
        assert_eq!(editor.history.undo_label(), Some("Delete Points"));
        assert!(doc.scene.contains(id), "the object stays");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(mesh_of(&doc, id).id(), before.id());
        assert_eq!(
            selected(&doc, id, Component::Point),
            [0],
            "undo brings the selection back too"
        );

        // Nothing selected: nothing happens.
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        let steps = editor.history.undo_len();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        assert_eq!(
            (mesh_of(&doc, id).mesh().face_count(), editor.history.undo_len()),
            (6, steps)
        );
    }

    #[test]
    fn melt_dissolve_and_connect_are_one_step_each() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        let faces = |doc: &Document| mesh_of(doc, id).mesh().face_count();
        select_in(&mut doc, &mut editor, &mut vp, id, Component::Polygon, vec![0, 1]);
        let adjacent = {
            let mesh = mesh_of(&doc, id);
            let shared = mesh
                .mesh()
                .face(0)
                .iter()
                .filter(|p| mesh.mesh().face(1).contains(p))
                .count();
            shared == 2
        };
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MeltSelected]);
        assert_eq!(faces(&doc), if adjacent { 5 } else { 6 });
        if adjacent {
            assert_eq!(editor.history.undo_label(), Some("Melt"));
            assert_eq!(
                selected(&doc, id, Component::Polygon).len(),
                1,
                "the merged polygon stays selected"
            );
        }

        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Points);
        // Two opposite corners of one face.
        let mesh = mesh_of(&doc, id);
        let face = mesh.mesh().face(0).to_vec();
        select_in(
            &mut doc,
            &mut editor,
            &mut vp,
            id,
            Component::Point,
            vec![face[0], face[2]],
        );
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ConnectSelected]);
        assert_eq!(mesh_of(&doc, id).mesh().face_count(), 7, "the face split in two");
        assert_eq!(editor.history.undo_label(), Some("Connect"));
        assert_eq!(mesh_of(&doc, id).topology().edge_count(), 12 + 1, "one new edge");
        assert!(
            selected(&doc, id, Component::Point).len() == 2,
            "the points stay selected"
        );

        editor.mode = EditMode::Edges;
        let topology = mesh_of(&doc, id).topology().clone();
        let new_edge = topology.edge_count() - 1;
        let corner = topology.edge_corners(new_edge)[0];
        select_in(
            &mut doc,
            &mut editor,
            &mut vp,
            id,
            Component::Edge,
            vec![u32::try_from(new_edge).unwrap()],
        );
        assert!(!selected(&doc, id, Component::Edge).is_empty(), "{corner}");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DissolveSelected]);
        assert_eq!(
            mesh_of(&doc, id).mesh().face_count(),
            6,
            "dissolving the new edge undoes the split"
        );
        assert_eq!(editor.history.undo_label(), Some("Dissolve"));
    }

    #[test]
    fn loop_and_line_cuts_are_one_step_each() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Edges);
        let mesh = mesh_of(&doc, id);
        // A vertical edge: its ring runs round the four sides.
        let vertical = (0..mesh.topology().edge_count())
            .find(|&e| {
                let [a, b] = mesh.topology().edges()[e];
                let (pa, pb) = (mesh.mesh().points()[a as usize], mesh.mesh().points()[b as usize]);
                (pa.x - pb.x).abs() < 1e-3 && (pa.z - pb.z).abs() < 1e-3
            })
            .unwrap();
        let cut = EditorCommand::LoopCut {
            node: id,
            edge: u32::try_from(vertical).unwrap(),
            fractions: vec![0.5],
        };
        run(&mut doc, &mut editor, &mut vp, &[cut]);
        let after = mesh_of(&doc, id);
        assert_eq!((after.mesh().point_count(), after.mesh().face_count()), (12, 10));
        assert_eq!(editor.history.undo_label(), Some("Loop Cut"));
        assert_eq!(
            selected(&doc, id, Component::Edge).len(),
            8,
            "the 4 new edges, from both sides"
        );

        // A line straight down through the moved cube, in world space.
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        doc.scene.get_mut(id).unwrap().data.transform.position = Vec3::new(500.0, 0.0, 0.0);
        let eye = Vec3::new(500.0, 1000.0, 0.0);
        let line = EditorCommand::LineCut {
            node: id,
            eye,
            from: Vec3::new(500.0, 0.0, -300.0) - eye,
            to: Vec3::new(500.0, 0.0, 300.0) - eye,
        };
        run(&mut doc, &mut editor, &mut vp, std::slice::from_ref(&line));
        assert_eq!(
            mesh_of(&doc, id).mesh().face_count(),
            6 + 1,
            "visible only: the top face splits, the sides are seen edge-on, the bottom is hidden"
        );
        assert_eq!(editor.history.undo_label(), Some("Line Cut"));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        editor.modeling_settings.line_cut.visible_only = false;
        run(&mut doc, &mut editor, &mut vp, &[line]);
        assert_eq!(mesh_of(&doc, id).mesh().face_count(), 6 + 4, "all the way round");
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
    fn make_editable_bakes_primitives_into_meshes_as_one_undo_step() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let (cube, light, group) = (id_of(&doc, "Cube"), id_of(&doc, "Light"), id_of(&doc, "Group"));
        let cube_before = doc.scene.get(cube).unwrap().data.clone();
        let light_before = doc.scene.get(light).unwrap().data.clone();
        editor.selection.set_all([cube, light, group]);
        assert!(can_make_editable(&doc, &editor.selection));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);

        let data = doc.scene.get(cube).unwrap().data.clone();
        let ObjectKind::Polygon { mesh, .. } = &data.kind else {
            panic!("not converted: {:?}", data.kind)
        };
        assert_eq!((mesh.mesh().point_count(), mesh.mesh().face_count()), (8, 6));
        let topology = Topology::build(mesh.mesh());
        assert!(topology.is_closed() && topology.is_manifold(), "a closed, manifold box");
        let bounds = mesh.bounds().unwrap();
        assert!(
            bounds.max.abs_diff_eq(Vec3::splat(100.0), 1e-4),
            "the 200 cm size is in the points"
        );
        assert_eq!((&data.name, data.transform), (&cube_before.name, cube_before.transform));
        assert_eq!(doc.scene.get(light).unwrap().data, light_before, "lights stay lights");
        assert_eq!(editor.history.undo_label(), Some("Make Editable"));
        assert!(!can_make_editable(&doc, &editor.selection), "nothing left to convert");

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.get(cube).unwrap().data, cube_before);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        let ObjectKind::Polygon { mesh: again, .. } = &doc.scene.get(cube).unwrap().data.kind else {
            panic!("redo did not restore the mesh")
        };
        assert_eq!(
            again.id(),
            mesh.id(),
            "the same mesh comes back, so its GPU copy is reused"
        );
    }

    /// A document with one polygon cube (8 points, 6 quads), active, in `mode`.
    fn polygon_cube(mode: EditMode) -> (Document, EditorState, ViewportPanel, NodeId) {
        let mesh = grid4d_eval::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap();
        let mut doc = Document::new("components");
        let id = doc
            .scene
            .insert(Node::new("Cube", ObjectKind::polygon(MeshHandle::new(mesh))));
        let mut editor = EditorState {
            mode,
            ..EditorState::default()
        };
        editor.selection.set_single(id);
        (doc, editor, viewport(), id)
    }

    fn selected(doc: &Document, id: NodeId, component: Component) -> Vec<usize> {
        let ObjectKind::Polygon { components, .. } = &doc.scene.get(id).unwrap().data.kind else {
            panic!("not a polygon object")
        };
        components.selected().get(component).iter().collect()
    }

    #[test]
    fn component_clicks_follow_the_modifiers_one_undo_step_each() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Points);
        let click = |hits: &[u32], mode| EditorCommand::SelectComponents {
            node: id,
            component: Component::Point,
            hits: hits.to_vec(),
            mode,
        };
        run(&mut doc, &mut editor, &mut vp, &[click(&[2], SelectMode::Replace)]);
        assert_eq!(selected(&doc, id, Component::Point), [2]);
        assert_eq!(editor.history.undo_label(), Some("Select Points"));
        run(&mut doc, &mut editor, &mut vp, &[click(&[5], SelectMode::Add)]);
        run(&mut doc, &mut editor, &mut vp, &[click(&[2], SelectMode::Remove)]);
        assert_eq!(selected(&doc, id, Component::Point), [5]);
        run(&mut doc, &mut editor, &mut vp, &[click(&[], SelectMode::Replace)]);
        assert!(selected(&doc, id, Component::Point).is_empty(), "empty space deselects");
        assert_eq!(editor.history.undo_len(), 4);
        run(&mut doc, &mut editor, &mut vp, &[click(&[], SelectMode::Replace)]);
        assert_eq!(editor.history.undo_len(), 4, "no change, no step");

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert_eq!(selected(&doc, id, Component::Point), [2, 5]);
    }

    #[test]
    fn an_edge_is_selected_through_all_of_its_corners() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Edges);
        let pick = |e, mode| EditorCommand::SelectComponents {
            node: id,
            component: Component::Edge,
            hits: vec![e],
            mode,
        };
        run(&mut doc, &mut editor, &mut vp, &[pick(3, SelectMode::Replace)]);
        assert_eq!(
            selected(&doc, id, Component::Edge).len(),
            2,
            "one corner in each of its two faces"
        );
        run(&mut doc, &mut editor, &mut vp, &[pick(3, SelectMode::Toggle)]);
        assert!(selected(&doc, id, Component::Edge).is_empty());
    }

    #[test]
    fn shift_and_ctrl_on_a_mode_convert_the_selection() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Points);
        let select = EditorCommand::SelectComponents {
            node: id,
            component: Component::Point,
            hits: vec![0],
            mode: SelectMode::Replace,
        };
        let convert = |tolerant| EditorCommand::ConvertSelection {
            to: EditMode::Polygons,
            tolerant,
        };
        run(&mut doc, &mut editor, &mut vp, &[select.clone(), convert(true)]);
        assert_eq!(editor.mode, EditMode::Polygons);
        assert_eq!(
            selected(&doc, id, Component::Polygon).len(),
            3,
            "every face using the point"
        );
        assert_eq!(editor.history.undo_label(), Some("Convert Selection"));

        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Points);
        run(&mut doc, &mut editor, &mut vp, &[select, convert(false)]);
        assert_eq!(editor.mode, EditMode::Polygons);
        assert!(
            selected(&doc, id, Component::Polygon).is_empty(),
            "strict: no face is fully selected"
        );
        assert_eq!(
            editor.history.undo_label(),
            Some("Select Points"),
            "nothing changed, no step"
        );
    }

    #[test]
    fn select_all_and_delete_respect_the_component_mode() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SelectAll]);
        assert_eq!(selected(&doc, id, Component::Polygon), [0, 1, 2, 3, 4, 5]);
        assert_eq!(editor.selection.as_slice(), &[id], "the object selection is untouched");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeselectAll]);
        assert!(selected(&doc, id, Component::Polygon).is_empty());
        assert_eq!(editor.history.undo_label(), Some("Deselect All"));
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::SelectAll, EditorCommand::DeleteSelected],
        );
        assert!(
            doc.scene.contains(id),
            "Delete removes the polygons, not the object, in a component mode"
        );
        assert_eq!(mesh_of(&doc, id).mesh().face_count(), 0);
        assert_eq!(editor.history.undo_label(), Some("Delete Polygons"));
    }

    #[test]
    fn grow_shrink_invert_and_connected_work_on_the_current_mode() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        let select = |f: u32| EditorCommand::SelectComponents {
            node: id,
            component: Component::Polygon,
            hits: vec![f],
            mode: SelectMode::Replace,
        };
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[select(0), EditorCommand::GrowSelection],
        );
        assert_eq!(
            selected(&doc, id, Component::Polygon).len(),
            5,
            "the face and its 4 neighbours"
        );
        assert_eq!(editor.history.undo_label(), Some("Grow Selection"));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::InvertSelection]);
        assert_eq!(selected(&doc, id, Component::Polygon).len(), 1, "the opposite face");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SelectConnected]);
        assert_eq!(selected(&doc, id, Component::Polygon).len(), 6, "the whole cube");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ShrinkSelection]);
        assert_eq!(selected(&doc, id, Component::Polygon).len(), 6, "no border, no shrink");
    }

    #[test]
    fn hidden_components_drop_out_of_select_all_and_invert() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        let select = |f: u32| EditorCommand::SelectComponents {
            node: id,
            component: Component::Polygon,
            hits: vec![f],
            mode: SelectMode::Replace,
        };
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[select(2), EditorCommand::HideSelected],
        );
        assert!(
            selected(&doc, id, Component::Polygon).is_empty(),
            "hidden ones are not selected"
        );
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SelectAll]);
        assert_eq!(selected(&doc, id, Component::Polygon), [0, 1, 3, 4, 5]);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[select(0), EditorCommand::InvertSelection],
        );
        assert_eq!(selected(&doc, id, Component::Polygon), [1, 3, 4, 5]);
        run(&mut doc, &mut editor, &mut vp, &[select(2)]);
        assert!(
            selected(&doc, id, Component::Polygon).is_empty(),
            "a hidden face cannot be picked"
        );
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::UnhideAll, EditorCommand::SelectAll],
        );
        assert_eq!(selected(&doc, id, Component::Polygon).len(), 6);
        // Undo walks back through every step, the hide included.
        for _ in 0..5 {
            run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        }
        assert_eq!(selected(&doc, id, Component::Polygon), [0, 1, 3, 4, 5]);
    }

    #[test]
    fn make_editable_without_primitives_records_nothing() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        editor.selection.set_all([id_of(&doc, "Light"), id_of(&doc, "Group")]);
        assert!(!can_make_editable(&doc, &editor.selection));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        assert_eq!(editor.history.undo_label(), None);
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
    fn commit_live_records_live_edits() {
        let (mut doc, mut editor, mut vp) = demo();
        let sphere = id_of(&doc, "Sphere");
        let before = doc.scene.get(sphere).unwrap().data.clone();
        let commit = EditorCommand::CommitLive {
            inverse: SceneEdit::SetData {
                id: sphere,
                data: before.clone(),
            },
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
            &[EditorCommand::CommitLive {
                inverse: SceneEdit::SetData { id: null, data: before },
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
        // The document has unsaved changes: New asks first.
        assert_eq!(editor.file.pending(), Some(&FileAction::New));
        assert!(editor.history.can_undo());

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AnswerUnsaved(UnsavedChoice::Discard)],
        );
        assert!(!editor.history.can_undo() && !editor.history.can_redo());
        assert_eq!(editor.history.state_id(), 0);
        assert!(doc.scene.detached_roots().is_empty());
        assert!(doc.scene.is_empty());
        assert!(!editor.file.is_dirty(&editor.history));
    }

    #[test]
    fn edit_and_commit_live_cover_document_settings() {
        let (mut doc, mut editor, mut vp) = demo();
        let original = doc.time.settings();

        // A live change (the fps field while dragging), committed on release.
        doc.time.set_fps(48);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CommitLive {
                inverse: SceneEdit::SetTime(original),
                label: "Project Settings",
            }],
        );
        // A discrete change, applied by the command itself.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Edit {
                edit: SceneEdit::SetName("Shot 010".to_owned()),
                label: "Rename Project",
            }],
        );
        assert_eq!(doc.time.fps(), 48);
        assert_eq!(doc.name, "Shot 010");
        assert_eq!(editor.history.undo_len(), 2);

        // Committing the current state again records nothing.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CommitLive {
                inverse: SceneEdit::SetName("Shot 010".to_owned()),
                label: "Rename Project",
            }],
        );
        assert_eq!(editor.history.undo_len(), 2);

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert_eq!(doc.name, "Untitled 1");
        assert_eq!(doc.time.settings(), original);
    }

    #[test]
    fn a_commit_for_a_deleted_node_records_nothing() {
        let (mut doc, mut editor, mut vp) = demo();
        let sphere = id_of(&doc, "Sphere");
        let before = doc.scene.get(sphere).unwrap().data.clone();
        editor.selection.set_single(sphere);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CommitLive {
                inverse: SceneEdit::SetData {
                    id: sphere,
                    data: before,
                },
                label: "Move",
            }],
        );
        assert_eq!(editor.history.undo_label(), Some("Delete"));
        assert_eq!(editor.history.undo_len(), 1);
    }
}
