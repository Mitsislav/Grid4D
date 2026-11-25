//! Reduce: a mesh made lighter by collapsing edges in the order of the error
//! each adds (Garland and Heckbert's quadric error metrics, our own code;
//! spec `docs/reference/generator-reduce.md`).
//!
//! Two stages, so a percentage can be changed interactively:
//!
//! * [`CollapseOrder::new`] triangulates the mesh and collapses edges, the
//!   cheapest first, until no collapse is allowed, recording each. A collapse
//!   moves one end of an edge onto the other (the end that adds less error),
//!   so the kept points are the input's own and keep their UVs exactly.
//! * [`CollapseOrder::replay`] applies the first collapses up to a share of
//!   the triangles: merges of point and UV numbers only, no geometry.
//!
//! Open borders and UV seams can be kept: their edges get constraint planes
//! (the paper's boundary planes), and a point on them may only slide along
//! them. A collapse that would turn a face over, pinch the surface or
//! double a face is refused.

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap},
};

use glam::{DVec3, Vec2, Vec3};

use crate::{PolyMesh, Triangulator, u32_index, us};

/// What the reduction keeps the shape of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Constraints {
    /// Open borders (edges with one face) keep their outline.
    pub borders: bool,
    /// Edges where the UVs split are kept like borders.
    pub seams: bool,
}

impl Default for Constraints {
    fn default() -> Self {
        Self {
            borders: true,
            seams: true,
        }
    }
}

/// A mesh triangulated and the order its edges collapse in.
#[derive(Clone, Debug, Default)]
pub struct CollapseOrder {
    points: Vec<Vec3>,
    /// Each wedge (a point with one UV) and its point.
    wedge_point: Vec<u32>,
    /// Each wedge's UV (empty when the mesh has none).
    wedge_uv: Vec<Vec2>,
    /// The triangulated input, by wedge.
    triangles: Vec<[u32; 3]>,
    /// Point `from` merged into point `to`, in order.
    collapses: Vec<[u32; 2]>,
    /// The triangles left after each collapse.
    remaining: Vec<u32>,
    /// Wedge merges (`from` → `to`); collapse `c`'s are
    /// `wedge_moves[move_start[c]..move_start[c + 1]]`.
    wedge_moves: Vec<[u32; 2]>,
    move_start: Vec<u32>,
}

/// A symmetric 4 × 4 error matrix: the squared distance to planes, summed.
#[derive(Clone, Copy, Debug, Default)]
struct Quadric([f64; 10]);

impl Quadric {
    /// `weight` × the squared distance to the plane through `point` with unit
    /// `normal`.
    fn plane(normal: DVec3, point: DVec3, weight: f64) -> Self {
        let (a, b, c) = (normal.x, normal.y, normal.z);
        let d = -normal.dot(point);
        Self([a * a, a * b, a * c, a * d, b * b, b * c, b * d, c * c, c * d, d * d].map(|v| v * weight))
    }

    fn add(&mut self, other: &Self) {
        for (s, o) in self.0.iter_mut().zip(other.0) {
            *s += o;
        }
    }

    fn error(&self, p: DVec3) -> f64 {
        let [aa, ab, ac, ad, bb, bc, bd, cc, cd, dd] = self.0;
        let (x, y, z) = (p.x, p.y, p.z);
        x * x * aa
            + 2.0 * x * y * ab
            + 2.0 * x * z * ac
            + 2.0 * x * ad
            + y * y * bb
            + 2.0 * y * z * bc
            + 2.0 * y * bd
            + z * z * cc
            + 2.0 * z * cd
            + dd
    }
}

/// A candidate collapse in the heap: cheapest first.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    cost: f64,
    from: u32,
    to: u32,
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate {
    /// Reversed, so the heap's top is the cheapest; ties by point numbers so
    /// the order never depends on the heap's history.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.from.cmp(&self.from))
            .then_with(|| other.to.cmp(&self.to))
    }
}

