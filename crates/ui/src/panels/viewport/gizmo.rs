//! Move / Scale / Rotate manipulator for the active object.
//!
//! All gizmo geometry is computed on the CPU in screen space and drawn with the
//! egui painter as an overlay, so it is always on top and resolution-independent.

use std::f32::consts::{PI, TAU};

use egui::{Color32, CornerRadius, PointerButton, Pos2, Rect, Response, Shape, Stroke, Ui, Vec2};
use glam::{Mat4, Quat, Vec3};
use oc4d_render::OrbitCamera;
use oc4d_scene::{NodeData, NodeId, Scene, SceneEdit};

use super::project;
use crate::{CommandQueue, EditorCommand, EditorState, Tool, theme::palette as p};

/// On-screen length of the axis handles, in points.
const HANDLE_PX: f32 = 90.0;
const RING_SEGMENTS: usize = 64;
const PICK_TOLERANCE: f32 = 8.0;

/// Screen-space description of the gizmo for the current frame.
pub(super) struct GizmoGeometry {
    node: NodeId,
    tool: Tool,
    origin_px: Pos2,
    /// World-space unit axes the handles follow.
    axes: [Vec3; 3],
    /// World length of a handle (so that it is `HANDLE_PX` long on screen).
    length: f32,
    ends_px: [Option<Pos2>; 3],
    rings: [Option<[Pos2; RING_SEGMENTS]>; 3],
    /// Whether each axis points towards the camera (flips rotation direction).
    toward_camera: [bool; 3],
    enabled: [bool; 3],
    parent_world: Mat4,
}

impl GizmoGeometry {
    /// Builds the gizmo for the active object, if the active tool has one.
    pub(super) fn new(scene: &Scene, editor: &EditorState, camera: &OrbitCamera, rect: Rect) -> Option<Self> {
        if !editor.tool.has_gizmo() {
            return None;
        }
        let id = editor.selection.active()?;
        scene.get(id)?;

        let world = scene.world_matrix(id);
        let (_, rotation, origin) = world.to_scale_rotation_translation();
        let use_world_axes = editor.world_coordinates && editor.tool != Tool::Scale;
        let axes = if use_world_axes {
            [Vec3::X, Vec3::Y, Vec3::Z]
        } else {
            [rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z]
        };

        let view_proj = camera.view_projection(rect.aspect_ratio());
        let origin_px = project(&view_proj, rect, origin)?;
        let depth = (origin - camera.eye()).dot(camera.forward()).max(camera.near);
        let length = camera.world_units_per_pixel(depth, rect.height()) * HANDLE_PX;
        let to_eye = camera.eye() - origin;

        let ends_px = axes.map(|axis| project(&view_proj, rect, origin + axis * length));
        // Only the rotate tool draws and hit-tests the rings.
        let rings = if editor.tool == Tool::Rotate {
            std::array::from_fn(|i| {
                let (u, v) = (axes[(i + 1) % 3], axes[(i + 2) % 3]);
                let mut ring = [Pos2::ZERO; RING_SEGMENTS];
                for (k, point) in ring.iter_mut().enumerate() {
                    let a = TAU * k as f32 / RING_SEGMENTS as f32;
                    let world_point = origin + (u * a.cos() + v * a.sin()) * length * 0.85;
                    *point = project(&view_proj, rect, world_point)?;
                }
                Some(ring)
            })
        } else {
            [None; 3]
        };

        Some(Self {
            node: id,
            tool: editor.tool,
            origin_px,
            axes,
            length,
            ends_px,
            rings,
            toward_camera: axes.map(|a| a.dot(to_eye) > 0.0),
            enabled: editor.axes,
            parent_world: scene.parent_world_matrix(id),
        })
    }

    /// A point on the Move/Scale handle of `axis` and the handle's screen direction.
    #[cfg(test)]
    pub(super) fn handle_for_test(&self, axis: usize) -> Option<(Pos2, Vec2)> {
        let end = self.ends_px[axis]?;
        let along = end - self.origin_px;
        Some((self.origin_px + along * 0.6, along.normalized()))
    }

