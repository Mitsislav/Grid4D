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
    /// Create ▸ Spline ▸ Pen: click to add spline points, drag to pull tangents.
    Pen,
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
            Self::Pen => "Pen",
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
            Self::Weld | Self::Pen => "",
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
            Self::Pen => {
                "Pen: click to add a point · drag to pull its tangents · click the first point to close · Esc finishes"
            }
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
        self.is_selection_tool()
            || self.is_mesh_tool()
            || self.is_cut_tool()
            || self.is_join_tool()
            || matches!(self, Self::Pen)
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

/// Options of the Pen (Attribute Manager, Tool mode). Editor state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PenSettings {
    /// The type of a spline the Pen starts.
    pub kind: grid4d_geom::spline::SplineType,
}

impl Default for PenSettings {
    fn default() -> Self {
        Self {
            kind: grid4d_geom::spline::SplineType::Bezier,
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
            // Parameters are keyed with their buttons or Autokeying.
            grid4d_scene::Param::Object(_) => false,
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

/// A key of an animated parameter (the F-Curve editor's selection).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveKey {
    pub node: NodeId,
    pub param: grid4d_scene::Param,
    pub frame: f64,
}

/// What the running program is, for Help ▸ About (see
/// `docs/reference/app-release.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    pub version: &'static str,
    /// The source commit (a short hash); empty outside a git checkout.
    pub commit: &'static str,
    /// The commit's date, `YYYY-MM-DD`.
    pub date: &'static str,
    /// The third-party licences (`assets/THIRD_PARTY_LICENSES.txt`).
    pub licences: &'static str,
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            commit: "",
            date: "",
            licences: "",
        }
    }
}

/// Command plugins' options (Plugins ▸): the dialog open, and the values each
/// command had when it last ran or was edited, for the next time (this
/// session only).
#[derive(Debug, Default)]
pub struct PluginCommands {
    /// The type id of the command whose options dialog is open.
    pub open: Option<String>,
    options: Vec<grid4d_scene::PluginObject>,
}

impl PluginCommands {
    /// `plugin`'s options, at their defaults the first time.
    pub fn options_mut(&mut self, plugin: &dyn grid4d_plugin::CommandPlugin) -> &mut grid4d_scene::PluginObject {
        let index = match self.options.iter().position(|o| o.type_id == plugin.type_id()) {
            Some(index) => index,
            None => {
                self.options.push(grid4d_plugin::Registry::new_object(plugin));
                self.options.len() - 1
            }
        };
        &mut self.options[index]
    }
}

/// What H or S over the F-Curve graph asked for (applied by the panel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameCurves {
    All,
    Selected,
}

/// The F-Curve editor's state that other parts of the editor act on. Editor state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CurveSelection {
    /// Selected keys.
    pub keys: Vec<CurveKey>,
    /// Tracks shown (picked in the track list); empty: every track of the
    /// selected objects.
    pub tracks: Vec<(NodeId, grid4d_scene::Param)>,
    /// The last click was in the graph: Delete removes keys.
    pub focused: bool,
    /// The pointer is over the graph (last frame): H and S frame the curves.
    pub hovered: bool,
    /// A framing request from the keyboard.
    pub frame: Option<FrameCurves>,
}

impl CurveSelection {
    #[must_use]
    pub fn contains(&self, node: NodeId, param: grid4d_scene::Param, frame: f64) -> bool {
        self.keys
            .iter()
            .any(|k| k.node == node && k.param == param && k.frame == frame)
    }

    /// Adds the key, or removes it if it is selected.
    pub fn toggle(&mut self, key: CurveKey) {
        let before = self.keys.len();
        self.keys
            .retain(|k| !(k.node == key.node && k.param == key.param && k.frame == key.frame));
        if self.keys.len() == before {
            self.keys.push(key);
        }
    }

    /// Adds the key if it is not selected.
    pub fn insert(&mut self, key: CurveKey) {
        if !self.contains(key.node, key.param, key.frame) {
            self.keys.push(key);
        }
    }
}

/// A material being dragged from the Material Manager (onto an object in the
/// Object Manager or the viewport). Editor state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialDrag {
    pub material: grid4d_scene::MaterialId,
}

/// The Material Manager's state that other panels act on. Editor state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaterialSelection {
    /// The selected material.
    pub selected: Option<grid4d_scene::MaterialId>,
    /// The Attribute Manager shows the selected material (rather than an object).
    pub shown: bool,
    /// The last click was in the Material Manager: Delete removes the material.
    pub focused: bool,
    /// A material on its way to an object.
    pub drag: Option<MaterialDrag>,
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