/// One neighbour of a point, as [`Work::neighbours_of`] finds it.
#[derive(Clone, Copy, Debug)]
struct Near {
    point: u32,
    /// The live triangles on the edge.
    count: u32,
    /// The first point's wedge in the first of them.
    wedge: u32,
    /// The first point's wedges differ between the edge's triangles (its
    /// UVs split along the edge).
    split: bool,
}

/// How strongly a kept edge holds its line, against the faces' error.
const CONSTRAINT_WEIGHT: f64 = 1000.0;

/// The working state while the order is computed.
struct Work<'a> {
    order: &'a mut CollapseOrder,
    constraints: Constraints,
    quadrics: Vec<Quadric>,
    /// The triangles as collapses change them (dead ones `u32::MAX`).
    triangles: Vec<[u32; 3]>,
    /// Their points (the wedges' points, kept beside them: fewer lookups).
    corners: Vec<[u32; 3]>,
    /// Each point's triangles (dead ones removed as met).
    around: Vec<Vec<u32>>,
    alive: Vec<bool>,
    heap: BinaryHeap<Candidate>,
    left: u32,
    // Scratch.
    neighbours: Vec<Near>,
    others: Vec<Near>,
    moves: Vec<[u32; 2]>,
    /// Edges no direction of which could collapse when met: tried again
    /// once the heap is empty.
    refused: Vec<[u32; 2]>,
}

impl CollapseOrder {
    /// The collapse order of `mesh` (triangulated; faces repeating a point are
    /// left out) under `constraints`.
    #[must_use]
    pub fn new(mesh: &PolyMesh, constraints: Constraints) -> Self {
        let mut order = Self {
            points: mesh.points().to_vec(),
            ..Self::default()
        };
        order.triangulate(mesh);
        if order.triangles.is_empty() || order.points.iter().any(|p| !p.is_finite()) {
            order.triangles.clear();
            return order;
        }
        let mut work = Work::new(&mut order, constraints);
        work.run();
        order
    }

    /// Fills the wedges and triangles from `mesh`.
    fn triangulate(&mut self, mesh: &PolyMesh) {
        let uvs = mesh.uvs();
        let mut wedges: HashMap<(u32, [u32; 2]), u32> = HashMap::new();
        let mut triangulator = Triangulator::new();
        let mut local = Vec::new();
        for f in 0..mesh.face_count() {
            let corners = mesh.face(f);
            let range = mesh.corner_range(f);
            local.clear();
            triangulator.face_triangles(mesh, f, &mut local);
            for &[a, b, c] in &local {
                if a == b || b == c || a == c {
                    continue;
                }
                let triangle = [a, b, c].map(|p| match uvs {
                    None => p,
                    Some(uvs) => {
                        let at = corners.iter().position(|&q| q == p).unwrap_or(0);
                        let uv = uvs.get(range.start + at).copied().unwrap_or_default();
                        *wedges.entry((p, [uv.x.to_bits(), uv.y.to_bits()])).or_insert_with(|| {
                            self.wedge_point.push(p);
                            self.wedge_uv.push(uv);
                            u32_index(self.wedge_point.len() - 1)
                        })
                    }
                });
                self.triangles.push(triangle);
            }
        }
        if uvs.is_none() {
            self.wedge_point = (0..u32_index(self.points.len())).collect();
        }
    }

    /// The number of triangles of the triangulated input.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// The number of triangles when every collapse is applied (the least the
    /// constraints allow).
    #[must_use]
    pub fn smallest(&self) -> usize {
        self.remaining.last().map_or(self.triangles.len(), |&n| us(n))
    }

