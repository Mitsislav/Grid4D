//! The scene as the renderer sees it: every visible surface as world-space
//! triangles in one bounding volume hierarchy, with their materials, and the
//! lights.

use glam::{Mat3, Mat4, Vec3};
use grid4d_eval::{EvaluatedLight, EvaluatedScene, MeshKey};
use grid4d_geom::{Bvh, PolyMesh, TriMesh, Triangulator};
use grid4d_scene::{Falloff, LightShadow, LightType};

/// How a surface reflects and gives off light: its material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Surface {
    /// Linear RGB.
    pub color: Vec3,
    pub roughness: f32,
    pub metallic: f32,
    /// Light it gives off, linear RGB.
    pub emission: Vec3,
}

/// A light as the renderer uses it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Light {
    pub kind: LightType,
    pub position: Vec3,
    /// The direction it shines in, unit length.
    pub direction: Vec3,
    /// Colour × intensity, linear RGB.
    pub color: Vec3,
    /// Cosines of half the outer and inner cone angles (equal for a hard edge).
    pub cos_outer: f32,
    pub cos_inner: f32,
    pub falloff: Falloff,
    pub radius: f32,
    pub shadow: LightShadow,
}

impl Light {
    fn new(light: &EvaluatedLight) -> Self {
        let details = &light.details;
        let half_cos = |degrees: f32| (degrees.to_radians() * 0.5).cos();
        let cos_outer = half_cos(details.outer_angle_deg);
        Self {
            kind: light.light_type,
            position: light.position,
            direction: light.direction,
            color: Vec3::from(light.color),
            cos_outer,
            cos_inner: if details.use_inner {
                half_cos(details.inner_angle_deg.min(details.outer_angle_deg))
            } else {
                cos_outer
            },
            falloff: details.falloff,
            radius: details.radius,
            shadow: light.shadow,
        }
    }
}

/// Where a ray met a surface.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub position: Vec3,
    /// The triangle's own normal (unit; either side).
    pub geometric: Vec3,
    /// The smoothed normal there (unit; either side).
    pub shading: Vec3,
    pub surface: Surface,
}

/// Everything a render needs from the scene, built once per render.
#[derive(Debug)]
pub struct RenderScene {
    /// Every triangle as its own face of three points, in world space.
    triangles: PolyMesh,
    bvh: Bvh,
    /// Three smoothed normals per triangle.
    normals: Vec<Vec3>,
    surfaces: Vec<Surface>,
    /// The surface of each triangle.
    surface_of: Vec<u32>,
    lights: Vec<Light>,
    bounds: Option<(Vec3, Vec3)>,
}

impl RenderScene {
    /// The scene `evaluated` shows — evaluate it with
    /// [`EvaluatedScene::for_render`] for what the renderer shows. Light and
    /// camera markers are left out.
    #[must_use]
    pub fn new(evaluated: &EvaluatedScene) -> Self {
        let mut points = Vec::new();
        let mut normals = Vec::new();
        let mut surfaces = Vec::new();
        let mut surface_of = Vec::new();
        let mut bounds: Option<(Vec3, Vec3)> = None;
        let mut triangulator = Triangulator::new();
        let mut tri = TriMesh::default();
        for node in evaluated.nodes() {
            let Some(draw) = node.drawable.filter(|d| !d.marker) else {
                continue;
            };
            let generated;
            let mesh = match draw.key {
                MeshKey::Polygon(id) => match evaluated.mesh(id) {
                    Some(handle) => handle.mesh(),
                    None => continue,
                },
                key => match key.build() {
                    Some(mesh) => {
                        generated = mesh;
                        &generated
                    }
                    None => continue,
                },
            };
            let model = node.world * Mat4::from_scale(draw.scale);
            let linear = Mat3::from_mat4(model);
            if linear.determinant().abs() <= f32::EPSILON {
                continue; // flattened to nothing
            }
            let normal_matrix = linear.inverse().transpose();
            triangulator.triangulate(mesh, draw.shading.smoothing(), &mut tri);
            let index = u32::try_from(surfaces.len()).unwrap_or(u32::MAX);
            surfaces.push(Surface {
                color: Vec3::from(draw.color),
                roughness: draw.surface.roughness,
                metallic: draw.surface.metallic,
                emission: Vec3::from(draw.surface.emission),
            });
            for triangle in tri.indices.as_chunks::<3>().0 {
                for &corner in triangle {
                    let corner = usize::try_from(corner).unwrap_or(usize::MAX);
                    let (Some(&p), Some(&n)) = (tri.positions.get(corner), tri.normals.get(corner)) else {
                        continue;
                    };
                    let p = model.transform_point3(p);
                    points.push(p);
                    normals.push((normal_matrix * n).normalize_or_zero());
                    bounds = Some(bounds.map_or((p, p), |(min, max)| (min.min(p), max.max(p))));
                }
                surface_of.push(index);
            }
        }
        let faces = (0..surface_of.len()).map(|t| {
            let first = u32::try_from(t * 3).unwrap_or(u32::MAX - 2);
            [first, first + 1, first + 2]
        });
        let triangles = PolyMesh::from_faces(points, faces).unwrap_or_default();
        let bvh = Bvh::build(&triangles);
        Self {
            triangles,
            bvh,
            normals,
            surfaces,
            surface_of,
            lights: evaluated.lights().iter().map(Light::new).collect(),
            bounds,
        }
    }

