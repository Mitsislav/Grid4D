//! Move / Scale / Rotate manipulator for the active object — or, in a component
//! mode, for the selected points, edges or polygons of the active polygon object.
//!
//! All gizmo geometry is computed on the CPU in screen space and drawn with the
//! egui painter as an overlay, so it is always on top and resolution-independent.

use std::f32::consts::{PI, TAU};

use egui::{Color32, CornerRadius, PointerButton, Pos2, Rect, Response, Shape, Stroke, Ui, Vec2};
use glam::{Mat3, Mat4, Quat, Vec3};
use grid4d_geom::{BitSet, Component, selection};
use grid4d_render::OrbitCamera;
use grid4d_scene::{MeshHandle, NodeData, NodeId, ObjectKind, Scene, SceneEdit, Tags};

use super::{
    project,
    snapping::{Snapper, generators_above},
};
use crate::{CommandQueue, EditorCommand, EditorState, Tool, keys::Keying, theme::palette as p};

/// On-screen length of the axis handles, in points.
const HANDLE_PX: f32 = 90.0;
const RING_SEGMENTS: usize = 64;
const PICK_TOLERANCE: f32 = 8.0;

/// What the gizmo transforms.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    /// The active object's transform.
    Object,
    /// The selected components of the active polygon object (`world`: its
    /// world matrix); the gizmo sits at the centre of their points.
    Components { component: Component, world: Mat4 },
}

/// Screen-space description of the gizmo for the current frame.
pub(super) struct GizmoGeometry {
    node: NodeId,
    tool: Tool,
    target: Target,
    /// World position of the gizmo's centre (the pivot).
    pub(super) origin: Vec3,
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
    /// Builds the gizmo for the active object, if the active tool has one, where
    /// it is shown at `frame` (animation). In a component mode it works on the
    /// selected components (centred on them) and is absent when nothing is
    /// selected.
    pub(super) fn new(
        scene: &Scene,
        editor: &EditorState,
        camera: &OrbitCamera,
        rect: Rect,
        frame: f64,
    ) -> Option<Self> {
        if !editor.tool.has_gizmo() {
            return None;
        }
        let id = editor.selection.active()?;
        let node = scene.get(id)?;
        // A Protection tag keeps the tool's channel of the object as it is.
        if editor.mode.component().is_none() {
            let protection = node.data.tags.protection();
            let locked = match editor.tool {
                Tool::Move => protection.position,
                Tool::Scale => protection.scale,
                Tool::Rotate => protection.rotation,
                Tool::LiveSelection
                | Tool::RectangleSelection
                | Tool::LoopSelection
                | Tool::RingSelection
                | Tool::Extrude
                | Tool::Inset
                | Tool::LoopCut
                | Tool::LineCut
                | Tool::Bevel
                | Tool::Bridge
                | Tool::Weld => false,
            };
            if locked {
                return None;
            }
        }

        let world = scene.world_matrix_at(id, frame);
        let (_, rotation, translation) = world.to_scale_rotation_translation();
        let (origin, target) = match editor.mode.component() {
            None => (translation, Target::Object),
            Some(component) => {
                let ObjectKind::Polygon { mesh, components } = &node.data.kind else {
                    return None;
                };
                let points = selection::points_of(mesh.mesh(), components.selected().get(component), component);
                let centre = centre_of(mesh, &points)?;
                (world.transform_point3(centre), Target::Components { component, world })
            }
        };
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
            target,
            origin,
            origin_px,
            axes,
            length,
            ends_px,
            rings,
            toward_camera: axes.map(|a| a.dot(to_eye) > 0.0),
            enabled: editor.axes,
            parent_world: scene.parent_world_matrix_at(id, frame),
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
                Tool::LiveSelection
                | Tool::RectangleSelection
                | Tool::LoopSelection
                | Tool::RingSelection
                | Tool::Extrude
                | Tool::Inset
                | Tool::LoopCut
                | Tool::LineCut
                | Tool::Bevel
                | Tool::Bridge
                | Tool::Weld => {}
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
    /// For a component transform: the points it moves.
    points: Option<PointEdit>,
    /// World position of the gizmo's centre when the drag started.
    start: Vec3,
    /// Move of an object: the world offset so far, before snapping.
    free: Vec3,
    /// What moves with the drag besides the dragged element (snapping ignores it).
    skip: Vec<NodeId>,
}

/// A transform of some of a polygon object's points, accumulated over a drag and
/// applied to the points as they were when it started — so nothing drifts, and
/// each frame writes a new mesh (new `MeshId`) whose selection is untouched.
#[derive(Debug, Clone)]
pub(super) struct PointEdit {
    original: MeshHandle,
    points: Vec<u32>,
    /// The object's world matrix (unchanged by the edit).
    world: Mat4,
    /// World position the edit scales and rotates around.
    pub(super) pivot: Vec3,
    /// So far: world translation, scale factor, rotation angle.
    pub(super) moved: Vec3,
    factor: f32,
    angle: f32,
    /// The translation before snapping (`moved` is where snapping put it).
    pub(super) free: Vec3,
    /// The edited points, for snapping to leave out.
    pub(super) moving: BitSet,
}

impl PointEdit {
    /// An edit of `points` of `mesh` (object world matrix `world`) around `pivot`.
    pub(super) fn new(mesh: &MeshHandle, points: &BitSet, world: Mat4, pivot: Vec3) -> Self {
        Self {
            original: mesh.clone(),
            points: points.iter().filter_map(|p| u32::try_from(p).ok()).collect(),
            world,
            pivot,
            moved: Vec3::ZERO,
            factor: 1.0,
            angle: 0.0,
            free: Vec3::ZERO,
            moving: points.clone(),
        }
    }