    /// The mesh keeping `share` (0 – 1) of the triangles, as nearly as the
    /// recorded collapses allow (never fewer than [`smallest`](Self::smallest)):
    /// triangles, the input's points that are still used, and UVs if the
    /// input had them.
    #[must_use]
    pub fn replay(&self, share: f32) -> PolyMesh {
        let total = self.triangles.len();
        let share = if share.is_nan() { 1.0 } else { share.clamp(0.0, 1.0) };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "between 0 and the count"
        )]
        let target = (f64::from(share) * total as f64).round() as usize;
        // The fewest collapses that bring the count down to the target.
        let applied = if target >= total {
            0
        } else {
            self.remaining.partition_point(|&n| us(n) > target) + 1
        }
        .min(self.collapses.len());
        let mut point_root: Vec<u32> = (0..u32_index(self.points.len())).collect();
        let mut wedge_root: Vec<u32> = (0..u32_index(self.wedge_point.len())).collect();
        for c in 0..applied {
            let [from, to] = self.collapses[c];
            point_root[us(from)] = to;
            let moves = us(self.move_start[c])..us(self.move_start[c + 1]);
            for &[from, to] in &self.wedge_moves[moves] {
                wedge_root[us(from)] = to;
            }
        }
        let find = |roots: &mut [u32], mut x: u32| {
            let mut root = x;
            while roots[us(root)] != root {
                root = roots[us(root)];
            }
            while roots[us(x)] != root {
                let next = roots[us(x)];
                roots[us(x)] = root;
                x = next;
            }
            root
        };
        let mut new_index = vec![u32::MAX; self.points.len()];
        let mut points = Vec::new();
        let mut faces = Vec::with_capacity(self.triangles.len());
        let mut uvs = Vec::new();
        for triangle in &self.triangles {
            let wedges = triangle.map(|w| find(&mut wedge_root, w));
            let corners = wedges.map(|w| find(&mut point_root, self.wedge_point[us(w)]));
            if corners[0] == corners[1] || corners[1] == corners[2] || corners[0] == corners[2] {
                continue;
            }
            let face = corners.map(|p| {
                if new_index[us(p)] == u32::MAX {
                    new_index[us(p)] = u32_index(points.len());
                    points.push(self.points[us(p)]);
                }
                new_index[us(p)]
            });
            faces.push(face);
            if !self.wedge_uv.is_empty() {
                uvs.extend(wedges.map(|w| self.wedge_uv[us(w)]));
            }
        }
        let mut mesh = PolyMesh::from_faces(points, faces).unwrap_or_default();
        if !uvs.is_empty() && uvs.len() == mesh.corner_count() {
            let _ = mesh.set_uvs(Some(uvs));
        }
        mesh
    }
}

impl<'a> Work<'a> {
    fn new(order: &'a mut CollapseOrder, constraints: Constraints) -> Self {
        let n = order.points.len();
        let mut around = vec![Vec::new(); n];
        let mut quadrics = vec![Quadric::default(); n];
        for (t, triangle) in order.triangles.iter().enumerate() {
            let [a, b, c] = triangle.map(|w| order.wedge_point[us(w)]);
            for p in [a, b, c] {
                around[us(p)].push(u32_index(t));
            }
            let [pa, pb, pc] = [a, b, c].map(|p| order.points[us(p)].as_dvec3());
            let normal = (pb - pa).cross(pc - pa);
            let area = normal.length() * 0.5;
            if area > 0.0 {
                // Unweighted (the paper's own): the error is a distance, so
                // dense parts are not taken first.
                let q = Quadric::plane(normal / (2.0 * area), pa, 1.0);
                for p in [a, b, c] {
                    quadrics[us(p)].add(&q);
                }
            }
        }
        let left = u32_index(order.triangles.len());
        let triangles = order.triangles.clone();
        let corners = triangles.iter().map(|t| t.map(|w| order.wedge_point[us(w)])).collect();
        let mut work = Self {
            order,
            constraints,
            quadrics,
            triangles,
            corners,
            around,
            alive: vec![true; n],
            heap: BinaryHeap::new(),
            left,
            neighbours: Vec::new(),
            others: Vec::new(),
            moves: Vec::new(),
            refused: Vec::new(),
        };
        work.add_constraint_planes();
        for p in 0..u32_index(n) {
            work.push_edges_of(p);
        }
        work
    }

