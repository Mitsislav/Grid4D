//! Collision shapes from what the viewport shows, and a body's surface and
//! mass on them.

use glam::{Mat4, Vec3};
use grid4d_geom::{PolyMesh, Triangulator};
use grid4d_scene::{Body, CollisionShape, MassMode, Motion};
use rapier3d::prelude::{CoefficientCombineRule, ColliderBuilder, MassProperties, Vector};

use crate::convert::{length_to_engine, to_engine};

/// Thinnest box side (cm), so a flat plane still has a box to hit.
const MIN_HALF: f32 = 0.5;

/// Colliders for one part of a body: `mesh` placed by `to_body` (the part's
/// points into the body's space, in centimetres, scale included).
pub(crate) fn colliders(mesh: &PolyMesh, to_body: Mat4, body: &Body) -> Vec<ColliderBuilder> {
    let points: Vec<Vec3> = mesh.points().iter().map(|&p| to_body.transform_point3(p)).collect();
    let Some((min, max)) = bounds(&points) else {
        return Vec::new();
    };
    let shape = match (body.shape, body.motion) {
        (CollisionShape::Automatic, Motion::Dynamic) | (CollisionShape::StaticMesh, Motion::Dynamic) => {
            CollisionShape::ConvexHull
        }
        (CollisionShape::Automatic, Motion::Collider) => CollisionShape::StaticMesh,
        (shape, _) => shape,
    };
    let centre = (min + max) * 0.5;
    let half = ((max - min) * 0.5).max(Vec3::splat(MIN_HALF));
    let cuboid = || {
        let [x, y, z] = half.to_array().map(length_to_engine);
        ColliderBuilder::cuboid(x, y, z).translation(to_engine(centre))
    };
    let collider = match shape {
        CollisionShape::Box | CollisionShape::Automatic => cuboid(),
        CollisionShape::Sphere => {
            ColliderBuilder::ball(length_to_engine(half.max_element())).translation(to_engine(centre))
        }
        CollisionShape::ConvexHull => {
            let engine: Vec<Vector> = points.iter().map(|&p| to_engine(p)).collect();
            // Flat or degenerate shapes have no hull: their box instead.
            ColliderBuilder::convex_hull(&engine).unwrap_or_else(cuboid)
        }
        CollisionShape::StaticMesh => {
            let (mut triangles, mut faces) = (Vec::new(), Vec::new());
            Triangulator::new().triangulate_points(mesh, &mut triangles, &mut faces);
            let engine: Vec<Vector> = points.iter().map(|&p| to_engine(p)).collect();
            ColliderBuilder::trimesh(engine, triangles).unwrap_or_else(|_| cuboid())
        }
    };
    vec![collider]
}

/// The total mass of `colliders` at density 1 (kg), for spreading a Custom
/// Mass over a body's parts.
pub(crate) fn volume_mass(colliders: &[ColliderBuilder]) -> f32 {
    colliders.iter().map(|c| c.shape.mass_properties(1.0).mass()).sum()
}

/// `collider` with `body`'s bounce, friction and mass (`total`: the whole
/// body's mass at density 1).
pub(crate) fn surface(collider: ColliderBuilder, body: &Body, total: f32) -> ColliderBuilder {
    let density = match body.mass_mode {
        MassMode::GlobalDensity => 1.0,
        MassMode::CustomDensity => body.density,
        // The custom mass, spread over the parts by their volume.
        MassMode::CustomMass if total > 0.0 => body.mass / total,
        MassMode::CustomMass => 0.0,
    };
    let mut properties: MassProperties = collider.shape.mass_properties(density.max(0.0));
    // Rotational Mass: harder (or easier) to set spinning.
    properties.inv_principal_inertia /= body.rotational_mass.max(Body::MIN_ROTATIONAL_MASS);
    collider
        .restitution(body.bounce)
        .restitution_combine_rule(CoefficientCombineRule::Max)
        .friction(body.friction)
        .friction_combine_rule(CoefficientCombineRule::Multiply)
        .mass_properties(properties)
}

fn bounds(points: &[Vec3]) -> Option<(Vec3, Vec3)> {
    let first = *points.first()?;
    Some(
        points
            .iter()
            .fold((first, first), |(min, max), &p| (min.min(p), max.max(p))),
    )
}
