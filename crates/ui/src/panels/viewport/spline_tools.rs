//! Points mode on an editable spline (see `docs/reference/object-spline.md`):
//! its control points and Bezier handles, picked and drawn on screen over the
//! view, Live Selection strokes over the points, and dragging a handle. Clicks
//! and rectangle frames reach the points through the selection tools.

use egui::{Color32, Key, Painter, PointerButton, Pos2, Rect, Response, Stroke, Ui, vec2};
use glam::{Mat4, Vec3};
use grid4d_geom::{BitSet, spline::SplineType};
use grid4d_scene::{Document, NodeData, NodeId, ObjectKind, SceneEdit, SplineHandle};

use super::{ViewportPanel, project, select_tools::Gesture, to_ndc};
use crate::{
    EditMode, EditorCommand, EditorState, SelectMode,
    theme::{metrics, view_palette as p},
};

/// Half the size of a drawn control point, in points.
const POINT_HALF: f32 = 3.0;
/// Radius of a drawn handle end, in points.
const HANDLE_RADIUS: f32 = 3.0;

/// A Bezier handle being dragged.
#[derive(Debug)]
pub(super) struct HandleDrag {
    node: NodeId,
    point: usize,
    /// 0: the incoming (left) tangent, 1: the outgoing (right) one.
    side: usize,
    /// The spline object before the drag: the undo step, and what Esc restores.
    before: NodeData,
    original: SplineHandle,
    world: Mat4,
}

/// A Live Selection stroke over a spline's points: what it has touched so far.
#[derive(Debug)]
pub(super) struct PointStroke {
    node: NodeId,
    mode: SelectMode,
    hits: BitSet,
}

/// One handle end on screen.
struct HandleEnd {
    point: usize,
    side: usize,
    base: Pos2,
    end: Pos2,
}

impl ViewportPanel {
    /// The active object in Points mode, when it is a visible editable spline.
    pub(super) fn spline_target(&self, editor: &EditorState) -> Option<NodeId> {
        if editor.mode != EditMode::Points {
            return None;
        }
        let id = editor.selection.active()?;
        self.evaluated.spline(id)?;
        Some(id)
    }

    /// Screen positions of `node`'s control points (`None`: behind the camera)
    /// and which are hidden.
    fn spline_screen(&self, rect: Rect, node: NodeId) -> Option<(Vec<Option<Pos2>>, BitSet)> {
        let (evaluated, spline, components) = self.evaluated.spline(node)?;
        let view_proj = self.camera.view_projection(rect.aspect_ratio());
        let world = evaluated.world;
        let points = spline
            .spline()
            .points
            .iter()
            .map(|&q| project(&view_proj, rect, world.transform_point3(q)))
            .collect();
        Some((points, components.hidden().points.clone()))
    }

    /// The handle ends of `node`'s selected points, for a Bezier spline.
    fn spline_handles(&self, rect: Rect, node: NodeId) -> Vec<HandleEnd> {
        let Some((evaluated, spline, components)) = self.evaluated.spline(node) else {
            return Vec::new();
        };
        let spline = spline.spline();
        if spline.kind != SplineType::Bezier {
            return Vec::new();
        }
        let view_proj = self.camera.view_projection(rect.aspect_ratio());
        let world = evaluated.world;
        let mut ends = Vec::new();
        for point in components.selected().points.iter() {
            let (Some(&q), Some(tangents)) = (spline.points.get(point), spline.tangents.get(point)) else {
                continue;
            };
            let Some(base) = project(&view_proj, rect, world.transform_point3(q)) else {
                continue;
            };
            for (side, &t) in tangents.iter().enumerate() {
                if t == Vec3::ZERO {
                    continue;
                }
                if let Some(end) = project(&view_proj, rect, world.transform_point3(q + t)) {
                    ends.push(HandleEnd { point, side, base, end });
                }
            }
        }
        ends
    }

    /// The visible control point nearest `pointer`, within the pick radius.
    pub(super) fn spline_point_at(&self, rect: Rect, node: NodeId, pointer: Pos2) -> Option<u32> {
        let (points, hidden) = self.spline_screen(rect, node)?;
        points
            .iter()
            .enumerate()
            .filter(|&(i, _)| !hidden.contains(i))
            .filter_map(|(i, s)| Some((i, s.as_ref()?.distance(pointer))))
            .filter(|&(_, d)| d <= metrics::COMPONENT_PICK_RADIUS)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .and_then(|(i, _)| u32::try_from(i).ok())
    }