    fn point(&self, wedge: u32) -> u32 {
        self.order.wedge_point[us(wedge)]
    }

    fn triangle(&self, t: u32) -> [u32; 3] {
        self.triangles[us(t)]
    }

    fn points_of(&self, t: u32) -> [u32; 3] {
        self.corners[us(t)]
    }

    fn is_dead(&self, t: u32) -> bool {
        self.triangles[us(t)][0] == u32::MAX
    }

    /// Point `p`'s neighbours: for each, the number of live triangles on the
    /// edge and whether `p`'s UVs split along it (into `out`).
    fn neighbours_of(&self, p: u32, out: &mut Vec<Near>) {
        out.clear();
        for &t in &self.around[us(p)] {
            if self.is_dead(t) {
                continue;
            }
            let corners = self.points_of(t);
            let Some(at) = corners.iter().position(|&q| q == p) else {
                continue;
            };
            let wedge = self.triangle(t)[at];
            for q in corners {
                if q != p {
                    out.push(Near {
                        point: q,
                        count: 1,
                        wedge,
                        split: false,
                    });
                }
            }
        }
        // Sorted and merged, not searched: a pole of a fine sphere has a
        // thousand neighbours.
        out.sort_unstable_by_key(|n| (n.point, n.wedge));
        let mut kept: usize = 0;
        for read in 0..out.len() {
            let near = out[read];
            match kept.checked_sub(1).map(|last| &mut out[last]) {
                Some(last) if last.point == near.point => {
                    last.count += 1;
                    last.split |= last.wedge != near.wedge;
                }
                _ => {
                    out[kept] = near;
                    kept += 1;
                }
            }
        }
        out.truncate(kept);
    }

    /// The wedges at `p` and at `q` of each live triangle with edge `p`–`q`
    /// (at most `N`).
    fn edge_wedges(&self, p: u32, q: u32) -> ([[u32; 2]; 2], usize) {
        let mut found = [[u32::MAX; 2]; 2];
        let mut count = 0;
        for &t in &self.around[us(p)] {
            if self.is_dead(t) {
                continue;
            }
            let triangle = self.triangle(t);
            let (Some(wp), Some(wq)) = (
                triangle.iter().copied().find(|&w| self.point(w) == p),
                triangle.iter().copied().find(|&w| self.point(w) == q),
            ) else {
                continue;
            };
            if count < 2 {
                found[count] = [wp, wq];
            }
            count += 1;
        }
        (found, count)
    }

    /// Whether edge `p`–`q` (with `count` triangles) holds point `p` to its
    /// line: an open border, an edge of more than two faces, or a UV seam
    /// whose UVs split at `p`'s end (where they split only at the far end,
    /// as on a sphere's pole, `p` itself is not on the cut).
    fn holds(&self, near: &Near) -> bool {
        match near.count {
            1 => self.constraints.borders,
            2 => self.constraints.seams && near.split,
            _ => true, // more than two faces: never moved across
        }
    }

    /// Whether edge `p`–`q` (with `count` triangles) keeps its line (it holds
    /// either end).
    fn is_kept(&self, p: u32, q: u32, count: u32) -> bool {
        match count {
            1 => self.constraints.borders,
            2 => {
                self.constraints.seams && {
                    let ([first, second], _) = self.edge_wedges(p, q);
                    first[0] != second[0] || first[1] != second[1]
                }
            }
            _ => true,
        }
    }

