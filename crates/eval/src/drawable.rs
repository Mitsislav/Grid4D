//! How an object is drawn: which unit geometry, at what scale, in what colour;
//! and the mesh a primitive becomes when it is made editable.

use glam::Vec3;
use grid4d_geom::{PolyMesh, Smoothing};
use grid4d_scene::{CAMERA_MARKER_SIZE, DEFAULT_PHONG_ANGLE, LIGHT_MARKER_RADIUS, ObjectKind};

use crate::{MeshKey, Ratio};

/// How a drawn mesh's normals are smoothed: its Phong angle (radians), kept as
/// bits so it can be part of the renderer's cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shading(u32);

impl Shading {
    #[must_use]
    pub fn from_angle(radians: f32) -> Self {
        // `+ 0.0` keeps -0.0 and 0.0 one key.
        Self((radians.clamp(0.0, std::f32::consts::PI) + 0.0).to_bits())
    }

    #[must_use]
    pub const fn angle(self) -> f32 {
        f32::from_bits(self.0)
    }

    #[must_use]
    pub const fn smoothing(self) -> Smoothing {
        Smoothing::Angle(self.angle())
    }
}

impl Default for Shading {
    /// Grid4D's smoothing for objects without a Phong tag.
    fn default() -> Self {
        Self::from_angle(DEFAULT_PHONG_ANGLE)
    }
}

/// How an object is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawable {
    pub key: MeshKey,
    /// The Phong angle the mesh is shaded with (a Phong tag, or the default).
    pub shading: Shading,
    /// Object-space scale applied to the unit mesh.
    pub scale: Vec3,
    pub color: [f32; 3],
    pub unlit: bool,
    /// A viewport marker (camera, light) rather than geometry: not drawn while the
    /// eye is inside it, so a camera created at the view does not block the view.
    pub marker: bool,
}

