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
    ComponentHit, ComponentStyle, FrameData, OrbitCamera, ScreenView, SnapCache, SnapHit, pick, pick_component,
};
use grid4d_scene::{Aabb, Document, NodeId, ObjectKindTag, Scene, WalkItem};

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

#[derive(Debug)]
pub struct ViewportPanel {
    camera: OrbitCamera,
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
}

impl ViewportPanel {
    /// Creates the panel. The viewport renders into its own offscreen target
    /// (own MSAA and depth), so nothing about the window's surface matters here.
    /// Without a render state (tests, a non-wgpu backend) it shows a notice.
    pub fn new(render_state: Option<&egui_wgpu::RenderState>) -> Self {
        if render_state.is_none() {
            log::error!("No wgpu render state: the 3D viewport is disabled. Run with the wgpu renderer.");
        }
        Self {
            camera: OrbitCamera::default(),
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

    pub fn show(&mut self, ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
        self.header(ui, editor);

        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return;
        }
        self.view_rect = rect;

        // With Live Selection in a component mode, a middle drag sizes the brush.
        let brush = (editor.tool == Tool::LiveSelection && editor.mode.component().is_some())
            .then_some(&mut editor.selection_settings.radius);
        let navigating = self.navigate(ui, &response, rect, brush);
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

        // ── Paint ────────────────────────────────────────────────────────────
        let painter = ui.painter_at(rect);
        overlay::background(&painter, rect);
        // A gizmo drag above may have moved an object; free if nothing changed.
        // Animation is shown at the current (whole) frame.
        self.evaluated.set_frame(f64::from(doc.time.frame()));
        self.evaluated.update(&doc.scene);
        self.drop_material(ui, rect, editor);
        // Component modes: the active polygon object, and what a click would select.
        let target = self.component_target(editor);
        self.update_hover(target, &response, rect, editor, navigating);

        if let Some(gpu) = &mut self.gpu {
            self.frame.set_camera(&self.camera, rect.aspect_ratio());
            self.frame.show_grid = self.show_grid;
            self.frame.extract(&self.evaluated, |id| editor.selection.contains(id));
            self.frame.component_style = component_style(ui.ctx().pixels_per_point());
            self.frame.extract_components(&self.evaluated, target, &self.hover);
            let texture = gpu.render(&self.frame, pixel_size(rect, ui.ctx().pixels_per_point()));
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

        overlay::view_label(&painter, rect, "Perspective");
        if editor.animation.autokey {
            overlay::autokey_frame(&painter, rect);
        }
        let view_proj = self.camera.view_projection(rect.aspect_ratio());
        overlay::selection_brackets(&painter, rect, &view_proj, &self.evaluated, &editor.selection);
        if let Some(geo) = &gizmo_geo {
            geo.paint(&painter, self.gizmo.hot_axis());
        }
        if self.show_axis_triad {
            overlay::axis_triad(&painter, rect, &self.camera);
        }
        if self.show_hud {
            overlay::hud(&painter, rect, &self.camera);
        }
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
        ScreenView {
            view_proj: self.camera.view_projection(rect.aspect_ratio()),
            eye: self.camera.eye(),
            size: GVec2::new(rect.width(), rect.height()),
        }
    }

    /// Viewport header with C4D's menu set and the navigation drag-icons.
    fn header(&mut self, ui: &mut Ui, editor: &mut EditorState) {
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
                            ui.add(egui::Button::new("Default Camera").selected(true));
                            ui.add_enabled(false, egui::Button::new("Use Scene Camera — planned"));
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
                            ui.add_enabled(false, egui::Button::new("4-View Arrangement — planned"));
                        });
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 1.0;
                        let style = IconStyle {
                            size: 22.0,
                            ..IconStyle::default()
                        };
                        let orbit = icon_button(ui, Icon::Rotate, style, "Drag to orbit").interact(Sense::drag());
                        if orbit.dragged() {
                            let d = orbit.drag_delta();
                            self.camera.orbit(-d.x * ORBIT_SPEED, d.y * ORBIT_SPEED);
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
            self.camera.orbit(-d.x * ORBIT_SPEED, d.y * ORBIT_SPEED);
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
            let grid4d_scene::ObjectKind::Polygon { mesh, .. } = &self.doc.scene.get(id).unwrap().data.kind else {
                panic!("not a polygon object");
            };
            mesh.clone()
        }

        fn components(&self, id: NodeId) -> grid4d_scene::Components {
            let grid4d_scene::ObjectKind::Polygon { components, .. } = &self.doc.scene.get(id).unwrap().data.kind
            else {
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
            let grid4d_scene::ObjectKind::Polygon { components, .. } = &self.doc.scene.get(id).unwrap().data.kind
            else {
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
        let grid4d_scene::ObjectKind::Polygon { mesh, .. } = &node.data.kind else {
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
        let grid4d_scene::ObjectKind::Polygon { mesh, .. } = &node.data.kind else {
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
        let grid4d_scene::ObjectKind::Polygon { components, .. } = &node.data.kind else {
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
            let null = grid4d_scene::Node::new("Target", grid4d_scene::ObjectKind::Null)
                .with_position(Vec3::new(400.0, 100.0, 0.0));
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
                grid4d_scene::ObjectKind::Polygon { mesh, .. } => mesh.mesh().face_count(),
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
        let grid4d_scene::ObjectKind::Polygon { mesh, .. } = &node.data.kind else {
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
        assert_eq!(grid4d_scene::Tags::material(tags.as_slice()), Some(material));
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
        h.doc.scene.get_mut(cube).unwrap().data.kind = grid4d_scene::ObjectKind::Cube {
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