    /// Adds a plane through each kept edge, upright on its face, to both ends.
    fn add_constraint_planes(&mut self) {
        let mut neighbours = Vec::new();
        for p in 0..u32_index(self.order.points.len()) {
            self.neighbours_of(p, &mut neighbours);
            for &Near { point: q, count, .. } in &neighbours {
                if q < p || !self.is_kept(p, q, count) {
                    continue;
                }
                let Some(&t) = self.around[us(p)]
                    .iter()
                    .find(|&&t| !self.is_dead(t) && self.points_of(t).contains(&q))
                else {
                    continue;
                };
                let [a, b, c] = self.points_of(t).map(|i| self.order.points[us(i)].as_dvec3());
                let face = (b - a).cross(c - a);
                let (pp, pq) = (self.order.points[us(p)].as_dvec3(), self.order.points[us(q)].as_dvec3());
                let along = pq - pp;
                let normal = along.cross(face).normalize_or_zero();
                if normal != DVec3::ZERO {
                    let q_plane = Quadric::plane(normal, pp, CONSTRAINT_WEIGHT);
                    self.quadrics[us(p)].add(&q_plane);
                    self.quadrics[us(q)].add(&q_plane);
                }
            }
        }
    }

    /// The error of moving `from` onto `to`.
    fn cost(&self, from: u32, to: u32) -> f64 {
        let mut sum = self.quadrics[us(from)];
        sum.add(&self.quadrics[us(to)]);
        sum.error(self.order.points[us(to)].as_dvec3())
    }

    /// The cost of edge `a`–`b`: its cheaper direction's.
    fn edge_cost(&self, a: u32, b: u32) -> f64 {
        self.cost(a, b).min(self.cost(b, a))
    }

    /// Pushes edge `a`–`b` with its cost (one entry per edge, not per
    /// direction: half the heap).
    fn push_edge(&mut self, a: u32, b: u32) {
        let (a, b) = (a.min(b), a.max(b));
        self.heap.push(Candidate {
            cost: self.edge_cost(a, b),
            from: a,
            to: b,
        });
    }

    /// Pushes every edge at `p` to a higher-numbered point (every edge once).
    fn push_edges_of(&mut self, p: u32) {
        let mut neighbours = std::mem::take(&mut self.neighbours);
        self.neighbours_of(p, &mut neighbours);
        for &Near { point: q, .. } in &neighbours {
            if q > p {
                self.push_edge(p, q);
            }
        }
        self.neighbours = neighbours;
    }

    /// Collapses edges, the cheapest first. Costs only grow (quadrics are
    /// summed, kept points never move), so an entry is checked when it comes
    /// out: if its edge now costs more it goes back in with the new cost,
    /// and the entry that comes out at its own cost is the cheapest edge.
    fn run(&mut self) {
        self.order.move_start.push(0);
        loop {
            let before = self.order.collapses.len();
            while let Some(candidate) = self.heap.pop() {
                let (a, b) = (candidate.from, candidate.to);
                if !self.alive[us(a)] || !self.alive[us(b)] {
                    continue;
                }
                let (ab, ba) = (self.cost(a, b), self.cost(b, a));
                if ab.min(ba) > candidate.cost {
                    self.heap.push(Candidate {
                        cost: ab.min(ba),
                        ..candidate
                    });
                    continue;
                }
                // The cheaper direction first, else the other.
                let directions = if ab <= ba { [(a, b), (b, a)] } else { [(b, a), (a, b)] };
                if !directions.into_iter().any(|(from, to)| {
                    let ok = self.allowed(from, to);
                    if ok {
                        self.collapse(from, to);
                    }
                    ok
                }) {
                    self.refused.push([a, b]);
                }
            }
            // Refused edges may be allowed now that their surroundings
            // changed; stop when a round collapses nothing.
            if self.order.collapses.len() == before || self.refused.is_empty() {
                break;
            }
            for [a, b] in std::mem::take(&mut self.refused) {
                if self.alive[us(a)] && self.alive[us(b)] {
                    self.push_edge(a, b);
                }
            }
        }
    }

    /// Whether `from` may move onto `to`; fills `self.moves` with the wedge
    /// merges it needs.
    fn allowed(&mut self, from: u32, to: u32) -> bool {
        let mut neighbours = std::mem::take(&mut self.neighbours);
        let mut others = std::mem::take(&mut self.others);
        self.neighbours_of(from, &mut neighbours);
        self.neighbours_of(to, &mut others);
        let ok = self.allowed_with(from, to, &neighbours, &others);
        self.neighbours = neighbours;
        self.others = others;
        ok
    }

