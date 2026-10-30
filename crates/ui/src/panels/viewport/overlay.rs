//! 2D overlays drawn on top of the 3D view: background, labels, axis triad, HUD.

use egui::{Align2, CornerRadius, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use glam::{Mat4, Vec3};
use grid4d_eval::EvaluatedScene;
use grid4d_render::{OrbitCamera, ViewFilter};
use grid4d_scene::LightType;

use super::project;
use crate::{Selection, theme::palette as p};

/// Cinema 4D-style corner brackets on the bounding box of each selected object
/// whose type the view shows.
pub(super) fn selection_brackets(
    painter: &Painter,
    rect: Rect,
    view_proj: &Mat4,
    scene: &EvaluatedScene,
    selection: &Selection,
    shown: ViewFilter,
) {
    if selection.is_empty() {
        return;
    }
    let stroke = Stroke::new(1.5, p::HIGHLIGHT.gamma_multiply(0.9));
    for node in scene.nodes() {
        let world = node.world;
        if selection.contains(node.id)
            && shown.shows_object(node.kind)
            && let Some(b) = node.bounds
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
    }
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

/// The Render View's status, in the view's bottom-left corner on a dark band.
pub(super) fn render_view_status(painter: &Painter, rect: Rect, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(12.0), p::TEXT_STRONG);
    let pos = rect.left_bottom() + vec2(10.0, -10.0 - galley.size().y);
    let band = Rect::from_min_size(pos, galley.size()).expand2(vec2(6.0, 3.0));
    painter.rect_filled(band, 3.0, egui::Color32::from_black_alpha(170));
    painter.galley(pos, galley, p::TEXT_STRONG);
}

/// Every visible camera object except the one the view looks through (`linked`),
/// as a wire pyramid of its field of view (at the view's aspect ratio) with a
/// small triangle marking its up side.
pub(super) fn camera_frustums(
    painter: &Painter,
    rect: Rect,
    view_proj: &Mat4,
    evaluated: &EvaluatedScene,
    scene: &grid4d_scene::Scene,
    selection: &Selection,
    linked: Option<grid4d_scene::NodeId>,
) {
    /// How far the pyramid reaches, in cm.
    const DEPTH: f32 = 160.0;
    let aspect = rect.aspect_ratio().max(0.1);
    for node in evaluated.nodes() {
        if Some(node.id) == linked {
            continue;
        }
        let Some(grid4d_scene::ObjectKind::Camera { fov_y_deg }) = scene.get(node.id).map(|n| &n.data.kind) else {
            continue;
        };
        let (_, rotation, apex) = node.world.to_scale_rotation_translation();
        let (right, up, forward) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::NEG_Z);
        let half_h = (fov_y_deg.to_radians() * 0.5).tan() * DEPTH;
        let half_w = half_h * aspect;
        let centre = apex + forward * DEPTH;
        let corners = [
            centre + right * half_w + up * half_h,
            centre - right * half_w + up * half_h,
            centre - right * half_w - up * half_h,
            centre + right * half_w - up * half_h,
        ];
        let colour = if selection.contains(node.id) {
            p::HIGHLIGHT
        } else {
            p::ICON_CAMERA.gamma_multiply(0.8)
        };
        let stroke = Stroke::new(1.2, colour);
        let line = |a: Vec3, b: Vec3| {
            if let (Some(a), Some(b)) = (project(view_proj, rect, a), project(view_proj, rect, b)) {
                painter.line_segment([a, b], stroke);
            }
        };
        for k in 0..4 {
            line(apex, corners[k]);
            line(corners[k], corners[(k + 1) % 4]);
        }
        // The up marker: a triangle above the top edge.
        let top = centre + up * half_h;
        let peak = top + up * half_h * 0.35;
        line(top + right * half_w * 0.3, peak);
        line(peak, top - right * half_w * 0.3);
    }
}