/// The unit mesh, scale and colour for an object; `None` for objects without
/// geometry (Nulls, shapes with zero size, polygon objects without polygons).
#[must_use]
pub fn drawable(kind: &ObjectKind, color: [f32; 3]) -> Option<Drawable> {
    let shaded = |key, scale| {
        Some(Drawable {
            key,
            shading: Shading::default(),
            scale,
            color,
            unlit: false,
            marker: false,
        })
    };
    match *kind {
        ObjectKind::Null => None,
        ObjectKind::Cube { size, segments } => shaded(MeshKey::Cube { segments }, size),
        ObjectKind::Sphere { radius, segments } => shaded(MeshKey::Sphere { segments }, Vec3::splat(radius)),
        ObjectKind::Cylinder {
            radius,
            height,
            rotation_segments,
            height_segments,
        } => shaded(
            MeshKey::Cylinder {
                rotation: rotation_segments,
                height: height_segments,
            },
            Vec3::new(radius, height, radius),
        ),
        ObjectKind::Plane {
            width,
            depth,
            width_segments,
            depth_segments,
        } => shaded(
            MeshKey::Plane {
                width: width_segments,
                depth: depth_segments,
            },
            Vec3::new(width, 1.0, depth),
        ),
        ObjectKind::Cone {
            bottom_radius,
            top_radius,
            height,
            rotation_segments,
            height_segments,
        } => {
            let (rb, rt) = (bottom_radius.max(0.0), top_radius.max(0.0));
            let r = rb.max(rt);
            if r <= 0.0 {
                return None;
            }
            shaded(
                MeshKey::Cone {
                    bottom: Ratio::new(rb, r),
                    top: Ratio::new(rt, r),
                    rotation: rotation_segments,
                    height: height_segments,
                },
                Vec3::new(r, height, r),
            )
        }
        ObjectKind::Torus {
            ring_radius,
            pipe_radius,
            ring_segments,
            pipe_segments,
        } => {
            let outer = ring_radius.abs() + pipe_radius.abs();
            if outer <= 0.0 {
                return None;
            }
            shaded(
                MeshKey::Torus {
                    ring: Ratio::new(ring_radius.abs(), outer),
                    ring_segments,
                    pipe_segments,
                },
                Vec3::splat(outer),
            )
        }
        ObjectKind::Disc {
            inner_radius,
            outer_radius,
            rotation_segments,
            disc_segments,
        } => {
            let (ri, ro) = (inner_radius.max(0.0), outer_radius.max(0.0));
            let r = ri.max(ro);
            if r <= 0.0 || ro == ri {
                return None; // no area
            }
            shaded(
                MeshKey::Disc {
                    inner: Ratio::new(ri.min(ro), r),
                    outer: Ratio::new(ri.max(ro), r),
                    rotation: rotation_segments,
                    segments: disc_segments,
                },
                Vec3::new(r, 1.0, r),
            )
        }
        ObjectKind::Capsule {
            radius,
            height,
            rotation_segments,
            height_segments,
            cap_segments,
        } => {
            let r = radius.abs();
            if r <= 0.0 {
                return None;
            }
            let total = height.abs().max(2.0 * r);
            shaded(
                MeshKey::Capsule {
                    straight: Ratio::new(total - 2.0 * r, total),
                    rotation: rotation_segments,
                    height: height_segments,
                    cap: cap_segments,
                },
                Vec3::splat(r),
            )
        }
        ObjectKind::Camera { .. } => Some(Drawable {
            key: MeshKey::Cube { segments: [1, 1, 1] },
            shading: Shading::default(),
            scale: Vec3::splat(CAMERA_MARKER_SIZE),
            color,
            unlit: false,
            marker: true,
        }),
        ObjectKind::Light { color: light, .. } => Some(Drawable {
            key: MeshKey::Sphere { segments: 24 },
            shading: Shading::default(),
            scale: Vec3::splat(LIGHT_MARKER_RADIUS),
            color: light,
            unlit: true,
            marker: true,
        }),
        ObjectKind::Polygon { ref mesh, .. } => {
            if mesh.mesh().face_count() == 0 {
                return None;
            }
            shaded(MeshKey::Polygon(mesh.id()), Vec3::ONE)
        }
        // A generator draws its result, which evaluation builds.
        ObjectKind::SubdivisionSurface { .. } => None,
    }
}