/// A tag selected in the Object Manager: the object, the kind of tag and
/// which of the object's tags of that kind (0 for the first; only Material
/// tags come several to an object).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TagRef {
    pub node: NodeId,
    pub kind: grid4d_scene::TagKind,
    pub nth: usize,
}

impl TagRef {
    /// Where this tag is in `tags`, if it is there.
    #[must_use]
    pub fn index_in(&self, tags: &[grid4d_scene::Tag]) -> Option<usize> {
        tags.iter()
            .enumerate()
            .filter(|(_, t)| t.kind() == self.kind)
            .nth(self.nth)
            .map(|(i, _)| i)
    }

    /// The tag at `index` of `node`'s `tags`.
    #[must_use]
    pub fn at(node: NodeId, tags: &[grid4d_scene::Tag], index: usize) -> Option<Self> {
        let kind = tags.get(index)?.kind();
        let nth = tags[..index].iter().filter(|t| t.kind() == kind).count();
        Some(Self { node, kind, nth })
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
    pub modeling_settings: ModelingSettings,
    pub pen: PenSettings,
    /// The spline the Pen finished (Esc, Enter, closing it): the next Pen click
    /// starts a new spline even while it is active.
    pub pen_finished: Option<NodeId>,
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
    /// The F-Curve editor's selection and requests.
    pub curves: CurveSelection,
    /// The Material Manager's selection, focus and drag.
    pub materials: MaterialSelection,
    /// The document's texture pictures, loaded in the background.
    pub textures: crate::textures::TextureCache,
    /// The materials' preview pictures, rendered in the background.
    pub previews: crate::previews::MaterialPreviews,
    /// The texture selected in the Texture Manager.
    pub texture_selection: Option<grid4d_scene::TextureId>,
    /// The object the viewport looks through (Cameras ▸ Use Camera); `None`: the
    /// view's own editor camera.
    pub view_camera: Option<NodeId>,
    /// Which docked panes were drawn, which is under the pointer, and
    /// requests to show a pane or switch the layout.
    pub panes: crate::workspace::PaneState,
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
    /// The About window shows the licences instead of the shortcuts.
    pub show_licences: bool,
    /// What the running program is (Help ▸ About); filled in by the binary.
    pub build: BuildInfo,
    /// Mesh ▸ Optimize… dialog is open.
    pub show_optimize: bool,
    /// Plugins ▸: the command whose options dialog is open, and each
    /// command's options as last set.
    pub plugin_commands: PluginCommands,
    /// The script library, the Script Manager's script and its console.
    pub scripts: crate::scripts::Scripts,
    /// The Render Settings window is open (Ctrl+B).
    pub show_render_settings: bool,
    /// The session's renders and the Picture Viewer (Shift+R).
    pub renders: crate::rendering::Renders,
    /// The rigid body simulation and its cache.
    pub simulation: crate::simulation::Sim,
    /// The user's preferences (Edit ▸ Preferences…); the binary loads and saves them.
    pub preferences: crate::preferences::Preferences,
    /// The Preferences window is open.
    pub show_preferences: bool,
    /// Recovery copies of unsaved work, and one offered after a crash.
    pub autosave: crate::autosave::Autosave,
    /// A crash report written since the last start, pointed to once.
    pub crash: crate::crash::CrashReports,
    /// An object being dragged from the Object Manager (other panels may take
    /// it: the Layer Manager).
    pub object_drag: Option<NodeId>,
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
        let index = tag.index_in(&node.data.tags)?;
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
            pen: PenSettings::default(),
            pen_finished: None,
            chord: None,
            axes: [true; 3],
            world_coordinates: false,
            snap: SnapSettings::default(),
            tag_selection: None,
            animation: AnimationSettings::default(),
            keys: KeySelection::default(),
            curves: CurveSelection::default(),
            materials: MaterialSelection::default(),
            textures: crate::textures::TextureCache::default(),
            previews: crate::previews::MaterialPreviews::default(),
            texture_selection: None,
            view_camera: None,
            panes: crate::workspace::PaneState::default(),
            seen_frame: None,
            commands: CommandQueue::default(),
            history: History::default(),
            file: DocumentFile::default(),
            status_hint: None,
            renderer_info: String::new(),
            show_about: false,
            show_licences: false,
            build: BuildInfo::default(),
            show_optimize: false,
            plugin_commands: PluginCommands::default(),
            scripts: crate::scripts::Scripts::default(),
            show_render_settings: false,
            renders: crate::rendering::Renders::default(),
            simulation: crate::simulation::Sim::default(),
            preferences: crate::preferences::Preferences::default(),
            show_preferences: false,
            autosave: crate::autosave::Autosave::default(),
            crash: crate::crash::CrashReports::default(),
            object_drag: None,
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
