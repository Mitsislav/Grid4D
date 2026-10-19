//! Editor (session) state that is *not* part of the document: selection, the
//! active tool, modes and transient UI flags.

use grid4d_geom::{Component, loops::WalkOptions};
use grid4d_scene::{NodeId, Scene};
use slotmap::SecondaryMap;

use crate::{CommandQueue, History, file::DocumentFile};

/// The active manipulation tool (top toolbar).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    LiveSelection,
    RectangleSelection,
    LoopSelection,
    RingSelection,
    Move,
    Scale,
    Rotate,
}

impl Tool {
    pub const ALL: [Self; 7] = [
        Self::LiveSelection,
        Self::RectangleSelection,
        Self::LoopSelection,
        Self::RingSelection,
        Self::Move,
        Self::Scale,
        Self::Rotate,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LiveSelection => "Live Selection",
            Self::RectangleSelection => "Rectangle Selection",
            Self::LoopSelection => "Loop Selection",
            Self::RingSelection => "Ring Selection",
            Self::Move => "Move",
            Self::Scale => "Scale",
            Self::Rotate => "Rotate",
        }
    }

    /// Cinema 4D keyboard shortcut; empty until verified against Maxon's docs.
    #[must_use]
    pub const fn shortcut(self) -> &'static str {
        match self {
            Self::LiveSelection => "9",
            Self::RectangleSelection | Self::LoopSelection | Self::RingSelection => "",
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
            Self::RectangleSelection => {
                "Rectangle Selection: drag a frame around objects · Shift adds · Ctrl removes · Esc cancels"
            }
            Self::LoopSelection => "Loop Selection works on the points, edges or polygons of a polygon object",
            Self::RingSelection => "Ring Selection works on the points, edges or polygons of a polygon object",
            Self::Move => "Move: drag an axis handle to move along it · Alt+drag to navigate",
            Self::Scale => "Scale: drag an axis handle to scale along it · Alt+drag to navigate",
            Self::Rotate => "Rotate: drag a ring to rotate around its axis · Alt+drag to navigate",
        }
    }

    #[must_use]
    pub const fn has_gizmo(self) -> bool {
        matches!(self, Self::Move | Self::Scale | Self::Rotate)
    }

    /// The Select menu's tools (their options show in the Attribute Manager).
    #[must_use]
    pub const fn is_selection_tool(self) -> bool {
        !self.has_gizmo()
    }
}

/// Options of the selection tools, edited in the Attribute Manager's Tool mode.
/// Editor state: not saved, not undoable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionSettings {
    /// Live Selection brush radius, in points (middle-drag changes it).
    pub radius: f32,
    pub live_visible_only: bool,
    pub live_tolerant: bool,
    pub rectangle_visible_only: bool,
    pub rectangle_tolerant: bool,
    pub loop_options: WalkOptions,
    pub ring_options: WalkOptions,
    /// Grow Selection: polygons must share an edge (else sharing a point is enough).
    pub grow_edges_only: bool,
}

/// Options of the transform tools (Attribute Manager, Tool mode). Editor state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransformSettings {
    /// Move tool, component modes: with nothing selected, dragging a component
    /// moves just that component and leaves the selection alone.
    pub tweak: bool,
}

impl Default for SelectionSettings {
    fn default() -> Self {
        Self {
            // The reference's default radius is still to be verified (spec open question).
            radius: 10.0,
            live_visible_only: true,
            live_tolerant: false,
            rectangle_visible_only: true,
            rectangle_tolerant: false,
            loop_options: WalkOptions::default(),
            ring_options: WalkOptions::default(),
            grow_edges_only: true,
        }
    }
}

/// Component mode (left palette). Points, Edges and Polygons work on the active
/// object when it is a polygon object.
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

    /// The component kind a component mode works on; `None` in Model mode.
    #[must_use]
    pub const fn component(self) -> Option<Component> {
        match self {
            Self::Model => None,
            Self::Points => Some(Component::Point),
            Self::Edges => Some(Component::Edge),
            Self::Polygons => Some(Component::Polygon),
        }
    }
}

/// Ordered object selection. The last entry is the *active* object
/// (the one the Attribute Manager and gizmo work on).
///
/// `contains` is O(1): the renderer asks it once per node per frame, so a
/// linear scan would make "Select All" quadratic on the hot path. `members`
/// always holds exactly the ids in `ids`.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    ids: Vec<NodeId>,
    members: SecondaryMap<NodeId, ()>,
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        // `members` is derived from `ids`, so the ordered list decides equality.
        self.ids == other.ids
    }
}

impl Eq for Selection {}

