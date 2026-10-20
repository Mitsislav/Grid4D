//! The 3D viewport: header menus, navigation, picking (objects, or components
//! of the active polygon object in a component mode), gizmo and the GPU view.

mod cut_tools;
mod gizmo;
mod gpu_view;
mod join_tools;
mod mesh_tools;
mod overlay;
mod select_tools;
mod snapping;

use egui::{Align, Align2, Layout, Margin, PointerButton, Pos2, Rect, Response, Sense, Ui, pos2};
use glam::{Mat4, Vec2 as GVec2, Vec3};
use grid4d_eval::EvaluatedScene;
use grid4d_geom::Component;
use grid4d_render::{
    ComponentHit, ComponentStyle, FrameData, OrbitCamera, Projection, ScreenView, SnapCache, SnapHit, pick,
    pick_component,
};
use grid4d_scene::{Aabb, Document, NodeData, NodeId, ObjectKind, ObjectKindTag, Scene, Tags, Transform, WalkItem};

use self::{gizmo::GizmoGeometry, gizmo::GizmoState, gpu_view::GpuView, select_tools::Gesture};
use crate::{
    EditorCommand, EditorState, SelectMode, Selection, Tool,
    icons::Icon,
    theme::{metrics, palette as p},
    widgets::{IconStyle, icon_button},
};

const ORBIT_SPEED: f32 = 0.0085;
const DOLLY_DRAG_SPEED: f32 = 0.005;
const DOLLY_WHEEL_SPEED: f32 = 0.0015;
const NAV_HINT: &str = "Alt+Left: orbit · Alt+Middle / Middle: pan · Alt+Right / Wheel: zoom · H: frame scene";

/// What a view looks at: in perspective, or straight along a world axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    Perspective,
    Top,
    Bottom,
    Front,
    Back,
    Left,
    Right,
}

impl ViewKind {
    pub const ALL: [Self; 7] = [
        Self::Perspective,
        Self::Top,
        Self::Bottom,
        Self::Front,
        Self::Back,
        Self::Left,
        Self::Right,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Perspective => "Perspective",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
            Self::Front => "Front",
            Self::Back => "Back",
            Self::Left => "Left",
            Self::Right => "Right",
        }
    }

    /// `camera` turned to look this way (target and zoom kept). Front looks
    /// along −Z, Right along −X, Top down −Y (with −Z up on screen).
    #[must_use]
    pub fn aim(self, camera: OrbitCamera) -> OrbitCamera {
        use std::f32::consts::{FRAC_PI_2, PI};
        let (yaw, pitch) = match self {
            Self::Perspective => {
                let default = OrbitCamera::default();
                (default.yaw, default.pitch)
            }
            Self::Front => (0.0, 0.0),
            Self::Back => (PI, 0.0),
            Self::Right => (FRAC_PI_2, 0.0),
            Self::Left => (PI + FRAC_PI_2, 0.0),
            Self::Top => (0.0, FRAC_PI_2),
            Self::Bottom => (0.0, -FRAC_PI_2),
        };
        OrbitCamera {
            yaw,
            pitch,
            roll: 0.0,
            projection: match self {
                Self::Perspective => Projection::Perspective,
                Self::Top | Self::Bottom | Self::Front | Self::Back | Self::Left | Self::Right => Projection::Parallel,
            },
            ..camera
        }
    }
}

/// How the panel's views are arranged (the Panel menu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Arrangement {
    /// The active view alone.
    #[default]
    Single,
    /// Views 1 and 2, one above the other.
    TwoStacked,
    /// Views 1 and 2 next to each other.
    TwoSideBySide,
    /// Views 1–4 in a grid.
    Four,
}

impl Arrangement {
    pub const ALL: [Self; 4] = [Self::Single, Self::TwoStacked, Self::TwoSideBySide, Self::Four];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Single => "Single View",
            Self::TwoStacked => "2 Views Stacked",
            Self::TwoSideBySide => "2 Views Side-by-Side",
            Self::Four => "4 Views",
        }
    }
}

/// One view's own settings (the active view's live in the panel's fields).
#[derive(Clone, Copy, Debug, PartialEq)]
struct ViewState {
    camera: OrbitCamera,
    kind: ViewKind,
    show_grid: bool,
    show_axis_triad: bool,
    show_hud: bool,
    rect: Rect,
}

/// The views a fresh panel has: View 1 Perspective, 2 Top, 3 Right, 4 Front.
const DEFAULT_VIEWS: [ViewKind; 4] = [ViewKind::Perspective, ViewKind::Top, ViewKind::Right, ViewKind::Front];
/// Space between views, in points.
const VIEW_GAP: f32 = 2.0;

#[derive(Debug)]
pub struct ViewportPanel {
    camera: OrbitCamera,
    /// The active view's projection.
    kind: ViewKind,
    /// Every view's settings; the active view's entry is stale (its live state is
    /// in the panel's own fields).
    views: [ViewState; 4],
    active: usize,
    arrangement: Arrangement,
    /// A view shown alone (F1–F4, a middle click), whatever the arrangement.
    solo: Option<usize>,
    /// Navigation moved the linked camera this frame.
    camera_moved: bool,
    /// What the document looks like this frame; re-evaluated only when it changed.
    evaluated: EvaluatedScene,
    /// Render snapshot, refilled every frame (no per-frame allocation).
    frame: FrameData,
    /// Renderer and offscreen target; `None` without a wgpu render state.
    gpu: Option<GpuView>,
    walk_stack: Vec<WalkItem>,
    gizmo: GizmoState,
    /// What a click would select in a component mode (pre-selection): the
    /// component under the cursor, or the loop/ring of the loop tools.
    hover: Vec<ComponentHit>,
    /// A Live Selection stroke or Rectangle Selection frame in progress.
    gesture: Gesture,
    show_grid: bool,
    show_axis_triad: bool,
    show_hud: bool,
    /// The 3D area last frame (the header's pan handle needs its height).
    view_rect: Rect,
    /// Loop/Path Cut: the loop a click would cut.
    loop_preview: Option<cut_tools::LoopPreview>,
    /// Primitives' meshes for snapping.
    snap_cache: SnapCache,
    /// The target the current drag is snapped to, shown in the view.
    snap_hit: Option<SnapHit>,
    /// The object the view looked through last frame (`EditorState::view_camera`).
    linked: Option<NodeId>,
    /// The editor camera, kept while the view looks through an object.
    editor_camera: Option<OrbitCamera>,
    /// The view as last taken from (or written to) the linked object: a different
    /// camera at the start of a frame was moved outside `show` (framing).
    synced: Option<OrbitCamera>,
    /// Navigation moving the linked object: its data before, for one undo step.
    camera_edit: Option<(NodeId, NodeData)>,
}

