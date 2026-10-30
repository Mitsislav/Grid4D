//! A deformer's strength handle (see `docs/reference/deformer-twist-taper-bulge.md`):
//! with a deformer selected in Model mode, an orange handle sits on its box;
//! dragging it sets the strength (Shift steps it), one undo step per drag, Esc
//! restores. The handle's place and the value it gives come from
//! `grid4d_geom::deform`, or from `grid4d-plugin` for a deformer plugin that
//! names a handle parameter.

use egui::{Key, Painter, PointerButton, Pos2, Rect, Response, Ui};
use glam::{Mat4, Vec3};
use grid4d_scene::{Document, NodeData, NodeId, ObjectKind, SceneEdit};

use super::{ViewportPanel, project, to_ndc};
use crate::{EditMode, EditorCommand, EditorState, keys::Keying, theme::view_palette as p};

/// The handle is grabbed within this many points.
const GRAB: f32 = 8.0;
const RADIUS: f32 = 5.0;

/// A handle drag in progress.
#[derive(Debug)]
pub(super) struct DeformerDrag {
    node: NodeId,
    before: NodeData,
    /// The deformer as it was shown when the drag started.
    original: ObjectKind,
    world: Mat4,
}

/// The handle of a deformer (in its space) and the plane it is dragged in
/// (a point and a normal); `None` for other objects.
fn handle_of(kind: &ObjectKind) -> Option<(Vec3, (Vec3, Vec3))> {
    match kind {
        ObjectKind::Bend(d) => Some((d.handle(), d.handle_plane())),
        ObjectKind::Twist(d) => Some((d.handle(), d.handle_plane())),
        ObjectKind::Taper(d) => Some((d.handle(), d.handle_plane())),
        ObjectKind::Bulge(d) => Some((d.handle(), d.handle_plane())),
        // Moved up and down in the deformer's XY plane.
        ObjectKind::Plugin(object) => {
            let handle = grid4d_plugin::registry().deformer(&object.type_id)?.handle(object)?;
            Some((handle, (handle, Vec3::Z)))
        }
        _ => None,
    }
}

/// `kind` with the strength that puts its handle at `target` (deformer space);
/// `step` rounds it (5° or 5 %).
fn with_handle(kind: &ObjectKind, target: Vec3, step: bool) -> ObjectKind {
    let degrees = |v: f32| {
        if step {
            (v.to_degrees() / 5.0).round() * 5.0_f32.to_radians()
        } else {
            v
        }
    };
    let percent = |v: f32| if step { (v * 20.0).round() / 20.0 } else { v };
    match kind {
        ObjectKind::Bend(d) => {
            let mut moved = d.with_handle(target);
            moved.strength = degrees(moved.strength);
            ObjectKind::Bend(moved)
        }
        ObjectKind::Twist(d) => {
            let mut moved = d.with_handle(target);
            moved.angle = degrees(moved.angle);
            ObjectKind::Twist(moved)
        }
        ObjectKind::Taper(d) => {
            let mut moved = d.with_handle(target);
            moved.strength = percent(moved.strength);
            ObjectKind::Taper(moved)
        }
        ObjectKind::Bulge(d) => {
            let mut moved = d.with_handle(target);
            moved.strength = percent(moved.strength);
            ObjectKind::Bulge(moved)
        }
        ObjectKind::Plugin(object) => match grid4d_plugin::registry().deformer(&object.type_id) {
            Some(plugin) => ObjectKind::Plugin(plugin.with_handle(object, target, step)),
            None => kind.clone(),
        },
        other => other.clone(),
    }
}

impl ViewportPanel {
    /// The selected deformer whose handle is shown: the active object in
    /// Model mode, when it is a visible deformer. Its id, world matrix and
    /// parameters at `frame`.
    fn deformer_target(&self, doc: &Document, editor: &EditorState, frame: f64) -> Option<(NodeId, Mat4, ObjectKind)> {
        if editor.mode != EditMode::Model {
            return None;
        }
        let id = editor.selection.active()?;
        let node = self.evaluated.nodes().iter().find(|n| n.id == id)?;
        let kind = doc.scene.get(id)?.data.kind_at(frame).into_owned();
        grid4d_eval::is_deformer(&kind).then_some((id, node.world, kind))
    }

    /// Where the target's handle shows on screen.
    fn deformer_handle_screen(&self, rect: Rect, world: Mat4, kind: &ObjectKind) -> Option<Pos2> {
        let (handle, _) = handle_of(kind)?;
        let view_proj = self.camera.view_projection(rect.aspect_ratio());
        project(&view_proj, rect, world.transform_point3(handle))
    }