    fn allowed_with(&mut self, from: u32, to: u32, around_from: &[Near], around_to: &[Near]) -> bool {
        let Some(edge) = around_from.iter().find(|n| n.point == to) else {
            return false; // no longer an edge
        };
        let shared = edge.count;
        if shared > 2 || self.left <= shared + 1 {
            return false;
        }
        // A point on kept edges slides only along them (and a corner of
        // them stays).
        let kept = around_from.iter().filter(|n| self.holds(n)).count();
        if kept > 0 && (kept != 2 || !self.holds(edge)) {
            return false;
        }
        // Topology: two border points join only along their border edge; the
        // two ends share exactly the edge's opposite points.
        let border = |list: &[Near]| list.iter().any(|n| n.count == 1);
        if border(around_from) && border(around_to) && shared != 1 {
            return false;
        }
        let mut common = [u32::MAX; 2];
        let mut found = 0;
        for n in around_from {
            if around_to.iter().any(|r| r.point == n.point) {
                if found < 2 {
                    common[found] = n.point;
                }
                found += 1;
            }
        }
        if found != us(shared) {
            return false;
        }
        // No face turned over, flattened or doubled.
        let target = self.order.points[us(to)].as_dvec3();
        for &t in &self.around[us(from)] {
            if self.is_dead(t) {
                continue;
            }
            let corners = self.points_of(t);
            if corners.contains(&to) {
                continue;
            }
            let position = |p: u32| self.order.points[us(p)].as_dvec3();
            let [a, b, c] = corners.map(position);
            let before = (b - a).cross(c - a);
            let moved = corners.map(|p| if p == from { target } else { position(p) });
            let after = (moved[1] - moved[0]).cross(moved[2] - moved[0]);
            if after.length_squared() <= before.length_squared() * 1e-12 || before.dot(after) <= 0.0 {
                return false;
            }
            // Doubled: only possible when both other corners neighbour `to`
            // too, i.e. are the edge's opposite points (a tetrahedron).
            let mut rest = corners.into_iter().filter(|&p| p != from);
            let (Some(a), Some(b)) = (rest.next(), rest.next()) else {
                continue;
            };
            if common.contains(&a) && common.contains(&b) {
                let doubled = self.around[us(to)].iter().any(|&s| {
                    !self.is_dead(s) && {
                        let other = self.points_of(s);
                        !other.contains(&from) && other.contains(&a) && other.contains(&b)
                    }
                });
                if doubled {
                    return false;
                }
            }
        }
        // Each of `from`'s wedges goes to `to`'s wedge on the same side.
        self.moves.clear();
        let (pairs, count) = self.edge_wedges(from, to);
        let pairs = &pairs[..count.min(2)];
        for &t in &self.around[us(from)] {
            if self.is_dead(t) {
                continue;
            }
            let Some(w) = self.triangle(t).into_iter().find(|&w| self.point(w) == from) else {
                continue;
            };
            if self.moves.iter().any(|m| m[0] == w) {
                continue;
            }
            // The UV `to` has on `w`'s side; two different ones mean the
            // edge's far end is cut and `w` would be stretched across it.
            let mut sides = pairs.iter().filter(|pair| pair[0] == w).map(|pair| pair[1]);
            let keep_seams = self.constraints.seams && !self.order.wedge_uv.is_empty();
            match (sides.next(), sides.next()) {
                (Some(a), Some(b)) if a != b && keep_seams => return false,
                (Some(a), _) => self.moves.push([w, a]),
                (None, _) if keep_seams => return false,
                (None, _) => match pairs.first() {
                    Some(pair) => self.moves.push([w, pair[1]]),
                    None => return false,
                },
            }
        }
        true
    }