    /// The world-space transform so far, for `tool` along world axis `axis`.
    fn delta(&self, tool: Tool, axis: Vec3) -> Mat4 {
        let around = |m: Mat4| Mat4::from_translation(self.pivot) * m * Mat4::from_translation(-self.pivot);
        match tool {
            Tool::Scale => {
                // Scale by `factor` along `axis` only: I + (factor − 1) · axis axisᵀ.
                let k = self.factor - 1.0;
                let m = Mat3::from_cols(
                    Vec3::X + axis * (k * axis.x),
                    Vec3::Y + axis * (k * axis.y),
                    Vec3::Z + axis * (k * axis.z),
                );
                around(Mat4::from_mat3(m))
            }
            Tool::Rotate => around(Mat4::from_quat(Quat::from_axis_angle(axis, self.angle))),
            Tool::Move
            | Tool::LiveSelection
            | Tool::RectangleSelection
            | Tool::LoopSelection
            | Tool::RingSelection
            | Tool::Extrude
            | Tool::Inset
            | Tool::LoopCut
            | Tool::LineCut
            | Tool::Bevel
            | Tool::Bridge
            | Tool::Weld => Mat4::from_translation(self.moved),
        }
    }

    /// Writes the original mesh with world transform `delta` applied to the
    /// edited points (in object space) to `node`.
    pub(super) fn apply(&self, delta: Mat4, scene: &mut Scene, node: NodeId) {
        if self.world.determinant().abs() <= f32::EPSILON {
            return;
        }
        let local = self.world.inverse() * delta * self.world;
        let mut points = self.original.mesh().points().to_vec();
        for &p in &self.points {
            if let Some(point) = usize::try_from(p).ok().and_then(|p| points.get_mut(p)) {
                *point = local.transform_point3(*point);
            }
        }
        let moved = self.original.with_points(&points);
        if let Some(node) = scene.get_mut(node)
            && let ObjectKind::Polygon { mesh, .. } = &mut node.data.kind
        {
            *mesh = moved;
        }
    }
}

/// Centre of the bounding box of `points` of `mesh` (object space); `None` if empty.
pub(super) fn centre_of(mesh: &MeshHandle, points: &BitSet) -> Option<Vec3> {
    let positions = mesh.mesh().points();
    let mut found = points.iter().filter_map(|p| positions.get(p).copied());
    let first = found.next()?;
    let (min, max) = found.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p)));
    Some((min + max) * 0.5)
}

