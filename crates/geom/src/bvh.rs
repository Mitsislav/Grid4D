//! A bounding volume hierarchy over a mesh's triangles, for ray casting (picking).

use glam::Vec3;

use crate::{PolyMesh, Triangulator, u32_index, us};

/// Where a ray hit a mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// Ray parameter of the hit, in units of the ray's direction.
    pub t: f32,
    /// The polygon that was hit.
    pub face: u32,
}

/// Triangles per leaf, at most.
const LEAF_SIZE: usize = 4;
/// Room on the traversal stack: a path visits at most its depth + 1 nodes
/// at once (the nearer child first, the other waiting).
const MAX_DEPTH: usize = 96;
/// Past this depth the surface-area build splits at the median, which
/// halves every node: never deeper than this + 33 for a `u32`-indexed mesh.
const SAH_DEPTH: usize = 56;
/// Candidate split planes per axis of the surface-area build, + 1.
const BINS: usize = 16;

/// How [`Bvh::build_with`] splits each node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BvhSplit {
    /// At the median of the triangles along the longest axis: the fastest
    /// to build (picking builds on first use).
    #[default]
    Median,
    /// Where the surface-area heuristic says rays will visit the fewest
    /// boxes (binned, 16 planes per axis): slower to build, faster rays
    /// (the final renderer).
    SurfaceArea,
}

#[derive(Clone, Copy, Debug, Default)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// A leaf's first triangle, or an inner node's first child (the second follows it).
    first: u32,
    /// Triangles in a leaf; `0` marks an inner node.
    count: u32,
}

/// A triangle while the hierarchy is built: its box and its index.
#[derive(Clone, Copy)]
struct Item {
    min: Vec3,
    max: Vec3,
    triangle: u32,
}

impl Item {
    /// Twice the box's centre (only compared).
    fn centroid(&self) -> Vec3 {
        self.min + self.max
    }
}

/// Bounding volume hierarchy over the triangles of a [`PolyMesh`].
///
/// Built once per mesh (in `O(n log n)`, splitting each node at the median of
/// its triangles along the longest axis); a ray then visits only the boxes it
/// crosses. Stores point indices, not positions: pass the same mesh's points to
/// [`raycast`](Self::raycast).
#[derive(Clone, Debug, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    /// Point indices of each triangle, in leaf order.
    triangles: Vec<[u32; 3]>,
    /// The polygon each triangle (in leaf order) came from.
    faces: Vec<u32>,
}

impl Bvh {
    /// Builds the hierarchy over `mesh`'s triangles (n-gons triangulated like the
    /// GPU mesh, so hits match what is drawn).
    #[must_use]
    pub fn build(mesh: &PolyMesh) -> Self {
        Self::build_swept(mesh, &[])
    }

    /// Builds the hierarchy over `mesh`'s triangles moving through the
    /// point positions `moments` (each as many points as the mesh's): every
    /// box covers its triangle at the mesh's own points and at every moment,
    /// so a ray at any time in between, against points blended between two
    /// moments, finds what it meets ([`raycast_by`](Self::raycast_by)).
    #[must_use]
    pub fn build_swept(mesh: &PolyMesh, moments: &[&[Vec3]]) -> Self {
        Self::build_with(mesh, moments, BvhSplit::Median)
    }