    /// The handle under `pointer`, if any.
    fn hit(&self, pointer: Pos2) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for axis in 0..3 {
            if !self.enabled[axis] {
                continue;
            }
            let distance = match self.tool {
                Tool::Rotate => self.rings[axis].as_ref().map(|ring| polyline_distance(ring, pointer)),
                _ => self.ends_px[axis].map(|end| {
                    let start = self.origin_px + (end - self.origin_px) * 0.15;
                    segment_distance(start, end, pointer)
                }),
            };
            if let Some(d) = distance
                && d < PICK_TOLERANCE
                && best.is_none_or(|(bd, _)| d < bd)
            {
                best = Some((d, axis));
            }
        }
        best.map(|(_, axis)| axis)
    }

    pub(super) fn paint(&self, painter: &egui::Painter, hot: Option<usize>) {
        for axis in 0..3 {
            if !self.enabled[axis] {
                continue;
            }
            let color = if hot == Some(axis) { p::HIGHLIGHT } else { p::AXIS[axis] };
            let stroke = Stroke::new(2.2, color);
            match self.tool {
                Tool::Rotate => {
                    if let Some(ring) = &self.rings[axis] {
                        painter.add(Shape::closed_line(ring.to_vec(), stroke));
                    }
                }
                Tool::Move | Tool::Scale => {
                    let Some(end) = self.ends_px[axis] else { continue };
                    let dir = end - self.origin_px;
                    if dir.length() < 6.0 {
                        continue; // Axis points straight at the camera.
                    }
                    painter.line_segment([self.origin_px, end], stroke);
                    if self.tool == Tool::Move {
                        arrow_head(painter, end, dir.normalized(), 13.0, color);
                    } else {
                        painter.rect_filled(
                            Rect::from_center_size(end, Vec2::splat(9.0)),
                            CornerRadius::same(1),
                            color,
                        );
                    }
                }
                Tool::LiveSelection => {}
            }
        }
        painter.circle_filled(self.origin_px, 3.5, p::TEXT_STRONG);
        painter.circle_stroke(self.origin_px, 3.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
    }
}

#[derive(Debug, Clone)]
struct Drag {
    node: NodeId,
    axis: usize,
    last_pointer: Pos2,
    /// The node's data when the drag started: the whole drag is one undo step.
    before: NodeData,
    label: &'static str,
}

/// Interaction state that survives between frames.
#[derive(Debug, Default)]
pub(super) struct GizmoState {
    hot: Option<usize>,
    drag: Option<Drag>,
}

/// What [`GizmoState::interact`] did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct GizmoInteraction {
    /// The gizmo owns the pointer, so the viewport must not treat the click as a pick.
    pub owns_pointer: bool,
    /// A drag changed the object's transform: geometry built before it is stale.
    pub moved_object: bool,
}

impl GizmoState {
    pub(super) fn hot_axis(&self) -> Option<usize> {
        self.drag.as_ref().map(|d| d.axis).or(self.hot)
    }

