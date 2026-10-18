use glam::{Mat4, Vec3};

/// Axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// The empty box: the identity element of [`Aabb::union`].
    pub const EMPTY: Self = Self {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    #[must_use]
    pub fn from_half_extents(half: Vec3) -> Self {
        Self { min: -half, max: half }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min.cmpgt(self.max).any()
    }

    #[must_use]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Radius of the bounding sphere around [`Aabb::center`].
    #[must_use]
    pub fn radius(&self) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            (self.max - self.min).length() * 0.5
        }
    }

    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Bounds of this box after an affine transform (transforms all 8 corners).
    #[must_use]
    pub fn transformed(&self, m: &Mat4) -> Self {
        if self.is_empty() {
            return *self;
        }
        let mut out = Self::EMPTY;
        for i in 0..8u8 {
            let corner = Vec3::new(
                if i & 1 == 0 { self.min.x } else { self.max.x },
                if i & 2 == 0 { self.min.y } else { self.max.y },
                if i & 4 == 0 { self.min.z } else { self.max.z },
            );
            let p = m.transform_point3(corner);
            out.min = out.min.min(p);
            out.max = out.max.max(p);
        }
        out
    }

    /// Slab test. Returns the ray parameter `t >= 0` of the entry point, if hit.
    /// `dir` does not need to be normalised; `t` is expressed in units of `dir`.
    #[must_use]
    pub fn ray_intersect(&self, origin: Vec3, dir: Vec3) -> Option<f32> {
        let inv = dir.recip();
        let t0 = (self.min - origin) * inv;
        let t1 = (self.max - origin) * inv;
        // NaN (0 · ∞): parallel to that slab and starting on its boundary, i.e. inside it.
        let on_face = t0.is_nan_mask() | t1.is_nan_mask();
        let t_near = Vec3::select(on_face, Vec3::NEG_INFINITY, t0.min(t1)).max_element();
        let t_far = Vec3::select(on_face, Vec3::INFINITY, t0.max(t1)).min_element();
        if t_far >= t_near.max(0.0) && t_far.is_finite() {
            Some(t_near.max(0.0))
        } else {
            None
        }
    }
}

impl Default for Aabb {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_and_misses() {
        let b = Aabb::from_half_extents(Vec3::splat(1.0));
        assert_eq!(b.ray_intersect(Vec3::new(0.0, 0.0, -5.0), Vec3::Z), Some(4.0));
        assert_eq!(b.ray_intersect(Vec3::new(3.0, 0.0, -5.0), Vec3::Z), None);
        // Starting inside the box hits at t = 0.
        assert_eq!(b.ray_intersect(Vec3::ZERO, Vec3::X), Some(0.0));
        // Grazing a face (axis-aligned, exactly on the boundary) still hits.
        assert_eq!(b.ray_intersect(Vec3::new(1.0, 0.0, -5.0), Vec3::Z), Some(4.0));
        assert_eq!(b.ray_intersect(Vec3::new(1.0, 1.0, -5.0), Vec3::Z), Some(4.0));
        assert_eq!(b.ray_intersect(Vec3::new(1.0 + 1e-3, 0.0, -5.0), Vec3::Z), None);
    }

    #[test]
    fn empty_union_identity() {
        let b = Aabb::from_half_extents(Vec3::ONE);
        assert_eq!(Aabb::EMPTY.union(b), b);
        assert!(Aabb::EMPTY.is_empty());
    }
}