    /// [`build_swept`](Self::build_swept), splitting nodes by `split`.
    /// Rays find the same nearest hit whichever way it was built (ties go to
    /// the lower polygon index).
    #[must_use]
    pub fn build_with(mesh: &PolyMesh, moments: &[&[Vec3]], split: BvhSplit) -> Self {
        let (mut triangles, mut faces) = (Vec::new(), Vec::new());
        Triangulator::new().triangulate_points(mesh, &mut triangles, &mut faces);
        let points = mesh.points();
        // Each triangle's box travels with it, so splitting reads memory in
        // order instead of jumping through an index.
        let mut items: Vec<Item> = triangles
            .iter()
            .enumerate()
            .map(|(k, t)| {
                let [a, b, c] = t.map(|p| points[us(p)]);
                let (mut min, mut max) = (a.min(b).min(c), a.max(b).max(c));
                for moment in moments {
                    for p in t.map(|p| moment.get(us(p)).copied().unwrap_or(points[us(p)])) {
                        min = min.min(p);
                        max = max.max(p);
                    }
                }
                Item {
                    min,
                    max,
                    triangle: u32_index(k),
                }
            })
            .collect();
        let mut nodes = Vec::new();
        if !items.is_empty() {
            nodes.reserve(2 * items.len() / LEAF_SIZE + 1);
            nodes.push(Node::default());
        }
        let mut pending = vec![(0usize, 0usize, items.len(), 0usize)];
        while let Some((node, start, end, depth)) = pending.pop() {
            if nodes.is_empty() {
                break;
            }
            let span = &mut items[start..end];
            let (mut min, mut max) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
            let (mut cmin, mut cmax) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
            for item in span.iter() {
                min = min.min(item.min);
                max = max.max(item.max);
                let centroid = item.centroid();
                cmin = cmin.min(centroid);
                cmax = cmax.max(centroid);
            }
            nodes[node].min = min;
            nodes[node].max = max;
            let extent = cmax - cmin;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            // Small enough, or every centroid in one spot (no split separates them).
            if span.len() <= LEAF_SIZE || extent[axis] <= 0.0 {
                nodes[node].first = u32_index(start);
                nodes[node].count = u32_index(span.len());
                continue;
            }
            let sah = (split == BvhSplit::SurfaceArea && depth < SAH_DEPTH)
                .then(|| surface_area_split(span, cmin, cmax))
                .flatten();
            let half = sah.unwrap_or_else(|| {
                let half = span.len() / 2;
                span.select_nth_unstable_by(half, |a, b| a.centroid()[axis].total_cmp(&b.centroid()[axis]));
                half
            });
            let left = nodes.len();
            nodes.push(Node::default());
            nodes.push(Node::default());
            nodes[node].first = u32_index(left);
            pending.push((left, start, start + half, depth + 1));
            pending.push((left + 1, start + half, end, depth + 1));
        }
        Self {
            nodes,
            triangles: items.iter().map(|item| triangles[us(item.triangle)]).collect(),
            faces: items.iter().map(|item| faces[us(item.triangle)]).collect(),
        }
    }

    /// Number of triangles in the hierarchy.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// The closest hit of the ray `origin + t · dir` (`t ≥ 0`) with a triangle,
    /// from either side. `points` must be the points of the mesh this was built
    /// from. Allocation-free.
    #[must_use]
    pub fn raycast(&self, points: &[Vec3], origin: Vec3, dir: Vec3) -> Option<RayHit> {
        self.raycast_where(points, origin, dir, |_| true)
    }

    /// Like [`raycast`](Self::raycast), but only polygons for which `accept`
    /// returns `true` can be hit (others are transparent, e.g. hidden ones).
    #[must_use]
    pub fn raycast_where(
        &self,
        points: &[Vec3],
        origin: Vec3,
        dir: Vec3,
        accept: impl Fn(u32) -> bool,
    ) -> Option<RayHit> {
        self.raycast_by(|p| points.get(us(p)).copied().unwrap_or(Vec3::NAN), origin, dir, accept)
    }

