use glam::Vec3;
use grid4d_eval::{EvaluatedScene, PickShape};
use grid4d_scene::NodeId;

use crate::Ray;

/// Returns the closest visible object hit by `ray`.
///
/// Tests in each object's local space, using its [`PickShape`]: exact for
/// spheres and polygon objects (a BVH per mesh, built on its first pick),
/// oriented bounding boxes for everything else (Nulls are skipped).
/// Allocation-free once each mesh's BVH exists.
#[must_use]
pub fn pick(scene: &EvaluatedScene, ray: Ray) -> Option<NodeId> {
    let mut best: Option<(f32, NodeId)> = None;
    for node in scene.nodes() {
        if node.pick == PickShape::None || node.world.determinant().abs() <= f32::EPSILON {
            continue;
        }
        let inv = node.world.inverse();
        let origin = inv.transform_point3(ray.origin);
        let dir = inv.transform_vector3(ray.dir);
        let hit = match node.pick {
            PickShape::None => None,
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
        let id = pick(&evaluated(&doc), ray).expect("should hit something");
        assert_eq!(doc.scene.get(id).unwrap().data.name, "Cube");
    }

    #[test]
    fn ray_into_the_sky_misses() {
        let doc = Document::with_demo_scene();
        let ray = Ray {
            origin: Vec3::new(0.0, 5000.0, 0.0),
            dir: Vec3::Y,
        };
        assert_eq!(pick(&evaluated(&doc), ray), None);
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
        assert_ne!(pick(&evaluated(&doc), corner), Some(sphere.0));
        let through = Ray {
            origin: center + Vec3::new(0.0, 0.0, -1000.0),
            dir: Vec3::Z,
        };
        assert_eq!(pick(&evaluated(&doc), through), Some(sphere.0));
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
        assert_eq!(pick(&eval, down(200.0)), Some(id));
        assert_eq!(
            pick(&eval, down(0.0)),
            None,
            "the hole is inside the box but not the mesh"
        );
        assert_eq!(pick(&eval, down(400.0)), None);
    }
}
