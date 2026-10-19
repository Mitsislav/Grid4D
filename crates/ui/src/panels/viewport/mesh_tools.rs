//! Dragging with a modelling tool (Extrude, Inset): left/right sets the Offset,
//! measured at the object's depth so the result follows the cursor. The tool
//! re-runs on the mesh as it was at the press every frame; one undo step on
//! release, Esc restores the mesh.

use egui::{Key, PointerButton, Pos2, Rect, Response, Ui};
use glam::Vec3;
use grid4d_geom::Component;
use grid4d_scene::{Document, NodeData, NodeId, SceneEdit};

use super::{ViewportPanel, select_tools::Gesture};
use crate::{EditorCommand, EditorState, modeling};

/// A modelling tool's drag in progress.
#[derive(Debug)]
pub(in crate::panels::viewport) struct MeshDrag {
    node: NodeId,
    component: Component,
    /// The object as it was at the press: every frame starts from it.
    before: NodeData,
    start: Pos2,
    /// World units per point of horizontal pointer travel.
    per_point: f32,
    offset: f32,
}

impl ViewportPanel {
    /// Handles a modelling tool's drag. Returns `true` while it uses the pointer
    /// (a press without drag is left to selection, so a click still selects).
    pub(super) fn mesh_tools(
        &mut self,
        ui: &Ui,
        response: &Response,
        rect: Rect,
        doc: &mut Document,
        editor: &mut EditorState,
    ) -> bool {
        let tool = editor.tool;
        if !tool.is_mesh_tool() {
            return false;
        }
        let (escape, alt, pointer, origin, down) = ui.input(|i| {
            (
                i.key_pressed(Key::Escape),
                i.modifiers.alt,
                i.pointer.interact_pos(),
                i.pointer.press_origin(),
                i.pointer.primary_down(),
            )
        });
        if !matches!(self.gesture, Gesture::MeshTool(_)) {
            if !response.drag_started_by(PointerButton::Primary) || alt {
                return false;
            }
            let Some(drag) = self.start_drag(doc, editor, rect, origin) else {
                return false;
            };
            self.gesture = Gesture::MeshTool(drag);
        }
        let Gesture::MeshTool(drag) = &mut self.gesture else {
            return false;
        };
        if escape {
            if let Some(node) = doc.scene.get_mut(drag.node) {
                node.data = drag.before.clone();
            }
            self.gesture = Gesture::Cancelled;
            return true;
        }
        if let Some(pointer) = pointer {
            let offset = (pointer.x - drag.start.x) * drag.per_point;
            if offset != drag.offset {
                drag.offset = offset;
                let data = modeling::apply(&drag.before, tool, drag.component, &editor.modeling_settings, offset)
                    .unwrap_or_else(|| drag.before.clone());
                if let Some(node) = doc.scene.get_mut(drag.node) {
                    node.data = data;
                }
            }
        }
        if !down && let Gesture::MeshTool(drag) = std::mem::take(&mut self.gesture) {
            if let Some(offset) = modeling::offset_mut(tool, &mut editor.modeling_settings)
                && drag.offset != 0.0
            {
                *offset = drag.offset;
            }
            // Records nothing if the drag ended where it started.
            editor.commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetData {
                    id: drag.node,
                    data: drag.before,
                },
                label: tool.label(),
            });
        }
        true
    }

    /// A drag on the active polygon object, if the tool works in this mode.
    fn start_drag(&self, doc: &Document, editor: &EditorState, rect: Rect, origin: Option<Pos2>) -> Option<MeshDrag> {
        let (node, component) = self.component_target(editor)?;
        if !modeling::works_in(editor.tool, component) {
            return None;
        }
        let (evaluated, mesh, _) = self.evaluated.polygon(node)?;
        let centre = evaluated
            .world
            .transform_point3(mesh.bounds().map_or(Vec3::ZERO, |b| b.center()));
        let depth = (centre - self.camera.eye())
            .dot(self.camera.forward())
            .max(self.camera.near);
        Some(MeshDrag {
            node,
            component,
            before: doc.scene.get(node)?.data.clone(),
            start: origin?,
            per_point: self.camera.world_units_per_pixel(depth, rect.height()),
            offset: 0.0,
        })
    }
}
