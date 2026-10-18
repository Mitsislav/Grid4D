//! The 3D viewport: header menus, navigation, picking, gizmo and the GPU view.

mod callback;
mod gizmo;
mod overlay;

use std::sync::{Arc, Mutex, PoisonError};

use egui::{Align, Align2, Layout, Margin, PointerButton, Pos2, Rect, Response, Sense, Ui, pos2};
use egui_wgpu::wgpu;
use glam::{Mat4, Vec2 as GVec2, Vec3};
use oc4d_render::{FrameData, GpuTargetConfig, OrbitCamera, ViewportRenderer, pick};
use oc4d_scene::{Aabb, Document, ObjectKindTag, Scene, WalkItem};

use self::{callback::ViewportCallback, gizmo::GizmoGeometry, gizmo::GizmoState};
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
    /// Render snapshot shared with the paint callback (no per-frame allocation).
    frame: Arc<Mutex<FrameData>>,
    gpu_ready: bool,
    walk_stack: Vec<WalkItem>,
    gizmo: GizmoState,
    show_grid: bool,
    show_axis_triad: bool,
    show_hud: bool,
    /// Height of the 3D area last frame (used by the header's pan handle).
    view_height: f32,
}

impl ViewportPanel {
    /// Creates the panel and installs the GPU renderer into egui-wgpu's resources.
    ///
    /// `depth_format` and `msaa_samples` must match the host's `NativeOptions`
    /// (`depth_buffer`, `multisampling`), since the viewport draws into egui's pass.
    pub fn new(
        render_state: Option<&egui_wgpu::RenderState>,
        depth_format: Option<wgpu::TextureFormat>,
        msaa_samples: u32,
    ) -> Self {
        let gpu_ready = match render_state {
            Some(rs) => {
                let target = GpuTargetConfig {
                    color_format: rs.target_format,
                    depth_format,
                    msaa_samples,
                };
                let renderer = ViewportRenderer::new(&rs.device, &target);
                rs.renderer.write().callback_resources.insert(renderer);
                true
            }
            None => {
                log::error!("No wgpu render state: the 3D viewport is disabled. Run with the wgpu renderer.");
                false
            }
        };
        Self {
            camera: OrbitCamera::default(),
            frame: Arc::new(Mutex::new(FrameData::new())),
            gpu_ready,
            walk_stack: Vec::with_capacity(64),
            gizmo: GizmoState::default(),
            show_grid: true,
            show_axis_triad: true,
            show_hud: true,
            view_height: 600.0,
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
        self.view_height = rect.height();

        let navigating = self.navigate(ui, &response, rect);
        let gizmo_owns_pointer = if navigating {
            false
        } else {
            let geo = GizmoGeometry::new(&doc.scene, editor, &self.camera, rect);
            self.gizmo.interact(ui, &response, geo.as_ref(), &mut doc.scene)
        };
        if !navigating && !gizmo_owns_pointer {
            self.pick_on_click(ui, &response, rect, &doc.scene, editor);
        }
        self.context_menu(&response, editor);

        // ── Paint ────────────────────────────────────────────────────────────
        let painter = ui.painter_at(rect);
        overlay::background(&painter, rect);

        if self.gpu_ready {
            {
                let mut frame = self.frame.lock().unwrap_or_else(PoisonError::into_inner);
                frame.set_camera(&self.camera, rect.aspect_ratio());
                frame.show_grid = self.show_grid;
                frame.extract(&doc.scene, |id| editor.selection.contains(id));
            }
            painter.add(egui_wgpu::Callback::new_paint_callback(
                rect,
                ViewportCallback::new(Arc::clone(&self.frame)),
            ));
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
        overlay::selection_brackets(
            &painter,
            rect,
            &view_proj,
            &doc.scene,
            &editor.selection,
            &mut self.walk_stack,
        );
        if let Some(geo) = GizmoGeometry::new(&doc.scene, editor, &self.camera, rect) {
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
                            self.camera.pan(GVec2::new(d.x, d.y), self.view_height);
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

    fn pick_on_click(&mut self, ui: &Ui, response: &Response, rect: Rect, scene: &Scene, editor: &mut EditorState) {
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
        match pick(scene, ray, &mut self.walk_stack) {
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