/// Where the visible lights shine: a Spot's cone (its outer angle) and an
/// Infinite light's direction, in the light colour (orange when selected). Omni
/// lights show only their marker.
pub(super) fn light_shapes(
    painter: &Painter,
    rect: Rect,
    view_proj: &Mat4,
    evaluated: &EvaluatedScene,
    selection: &Selection,
) {
    /// How far the cone and the arrow reach, in cm.
    const LENGTH: f32 = 160.0;
    /// Segments of the cone's rim.
    const RIM: u8 = 24;
    for light in evaluated.lights() {
        let colour = if selection.contains(light.id) {
            p::HIGHLIGHT
        } else {
            p::ICON_LIGHT.gamma_multiply(0.8)
        };
        let stroke = Stroke::new(1.2, colour);
        let line = |a: Vec3, b: Vec3| {
            if let (Some(a), Some(b)) = (project(view_proj, rect, a), project(view_proj, rect, b)) {
                painter.line_segment([a, b], stroke);
            }
        };
        let forward = light.direction;
        let side = forward.any_orthonormal_vector();
        let other = forward.cross(side);
        match light.light_type {
            LightType::Omni => {}
            LightType::Spot => {
                let apex = light.position;
                let half = (light.details.outer_angle_deg.to_radians() * 0.5).min(1.5);
                let radius = half.tan() * LENGTH;
                let centre = apex + forward * LENGTH;
                let rim = |k: u8| {
                    let a = f32::from(k) / f32::from(RIM) * std::f32::consts::TAU;
                    centre + (side * a.cos() + other * a.sin()) * radius
                };
                for k in 0..RIM {
                    line(rim(k), rim(k + 1));
                }
                for k in [0, RIM / 4, RIM / 2, RIM * 3 / 4] {
                    line(apex, rim(k));
                }
            }
            LightType::Infinite => {
                let start = light.position;
                let tip = start + forward * LENGTH;
                line(start, tip);
                for along in [side, other] {
                    line(tip, tip - forward * 24.0 + along * 10.0);
                    line(tip, tip - forward * 24.0 - along * 10.0);
                }
            }
        }
    }
}

/// The red frame around the view while Autokeying is on.
pub(super) fn autokey_frame(painter: &Painter, rect: Rect) {
    painter.rect_stroke(
        rect.shrink(1.5),
        CornerRadius::ZERO,
        Stroke::new(3.0, p::RECORD),
        StrokeKind::Inside,
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
        format!(
            "Grid Spacing : {}",
            crate::preferences::units(painter.ctx()).format(f64::from(camera.grid_spacing()), 2, "")
        ),
        FontId::proportional(11.5),
        p::TEXT_DIM,
    );
}

/// The Live Selection brush around the pointer.
pub(super) fn brush(painter: &Painter, center: Pos2, radius: f32) {
    painter.circle_stroke(center, radius, Stroke::new(1.0, p::TEXT_STRONG.gamma_multiply(0.8)));
}

/// The Rectangle Selection frame being dragged.
/// Line Cut: the line being drawn, with its ends.
pub(super) fn cut_line(painter: &Painter, start: Pos2, end: Pos2) {
    painter.line_segment([start, end], Stroke::new(1.5, p::ACCENT));
    for end in [start, end] {
        painter.circle_filled(end, 3.0, p::ACCENT);
    }
}

/// Loop/Path Cut: the edges a click would add (world space).
pub(super) fn cut_preview(painter: &Painter, rect: Rect, view_proj: &Mat4, segments: &[[Vec3; 2]]) {
    for &[a, b] in segments {
        if let (Some(a), Some(b)) = (project(view_proj, rect, a), project(view_proj, rect, b)) {
            painter.line_segment([a, b], Stroke::new(1.5, p::ACCENT));
        }
    }
}

pub(super) fn marquee(painter: &Painter, frame: Rect) {
    painter.rect_filled(frame, CornerRadius::ZERO, p::ACCENT.gamma_multiply(0.12));
    painter.rect_stroke(
        frame,
        CornerRadius::ZERO,
        Stroke::new(1.0, p::ACCENT),
        StrokeKind::Inside,
    );
}

/// The snap target a drag is stuck to: a marker on the point (and the edge or
/// grid line drawn over), with the target's name.
pub(super) fn snap_target(painter: &Painter, rect: Rect, view_proj: &Mat4, hit: &grid4d_render::SnapHit) {
    let stroke = Stroke::new(2.0, p::SNAP);
    if let Some([a, b]) = hit.segment
        && let (Some(a), Some(b)) = (project(view_proj, rect, a), project(view_proj, rect, b))
    {
        painter.line_segment([a, b], stroke);
    }
    let Some(at) = project(view_proj, rect, hit.position) else {
        return;
    };
    painter.rect_stroke(
        Rect::from_center_size(at, vec2(10.0, 10.0)),
        CornerRadius::ZERO,
        stroke,
        StrokeKind::Middle,
    );
    painter.circle_filled(at, 2.0, p::SNAP);
    let galley = painter.layout_no_wrap(hit.kind.label().to_owned(), FontId::proportional(11.5), p::SNAP);
    let corner = at + vec2(11.0, -11.0 - galley.size().y);
    painter.rect_filled(
        Rect::from_min_size(corner, galley.size()).expand(3.0),
        CornerRadius::same(3),
        egui::Color32::from_black_alpha(180),
    );
    painter.galley(corner, galley, p::SNAP);
}
