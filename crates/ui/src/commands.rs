//! Structural editor operations, queued by panels and applied once per frame.

use glam::Vec3;
use oc4d_scene::{Document, Node, NodeId, ObjectKindTag, Transform};

use crate::{EditorState, ViewportPanel, theme::palette};

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
    NewDocument,
    CreateObject(ObjectKindTag),
    DeleteSelected,
    Select { id: NodeId, mode: SelectMode },
    SelectAll,
    DeselectAll,
    Reparent { node: NodeId, new_parent: Option<NodeId> },
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
            *doc = Document::new("Untitled 1");
            editor.selection.clear();
            viewport.reset_view();
        }
        EditorCommand::CreateObject(tag) => {
            let name = doc.scene.unique_name(tag.label());
            let mut node = Node::new(name, tag.default_kind());
            match tag {
                // Like C4D: a new camera is created at the current view.
                ObjectKindTag::Camera => {
                    let cam = viewport.camera();
                    node.transform = Transform::from_position(cam.eye());
                    node.transform.rotation = Vec3::new(cam.yaw, -cam.pitch, 0.0);
                    node.display_color = [0.3, 0.3, 0.32];
                }
                ObjectKindTag::Light => {
                    node.transform.position = Vec3::new(0.0, 400.0, 0.0);
                    node.display_color = [1.0, 0.95, 0.85];
                }
                ObjectKindTag::Null
                | ObjectKindTag::Cube
                | ObjectKindTag::Sphere
                | ObjectKindTag::Cylinder
                | ObjectKindTag::Plane => {}
            }
            let id = doc.scene.insert(node);
            editor.selection.set_single(id);
        }
        EditorCommand::DeleteSelected => {
            for &id in editor.selection.as_slice() {
                doc.scene.remove(id);
            }
            editor.selection.clear();
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
            if let Err(err) = doc.scene.reparent(node, new_parent, true) {
                log::warn!("Reparent rejected: {err}");
            }
        }
        EditorCommand::FrameScene => viewport.frame_scene(&doc.scene),
        EditorCommand::FrameSelected => viewport.frame_selection(&doc.scene, &editor.selection),
        EditorCommand::ResetView => viewport.reset_view(),
    }
    editor.selection.retain_existing(&doc.scene);
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

    /// A headless viewport: without a wgpu render state it only keeps the camera.
    fn viewport() -> ViewportPanel {
        ViewportPanel::new(None, None, 1)
    }

    fn id_of(doc: &Document, name: &str) -> NodeId {
        doc.scene
            .iter()
            .find(|(_, n)| n.name == name)
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
        assert_eq!(doc.scene.get(second).unwrap().kind, ObjectKindTag::Cube.default_kind());
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
        assert!(camera.transform.position.abs_diff_eq(vp.camera().eye(), 1e-3));
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
        assert_ne!(*vp.camera(), oc4d_render::OrbitCamera::default());

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::NewDocument]);
        assert!(doc.scene.is_empty());
        assert!(editor.selection.is_empty());
        assert_eq!(*vp.camera(), oc4d_render::OrbitCamera::default());
    }
}