impl ViewportPanel {
    /// Creates the panel. The viewport renders into its own offscreen target
    /// (own MSAA and depth), so nothing about the window's surface matters here.
    /// Without a render state (tests, a non-wgpu backend) it shows a notice.
    pub fn new(render_state: Option<&egui_wgpu::RenderState>) -> Self {
        if render_state.is_none() {
            log::error!("No wgpu render state: the 3D viewport is disabled. Run with the wgpu renderer.");
        }
        let view = |kind: ViewKind| ViewState {
            camera: kind.aim(OrbitCamera::default()),
            kind,
            show_grid: true,
            show_axis_triad: true,
            show_hud: true,
            rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)),
        };
        Self {
            camera: OrbitCamera::default(),
            kind: ViewKind::Perspective,
            views: DEFAULT_VIEWS.map(view),
            active: 0,
            arrangement: Arrangement::Single,
            solo: None,
            camera_moved: false,
            evaluated: EvaluatedScene::new(),
            frame: FrameData::new(),
            gpu: render_state.map(GpuView::new),
            walk_stack: Vec::with_capacity(64),
            gizmo: GizmoState::default(),
            hover: Vec::new(),
            gesture: Gesture::None,
            show_grid: true,
            show_axis_triad: true,
            show_hud: true,
            view_rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)),
            loop_preview: None,
            snap_cache: SnapCache::default(),
            snap_hit: None,
            linked: None,
            editor_camera: None,
            synced: None,
            camera_edit: None,
        }
    }

    #[must_use]
    pub fn camera(&self) -> &OrbitCamera {
        &self.camera
    }

    pub fn reset_view(&mut self) {
        self.camera = OrbitCamera::default();
    }

    pub fn frame_scene(&mut self, scene: &Scene) {
        let bounds = scene.world_bounds(&mut self.walk_stack, |_| true);
        self.frame_bounds(bounds);
    }

    /// Frames the selected objects including their children.
    pub fn frame_selection(&mut self, scene: &Scene, selection: &Selection) {
        if selection.is_empty() {
            self.frame_scene(scene);
            return;
        }
        let mut bounds = Aabb::EMPTY;
        let mut selected_depth: Option<u32> = None;
        scene.walk(&mut self.walk_stack, |id, node, world, depth| {
            if selected_depth.is_some_and(|d| depth <= d) {
                selected_depth = None;
            }
            if selected_depth.is_none() && selection.contains(id) {
                selected_depth = Some(depth);
            }
            if selected_depth.is_some() {
                bounds = bounds.union(node.world_bounds(world));
            }
            true
        });
        self.frame_bounds(bounds);
    }

    fn frame_bounds(&mut self, bounds: Aabb) {
        if bounds.is_empty() {
            self.reset_view();
        } else {
            self.camera.frame_sphere(bounds.center(), bounds.radius());
        }
    }

    /// The active view's projection.
    #[must_use]
    pub fn view_kind(&self) -> ViewKind {
        self.kind
    }

    /// The view arrangement and the view shown alone, if any.
    #[must_use]
    pub fn layout(&self) -> (Arrangement, Option<usize>) {
        (self.arrangement, self.solo)
    }

    /// Shows view `index` alone (F1–F4) and makes it active; `None`: all four views
    /// (F5).
    pub fn show_views(&mut self, index: Option<usize>) {
        match index {
            Some(i) if i < self.views.len() => {
                self.set_active(i);
                self.solo = Some(i);
            }
            Some(_) => {}
            None => {
                self.arrangement = Arrangement::Four;
                self.solo = None;
            }
        }
    }

    /// The active view's settings as a [`ViewState`].
    fn current(&self) -> ViewState {
        ViewState {
            camera: self.camera,
            kind: self.kind,
            show_grid: self.show_grid,
            show_axis_triad: self.show_axis_triad,
            show_hud: self.show_hud,
            rect: self.view_rect,
        }
    }

    fn load(&mut self, view: ViewState) {
        self.camera = view.camera;
        self.kind = view.kind;
        self.show_grid = view.show_grid;
        self.show_axis_triad = view.show_axis_triad;
        self.show_hud = view.show_hud;
        self.view_rect = view.rect;
    }

    /// Exchanges the panel's fields with view `index`'s (a no-op for the active
    /// view): call once to work on that view, and again to put it back.
    fn swap_view(&mut self, index: usize) {
        if index == self.active || index >= self.views.len() {
            return;
        }
        let other = self.views[index];
        self.views[index] = self.current();
        self.load(other);
    }

    /// Makes view `index` the active one (tools, picking and the gizmo work in it).
    fn set_active(&mut self, index: usize) {
        if index == self.active || index >= self.views.len() {
            return;
        }
        self.views[self.active] = self.current();
        let next = self.views[index];
        self.load(next);
        self.active = index;
    }

    /// The views on show and where: up to four (view, cell).
    fn cells(&self, area: Rect) -> [Option<(usize, Rect)>; 4] {
        let half = |a: f32, b: f32| (a + b) * 0.5;
        let (mid_x, mid_y) = (half(area.left(), area.right()), half(area.top(), area.bottom()));
        let g = VIEW_GAP * 0.5;
        let left = Rect::from_min_max(area.min, pos2(mid_x - g, area.bottom()));
        let right = Rect::from_min_max(pos2(mid_x + g, area.top()), area.max);
        let top = Rect::from_min_max(area.min, pos2(area.right(), mid_y - g));
        let bottom = Rect::from_min_max(pos2(area.left(), mid_y + g), area.max);
        if let Some(solo) = self.solo {
            return [Some((solo, area)), None, None, None];
        }
        match self.arrangement {
            Arrangement::Single => [Some((self.active, area)), None, None, None],
            Arrangement::TwoStacked => [Some((0, top)), Some((1, bottom)), None, None],
            Arrangement::TwoSideBySide => [Some((0, left)), Some((1, right)), None, None],
            Arrangement::Four => [
                Some((0, Rect::from_min_max(area.min, pos2(mid_x - g, mid_y - g)))),
                Some((
                    1,
                    Rect::from_min_max(pos2(mid_x + g, area.top()), pos2(area.right(), mid_y - g)),
                )),
                Some((
                    2,
                    Rect::from_min_max(pos2(area.left(), mid_y + g), pos2(mid_x - g, area.bottom())),
                )),
                Some((3, Rect::from_min_max(pos2(mid_x + g, mid_y + g), area.max))),
            ],
        }
    }

    pub fn show(&mut self, ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
        let area = ui.available_rect_before_wrap();
        ui.allocate_rect(area, Sense::hover());
        if area.width() < 2.0 || area.height() < 2.0 {
            return;
        }
        let mut cells = self.cells(area);
        // The active view must be on show.
        if !cells.iter().flatten().any(|&(i, _)| i == self.active)
            && let Some((first, _)) = cells.iter().flatten().next().copied()
        {
            self.set_active(first);
            cells = self.cells(area);
        }
        // A press or a scroll in another view makes it active, and acts there. The
        // press's own position: the pointer may have left by the end of the frame.
        let at = ui.input(|i| {
            let press = i.events.iter().find_map(|event| match event {
                egui::Event::PointerButton { pos, pressed: true, .. } => Some(*pos),
                _ => None,
            });
            let scrolled = i.smooth_scroll_delta != egui::Vec2::ZERO;
            press.or_else(|| i.pointer.latest_pos().filter(|_| scrolled))
        });
        if let Some(pos) = at
            && let Some((i, _)) = cells.iter().flatten().find(|(_, cell)| cell.contains(pos))
        {
            self.set_active(*i);
        }
        // View 1 may look through a scene camera.
        self.swap_view(0);
        self.follow_view_camera(doc, editor);
        self.swap_view(0);

        let several = cells.iter().flatten().count() > 1;
        self.camera_moved = false;
        for (index, cell) in cells.into_iter().flatten() {
            let interactive = index == self.active;
            self.swap_view(index);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(cell).id_salt(("grid4d.view", index)));
            self.show_view(&mut child, doc, editor, index, interactive);
            self.swap_view(index);
            if several && interactive {
                ui.painter().rect_stroke(
                    cell.expand(0.5),
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(1.0, p::ACCENT.gamma_multiply(0.8)),
                    egui::StrokeKind::Outside,
                );
            }
        }
        // A gesture that moved the linked object has ended: one undo step.
        if !self.camera_moved && !ui.input(|i| i.pointer.any_down()) {
            self.finish_camera_edit(editor);
        }
    }

    /// One view: its header, then the 3D area. Only the active view (`interactive`)
    /// takes input; the others are drawn.
    fn show_view(
        &mut self,
        ui: &mut Ui,
        doc: &mut Document,
        editor: &mut EditorState,
        index: usize,
        interactive: bool,
    ) {
        self.header(ui, doc, editor, index);

        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return;
        }
        self.view_rect = rect;
        let painter = ui.painter_at(rect);
        if !interactive {
            self.paint_view(ui, &painter, rect, doc, editor, index, None);
            return;
        }

        // A middle click (no drag) shows the view alone, or the arrangement again.
        if response.clicked_by(PointerButton::Middle) {
            self.solo = match self.solo {
                Some(_) => None,
                None if self.arrangement != Arrangement::Single => Some(index),
                None => None,
            };
        }

        // With Live Selection in a component mode, a middle drag sizes the brush.
        let brush = (editor.tool == Tool::LiveSelection && editor.mode.component().is_some())
            .then_some(&mut editor.selection_settings.radius);
        let navigating = self.navigate(ui, &response, rect, brush);
        // Navigation (or the header's icons) moved a linked camera: move the object.
        let moved_view = index == 0 && self.synced.is_some_and(|s| s != self.camera);
        if moved_view && let Some(id) = self.linked {
            self.write_view_camera(doc, editor, id);
            self.camera_moved = true;
        }
        // Built once per frame (after navigation moved the camera) and shared by
        // interaction and painting; rebuilt only if a drag just moved the object.
        let keying = crate::keys::Keying::of(doc, editor);
        let mut gizmo_geo = GizmoGeometry::new(&doc.scene, editor, &self.camera, rect, keying.frame);
        self.snap_hit = None;
        let gizmo_owns_pointer = if navigating {
            false
        } else {
            let mut snapper = snapping::Snapper {
                settings: editor.snap,
                scene: &self.evaluated,
                view: self.screen_view(rect),
                rect,
                cache: &mut self.snap_cache,
                grid_spacing: self.camera.grid_spacing(),
                hit: None,
            };
            let interaction = self.gizmo.interact(
                ui,
                &response,
                gizmo_geo.as_ref(),
                &mut doc.scene,
                &mut editor.commands,
                &mut snapper,
                keying,
            );
            self.snap_hit = snapper.hit;
            if interaction.moved_object {
                gizmo_geo = GizmoGeometry::new(&doc.scene, editor, &self.camera, rect, keying.frame);
            }
            interaction.owns_pointer
        };
        if !navigating && !gizmo_owns_pointer {
            self.evaluated.set_frame(f64::from(doc.time.frame()));
            self.evaluated.update(&doc.scene);
            if !self.mesh_tools(ui, &response, rect, doc, editor)
                && !self.cut_tools(ui, &response, rect, editor)
                && !self.join_tools(ui, &response, rect, editor)
                && !self.selection_tools(ui, &response, rect, doc, editor)
            {
                self.pick_on_click(ui, &response, rect, editor);
            }
        }
        self.context_menu(&response, editor);

        // A gizmo drag above may have moved an object; free if nothing changed.
        // Animation is shown at the current (whole) frame.
        self.evaluated.set_frame(f64::from(doc.time.frame()));
        self.evaluated.update(&doc.scene);
        self.drop_material(ui, rect, editor);
        // Component modes: the active polygon object, and what a click would select.
        let target = self.component_target(editor);
        self.update_hover(target, &response, rect, editor, navigating);
        self.paint_view(ui, &painter, rect, doc, editor, index, gizmo_geo.as_ref());
        let view_proj = self.camera.view_projection(rect.aspect_ratio());
        match &self.gesture {
            Gesture::Frame { start, end } => overlay::marquee(&painter, Rect::from_two_pos(*start, *end)),
            Gesture::LineCut { start, end, .. } | Gesture::Bridge { start, end, .. } => {
                overlay::cut_line(&painter, *start, *end);
            }
            Gesture::None | Gesture::Stroke(_) | Gesture::Tweak { .. } | Gesture::MeshTool(_) | Gesture::Cancelled => {}
        }
        if let Some(preview) = &self.loop_preview {
            overlay::cut_preview(&painter, rect, &view_proj, &preview.segments);
        }
        if let Some(hit) = &self.snap_hit {
            overlay::snap_target(&painter, rect, &view_proj, hit);
        }
        if target.is_some()
            && editor.tool == Tool::LiveSelection
            && let Some(pos) = response.hover_pos()
        {
            overlay::brush(&painter, pos, editor.selection_settings.radius);
        }

        if response.hovered() {
            let alt = ui.input(|i| i.modifiers.alt);
            editor.status_hint = Some(match (alt, editor.mode.component()) {
                (true, _) => NAV_HINT,
                (false, Some(component)) => component_hint(component, editor.tool, target.is_some()),
                (false, None) => editor.tool.hint(),
            });
        }
    }

    /// Draws a view: the rendered scene, then the overlays every view shows (title,
    /// frustums, brackets, the gizmo, triad, HUD). `gizmo`: the active view's,
    /// already built; other views build their own (drawn, not used).
    #[expect(clippy::too_many_arguments, reason = "the pieces of one view's frame")]
    fn paint_view(
        &mut self,
        ui: &Ui,
        painter: &egui::Painter,
        rect: Rect,
        doc: &Document,
        editor: &EditorState,
        index: usize,
        gizmo: Option<&GizmoGeometry>,
    ) {
        overlay::background(painter, rect);
        self.evaluated.set_frame(f64::from(doc.time.frame()));
        self.evaluated.update(&doc.scene);
        let target = self.component_target(editor);
        let interactive = index == self.active;
        if let Some(gpu) = &mut self.gpu {
            self.frame.set_camera(&self.camera, rect.aspect_ratio());
            self.frame.show_grid = self.show_grid;
            self.frame.extract(&self.evaluated, |id| editor.selection.contains(id));
            self.frame.component_style = component_style(ui.ctx().pixels_per_point());
            // What a click would select shows where the pointer is.
            let hover: &[ComponentHit] = if interactive { &self.hover } else { &[] };
            self.frame.extract_components(&self.evaluated, target, hover);
            let texture = gpu.render(index, &self.frame, pixel_size(rect, ui.ctx().pixels_per_point()));
            // Drawn over the background gradient: the target is cleared to transparent.
            painter.image(
                texture,
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "3D viewport unavailable: wgpu renderer not initialised",
                egui::FontId::proportional(14.0),
                p::TEXT_DIM,
            );
        }

        let linked = self.linked.filter(|_| index == 0);
        let label = linked
            .and_then(|id| doc.scene.get(id))
            .map_or_else(|| self.kind.label().to_owned(), |n| n.data.name.clone());
        overlay::view_label(painter, rect, &label);
        if editor.animation.autokey {
            overlay::autokey_frame(painter, rect);
        }
        let view_proj = self.camera.view_projection(rect.aspect_ratio());
        overlay::camera_frustums(
            painter,
            rect,
            &view_proj,
            &self.evaluated,
            &doc.scene,
            &editor.selection,
            linked,
        );
        overlay::selection_brackets(painter, rect, &view_proj, &self.evaluated, &editor.selection);
        let own;
        let gizmo = match gizmo {
            Some(geo) => Some(geo),
            None if !interactive => {
                own = GizmoGeometry::new(&doc.scene, editor, &self.camera, rect, f64::from(doc.time.frame()));
                own.as_ref()
            }
            None => None,
        };
        if let Some(geo) = gizmo {
            geo.paint(painter, if interactive { self.gizmo.hot_axis() } else { None });
        }
        if self.show_axis_triad {
            overlay::axis_triad(painter, rect, &self.camera);
        }
        if self.show_hud {
            overlay::hud(painter, rect, &self.camera);
        }
    }

    /// The active object and the component kind of the current mode, when the
    /// mode is a component mode and the active object a visible polygon object.
    fn component_target(&self, editor: &EditorState) -> Option<(NodeId, Component)> {
        let component = editor.mode.component()?;
        let id = editor.selection.active()?;
        self.evaluated.polygon(id)?;
        Some((id, component))
    }

    /// Recomputes what a click would select (none while navigating or dragging).
    fn update_hover(
        &mut self,
        target: Option<(NodeId, Component)>,
        response: &Response,
        rect: Rect,
        editor: &EditorState,
        navigating: bool,
    ) {
        self.hover.clear();
        let (Some((node, component)), Some(pos)) = (target, response.hover_pos()) else {
            return;
        };
        if navigating || response.dragged() || !matches!(self.gesture, Gesture::None) {
            return;
        }
        let cursor = local(pos, rect);
        let wrap = |i: u32| match component {
            Component::Point => ComponentHit::Point(i),
            Component::Edge => ComponentHit::Edge(i),
            Component::Polygon => ComponentHit::Polygon(i),
        };
        match editor.tool {
            Tool::LoopSelection | Tool::RingSelection => {
                let hits = self.loop_hits(rect, node, component, editor, cursor);
                self.hover.extend(hits.into_iter().map(wrap));
            }
            // The brush circle shows what Live Selection will paint; the cut
            // tools draw their own preview.
            Tool::LiveSelection | Tool::LoopCut | Tool::LineCut => {}
            // A click with Bevel/Bridge/Weld still selects; show what it would pick.
            Tool::Bevel | Tool::Bridge | Tool::Weld => {
                let view = self.screen_view(rect);
                let radius = metrics::COMPONENT_PICK_RADIUS;
                self.hover
                    .extend(pick_component(&self.evaluated, node, component, &view, cursor, radius));
            }
            // Mesh tools: a click still selects, a drag runs the tool.
            Tool::RectangleSelection | Tool::Move | Tool::Scale | Tool::Rotate | Tool::Extrude | Tool::Inset => {
                let view = self.screen_view(rect);
                let radius = metrics::COMPONENT_PICK_RADIUS;
                self.hover
                    .extend(pick_component(&self.evaluated, node, component, &view, cursor, radius));
            }
        }
    }

    /// The camera as screen-space picking sees it, in points relative to `rect`.
    fn screen_view(&self, rect: Rect) -> ScreenView {
        ScreenView::new(&self.camera, GVec2::new(rect.width(), rect.height()))
    }

    /// Viewport header with C4D's menu set and the navigation drag-icons.
    /// Links the view to `editor.view_camera` (or back to the editor camera) and
    /// follows the linked object at the current frame. A view moved outside `show`
    /// (framing) is written to the object first.
    fn follow_view_camera(&mut self, doc: &mut Document, editor: &mut EditorState) {
        let linked = editor.view_camera.filter(|&id| doc.scene.contains(id));
        editor.view_camera = linked;
        if linked != self.linked {
            self.finish_camera_edit(editor);
            match (self.linked, linked) {
                (None, Some(_)) => self.editor_camera = Some(self.camera),
                (Some(_), None) => {
                    if let Some(camera) = self.editor_camera.take() {
                        self.camera = camera;
                    }
                }
                (Some(_), Some(_)) | (None, None) => {}
            }
            self.linked = linked;
            self.synced = None;
        }
        let Some(id) = linked else { return };
        if self.synced.is_some_and(|s| s != self.camera) {
            self.write_view_camera(doc, editor, id);
        }
        let frame = f64::from(doc.time.frame());
        self.evaluated.set_frame(frame);
        self.evaluated.update(&doc.scene);
        // Where the object is drawn (animation, Target tags); a hidden one from the scene.
        let world = self
            .evaluated
            .nodes()
            .iter()
            .find(|n| n.id == id)
            .map_or_else(|| doc.scene.world_matrix_at(id, frame), |n| n.world);
        let fov = match doc.scene.get(id).map(|n| &n.data.kind) {
            Some(ObjectKind::Camera { fov_y_deg }) => fov_y_deg.to_radians(),
            _ => self.camera.fov_y,
        };
        self.camera = OrbitCamera::from_world(&world, fov, self.camera.distance);
        self.synced = Some(self.camera);
    }

    /// Moves the linked object to where the view now is (a temporary value or a
    /// key, as for the gizmo); a Protection tag on position or rotation holds it.
    fn write_view_camera(&mut self, doc: &mut Document, editor: &mut EditorState, id: NodeId) {
        self.synced = Some(self.camera);
        let Some(node) = doc.scene.get(id) else { return };
        let protection = node.data.tags.protection();
        if protection.position || protection.rotation {
            return; // the next frame follows the object again
        }
        if self.camera_edit.as_ref().is_none_or(|(edited, _)| *edited != id) {
            self.finish_camera_edit(editor);
            self.camera_edit = Some((id, node.data.clone()));
        }
        let keying = crate::keys::Keying::of(doc, editor);
        let parent = doc.scene.parent_world_matrix_at(id, keying.frame);
        let local = Transform::from_matrix(&(parent.inverse() * self.camera.eye_matrix()));
        let mut shown = node.data.transform_at(keying.frame);
        shown.position = local.position;
        // Continuous angles (no jump from 180° to −180°), as with the gizmo.
        for i in 0..3 {
            let d = local.rotation[i] - shown.rotation[i];
            shown.rotation[i] += (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        }
        let mut data = node.data.clone();
        data.set_transform_at(&shown, keying.frame, keying.autokey);
        if let Some(node) = doc.scene.get_mut(id)
            && node.data != data
        {
            node.data = data;
        }
    }

    /// Records the navigation that moved the linked object as one undo step.
    fn finish_camera_edit(&mut self, editor: &mut EditorState) {
        if let Some((id, before)) = self.camera_edit.take() {
            editor.commands.push(EditorCommand::CommitLive {
                inverse: grid4d_scene::SceneEdit::SetData { id, data: before },
                label: "Move Camera",
            });
        }
    }

    fn header(&mut self, ui: &mut Ui, doc: &Document, editor: &mut EditorState, index: usize) {
        egui::Frame::new()
            .fill(p::BG_HEADER)
            .inner_margin(Margin::symmetric(6, 0))
            .show(ui, |ui| {
                ui.set_height(metrics::HEADER_HEIGHT);
                ui.horizontal_centered(|ui| {
                    egui::MenuBar::new().ui(ui, |ui| {
                        ui.menu_button("View", |ui| {
                            for (label, cmd) in [
                                ("Frame Scene (H)", EditorCommand::FrameScene),
                                ("Frame Selected Objects (O)", EditorCommand::FrameSelected),
                                ("Reset View", EditorCommand::ResetView),
                            ] {
                                if ui.button(label).clicked() {
                                    editor.commands.push(cmd);
                                    ui.close();
                                }
                            }
                        });
                        ui.menu_button("Cameras", |ui| {
                            // View 1 may look through a camera; every view picks its projection.
                            if index == 0 {
                                cameras_menu(ui, doc, editor);
                                ui.separator();
                            }
                            for kind in ViewKind::ALL {
                                let current = self.kind == kind && (index != 0 || editor.view_camera.is_none());
                                if ui.add(egui::Button::new(kind.label()).selected(current)).clicked() {
                                    if index == 0 {
                                        editor.view_camera = None;
                                    }
                                    self.kind = kind;
                                    self.camera = kind.aim(self.camera);
                                    ui.close();
                                }
                            }
                        });
                        ui.menu_button("Display", |ui| {
                            ui.add(egui::Button::new("Gouraud Shading").selected(true));
                            for planned in ["Gouraud Shading (Lines)", "Quick Shading", "Wireframe", "Box"] {
                                ui.add_enabled(false, egui::Button::new(planned));
                            }
                        });
                        ui.menu_button("Options", |ui| {
                            ui.checkbox(&mut self.show_grid, "Grid");
                            ui.checkbox(&mut self.show_axis_triad, "Axis Triad");
                            ui.checkbox(&mut self.show_hud, "HUD");
                        });
                        ui.menu_button("Filter", |ui| {
                            ui.add_enabled(false, egui::Button::new("Object filters — planned"));
                        });
                        ui.menu_button("Panel", |ui| {
                            for arrangement in Arrangement::ALL {
                                let current = self.arrangement == arrangement && self.solo.is_none();
                                if ui
                                    .add(egui::Button::new(arrangement.label()).selected(current))
                                    .clicked()
                                {
                                    self.arrangement = arrangement;
                                    self.solo = None;
                                    ui.close();
                                }
                            }
                            ui.separator();
                            for (i, label, key) in [
                                (0, "View 1", "F1"),
                                (1, "View 2", "F2"),
                                (2, "View 3", "F3"),
                                (3, "View 4", "F4"),
                            ] {
                                let alone = self.solo == Some(i);
                                if ui
                                    .add(egui::Button::new(label).shortcut_text(key).selected(alone))
                                    .clicked()
                                {
                                    editor.commands.push(EditorCommand::ShowView(Some(i)));
                                    ui.close();
                                }
                            }
                            if ui.add(egui::Button::new("All Views").shortcut_text("F5")).clicked() {
                                editor.commands.push(EditorCommand::ShowView(None));
                                ui.close();
                            }
                            ui.label(
                                egui::RichText::new("Middle-click a view to show it alone or in place")
                                    .color(p::TEXT_DIM)
                                    .size(11.0),
                            );
                        });
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 1.0;
                        let style = IconStyle {
                            size: 22.0,
                            ..IconStyle::default()
                        };
                        // A narrow view (four on a small window) keeps its menus
                        // and drops the icons rather than drawing them over the menus.
                        let icons = if self.camera.is_parallel() { 2.0 } else { 3.0 };
                        if ui.available_width() < icons * (style.size + 1.0) {
                            return;
                        }
                        // Orthographic views do not orbit.
                        if !self.camera.is_parallel() {
                            let orbit = icon_button(ui, Icon::Rotate, style, "Drag to orbit").interact(Sense::drag());
                            if orbit.dragged() {
                                let d = orbit.drag_delta();
                                self.camera.orbit(-d.x * ORBIT_SPEED, d.y * ORBIT_SPEED);
                            }
                        }
                        let zoom = icon_button(ui, Icon::NavZoom, style, "Drag to zoom").interact(Sense::drag());
                        if zoom.dragged() {
                            let d = zoom.drag_delta();
                            self.camera.dolly((-(d.x - d.y) * DOLLY_DRAG_SPEED).exp());
                        }
                        let pan = icon_button(ui, Icon::Move, style, "Drag to pan").interact(Sense::drag());
                        if pan.dragged() {
                            let d = pan.drag_delta();
                            self.camera.pan(GVec2::new(d.x, d.y), self.view_rect.height());
                        }
                    });
                });
            });
    }

    /// Cinema 4D navigation. Returns `true` while the camera is being moved.
    /// With a `brush` (Live Selection), a plain middle drag sizes it instead of panning.
    fn navigate(&mut self, ui: &Ui, response: &Response, rect: Rect, brush: Option<&mut f32>) -> bool {
        let (alt, scroll) = ui.input(|i| (i.modifiers.alt, i.smooth_scroll_delta.y));
        let d = response.drag_delta();
        let mut active = false;

        if response.dragged_by(PointerButton::Middle) {
            match brush {
                Some(radius) if !alt => *radius = (*radius + d.x).clamp(1.0, 1000.0),
                _ => self.camera.pan(GVec2::new(d.x, d.y), rect.height()),
            }
            active = true;
        } else if alt && response.dragged_by(PointerButton::Primary) {
            // Orthographic views look along an axis: Alt + left drag pans there.
            if self.camera.is_parallel() {
                self.camera.pan(GVec2::new(d.x, d.y), rect.height());
            } else {
                self.camera.orbit(-d.x * ORBIT_SPEED, d.y * ORBIT_SPEED);
            }
            active = true;
        } else if alt && response.dragged_by(PointerButton::Secondary) {
            self.camera.dolly((-(d.x - d.y) * DOLLY_DRAG_SPEED).exp());
            active = true;
        }
        if response.hovered() && scroll != 0.0 {
            self.camera.dolly((-scroll * DOLLY_WHEEL_SPEED).exp());
            active = true;
        }
        active
    }

    fn pick_on_click(&self, ui: &Ui, response: &Response, rect: Rect, editor: &mut EditorState) {
        if !response.clicked_by(PointerButton::Primary) {
            return;
        }
        let modifiers = ui.input(|i| i.modifiers);
        if modifiers.alt {
            return;
        }
        let Some(pointer) = response.interact_pointer_pos() else {
            return;
        };
        // A component mode on a polygon object picks its components, never objects.
        if let Some((node, component)) = self.component_target(editor) {
            let hit = pick_component(
                &self.evaluated,
                node,
                component,
                &self.screen_view(rect),
                local(pointer, rect),
                metrics::COMPONENT_PICK_RADIUS,
            );
            editor.commands.push(EditorCommand::SelectComponents {
                node,
                component,
                hits: hit.map(ComponentHit::index).into_iter().collect(),
                mode: SelectMode::for_components(modifiers),
            });
            return;
        }
        let ray = self.camera.ray_from_ndc(to_ndc(pointer, rect), rect.aspect_ratio());
        let mode = SelectMode::from_modifiers(modifiers);
        match pick(&self.evaluated, ray) {
            Some(id) => editor.commands.push(EditorCommand::Select { id, mode }),
            None if mode == SelectMode::Replace => editor.commands.push(EditorCommand::DeselectAll),
            None => {}
        }
    }

    /// A material dragged from the Material Manager and released over an object
    /// is assigned to it.
    fn drop_material(&self, ui: &Ui, rect: Rect, editor: &mut EditorState) {
        let Some(drag) = editor.materials.drag else { return };
        let Some(pointer) = ui.ctx().pointer_latest_pos().filter(|p| rect.contains(*p)) else {
            return;
        };
        editor.status_hint = Some("Release over an object to give it this material");
        if !ui.input(|i| i.pointer.any_released()) {
            return;
        }
        let ray = self.camera.ray_from_ndc(to_ndc(pointer, rect), rect.aspect_ratio());
        if let Some(id) = pick(&self.evaluated, ray) {
            editor.commands.push(EditorCommand::AssignMaterial {
                material: drag.material,
                nodes: vec![id],
            });
        }
    }

    fn context_menu(&self, response: &Response, editor: &mut EditorState) {
        response.context_menu(|ui| {
            for (label, cmd) in [
                ("Frame Scene", EditorCommand::FrameScene),
                ("Frame Selected Objects", EditorCommand::FrameSelected),
            ] {
                if ui.button(label).clicked() {
                    editor.commands.push(cmd);
                    ui.close();
                }
            }
            ui.separator();
            ui.menu_button("Create", |ui| {
                for tag in ObjectKindTag::ALL {
                    if ui.button(tag.label()).clicked() {
                        editor.commands.push(EditorCommand::CreateObject(tag));
                        ui.close();
                    }
                }
            });
            if !editor.selection.is_empty() {
                ui.separator();
                if ui.button("Delete").clicked() {
                    editor.commands.push(EditorCommand::DeleteSelected);
                    ui.close();
                }
            }
        });
    }
}

