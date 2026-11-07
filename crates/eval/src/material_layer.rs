//! The Material tags that reach an object, as evaluation resolves them (see
//! `docs/reference/tag-material.md`): which materials it shows, in which
//! order, and how each lies on its surface.
//!
//! An object shows its own Material tags that link an existing material, in
//! their order, each later one over the earlier ones; without any it shows its
//! nearest ancestor's. The layers are kept once per update in
//! [`EvaluatedScene`](crate::EvaluatedScene) and named by a
//! [`MaterialLayers`] range, so children share their parent's.

use glam::{Affine2, Mat4, Vec2};
use grid4d_geom::{PolyMesh, uv};
use grid4d_scene::{Aabb, MaterialId, MaterialTag, NodeId, Projection};

/// One Material tag as it reaches an object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialLayer {
    pub material: MaterialId,
    pub projection: Projection,
    /// The tag's turn, tiling and offset, applied to the projected
    /// coordinates (before a channel's own tiling and offset).
    pub uv_transform: Affine2,
    /// From the object's space into the projection's shape's own (the cube
    /// −1 … 1); `None`: the shape is fitted to the object's bounds.
    pub placement: Option<Mat4>,
    /// Camera projection: the camera object (`None`: the view's camera).
    pub camera: Option<NodeId>,
}

impl MaterialLayer {
    #[must_use]
    pub fn new(material: MaterialId, tag: &MaterialTag) -> Self {
        Self {
            material,
            projection: tag.projection,
            uv_transform: tag.uv_transform(),
            placement: tag.placement.map(|p| p.to_unit()),
            camera: tag.camera,
        }
    }

    /// From the space of an object with object-space `bounds` into the
    /// projection's shape's own; `None` for UV and Camera, or for a fitted
    /// shape on an object without bounds.
    #[must_use]
    pub fn to_unit(&self, bounds: Option<Aabb>) -> Option<Mat4> {
        if !self.projection.has_shape() {
            return None;
        }
        self.placement
            .or_else(|| bounds.filter(|b| !b.is_empty()).map(|b| uv::fit(b.min, b.max)))
    }

    /// The texture coordinates this layer gives each corner of `mesh` (its
    /// points in the object's space, at their size), the tag's transform
    /// applied. `camera` maps the object's space to the camera's clip space
    /// (Camera projection; without one the mesh's UVs are used, as for UV).
    #[must_use]
    pub fn corners(&self, mesh: &PolyMesh, bounds: Option<Aabb>, camera: Option<Mat4>) -> Vec<Vec2> {
        let (projection, matrix) = match self.projection {
            Projection::Camera => camera.map_or((Projection::Uv, Mat4::IDENTITY), |m| (Projection::Camera, m)),
            Projection::Uv => (Projection::Uv, Mat4::IDENTITY),
            Projection::Box | Projection::Spherical | Projection::Cylindrical | Projection::Flat => {
                match self.to_unit(bounds) {
                    Some(m) => (self.projection, m),
                    None => (Projection::Uv, Mat4::IDENTITY),
                }
            }
        };
        let mut uvs = uv::project(projection, mesh, matrix);
        if self.uv_transform != Affine2::IDENTITY {
            for uv in &mut uvs {
                *uv = self.uv_transform.transform_point2(*uv);
            }
        }
        uvs
    }
}

/// A run of [`MaterialLayer`]s in an [`EvaluatedScene`](crate::EvaluatedScene)
/// (see [`EvaluatedScene::material_layers`](crate::EvaluatedScene::material_layers)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaterialLayers {
    pub(crate) start: u32,
    pub(crate) len: u32,
}

impl MaterialLayers {
    pub const EMPTY: Self = Self { start: 0, len: 0 };

    /// The layers from `start` up to `end` of the scene's list.
    pub(crate) fn between(start: usize, end: usize) -> Self {
        let start32 = u32::try_from(start).unwrap_or(u32::MAX);
        Self {
            start: start32,
            len: u32::try_from(end.saturating_sub(start)).unwrap_or(0),
        }
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn len(self) -> usize {
        usize::try_from(self.len).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use glam::Vec3;
    use grid4d_scene::{Material, Materials, Placement};

    use super::*;

    #[test]
    fn a_layer_projects_and_transforms_coordinates() {
        let mut materials = Materials::default();
        let m = materials.insert(Material::new("Mat"));
        let cube = grid4d_geom::primitives::cube(Vec3::new(4.0, 2.0, 6.0), [1, 1, 1]);
        let (min, max) = cube.bounds().unwrap();
        let bounds = Some(Aabb { min, max });

        let mut tag = MaterialTag::new(Some(m));
        let layer = MaterialLayer::new(m, &tag);
        assert_eq!(layer.to_unit(bounds), None, "UV has no shape");
        assert_eq!(
            layer.corners(&cube, bounds, None),
            cube.uvs().unwrap(),
            "the cube's own UVs"
        );

        tag.projection = Projection::Box;
        tag.tiling = [2.0, 2.0];
        let layer = MaterialLayer::new(m, &tag);
        let uvs = layer.corners(&cube, bounds, None);
        let (lo, hi) = uvs.iter().fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(a, b), &uv| {
            (a.min(uv), b.max(uv))
        });
        assert!(
            lo.abs_diff_eq(Vec2::ZERO, 1e-5) && hi.abs_diff_eq(Vec2::splat(2.0), 1e-5),
            "fitted, tiled twice"
        );

        tag.placement = Some(Placement {
            position: Vec3::ZERO,
            rotation: Vec3::ZERO,
            size: Vec3::splat(12.0),
        });
        let layer = MaterialLayer::new(m, &tag);
        assert!(layer.to_unit(None).is_some(), "a placement needs no bounds");
        let uvs = layer.corners(&cube, None, None);
        assert!(uvs.iter().all(|uv| uv.min_element() >= 0.5 && uv.max_element() <= 1.5));

        // Camera without a camera: the object's own UVs.
        tag.projection = Projection::Camera;
        tag.tiling = [1.0; 2];
        let layer = MaterialLayer::new(m, &tag);
        assert_eq!(layer.corners(&cube, bounds, None), cube.uvs().unwrap());
        assert_eq!(MaterialLayers::between(3, 5).len(), 2);
        assert!(MaterialLayers::EMPTY.is_empty());
    }
}
