//! Dragging with a modelling tool (Extrude, Inset, Bevel, Slide): left/right
//! sets the Offset, measured at the object's depth so the result follows the
//! cursor (Slide's as a share of the edges it slides along, towards the
//! side the drag goes). The tool re-runs on the mesh as it was at the press
//! every frame; one undo step on release, Esc restores the mesh.

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
    /// Offset per point of horizontal pointer travel (world units, or a
    /// share of the rails for Slide).
    per_point: f32,
    offset: f32,
    /// Where the offset starts (Plane Cut: its plane's place; others 0).
    base: f32,
    /// The object's placement, for planes in world space.
    context: modeling::Context,
    /// Slide: the view's right in object space (where single points go).
    direction: Vec3,
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
            if tool == crate::Tool::Slide {
                editor.modeling_settings.slide_direction = drag.direction;
            }
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
            let offset = drag.base + (pointer.x - drag.start.x) * drag.per_point;
            if offset != drag.offset {
                drag.offset = offset;
                let settings = &editor.modeling_settings;
                let data = modeling::apply(&drag.before, tool, drag.component, settings, offset, &drag.context)
                    .unwrap_or_else(|| drag.before.clone());
                if let Some(node) = doc.scene.get_mut(drag.node) {
                    node.data = data;
                }
            }
        }
        if !down && let Gesture::MeshTool(drag) = std::mem::take(&mut self.gesture) {
            if let Some(offset) = modeling::offset_mut(tool, &mut editor.modeling_settings)
                && drag.offset != drag.base
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
        // Tools without an Offset run only from Apply.
        let mut settings = editor.modeling_settings;
        let base = *modeling::offset_mut(editor.tool, &mut settings)?;
        let base = if editor.tool == crate::Tool::PlaneCut {
            base
        } else {
            0.0
        };
        let (evaluated, mesh, components) = self.evaluated.polygon(node)?;
        let world = evaluated.world;
        let centre = world.transform_point3(mesh.bounds().map_or(Vec3::ZERO, |b| b.center()));
        let depth = (centre - self.camera.eye())
            .dot(self.camera.forward())
            .max(self.camera.near);
        let mut per_point = self.camera.world_units_per_pixel(depth, rect.height());
        let direction = world
            .inverse()
            .transform_vector3(self.camera.right())
            .normalize_or(Vec3::X);
        if editor.tool == crate::Tool::Crease {
            // A crease from 0 to 100 % over 200 points of travel.
            per_point = 1.0 / 200.0;
        }
        if editor.tool == crate::Tool::Slide {
            // A share of the rails' mean length, towards the side the drag goes.
            let (source, topology) = (mesh.mesh(), mesh.topology());
            let rails = modeling::slide_rails(source, topology, components.state(), component, direction)?;
            let at = |p: u32| world.transform_point3(source.points()[usize::try_from(p).unwrap_or(0)]);
            let lengths: Vec<f32> = rails
                .iter()
                .filter_map(|r| r.ahead.or(r.behind).map(|q| at(q).distance(at(r.point))))
                .collect();
            let mean = lengths.iter().sum::<f32>() / lengths.len().max(1) as f32;
            if mean <= f32::EPSILON {
                return None;
            }
            per_point /= mean;
            let ahead = rails
                .iter()
                .find_map(|r| r.ahead.map(|q| at(q) - at(r.point)))
                .unwrap_or(Vec3::ZERO);
            if component == Component::Edge && ahead.dot(self.camera.right()) < 0.0 {
                per_point = -per_point;
            }
        }
        Some(MeshDrag {
            node,
            component,
            before: doc.scene.get(node)?.data.clone(),
            start: origin?,
            per_point,
            offset: base,
            base,
            context: modeling::Context { world, path: None },
            direction,
        })
    }
}
