//! The render region in the active view: drawn with Render ▸ Render Region,
//! its edges and corners dragged after, shown as a dashed frame (see
//! `docs/reference/render-queue.md`). The region is a document render
//! setting: a drag writes it live and records one undo step.

use egui::{Color32, CursorIcon, Key, Painter, PointerButton, Pos2, Rect, Response, Stroke, Ui, pos2};
use grid4d_scene::{Document, RenderRegion, RenderSettings, SceneEdit};

use super::ViewportPanel;
use crate::{EditorCommand, EditorState, theme};

/// How near an edge (in points) a press grabs it.
const GRAB: f32 = 6.0;

/// A region being drawn or reshaped.
#[derive(Clone, Debug)]
pub(super) struct RegionDrag {
    /// The settings before the drag (undo restores them, Esc too).
    before: RenderSettings,
    /// Which edges follow the pointer: left, top, right, bottom; all four
    /// while drawing a new one from `anchor`.
    edges: [bool; 4],
    /// Drawing: the corner the drag started at (fractions of the view).
    anchor: Option<[f32; 2]>,
    /// Reshaping: the region's fractions at the press.
    start: [f32; 4],
}

/// `pos` as fractions of `rect` (0…1, kept inside).
fn fraction(rect: Rect, pos: Pos2) -> [f32; 2] {
    [
        ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0),
        ((pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0),
    ]
}

/// The region's rectangle on screen.
pub(super) fn screen_rect(rect: Rect, region: RenderRegion) -> Rect {
    let [l, t, r, b] = region.fractions();
    Rect::from_min_max(
        pos2(rect.left() + l * rect.width(), rect.top() + t * rect.height()),
        pos2(rect.left() + r * rect.width(), rect.top() + b * rect.height()),
    )
}

/// The edges of `frame` near `at`: left, top, right, bottom.
fn edges_near(frame: Rect, at: Pos2) -> [bool; 4] {
    let inside_x = (frame.left() - GRAB..=frame.right() + GRAB).contains(&at.x);
    let inside_y = (frame.top() - GRAB..=frame.bottom() + GRAB).contains(&at.y);
    [
        inside_y && (at.x - frame.left()).abs() <= GRAB,
        inside_x && (at.y - frame.top()).abs() <= GRAB,
        inside_y && (at.x - frame.right()).abs() <= GRAB,
        inside_x && (at.y - frame.bottom()).abs() <= GRAB,
    ]
}

impl ViewportPanel {
    /// Draws or reshapes the render region. Returns `true` while it uses
    /// the pointer (nothing else then reacts to it).
    pub(super) fn render_region_drag(
        &mut self,
        ui: &Ui,
        response: &Response,
        rect: Rect,
        doc: &mut Document,
        editor: &mut EditorState,
    ) -> bool {
        let (down, escape, alt, pointer, press) = ui.input(|i| {
            (
                i.pointer.primary_down(),
                i.key_pressed(Key::Escape),
                i.modifiers.alt,
                i.pointer.interact_pos(),
                i.pointer.press_origin(),
            )
        });
        let hovered = pointer.filter(|p| rect.contains(*p));
        // The cursor says what a press would do.
        if self.region_drag.is_none()
            && let Some(at) = hovered
        {
            if editor.drawing_region {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            } else if let Some(region) = doc.render.region {
                match edges_near(screen_rect(rect, region), at) {
                    [true, true, ..] | [.., true, true] => ui.ctx().set_cursor_icon(CursorIcon::ResizeNwSe),
                    [true, _, _, true] | [_, true, true, _] => ui.ctx().set_cursor_icon(CursorIcon::ResizeNeSw),
                    [true, ..] | [_, _, true, _] => ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal),
                    [_, true, ..] | [.., true] => ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical),
                    _ => {}
                }
            }
        }
        if self.region_drag.is_none() {
            if !response.drag_started_by(PointerButton::Primary) || alt {
                return false;
            }
            let Some(at) = press else {
                return false;
            };
            let before = doc.render.clone();
            if editor.drawing_region {
                self.region_drag = Some(RegionDrag {
                    before,
                    edges: [true; 4],
                    anchor: Some(fraction(rect, at)),
                    start: [0.0; 4],
                });
            } else {
                let Some(region) = doc.render.region else {
                    return false;
                };
                let edges = edges_near(screen_rect(rect, region), at);
                if !edges.contains(&true) {
                    return false;
                }
                self.region_drag = Some(RegionDrag {
                    before,
                    edges,
                    anchor: None,
                    start: region.fractions(),
                });
            }
        }
        let Some(drag) = &self.region_drag else {
            return false;
        };
        if escape {
            doc.render = drag.before.clone();
            self.region_drag = None;
            editor.drawing_region = false;
            return true;
        }
        if let Some(at) = pointer {
            let [x, y] = fraction(rect, at);
            let region = match drag.anchor {
                Some(anchor) => RenderRegion::from_fractions(anchor, [x, y]),
                None => {
                    let [mut l, mut t, mut r, mut b] = drag.start;
                    let [left, top, right, bottom] = drag.edges;
                    if left {
                        l = x;
                    }
                    if top {
                        t = y;
                    }
                    if right {
                        r = x;
                    }
                    if bottom {
                        b = y;
                    }
                    RenderRegion::from_fractions([l, t], [r, b])
                }
            };
            // A drag that has no area yet keeps what was there.
            if let Some(region) = region
                && doc.render.region != Some(region)
            {
                doc.render.region = Some(region);
            }
        }
        if !down && let Some(drag) = self.region_drag.take() {
            editor.drawing_region = false;
            if doc.render != drag.before {
                editor.commands.push(EditorCommand::CommitLive {
                    inverse: SceneEdit::SetRender(drag.before),
                    label: "Render Region",
                });
            }
        }
        true
    }

    /// The render region's dashed frame over the view.
    pub(super) fn paint_render_region(&self, painter: &Painter, rect: Rect, doc: &Document) {
        let Some(region) = doc.render.region else {
            return;
        };
        let frame = screen_rect(rect, region);
        let palette = theme::view_palette();
        let stroke = Stroke::new(1.5, palette.highlight);
        let shadow = Stroke::new(3.0, Color32::from_black_alpha(140));
        let corners = [
            frame.left_top(),
            frame.right_top(),
            frame.right_bottom(),
            frame.left_bottom(),
            frame.left_top(),
        ];
        for pair in corners.windows(2) {
            painter.line_segment([pair[0], pair[1]], shadow);
            painter.extend(egui::Shape::dashed_line(&[pair[0], pair[1]], stroke, 6.0, 4.0));
        }
        for corner in &corners[..4] {
            painter.rect_filled(
                Rect::from_center_size(*corner, egui::vec2(6.0, 6.0)),
                1.0,
                palette.highlight,
            );
        }
    }
}
