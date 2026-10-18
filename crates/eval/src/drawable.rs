//! How an object is drawn: which unit geometry, at what scale, in what colour.

use glam::Vec3;
use grid4d_scene::{CAMERA_MARKER_SIZE, LIGHT_MARKER_RADIUS, ObjectKind};

use crate::{MeshKey, Ratio};

/// How an object is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawable {
    pub key: MeshKey,
    /// Object-space scale applied to the unit mesh.
    pub scale: Vec3,
    pub color: [f32; 3],
    pub unlit: bool,
}

/// The unit mesh, scale and colour for an object; `None` for objects without
/// geometry (Nulls, shapes with zero size).
#[must_use]
pub fn drawable(kind: &ObjectKind, color: [f32; 3]) -> Option<Drawable> {
    let shaded = |key, scale| {
        Some(Drawable {
            key,
            scale,
            color,
            unlit: false,
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
        ObjectKind::Camera { .. } => shaded(MeshKey::Cube { segments: [1, 1, 1] }, Vec3::splat(CAMERA_MARKER_SIZE)),
        ObjectKind::Light { color: light, .. } => Some(Drawable {
            key: MeshKey::Sphere { segments: 24 },
            scale: Vec3::splat(LIGHT_MARKER_RADIUS),
            color: light,
            unlit: true,
        }),
    }
}

#[cfg(test)]
mod tests {
    use grid4d_scene::ObjectKindTag;

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
            assert_eq!(style.is_some(), tag != ObjectKindTag::Null, "{tag:?}");
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