/// Size of `rect` in physical pixels, at least 1 × 1.
/// The Cameras menu: the editor camera, the scene's cameras, the active object.
fn cameras_menu(ui: &mut Ui, doc: &Document, editor: &mut EditorState) {
    let linked = editor.view_camera;
    if ui
        .add(egui::Button::new("Default Camera").selected(linked.is_none()))
        .on_hover_text("The view's own camera")
        .clicked()
    {
        editor.view_camera = None;
        ui.close();
    }
    ui.menu_button("Use Camera", |ui| {
        let mut any = false;
        doc.scene.walk(&mut Vec::new(), |id, node, _, _| {
            if matches!(node.data.kind, ObjectKind::Camera { .. }) {
                any = true;
                if ui
                    .add(egui::Button::new(&node.data.name).selected(linked == Some(id)))
                    .clicked()
                {
                    editor.view_camera = Some(id);
                    ui.close();
                }
            }
            true
        });
        if !any {
            ui.add_enabled(false, egui::Button::new("No cameras in the scene"));
        }
    });
    let active = editor.selection.active();
    if ui
        .add_enabled(active.is_some(), egui::Button::new("Selected Object as Camera"))
        .on_hover_text("Look along the active object's −Z axis (e.g. to aim a light)")
        .clicked()
    {
        editor.view_camera = active;
        ui.close();
    }
}

