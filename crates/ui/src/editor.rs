//! Editor (session) state that is *not* part of the document: selection, the
//! active tool, modes and transient UI flags.

use grid4d_geom::{
    Component,
    loops::WalkOptions,
    ops::{BevelOptions, BridgeOptions, ExtrudeOptions, InsetOptions, OptimizeOptions},
};
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
    /// Mesh ▸ Extrude (D): drag to extrude polygons or border edges.
    Extrude,
    /// Mesh ▸ Inset (I), once called Extrude Inner: drag to inset polygons.
    Inset,
    /// Mesh ▸ Loop/Path Cut (K~L): hover an edge to preview the loop, click to cut.
    LoopCut,
    /// Mesh ▸ Line Cut (K~K), the knife: drag a line across the object.
    LineCut,
    /// Mesh ▸ Bevel (M~S): drag to bevel the selected edges or points.
    Bevel,
    /// Mesh ▸ Bridge (B): drag from one polygon or border edge to another.
    Bridge,
    /// Mesh ▸ Weld: click to weld the selected points into one.
    Weld,
}

impl Tool {
    /// The top toolbar's tools (the modelling tools live in the Mesh menu).
    pub const TOOLBAR: [Self; 7] = [
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
            Self::Extrude => "Extrude",
            Self::Inset => "Inset",
            Self::LoopCut => "Loop/Path Cut",
            Self::LineCut => "Line Cut",
            Self::Bevel => "Bevel",
            Self::Bridge => "Bridge",
            Self::Weld => "Weld",
        }
    }

    /// Cinema 4D keyboard shortcut (verified against Maxon's shortcut list); `U~L`
    /// is a chord: U, then L.
    #[must_use]
    pub const fn shortcut(self) -> &'static str {
        match self {
            Self::LiveSelection => "9",
            Self::RectangleSelection => "0",
            Self::LoopSelection => "U~L",
            Self::RingSelection => "U~B",
            Self::LoopCut => "K~L",
            Self::LineCut => "K~K",
            Self::Bevel => "M~S",
            Self::Bridge => "B",
            Self::Weld => "",
            Self::Move => "E",
            Self::Scale => "T",
            Self::Rotate => "R",
            Self::Extrude => "D",
            Self::Inset => "I",
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
            Self::Extrude => "Extrude works on the polygons or border edges of a polygon object",
            Self::Inset => "Inset works on the polygons of a polygon object (Polygons mode)",
            Self::LoopCut => "Loop/Path Cut: in a component mode, hover an edge of a polygon object and click to cut",
            Self::LineCut => "Line Cut: in a component mode, drag a line across a polygon object",
            Self::Bevel => "Bevel works on the selected edges or points of a polygon object",
            Self::Bridge => "Bridge: in Polygons or Edges mode, drag from one polygon or border edge to another",
            Self::Weld => "Weld: select points, then click one of them (or anywhere else for their centre)",
        }
    }

    #[must_use]
    pub const fn has_gizmo(self) -> bool {
        matches!(self, Self::Move | Self::Scale | Self::Rotate)
    }

    /// The Select menu's tools.
    #[must_use]
    pub const fn is_selection_tool(self) -> bool {
        matches!(
            self,
            Self::LiveSelection | Self::RectangleSelection | Self::LoopSelection | Self::RingSelection
        )
    }

    /// The Mesh menu's modelling tools: drag in the viewport, or Apply.
    #[must_use]
    pub const fn is_mesh_tool(self) -> bool {
        matches!(self, Self::Extrude | Self::Inset | Self::Bevel)
    }

    /// The tools that join components: Bridge (drag between two) and Weld (click).
    #[must_use]
    pub const fn is_join_tool(self) -> bool {
        matches!(self, Self::Bridge | Self::Weld)
    }

    /// The cutting tools: Loop/Path Cut (click) and Line Cut (drag).
    #[must_use]
    pub const fn is_cut_tool(self) -> bool {
        matches!(self, Self::LoopCut | Self::LineCut)
    }

    /// The tool's options are worth showing in the Attribute Manager as it is picked.
    #[must_use]
    pub const fn shows_options(self) -> bool {
        self.is_selection_tool() || self.is_mesh_tool() || self.is_cut_tool() || self.is_join_tool()
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

/// Whether a snap moves the element in space or lines it up on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SnapType {
    /// 3D in perspective views, 2D in orthographic ones (the viewport is
    /// perspective, so 3D for now).
    #[default]
    Auto,
    /// Lined up with the target on screen; the element keeps its depth.
    TwoD,
    /// Onto the target's position in space.
    ThreeD,
}