/// The mesh a primitive becomes when it is made editable ("Make Editable"):
/// exactly what the viewport draws for it, with the size applied to the points.
/// `None` for objects that are not primitives (Nulls, cameras, lights) or are
/// already editable, and for shapes without area.
#[must_use]
pub fn editable_mesh(kind: &ObjectKind) -> Option<PolyMesh> {
    match kind {
        ObjectKind::Cube { .. }
        | ObjectKind::Sphere { .. }
        | ObjectKind::Cylinder { .. }
        | ObjectKind::Plane { .. }
        | ObjectKind::Cone { .. }
        | ObjectKind::Torus { .. }
        | ObjectKind::Disc { .. }
        | ObjectKind::Capsule { .. } => {
            let draw = drawable(kind, [0.0; 3])?;
            let mut mesh = draw.key.build()?;
            for point in mesh.points_mut() {
                *point *= draw.scale;
            }
            Some(mesh)
        }
        ObjectKind::Null
        | ObjectKind::Camera { .. }
        | ObjectKind::Light { .. }
        | ObjectKind::Polygon { .. }
        | ObjectKind::SubdivisionSurface { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{MeshHandle, ObjectKindTag};

    use super::*;

    #[test]
    fn shape_ratios_are_part_of_the_key_and_size_is_the_scale() {
        let torus = |ring, pipe| ObjectKind::Torus {
            ring_radius: ring,
            pipe_radius: pipe,
            ring_segments: 36,
            pipe_segments: 18,
        };
        let small = drawable(&torus(200.0, 50.0), [1.0; 3]).unwrap();
        let big = drawable(&torus(400.0, 100.0), [1.0; 3]).unwrap();
        let fat = drawable(&torus(200.0, 100.0), [1.0; 3]).unwrap();
        assert_eq!(small.key, big.key, "uniformly scaled: same mesh");
        assert_eq!(big.scale, Vec3::splat(500.0));
        assert_ne!(small.key, fat.key, "a thicker pipe is another shape");
    }

    #[test]
    fn every_kind_is_drawable_except_nulls_and_empty_shapes() {
        for tag in ObjectKindTag::ALL {
            let style = drawable(&tag.default_kind(), [1.0; 3]);
            // The default polygon object is empty; a generator draws its result.
            let expected = !matches!(
                tag,
                ObjectKindTag::Null | ObjectKindTag::Polygon | ObjectKindTag::SubdivisionSurface
            );
            assert_eq!(style.is_some(), expected, "{tag:?}");
        }
        let flat_cone = ObjectKind::Cone {
            bottom_radius: 0.0,
            top_radius: 0.0,
            height: 10.0,
            rotation_segments: 8,
            height_segments: 1,
        };
        assert_eq!(drawable(&flat_cone, [1.0; 3]), None);
    }

    #[test]
    fn editable_meshes_look_like_the_primitive() {
        for tag in ObjectKindTag::PRIMITIVES {
            let kind = tag.default_kind();
            let mesh = editable_mesh(&kind).unwrap_or_else(|| panic!("{tag:?}"));
            let draw = drawable(&kind, [1.0; 3]).unwrap();
            let unit = draw.key.build().unwrap();
            assert_eq!(mesh.face_count(), unit.face_count(), "{tag:?}");
            let (min, max) = mesh.bounds().unwrap();
            let (umin, umax) = unit.bounds().unwrap();
            assert!(
                min.abs_diff_eq(umin * draw.scale, 1e-3),
                "{tag:?}: size applied to the points"
            );
            assert!(max.abs_diff_eq(umax * draw.scale, 1e-3), "{tag:?}");
        }
        for tag in [
            ObjectKindTag::Null,
            ObjectKindTag::Camera,
            ObjectKindTag::Light,
            ObjectKindTag::Polygon,
        ] {
            assert!(editable_mesh(&tag.default_kind()).is_none(), "{tag:?}");
        }
    }

    #[test]
    fn editable_cylinders_keep_their_caps_separate() {
        let kind = ObjectKindTag::Cylinder.default_kind(); // 36 around, 4 high
        let mesh = editable_mesh(&kind).unwrap();
        let mut distinct: Vec<[u32; 3]> = mesh.points().iter().map(|p| p.to_array().map(f32::to_bits)).collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(mesh.point_count() - distinct.len(), 2 * 36, "each rim exists twice");
        assert!(
            !grid4d_geom::Topology::build(&mesh).is_closed(),
            "until Optimize merges them"
        );
    }

    #[test]
    fn polygon_objects_draw_their_mesh_unscaled() {
        let mesh = MeshHandle::new(editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap());
        let kind = ObjectKind::polygon(mesh.clone());
        let d = drawable(&kind, [0.5; 3]).unwrap();
        assert_eq!(
            (d.key, d.scale, d.unlit),
            (MeshKey::Polygon(mesh.id()), Vec3::ONE, false)
        );
        assert_eq!(MeshKey::Polygon(mesh.id()).build(), None, "stored, not generated");
    }

    #[test]
    fn lights_are_unlit_markers_in_their_own_colour() {
        let light = ObjectKind::Light {
            color: [1.0, 0.5, 0.25],
            intensity: 2.0,
        };
        let d = drawable(&light, [0.0; 3]).unwrap();
        assert!(d.unlit);
        assert_eq!(d.color, [1.0, 0.5, 0.25]);
        assert_eq!(d.scale, Vec3::splat(LIGHT_MARKER_RADIUS));
    }
}
