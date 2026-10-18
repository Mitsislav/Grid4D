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
                _ => {}
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
        _ => palette::ICON_PRIMITIVE,
    }
}