    /// Handles hover and drag. The transform is written live while dragging;
    /// when the drag ends, one undo step for it is pushed to `commands`.
    pub(super) fn interact(
        &mut self,
        ui: &Ui,
        response: &Response,
        geo: Option<&GizmoGeometry>,
        scene: &mut Scene,
        commands: &mut CommandQueue,
    ) -> GizmoInteraction {
        let alt = ui.input(|i| i.modifiers.alt);

        if let Some(drag) = &mut self.drag {
            let mut moved_object = false;
            if response.dragged_by(PointerButton::Primary)
                && let (Some(geo), Some(pointer)) = (geo, response.interact_pointer_pos())
                && geo.node == drag.node
            {
                moved_object = pointer != drag.last_pointer;
                apply_drag(geo, drag, pointer, scene);
                drag.last_pointer = pointer;
            } else if let Some(done) = self.drag.take() {
                // Recorded only if the drag actually changed something.
                commands.push(EditorCommand::CommitLive {
                    inverse: SceneEdit::SetData {
                        id: done.node,
                        data: done.before,
                    },
                    label: done.label,
                });
            }
            return GizmoInteraction {
                owns_pointer: true,
                moved_object,
            };
        }

        self.hot = match (geo, response.hover_pos()) {
            (Some(geo), Some(pointer)) if !alt => geo.hit(pointer),
            _ => None,
        };

        if response.drag_started_by(PointerButton::Primary) && !alt {
            // Hit-test where the press happened, not where the drag threshold was crossed.
            let press = ui.input(|i| i.pointer.press_origin());
            if let (Some(geo), Some(press)) = (geo, press)
                && let Some(axis) = geo.hit(press)
                && let Some(node) = scene.get(geo.node)
            {
                self.drag = Some(Drag {
                    node: geo.node,
                    axis,
                    last_pointer: response.interact_pointer_pos().unwrap_or(press),
                    before: node.data.clone(),
                    label: geo.tool.label(),
                });
                return GizmoInteraction {
                    owns_pointer: true,
                    moved_object: false,
                };
            }
        }
        GizmoInteraction {
            owns_pointer: self.hot.is_some() && response.clicked(),
            moved_object: false,
        }
    }
}

fn apply_drag(geo: &GizmoGeometry, drag: &Drag, pointer: Pos2, scene: &mut Scene) {
    let Some(node) = scene.get_mut(drag.node) else { return };
    let axis = drag.axis;
    let t = &mut node.data.transform;

    match geo.tool {
        Tool::Move | Tool::Scale => {
            let Some(end) = geo.ends_px[axis] else { return };
            let screen_axis = end - geo.origin_px;
            let len_px = screen_axis.length();
            if len_px < 1.0 {
                return;
            }
            let delta_px = (pointer - drag.last_pointer).dot(screen_axis / len_px);
            if geo.tool == Tool::Move {
                let world_delta = geo.axes[axis] * (delta_px / len_px * geo.length);
                t.position += geo.parent_world.inverse().transform_vector3(world_delta);
            } else {
                let factor = (1.0 + delta_px / len_px).max(0.01);
                t.scale[axis] = (t.scale[axis] * factor).max(0.001);
            }
        }
        Tool::Rotate => {
            let before = drag.last_pointer - geo.origin_px;
            let after = pointer - geo.origin_px;
            if before.length() < 2.0 || after.length() < 2.0 {
                return;
            }
            let mut delta = after.y.atan2(after.x) - before.y.atan2(before.x);
            if delta > PI {
                delta -= TAU;
            } else if delta < -PI {
                delta += TAU;
            }
            // Screen y points down: clockwise on screen is a negative rotation
            // about an axis that points at the viewer.
            let angle = if geo.toward_camera[axis] { -delta } else { delta };
            let world_delta = Quat::from_axis_angle(geo.axes[axis], angle);
            let (_, parent_rotation, _) = geo.parent_world.to_scale_rotation_translation();
            let local = parent_rotation.inverse() * world_delta * parent_rotation * t.rotation_quat();
            t.set_rotation_quat(local);
        }
        Tool::LiveSelection => {}
    }
}

fn arrow_head(painter: &egui::Painter, tip: Pos2, dir: Vec2, size: f32, color: Color32) {
    let back = tip - dir * size;
    let side = dir.rot90() * size * 0.42;
    painter.add(Shape::convex_polygon(
        vec![tip + dir * 2.0, back + side, back - side],
        color,
        Stroke::NONE,
    ));
}

fn segment_distance(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

fn polyline_distance(points: &[Pos2; RING_SEGMENTS], p: Pos2) -> f32 {
    (0..RING_SEGMENTS)
        .map(|i| segment_distance(points[i], points[(i + 1) % RING_SEGMENTS], p))
        .fold(f32::INFINITY, f32::min)
}
