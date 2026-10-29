//! Shared, immutable splines — the curves of editable spline objects.

use std::{fmt, sync::Arc};

use glam::Vec3;
use grid4d_geom::spline::{Spline, SplineType};

/// Identifies one version of a spline's content, unique like [`MeshId`](crate::MeshId),
/// so caches keyed by it (sampled curves) never mix up two splines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SplineId(u64);

/// The curve of an editable spline object: immutable and cheap to clone. An
/// edit makes a new handle with a new [`SplineId`]; undo steps keep the old one.
#[derive(Clone)]
pub struct SplineHandle(Arc<Shared>);

struct Shared {
    id: SplineId,
    spline: Spline,
}

impl SplineHandle {
    #[must_use]
    pub fn new(spline: Spline) -> Self {
        Self(Arc::new(Shared {
            id: SplineId(crate::scene::next_revision()),
            spline,
        }))
    }

    #[must_use]
    pub fn id(&self) -> SplineId {
        self.0.id
    }

    #[must_use]
    pub fn spline(&self) -> &Spline {
        &self.0.spline
    }

    /// The same spline with its points and tangents replaced (one entry per
    /// point; extra entries are ignored, missing ones stay): a new handle.
    #[must_use]
    pub fn with_points(&self, points: &[Vec3], tangents: &[[Vec3; 2]]) -> Self {
        let mut spline = self.0.spline.clone();
        for (point, &new) in spline.points.iter_mut().zip(points) {
            *point = new;
        }
        for (tangent, &new) in spline.tangents.iter_mut().zip(tangents) {
            *tangent = new;
        }
        Self::new(spline)
    }

    /// The same spline changed by `change`: a new handle.
    #[must_use]
    pub fn edited(&self, change: impl FnOnce(&mut Spline)) -> Self {
        let mut spline = self.0.spline.clone();
        change(&mut spline);
        Self::new(spline)
    }

    /// An editable spline without points.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(Spline {
            kind: SplineType::Linear,
            points: Vec::new(),
            tangents: Vec::new(),
            segments: Vec::new(),
        })
    }
}

/// Equal when they share the curve or describe the same curve.
impl PartialEq for SplineHandle {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id || self.0.spline == other.0.spline
    }
}

/// Counts instead of contents.
impl fmt::Debug for SplineHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SplineHandle")
            .field("id", &self.0.id)
            .field("points", &self.0.spline.points.len())
            .field("segments", &self.0.spline.segments.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_make_new_handles_and_clones_share() {
        let a = SplineHandle::new(Spline::new(SplineType::Linear, vec![Vec3::ZERO, Vec3::X], false));
        let b = a.clone();
        assert_eq!(a.id(), b.id());
        let moved = a.with_points(&[Vec3::Y], &[]);
        assert_ne!(moved.id(), a.id());
        assert_eq!(moved.spline().points, [Vec3::Y, Vec3::X], "missing entries stay");
        assert_eq!(a.spline().points[0], Vec3::ZERO, "the original is untouched");
        assert!(SplineHandle::empty().spline().points.is_empty());
    }
}