    /// The handle end nearest `pointer`, within the pick radius: (point, side).
    fn spline_handle_at(&self, rect: Rect, node: NodeId, pointer: Pos2) -> Option<(usize, usize)> {
        self.spline_handles(rect, node)
            .into_iter()
            .map(|h| (h.point, h.side, h.end.distance(pointer)))
            .filter(|&(.., d)| d <= metrics::COMPONENT_PICK_RADIUS)
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(point, side, _)| (point, side))
    }

    /// The visible control points inside `frame` (screen points).
    pub(super) fn spline_points_in(&self, rect: Rect, node: NodeId, frame: Rect) -> Vec<u32> {
        self.spline_points_where(rect, node, |s| frame.contains(s))
    }

    fn spline_points_where(&self, rect: Rect, node: NodeId, inside: impl Fn(Pos2) -> bool) -> Vec<u32> {
        let Some((points, hidden)) = self.spline_screen(rect, node) else {
            return Vec::new();
        };
        points
            .iter()
            .enumerate()
            .filter(|&(i, s)| !hidden.contains(i) && s.is_some_and(&inside))
            .filter_map(|(i, _)| u32::try_from(i).ok())
            .collect()
    }

    /// A Live Selection stroke over the spline's points: the points under the
    /// brush are gathered while the button is held and selected on release (one
    /// undo step). Plain clicks are left to the click pick.
    pub(super) fn spline_stroke(
        &mut self,
        ui: &Ui,
        response: &Response,
        rect: Rect,
        editor: &mut EditorState,
        node: NodeId,
    ) -> bool {
        let (down, escape, modifiers, pointer) = ui.input(|i| {
            (
                i.pointer.primary_down(),
                i.key_pressed(Key::Escape),
                i.modifiers,
                i.pointer.interact_pos(),
            )
        });
        if self.point_stroke.is_none() {
            if !response.drag_started_by(PointerButton::Primary) || modifiers.alt {
                return false;
            }
            self.point_stroke = Some(PointStroke {
                node,
                mode: SelectMode::for_components(modifiers),
                hits: BitSet::new(),
            });
        }
        if escape {
            self.point_stroke = None;
            self.gesture = Gesture::Cancelled;
            return true;
        }
        if let Some(pointer) = pointer {
            let radius = editor.selection_settings.radius;
            let touched = self.spline_points_where(rect, node, |s| s.distance(pointer) <= radius);
            if let Some(stroke) = &mut self.point_stroke {
                stroke
                    .hits
                    .extend(touched.into_iter().filter_map(|i| usize::try_from(i).ok()));
            }
        }
        if !down && let Some(stroke) = self.point_stroke.take() {
            editor.commands.push(EditorCommand::SelectComponents {
                node: stroke.node,
                component: grid4d_geom::Component::Point,
                hits: stroke.hits.iter().filter_map(|i| u32::try_from(i).ok()).collect(),
                mode: stroke.mode,
            });
        }
        true
    }

    /// Dragging a Bezier handle of a selected point: the tangent follows the
    /// pointer in the view's plane; the opposite one stays in line (keeping its
    /// length) unless Shift is held. One undo step; Esc restores. Returns `true`
    /// while it uses the pointer.
    pub(super) fn handle_drag(
        &mut self,
        ui: &Ui,
        response: &Response,
        rect: Rect,
        doc: &mut Document,
        editor: &mut EditorState,
    ) -> bool {
        let (down, escape, shift, alt, pointer) = ui.input(|i| {
            (
                i.pointer.primary_down(),
                i.key_pressed(Key::Escape),
                i.modifiers.shift,
                i.modifiers.alt,
                i.pointer.interact_pos(),
            )
        });
        if self.handle_dragging.is_none() {
            let Some(node) = self.spline_target(editor) else {
                return false;
            };
            if !response.drag_started_by(PointerButton::Primary) || alt {
                return false;
            }
            let press = ui.input(|i| i.pointer.press_origin());
            let Some((point, side)) = press.and_then(|at| self.spline_handle_at(rect, node, at)) else {
                return false;
            };
            let (Some(data), Some((evaluated, ..))) = (doc.scene.get(node), self.evaluated.spline(node)) else {
                return false;
            };
            let ObjectKind::Spline { spline, .. } = &data.data.kind else {
                return false;
            };
            self.handle_dragging = Some(HandleDrag {
                node,
                point,
                side,
                before: data.data.clone(),
                original: spline.clone(),
                world: evaluated.world,
            });
        }
        let Some(drag) = &self.handle_dragging else {
            return false;
        };
        if escape {
            if let Some(node) = doc.scene.get_mut(drag.node) {
                node.data = drag.before.clone();
            }
            self.handle_dragging = None;
            self.gesture = Gesture::Cancelled;
            return true;
        }
        if let Some(pointer) = pointer
            && let Some(moved) = self.dragged_handle(drag, pointer, rect, shift)
            && let Some(node) = doc.scene.get_mut(drag.node)
            && let ObjectKind::Spline { spline, .. } = &mut node.data.kind
        {
            *spline = moved;
        }
        if !down && let Some(drag) = self.handle_dragging.take() {
            editor.commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetData {
                    id: drag.node,
                    data: drag.before,
                },
                label: "Move Tangent",
            });
        }
        true
    }

    /// The spline with `drag`'s handle at `pointer` (in the plane through the
    /// handle's start, facing the view).
    fn dragged_handle(&self, drag: &HandleDrag, pointer: Pos2, rect: Rect, broken: bool) -> Option<SplineHandle> {
        let spline = drag.original.spline();
        let point = *spline.points.get(drag.point)?;
        let tangents = *spline.tangents.get(drag.point)?;
        let start = drag.world.transform_point3(point + tangents[drag.side]);
        let ray = self.camera.ray_from_ndc(to_ndc(pointer, rect), rect.aspect_ratio());
        let normal = self.camera.forward();
        let along = ray.dir.dot(normal);
        if along.abs() <= 1e-6 || drag.world.determinant().abs() <= f32::EPSILON {
            return None;
        }
        let hit = ray.origin + ray.dir * ((start - ray.origin).dot(normal) / along);
        let tangent = drag.world.inverse().transform_point3(hit) - point;
        let mut all = spline.tangents.clone();
        let pair = all.get_mut(drag.point)?;
        pair[drag.side] = tangent;
        if !broken {
            let other = 1 - drag.side;
            let length = tangents[other].length();
            pair[other] = -tangent.normalize_or_zero() * length;
        }
        Some(drag.original.with_points(&[], &all))
    }

    /// The spline target's control points (selected ones highlighted, the one
    /// under the pointer marked) and the handles of selected Bezier points.
    pub(super) fn paint_spline_points(&self, ui: &Ui, painter: &Painter, rect: Rect, node: NodeId) {
        let Some((points, hidden)) = self.spline_screen(rect, node) else {
            return;
        };
        let selected = self
            .evaluated
            .spline(node)
            .map(|(_, _, c)| c.selected().points.clone())
            .unwrap_or_default();
        for handle in self.spline_handles(rect, node) {
            painter.line_segment([handle.base, handle.end], Stroke::new(1.0, p().text_dim));
            painter.circle_filled(handle.end, HANDLE_RADIUS, p().highlight);
        }
        let hovered = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|_| self.handle_dragging.is_none())
            .and_then(|at| self.spline_point_at(rect, node, at))
            .and_then(|i| usize::try_from(i).ok());
        for (i, screen) in points.iter().enumerate() {
            let Some(at) = screen.filter(|_| !hidden.contains(i)) else {
                continue;
            };
            let fill = if selected.contains(i) {
                p().highlight
            } else {
                p().accent
            };
            let square = Rect::from_center_size(at, vec2(POINT_HALF, POINT_HALF) * 2.0);
            painter.rect_filled(square, 0.0, fill);
            if hovered == Some(i) {
                painter.rect_stroke(
                    square.expand(2.0),
                    0.0,
                    Stroke::new(1.0, Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
            }
        }
    }
}