impl SnapType {
    pub const ALL: [Self; 3] = [Self::Auto, Self::TwoD, Self::ThreeD];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto Snapping",
            Self::TwoD => "2D Snapping",
            Self::ThreeD => "3D Snapping",
        }
    }

    /// Whether a snap moves the element in space (Auto: the view is perspective).
    #[must_use]
    pub const fn in_space(self) -> bool {
        match self {
            Self::Auto | Self::ThreeD => true,
            Self::TwoD => false,
        }
    }
}

/// Snapping (Shift+S; see `docs/reference/modeling-snapping.md`). Editor state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapSettings {
    pub enabled: bool,
    pub kind: SnapType,
    /// Screen distance (points) within which a target is caught.
    pub radius: f32,
    pub targets: grid4d_render::SnapTargets,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: SnapType::Auto,
            radius: 10.0,
            targets: grid4d_render::SnapTargets::default(),
        }
    }
}

/// Options of the transform tools (Attribute Manager, Tool mode). Editor state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransformSettings {
    /// Move tool, component modes: with nothing selected, dragging a component
    /// moves just that component and leaves the selection alone.
    pub tweak: bool,
}

/// Options of the modelling tools (Attribute Manager, Tool mode). Editor state.
/// Angles in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelingSettings {
    /// Extrude; its offset, subdivision and n-gon option also apply to edges.
    pub extrude: ExtrudeOptions,
    /// Extrude in Edges mode: tilt of the new polygons around their edge.
    pub edge_rotation: f32,
    pub inset: InsetOptions,
    pub loop_cut: LoopCutSettings,
    pub line_cut: LineCutSettings,
    pub bevel: BevelOptions,
    pub bridge: BridgeOptions,
    pub optimize: OptimizeOptions,
}

/// Loop/Path Cut options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopCutSettings {
    /// Where a single cut crosses the edge, `0..1` from its reference end (the click sets it).
    pub offset: f32,
    /// Cuts, evenly spaced when more than one.
    pub cuts: u32,
    /// The new edges become the edge selection.
    pub select_cuts: bool,
}

/// Line Cut options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "independent options of the reference tool")]
pub struct LineCutSettings {
    pub visible_only: bool,
    pub infinite: bool,
    /// Split the polygons too (else only points on the cut edges).
    pub connect: bool,
    pub select_cuts: bool,
}

impl LoopCutSettings {
    /// Fractions along the edge for a cut at `offset`: the offset itself for one
    /// cut, evenly spaced otherwise.
    #[must_use]
    pub fn fractions(&self, offset: f32) -> Vec<f32> {
        if self.cuts <= 1 {
            return vec![offset.clamp(0.01, 0.99)];
        }
        let n = f32::from(u16::try_from(self.cuts).unwrap_or(u16::MAX));
        (1..=self.cuts)
            .map(|k| f32::from(u16::try_from(k).unwrap_or(u16::MAX)) / (n + 1.0))
            .collect()
    }
}

impl Default for ModelingSettings {
    fn default() -> Self {
        // The manual gives no defaults: these are ours (docs/reference/modeling-tools.md).
        let max_angle = 89f32.to_radians();
        Self {
            extrude: ExtrudeOptions {
                offset: 10.0,
                subdivision: 0,
                create_ngons: false,
                create_caps: false,
                preserve_groups: true,
                max_angle,
            },
            edge_rotation: 0.0,
            inset: InsetOptions {
                offset: 10.0,
                subdivision: 1,
                create_ngons: false,
                preserve_groups: true,
                max_angle,
            },
            loop_cut: LoopCutSettings {
                offset: 0.5,
                cuts: 1,
                select_cuts: true,
            },
            line_cut: LineCutSettings {
                visible_only: true,
                infinite: false,
                connect: true,
                select_cuts: true,
            },
            bevel: BevelOptions {
                offset: 10.0,
                subdivision: 0,
                depth: 1.0,
                limit: true,
            },
            bridge: BridgeOptions {
                subdivision: 0,
                spin: 0,
                delete_originals: true,
            },
            optimize: OptimizeOptions {
                merge_points: true,
                tolerance: 0.01,
                unused_points: true,
            },
        }
    }
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

/// Recording keys (timeline, Animate menu; see
/// `docs/reference/animation-recording.md`). Editor state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "independent toggles of the reference")]
pub struct AnimationSettings {
    /// Changes to animated parameters become keys at the current frame (Ctrl+F9).
    pub autokey: bool,
    /// The channels Record (F9) writes.
    pub position: bool,
    pub scale: bool,
    pub rotation: bool,
}

impl Default for AnimationSettings {
    fn default() -> Self {
        Self {
            autokey: false,
            position: true,
            scale: true,
            rotation: true,
        }
    }
}

impl AnimationSettings {
    /// Whether Record writes `param`.
    #[must_use]
    pub const fn records(&self, param: grid4d_scene::Param) -> bool {
        match param {
            grid4d_scene::Param::Position(_) => self.position,
            grid4d_scene::Param::Scale(_) => self.scale,
            grid4d_scene::Param::Rotation(_) => self.rotation,
        }
    }
}

/// Keys selected on the timeline: frames whose keys (of every selected object)
/// are selected. Editor state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeySelection {
    /// Sorted, no duplicates.
    pub frames: Vec<f64>,
    /// The last click was in the timeline's key strip: Delete removes keys.
    pub focused: bool,
}

