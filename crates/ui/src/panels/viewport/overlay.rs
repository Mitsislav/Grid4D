//! 2D overlays drawn on top of the 3D view: background, labels, axis triad, HUD.

use egui::{Align2, FontId, Mesh, Painter, Rect, Shape, Stroke, pos2, vec2};
use glam::{Mat4, Vec3};
use oc4d_render::OrbitCamera;
use oc4d_scene::{Scene, WalkItem};

use super::project;
use crate::{Selection, theme::palette as p};

/// Cinema 4D-style corner brackets on the bounding box of each selected object.
pub(super) fn selection_brackets(
    painter: &Painter,
    rect: Rect,
    view_proj: &Mat4,
    scene: &Scene,
    selection: &Selection,
    stack: &mut Vec<WalkItem>,
) {
    if selection.is_empty() {
        return;
    }
    let stroke = Stroke::new(1.5, p::HIGHLIGHT.gamma_multiply(0.9));
    scene.walk(stack, |id, node, world, _| {
        if !node.visible_in_editor || !node.enabled {
            return false;
        }
        if selection.contains(id)
            && let Some(b) = node.kind.local_bounds()
        {
            let corner = |i: u8| {
                Vec3::new(
                    if i & 1 == 0 { b.min.x } else { b.max.x },
                    if i & 2 == 0 { b.min.y } else { b.max.y },
                    if i & 4 == 0 { b.min.z } else { b.max.z },
                )
            };
            for i in 0..8u8 {
                let c = corner(i);
                let Some(a) = project(view_proj, rect, world.transform_point3(c)) else {
                    continue;
                };
                for bit in [1u8, 2, 4] {
                    let toward = c + (corner(i ^ bit) - c) * 0.2;
                    if let Some(e) = project(view_proj, rect, world.transform_point3(toward)) {
                        painter.line_segment([a, e], stroke);
                    }
                }
            }
        }
        true
    });
}

/// Vertical gradient behind the 3D content.
pub(super) fn background(painter: &Painter, rect: Rect) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), p::VIEWPORT_TOP);
    mesh.colored_vertex(rect.right_top(), p::VIEWPORT_TOP);
    mesh.colored_vertex(rect.right_bottom(), p::VIEWPORT_BOTTOM);
    mesh.colored_vertex(rect.left_bottom(), p::VIEWPORT_BOTTOM);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

/// The view name in the top-left corner ("Perspective").
pub(super) fn view_label(painter: &Painter, rect: Rect, label: &str) {
    painter.text(
        rect.left_top() + vec2(12.0, 10.0),
        Align2::LEFT_TOP,
        label,
        FontId::proportional(12.5),
        p::TEXT.gamma_multiply(0.85),
    );
}

/// World-axis orientation indicator in the bottom-left corner.
pub(super) fn axis_triad(painter: &Painter, rect: Rect, camera: &OrbitCamera) {
    let center = rect.left_bottom() + vec2(44.0, -44.0);
    let view = camera.view();
    let mut axes = [
        (Vec3::X, "X", p::AXIS_X),
        (Vec3::Y, "Y", p::AXIS_Y),
        (Vec3::Z, "Z", p::AXIS_Z),
    ]
    .map(|(axis, label, color)| (view.transform_vector3(axis), label, color));
    // Draw the axis pointing most towards the viewer last.
    axes.sort_by(|a, b| a.0.z.total_cmp(&b.0.z));
    for (v, label, color) in axes {
        let end = center + vec2(v.x, -v.y) * 26.0;
        let faded = if v.z < -0.3 { color.gamma_multiply(0.55) } else { color };
        painter.line_segment([center, end], Stroke::new(2.0, faded));
        painter.text(
            end + vec2(v.x, -v.y) * 9.0,
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(11.5),
            faded,
        );
    }
    painter.circle_filled(center, 2.5, p::TEXT_DIM);
}

/// Bottom-right HUD, like C4D's "Grid Spacing : 100 cm".
pub(super) fn hud(painter: &Painter, rect: Rect, camera: &OrbitCamera) {
    painter.text(
        pos2(rect.right() - 12.0, rect.bottom() - 10.0),
        Align2::RIGHT_BOTTOM,
        format!("Grid Spacing : {} cm", camera.grid_spacing()),
        FontId::proportional(11.5),
        p::TEXT_DIM,
    );
}