    /// Like [`raycast_where`](Self::raycast_where), with each point's position
    /// given by `point` (by its index): points that move, within the boxes
    /// [`build_swept`](Self::build_swept) made.
    #[must_use]
    pub fn raycast_by(
        &self,
        point: impl Fn(u32) -> Vec3,
        origin: Vec3,
        dir: Vec3,
        accept: impl Fn(u32) -> bool,
    ) -> Option<RayHit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = dir.recip();
        let mut best: Option<RayHit> = None;
        let root = self.nodes[0];
        let entry = box_entry(root.min, root.max, origin, inv, f32::INFINITY)?;
        // Nodes waiting, with where the ray enters their box: the nearer
        // child is visited first, so hits found early cut the rest short.
        let mut stack = [(0u32, 0.0f32); MAX_DEPTH];
        stack[0] = (0, entry);
        let mut len = 1;
        while len > 0 {
            len -= 1;
            let (index, entry) = stack[len];
            // A little past the best hit: where the ray enters a box is
            // rounded too, and a triangle on the box's face may be nearer
            // by an ulp.
            let limit = best.map_or(f32::INFINITY, |hit| hit.t * (1.0 + 1e-5) + 1e-12);
            if entry > limit {
                continue;
            }
            let node = self.nodes[us(index)];
            if node.count > 0 {
                let range = us(node.first)..us(node.first) + us(node.count);
                for (triangle, &face) in self.triangles[range.clone()].iter().zip(&self.faces[range]) {
                    if !accept(face) {
                        continue;
                    }
                    let [a, b, c] = triangle.map(&point);
                    // Equal distances go to the lower polygon, whatever the
                    // order the boxes were visited in.
                    if let Some(t) = ray_triangle(origin, dir, a, b, c)
                        && best.is_none_or(|hit| t < hit.t || (t == hit.t && face < hit.face))
                    {
                        best = Some(RayHit { t, face });
                    }
                }
            } else if len + 2 <= MAX_DEPTH {
                let (a, b) = (node.first, node.first + 1);
                let (na, nb) = (self.nodes[us(a)], self.nodes[us(b)]);
                let ea = box_entry(na.min, na.max, origin, inv, limit);
                let eb = box_entry(nb.min, nb.max, origin, inv, limit);
                let mut push = |node: u32, entry: f32| {
                    stack[len] = (node, entry);
                    len += 1;
                };
                match (ea, eb) {
                    (Some(ea), Some(eb)) if ea <= eb => {
                        push(b, eb);
                        push(a, ea);
                    }
                    (Some(ea), Some(eb)) => {
                        push(a, ea);
                        push(b, eb);
                    }
                    (Some(ea), None) => push(a, ea),
                    (None, Some(eb)) => push(b, eb),
                    (None, None) => {}
                }
            }
        }
        best
    }
}

/// Where a binned surface-area split divides `span` (whose centroids lie in
/// `cmin`…`cmax`): sorts it into the two sides and returns the left side's
/// length, or `None` when no plane separates the triangles.
fn surface_area_split(span: &mut [Item], cmin: Vec3, cmax: Vec3) -> Option<usize> {
    #[derive(Clone, Copy)]
    struct Bin {
        min: Vec3,
        max: Vec3,
        count: u32,
    }
    const EMPTY: Bin = Bin {
        min: Vec3::INFINITY,
        max: Vec3::NEG_INFINITY,
        count: 0,
    };
    fn area(min: Vec3, max: Vec3) -> f32 {
        let d = (max - min).max(Vec3::ZERO);
        d.x * d.y + d.y * d.z + d.z * d.x
    }
    let extent = cmax - cmin;
    let bin_of = |axis: usize, centroid: Vec3| -> usize {
        let scale = BINS as f32 / extent[axis];
        // Non-negative (centroids lie in the bounds) and capped.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin index")]
        let bin = ((centroid[axis] - cmin[axis]) * scale) as usize;
        bin.min(BINS - 1)
    };
    // (cost, axis, last bin on the left)
    let mut best: Option<(f32, usize, usize)> = None;
    for axis in 0..3 {
        // Flat (or not a number): no plane along this axis.
        if extent[axis].is_nan() || extent[axis] <= 0.0 {
            continue;
        }
        let mut bins = [EMPTY; BINS];
        for item in span.iter() {
            let bin = &mut bins[bin_of(axis, item.centroid())];
            bin.min = bin.min.min(item.min);
            bin.max = bin.max.max(item.max);
            bin.count += 1;
        }
        // Left of plane i: bins 0..=i; right: the rest.
        let mut right = [(0.0f32, 0u32); BINS];
        let mut acc = EMPTY;
        for i in (1..BINS).rev() {
            acc.min = acc.min.min(bins[i].min);
            acc.max = acc.max.max(bins[i].max);
            acc.count += bins[i].count;
            right[i - 1] = (area(acc.min, acc.max), acc.count);
        }
        let mut acc = EMPTY;
        for (i, bin) in bins.iter().enumerate().take(BINS - 1) {
            acc.min = acc.min.min(bin.min);
            acc.max = acc.max.max(bin.max);
            acc.count += bin.count;
            let (right_area, right_count) = right[i];
            if acc.count == 0 || right_count == 0 {
                continue;
            }
            let cost = area(acc.min, acc.max) * f32::from(u16::try_from(acc.count).unwrap_or(u16::MAX))
                + right_area * f32::from(u16::try_from(right_count).unwrap_or(u16::MAX));
            if best.is_none_or(|(c, _, _)| cost < c) {
                best = Some((cost, axis, i));
            }
        }
    }
    let (_, axis, plane) = best?;
    // Partition: bins up to `plane` to the front.
    let (mut i, mut j) = (0, span.len());
    while i < j {
        if bin_of(axis, span[i].centroid()) <= plane {
            i += 1;
        } else {
            j -= 1;
            span.swap(i, j);
        }
    }
    (i > 0 && i < span.len()).then_some(i)
}

