//! Soft selection: a weight for every point from a selection, so the points
//! around it follow a transform less the farther they are (spec
//! `docs/reference/modeling-soft-selection.md`).
//!
//! A selected point weighs 1; another weighs the falloff of its distance to
//! the nearest selected point, 0 at the radius and beyond. Distance is
//! measured straight through space (by a grid of cells as wide as the
//! radius) or along the mesh's edges (shortest paths, Dijkstra).

use std::{cmp::Ordering, collections::BinaryHeap, collections::HashMap};

use glam::{IVec3, Vec3};

use crate::{BitSet, Topology, u32_index, us};

/// How a weight falls from 1 at the selection to 0 at the radius.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Falloff {
    /// Straight.
    Linear,
    /// Eased at both ends.
    #[default]
    Smooth,
    /// Round near the selection, steep at the radius.
    Dome,
    /// Dropping quickly near the selection.
    Sharp,
}

impl Falloff {
    pub const ALL: [Self; 4] = [Self::Linear, Self::Smooth, Self::Dome, Self::Sharp];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Smooth => "Smooth",
            Self::Dome => "Dome",
            Self::Sharp => "Sharp",
        }
    }

    /// The weight at `t` = distance / radius: 1 at 0, 0 at 1 and beyond,
    /// never rising.
    #[must_use]
    pub fn weight(self, t: f32) -> f32 {
        let t = if t.is_nan() { 1.0 } else { t.clamp(0.0, 1.0) };
        match self {
            Self::Linear => 1.0 - t,
            Self::Smooth => 1.0 - t * t * (3.0 - 2.0 * t),
            Self::Dome => (1.0 - t * t).max(0.0).sqrt(),
            Self::Sharp => (1.0 - t) * (1.0 - t),
        }
    }
}

/// How the distance to the selection is measured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SoftDistance {
    /// Straight through space.
    #[default]
    Volume,
    /// Along the mesh's edges: the other side of a thin part stays.
    Surface,
}

impl SoftDistance {
    pub const ALL: [Self; 2] = [Self::Volume, Self::Surface];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Surface => "Surface",
        }
    }
}

/// The soft-selection options (editor state, never in a document).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoftSelection {
    pub enabled: bool,
    /// How far from the selection points still follow (> 0).
    pub radius: f32,
    pub falloff: Falloff,
    pub distance: SoftDistance,
}

impl SoftSelection {
    /// The smallest radius accepted (cm).
    pub const MIN_RADIUS: f32 = 0.01;
}

impl Default for SoftSelection {
    /// Off; 50 cm, Smooth, through space.
    fn default() -> Self {
        Self {
            enabled: false,
            radius: 50.0,
            falloff: Falloff::Smooth,
            distance: SoftDistance::Volume,
        }
    }
}

/// Reused space for [`soft_weights`].
#[derive(Debug, Default)]
pub struct SoftScratch {
    cells: HashMap<IVec3, Vec<u32>>,
    distance: Vec<f32>,
    heap: BinaryHeap<Reach>,
}

/// A point reached at a distance, in Dijkstra's heap (nearest first).
#[derive(Clone, Copy, Debug)]
struct Reach {
    distance: f32,
    point: u32,
}

impl PartialEq for Reach {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Reach {}

impl PartialOrd for Reach {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Reach {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .distance
            .total_cmp(&self.distance)
            .then_with(|| other.point.cmp(&self.point))
    }
}

/// Fills `out` with a weight per point of `points` (see the module docs):
/// `selected` and `hidden` by point; `topology` gives the edges Surface
/// distance follows (without it, distance is through space). Hidden points
/// weigh 0 and are never passed through.
pub fn soft_weights(
    points: &[Vec3],
    selected: &BitSet,
    hidden: &BitSet,
    topology: Option<&Topology>,
    settings: &SoftSelection,
    scratch: &mut SoftScratch,
    out: &mut Vec<f32>,
) {
    out.clear();
    out.resize(points.len(), 0.0);
    let radius = settings.radius.max(SoftSelection::MIN_RADIUS);
    for p in selected.iter().filter(|&p| p < points.len() && !hidden.contains(p)) {
        out[p] = 1.0;
    }
    match (settings.distance, topology) {
        (SoftDistance::Surface, Some(topology)) => surface(points, selected, hidden, topology, radius, scratch),
        _ => volume(points, selected, hidden, radius, scratch),
    }
    for (p, weight) in out.iter_mut().enumerate() {
        if *weight < 1.0 && !hidden.contains(p) {
            *weight = settings.falloff.weight(scratch.distance[p] / radius);
        }
    }
}

/// Fills `scratch.distance` with each point's straight distance to the
/// nearest selected point (infinite beyond the radius).
fn volume(points: &[Vec3], selected: &BitSet, hidden: &BitSet, radius: f32, scratch: &mut SoftScratch) {
    scratch.distance.clear();
    scratch.distance.resize(points.len(), f32::INFINITY);
    for list in scratch.cells.values_mut() {
        list.clear();
    }
    let cell = |p: Vec3| (p / radius).floor().as_ivec3();
    for p in selected.iter().filter(|&p| p < points.len() && !hidden.contains(p)) {
        if points[p].is_finite() {
            scratch.cells.entry(cell(points[p])).or_default().push(u32_index(p));
        }
    }
    for (p, &point) in points.iter().enumerate() {
        if hidden.contains(p) || !point.is_finite() {
            continue;
        }
        let home = cell(point);
        let mut nearest = f32::INFINITY;
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    if let Some(list) = scratch.cells.get(&(home + IVec3::new(x, y, z))) {
                        for &s in list {
                            nearest = nearest.min(point.distance(points[us(s)]));
                        }
                    }
                }
            }
        }
        if nearest < radius {
            scratch.distance[p] = nearest;
        }
    }
}