    /// The world bounds of every surface (min, max); `None` for an empty scene.
    #[must_use]
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        self.bounds
    }

    /// Number of triangles.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.surface_of.len()
    }

    pub(crate) fn lights(&self) -> &[Light] {
        &self.lights
    }

    /// The nearest surface the ray `origin + t · dir` meets.
    pub(crate) fn intersect(&self, origin: Vec3, dir: Vec3) -> Option<Hit> {
        let hit = self.bvh.raycast(self.triangles.points(), origin, dir)?;
        let triangle = usize::try_from(hit.face).ok()?;
        let corners = triangle * 3;
        let points = self.triangles.points().get(corners..corners + 3)?;
        let normals = self.normals.get(corners..corners + 3)?;
        let (a, e1, e2) = (points[0], points[1] - points[0], points[2] - points[0]);
        let position = origin + dir * hit.t;
        // Barycentric weights of `position` in the triangle.
        let p = position - a;
        let (d00, d01, d11) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
        let (d20, d21) = (p.dot(e1), p.dot(e2));
        let denominator = d00 * d11 - d01 * d01;
        let (wb, wc) = if denominator.abs() > f32::MIN_POSITIVE {
            (
                (d11 * d20 - d01 * d21) / denominator,
                (d00 * d21 - d01 * d20) / denominator,
            )
        } else {
            (0.0, 0.0)
        };
        let geometric = e1.cross(e2).normalize_or(Vec3::Y);
        let shading = (normals[0] * (1.0 - wb - wc) + normals[1] * wb + normals[2] * wc).normalize_or(geometric);
        let surface = *self
            .surface_of
            .get(triangle)
            .and_then(|&s| self.surfaces.get(usize::try_from(s).ok()?))?;
        Some(Hit {
            position,
            geometric,
            shading,
            surface,
        })
    }

    /// Whether a surface blocks the ray `origin + t · dir` before `distance`.
    pub(crate) fn occluded(&self, origin: Vec3, dir: Vec3, distance: f32) -> bool {
        self.bvh
            .raycast(self.triangles.points(), origin, dir)
            .is_some_and(|hit| hit.t < distance)
    }
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{Document, Node, ObjectKind, ObjectKindTag};

    use super::*;

    #[test]
    fn surfaces_become_world_triangles_and_markers_stay_out() {
        let mut doc = Document::new("scene");
        doc.scene
            .insert(Node::new("Cube", ObjectKindTag::Cube.default_kind()).with_position(Vec3::new(0.0, 100.0, 0.0)));
        doc.scene.insert(Node::new("Light", ObjectKind::light([1.0; 3], 1.0)));
        doc.scene
            .insert(Node::new("Camera", ObjectKindTag::Camera.default_kind()));
        let mut eval = EvaluatedScene::for_render();
        eval.update(&doc.scene);
        let scene = RenderScene::new(&eval);
        assert_eq!(scene.triangle_count(), 12, "the cube only");
        let (min, max) = scene.bounds().unwrap();
        assert!(min.abs_diff_eq(Vec3::new(-100.0, 0.0, -100.0), 1e-3));
        assert!(max.abs_diff_eq(Vec3::new(100.0, 200.0, 100.0), 1e-3));
        assert_eq!(scene.lights().len(), 1);

        // A ray down onto the top face.
        let hit = scene.intersect(Vec3::new(10.0, 500.0, 20.0), Vec3::NEG_Y).unwrap();
        assert!(hit.position.abs_diff_eq(Vec3::new(10.0, 200.0, 20.0), 1e-3));
        assert!(hit.geometric.y.abs() > 0.999 && hit.shading.abs_diff_eq(Vec3::Y, 1e-4));
        assert!(scene.occluded(Vec3::new(10.0, 500.0, 20.0), Vec3::NEG_Y, 400.0));
        assert!(!scene.occluded(Vec3::new(10.0, 500.0, 20.0), Vec3::NEG_Y, 250.0));
    }
}