impl Selection {
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.members.contains_key(id)
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
        self.clear();
        self.push(id);
    }

    /// Adds `id` and makes it the active object.
    pub fn add(&mut self, id: NodeId) {
        if self.contains(id) {
            self.ids.retain(|&x| x != id);
            self.ids.push(id);
        } else {
            self.push(id);
        }
    }

    pub fn remove(&mut self, id: NodeId) {
        if self.members.remove(id).is_some() {
            self.ids.retain(|&x| x != id);
        }
    }

    pub fn toggle(&mut self, id: NodeId) {
        if self.members.remove(id).is_some() {
            self.ids.retain(|&x| x != id);
        } else {
            self.push(id);
        }
    }

    pub fn clear(&mut self) {
        self.ids.clear();
        self.members.clear();
    }

    /// Replaces the selection; duplicates in `ids` are ignored.
    pub fn set_all(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.clear();
        for id in ids {
            if !self.contains(id) {
                self.push(id);
            }
        }
    }

    /// Drops ids that no longer exist in `scene`.
    pub fn retain_existing(&mut self, scene: &Scene) {
        self.ids.retain(|&id| scene.contains(id));
        self.members.retain(|id, ()| scene.contains(id));
    }

    /// Appends an id that is known not to be selected yet.
    fn push(&mut self, id: NodeId) {
        self.members.insert(id, ());
        self.ids.push(id);
    }
}

/// Everything the editor needs besides the document itself.
#[derive(Debug)]
pub struct EditorState {
    pub selection: Selection,
    pub tool: Tool,
    pub mode: EditMode,
    pub selection_settings: SelectionSettings,
    pub transform_settings: TransformSettings,
    /// X / Y / Z axis buttons. `true` = the axis is enabled (C4D semantics).
    pub axes: [bool; 3],
    /// World (`true`) or object (`false`) coordinate system for the gizmo.
    pub world_coordinates: bool,
    pub snapping: bool,
    pub commands: CommandQueue,
    /// Undo/redo steps; written only by `commands::apply`.
    pub history: History,
    /// The document's file: path, unsaved changes, dialogs, alerts.
    pub file: DocumentFile,
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
            selection_settings: SelectionSettings::default(),
            transform_settings: TransformSettings::default(),
            axes: [true; 3],
            world_coordinates: false,
            snapping: false,
            commands: CommandQueue::default(),
            history: History::default(),
            file: DocumentFile::default(),
            status_hint: None,
            renderer_info: String::new(),
            show_about: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{Node, ObjectKind};

    use super::*;

    fn scene_with(n: usize) -> (Scene, Vec<NodeId>) {
        let mut scene = Scene::new();
        let ids = (0..n)
            .map(|i| scene.insert(Node::new(format!("n{i}"), ObjectKind::Null)))
            .collect();
        (scene, ids)
    }

    /// The O(1) membership set must always agree with the ordered list.
    fn assert_consistent(sel: &Selection, all: &[NodeId]) {
        for &id in all {
            assert_eq!(sel.contains(id), sel.as_slice().contains(&id), "{id:?}");
        }
        let mut sorted = sel.as_slice().to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), sel.len(), "duplicate ids in selection");
    }

    #[test]
    fn membership_tracks_every_operation() {
        let (mut scene, ids) = scene_with(4);
        let [a, b, c, d] = [ids[0], ids[1], ids[2], ids[3]];
        let mut sel = Selection::default();

        sel.set_single(a);
        assert_consistent(&sel, &ids);
        sel.add(b);
        sel.add(a); // re-adding moves `a` to the end (active)
        assert_eq!(sel.as_slice(), &[b, a]);
        assert_eq!(sel.active(), Some(a));
        assert_consistent(&sel, &ids);

        sel.toggle(c);
        sel.toggle(b);
        assert_eq!(sel.as_slice(), &[a, c]);
        assert_consistent(&sel, &ids);

        sel.set_all([d, a, d]);
        assert_eq!(sel.as_slice(), &[d, a]);
        assert_consistent(&sel, &ids);

        scene.remove(d);
        sel.retain_existing(&scene);
        assert_eq!(sel.as_slice(), &[a]);
        assert!(!sel.contains(d));
        assert_consistent(&sel, &ids);

        sel.clear();
        assert!(sel.is_empty());
        assert_consistent(&sel, &ids);
    }

    #[test]
    fn stale_ids_are_never_members() {
        let (mut scene, ids) = scene_with(1);
        let mut sel = Selection::default();
        sel.set_single(ids[0]);
        scene.remove(ids[0]);
        // A new node may reuse the slot, but with a new generation.
        let fresh = scene.insert(Node::new("fresh", ObjectKind::Null));
        assert!(!sel.contains(fresh));
    }

    #[test]
    fn equality_ignores_the_membership_index() {
        let (_, ids) = scene_with(2);
        let mut x = Selection::default();
        let mut y = Selection::default();
        x.set_all([ids[0], ids[1]]);
        y.add(ids[0]);
        y.add(ids[1]);
        assert_eq!(x, y);
        y.add(ids[0]);
        assert_ne!(x, y, "order (and so the active object) matters");
    }
}