/// Undo label of a gizmo drag.
const fn drag_label(tool: Tool, target: Target) -> &'static str {
    let Target::Components { component, .. } = target else {
        return tool.label();
    };
    match (tool, component) {
        (Tool::Scale, Component::Point) => "Scale Points",
        (Tool::Scale, Component::Edge) => "Scale Edges",
        (Tool::Scale, Component::Polygon) => "Scale Polygons",
        (Tool::Rotate, Component::Point) => "Rotate Points",
        (Tool::Rotate, Component::Edge) => "Rotate Edges",
        (Tool::Rotate, Component::Polygon) => "Rotate Polygons",
        (_, component) => move_label(component),
    }
}

/// Undo label of moving components (the gizmo, or Tweak).
pub(super) const fn move_label(component: Component) -> &'static str {
    match component {
        Component::Point => "Move Points",
        Component::Edge => "Move Edges",
        Component::Polygon => "Move Polygons",
    }
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

    /// Handles hover and drag. The transform is written live while dragging, as
    /// `keying` says (keys or temporary values for animated parameters); when the
    /// drag ends, one undo step for it is pushed to `commands`.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call site; the pieces of one frame's input"
    )]
    pub(super) fn interact(
        &mut self,
        ui: &Ui,
        response: &Response,
        geo: Option<&GizmoGeometry>,
        scene: &mut Scene,
        commands: &mut CommandQueue,
        snapper: &mut Snapper<'_>,
        keying: Keying,
    ) -> GizmoInteraction {
        let alt = ui.input(|i| i.modifiers.alt);

        if let Some(drag) = &mut self.drag {
            let mut moved_object = false;
            if response.dragged_by(PointerButton::Primary)
                && let (Some(geo), Some(pointer)) = (geo, response.interact_pointer_pos())
                && geo.node == drag.node
            {
                moved_object = pointer != drag.last_pointer;
                if drag.points.is_some() {
                    apply_component_drag(geo, drag, pointer, scene, snapper);
                } else {
                    apply_drag(geo, drag, pointer, scene, snapper, keying);
                }
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
                let points = match (geo.target, &node.data.kind) {
                    (Target::Components { component, world }, ObjectKind::Polygon { mesh, components }) => {
                        let points = selection::points_of(mesh.mesh(), components.selected().get(component), component);
                        Some(PointEdit::new(mesh, &points, world, geo.origin))
                    }
                    _ => None,
                };
                // An object takes its children along; generators above change too.
                let mut skip = generators_above(scene, geo.node);
                if points.is_none() {
                    skip.push(geo.node);
                }
                self.drag = Some(Drag {
                    node: geo.node,
                    axis,
                    last_pointer: response.interact_pointer_pos().unwrap_or(press),
                    before: node.data.clone(),
                    label: drag_label(geo.tool, geo.target),
                    points,
                    start: geo.origin,
                    free: Vec3::ZERO,
                    skip,
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

/// How far the pointer moved along a Move/Scale handle, as a share of the handle's
/// on-screen length; `None` if the handle points at the camera.
fn along_handle(geo: &GizmoGeometry, axis: usize, from: Pos2, to: Pos2) -> Option<f32> {
    let end = geo.ends_px[axis]?;
    let screen_axis = end - geo.origin_px;
    let len_px = screen_axis.length();
    (len_px >= 1.0).then(|| (to - from).dot(screen_axis / len_px) / len_px)
}

/// The rotation (radians, about `axis`) of dragging around the gizmo from `from` to `to`.
fn ring_angle(geo: &GizmoGeometry, axis: usize, from: Pos2, to: Pos2) -> Option<f32> {
    let (before, after) = (from - geo.origin_px, to - geo.origin_px);
    if before.length() < 2.0 || after.length() < 2.0 {
        return None;
    }
    let mut delta = after.y.atan2(after.x) - before.y.atan2(before.x);
    if delta > PI {
        delta -= TAU;
    } else if delta < -PI {
        delta += TAU;
    }
    // Screen y points down: clockwise on screen is a negative rotation about an
    // axis that points at the viewer.
    Some(if geo.toward_camera[axis] { -delta } else { delta })
}

/// One frame of a component drag: accumulate, then rewrite the points.
fn apply_component_drag(
    geo: &GizmoGeometry,
    drag: &mut Drag,
    pointer: Pos2,
    scene: &mut Scene,
    snapper: &mut Snapper<'_>,
) {
    let (axis, node, last) = (drag.axis, drag.node, drag.last_pointer);
    let Drag {
        points: Some(edit),
        skip,
        ..
    } = drag
    else {
        return;
    };
    match geo.tool {
        Tool::Move => {
            if let Some(share) = along_handle(geo, axis, last, pointer) {
                edit.free += geo.axes[axis] * (share * geo.length);
            }
            let line = Some((edit.pivot, geo.axes[axis]));
            let to = snapper.snap(pointer, edit.pivot + edit.free, line, skip, Some((node, &edit.moving)));
            edit.moved = to - edit.pivot;
        }
        Tool::Scale => {
            if let Some(share) = along_handle(geo, axis, last, pointer) {
                edit.factor = (edit.factor * (1.0 + share).max(0.01)).max(0.001);
            }
        }
        Tool::Rotate => {
            if let Some(angle) = ring_angle(geo, axis, last, pointer) {
                edit.angle += angle;
            }
        }
        Tool::LiveSelection
        | Tool::RectangleSelection
        | Tool::LoopSelection
        | Tool::RingSelection
        | Tool::Extrude
        | Tool::Inset
        | Tool::LoopCut
        | Tool::LineCut
        | Tool::Bevel
        | Tool::Bridge
        | Tool::Weld => return,
    }
    edit.apply(edit.delta(geo.tool, geo.axes[axis]), scene, node);
}

/// One frame of an object drag: the transform shown at the current frame is
/// changed and written back through `keying` (see `NodeData::set_transform_at`).
fn apply_drag(
    geo: &GizmoGeometry,
    drag: &mut Drag,
    pointer: Pos2,
    scene: &mut Scene,
    snapper: &mut Snapper<'_>,
    keying: Keying,
) {
    let axis = drag.axis;
    if geo.tool == Tool::Move {
        // From the position the drag started at: the unsnapped offset, snapped.
        let Some(share) = along_handle(geo, axis, drag.last_pointer, pointer) else {
            return;
        };
        drag.free += geo.axes[axis] * (share * geo.length);
        let line = Some((drag.start, geo.axes[axis]));
        let to = snapper.snap(pointer, drag.start + drag.free, line, &drag.skip, None);
        let Some(node) = scene.get_mut(drag.node) else { return };
        let mut shown = node.data.transform_at(keying.frame);
        shown.position = drag.before.transform_at(keying.frame).position
            + geo.parent_world.inverse().transform_vector3(to - drag.start);
        node.data.set_transform_at(&shown, keying.frame, keying.autokey);
        return;
    }
    let Some(node) = scene.get_mut(drag.node) else { return };
    let mut shown = node.data.transform_at(keying.frame);
    let t = &mut shown;

    match geo.tool {
        Tool::Scale => {
            let Some(share) = along_handle(geo, axis, drag.last_pointer, pointer) else {
                return;
            };
            let factor = (1.0 + share).max(0.01);
            t.scale[axis] = (t.scale[axis] * factor).max(0.001);
        }
        Tool::Rotate => {
            let Some(angle) = ring_angle(geo, axis, drag.last_pointer, pointer) else {
                return;
            };
            let world_delta = Quat::from_axis_angle(geo.axes[axis], angle);
            let (_, parent_rotation, _) = geo.parent_world.to_scale_rotation_translation();
            let local = parent_rotation.inverse() * world_delta * parent_rotation * t.rotation_quat();
            let previous = t.rotation;
            t.set_rotation_quat(local);
            // Angles stay continuous (no jump from 180° to −180°), so keys interpolate
            // the way the object turned.
            for i in 0..3 {
                t.rotation[i] = previous[i] + wrap_angle(t.rotation[i] - previous[i]);
            }
        }
        Tool::Move
        | Tool::LiveSelection
        | Tool::RectangleSelection
        | Tool::LoopSelection
        | Tool::RingSelection
        | Tool::Extrude
        | Tool::Inset
        | Tool::LoopCut
        | Tool::LineCut
        | Tool::Bevel
        | Tool::Bridge
        | Tool::Weld => return,
    }
    node.data.set_transform_at(&shown, keying.frame, keying.autokey);
}

/// `angle` brought into `[-π, π]`.
fn wrap_angle(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
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
