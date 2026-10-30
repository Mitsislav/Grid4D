//! The Pen (Create ▸ Spline ▸ Pen; see `docs/reference/object-spline.md`):
//! clicks add points to the spline being drawn on the grid's plane, a drag
//! pulls the new point's tangents, a click on the first point closes it. The
//! points themselves are added by commands (`EditorCommand::PenPoint`, one undo
//! step each); this file turns pointer input into them and draws the preview.

use egui::{Key, Painter, Pos2, Rect, Response, Stroke, Ui, vec2};
use glam::{Mat4, Vec3};
use grid4d_render::GridPlane;
use grid4d_scene::{Document, NodeId};

use super::{ViewportPanel, project, to_ndc};
use crate::{
    EditorCommand, EditorState, Tool,
    theme::{metrics, view_palette as p},
};

/// Screen distance (points) a press must move to pull tangents.
const PULL_THRESHOLD: f32 = 4.0;

/// The Pen's input state.
#[derive(Debug, Default)]
pub(super) struct PenState {
    /// The point being placed while the button is held: world position and
    /// where it was pressed on screen.
    press: Option<(Vec3, Pos2)>,
    /// While pulling: the end of the outgoing tangent (world).
    pull: Option<Vec3>,
    /// Where the cursor meets the plane: the end of the preview line.
    pub(super) hover: Option<Vec3>,
    /// The Pen was the tool last frame (switching away finishes the spline).
    active: bool,
}

/// The spline the Pen draws, as it is shown: its points in world space and
/// whether its last segment is open, with that segment's first point.
struct Drawn {
    points: Vec<Vec3>,
    open_start: Option<usize>,
}

