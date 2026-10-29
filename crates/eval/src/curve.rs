//! Curves: splines as viewers see them — sampled into polylines once per
//! source, cached per node, and named by a [`CurveId`] the way polygon objects'
//! meshes are named by their `MeshId`.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use grid4d_geom::spline::{Interpolation, Polyline, SplinePrimitive};
use grid4d_scene::SplineId;

/// Names one sampled curve; a changed spline gets a new id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CurveId(u64);

impl CurveId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// A sampled curve, shared: clones are cheap.
#[derive(Clone, Debug)]
pub struct CurveHandle {
    id: CurveId,
    polyline: Arc<Polyline>,
}

impl CurveHandle {
    /// A new curve with a fresh id.
    #[must_use]
    pub fn new(polyline: Polyline) -> Self {
        Self {
            id: CurveId::next(),
            polyline: Arc::new(polyline),
        }
    }

    #[must_use]
    pub fn id(&self) -> CurveId {
        self.id
    }

    /// The curve's points per segment, in object space.
    #[must_use]
    pub fn polyline(&self) -> &Polyline {
        &self.polyline
    }
}

impl PartialEq for CurveHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// What a curve is sampled from: an unchanged source means an unchanged curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CurveSource {
    Primitive(SplinePrimitive),
    /// An editable spline's content and sampling.
    Editable(SplineId, Interpolation),
}

/// A node's sampled curve and what it was sampled from.
#[derive(Debug)]
pub(crate) struct CachedCurve {
    source: CurveSource,
    pub(crate) handle: CurveHandle,
    /// The scene revision of the last update that used it.
    pub(crate) seen: u64,
}

impl CachedCurve {
    pub(crate) fn new(source: CurveSource, polyline: Polyline, seen: u64) -> Self {
        Self {
            handle: CurveHandle::new(polyline),
            source,
            seen,
        }
    }

    /// Keeps the curve if `source` is unchanged, else samples it again with `sample`.
    pub(crate) fn refresh(&mut self, source: CurveSource, sample: impl FnOnce() -> Polyline, seen: u64) {
        if self.source != source {
            *self = Self::new(source, sample(), seen);
        }
        self.seen = seen;
    }
}