fn pixel_size(rect: Rect, pixels_per_point: f32) -> [u32; 2] {
    let px = |points: f32| {
        let pixels = (points * pixels_per_point).round().clamp(1.0, 16_384.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "rounded and clamped to 1..=16384"
        )]
        let pixels = pixels as u32;
        pixels
    };
    [px(rect.width()), px(rect.height())]
}

/// Window position → position in `rect` (points from its top-left corner).
fn local(pos: Pos2, rect: Rect) -> GVec2 {
    GVec2::new(pos.x - rect.left(), pos.y - rect.top())
}

/// The component overlay in the theme's colours, sizes in physical pixels.
fn component_style(pixels_per_point: f32) -> ComponentStyle {
    let linear = |c: egui::Color32| egui::Rgba::from(c).to_array();
    ComponentStyle {
        wire: linear(p::ACCENT),
        selected: linear(p::HIGHLIGHT),
        hover: linear(p::TEXT_STRONG),
        point_size: metrics::COMPONENT_POINT * pixels_per_point,
        edge_width: metrics::COMPONENT_EDGE * pixels_per_point,
        selected_edge_width: metrics::COMPONENT_EDGE_SELECTED * pixels_per_point,
        ..ComponentStyle::default()
    }
}

/// Status-bar help in a component mode (`active`: a polygon object to work on).
const fn component_hint(component: Component, tool: Tool, active: bool) -> &'static str {
    if !active {
        return "Component modes work on a polygon object: select one, or press C to make a primitive editable";
    }
    match tool {
        Tool::LiveSelection => {
            return "Live Selection: paint to select · Shift adds · Ctrl removes · middle-drag sizes the brush · Esc cancels";
        }
        Tool::RectangleSelection => {
            return "Rectangle Selection: drag a frame · Shift adds · Ctrl removes · Esc cancels";
        }
        Tool::LoopSelection => {
            return "Loop Selection: click to select the highlighted loop · Shift adds · Ctrl removes";
        }
        Tool::RingSelection => {
            return "Ring Selection: click to select the highlighted ring · Shift adds · Ctrl removes";
        }
        Tool::Extrude => {
            return match component {
                Component::Polygon => {
                    "Extrude: drag left/right to extrude the selected polygons (all when none is) · click selects · Esc cancels"
                }
                Component::Edge => {
                    "Extrude: drag left/right to extrude the selected border edges (all when none is) · click selects · Esc cancels"
                }
                Component::Point => "Extrude does not work on points yet: use Edges or Polygons mode",
            };
        }
        Tool::Inset => {
            return match component {
                Component::Polygon => {
                    "Inset: drag left/right to inset the selected polygons (all when none is) · click selects · Esc cancels"
                }
                Component::Point | Component::Edge => "Inset works in Polygons mode",
            };
        }
        Tool::LoopCut => {
            return "Loop/Path Cut: hover an edge to see the loop · click cuts it there · Number of Cuts spaces them evenly";
        }
        Tool::LineCut => return "Line Cut: drag a line across the object · release cuts · Esc cancels",
        Tool::Bevel => {
            return match component {
                Component::Edge | Component::Point => {
                    "Bevel: drag left/right to bevel the selection · Apply in the Attribute Manager · Esc cancels"
                }
                Component::Polygon => "Bevel works on edges or points: switch to Edges or Points mode",
            };
        }
        Tool::Bridge => {
            return match component {
                Component::Polygon => "Bridge: drag from one polygon to another · Apply bridges the two selected",
                Component::Edge => "Bridge: drag from one border edge to another · Apply bridges the two selected",
                Component::Point => "Bridge works in Polygons or Edges mode",
            };
        }
        Tool::Weld => return "Weld: click a selected point to weld onto it, or anywhere else to weld at the centre",
        Tool::Move | Tool::Scale | Tool::Rotate => {}
    }
    match component {
        Component::Point => {
            "Points: drag the gizmo to transform the selection · click selects · Shift adds · Ctrl removes"
        }
        Component::Edge => "Edges: drag the gizmo to transform the selection · click selects · double-click: loop",
        Component::Polygon => {
            "Polygons: drag the gizmo to transform the selection · click selects · double-click: connected part"
        }
    }
}

/// Window position → normalised device coordinates (`[-1, 1]`, Y up).
fn to_ndc(pos: Pos2, rect: Rect) -> GVec2 {
    GVec2::new(
        (pos.x - rect.left()) / rect.width() * 2.0 - 1.0,
        1.0 - (pos.y - rect.top()) / rect.height() * 2.0,
    )
}