/// Fills `scratch.distance` with each point's distance along the edges to
/// the nearest selected point (infinite beyond the radius or unreachable).
fn surface(
    points: &[Vec3],
    selected: &BitSet,
    hidden: &BitSet,
    topology: &Topology,
    radius: f32,
    scratch: &mut SoftScratch,
) {
    let distance = &mut scratch.distance;
    distance.clear();
    distance.resize(points.len(), f32::INFINITY);
    let heap = &mut scratch.heap;
    heap.clear();
    for p in selected.iter().filter(|&p| p < points.len() && !hidden.contains(p)) {
        distance[p] = 0.0;
        heap.push(Reach {
            distance: 0.0,
            point: u32_index(p),
        });
    }
    while let Some(Reach { distance: d, point }) = heap.pop() {
        let p = us(point);
        if d > distance[p] {
            continue;
        }
        for &e in topology.point_edges(p) {
            let Some(&[a, b]) = topology.edges().get(us(e)) else {
                continue;
            };
            let q = us(if us(a) == p { b } else { a });
            if q >= points.len() || hidden.contains(q) {
                continue;
            }
            let next = d + points[p].distance(points[q]);
            if next < distance[q] && next < radius {
                distance[q] = next;
                heap.push(Reach {
                    distance: next,
                    point: u32_index(q),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PolyMesh, primitives};

    fn weights(mesh: &PolyMesh, selected: &[usize], settings: SoftSelection) -> Vec<f32> {
        let selected: BitSet = selected.iter().copied().collect();
        let topology = Topology::build(mesh);
        let mut out = Vec::new();
        soft_weights(
            mesh.points(),
            &selected,
            &BitSet::new(),
            Some(&topology),
            &settings,
            &mut SoftScratch::default(),
            &mut out,
        );
        out
    }

    #[test]
    fn every_falloff_falls_from_one_to_zero() {
        for falloff in Falloff::ALL {
            assert_eq!(falloff.weight(0.0), 1.0, "{falloff:?}");
            assert_eq!(falloff.weight(1.0), 0.0, "{falloff:?}");
            assert_eq!(falloff.weight(2.0), 0.0);
            let mut last = 1.0;
            for k in 1..=100 {
                let w = falloff.weight(k as f32 / 100.0);
                assert!(w <= last && (0.0..=1.0).contains(&w), "{falloff:?} at {k}");
                last = w;
            }
        }
    }

    #[test]
    fn points_near_the_selection_follow_by_the_falloff() {
        // A 200 cm plane of 20 × 20: the centre point selected, radius 50.
        let plane = primitives::plane(200.0, 200.0, 20, 20);
        let centre = plane.points().iter().position(|p| p.length() < 1e-3).unwrap();
        let settings = SoftSelection {
            enabled: true,
            falloff: Falloff::Linear,
            ..SoftSelection::default()
        };
        let w = weights(&plane, &[centre], settings);
        for (p, point) in plane.points().iter().enumerate() {
            let d = point.length();
            let expected = (1.0 - d / 50.0).max(0.0);
            assert!((w[p] - expected).abs() < 1e-5, "{point} {} {expected}", w[p]);
        }
        assert_eq!(w[centre], 1.0);
    }

    #[test]
    fn surface_distance_leaves_the_far_layer() {
        // Two parallel strips 10 cm apart (a folded strip): the top one's
        // middle point selected, radius 30.
        let mut strip = primitives::plane(100.0, 20.0, 10, 2);
        let count = strip.point_count();
        let top: Vec<Vec3> = strip.points().iter().map(|p| *p + Vec3::Y * 10.0).collect();
        let faces: Vec<Vec<u32>> = strip
            .faces()
            .map(|f| f.to_vec())
            .chain(strip.faces().map(|f| f.iter().map(|&c| c + u32_index(count)).collect()))
            .collect();
        let mut points = strip.points().to_vec();
        points.extend(top);
        strip = PolyMesh::from_faces(points, faces).unwrap();
        let middle = (count..strip.point_count())
            .find(|&p| (strip.points()[p] - Vec3::Y * 10.0).length() < 1e-3)
            .unwrap();
        let mut settings = SoftSelection {
            enabled: true,
            radius: 30.0,
            ..SoftSelection::default()
        };
        settings.distance = SoftDistance::Volume;
        let through = weights(&strip, &[middle], settings);
        assert!(
            through[..count].iter().any(|&w| w > 0.0),
            "through space the layer below follows"
        );
        settings.distance = SoftDistance::Surface;
        let along = weights(&strip, &[middle], settings);
        assert!(
            along[..count].iter().all(|&w| w == 0.0),
            "along the surface it does not"
        );
        assert!(along[count..].iter().filter(|&&w| w > 0.0).count() > 1);
    }

    #[test]
    fn hidden_points_weigh_nothing_and_block_nothing_else() {
        let plane = primitives::plane(100.0, 100.0, 10, 10);
        let selected: BitSet = [0].into_iter().collect();
        let hidden: BitSet = [1].into_iter().collect();
        let mut out = Vec::new();
        let settings = SoftSelection {
            enabled: true,
            ..SoftSelection::default()
        };
        soft_weights(
            plane.points(),
            &selected,
            &hidden,
            None,
            &settings,
            &mut SoftScratch::default(),
            &mut out,
        );
        assert_eq!(out[1], 0.0);
        assert_eq!(out[0], 1.0);
        assert!(out[2] > 0.0);
    }
}
