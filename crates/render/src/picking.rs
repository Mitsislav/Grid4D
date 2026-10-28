use glam::{Vec2, Vec3};
use grid4d_eval::{EvaluatedScene, PickShape};
use grid4d_scene::NodeId;

use crate::{Ray, ScreenView, ViewFilter};

/// Returns the closest visible object hit by `ray`, among the types `shown`.
///
/// Tests in each object's local space, using its [`PickShape`]: exact for
/// spheres and polygon objects (a BVH per mesh, built on its first pick),
/// oriented bounding boxes for everything else (Nulls are skipped).
/// Allocation-free once each mesh's BVH exists.
#[must_use]
pub fn pick(scene: &EvaluatedScene, ray: Ray, shown: ViewFilter) -> Option<NodeId> {
    let mut best: Option<(f32, NodeId)> = None;
    for node in scene.nodes() {
        if node.pick == PickShape::None
            || !shown.shows_object(node.kind)
            || node.world.determinant().abs() <= f32::EPSILON
        {
            continue;
        }
        let inv = node.world.inverse();
        let origin = inv.transform_point3(ray.origin);
        let dir = inv.transform_vector3(ray.dir);
        let hit = match node.pick {
            // Curves are picked on screen (`pick_curve`).
            PickShape::None | PickShape::Curve(_) => None,
            PickShape::Sphere { radius } => ray_sphere(origin, dir, radius),
            PickShape::Bounds => node.bounds.and_then(|b| b.ray_intersect(origin, dir)),
            // The box first: most rays miss most objects.
            PickShape::Mesh(mesh) => node
                .bounds
                .and_then(|b| b.ray_intersect(origin, dir))
                .and(scene.mesh(mesh))
                .and_then(|mesh| mesh.raycast(origin, dir))
                .map(|hit| hit.t),
        };
        // Affine maps preserve the ray parameter, so `t` is comparable across objects.
        if let Some(t) = hit
            && best.is_none_or(|(best_t, _)| t < best_t)
        {
            best = Some((t, node.id));
        }
    }
    best.map(|(_, id)| id)
}

/// The spline whose curve passes nearest `pointer` (in `view`'s screen
/// coordinates), if within `radius`; among the types `shown`. Splines are thin,
/// so they are picked by their distance on screen, not by a ray.
/// Allocation-free.
#[must_use]
pub fn pick_curve(
    scene: &EvaluatedScene,
    view: &ScreenView,
    pointer: Vec2,
    radius: f32,
    shown: ViewFilter,
) -> Option<NodeId> {
    let mut best: Option<(f32, NodeId)> = None;
    for node in scene.nodes() {
        let PickShape::Curve(id) = node.pick else { continue };
        if !shown.shows_object(node.kind) {
            continue;
        }
        let Some(curve) = scene.curve(id) else { continue };
        let screen = |p: Vec3| {
            let clip = view.view_proj * node.world.transform_point3(p).extend(1.0);
            (clip.w > 1e-6 && clip.z >= 0.0).then(|| view.to_screen(clip))
        };
        let mut consider = |distance: f32| {
            if distance <= radius && best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, node.id));
            }
        };
        for (run, closed) in curve.polyline().runs() {
            if let [single] = run {
                if let Some(p) = screen(*single) {
                    consider(p.distance(pointer));
                }
                continue;
            }
            let pairs = if closed && run.len() > 2 {
                run.len()
            } else {
                run.len().saturating_sub(1)
            };
            for i in 0..pairs {
                let (Some(a), Some(b)) = (screen(run[i]), screen(run[(i + 1) % run.len()])) else {
                    continue;
                };
                consider(distance_to_segment(pointer, a, b));
            }
        }
    }
    best.map(|(_, id)| id)
}

/// Distance from `p` to the segment `a`–`b`.
fn distance_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

fn ray_sphere(origin: Vec3, dir: Vec3, radius: f32) -> Option<f32> {
    let a = dir.dot(dir);
    let b = origin.dot(dir);
    let c = origin.dot(origin) - radius * radius;
    let disc = b * b - a * c;
    if disc < 0.0 || a <= f32::EPSILON {
        return None;
    }
    let sq = disc.sqrt();
    let far = (-b + sq) / a;
    (far >= 0.0).then(|| ((-b - sq) / a).max(0.0))
}

#[cfg(test)]
mod tests {
    use glam::Vec2;
    use grid4d_scene::{Document, MeshHandle, Node, ObjectKind, ObjectKindTag};

    use super::*;
    use crate::OrbitCamera;

    fn evaluated(doc: &Document) -> EvaluatedScene {
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        eval
    }

    #[test]
    fn center_ray_picks_the_cube() {
        let doc = Document::with_demo_scene();
        let cam = OrbitCamera::default();
        let ray = cam.ray_from_ndc(Vec2::ZERO, 1.6);
        let id = pick(&evaluated(&doc), ray, ViewFilter::ALL).expect("should hit something");
        assert_eq!(doc.scene.get(id).unwrap().data.name, "Cube");
    }