/// Slab test: where the ray enters the box (boundary included; 0 from
/// inside), if it does before `limit`.
fn box_entry(min: Vec3, max: Vec3, origin: Vec3, inv_dir: Vec3, limit: f32) -> Option<f32> {
    let t0 = (min - origin) * inv_dir;
    let t1 = (max - origin) * inv_dir;
    // NaN (0 · ∞): parallel to that slab and starting on its boundary, i.e. inside it.
    let on_face = t0.is_nan_mask() | t1.is_nan_mask();
    let near = Vec3::select(on_face, Vec3::NEG_INFINITY, t0.min(t1))
        .max_element()
        .max(0.0);
    let far = Vec3::select(on_face, Vec3::INFINITY, t0.max(t1))
        .min_element()
        .min(limit);
    // A ray through a corner or along a face of the box may find its near
    // and far ends an ulp apart the wrong way round: still a hit.
    (near <= far + far.abs() * 1e-5 + 1e-12).then_some(near)
}

/// How far outside a triangle (in barycentric units) still counts as inside, so a
/// ray through a shared edge or corner never slips between the two triangles.
const EDGE_TOLERANCE: f32 = 1e-5;

/// Möller–Trumbore, two-sided: the ray parameter of the hit, if any.
fn ray_triangle(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < f32::MIN_POSITIVE || !det.is_finite() {
        return None; // parallel to the triangle, or a degenerate one
    }
    let inv_det = det.recip();
    let s = origin - a;
    let u = s.dot(p) * inv_det;
    if !(-EDGE_TOLERANCE..=1.0 + EDGE_TOLERANCE).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv_det;
    if v < -EDGE_TOLERANCE || u + v > 1.0 + EDGE_TOLERANCE {
        return None;
    }
    let t = e2.dot(q) * inv_det;
    (t >= 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{primitives, shapes};

    /// Every triangle, no hierarchy: the reference the BVH must agree with.
    fn brute_force(mesh: &PolyMesh, origin: Vec3, dir: Vec3) -> Option<f32> {
        let (mut triangles, mut faces) = (Vec::new(), Vec::new());
        Triangulator::new().triangulate_points(mesh, &mut triangles, &mut faces);
        triangles
            .iter()
            .filter_map(|t| {
                let [a, b, c] = t.map(|p| mesh.points()[us(p)]);
                ray_triangle(origin, dir, a, b, c)
            })
            .min_by(f32::total_cmp)
    }

    /// A deterministic stream of numbers in `[-1, 1)`.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let bits = u32::try_from(self.0 >> 40).unwrap(); // 24 bits
            f32::from(u16::try_from(bits >> 8).unwrap()) / 32_768.0 - 1.0
        }

        fn vec3(&mut self) -> Vec3 {
            Vec3::new(self.next(), self.next(), self.next())
        }
    }

    #[test]
    fn rays_agree_with_brute_force() {
        let meshes = [
            ("sphere", primitives::sphere(1.0, 48)),
            ("torus", shapes::torus(24, 12)),
            ("cylinder", shapes::cylinder(9)),
            ("star", shapes::star()),
        ];
        let mut rng = Lcg(7);
        for (name, mesh) in meshes {
            let bvh = Bvh::build(&mesh);
            let mut hits = 0;
            for _ in 0..500 {
                let origin = rng.vec3() * 4.0;
                let target = rng.vec3() * 0.8;
                let dir = target - origin;
                let expected = brute_force(&mesh, origin, dir);
                let got = bvh.raycast(mesh.points(), origin, dir);
                assert_eq!(got.map(|h| h.t), expected, "{name}: ray from {origin} to {target}");
                hits += usize::from(got.is_some());
            }
            assert!(
                hits > 50,
                "{name}: only {hits} of 500 rays hit — test rays miss the mesh"
            );
        }
    }

    #[test]
    fn both_builds_find_the_same_hits() {
        let meshes = [
            ("sphere", primitives::sphere(1.0, 48)),
            ("torus", shapes::torus(24, 12)),
            ("cylinder", shapes::cylinder(9)),
            ("star", shapes::star()),
            ("l", shapes::l_shape()),
        ];
        let mut rng = Lcg(11);
        for (name, mesh) in meshes {
            let median = Bvh::build(&mesh);
            let sah = Bvh::build_with(&mesh, &[], BvhSplit::SurfaceArea);
            assert_eq!(sah.triangle_count(), median.triangle_count());
            for _ in 0..500 {
                let origin = rng.vec3() * 4.0;
                let dir = rng.vec3() * 0.8 - origin;
                let expected = median.raycast(mesh.points(), origin, dir);
                assert_eq!(sah.raycast(mesh.points(), origin, dir), expected, "{name}");
                assert_eq!(expected.map(|h| h.t), brute_force(&mesh, origin, dir), "{name}");
            }
            // Rays through the shared corners and edges of the polygons:
            // ties go to the lower polygon in both.
            for p in mesh.points() {
                let origin = *p * 3.0 + Vec3::new(0.01, 0.02, 0.03);
                let dir = *p - origin;
                assert_eq!(
                    sah.raycast(mesh.points(), origin, dir),
                    median.raycast(mesh.points(), origin, dir),
                    "{name}: through {p}"
                );
            }
        }
    }

    #[test]
    fn the_surface_area_build_holds_every_triangle_once() {
        // Uneven sizes: a large plane under many small spheres' worth.
        let mut mesh = primitives::plane(500.0, 500.0, 1, 1);
        let sphere = primitives::sphere(1.0, 64);
        let offset = u32_index(mesh.point_count());
        for &p in sphere.points() {
            mesh.add_point(p + Vec3::new(30.0, 2.0, -40.0)).unwrap();
        }
        for face in sphere.faces() {
            let corners: Vec<u32> = face.iter().map(|&c| c + offset).collect();
            mesh.add_face(&corners).unwrap();
        }
        let bvh = Bvh::build_with(&mesh, &[], BvhSplit::SurfaceArea);
        let expected: usize = mesh.faces().map(|f| f.len() - 2).sum();
        assert_eq!(bvh.triangle_count(), expected);
        let mut seen = vec![0u32; expected];
        let mut stack = vec![(0usize, 1usize)];
        while let Some((node, depth)) = stack.pop() {
            assert!(depth < MAX_DEPTH);
            let n = bvh.nodes[node];
            if n.count > 0 {
                assert!(us(n.count) <= LEAF_SIZE);
                for count in &mut seen[us(n.first)..us(n.first + n.count)] {
                    *count += 1;
                }
            } else {
                stack.push((us(n.first), depth + 1));
                stack.push((us(n.first) + 1, depth + 1));
            }
        }
        assert!(seen.iter().all(|&count| count == 1));
    }

    #[test]
    fn hits_report_the_polygon_and_distance() {
        let cube = shapes::cube(2.0);
        let bvh = Bvh::build(&cube);
        let hit = bvh.raycast(cube.points(), Vec3::new(0.2, 0.3, 5.0), -Vec3::Z).unwrap();
        assert!((hit.t - 4.0).abs() < 1e-6, "front face at z = 1");
        assert!(cube.face_normal(us(hit.face)).abs_diff_eq(Vec3::Z, 1e-6));
        // From inside, the far wall is hit (two-sided); pointing away from a face misses it.
        let inside = bvh.raycast(cube.points(), Vec3::ZERO, Vec3::X).unwrap();
        assert!((inside.t - 1.0).abs() < 1e-6);
        assert_eq!(bvh.raycast(cube.points(), Vec3::new(0.0, 0.0, 5.0), Vec3::Z), None);
    }

    #[test]
    fn rays_along_edges_and_through_corners_never_slip_through() {
        // A 10 × 10 grid of quads: every ray below runs exactly through edges.
        let grid = primitives::plane(10.0, 10.0, 10, 10);
        let bvh = Bvh::build(&grid);
        for i in 0..=40u8 {
            for j in 0..=40u8 {
                let (x, z) = (f32::from(i) * 0.25 - 5.0, f32::from(j) * 0.25 - 5.0);
                let hit = bvh.raycast(grid.points(), Vec3::new(x, 3.0, z), -Vec3::Y);
                assert!(
                    hit.is_some_and(|h| (h.t - 3.0).abs() < 1e-5),
                    "slipped through at ({x}, {z})"
                );
            }
        }
    }

    #[test]
    fn filtered_faces_are_transparent() {
        let cube = shapes::cube(2.0);
        let bvh = Bvh::build(&cube);
        let front = bvh.raycast(cube.points(), Vec3::new(0.0, 0.0, 5.0), -Vec3::Z).unwrap();
        let behind = bvh
            .raycast_where(cube.points(), Vec3::new(0.0, 0.0, 5.0), -Vec3::Z, |f| f != front.face)
            .unwrap();
        assert!(
            (behind.t - 6.0).abs() < 1e-6,
            "through the skipped front face to the back"
        );
    }

    #[test]
    fn concave_polygons_are_hit_only_where_they_are() {
        // The L covers three of the four unit squares of [0, 2]²; [1, 2]² is the notch.
        let l = shapes::l_shape();
        let bvh = Bvh::build(&l);
        let down = |x: f32, y: f32| bvh.raycast(l.points(), Vec3::new(x, y, 1.0), -Vec3::Z);
        assert!(down(0.5, 0.5).is_some());
        assert!(down(1.5, 0.5).is_some());
        assert!(down(0.5, 1.5).is_some());
        assert_eq!(down(1.5, 1.5), None, "the notch is empty");
    }

    #[test]
    fn the_hierarchy_holds_every_triangle_once_in_shallow_leaves() {
        let mesh = primitives::sphere(1.0, 64);
        let bvh = Bvh::build(&mesh);
        let expected: usize = mesh.faces().map(|f| f.len() - 2).sum();
        assert_eq!(bvh.triangle_count(), expected);
        let mut seen = vec![0u32; expected];
        let mut stack = vec![(0usize, 1usize)];
        let mut max_depth = 0;
        while let Some((node, depth)) = stack.pop() {
            max_depth = max_depth.max(depth);
            let n = bvh.nodes[node];
            if n.count > 0 {
                assert!(us(n.count) <= LEAF_SIZE);
                for count in &mut seen[us(n.first)..us(n.first + n.count)] {
                    *count += 1;
                }
            } else {
                stack.push((us(n.first), depth + 1));
                stack.push((us(n.first) + 1, depth + 1));
            }
        }
        assert!(
            seen.iter().all(|&count| count == 1),
            "every triangle in exactly one leaf"
        );
        assert!(max_depth <= 14, "{expected} triangles at depth {max_depth}");
    }

    #[test]
    fn empty_and_degenerate_meshes_are_safe() {
        let empty = PolyMesh::new();
        assert_eq!(Bvh::build(&empty).raycast(empty.points(), Vec3::ZERO, Vec3::X), None);
        // Every triangle in the same spot: one leaf, no endless splitting.
        let points = vec![Vec3::ZERO; 3];
        let flat = PolyMesh::from_faces(points, [[0u32, 1, 2]; 1]).unwrap();
        let bvh = Bvh::build(&flat);
        assert_eq!(bvh.raycast(flat.points(), Vec3::new(0.0, 0.0, 1.0), -Vec3::Z), None);
    }

    #[test]
    fn swept_boxes_find_moving_triangles_anywhere_on_their_way() {
        let mut mesh = PolyMesh::new();
        for p in [Vec3::ZERO, Vec3::X, Vec3::Y] {
            mesh.add_point(p).unwrap();
        }
        mesh.add_face(&[0, 1, 2]).unwrap();
        // It moves 10 along +X.
        let later: Vec<Vec3> = mesh.points().iter().map(|p| *p + Vec3::new(10.0, 0.0, 0.0)).collect();
        let bvh = Bvh::build_swept(&mesh, &[mesh.points(), &later]);
        for t in [0.0f32, 0.3, 1.0] {
            let at = |p: u32| mesh.points()[us(p)].lerp(later[us(p)], t);
            let origin = Vec3::new(0.2 + 10.0 * t, 0.2, 5.0);
            let hit = bvh.raycast_by(at, origin, Vec3::NEG_Z, |_| true);
            assert!(hit.is_some_and(|h| (h.t - 5.0).abs() < 1e-4), "{t}");
        }
        // Built without moments: the plain hierarchy.
        assert!(
            Bvh::build(&mesh)
                .raycast(mesh.points(), Vec3::new(5.2, 0.2, 5.0), Vec3::NEG_Z)
                .is_none()
        );
    }
}
