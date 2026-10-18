//! The 3D viewport: header menus, navigation, picking, gizmo and the GPU view.

mod gizmo;
mod gpu_view;
mod overlay;

use egui::{Align, Align2, Layout, Margin, PointerButton, Pos2, Rect, Response, Sense, Ui, pos2};
use glam::{Mat4, Vec2 as GVec2, Vec3};
use grid4d_eval::EvaluatedScene;
use grid4d_render::{FrameData, OrbitCamera, pick};
use grid4d_scene::{Aabb, Document, ObjectKindTag, Scene, WalkItem};

use self::{gizmo::GizmoGeometry, gizmo::GizmoState, gpu_view::GpuView};
use crate::{
    EditorCommand, EditorState, SelectMode, Selection,
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
    show_grid: bool,
    show_axis_triad: bool,
    show_hud: bool,
    /// The 3D area last frame (the header's pan handle needs its height).
    view_rect: Rect,
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
            show_grid: true,
            show_axis_triad: true,
            show_hud: true,
            view_rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)),
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

        let navigating = self.navigate(ui, &response, rect);
        // Built once per frame (after navigation moved the camera) and shared by
        // interaction and painting; rebuilt only if a drag just moved the object.
        let mut gizmo_geo = GizmoGeometry::new(&doc.scene, editor, &self.camera, rect);
        let gizmo_owns_pointer = if navigating {
            false
        } else {
            let interaction =
                self.gizmo
                    .interact(ui, &response, gizmo_geo.as_ref(), &mut doc.scene, &mut editor.commands);
            if interaction.moved_object {
                gizmo_geo = GizmoGeometry::new(&doc.scene, editor, &self.camera, rect);
            }
            interaction.owns_pointer
        };
        if !navigating && !gizmo_owns_pointer {
            self.evaluated.update(&doc.scene);
            self.pick_on_click(ui, &response, rect, editor);
        }
        self.context_menu(&response, editor);

        // ── Paint ────────────────────────────────────────────────────────────
        let painter = ui.painter_at(rect);
        overlay::background(&painter, rect);
        // A gizmo drag above may have moved an object; free if nothing changed.
        self.evaluated.update(&doc.scene);

        if let Some(gpu) = &mut self.gpu {
            self.frame.set_camera(&self.camera, rect.aspect_ratio());
            self.frame.show_grid = self.show_grid;
            self.frame.extract(&self.evaluated, |id| editor.selection.contains(id));
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

        if response.hovered() {
            let alt = ui.input(|i| i.modifiers.alt);
            editor.status_hint = Some(if alt { NAV_HINT } else { editor.tool.hint() });
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
    fn navigate(&mut self, ui: &Ui, response: &Response, rect: Rect) -> bool {
        let (alt, scroll) = ui.input(|i| (i.modifiers.alt, i.smooth_scroll_delta.y));
        let d = response.drag_delta();
        let mut active = false;

        if response.dragged_by(PointerButton::Middle) {
            self.camera.pan(GVec2::new(d.x, d.y), rect.height());
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
        let ray = self.camera.ray_from_ndc(to_ndc(pointer, rect), rect.aspect_ratio());
        let mode = SelectMode::from_modifiers(modifiers);
        match pick(&self.evaluated, ray) {
            Some(id) => editor.commands.push(EditorCommand::Select { id, mode }),
            None if mode == SelectMode::Replace => editor.commands.push(EditorCommand::DeselectAll),
            None => {}
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
    use crate::{EditorCommand, Tool, commands};

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

        fn position(&self, id: grid4d_scene::NodeId) -> Vec3 {
            self.doc.scene.get(id).unwrap().data.transform.position
        }
    }

    #[test]
    fn a_gizmo_drag_is_one_undo_step() {
        let mut h = Harness::new();
        h.frame(vec![]); // lay out once so the viewport knows its 3D rect
        let cube = h.editor.selection.active().unwrap();
        let geo = GizmoGeometry::new(&h.doc.scene, &h.editor, &h.viewport.camera, h.viewport.view_rect).unwrap();
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
    fn a_click_without_drag_records_nothing() {
        let mut h = Harness::new();
        h.frame(vec![]);
        let geo = GizmoGeometry::new(&h.doc.scene, &h.editor, &h.viewport.camera, h.viewport.view_rect).unwrap();
        let (start, _) = geo.handle_for_test(1).unwrap();
        h.frame(vec![Event::PointerMoved(start)]);
        h.frame(vec![Harness::button(start, true)]);
        h.frame(vec![Harness::button(start, false)]);
        h.frame(vec![]);
        assert!(!h.editor.history.can_undo());
    }
}