impl KeySelection {
    #[must_use]
    pub fn contains(&self, frame: f64) -> bool {
        self.frames.binary_search_by(|f| f.total_cmp(&frame)).is_ok()
    }

    /// Adds `frame`, or removes it if it is selected.
    pub fn toggle(&mut self, frame: f64) {
        match self.frames.binary_search_by(|f| f.total_cmp(&frame)) {
            Ok(i) => {
                self.frames.remove(i);
            }
            Err(i) => self.frames.insert(i, frame),
        }
    }

    /// Adds `frame` (kept sorted).
    pub fn insert(&mut self, frame: f64) {
        if let Err(i) = self.frames.binary_search_by(|f| f.total_cmp(&frame)) {
            self.frames.insert(i, frame);
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

/// A tag selected in the Object Manager: the object and the kind of tag (an
/// object holds at most one of each kind).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TagRef {
    pub node: NodeId,
    pub kind: grid4d_scene::TagKind,
}

/// Everything the editor needs besides the document itself.
#[derive(Debug)]
pub struct EditorState {
    pub selection: Selection,
    pub tool: Tool,
    pub mode: EditMode,
    pub selection_settings: SelectionSettings,
    pub transform_settings: TransformSettings,
    pub modeling_settings: ModelingSettings,
    /// The first key of a two-key shortcut, waiting for the second (`U`, then `L`).
    pub chord: Option<crate::shortcuts::Chord>,
    /// X / Y / Z axis buttons. `true` = the axis is enabled (C4D semantics).
    pub axes: [bool; 3],
    /// World (`true`) or object (`false`) coordinate system for the gizmo.
    pub world_coordinates: bool,
    pub snap: SnapSettings,
    /// The tag shown in the Attribute Manager (only while its object is active).
    pub tag_selection: Option<TagRef>,
    pub animation: AnimationSettings,
    /// Keys selected on the timeline.
    pub keys: KeySelection,
    /// The frame temporary values were last checked against (see
    /// `commands::apply_pending`).
    pub seen_frame: Option<i32>,
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
    /// Mesh ▸ Optimize… dialog is open.
    pub show_optimize: bool,
}

impl EditorState {
    /// The selected tag and its index in its object's tags, if it is still there
    /// and its object is the active one.
    #[must_use]
    pub fn selected_tag(&self, doc: &grid4d_scene::Document) -> Option<(TagRef, usize)> {
        let tag = self.tag_selection?;
        if self.selection.active() != Some(tag.node) {
            return None;
        }
        let node = doc.scene.get(tag.node)?;
        let index = grid4d_scene::Tags::tag_index(node.data.tags.as_slice(), tag.kind)?;
        Some((tag, index))
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            selection: Selection::default(),
            tool: Tool::Move,
            mode: EditMode::Model,
            selection_settings: SelectionSettings::default(),
            transform_settings: TransformSettings::default(),
            modeling_settings: ModelingSettings::default(),
            chord: None,
            axes: [true; 3],
            world_coordinates: false,
            snap: SnapSettings::default(),
            tag_selection: None,
            animation: AnimationSettings::default(),
            keys: KeySelection::default(),
            seen_frame: None,
            commands: CommandQueue::default(),
            history: History::default(),
            file: DocumentFile::default(),
            status_hint: None,
            renderer_info: String::new(),
            show_about: false,
            show_optimize: false,
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