/// World position → window position. `None` when behind the camera.
pub(super) fn project(view_proj: &Mat4, rect: Rect, point: Vec3) -> Option<Pos2> {
    let clip = *view_proj * point.extend(1.0);
    if clip.w <= 1e-4 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some(pos2(
        rect.left() + (ndc.x * 0.5 + 0.5) * rect.width(),
        rect.top() + (0.5 - ndc.y * 0.5) * rect.height(),
    ))
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, RawInput, Vec2, vec2};

    use super::*;
    use crate::{EditMode, EditorCommand, SelectMode, Tool, commands};

    /// Runs the viewport headless (no GPU) in a real egui context.
    struct Harness {
        ctx: egui::Context,
        time: f64,
        doc: Document,
        editor: EditorState,
        viewport: ViewportPanel,
    }

    impl Harness {
        fn new() -> Self {
            let doc = Document::with_demo_scene();
            let mut editor = EditorState {
                tool: Tool::Move,
                ..EditorState::default()
            };
            let cube = doc.scene.roots()[0];
            editor.selection.set_single(cube);
            Self {
                ctx: egui::Context::default(),
                time: 0.0,
                doc,
                editor,
                viewport: ViewportPanel::new(None),
            }
        }

        /// One frame: input events, layout, then the queued commands (as the app does).
        fn frame(&mut self, events: Vec<Event>) {
            self.time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
                time: Some(self.time),
                events,
                ..RawInput::default()
            };
            let (doc, editor, viewport) = (&mut self.doc, &mut self.editor, &mut self.viewport);
            self.ctx
                .run_ui(input, |ui| {
                    egui::CentralPanel::no_frame().show(ui, |ui| viewport.show(ui, doc, editor));
                })
                // No renderer here: font textures are never uploaded.
                .drop_without_applying_deltas();
            commands::apply_pending(doc, editor, viewport);
        }

        fn button(pos: Pos2, pressed: bool) -> Event {
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }
        }

        /// Makes the object named `name` active and editable, then enters `mode` with `tool`.
        fn edit(&mut self, name: &str, mode: EditMode, tool: Tool) -> NodeId {
            let id = self.doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0;
            self.editor.selection.set_single(id);
            self.editor.commands.push(EditorCommand::MakeEditable);
            self.frame(vec![]);
            self.editor.mode = mode;
            self.editor.tool = tool;
            self.frame(vec![]);
            id
        }

        fn mesh(&self, id: NodeId) -> grid4d_scene::MeshHandle {
            let ObjectKind::Polygon { mesh, .. } = &self.doc.scene.get(id).unwrap().data.kind else {
                panic!("not a polygon object");
            };
            mesh.clone()
        }

        fn components(&self, id: NodeId) -> grid4d_scene::Components {
            let ObjectKind::Polygon { components, .. } = &self.doc.scene.get(id).unwrap().data.kind else {
                panic!("not a polygon object");
            };
            components.clone()
        }

        fn select(&mut self, id: NodeId, component: Component, hits: Vec<u32>) {
            self.editor.commands.push(EditorCommand::SelectComponents {
                node: id,
                component,
                hits,
                mode: SelectMode::Replace,
            });
            self.frame(vec![]);
        }

        /// Drags gizmo handle `axis` by `steps` × 15 points along it.
        fn drag_handle(&mut self, axis: usize, steps: u8) {
            let geo = GizmoGeometry::new(
                &self.doc.scene,
                &self.editor,
                &self.viewport.camera,
                self.viewport.view_rect,
                f64::from(self.doc.time.frame()),
            )
            .expect("a gizmo");
            let (start, dir) = geo.handle_for_test(axis).unwrap();
            self.frame(vec![Event::PointerMoved(start)]);
            self.frame(vec![Self::button(start, true)]);
            let mut pointer = start;
            for _ in 0..steps {
                pointer += dir * 15.0;
                self.frame(vec![Event::PointerMoved(pointer)]);
            }
            self.frame(vec![Self::button(pointer, false)]);
            self.frame(vec![]);
        }

        /// Press at `from`, drag in steps to `to`, release.
        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.frame(vec![Event::PointerMoved(from)]);
            self.frame(vec![Self::button(from, true)]);
            for step in 1..=4u8 {
                self.frame(vec![Event::PointerMoved(from + (to - from) * f32::from(step) / 4.0)]);
            }
            self.frame(vec![Self::button(to, false)]);
            self.frame(vec![]);
        }

        fn selected(&self, id: NodeId, component: Component) -> Vec<usize> {
            let ObjectKind::Polygon { components, .. } = &self.doc.scene.get(id).unwrap().data.kind else {
                panic!("not a polygon object");
            };
            components.selected().get(component).iter().collect()
        }

        fn position(&self, id: NodeId) -> Vec3 {
            self.doc.scene.get(id).unwrap().data.transform.position
        }
    }

    #[test]
    fn a_protection_tag_hides_the_gizmo_of_its_channels() {
        let mut h = Harness::new();
        h.frame(vec![]);
        let cube = h.editor.selection.active().unwrap();
        h.doc.scene.get_mut(cube).unwrap().data.tags = vec![grid4d_scene::Tag::Protection {
            position: true,
            scale: false,
            rotation: true,
        }];
        let gizmo = |h: &Harness, tool| {
            let mut editor = EditorState {
                tool,
                ..EditorState::default()
            };
            editor.selection.set_single(cube);
            GizmoGeometry::new(&h.doc.scene, &editor, &h.viewport.camera, h.viewport.view_rect, 0.0).is_some()
        };
        assert!(!gizmo(&h, Tool::Move), "position protected");
        assert!(gizmo(&h, Tool::Scale), "scale free");
        assert!(!gizmo(&h, Tool::Rotate), "rotation protected");
    }

    #[test]
    fn a_gizmo_drag_is_one_undo_step() {
        let mut h = Harness::new();
        h.frame(vec![]); // lay out once so the viewport knows its 3D rect
        let cube = h.editor.selection.active().unwrap();
        let geo = GizmoGeometry::new(&h.doc.scene, &h.editor, &h.viewport.camera, h.viewport.view_rect, 0.0).unwrap();
        let (start, dir): (Pos2, Vec2) = geo.handle_for_test(0).unwrap();
        let before = h.position(cube);

        h.frame(vec![Event::PointerMoved(start)]);
        h.frame(vec![Harness::button(start, true)]);
        let mut pointer = start;
        for _ in 0..4 {
            pointer += dir * 15.0;
            h.frame(vec![Event::PointerMoved(pointer)]);
        }
        let dragged = h.position(cube);
        assert!(
            dragged.x > before.x + 1.0,
            "the X handle moves along X: {before:?} -> {dragged:?}"
        );
        assert!((dragged.y - before.y).abs() < 1e-3 && (dragged.z - before.z).abs() < 1e-3);
        assert!(!h.editor.history.can_undo(), "nothing is recorded while dragging");

        h.frame(vec![Harness::button(pointer, false)]);
        h.frame(vec![]);
        assert_eq!(h.editor.history.undo_len(), 1, "the whole drag is one step");
        assert_eq!(h.editor.history.undo_label(), Some("Move"));

        h.editor.commands.push(EditorCommand::Undo);
        h.frame(vec![]);
        assert_eq!(h.position(cube), before);
        h.editor.commands.push(EditorCommand::Redo);
        h.frame(vec![]);
        assert_eq!(h.position(cube), dragged);
    }

    #[test]
    fn a_rectangle_frame_selects_the_visible_points_inside_it() {
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Points, Tool::RectangleSelection);
        let rect = h.viewport.view_rect;
        h.drag(rect.min + vec2(4.0, 4.0), rect.max - vec2(4.0, 4.0));
        assert_eq!(
            h.selected(cube, Component::Point).len(),
            7,
            "the corner behind the cube stays out"
        );
        assert_eq!(h.editor.history.undo_label(), Some("Select Points"));
        assert!(h.viewport.gesture_is_idle());
    }

    #[test]
    fn a_live_selection_stroke_is_one_undo_step_and_esc_cancels_one() {
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Points, Tool::LiveSelection);
        h.editor.selection_settings.radius = 2000.0;
        let centre = h.viewport.view_rect.center();
        let steps = h.editor.history.undo_len();
        h.drag(centre, centre + vec2(30.0, 10.0));
        assert_eq!(h.selected(cube, Component::Point).len(), 7);
        assert_eq!(h.editor.history.undo_len(), steps + 1, "the whole stroke is one step");
        assert_eq!(h.editor.history.undo_label(), Some("Live Selection"));

        // A Ctrl stroke would remove everything; Esc puts it back and records nothing.
        let ctrl = Modifiers::COMMAND;
        h.frame(vec![Event::ModifiersChanged(ctrl), Event::PointerMoved(centre)]);
        h.frame(vec![Event::PointerButton {
            pos: centre,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: ctrl,
        }]);
        h.frame(vec![Event::PointerMoved(centre + vec2(20.0, 0.0))]);
        assert!(h.selected(cube, Component::Point).is_empty(), "painted away live");
        h.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: ctrl,
        }]);
        h.frame(vec![
            Harness::button(centre, false),
            Event::ModifiersChanged(Modifiers::NONE),
        ]);
        h.frame(vec![]);
        assert_eq!(
            h.selected(cube, Component::Point).len(),
            7,
            "Esc restored the selection"
        );
        assert_eq!(
            h.editor.history.undo_len(),
            steps + 1,
            "a cancelled stroke records nothing"
        );
    }

    #[test]
    fn loop_selection_takes_the_whole_equator() {
        let mut h = Harness::new();
        let sphere = h.edit("Sphere", EditMode::Edges, Tool::LoopSelection);
        let node = h.doc.scene.get(sphere).unwrap();
        let ObjectKind::Polygon { mesh, .. } = &node.data.kind else {
            panic!("the sphere was made editable");
        };
        let mesh = mesh.clone();
        let world = h.doc.scene.world_matrix(sphere);
        let eye = h.viewport.camera.eye();
        let points = mesh.mesh().points();
        // The equator edge nearest to the camera, at its middle.
        let middle = mesh
            .topology()
            .edges()
            .iter()
            .filter(|&&[a, b]| points[a as usize].y.abs() < 1e-3 && points[b as usize].y.abs() < 1e-3)
            .map(|&[a, b]| world.transform_point3((points[a as usize] + points[b as usize]) * 0.5))
            .min_by(|a, b| a.distance(eye).total_cmp(&b.distance(eye)))
            .unwrap();
        let rect = h.viewport.view_rect;
        let view_proj = h.viewport.camera.view_projection(rect.aspect_ratio());
        let at = project(&view_proj, rect, middle).unwrap();
        h.frame(vec![Event::PointerMoved(at)]);
        assert_eq!(h.viewport.hover.len(), 24, "the loop is previewed under the cursor");
        h.frame(vec![Harness::button(at, true)]);
        h.frame(vec![Harness::button(at, false)]);
        h.frame(vec![]);
        let corners = h.selected(sphere, Component::Edge);
        let topology = mesh.topology();
        let edges: std::collections::BTreeSet<u32> = corners.iter().filter_map(|&c| topology.corner_edge(c)).collect();
        assert_eq!(edges.len(), 24, "24 segments around");
        assert_eq!(h.editor.history.undo_label(), Some("Select Edges"));
    }

    #[test]
    fn a_rectangle_in_model_mode_selects_objects() {
        let mut h = Harness::new();
        h.editor.tool = Tool::RectangleSelection;
        h.frame(vec![]);
        let rect = h.viewport.view_rect;
        h.drag(rect.min + vec2(2.0, 2.0), rect.max - vec2(2.0, 2.0));
        let names: Vec<&str> = h
            .editor
            .selection
            .as_slice()
            .iter()
            .map(|&id| h.doc.scene.get(id).unwrap().data.name.as_str())
            .collect();
        assert!(names.contains(&"Cube") && names.contains(&"Sphere"), "{names:?}");
        assert!(!names.contains(&"Group"), "Nulls are not picked in the viewport");
    }

    #[test]
    fn a_click_in_points_mode_selects_the_visible_point_under_the_cursor() {
        let mut h = Harness::new();
        h.editor.commands.push(EditorCommand::MakeEditable);
        h.frame(vec![]);
        h.editor.mode = EditMode::Points;
        h.frame(vec![]);
        let cube = h.editor.selection.active().unwrap();
        let node = h.doc.scene.get(cube).unwrap();
        let ObjectKind::Polygon { mesh, .. } = &node.data.kind else {
            panic!("the cube was made editable");
        };
        let world = h.doc.scene.world_matrix(cube);
        let eye = h.viewport.camera.eye();
        let (nearest, position) = mesh
            .mesh()
            .points()
            .iter()
            .map(|&p| world.transform_point3(p))
            .enumerate()
            .min_by(|a, b| a.1.distance(eye).total_cmp(&b.1.distance(eye)))
            .unwrap();
        let rect = h.viewport.view_rect;
        let view_proj = h.viewport.camera.view_projection(rect.aspect_ratio());
        let at = project(&view_proj, rect, position).unwrap() + vec2(2.0, 1.0);

        h.frame(vec![Event::PointerMoved(at)]);
        assert_eq!(h.viewport.hover, [ComponentHit::Point(u32::try_from(nearest).unwrap())]);
        h.frame(vec![Harness::button(at, true)]);
        h.frame(vec![Harness::button(at, false)]);
        h.frame(vec![]);
        let node = h.doc.scene.get(cube).unwrap();
        let ObjectKind::Polygon { components, .. } = &node.data.kind else {
            panic!();
        };
        assert_eq!(components.selected().points.iter().collect::<Vec<_>>(), [nearest]);
        assert_eq!(h.editor.history.undo_label(), Some("Select Points"));
        assert_eq!(h.editor.selection.as_slice(), &[cube], "objects stay selected");
    }

    #[test]
    fn the_gizmo_moves_the_selected_points_as_one_undo_step() {
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Points, Tool::Move);
        h.select(cube, Component::Point, vec![0]);
        let (before, components) = (h.mesh(cube), h.components(cube));
        let steps = h.editor.history.undo_len();

        h.drag_handle(0, 4);
        let after = h.mesh(cube);
        assert_ne!(after.id(), before.id(), "the moved mesh is a new mesh");
        assert!(h.components(cube).same(&components), "…with the very same selection");
        let moved = after.mesh().points()[0] - before.mesh().points()[0];
        assert!(
            moved.x > 1.0 && moved.y.abs() < 1e-3 && moved.z.abs() < 1e-3,
            "along X: {moved:?}"
        );
        assert_eq!(
            after.mesh().points()[1..],
            before.mesh().points()[1..],
            "only the selected point"
        );
        assert_eq!(h.editor.history.undo_len(), steps + 1, "the whole drag is one step");
        assert_eq!(h.editor.history.undo_label(), Some("Move Points"));

        h.editor.commands.push(EditorCommand::Undo);
        h.frame(vec![]);
        assert_eq!(h.mesh(cube).id(), before.id(), "undo brings the original mesh back");
    }

    /// The gizmo sits where the animation puts the object, and a drag writes a
    /// temporary value, or a key with Autokeying.
    #[test]
    fn the_gizmo_follows_animation_and_keys_with_autokeying() {
        use grid4d_scene::{Axis, Param};
        let mut h = Harness::new();
        let cube = h.doc.scene.roots()[0];
        let px = Param::Position(Axis::X);
        // Keys at 0 and 10 (x = 0 and 200); frame 10 shows x = 200.
        let data = &mut h.doc.scene.get_mut(cube).unwrap().data;
        data.record([px], 0.0, 0.0);
        data.set_value_at(px, 200.0, 10.0, true);
        h.doc.time.set_frame(10);
        h.frame(vec![]);
        let geo = GizmoGeometry::new(&h.doc.scene, &h.editor, &h.viewport.camera, h.viewport.view_rect, 10.0).unwrap();
        assert!(
            (geo.origin.x - 200.0).abs() < 1e-3,
            "on the animated object: {}",
            geo.origin
        );

        h.drag_handle(0, 4);
        let data = &h.doc.scene.get(cube).unwrap().data;
        let moved = data.value_at(px, 10.0);
        assert!(moved > 201.0, "moved along X: {moved}");
        assert_eq!(data.key_state(px, 10.0), grid4d_scene::KeyState::Changed { key: true });
        assert_eq!(
            data.tracks[0].curve.key_at(10.0).unwrap().value,
            200.0,
            "the key is unchanged"
        );

        h.editor.commands.push(EditorCommand::Undo);
        h.editor.animation.autokey = true;
        h.frame(vec![]);
        h.drag_handle(0, 4);
        let data = &h.doc.scene.get(cube).unwrap().data;
        assert!(
            data.tracks[0].curve.key_at(10.0).unwrap().value > 201.0,
            "Autokeying wrote the key"
        );
        assert_eq!(data.unkeyed, None);
        assert_eq!(data.tracks.len(), 1, "unchanged parameters get no keys");
    }

    #[test]
    fn scaling_a_polygon_spreads_its_points_around_its_centre() {
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Polygons, Tool::Scale);
        let before = h.mesh(cube);
        let top = (0..before.mesh().face_count())
            .find(|&f| before.mesh().face_normal(f).abs_diff_eq(Vec3::Y, 1e-5))
            .unwrap();
        h.select(cube, Component::Polygon, vec![u32::try_from(top).unwrap()]);
        h.drag_handle(0, 3);
        let after = h.mesh(cube);
        let (old_centre, new_centre) = (before.mesh().face_center(top), after.mesh().face_center(top));
        assert!(new_centre.abs_diff_eq(old_centre, 1e-3), "the pivot stays put");
        let width = |m: &grid4d_scene::MeshHandle| {
            let xs = m.mesh().face(top).iter().map(|&p| m.mesh().points()[p as usize].x);
            xs.clone().fold(f32::NEG_INFINITY, f32::max) - xs.fold(f32::INFINITY, f32::min)
        };
        assert!(
            width(&after) > width(&before) * 1.2,
            "{} -> {}",
            width(&before),
            width(&after)
        );
        assert_eq!(h.editor.history.undo_label(), Some("Scale Polygons"));
    }

    #[test]
    fn tweak_moves_the_point_under_the_cursor_without_selecting_it() {
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Points, Tool::Move);
        h.editor.transform_settings.tweak = true;
        let before = h.mesh(cube);
        let world = h.doc.scene.world_matrix(cube);
        let eye = h.viewport.camera.eye();
        let (nearest, position) = before
            .mesh()
            .points()
            .iter()
            .map(|&p| world.transform_point3(p))
            .enumerate()
            .min_by(|a, b| a.1.distance(eye).total_cmp(&b.1.distance(eye)))
            .unwrap();
        let rect = h.viewport.view_rect;
        let view_proj = h.viewport.camera.view_projection(rect.aspect_ratio());
        let at = project(&view_proj, rect, position).unwrap();
        h.drag(at, at + vec2(40.0, 0.0));
        let after = h.mesh(cube);
        assert!(after.mesh().points()[nearest].distance(before.mesh().points()[nearest]) > 1.0);
        assert_eq!(
            after
                .mesh()
                .points()
                .iter()
                .filter(|p| !before.mesh().points().contains(p))
                .count(),
            1
        );
        assert!(
            h.selected(cube, Component::Point).is_empty(),
            "the selection is left alone"
        );
        assert_eq!(h.editor.history.undo_label(), Some("Move Points"));
    }

    /// Snap targets: only `kinds`.
    fn snap_only(kinds: &[grid4d_render::SnapKind]) -> grid4d_render::SnapTargets {
        let mut targets = grid4d_render::SnapTargets::default();
        for kind in grid4d_render::SnapKind::ALL {
            *targets.flag_mut(kind) = kinds.contains(&kind);
        }
        targets
    }

    /// Drags the active object's X handle in 3-point steps; the X position after each.
    fn drag_x_in_small_steps(h: &mut Harness, frames: usize) -> Vec<(f32, Option<grid4d_render::SnapKind>)> {
        h.frame(vec![]);
        let geo = GizmoGeometry::new(&h.doc.scene, &h.editor, &h.viewport.camera, h.viewport.view_rect, 0.0).unwrap();
        let (start, dir) = geo.handle_for_test(0).unwrap();
        let cube = h.editor.selection.active().unwrap();
        h.frame(vec![Event::PointerMoved(start)]);
        h.frame(vec![Harness::button(start, true)]);
        let mut pointer = start;
        let mut xs = Vec::new();
        for _ in 0..frames {
            pointer += dir * 3.0;
            h.frame(vec![Event::PointerMoved(pointer)]);
            xs.push((h.position(cube).x, h.viewport.snap_hit.map(|hit| hit.kind)));
        }
        h.frame(vec![Harness::button(pointer, false)]);
        h.frame(vec![]);
        xs
    }

    #[test]
    fn a_snapping_move_lines_the_axis_up_with_a_target_along_the_handle() {
        use grid4d_render::SnapKind;
        let target = |h: &mut Harness| {
            // A Null on the cube's X line (the cube's axis is at y = 100).
            let null = grid4d_scene::Node::new("Target", ObjectKind::Null).with_position(Vec3::new(400.0, 100.0, 0.0));
            h.doc.scene.insert(null);
        };
        let mut h = Harness::new();
        target(&mut h);
        h.editor.snap.enabled = true;
        h.editor.snap.targets = snap_only(&[SnapKind::Axis]);
        let xs = drag_x_in_small_steps(&mut h, 150);
        let stuck: Vec<_> = xs.iter().filter(|(x, _)| (x - 400.0).abs() < 1e-3).collect();
        assert!(stuck.len() >= 3, "held at the target for a few frames: {xs:?}");
        assert!(stuck.iter().all(|(_, kind)| *kind == Some(SnapKind::Axis)), "and shown");
        assert!(xs.last().unwrap().0 > 410.0, "then let go: {xs:?}");
        assert_eq!(h.editor.history.undo_len(), 1, "one undo step");
        assert_eq!(h.viewport.snap_hit, None, "nothing shown after the drag");

        let mut free = Harness::new();
        target(&mut free);
        let xs = drag_x_in_small_steps(&mut free, 150);
        // (The first frames are below egui's drag threshold.)
        let moving: Vec<f32> = xs.iter().map(|&(x, _)| x).skip_while(|&x| x == 0.0).collect();
        assert!(
            moving.windows(2).all(|w| w[1] > w[0]),
            "without snapping it never sticks: {xs:?}"
        );
        assert!(xs.iter().all(|(_, kind)| kind.is_none()));
    }

    #[test]
    fn tweak_snaps_a_point_onto_another_point_in_3d_or_on_screen_in_2d() {
        use grid4d_render::SnapKind;
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Points, Tool::Move);
        h.editor.transform_settings.tweak = true;
        h.editor.snap.enabled = true;
        h.editor.snap.targets = snap_only(&[SnapKind::Vertex]);
        let before = h.mesh(cube);
        let world = h.doc.scene.world_matrix(cube);
        let index_of = |p: Vec3| {
            before
                .mesh()
                .points()
                .iter()
                .position(|&q| world.transform_point3(q).abs_diff_eq(p, 1e-3))
                .unwrap()
        };
        // The cube spans y 0..200: the corner nearest the camera, and the one behind it.
        let (a, b) = (Vec3::new(100.0, 200.0, 100.0), Vec3::new(100.0, 200.0, -100.0));
        let (ia, ib) = (index_of(a), index_of(b));
        let rect = h.viewport.view_rect;
        let view_proj = h.viewport.camera.view_projection(rect.aspect_ratio());
        let (from, to) = (
            project(&view_proj, rect, a).unwrap(),
            project(&view_proj, rect, b).unwrap(),
        );

        let drag = |h: &mut Harness| {
            h.frame(vec![Event::PointerMoved(from)]);
            h.frame(vec![Harness::button(from, true)]);
            for step in 1..=4u8 {
                let at = from + (to + vec2(2.0, 1.0) - from) * f32::from(step) / 4.0;
                h.frame(vec![Event::PointerMoved(at)]);
            }
            let shown = h.viewport.snap_hit.map(|hit| hit.kind);
            h.frame(vec![Harness::button(to + vec2(2.0, 1.0), false)]);
            h.frame(vec![]);
            shown
        };
        assert_eq!(
            drag(&mut h),
            Some(SnapKind::Vertex),
            "the target is shown while dragging"
        );
        let after = h.mesh(cube);
        assert!(
            after.mesh().points()[ia].abs_diff_eq(before.mesh().points()[ib], 1e-3),
            "onto the other point: {}",
            after.mesh().points()[ia]
        );
        assert_eq!(h.editor.history.undo_label(), Some("Move Points"));

        h.editor.commands.push(EditorCommand::Undo);
        h.frame(vec![]);
        h.editor.snap.kind = crate::SnapType::TwoD;
        drag(&mut h);
        let moved = world.transform_point3(h.mesh(cube).mesh().points()[ia]);
        assert!(
            project(&view_proj, rect, moved).unwrap().distance(to) < 0.1,
            "lined up on screen"
        );
        assert!(moved.distance(b) > 10.0, "but at its own depth: {moved}");
    }

    #[test]
    fn an_extrude_drag_is_one_undo_step_and_esc_cancels_one() {
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Polygons, Tool::Extrude);
        let before = h.mesh(cube);
        let top = (0..before.mesh().face_count())
            .find(|&f| before.mesh().face_normal(f).abs_diff_eq(Vec3::Y, 1e-5))
            .unwrap();
        h.select(cube, Component::Polygon, vec![u32::try_from(top).unwrap()]);
        let steps = h.editor.history.undo_len();
        let centre = h.viewport.view_rect.center();

        h.drag(centre, centre + vec2(60.0, 0.0));
        let after = h.mesh(cube);
        assert_eq!(after.mesh().face_count(), 6 + 4, "the top face extruded");
        assert!(
            after.mesh().face_center(top).y > before.mesh().face_center(top).y + 1.0,
            "dragging right extrudes outwards"
        );
        assert_eq!(
            h.selected(cube, Component::Polygon),
            [top],
            "the moved face stays selected"
        );
        assert_eq!(h.editor.history.undo_len(), steps + 1);
        assert_eq!(h.editor.history.undo_label(), Some("Extrude"));
        let offset = h.editor.modeling_settings.extrude.offset;
        assert!(offset > 1.0, "the dragged offset is kept: {offset}");

        // Esc mid-drag restores the mesh and records nothing.
        let to = centre + vec2(40.0, 0.0);
        h.frame(vec![Event::PointerMoved(centre)]);
        h.frame(vec![Harness::button(centre, true)]);
        h.frame(vec![Event::PointerMoved(to)]);
        assert_ne!(h.mesh(cube).id(), after.id(), "live while dragging");
        h.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![Harness::button(to, false)]);
        h.frame(vec![]);
        assert_eq!(h.mesh(cube).id(), after.id(), "Esc restores");
        assert_eq!(h.editor.history.undo_len(), steps + 1);

        h.editor.commands.push(EditorCommand::Undo);
        h.frame(vec![]);
        assert_eq!(h.mesh(cube).id(), before.id(), "undo brings the cube back");
    }

    #[test]
    fn inset_needs_polygons_mode_and_a_click_still_selects() {
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Edges, Tool::Inset);
        let before = h.mesh(cube);
        let centre = h.viewport.view_rect.center();
        h.drag(centre, centre + vec2(60.0, 0.0));
        assert_eq!(h.mesh(cube).id(), before.id(), "Inset does nothing in Edges mode");

        h.editor.mode = EditMode::Polygons;
        h.frame(vec![]);
        h.drag(centre, centre + vec2(30.0, 0.0));
        assert_eq!(
            h.mesh(cube).mesh().face_count(),
            6 + 6 * 4,
            "nothing selected: every polygon"
        );
        assert_eq!(h.editor.history.undo_label(), Some("Inset"));
    }

    #[test]
    fn a_loop_cut_click_cuts_where_the_cursor_is() {
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Edges, Tool::LoopCut);
        let before = h.mesh(cube);
        // The middle of a vertical edge facing the camera, on screen.
        let world = h.doc.scene.world_matrix(cube);
        let eye = h.viewport.camera.eye();
        let topology = before.topology();
        let points = before.mesh().points();
        let vertical = (0..topology.edge_count())
            .filter(|&e| {
                let [a, b] = topology.edges()[e];
                let (pa, pb) = (points[a as usize], points[b as usize]);
                (pa.x - pb.x).abs() < 1e-3 && (pa.z - pb.z).abs() < 1e-3
            })
            .min_by(|&e, &f| {
                let middle = |e: usize| {
                    let [a, b] = topology.edges()[e];
                    world.transform_point3((points[a as usize] + points[b as usize]) * 0.5)
                };
                middle(e).distance(eye).total_cmp(&middle(f).distance(eye))
            })
            .unwrap();
        let [a, b] = topology.edges()[vertical];
        let middle = world.transform_point3((points[a as usize] + points[b as usize]) * 0.5);
        let rect = h.viewport.view_rect;
        let at = project(&h.viewport.camera.view_projection(rect.aspect_ratio()), rect, middle).unwrap();

        h.frame(vec![Event::PointerMoved(at)]);
        h.frame(vec![]);
        let preview = h.viewport.loop_preview.as_ref().expect("a preview of the loop");
        assert_eq!(preview.segments.len(), 4, "round the four sides");
        h.frame(vec![Harness::button(at, true)]);
        h.frame(vec![Harness::button(at, false)]);
        h.frame(vec![]);
        let after = h.mesh(cube);
        assert_eq!((after.mesh().point_count(), after.mesh().face_count()), (12, 10));
        assert_eq!(h.editor.history.undo_label(), Some("Loop Cut"));
        let offset = h.editor.modeling_settings.loop_cut.offset;
        assert!((offset - 0.5).abs() < 0.05, "clicked in the middle: {offset}");

        // A click with no hover before it (move, press and release in one frame).
        h.editor.commands.push(EditorCommand::Undo);
        h.frame(vec![Event::PointerGone]);
        h.frame(vec![]);
        h.frame(vec![
            Event::PointerMoved(at),
            Harness::button(at, true),
            Harness::button(at, false),
            Event::PointerGone,
        ]);
        h.frame(vec![]);
        assert_eq!(
            h.mesh(cube).mesh().face_count(),
            10,
            "cut in one frame, even if the pointer left right after"
        );
    }

    /// Phase 4's acceptance (`docs/IMPLEMENTATION_PLAN.md`): a box modelled with
    /// the tools — extrude, loop cut, bevel, Subdivision Surface — each step undone
    /// and redone, then saved and reloaded.
    #[test]
    fn phase_4_acceptance_model_undo_redo_save_and_reload() {
        let mut h = Harness::new();
        let state = |h: &Harness, cube: NodeId| {
            let node = h.doc.scene.get(cube).unwrap();
            let faces = match &node.data.kind {
                ObjectKind::Polygon { mesh, .. } => mesh.mesh().face_count(),
                _ => 0,
            };
            let parent = node
                .parent()
                .and_then(|p| h.doc.scene.get(p))
                .map(|n| n.data.kind.tag());
            (faces, parent)
        };
        // 1. The box, made editable.
        let cube = h.edit("Cube", EditMode::Polygons, Tool::Extrude);
        let mut states = vec![state(&h, cube)];
        assert_eq!(states[0].0, 6);

        // 2. Extrude the top polygon (Apply).
        let mesh = h.mesh(cube);
        let top = (0..mesh.mesh().face_count())
            .find(|&f| mesh.mesh().face_normal(f).abs_diff_eq(Vec3::Y, 1e-4))
            .unwrap();
        h.select(cube, Component::Polygon, vec![u32::try_from(top).unwrap()]);
        h.editor.modeling_settings.extrude.offset = 100.0;
        h.editor.commands.push(EditorCommand::ApplyMeshTool);
        h.frame(vec![]);
        states.push(state(&h, cube));
        assert_eq!(states[1].0, 10, "four new walls");

        // 3. A loop cut around the box: click the middle of the nearest vertical edge.
        h.editor.mode = EditMode::Edges;
        h.editor.tool = Tool::LoopCut;
        h.frame(vec![]);
        let mesh = h.mesh(cube);
        let world = h.doc.scene.world_matrix(cube);
        let eye = h.viewport.camera.eye();
        let topology = mesh.topology();
        let points = mesh.mesh().points();
        let middle = |e: usize| {
            let [a, b] = topology.edges()[e];
            world.transform_point3((points[a as usize] + points[b as usize]) * 0.5)
        };
        let vertical = (0..topology.edge_count())
            .filter(|&e| {
                let [a, b] = topology.edges()[e];
                let (pa, pb) = (points[a as usize], points[b as usize]);
                (pa.x - pb.x).abs() < 1e-3 && (pa.z - pb.z).abs() < 1e-3
            })
            .min_by(|&e, &f| middle(e).distance(eye).total_cmp(&middle(f).distance(eye)))
            .unwrap();
        let rect = h.viewport.view_rect;
        let at = project(
            &h.viewport.camera.view_projection(rect.aspect_ratio()),
            rect,
            middle(vertical),
        )
        .unwrap();
        h.frame(vec![Event::PointerMoved(at)]);
        h.frame(vec![]);
        h.frame(vec![Harness::button(at, true)]);
        h.frame(vec![Harness::button(at, false)]);
        h.frame(vec![Event::PointerGone]);
        states.push(state(&h, cube));
        assert_eq!(states[2].0, 14, "the cut adds a ring of four polygons");

        // 4. Bevel one edge (Apply).
        h.editor.tool = Tool::Bevel;
        h.select(cube, Component::Edge, vec![0]);
        h.editor.commands.push(EditorCommand::ApplyMeshTool);
        h.frame(vec![]);
        states.push(state(&h, cube));
        assert!(states[3].0 > states[2].0, "bevelled: {states:?}");

        // 5. Smooth it: Alt-create a Subdivision Surface around it.
        h.editor.mode = EditMode::Model;
        h.editor.selection.set_single(cube);
        h.editor
            .commands
            .push(EditorCommand::WrapInGenerator(ObjectKindTag::SubdivisionSurface));
        h.frame(vec![]);
        states.push(state(&h, cube));
        assert_eq!(states[4].1, Some(ObjectKindTag::SubdivisionSurface));
        assert_eq!(
            h.editor.history.undo_len(),
            7,
            "one step per tool, plus the two selections"
        );

        // Every step undoes and redoes exactly (selection steps change no geometry).
        let final_mesh = h.mesh(cube);
        let step = |h: &mut Harness, command: EditorCommand, to: (usize, Option<ObjectKindTag>)| {
            for _ in 0..2 {
                h.editor.commands.push(command.clone());
                h.frame(vec![]);
                if state(h, cube) == to {
                    return;
                }
            }
            panic!("{command:?} did not reach {to:?}: {:?}", state(h, cube));
        };
        for expected in states.iter().rev().skip(1) {
            step(&mut h, EditorCommand::Undo, *expected);
        }
        step(&mut h, EditorCommand::Undo, (0, None));
        assert!(!h.editor.history.can_undo(), "back to the parametric cube");
        for expected in &states {
            step(&mut h, EditorCommand::Redo, *expected);
        }
        assert!(!h.editor.history.can_redo());
        assert_eq!(h.mesh(cube).mesh(), final_mesh.mesh());

        // Saved and reloaded: the same mesh under the same generator.
        let loaded = grid4d_io::from_bytes(&grid4d_io::to_bytes(&h.doc).unwrap()).unwrap();
        let (id, node) = loaded.scene.iter().find(|(_, n)| n.data.name == "Cube").unwrap();
        let ObjectKind::Polygon { mesh, .. } = &node.data.kind else {
            panic!("not a polygon object after loading")
        };
        assert_eq!(mesh.mesh(), final_mesh.mesh());
        let parent = loaded
            .scene
            .get(id)
            .and_then(|n| n.parent())
            .and_then(|p| loaded.scene.get(p));
        assert_eq!(
            parent.map(|n| n.data.kind.tag()),
            Some(ObjectKindTag::SubdivisionSurface)
        );
    }

    fn camera_id(h: &Harness) -> NodeId {
        h.doc.scene.iter().find(|(_, n)| n.data.name == "Camera").unwrap().0
    }

    /// Orbits with Alt + left drag across the view.
    fn orbit(h: &mut Harness) {
        let rect = h.viewport.view_rect;
        let (from, to) = (rect.center(), rect.center() + vec2(80.0, 30.0));
        let press = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::ALT,
        };
        h.frame(vec![Event::ModifiersChanged(Modifiers::ALT), Event::PointerMoved(from)]);
        h.frame(vec![press(from, true)]);
        for k in 1..=6 {
            h.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 6.0))]);
        }
        h.frame(vec![press(to, false)]);
        h.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        h.frame(vec![]);
    }

    #[test]
    fn the_view_looks_through_a_linked_camera_and_navigation_moves_it() {
        let mut h = Harness::new();
        let camera = camera_id(&h);
        let editor_view = h.viewport.camera;
        h.editor.view_camera = Some(camera);
        h.frame(vec![]);
        let world = h.doc.scene.world_matrix(camera);
        let view = h.viewport.camera;
        assert!(view.eye().abs_diff_eq(world.w_axis.truncate(), 1e-2), "at the camera");
        let forward = world.transform_vector3(Vec3::NEG_Z).normalize();
        assert!(view.forward().abs_diff_eq(forward, 1e-4), "along its −Z");
        assert!((view.fov_y - 40f32.to_radians()).abs() < 1e-5, "its field of view");

        let before = h.doc.scene.get(camera).unwrap().data.clone();
        let steps = h.editor.history.undo_len();
        orbit(&mut h);
        let after = h.doc.scene.get(camera).unwrap().data.transform;
        assert_ne!(after, before.transform, "orbiting moved the camera object");
        assert!(
            h.viewport
                .camera
                .eye()
                .abs_diff_eq(h.doc.scene.world_matrix(camera).w_axis.truncate(), 1e-2),
            "and the view stays on it"
        );
        assert_eq!(h.editor.history.undo_len(), steps + 1, "one step per gesture");
        assert_eq!(h.editor.history.undo_label(), Some("Move Camera"));

        h.editor.commands.push(EditorCommand::Undo);
        h.frame(vec![]);
        h.frame(vec![]);
        assert_eq!(h.doc.scene.get(camera).unwrap().data, before);
        assert!(
            h.viewport.camera.eye().abs_diff_eq(view.eye(), 1e-2),
            "the view follows the undo"
        );

        // Framing moves the camera too.
        h.editor.commands.push(EditorCommand::FrameScene);
        h.frame(vec![]);
        h.frame(vec![]);
        assert_ne!(h.doc.scene.get(camera).unwrap().data.transform, before.transform);
        assert_eq!(h.editor.history.undo_label(), Some("Move Camera"));

        // Back to the default camera: the editor view as it was.
        h.editor.view_camera = None;
        h.frame(vec![]);
        assert_eq!(h.viewport.camera, editor_view);
    }

    #[test]
    fn an_animated_camera_is_followed_and_autokeying_keys_it() {
        use grid4d_scene::{Axis, Param};
        let mut h = Harness::new();
        let camera = camera_id(&h);
        let px = Param::Position(Axis::X);
        let data = &mut h.doc.scene.get_mut(camera).unwrap().data;
        data.record([px], 0.0, 0.0);
        data.set_value_at(px, 500.0, 10.0, true);
        h.editor.view_camera = Some(camera);
        h.doc.time.set_frame(10);
        h.frame(vec![]);
        assert!(
            (h.viewport.camera.eye().x - 500.0).abs() < 1e-2,
            "at its frame-10 place"
        );

        h.editor.animation.autokey = true;
        orbit(&mut h);
        let data = &h.doc.scene.get(camera).unwrap().data;
        let key = data
            .tracks
            .iter()
            .find(|t| t.param == px)
            .unwrap()
            .curve
            .key_at(10.0)
            .unwrap();
        assert!(
            (key.value - 500.0).abs() > 1.0,
            "the key moved with the view: {}",
            key.value
        );
        assert!(
            data.tracks.iter().any(|t| t.param == px),
            "Autokeying writes keys for animated channels"
        );
    }

    #[test]
    fn a_protected_camera_holds_the_view() {
        let mut h = Harness::new();
        let camera = camera_id(&h);
        h.doc
            .scene
            .get_mut(camera)
            .unwrap()
            .data
            .tags
            .push(grid4d_scene::TagKind::Protection.default_tag());
        h.editor.view_camera = Some(camera);
        h.frame(vec![]);
        let (before, view) = (h.doc.scene.get(camera).unwrap().data.clone(), h.viewport.camera);
        let steps = h.editor.history.undo_len();
        orbit(&mut h);
        assert_eq!(h.doc.scene.get(camera).unwrap().data, before);
        assert_eq!(h.editor.history.undo_len(), steps, "nothing recorded");
        assert!(
            h.viewport.camera.eye().abs_diff_eq(view.eye(), 1e-3),
            "the view snaps back"
        );
    }

    #[test]
    fn any_object_can_be_the_view_and_deleting_it_unlinks() {
        let mut h = Harness::new();
        let light = h.doc.scene.iter().find(|(_, n)| n.data.name == "Light").unwrap().0;
        let fov = h.viewport.camera.fov_y;
        let editor_view = h.viewport.camera;
        h.editor.view_camera = Some(light);
        h.frame(vec![]);
        let at = h.doc.scene.world_matrix(light).w_axis.truncate();
        assert!(h.viewport.camera.eye().abs_diff_eq(at, 1e-2));
        assert_eq!(
            h.viewport.camera.fov_y, fov,
            "not a camera: the view's own field of view"
        );
        h.editor.selection.set_single(light);
        h.editor.commands.push(EditorCommand::DeleteSelected);
        h.frame(vec![]);
        h.frame(vec![]);
        assert_eq!(h.editor.view_camera, None);
        assert_eq!(h.viewport.camera, editor_view, "the editor view returns");
    }

    #[test]
    fn four_views_each_keep_their_camera_and_a_press_activates_one() {
        let mut h = Harness::new();
        let perspective = h.viewport.camera;
        h.editor.commands.push(EditorCommand::ShowView(None));
        h.frame(vec![]);
        h.frame(vec![]);
        assert_eq!(h.viewport.layout(), (Arrangement::Four, None));
        let first = h.viewport.view_rect;
        assert!(first.width() < 520.0 && first.height() < 360.0, "a quarter: {first:?}");

        // A click in the bottom-right quarter: View 4, Front, becomes active.
        let corner = pos2(first.right() + 300.0, first.bottom() + 200.0);
        h.frame(vec![Event::PointerMoved(corner)]);
        h.frame(vec![Harness::button(corner, true)]);
        h.frame(vec![Harness::button(corner, false)]);
        assert_eq!(h.viewport.view_kind(), ViewKind::Front);
        assert!(h.viewport.camera.is_parallel());
        assert!(h.viewport.camera.forward().abs_diff_eq(Vec3::NEG_Z, 1e-5));

        // In the Top view a click on the cube selects it.
        let top_right = pos2(first.right() + 300.0, first.top() + 100.0);
        h.frame(vec![Event::PointerMoved(top_right)]);
        h.frame(vec![Harness::button(top_right, true)]);
        h.frame(vec![Harness::button(top_right, false)]);
        assert_eq!(h.viewport.view_kind(), ViewKind::Top);
        h.editor.selection.clear();
        for _ in 0..30 {
            h.frame(vec![]); // past the double-click delay
        }
        let cube = h.doc.scene.roots()[0];
        let rect = h.viewport.view_rect;
        let view_proj = h.viewport.camera.view_projection(rect.aspect_ratio());
        let at = project(&view_proj, rect, h.doc.scene.world_matrix(cube).w_axis.truncate()).unwrap();
        h.frame(vec![Event::PointerMoved(at)]);
        h.frame(vec![Harness::button(at, true)]);
        h.frame(vec![Harness::button(at, false)]);
        assert_eq!(h.editor.selection.as_slice(), &[cube], "picked from above");

        // Back in View 1: its own camera, untouched.
        h.editor.commands.push(EditorCommand::ShowView(Some(0)));
        h.frame(vec![]);
        h.frame(vec![]);
        assert_eq!(h.viewport.layout(), (Arrangement::Four, Some(0)), "View 1 alone");
        assert_eq!(h.viewport.view_kind(), ViewKind::Perspective);
        assert_eq!(h.viewport.camera, perspective);

        // A middle click (no drag) puts it back among the others.
        let centre = h.viewport.view_rect.center();
        let middle = |pressed| Event::PointerButton {
            pos: centre,
            button: PointerButton::Middle,
            pressed,
            modifiers: Modifiers::NONE,
        };
        h.frame(vec![Event::PointerMoved(centre)]);
        h.frame(vec![middle(true)]);
        h.frame(vec![middle(false)]);
        h.frame(vec![]);
        assert_eq!(h.viewport.layout(), (Arrangement::Four, None));

        // And alone again, then back, each with one middle click.
        for _ in 0..30 {
            h.frame(vec![]); // past the double-click delay
        }
        for solo in [Some(0), None] {
            let at = h.viewport.view_rect.center();
            let click = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Middle,
                pressed,
                modifiers: Modifiers::NONE,
            };
            h.frame(vec![Event::PointerMoved(at), click(true), click(false)]);
            h.frame(vec![]);
            assert_eq!(h.viewport.layout(), (Arrangement::Four, solo));
            for _ in 0..30 {
                h.frame(vec![]);
            }
        }
    }

    #[test]
    fn orthographic_views_pan_instead_of_orbiting() {
        let mut h = Harness::new();
        h.viewport.kind = ViewKind::Right;
        h.viewport.camera = ViewKind::Right.aim(h.viewport.camera);
        h.frame(vec![]);
        let before = h.viewport.camera;
        orbit(&mut h);
        let after = h.viewport.camera;
        assert_eq!(
            (after.yaw, after.pitch),
            (before.yaw, before.pitch),
            "still looking along −X"
        );
        assert_ne!(after.target, before.target, "panned");
    }

    #[test]
    fn a_material_dropped_on_an_object_is_assigned_to_it() {
        let mut h = Harness::new();
        let cube = h.doc.scene.roots()[0];
        let material = h.doc.scene.materials_mut().insert(grid4d_scene::Material::new("Mat"));
        h.frame(vec![]);
        let world = h.doc.scene.world_matrix(cube);
        let rect = h.viewport.view_rect;
        let view_proj = h.viewport.camera.view_projection(rect.aspect_ratio());
        let centre = project(&view_proj, rect, world.transform_point3(Vec3::ZERO)).unwrap();
        // As the Material Manager leaves it: a drag in progress, released over the cube.
        h.editor.materials.drag = Some(crate::MaterialDrag { material });
        h.frame(vec![Event::PointerMoved(centre)]);
        h.frame(vec![Harness::button(centre, false)]);
        let tags = &h.doc.scene.get(cube).unwrap().data.tags;
        assert_eq!(tags.material(), Some(material));
        assert_eq!(h.editor.history.undo_label(), Some("Assign Material"));
    }

    #[test]
    fn a_line_cut_drag_cuts_and_esc_cancels() {
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Polygons, Tool::LineCut);
        let before = h.mesh(cube);
        let world = h.doc.scene.world_matrix(cube);
        let rect = h.viewport.view_rect;
        let view_proj = h.viewport.camera.view_projection(rect.aspect_ratio());
        let centre = project(&view_proj, rect, world.transform_point3(Vec3::ZERO)).unwrap();
        let (from, to) = (centre - vec2(200.0, 0.0), centre + vec2(200.0, 0.0));

        // Esc while drawing: nothing.
        h.frame(vec![Event::PointerMoved(from)]);
        h.frame(vec![Harness::button(from, true)]);
        h.frame(vec![Event::PointerMoved(to)]);
        h.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![Harness::button(to, false)]);
        h.frame(vec![]);
        assert_eq!(h.mesh(cube).id(), before.id(), "Esc cancels");

        h.drag(from, to);
        let after = h.mesh(cube);
        assert!(
            after.mesh().face_count() > 6,
            "the visible faces were cut: {}",
            after.mesh().face_count()
        );
        assert_eq!(h.editor.history.undo_label(), Some("Line Cut"));
        assert!(!h.selected(cube, Component::Edge).is_empty(), "the cuts are selected");
    }

    #[test]
    fn bevel_drag_bridge_drag_and_weld_click() {
        // Bevel: drag with one edge selected, like Extrude.
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Edges, Tool::Bevel);
        h.select(cube, Component::Edge, vec![0]);
        let centre = h.viewport.view_rect.center();
        h.drag(centre, centre + vec2(40.0, 0.0));
        assert_eq!(h.mesh(cube).mesh().face_count(), 7);
        assert_eq!(h.editor.history.undo_label(), Some("Bevel"));

        // Bridge: drag from the front face's middle polygon to the top's.
        let mut h = Harness::new();
        let cube = h.doc.scene.iter().find(|(_, n)| n.data.name == "Cube").unwrap().0;
        h.doc.scene.get_mut(cube).unwrap().data.kind = ObjectKind::Cube {
            size: Vec3::splat(200.0),
            segments: [3, 3, 3],
        };
        let cube = h.edit("Cube", EditMode::Polygons, Tool::Bridge);
        let mesh = h.mesh(cube);
        let world = h.doc.scene.world_matrix(cube);
        let rect = h.viewport.view_rect;
        let view_proj = h.viewport.camera.view_projection(rect.aspect_ratio());
        let middle_of = |normal: Vec3| {
            let f = (0..mesh.mesh().face_count())
                .find(|&f| {
                    let c = mesh.mesh().face_center(f) * (Vec3::ONE - normal.abs());
                    mesh.mesh().face_normal(f).abs_diff_eq(normal, 1e-4) && c.length() < 1e-3
                })
                .unwrap();
            project(&view_proj, rect, world.transform_point3(mesh.mesh().face_center(f))).unwrap()
        };
        let (front, top) = (middle_of(Vec3::Z), middle_of(Vec3::Y));
        h.drag(front, top);
        assert_eq!(h.mesh(cube).mesh().face_count(), 54 - 2 + 4);
        assert_eq!(h.editor.history.undo_label(), Some("Bridge"));

        // Weld: select two points, click empty space: they meet in the middle.
        let mut h = Harness::new();
        let cube = h.edit("Cube", EditMode::Points, Tool::Weld);
        let [a, b] = h.mesh(cube).topology().edges()[0];
        h.select(cube, Component::Point, vec![a, b]);
        let corner = h.viewport.view_rect.left_top() + vec2(20.0, 20.0);
        h.frame(vec![Event::PointerMoved(corner)]);
        h.frame(vec![Harness::button(corner, true)]);
        h.frame(vec![Harness::button(corner, false)]);
        h.frame(vec![]);
        assert_eq!(h.mesh(cube).mesh().point_count(), 7);
        assert_eq!(h.editor.history.undo_label(), Some("Weld"));
    }

    #[test]
    fn a_subdivision_surface_is_picked_by_its_result_and_its_cage_is_edited() {
        let mut h = Harness::new();
        let cube = h.editor.selection.active().unwrap();
        h.editor.commands.push(EditorCommand::MakeEditable);
        h.frame(vec![]);
        h.editor
            .commands
            .push(EditorCommand::WrapInGenerator(ObjectKindTag::SubdivisionSurface));
        h.frame(vec![]);
        let sds = h.editor.selection.active().unwrap();
        assert_ne!(sds, cube);

        // Model mode: a click on the smooth surface selects the generator.
        h.editor.selection.clear();
        h.frame(vec![]);
        let rect = h.viewport.view_rect;
        let view_proj = h.viewport.camera.view_projection(rect.aspect_ratio());
        let centre = h.doc.scene.world_matrix(cube).transform_point3(Vec3::ZERO);
        let at = project(&view_proj, rect, centre).unwrap();
        h.frame(vec![Event::PointerMoved(at)]);
        h.frame(vec![Harness::button(at, true)]);
        h.frame(vec![Harness::button(at, false)]);
        h.frame(vec![]);
        assert_eq!(h.editor.selection.as_slice(), &[sds]);

        // Points mode on the cage: its corners stay visible outside the smooth surface.
        h.editor.selection.set_single(cube);
        h.editor.mode = EditMode::Points;
        h.frame(vec![]);
        let world = h.doc.scene.world_matrix(cube);
        let eye = h.viewport.camera.eye();
        let (nearest, position) = h
            .mesh(cube)
            .mesh()
            .points()
            .iter()
            .map(|&p| world.transform_point3(p))
            .enumerate()
            .min_by(|a, b| a.1.distance(eye).total_cmp(&b.1.distance(eye)))
            .unwrap();
        let at = project(&view_proj, rect, position).unwrap() + vec2(2.0, 1.0);
        h.frame(vec![Event::PointerMoved(at)]);
        assert_eq!(h.viewport.hover, [ComponentHit::Point(u32::try_from(nearest).unwrap())]);
    }

    #[test]
    fn a_click_without_drag_records_nothing() {
        let mut h = Harness::new();
        h.frame(vec![]);
        let geo = GizmoGeometry::new(&h.doc.scene, &h.editor, &h.viewport.camera, h.viewport.view_rect, 0.0).unwrap();
        let (start, _) = geo.handle_for_test(1).unwrap();
        h.frame(vec![Event::PointerMoved(start)]);
        h.frame(vec![Harness::button(start, true)]);
        h.frame(vec![Harness::button(start, false)]);
        h.frame(vec![]);
        assert!(!h.editor.history.can_undo());
    }
}