    /// Drags a deformer's handle. Returns `true` while it uses the pointer.
    pub(super) fn deformer_handle_drag(
        &mut self,
        ui: &Ui,
        response: &Response,
        rect: Rect,
        doc: &mut Document,
        editor: &mut EditorState,
    ) -> bool {
        let keying = Keying::of(doc, editor);
        let (down, escape, shift, alt, pointer) = ui.input(|i| {
            (
                i.pointer.primary_down(),
                i.key_pressed(Key::Escape),
                i.modifiers.shift,
                i.modifiers.alt,
                i.pointer.interact_pos(),
            )
        });
        if self.deformer_drag.is_none() {
            if !response.drag_started_by(PointerButton::Primary) || alt {
                return false;
            }
            let Some((node, world, kind)) = self.deformer_target(doc, editor, keying.frame) else {
                return false;
            };
            let press = ui.input(|i| i.pointer.press_origin());
            let grabbed = press
                .zip(self.deformer_handle_screen(rect, world, &kind))
                .is_some_and(|(at, handle)| at.distance(handle) <= GRAB);
            let Some(before) = doc.scene.get(node).map(|n| n.data.clone()).filter(|_| grabbed) else {
                return false;
            };
            self.deformer_drag = Some(DeformerDrag {
                node,
                before,
                original: kind,
                world,
            });
        }
        let Some(drag) = &self.deformer_drag else {
            return false;
        };
        if escape {
            if let Some(node) = doc.scene.get_mut(drag.node) {
                node.data = drag.before.clone();
            }
            self.deformer_drag = None;
            return true;
        }
        if let Some(target) = pointer.and_then(|at| self.handle_target(drag, at, rect)) {
            let moved = with_handle(&drag.original, target, shift);
            let mut data = drag.before.clone();
            data.set_kind_at(&moved, keying.frame, keying.autokey);
            if let Some(node) = doc.scene.get_mut(drag.node)
                && node.data != data
            {
                node.data = data;
            }
        }
        if !down && let Some(drag) = self.deformer_drag.take() {
            editor.commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetData {
                    id: drag.node,
                    data: drag.before,
                },
                label: "Move Handle",
            });
        }
        true
    }

    /// Where the pointer meets the handle's plane, in the deformer's space.
    fn handle_target(&self, drag: &DeformerDrag, pointer: Pos2, rect: Rect) -> Option<Vec3> {
        let (_, (point, normal)) = handle_of(&drag.original)?;
        if drag.world.determinant().abs() <= f32::EPSILON {
            return None;
        }
        let point = drag.world.transform_point3(point);
        let normal = drag.world.transform_vector3(normal).normalize_or_zero();
        let ray = self.camera.ray_from_ndc(to_ndc(pointer, rect), rect.aspect_ratio());
        let along = ray.dir.dot(normal);
        if along.abs() <= 1e-6 {
            return None;
        }
        let hit = ray.origin + ray.dir * ((point - ray.origin).dot(normal) / along);
        Some(drag.world.inverse().transform_point3(hit))
    }

    /// The selected deformer's handle.
    pub(super) fn paint_deformer_handle(&self, painter: &Painter, rect: Rect, doc: &Document, editor: &EditorState) {
        let frame = f64::from(doc.time.frame());
        let Some((_, world, kind)) = self.deformer_target(doc, editor, frame) else {
            return;
        };
        if let Some(at) = self.deformer_handle_screen(rect, world, &kind) {
            painter.circle_filled(at, RADIUS, p().highlight);
            painter.circle_stroke(at, RADIUS + 1.0, egui::Stroke::new(1.0, p().bg_well));
        }
    }
}

#[cfg(test)]
mod tests {
    use grid4d_geom::deform::Taper;

    use super::*;

    #[test]
    fn shift_steps_the_values() {
        let taper = ObjectKind::Taper(Taper {
            size: Vec3::splat(100.0),
            ..Taper::default()
        });
        let ObjectKind::Taper(moved) = with_handle(&taper, Vec3::new(31.0, 50.0, 0.0), true) else {
            unreachable!()
        };
        assert!(
            (moved.strength - 0.4).abs() < 1e-6,
            "38 % stepped to 40 %: {}",
            moved.strength
        );
        assert!(handle_of(&ObjectKind::Null).is_none());
    }

    #[test]
    fn a_deformer_plugins_handle_sets_its_parameter() {
        use grid4d_scene::PluginValue;
        crate::test_plugins();
        let wave = grid4d_plugin::registry().deformer("grid4d.wave").unwrap();
        let kind = ObjectKind::Plugin(grid4d_plugin::Registry::new_object(wave));
        // Box 200 high, Amplitude 20: the handle 20 above the top.
        let (handle, (_, normal)) = handle_of(&kind).unwrap();
        assert_eq!((handle, normal), (Vec3::new(0.0, 120.0, 0.0), Vec3::Z));
        let ObjectKind::Plugin(moved) = with_handle(&kind, Vec3::new(0.0, 147.2, 0.0), true) else {
            unreachable!()
        };
        assert_eq!(
            moved.get("Amplitude"),
            Some(PluginValue::Float(47.0)),
            "whole centimetres with Shift"
        );
    }
}
