//! The joining tools: Bridge (drag from one polygon or border edge to another)
//! and Weld (click a selected point, or elsewhere for the selection's centre).

use egui::{Key, PointerButton, Pos2, Rect, Response, Ui};
use grid4d_geom::Component;
use grid4d_render::{ComponentHit, pick_component};

use super::{ViewportPanel, local, select_tools::Gesture};
use crate::{EditorCommand, EditorState, Tool, theme::metrics};

impl ViewportPanel {
    /// Handles Bridge and Weld. Returns `true` if it used the pointer this frame.
    pub(super) fn join_tools(&mut self, ui: &Ui, response: &Response, rect: Rect, editor: &mut EditorState) -> bool {
        match editor.tool {
            Tool::Bridge => self.bridge(ui, response, rect, editor),
            Tool::Weld => self.weld(ui, response, rect, editor),
            Tool::LiveSelection
            | Tool::RectangleSelection
            | Tool::LoopSelection
            | Tool::RingSelection
            | Tool::Move
            | Tool::Scale
            | Tool::Rotate
            | Tool::Extrude
            | Tool::Inset
            | Tool::LoopCut
            | Tool::LineCut
            | Tool::Bevel => false,
        }
    }

    /// The polygon, or edge in Edges mode, under `pos` on the active object.
    fn component_at(
        &self,
        rect: Rect,
        editor: &EditorState,
        pos: Pos2,
    ) -> Option<(grid4d_scene::NodeId, Component, u32)> {
        let (node, component) = self.component_target(editor)?;
        if !matches!(component, Component::Polygon | Component::Edge) {
            return None;
        }
        let view = self.screen_view(rect);
        let radius = metrics::COMPONENT_PICK_RADIUS;
        match pick_component(&self.evaluated, node, component, &view, local(pos, rect), radius)? {
            ComponentHit::Polygon(i) | ComponentHit::Edge(i) => Some((node, component, i)),
            ComponentHit::Point(_) => None,
        }
    }

    /// Bridge: press on one polygon (or border edge), release on another.
    fn bridge(&mut self, ui: &Ui, response: &Response, rect: Rect, editor: &mut EditorState) -> bool {
        let (escape, alt, pointer, origin, down) = ui.input(|i| {
            (
                i.key_pressed(Key::Escape),
                i.modifiers.alt,
                i.pointer.interact_pos(),
                i.pointer.press_origin(),
                i.pointer.primary_down(),
            )
        });
        if !matches!(self.gesture, Gesture::Bridge { .. }) {
            if !response.drag_started_by(PointerButton::Primary) || alt {
                return false;
            }
            let Some(start) = origin else { return false };
            let Some((node, component, from)) = self.component_at(rect, editor, start) else {
                return false;
            };
            self.gesture = Gesture::Bridge {
                node,
                component,
                from,
                start,
                end: pointer.unwrap_or(start),
            };
        }
        let Gesture::Bridge { end, .. } = &mut self.gesture else {
            return false;
        };
        if escape {
            self.gesture = Gesture::Cancelled;
            return true;
        }
        if let Some(pointer) = pointer {
            *end = pointer;
        }
        if !down
            && let Gesture::Bridge {
                node,
                component,
                from,
                end,
                ..
            } = std::mem::take(&mut self.gesture)
            && let Some((target, kind, to)) = self.component_at(rect, editor, end)
            && target == node
            && kind == component
            && to != from
        {
            editor.commands.push(EditorCommand::Bridge {
                node,
                component,
                from,
                to,
            });
        }
        true
    }

    /// Weld: a click welds the selection onto the clicked selected point, or at
    /// its centre when the click is anywhere else.
    fn weld(&self, ui: &Ui, response: &Response, rect: Rect, editor: &mut EditorState) -> bool {
        let alt = ui.input(|i| i.modifiers.alt);
        if !response.clicked_by(PointerButton::Primary) || alt {
            return false;
        }
        let (Some((node, _)), Some(pos)) = (self.component_target(editor), response.interact_pointer_pos()) else {
            return false;
        };
        let view = self.screen_view(rect);
        let radius = metrics::COMPONENT_PICK_RADIUS;
        let onto = match pick_component(&self.evaluated, node, Component::Point, &view, local(pos, rect), radius) {
            Some(ComponentHit::Point(p)) => Some(p),
            _ => None,
        };
        editor.commands.push(EditorCommand::Weld { node, onto });
        true
    }
}