    /// Moves `from` onto `to` (the merges in `self.moves`) and records it.
    fn collapse(&mut self, from: u32, to: u32) {
        let around = std::mem::take(&mut self.around[us(from)]);
        let mut removed = 0;
        for &t in &around {
            if self.is_dead(t) {
                continue;
            }
            if self.points_of(t).contains(&to) {
                self.triangles[us(t)] = [u32::MAX; 3];
                self.corners[us(t)] = [u32::MAX; 3];
                removed += 1;
                continue;
            }
            for corner in &mut self.corners[us(t)] {
                if *corner == from {
                    *corner = to;
                }
            }
            let moves = &self.moves;
            let point = &self.order.wedge_point;
            let triangle = &mut self.triangles[us(t)];
            for w in triangle.iter_mut() {
                if point[us(*w)] == from
                    && let Some(m) = moves.iter().find(|m| m[0] == *w)
                {
                    *w = m[1];
                }
            }
            self.around[us(to)].push(t);
        }
        self.around[us(to)].retain(|&t| self.triangles[us(t)][0] != u32::MAX);
        self.alive[us(from)] = false;
        let q = self.quadrics[us(from)];
        self.quadrics[us(to)].add(&q);
        self.left -= removed;
        self.order.collapses.push([from, to]);
        self.order.remaining.push(self.left);
        self.order.wedge_moves.extend_from_slice(&self.moves);
        self.order.move_start.push(u32_index(self.order.wedge_moves.len()));
        // The edges `to` gains: `from`'s to points `to` did not reach
        // (`allowed` left both lists in the scratch space). Its old edges
        // keep their entries, checked when they come out.
        let (gained, had) = (std::mem::take(&mut self.neighbours), std::mem::take(&mut self.others));
        for near in &gained {
            if near.point != to && !had.iter().any(|n| n.point == near.point) {
                self.push_edge(to, near.point);
            }
        }
        (self.neighbours, self.others) = (gained, had);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn sphere() -> PolyMesh {
        primitives::sphere(100.0, 128)
    }

    #[test]
    fn a_sphere_reduced_to_a_tenth_keeps_its_shape() {
        let mesh = sphere();
        let order = CollapseOrder::new(&mesh, Constraints::default());
        let total = order.triangle_count();
        let reduced = order.replay(0.1);
        let kept = reduced.face_count();
        assert!(
            (kept as f64 - total as f64 * 0.1).abs() <= total as f64 * 0.01,
            "{kept} of {total}"
        );
        for f in 0..reduced.face_count() {
            let centre = reduced.face_center(f);
            assert!((centre.length() - 100.0).abs() < 2.0, "within 2 % of the radius");
            assert!(reduced.face_normal(f).dot(centre) > 0.0, "facing out");
        }
        for p in reduced.points() {
            assert!((p.length() - 100.0).abs() < 1e-3, "the input's own points");
        }
    }

    #[test]
    fn keeping_everything_or_nothing() {
        let mesh = sphere();
        let order = CollapseOrder::new(&mesh, Constraints::default());
        let all = order.replay(1.0);
        assert_eq!(all.face_count(), order.triangle_count());
        assert_eq!(all.point_count(), mesh.point_count());
        let least = order.replay(0.0);
        assert_eq!(least.face_count(), order.smallest());
        assert!(least.face_count() >= 4, "a closed shape stays one");
        assert!(crate::Topology::build(&least).is_closed(), "and closed");
    }

    #[test]
    fn borders_keep_their_outline() {
        // A plane with a square hole: every border point of the result lies
        // on the input's border lines.
        let mut plane = primitives::plane(200.0, 200.0, 20, 20);
        let hole: Vec<usize> = (0..plane.face_count())
            .filter(|&f| {
                let c = plane.face_center(f);
                c.x.abs() < 40.0 && c.z.abs() < 40.0
            })
            .collect();
        plane = plane.with_faces_where(|f| !hole.contains(&f));
        let order = CollapseOrder::new(&plane, Constraints::default());
        let reduced = order.replay(0.1);
        let topology = crate::Topology::build(&reduced);
        let on_lines = |p: Vec3| {
            (p.x.abs() - 100.0).abs() < 1e-3
                || (p.z.abs() - 100.0).abs() < 1e-3
                || ((p.x.abs() - 40.0).abs() < 1e-3 && p.z.abs() <= 40.0 + 1e-3)
                || ((p.z.abs() - 40.0).abs() < 1e-3 && p.x.abs() <= 40.0 + 1e-3)
        };
        let mut border_points = 0;
        for e in 0..topology.edge_count() {
            if topology.edge_faces(e).len() == 1 {
                for p in topology.edges()[e] {
                    border_points += 1;
                    assert!(on_lines(reduced.points()[us(p)]), "{:?}", reduced.points()[us(p)]);
                }
            }
        }
        assert!(border_points > 0);
        // Corners of the outline stay.
        for corner in [Vec3::new(100.0, 0.0, 100.0), Vec3::new(40.0, 0.0, 40.0)] {
            assert!(reduced.points().iter().any(|p| p.distance(corner) < 1e-3));
        }
    }

    /// Which side of a box a direction points to (0 – 5).
    fn side(normal: Vec3) -> usize {
        let axis = (0..3)
            .max_by(|&i, &j| normal[i].abs().total_cmp(&normal[j].abs()))
            .unwrap();
        axis * 2 + usize::from(normal[axis] > 0.0)
    }

    #[test]
    fn uv_seams_stay_where_they_are() {
        // A cube whose sides are separate UV islands (side k at U 2k – 2k+1):
        // every edge between sides is a seam, so no triangle may come to
        // span two sides, and every UV stays in its side's island.
        let mut cube = primitives::cube(Vec3::splat(100.0), [8, 8, 8]);
        let mut uvs = cube.uvs().unwrap().to_vec();
        for f in 0..cube.face_count() {
            let offset = 2.0 * side(cube.face_normal(f)) as f32;
            for uv in &mut uvs[cube.corner_range(f)] {
                uv.x += offset;
            }
        }
        cube.set_uvs(Some(uvs)).unwrap();
        let order = CollapseOrder::new(&cube, Constraints::default());
        let reduced = order.replay(0.0);
        assert!(reduced.face_count() < order.triangle_count() / 4, "it still reduces");
        let uvs = reduced.uvs().expect("UVs kept");
        for f in 0..reduced.face_count() {
            let k = side(reduced.face_normal(f));
            let normal = reduced.face_normal(f);
            for (c, &p) in reduced.face(f).iter().enumerate() {
                let point = reduced.points()[us(p)];
                let axis = k / 2;
                assert!((point[axis].abs() - 50.0).abs() < 1e-3, "the triangle lies in one side");
                assert!(normal[axis].abs() > 0.999);
                let u = uvs[reduced.corner_range(f).start + c].x;
                let island = 2.0 * k as f32;
                assert!((island..=island + 1.0).contains(&u), "its UVs stay in its island");
            }
        }
    }

    #[test]
    fn replays_are_deterministic_and_garbage_is_harmless() {
        let mesh = sphere();
        let a = CollapseOrder::new(&mesh, Constraints::default()).replay(0.3);
        let b = CollapseOrder::new(&mesh, Constraints::default()).replay(0.3);
        assert_eq!(a, b);
        let empty = CollapseOrder::new(&PolyMesh::default(), Constraints::default());
        assert_eq!(empty.replay(0.5).face_count(), 0);
        let mut bad = primitives::cube(Vec3::splat(10.0), [1, 1, 1]);
        bad.points_mut()[0] = Vec3::NAN;
        assert_eq!(
            CollapseOrder::new(&bad, Constraints::default())
                .replay(0.5)
                .face_count(),
            0
        );
        let order = CollapseOrder::new(&mesh, Constraints::default());
        assert_eq!(order.replay(f32::NAN).face_count(), order.triangle_count());
    }
}
