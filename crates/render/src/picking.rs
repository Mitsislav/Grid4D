use glam::Vec3;
use oc4d_scene::{LIGHT_MARKER_RADIUS, NodeId, ObjectKind, Scene, WalkItem};

use crate::Ray;

/// Returns the closest visible object hit by `ray`.
///
/// Tests in each object's local space (exact for spheres, oriented boxes for
/// everything else). `stack` is a reusable traversal buffer.
pub fn pick(scene: &Scene, ray: Ray, stack: &mut Vec<WalkItem>) -> Option<NodeId> {
    let mut best: Option<(f32, NodeId)> = None;
    scene.walk(stack, |id, node, world, _| {
        if !node.data.visible_in_editor || !node.data.enabled {
            return false;
        }
        let Some(bounds) = node.data.kind.local_bounds() else {
            return true;
        };
        if world.determinant().abs() <= f32::EPSILON {
            return true;
        }
        let inv = world.inverse();
        let origin = inv.transform_point3(ray.origin);
        let dir = inv.transform_vector3(ray.dir);
        let hit = match node.data.kind {
            ObjectKind::Sphere { radius } => ray_sphere(origin, dir, radius),
            ObjectKind::Light { .. } => ray_sphere(origin, dir, LIGHT_MARKER_RADIUS),
            _ => bounds.ray_intersect(origin, dir),
        };
        // Affine maps preserve the ray parameter, so `t` is comparable across objects.
        if let Some(t) = hit
            && best.is_none_or(|(best_t, _)| t < best_t)
        {
            best = Some((t, id));
        }
        true
    });
    best.map(|(_, id)| id)
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
    use oc4d_scene::Document;

    use super::*;
    use crate::OrbitCamera;

    #[test]
    fn center_ray_picks_the_cube() {
        let doc = Document::with_demo_scene();
        let cam = OrbitCamera::default();
        let ray = cam.ray_from_ndc(Vec2::ZERO, 1.6);
        let id = pick(&doc.scene, ray, &mut Vec::new()).expect("should hit something");
        assert_eq!(doc.scene.get(id).unwrap().data.name, "Cube");
    }

    #[test]
    fn ray_into_the_sky_misses() {
        let doc = Document::with_demo_scene();
        let ray = Ray {
            origin: Vec3::new(0.0, 5000.0, 0.0),
            dir: Vec3::Y,
        };
        assert_eq!(pick(&doc.scene, ray, &mut Vec::new()), None);
    }
}
