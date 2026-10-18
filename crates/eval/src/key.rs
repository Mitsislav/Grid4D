//! A [`MeshKey`] names an object's geometry at *unit size*; the object's size
//! becomes a scale applied by whoever draws it.
//!
//! Objects that differ only in size share one key (and so one GPU mesh and one
//! instanced draw call). Parameters that change the shape itself — segment
//! counts, a cone's radius ratio, a torus's pipe thickness — are part of the key.

use glam::Vec3;
use oc4d_geom::{PolyMesh, primitives};

/// A shape ratio in `[0, 1]`, stored as `f32` bits so keys can be hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ratio(u32);

impl Ratio {
    /// `value / of`, clamped to `[0, 1]`; `-0.0` and `+0.0` give the same key.
    #[must_use]
    pub fn new(value: f32, of: f32) -> Self {
        let r = if of > 0.0 { (value / of).clamp(0.0, 1.0) } else { 0.0 };
        Self((r + 0.0).to_bits())
    }

    #[must_use]
    pub fn get(self) -> f32 {
        f32::from_bits(self.0)
    }
}

/// Unit-size geometry of a drawable object (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshKey {
    /// Cube spanning `[-0.5, 0.5]³`.
    Cube { segments: [u32; 3] },
    /// Sphere of radius 1.
    Sphere { segments: u32 },
    /// Radius 1, height 1.
    Cylinder { rotation: u32, height: u32 },
    /// 1 × 1 on the XZ plane.
    Plane { width: u32, depth: u32 },
    /// Height 1; the larger radius is 1.
    Cone {
        bottom: Ratio,
        top: Ratio,
        rotation: u32,
        height: u32,
    },
    /// Outer radius 1: `ring` is the ring radius' share of it, the pipe takes the rest.
    Torus {
        ring: Ratio,
        ring_segments: u32,
        pipe_segments: u32,
    },
    /// The larger radius is 1.
    Disc {
        inner: Ratio,
        outer: Ratio,
        rotation: u32,
        segments: u32,
    },
    /// Radius 1; `straight` is the share of the total height that is cylinder.
    Capsule {
        straight: Ratio,
        rotation: u32,
        height: u32,
        cap: u32,
    },
}

impl MeshKey {
    /// The unit geometry for this key.
    #[must_use]
    pub fn build(self) -> PolyMesh {
        match self {
            Self::Cube { segments } => primitives::cube(Vec3::ONE, segments),
            Self::Sphere { segments } => primitives::sphere(1.0, segments),
            Self::Cylinder { rotation, height } => primitives::cylinder(1.0, 1.0, rotation, height),
            Self::Plane { width, depth } => primitives::plane(1.0, 1.0, width, depth),
            Self::Cone {
                bottom,
                top,
                rotation,
                height,
            } => primitives::cone(bottom.get(), top.get(), 1.0, rotation, height),
            Self::Torus {
                ring,
                ring_segments,
                pipe_segments,
            } => primitives::torus(ring.get(), 1.0 - ring.get(), ring_segments, pipe_segments),
            Self::Disc {
                inner,
                outer,
                rotation,
                segments,
            } => primitives::disc(inner.get(), outer.get(), rotation, segments),
            Self::Capsule {
                straight,
                rotation,
                height,
                cap,
            } => {
                // Radius 1 and total height h: the straight part is h − 2 = s·h.
                let s = straight.get().min(0.999);
                primitives::capsule(1.0, 2.0 / (1.0 - s), rotation, height, cap)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_meshes_have_unit_size() {
        let size = |key: MeshKey| key.build().bounds().map(|(min, max)| max - min).unwrap();
        assert!(size(MeshKey::Cube { segments: [1, 1, 1] }).abs_diff_eq(Vec3::ONE, 1e-6));
        assert!(size(MeshKey::Sphere { segments: 24 }).abs_diff_eq(Vec3::splat(2.0), 1e-3));
        assert!(
            size(MeshKey::Cylinder {
                rotation: 36,
                height: 1
            })
            .abs_diff_eq(Vec3::new(2.0, 1.0, 2.0), 1e-3)
        );
        let capsule = MeshKey::Capsule {
            straight: Ratio::new(100.0, 200.0),
            rotation: 36,
            height: 1,
            cap: 8,
        };
        assert!(
            size(capsule).abs_diff_eq(Vec3::new(2.0, 4.0, 2.0), 1e-3),
            "radius 1, total height 4"
        );
    }

    #[test]
    fn ratios_are_canonical() {
        assert_eq!(Ratio::new(-0.0, 1.0), Ratio::new(0.0, 1.0));
        assert_eq!(Ratio::new(5.0, 0.0).get(), 0.0);
        assert_eq!(Ratio::new(3.0, 2.0).get(), 1.0);
    }
}