impl ViewportPanel {
    /// Handles the Pen. Returns `true` while it is the tool: it takes every
    /// click in the view.
    pub(super) fn pen_tool(
        &mut self,
        ui: &Ui,
        response: &Response,
        rect: Rect,
        doc: &Document,
        editor: &mut EditorState,
    ) -> bool {
        if editor.tool != Tool::Pen {
            if std::mem::take(&mut self.pen.active) {
                self.pen = PenState::default();
                editor.commands.push(EditorCommand::PenFinish);
            }
            return false;
        }
        self.pen.active = true;
        let (pressed, down, escape, enter, alt, pointer, origin) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Enter),
                i.modifiers.alt,
                i.pointer.interact_pos(),
                i.pointer.press_origin(),
            )
        });
        self.pen.hover = response.hover_pos().and_then(|at| self.pen_plane(rect, at));
        let Some((position, start)) = self.pen.press else {
            if escape || enter {
                editor.commands.push(EditorCommand::PenFinish);
                return true;
            }
            // A press and release in one frame leave no press origin: the
            // pointer is where it was pressed. Clicks never select with the Pen.
            let Some(at) = origin
                .or(pointer)
                .filter(|at| pressed && !alt && rect.contains(*at) && response.hovered())
            else {
                return true;
            };
            if self.pen_closes_at(rect, doc, editor, at) {
                editor.commands.push(EditorCommand::PenClose);
            } else if let Some(position) = self.pen_plane(rect, at) {
                self.pen.press = Some((position, at));
            }
            return true;
        };
        if escape {
            self.pen.press = None;
            self.pen.pull = None;
            return true;
        }
        let pulled = pointer.filter(|at| at.distance(start) > PULL_THRESHOLD);
        self.pen.pull = pulled.and_then(|at| self.pen_plane(rect, at));
        if !down {
            let tangent = self.pen.pull.map_or(Vec3::ZERO, |end| end - position);
            editor.commands.push(EditorCommand::PenPoint { position, tangent });
            self.pen.press = None;
            self.pen.pull = None;
        }
        true
    }

    /// Where the screen position `at` meets the plane the Pen draws on: the
    /// floor in perspective views, the view's own plane through the origin in
    /// parallel ones.
    fn pen_plane(&self, rect: Rect, at: Pos2) -> Option<Vec3> {
        let ray = self.camera.ray_from_ndc(to_ndc(at, rect), rect.aspect_ratio());
        let normal = if self.camera.is_parallel() {
            GridPlane::facing(self.camera.back()).axes().0
        } else {
            Vec3::Y
        };
        let along = ray.dir.dot(normal);
        if along.abs() <= 1e-6 {
            return None;
        }
        let t = -ray.origin.dot(normal) / along;
        (t > 0.0).then(|| ray.origin + ray.dir * t)
    }

    /// The spline being drawn, as shown.
    fn pen_drawn(&self, doc: &Document, editor: &EditorState) -> Option<(NodeId, Drawn)> {
        let id = crate::commands::pen_spline(doc, editor)?;
        let (node, spline, _) = self.evaluated.spline(id)?;
        let spline = spline.spline();
        let world: Mat4 = node.world;
        let points = spline.points.iter().map(|&q| world.transform_point3(q)).collect();
        let open_start = spline
            .segment_ranges()
            .zip(&spline.segments)
            .last()
            .filter(|(_, segment)| !segment.closed)
            .map(|((start, _), _)| start);
        Some((id, Drawn { points, open_start }))
    }

    /// A press at `at` closes the segment being drawn: it is on that segment's
    /// first point, and the segment has at least three points.
    fn pen_closes_at(&self, rect: Rect, doc: &Document, editor: &EditorState, at: Pos2) -> bool {
        let Some((_, drawn)) = self.pen_drawn(doc, editor) else {
            return false;
        };
        let Some(start) = drawn.open_start.filter(|&s| drawn.points.len() >= s + 3) else {
            return false;
        };
        let view_proj = self.camera.view_projection(rect.aspect_ratio());
        drawn
            .points
            .get(start)
            .and_then(|&first| project(&view_proj, rect, first))
            .is_some_and(|first| first.distance(at) <= metrics::COMPONENT_PICK_RADIUS)
    }

    /// The spline being drawn: its points, the line from its last point to the
    /// cursor and, while pulling, the new point's tangents.
    pub(super) fn paint_pen(&self, painter: &Painter, rect: Rect, doc: &Document, editor: &EditorState) {
        if editor.tool != Tool::Pen {
            return;
        }
        let view_proj = self.camera.view_projection(rect.aspect_ratio());
        let screen = |q: Vec3| project(&view_proj, rect, q);
        let square = |at: Pos2, color| painter.rect_filled(Rect::from_center_size(at, vec2(6.0, 6.0)), 0.0, color);
        let drawn = self.pen_drawn(doc, editor).map(|(_, drawn)| drawn);
        if let Some(drawn) = &drawn {
            for &q in &drawn.points {
                if let Some(at) = screen(q) {
                    square(at, p().accent);
                }
            }
        }
        let last = drawn
            .as_ref()
            .filter(|d| d.open_start.is_some())
            .and_then(|d| d.points.last().copied());
        let dashed = Stroke::new(1.0, p().highlight);
        if let Some((position, _)) = self.pen.press {
            if let (Some(from), Some(to)) = (last.and_then(screen), screen(position)) {
                painter.line_segment([from, to], dashed);
            }
            if let Some(end) = self.pen.pull {
                let back = position * 2.0 - end;
                if let (Some(a), Some(b)) = (screen(back), screen(end)) {
                    painter.line_segment([a, b], Stroke::new(1.0, p().text_dim));
                    painter.circle_filled(a, 3.0, p().highlight);
                    painter.circle_filled(b, 3.0, p().highlight);
                }
            }
            if let Some(at) = screen(position) {
                square(at, p().highlight);
            }
        } else if let (Some(from), Some(to)) = (last.and_then(screen), self.pen.hover.and_then(screen)) {
            painter.line_segment([from, to], dashed);
        }
    }
}