    #[test]
    fn ray_into_the_sky_misses() {
        let doc = Document::with_demo_scene();
        let ray = Ray {
            origin: Vec3::new(0.0, 5000.0, 0.0),
            dir: Vec3::Y,
        };
        assert_eq!(pick(&evaluated(&doc), ray, ViewFilter::ALL), None);
    }

    #[test]
    fn spheres_are_picked_exactly_not_by_their_box() {
        let doc = Document::with_demo_scene();
        let sphere = doc.scene.iter().find(|(_, n)| n.data.name == "Sphere").unwrap();
        let center = doc.scene.world_matrix(sphere.0).w_axis.truncate();
        // Through the corner of its bounding box (radius 80): inside the box, outside the sphere.
        let corner = Ray {
            origin: center + Vec3::new(75.0, 75.0, -1000.0),
            dir: Vec3::Z,
        };
        assert_ne!(pick(&evaluated(&doc), corner, ViewFilter::ALL), Some(sphere.0));
        let through = Ray {
            origin: center + Vec3::new(0.0, 0.0, -1000.0),
            dir: Vec3::Z,
        };
        assert_eq!(pick(&evaluated(&doc), through, ViewFilter::ALL), Some(sphere.0));
    }

    #[test]
    fn polygon_objects_are_picked_by_their_triangles() {
        let mut doc = Document::new("meshes");
        let torus = grid4d_eval::editable_mesh(&ObjectKindTag::Torus.default_kind()).unwrap();
        let kind = ObjectKind::polygon(MeshHandle::new(torus));
        let id = doc
            .scene
            .insert(Node::new("Torus", kind).with_position(Vec3::new(0.0, 0.0, 500.0)));
        let eval = evaluated(&doc);
        let down = |x: f32| Ray {
            origin: Vec3::new(x, 1000.0, 500.0),
            dir: -Vec3::Y,
        };
        // Ring radius 200, pipe radius 50: the tube spans x = 150…250.
        assert_eq!(pick(&eval, down(200.0), ViewFilter::ALL), Some(id));
        assert_eq!(
            pick(&eval, down(0.0), ViewFilter::ALL),
            None,
            "the hole is inside the box but not the mesh"
        );
        assert_eq!(pick(&eval, down(400.0), ViewFilter::ALL), None);
    }

    #[test]
    fn hidden_types_let_the_ray_through() {
        let mut doc = Document::new("filter");
        let mesh = grid4d_eval::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap();
        let behind = doc
            .scene
            .insert(Node::new("Behind", ObjectKind::polygon(MeshHandle::new(mesh))));
        let front = doc
            .scene
            .insert(Node::new("Front", ObjectKindTag::Cube.default_kind()).with_position(Vec3::new(0.0, 0.0, 400.0)));
        let eval = evaluated(&doc);
        let ray = Ray {
            origin: Vec3::new(0.0, 0.0, 1000.0),
            dir: -Vec3::Z,
        };
        assert_eq!(pick(&eval, ray, ViewFilter::ALL), Some(front));
        let polygons_only = ViewFilter::only(crate::FilterItem::Polygon);
        assert_eq!(pick(&eval, ray, polygons_only), Some(behind));
        assert_eq!(pick(&eval, ray, ViewFilter::NONE), None);
    }

    #[test]
    fn splines_are_picked_near_their_line_on_screen() {
        let mut doc = Document::new("t");
        let circle = doc
            .scene
            .insert(Node::new("Circle", ObjectKindTag::Circle.default_kind()));
        let eval = evaluated(&doc);
        // Looking at the XY plane straight on: the circle of radius 200 around the centre.
        let mut camera = OrbitCamera::default();
        (camera.yaw, camera.pitch, camera.target, camera.distance) = (0.0, 0.0, Vec3::ZERO, 1000.0);
        let view = ScreenView::new(&camera, Vec2::new(800.0, 600.0));
        let on_line = view.to_screen(view.view_proj * Vec3::new(200.0, 0.0, 0.0).extend(1.0));
        assert_eq!(pick_curve(&eval, &view, on_line, 6.0, ViewFilter::ALL), Some(circle));
        assert_eq!(
            pick_curve(&eval, &view, on_line + Vec2::new(4.0, 0.0), 6.0, ViewFilter::ALL),
            Some(circle),
            "a few pixels off"
        );
        let centre = Vec2::new(400.0, 300.0);
        assert_eq!(
            pick_curve(&eval, &view, centre, 6.0, ViewFilter::ALL),
            None,
            "inside: nothing"
        );
        let hidden = ViewFilter::NONE;
        assert_eq!(pick_curve(&eval, &view, on_line, 6.0, hidden), None, "filtered out");
        // A ray never picks a curve.
        assert_eq!(
            pick(
                &eval,
                camera.ray_from_ndc(Vec2::new(0.5, 0.0), 800.0 / 600.0),
                ViewFilter::ALL
            ),
            None
        );
    }
}
