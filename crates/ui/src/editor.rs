//! Editor (session) state that is *not* part of the document: selection, the
//! active tool, modes and transient UI flags.

use oc4d_scene::{NodeId, Scene};

use crate::CommandQueue;

/// The active manipulation tool (top toolbar).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    LiveSelection,
    Move,
    Scale,
    Rotate,
}

impl Tool {
    pub const ALL: [Self; 4] = [Self::LiveSelection, Self::Move, Self::Scale, Self::Rotate];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LiveSelection => "Live Selection",
            Self::Move => "Move",
            Self::Scale => "Scale",
            Self::Rotate => "Rotate",
        }
    }

    /// Cinema 4D keyboard shortcut.
    #[must_use]
    pub const fn shortcut(self) -> &'static str {
        match self {
            Self::LiveSelection => "9",
            Self::Move => "E",
            Self::Scale => "T",
            Self::Rotate => "R",
        }
    }

    /// Status-bar help text.
    #[must_use]
    pub const fn hint(self) -> &'static str {
        match self {
            Self::LiveSelection => "Live Selection: click an object to select it · Shift adds · Ctrl toggles",
            Self::Move => "Move: drag an axis handle to move along it · Alt+drag to navigate",
            Self::Scale => "Scale: drag an axis handle to scale along it · Alt+drag to navigate",
            Self::Rotate => "Rotate: drag a ring to rotate around its axis · Alt+drag to navigate",
        }
    }

    #[must_use]
    pub const fn has_gizmo(self) -> bool {
        !matches!(self, Self::LiveSelection)
    }
}

/// Component mode (left palette). Only `Model` is functional until meshes are editable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditMode {
    Model,
    Points,
    Edges,
    Polygons,
}

impl EditMode {
    pub const ALL: [Self; 4] = [Self::Model, Self::Points, Self::Edges, Self::Polygons];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Model => "Model",
            Self::Points => "Points",
            Self::Edges => "Edges",
            Self::Polygons => "Polygons",
        }
    }
}

/// Ordered object selection. The last entry is the *active* object
/// (the one the Attribute Manager and gizmo work on).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    ids: Vec<NodeId>,
}

impl Selection {
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.ids.contains(&id)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    #[must_use]
    pub fn as_slice(&self) -> &[NodeId] {
        &self.ids
    }

    #[must_use]
    pub fn active(&self) -> Option<NodeId> {
        self.ids.last().copied()
    }

    pub fn set_single(&mut self, id: NodeId) {
        self.ids.clear();
        self.ids.push(id);
    }

    /// Adds `id` and makes it the active object.
    pub fn add(&mut self, id: NodeId) {
        self.ids.retain(|&x| x != id);
        self.ids.push(id);
    }

    pub fn toggle(&mut self, id: NodeId) {
        if self.contains(id) {
            self.ids.retain(|&x| x != id);
        } else {
            self.ids.push(id);
        }
    }

    pub fn clear(&mut self) {
        self.ids.clear();
    }

    pub fn set_all(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.ids.clear();
        self.ids.extend(ids);
    }

    /// Drops ids that no longer exist in `scene`.
    pub fn retain_existing(&mut self, scene: &Scene) {
        self.ids.retain(|&id| scene.contains(id));
    }
}

/// Everything the editor needs besides the document itself.
#[derive(Debug)]
pub struct EditorState {
    pub selection: Selection,
    pub tool: Tool,
    pub mode: EditMode,
    /// X / Y / Z axis buttons. `true` = the axis is enabled (C4D semantics).
    pub axes: [bool; 3],
    /// World (`true`) or object (`false`) coordinate system for the gizmo.
    pub world_coordinates: bool,
    pub snapping: bool,
    pub commands: CommandQueue,
    /// One-line help for the status bar; set by whatever the pointer is over,
    /// cleared every frame.
    pub status_hint: Option<&'static str>,
    /// Graphics adapter description shown in the status bar.
    pub renderer_info: String,
    pub show_about: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            selection: Selection::default(),
            tool: Tool::Move,
            mode: EditMode::Model,
            axes: [true; 3],
            world_coordinates: false,
            snapping: false,
            commands: CommandQueue::default(),
            status_hint: None,
            renderer_info: String::new(),
            show_about: false,
        }
    }
}
