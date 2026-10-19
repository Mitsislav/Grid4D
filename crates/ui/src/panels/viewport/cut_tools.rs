//! The cutting tools: Loop/Path Cut (hover an edge for a preview of the loop, click
//! to cut) and Line Cut (drag a line across the object, release to cut).

use egui::{Key, PointerButton, Pos2, Rect, Response, Ui};
use glam::Vec3;
use grid4d_geom::Component;
use grid4d_render::{ComponentHit, pick_component};
use grid4d_scene::{MeshId, NodeId};

use super::{ViewportPanel, local, project, select_tools::Gesture, to_ndc};
use crate::{EditorCommand, EditorState, Tool, modeling, theme::metrics};

/// What a Loop/Path Cut click would cut, rebuilt only when the edge or the
/// fractions change.
#[derive(Debug)]
pub(in crate::panels::viewport) struct LoopPreview {
    node: NodeId,
    mesh: MeshId,
    edge: usize,
    fractions: Vec<f32>,
    /// The new edges, in world space.
    pub(super) segments: Vec<[Vec3; 2]>,
}

impl ViewportPanel {
    /// Handles the cutting tools. Returns `true` if it used the pointer this frame.
    pub(super) fn cut_tools(&mut self, ui: &Ui, response: &Response, rect: Rect, editor: &mut EditorState) -> bool {
        match editor.tool {
            Tool::LoopCut => self.loop_cut(ui, response, rect, editor),
            Tool::LineCut => {
                self.loop_preview = None;
                self.line_cut(ui, response, rect, editor)
            }
            Tool::LiveSelection
            | Tool::RectangleSelection
            | Tool::LoopSelection
            | Tool::RingSelection
            | Tool::Move
            | Tool::Scale
            | Tool::Rotate
            | Tool::Extrude
            | Tool::Inset => {
                self.loop_preview = None;
                false
            }
        }
    }

    /// Loop/Path Cut: the edge under the cursor and where along it set the cut.
    fn loop_cut(&mut self, ui: &Ui, response: &Response, rect: Rect, editor: &mut EditorState) -> bool {
        let alt = ui.input(|i| i.modifiers.alt);
        let click = response.clicked_by(PointerButton::Primary) && !alt;
        // A click cuts where it was pressed, even if the pointer left the view since.
        let pos = if click {
            response.interact_pointer_pos()
        } else {
            response.hover_pos()
        };
        let Some((node, edge, fractions)) = pos.and_then(|pos| self.loop_cut_at(pos, rect, editor)) else {
            self.loop_preview = None;
            return false;
        };
        if click {
            let settings = &mut editor.modeling_settings.loop_cut;
            if settings.cuts <= 1 {
                settings.offset = fractions[0];
            }
            editor.commands.push(EditorCommand::LoopCut {
                node,
                edge: u32::try_from(edge).unwrap_or(u32::MAX),
                fractions,
            });
            self.loop_preview = None;
            return true;
        }
        false
    }

    /// The hovered edge and the cut there, with its preview brought up to date.
    fn loop_cut_at(&mut self, pos: Pos2, rect: Rect, editor: &EditorState) -> Option<(NodeId, usize, Vec<f32>)> {
        let (node, _) = self.component_target(editor)?;
        let view = self.screen_view(rect);
        let radius = metrics::COMPONENT_PICK_RADIUS;
        let Some(ComponentHit::Edge(edge)) =
            pick_component(&self.evaluated, node, Component::Edge, &view, local(pos, rect), radius)
        else {
            return None;
        };
        let edge = usize::try_from(edge).ok()?;
        let (evaluated, mesh, _) = self.evaluated.polygon(node)?;
        let (world, mesh) = (evaluated.world, mesh.clone());
        // Where along the edge (from its first point) the cursor is, on screen.
        let [a, b] = *mesh.topology().edges().get(edge)?;
        let view_proj = self.camera.view_projection(rect.aspect_ratio());
        let end = |p: u32| {
            project(
                &view_proj,
                rect,
                world.transform_point3(mesh.mesh().points()[usize::try_from(p).ok()?]),
            )
        };
        let (pa, pb) = (end(a)?, end(b)?);
        let along = pb - pa;
        let t = ((pos - pa).dot(along) / along.length_sq().max(f32::EPSILON)).clamp(0.0, 1.0);
        let fractions = editor.modeling_settings.loop_cut.fractions(t);

        let stale = self
            .loop_preview
            .as_ref()
            .is_none_or(|p| p.node != node || p.mesh != mesh.id() || p.edge != edge || p.fractions != fractions);
        if stale {
            let segments = modeling::loop_cut(&mesh, edge, &fractions)
                .map(|cut| {
                    let at = |p: u32| world.transform_point3(cut.mesh.points()[usize::try_from(p).unwrap_or(0)]);
                    cut.new_edges.iter().map(|&[p, q]| [at(p), at(q)]).collect()
                })
                .unwrap_or_default();
            self.loop_preview = Some(LoopPreview {
                node,
                mesh: mesh.id(),
                edge,
                fractions: fractions.clone(),
                segments,
            });
        }
        Some((node, edge, fractions))
    }

    /// Line Cut: press, drag, release cuts along the line; Esc cancels.
    fn line_cut(&mut self, ui: &Ui, response: &Response, rect: Rect, editor: &mut EditorState) -> bool {
        let (escape, alt, pointer, origin, down) = ui.input(|i| {
            (
                i.key_pressed(Key::Escape),
                i.modifiers.alt,
                i.pointer.interact_pos(),
                i.pointer.press_origin(),
                i.pointer.primary_down(),
            )
        });
        if !matches!(self.gesture, Gesture::LineCut { .. }) {
            if !response.drag_started_by(PointerButton::Primary) || alt {
                return false;
            }
            let (Some((node, _)), Some(start)) = (self.component_target(editor), origin) else {
                return false;
            };
            self.gesture = Gesture::LineCut {
                node,
                start,
                end: pointer.unwrap_or(start),
            };
        }
        let Gesture::LineCut { node, start, end } = &mut self.gesture else {
            return false;
        };
        if escape {
            self.gesture = Gesture::Cancelled;
            return true;
        }
        if let Some(pointer) = pointer {
            *end = pointer;
        }
        if !down {
            let (node, start, end) = (*node, *start, *end);
            self.gesture = Gesture::None;
            if start.distance(end) >= 3.0 {
                let aspect = rect.aspect_ratio();
                let ray = |p: Pos2| self.camera.ray_from_ndc(to_ndc(p, rect), aspect);
                let (from, to) = (ray(start), ray(end));
                editor.commands.push(EditorCommand::LineCut {
                    node,
                    eye: from.origin,
                    from: from.dir,
                    to: to.dir,
                });
            }
        }
        true
    }
}
