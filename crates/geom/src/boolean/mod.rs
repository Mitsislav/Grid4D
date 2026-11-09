//! Booleans of polygon meshes: union, subtract, intersect and *without* of
//! two meshes taken as solids (spec `docs/reference/generator-boolean.md`).
//!
//! **Step 17.2 is a spike**: one pipeline, two ways to hold the points the
//! cut makes ([`Method`]), compared on a corpus of hard cases. The pipeline:
//!
//! 1. both meshes are triangulated; each triangle gets its own plane and
//!    three *edge planes* (through an edge, along the axis its normal leans
//!    on least), which bound it;
//! 2. triangle pairs whose boxes touch are tested exactly; a triangle is cut
//!    by every plane of the other mesh that crosses it (by the other
//!    triangle's edge planes when the two lie in one plane), into convex
//!    pieces bounded by planes;
//! 3. equal points are joined, and every point lying on a piece's edge is
//!    inserted into it, so pieces of both meshes meet edge to edge;
//! 4. pieces are grouped across edges that do not lie on the other mesh, and
//!    each group is inside or outside the other mesh by its generalised
//!    winding number (Jacobson et al. 2013), so slightly open meshes work;
//!    pieces lying on the other mesh's surface are kept by the mode's rule;
//! 5. the kept pieces become the result's faces (B's turned for Subtract),
//!    each recording its input and face.
//!
//! Signs of points against planes are exact ([`crate::exact`]); nothing is
//! decided by a tolerance with [`Method::Exact`].

mod kernel;
#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use glam::{DVec3, I64Vec3};

use crate::{PolyMesh, Triangulator, u32_index, us};
use kernel::{Kernel, PlaneDef};

/// What the Boolean keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BooleanMode {
    /// Everything inside either.
    Union,
    /// A with B's volume cut away, closed where B cut.
    #[default]
    Subtract,
    /// Only what is inside both.
    Intersect,
    /// A's surface outside B, left open where B cut it.
    Without,
}

impl BooleanMode {
    pub const ALL: [Self; 4] = [Self::Union, Self::Subtract, Self::Intersect, Self::Without];
}

/// How the points made by the cut are held (the spike's two candidates).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Method {
    /// Implicitly, as the meeting point of three input planes: every test
    /// stays exact (plane-based).
    #[default]
    Exact,
    /// Rounded to `f64` coordinates when made; tests on them are exact, but
    /// they no longer lie exactly on the planes that made them, so joining
    /// and on-the-surface tests need a tolerance.
    Rounded,
}

/// Which input a face of the result came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    A,
    B,
}

/// Where a face of the result came from: its input and that input's face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FaceOrigin {
    pub input: Input,
    pub face: u32,
}

/// A Boolean's result.
#[derive(Clone, Debug)]
pub struct BooleanResult {
    pub mesh: PolyMesh,
    /// One per face of `mesh`.
    pub origins: Vec<FaceOrigin>,
    pub stats: BooleanStats,
}

/// What the work took (for the spike's comparison).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BooleanStats {
    /// Time spent preparing, cutting, joining points, stitching,
    /// classifying and building the result.
    pub times: [Duration; 6],
    /// Triangle pairs whose boxes touch.
    pub pairs: usize,
    /// Convex pieces after cutting (both inputs).
    pub pieces: usize,
    /// Signs the interval filter could not decide.
    pub exact_calls: usize,
    /// Groups classified by a winding number.
    pub groups: usize,
}

/// Why a Boolean could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanError {
    /// An input point is not a finite number.
    NotFinite,
    /// The result would exceed a mesh's limits.
    TooLarge,
}

/// One triangle of either input.
struct Tri {
    input: Input,
    face: u32,
    points: [u32; 3],
    support: u32,
    /// Edge `i` runs from corner `i` to corner `i + 1`.
    edges: [u32; 3],
    /// The side of edge plane `i` the triangle lies on.
    inner: [i8; 3],
    lo: DVec3,
    hi: DVec3,
}

/// A convex piece of a triangle: its plane, its bounding planes in order and
/// its corners (`verts[i]` lies on `edges[i - 1]` and `edges[i]`).
#[derive(Clone, Debug)]
struct Piece {
    tri: u32,
    edges: Vec<u32>,
    verts: Vec<u32>,
}

/// Results of splitting a piece by a plane.
enum Split {
    Keep,
    Both(Piece, Piece),
}

/// Combines `a` and `b` as solids (see the module documentation).
pub fn boolean(a: &PolyMesh, b: &PolyMesh, mode: BooleanMode, method: Method) -> Result<BooleanResult, BooleanError> {
    let mut times = [Duration::ZERO; 6];
    let mut clock = Instant::now();
    let mut lap = |slot: usize| {
        times[slot] = clock.elapsed();
        clock = Instant::now();
    };
    let mut work = Work::new(method);
    work.add_mesh(a, Input::A)?;
    work.add_mesh(b, Input::B)?;
    lap(0);
    work.cut();
    lap(1);
    work.join_points();
    lap(2);
    work.stitch();
    lap(3);
    let keep = work.classify(mode);
    lap(4);
    let mut result = work.output(&keep)?;
    lap(5);
    result.stats.times = times;
    Ok(result)
}

/// A grid of points by cell, for finding near points.
struct Grid {
    cell: f64,
    map: HashMap<I64Vec3, Vec<u32>>,
}

impl Grid {
    fn new(cell: f64) -> Self {
        Self {
            cell: cell.max(f64::MIN_POSITIVE),
            map: HashMap::new(),
        }
    }

    fn key(&self, p: DVec3) -> I64Vec3 {
        (p / self.cell).floor().as_i64vec3()
    }

    fn insert(&mut self, id: u32, p: DVec3) {
        self.map.entry(self.key(p)).or_default().push(id);
    }

    /// Every id in the cells the box touches.
    fn near(&self, lo: DVec3, hi: DVec3, out: &mut Vec<u32>) {
        out.clear();
        let (a, b) = (self.key(lo), self.key(hi));
        let span = (b - a + I64Vec3::ONE).max(I64Vec3::ZERO);
        let cells = span.x.saturating_mul(span.y).saturating_mul(span.z);
        if cells > i64::try_from(self.map.len()).unwrap_or(i64::MAX) {
            for ids in self.map.values() {
                out.extend_from_slice(ids);
            }
            return;
        }
        for x in a.x..=b.x {
            for y in a.y..=b.y {
                for z in a.z..=b.z {
                    if let Some(ids) = self.map.get(&I64Vec3::new(x, y, z)) {
                        out.extend_from_slice(ids);
                    }
                }
            }
        }
    }
}

struct Work {
    kernel: Kernel,
    tris: Vec<Tri>,
    /// Each input's triangles as `[f64 corners]`, for winding numbers.
    solids: [Vec<[DVec3; 3]>; 2],
    /// Per triangle: the other input's triangles it meets (cut or touch).
    others: Vec<Vec<u32>>,
    /// Per triangle: the other input's triangles lying in its plane.
    coplanar: Vec<Vec<u32>>,
    pieces: Vec<Piece>,
    /// Per kernel point: the point it was joined to.
    canon: Vec<u32>,
    /// Per piece: its corners after joining and stitching.
    outlines: Vec<Vec<u32>>,
    /// Tolerance for approximate searches (and [`Method::Rounded`]'s tests).
    tolerance: f64,
    pairs: usize,
    groups: usize,
}

impl Work {
    fn new(method: Method) -> Self {
        Self {
            kernel: Kernel::new(method),
            tris: Vec::new(),
            solids: [Vec::new(), Vec::new()],
            others: Vec::new(),
            coplanar: Vec::new(),
            pieces: Vec::new(),
            canon: Vec::new(),
            outlines: Vec::new(),
            tolerance: 0.0,
            pairs: 0,
            groups: 0,
        }
    }

    fn add_mesh(&mut self, mesh: &PolyMesh, input: Input) -> Result<(), BooleanError> {
        if mesh.points().iter().any(|p| !p.is_finite()) {
            return Err(BooleanError::NotFinite);
        }
        let mut triangles = Vec::new();
        let mut faces = Vec::new();
        Triangulator::new().triangulate_points(mesh, &mut triangles, &mut faces);
        let slot = usize::from(input == Input::B);
        for (corners, &face) in triangles.iter().zip(&faces) {
            let p = corners.map(|c| mesh.points()[us(c)].as_dvec3());
            let support = self.kernel.add_plane(PlaneDef::Support(p));
            if self.kernel.is_degenerate(support) {
                continue;
            }
            self.solids[slot].push(p);
            let normal = self.kernel.normal(support).abs();
            let axis = if normal.x >= normal.y && normal.x >= normal.z {
                0
            } else if normal.y >= normal.z {
                1
            } else {
                2
            };
            let points = p.map(|q| self.kernel.input_point(q));
            let edges = [0, 1, 2].map(|i| self.kernel.add_plane(PlaneDef::Edge([p[i], p[(i + 1) % 3]], axis)));
            let inner = [0, 1, 2].map(|i| self.kernel.side(points[(i + 2) % 3], edges[i]));
            self.tris.push(Tri {
                input,
                face,
                points,
                support,
                edges,
                inner,
                lo: p[0].min(p[1]).min(p[2]),
                hi: p[0].max(p[1]).max(p[2]),
            });
        }
        Ok(())
    }

    /// Pairs of triangles, one from each input, whose boxes touch.
    fn pairs(&self) -> Vec<(u32, u32)> {
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        let mut size = 0.0;
        for t in &self.tris {
            lo = lo.min(t.lo);
            hi = hi.max(t.hi);
            size += (t.hi - t.lo).max_element();
        }
        let count = self.tris.len().max(1);
        let cell = (size / f64::from(u32_index(count))).max((hi - lo).max_element() * 1e-6);
        let mut grid = Grid::new(cell);
        // Triangles spanning very many cells are tried against every one.
        let mut large = Vec::new();
        for (id, t) in self.tris.iter().enumerate().filter(|(_, t)| t.input == Input::B) {
            let (a, b) = (grid.key(t.lo), grid.key(t.hi));
            let span = b - a + I64Vec3::ONE;
            if span.x.saturating_mul(span.y).saturating_mul(span.z) > 4096 {
                large.push(u32_index(id));
                continue;
            }
            for x in a.x..=b.x {
                for y in a.y..=b.y {
                    for z in a.z..=b.z {
                        grid.map.entry(I64Vec3::new(x, y, z)).or_default().push(u32_index(id));
                    }
                }
            }
        }
        let mut pairs = Vec::new();
        let mut near = Vec::new();
        for (ia, ta) in self.tris.iter().enumerate().filter(|(_, t)| t.input == Input::A) {
            grid.near(ta.lo, ta.hi, &mut near);
            near.extend_from_slice(&large);
            near.sort_unstable();
            near.dedup();
            for &ib in &near {
                let tb = &self.tris[us(ib)];
                if ta.lo.cmple(tb.hi).all() && tb.lo.cmple(ta.hi).all() {
                    pairs.push((u32_index(ia), ib));
                }
            }
        }
        pairs
    }

    /// Cuts every triangle by the planes of the other input that cross it.
    fn cut(&mut self) {
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for t in &self.tris {
            lo = lo.min(t.lo);
            hi = hi.max(t.hi);
        }
        let scale = (hi - lo).max_element().max(f64::MIN_POSITIVE);
        self.tolerance = scale * 1e-9;
        let n = self.tris.len();
        self.others = vec![Vec::new(); n];
        self.coplanar = vec![Vec::new(); n];
        // Per triangle: (plane, the other triangle whose box limits the cut).
        let mut cuts: Vec<Vec<(u32, u32)>> = vec![Vec::new(); n];
        let pairs = self.pairs();
        self.pairs = pairs.len();
        for (ia, ib) in pairs {
            let (ta, tb) = (&self.tris[us(ia)], &self.tris[us(ib)]);
            let (pa, sa, ea) = (ta.points, ta.support, ta.edges);
            let (pb, sb, eb) = (tb.points, tb.support, tb.edges);
            let a_sides = pa.map(|p| self.kernel.side(p, sb));
            if a_sides.iter().all(|&s| s > 0) || a_sides.iter().all(|&s| s < 0) {
                continue;
            }
            let b_sides = pb.map(|p| self.kernel.side(p, sa));
            if b_sides.iter().all(|&s| s > 0) || b_sides.iter().all(|&s| s < 0) {
                continue;
            }
            self.others[us(ia)].push(ib);
            self.others[us(ib)].push(ia);
            if a_sides.iter().all(|&s| s == 0) {
                self.coplanar[us(ia)].push(ib);
                self.coplanar[us(ib)].push(ia);
                cuts[us(ia)].extend(eb.map(|e| (e, ib)));
                cuts[us(ib)].extend(ea.map(|e| (e, ia)));
            } else {
                cuts[us(ia)].push((sb, ib));
                cuts[us(ib)].push((sa, ia));
            }
        }
        for (id, planes) in cuts.into_iter().enumerate() {
            let tri = &self.tris[id];
            let mut pieces = vec![Piece {
                tri: u32_index(id),
                edges: tri.edges.to_vec(),
                verts: tri.points.to_vec(),
            }];
            for (plane, by) in planes {
                let by = &self.tris[us(by)];
                let (blo, bhi) = (by.lo - self.tolerance, by.hi + self.tolerance);
                let mut next = Vec::with_capacity(pieces.len() + 1);
                for piece in pieces {
                    let (plo, phi) = self.piece_box(&piece);
                    if plo.cmpgt(bhi).any() || phi.cmplt(blo).any() {
                        next.push(piece);
                        continue;
                    }
                    match self.split(&piece, plane) {
                        Split::Keep => next.push(piece),
                        Split::Both(below, above) => {
                            next.push(below);
                            next.push(above);
                        }
                    }
                }
                pieces = next;
            }
            self.pieces.extend(pieces);
        }
    }

    fn piece_box(&self, piece: &Piece) -> (DVec3, DVec3) {
        piece.verts.iter().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), &v| {
                let p = self.kernel.approx(v);
                (lo.min(p), hi.max(p))
            },
        )
    }

    /// Splits a convex piece by a plane (a piece on one side, or in the
    /// plane, is kept whole).
    fn split(&mut self, piece: &Piece, plane: u32) -> Split {
        let sides: Vec<i8> = piece.verts.iter().map(|&v| self.kernel.side(v, plane)).collect();
        if !(sides.iter().any(|&s| s > 0) && sides.iter().any(|&s| s < 0)) {
            return Split::Keep;
        }
        let support = self.tris[us(piece.tri)].support;
        let n = piece.verts.len();
        let mut below = Piece {
            tri: piece.tri,
            edges: Vec::with_capacity(n + 1),
            verts: Vec::with_capacity(n + 1),
        };
        let mut above = below.clone();
        // `verts[i]` starts edge `i`; a pushed (vertex, edge) pair is a corner
        // and the edge leaving it.
        for i in 0..n {
            let j = (i + 1) % n;
            let (a, e) = (piece.verts[i], piece.edges[i]);
            let (sa, sb) = (sides[i], sides[j]);
            let crossing = (sa < 0 && sb > 0) || (sa > 0 && sb < 0);
            let x = if crossing {
                Some(self.kernel.meet(support, e, plane))
            } else {
                None
            };
            if sa <= 0 {
                below.verts.push(a);
                below.edges.push(if sa == 0 && sb > 0 { plane } else { e });
            }
            if sa >= 0 {
                above.verts.push(a);
                above.edges.push(if sa == 0 && sb < 0 { plane } else { e });
            }
            if let Some(x) = x {
                if sa < 0 {
                    below.verts.push(x);
                    below.edges.push(plane);
                    above.verts.push(x);
                    above.edges.push(e);
                } else {
                    above.verts.push(x);
                    above.edges.push(plane);
                    below.verts.push(x);
                    below.edges.push(e);
                }
            }
        }
        // Each corner's incoming edge is the previous pair's outgoing one, so
        // `verts[i]` lies on `edges[i - 1]` and `edges[i]` again.
        Split::Both(below, above)
    }

    /// Joins equal points: each made point with any point found near it.
    fn join_points(&mut self) {
        self.canon = (0..u32_index(self.kernel.point_count())).collect();
        let mut grid = Grid::new(self.tolerance * 1024.0);
        let mut made = Vec::new();
        let mut seen = HashSet::new();
        for piece in &self.pieces {
            for &v in &piece.verts {
                if seen.insert(v) {
                    grid.insert(v, self.kernel.approx(v));
                    if self.kernel.is_made(v) {
                        made.push(v);
                    }
                }
            }
        }
        let mut near = Vec::new();
        for m in made {
            let p = self.kernel.approx(m);
            grid.near(p - self.tolerance, p + self.tolerance, &mut near);
            for &other in &near {
                if other == m || find(&mut self.canon, other) == find(&mut self.canon, m) {
                    continue;
                }
                if self.kernel.approx(other).distance(p) > self.tolerance {
                    continue;
                }
                let same = match self.kernel.method() {
                    Method::Exact => self.kernel.same(m, other),
                    Method::Rounded => true,
                };
                if same {
                    let (a, b) = (find(&mut self.canon, m), find(&mut self.canon, other));
                    // The smaller id (an input point before made ones) leads.
                    let (keep, drop) = if a < b { (a, b) } else { (b, a) };
                    self.canon[us(drop)] = keep;
                }
            }
        }
    }

    /// Inserts into every piece's edges the points lying on them, so pieces
    /// meet edge to edge.
    fn stitch(&mut self) {
        let mut involved = vec![false; self.tris.len()];
        for (id, others) in self.others.iter().enumerate() {
            involved[id] = !others.is_empty();
        }
        // Points of involved pieces, by representative.
        let mut points = HashSet::new();
        for piece in self.pieces.iter().filter(|p| involved[us(p.tri)]) {
            for &v in &piece.verts {
                points.insert(find(&mut self.canon, v));
            }
        }
        // Pieces whose edges may hold them: involved ones and those of
        // triangles sharing a point with them.
        let touched: Vec<bool> = self
            .tris
            .iter()
            .enumerate()
            .map(|(id, t)| involved[id] || t.points.iter().any(|&p| points.contains(&root(&self.canon, p))))
            .collect();
        // Cells the size of a typical involved triangle (the median: a few
        // long ones must not make every cell hold everything).
        let mut sizes: Vec<f64> = self
            .tris
            .iter()
            .enumerate()
            .filter(|(id, _)| involved[*id])
            .map(|(_, t)| (t.hi - t.lo).max_element())
            .collect();
        sizes.sort_by(f64::total_cmp);
        let typical = sizes.get(sizes.len() / 2).copied().unwrap_or(1.0);
        let mut grid = Grid::new(typical.max(self.tolerance * 16.0));
        for &p in &points {
            grid.insert(p, self.kernel.approx(p));
        }
        let mut near = Vec::new();
        let mut found: Vec<(f64, u32)> = Vec::new();
        self.outlines = Vec::with_capacity(self.pieces.len());
        for index in 0..self.pieces.len() {
            let piece = self.pieces[index].clone();
            let canon: Vec<u32> = piece.verts.iter().map(|&v| find(&mut self.canon, v)).collect();
            let n = canon.len();
            let mut outline = Vec::with_capacity(n);
            for i in 0..n {
                let (u, v) = (canon[i], canon[(i + 1) % n]);
                outline.push(u);
                if !touched[us(piece.tri)] || u == v {
                    continue;
                }
                let (pu, pv) = (self.kernel.approx(u), self.kernel.approx(v));
                grid.near(pu.min(pv) - self.tolerance, pu.max(pv) + self.tolerance, &mut near);
                found.clear();
                let along = pv - pu;
                let length2 = along.length_squared();
                for &w in &near {
                    if w == u || w == v {
                        continue;
                    }
                    let pw = self.kernel.approx(w);
                    let t = (pw - pu).dot(along) / length2.max(f64::MIN_POSITIVE);
                    // A quick test with room at both ends: the exact one decides.
                    let slack = self.tolerance / length2.sqrt().max(f64::MIN_POSITIVE);
                    if !(-slack..=1.0 + slack).contains(&t)
                        || (pu + along * t.clamp(0.0, 1.0)).distance(pw) > self.tolerance
                    {
                        continue;
                    }
                    if self.on_edge(&piece, i, w) {
                        found.push((t, w));
                    }
                }
                // Points on one line are ordered exactly by their coordinate
                // along the line's main axis (approximate positions can swap
                // points a hair apart, and the edge's other side would not).
                let axis = along.abs().max_position();
                let towards = if along[axis] > 0.0 { 1 } else { -1 };
                for k in 1..found.len() {
                    let mut j = k;
                    while j > 0 && self.kernel.compare(found[j - 1].1, found[j].1, axis) * towards > 0 {
                        found.swap(j - 1, j);
                        j -= 1;
                    }
                }
                outline.extend(found.iter().map(|&(_, w)| w));
            }
            outline.dedup();
            if outline.len() > 1 && outline.first() == outline.last() {
                outline.pop();
            }
            self.outlines.push(outline);
        }
    }

    /// Whether `w` lies strictly inside edge `i` of `piece`.
    fn on_edge(&mut self, piece: &Piece, i: usize, w: u32) -> bool {
        let n = piece.verts.len();
        let (prev, next) = ((i + n - 1) % n, (i + 1) % n);
        match self.kernel.method() {
            Method::Rounded => {
                let (pu, pv) = (
                    self.kernel.approx(piece.verts[i]),
                    self.kernel.approx(piece.verts[next]),
                );
                let pw = self.kernel.approx(w);
                pw.distance(pu) > self.tolerance && pw.distance(pv) > self.tolerance
            }
            Method::Exact => {
                let support = self.tris[us(piece.tri)].support;
                self.kernel.side(w, support) == 0
                    && self.kernel.side(w, piece.edges[i]) == 0
                    && {
                        let inner = self.kernel.side(piece.verts[next], piece.edges[prev]);
                        self.kernel.side(w, piece.edges[prev]) == inner
                    }
                    && {
                        let inner = self.kernel.side(piece.verts[i], piece.edges[next]);
                        self.kernel.side(w, piece.edges[next]) == inner
                    }
            }
        }
    }

    /// Whether point `p` lies on triangle `t` (its plane, within its edges).
    fn on_tri(&mut self, p: u32, t: u32) -> bool {
        let tri = &self.tris[us(t)];
        let (support, edges, inner) = (tri.support, tri.edges, tri.inner);
        if self.kernel.side_within(p, support, self.tolerance) != 0 {
            return false;
        }
        (0..3).all(|i| {
            let s = self.kernel.side_within(p, edges[i], self.tolerance);
            s == 0 || s == inner[i]
        })
    }

    /// Which pieces the mode keeps, and turned or not: `Some(false)` kept as
    /// it is, `Some(true)` turned over.
    fn classify(&mut self, mode: BooleanMode) -> Vec<Option<bool>> {
        let count = self.pieces.len();
        // Pieces lying on the other input's surface: +1 facing the same way,
        // −1 facing the other way.
        let mut on = vec![0i8; count];
        for (index, slot) in on.iter_mut().enumerate() {
            let tri = self.pieces[index].tri;
            if self.coplanar[us(tri)].is_empty() {
                continue;
            }
            let outline = self.outlines[index].clone();
            for t in self.coplanar[us(tri)].clone() {
                if outline.iter().all(|&p| self.on_tri(p, t)) {
                    *slot = self
                        .kernel
                        .normals_agree(self.tris[us(tri)].support, self.tris[us(t)].support);
                    break;
                }
            }
        }
        // Groups of pieces of one input joined by edges not on the other.
        let mut by_edge: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        for (index, outline) in self.outlines.iter().enumerate() {
            if on[index] != 0 {
                continue;
            }
            let n = outline.len();
            for i in 0..n {
                let (u, v) = (outline[i], outline[(i + 1) % n]);
                by_edge.entry((u.min(v), u.max(v))).or_default().push(u32_index(index));
            }
        }
        let mut group: Vec<u32> = (0..u32_index(count)).collect();
        let mut sharing: Vec<_> = by_edge.into_iter().filter(|(_, p)| p.len() > 1).collect();
        sharing.sort_unstable_by_key(|(edge, _)| *edge);
        for ((u, v), pieces) in sharing {
            let input = |work: &Self, piece: u32| work.tris[us(work.pieces[us(piece)].tri)].input;
            let (mut from_a, mut from_b): (Vec<u32>, Vec<u32>) =
                pieces.iter().partition(|&&piece| input(self, piece) == Input::A);
            for side in [&mut from_a, &mut from_b] {
                side.sort_unstable();
            }
            for pair in from_a.windows(2).chain(from_b.windows(2)) {
                let (p, q) = (pair[0], pair[1]);
                let (tp, tq) = (self.pieces[us(p)].tri, self.pieces[us(q)].tri);
                let on_other = |work: &mut Self, tri: u32| {
                    work.others[us(tri)]
                        .clone()
                        .into_iter()
                        .any(|t| work.on_tri(u, t) && work.on_tri(v, t))
                };
                if on_other(self, tp) || on_other(self, tq) {
                    continue;
                }
                let (a, b) = (find(&mut group, p), find(&mut group, q));
                if a != b {
                    group[us(a.max(b))] = a.min(b);
                }
            }
        }
        // Each group's place, from its largest pieces' winding numbers.
        let mut members: HashMap<u32, Vec<u32>> = HashMap::new();
        for index in 0..u32_index(count) {
            if on[us(index)] == 0 {
                members.entry(find(&mut group, index)).or_default().push(index);
            }
        }
        let mut inside = vec![false; count];
        self.groups = members.len();
        for pieces in members.values() {
            let mut by_area: Vec<(f64, u32)> = pieces.iter().map(|&p| (self.area(p), p)).collect();
            by_area.sort_by(|a, b| b.0.total_cmp(&a.0));
            let input = self.tris[us(self.pieces[us(by_area[0].1)].tri)].input;
            let other = &self.solids[usize::from(input == Input::A)];
            // The largest piece answers, unless its centre is too near the
            // other surface to tell; then the next largest (slivers last:
            // their centres may lie a hair off them).
            let mut best = 0.5;
            for &(_, p) in by_area.iter().take(4) {
                let w = winding_number(other, self.centre(p));
                if (best - 0.5f64).abs() < 0.25 && (w - 0.5).abs() > (best - 0.5f64).abs() {
                    best = w;
                }
            }
            for &p in pieces {
                inside[us(p)] = best > 0.5;
            }
        }
        (0..count)
            .map(|index| {
                let from_a = self.tris[us(self.pieces[index].tri)].input == Input::A;
                match (from_a, on[index], inside[index]) {
                    (true, 1, _) => matches!(mode, BooleanMode::Union | BooleanMode::Intersect).then_some(false),
                    (true, -1, _) => matches!(mode, BooleanMode::Subtract | BooleanMode::Without).then_some(false),
                    (false, 1 | -1, _) => None,
                    (true, _, true) => (mode == BooleanMode::Intersect).then_some(false),
                    (true, _, false) => (mode != BooleanMode::Intersect).then_some(false),
                    (false, _, true) => match mode {
                        BooleanMode::Intersect => Some(false),
                        BooleanMode::Subtract => Some(true),
                        BooleanMode::Union | BooleanMode::Without => None,
                    },
                    (false, _, false) => (mode == BooleanMode::Union).then_some(false),
                }
            })
            .collect()
    }

    fn area(&self, piece: u32) -> f64 {
        let outline = &self.outlines[us(piece)];
        let Some(&first) = outline.first() else { return 0.0 };
        let p0 = self.kernel.approx(first);
        outline
            .windows(2)
            .skip(1)
            .map(|w| {
                (self.kernel.approx(w[0]) - p0)
                    .cross(self.kernel.approx(w[1]) - p0)
                    .length()
            })
            .sum::<f64>()
            * 0.5
    }

    fn centre(&self, piece: u32) -> DVec3 {
        let outline = &self.outlines[us(piece)];
        let sum: DVec3 = outline.iter().map(|&p| self.kernel.approx(p)).sum();
        sum / f64::from(u32_index(outline.len().max(1)))
    }

    fn output(&self, keep: &[Option<bool>]) -> Result<BooleanResult, BooleanError> {
        let mut index: HashMap<u32, u32> = HashMap::new();
        let mut points = Vec::new();
        let mut faces: Vec<Vec<u32>> = Vec::new();
        let mut origins = Vec::new();
        for (piece, turned) in keep.iter().enumerate() {
            let Some(turned) = *turned else { continue };
            let outline = &self.outlines[piece];
            if outline.len() < 3 {
                continue;
            }
            let mut face: Vec<u32> = outline
                .iter()
                .map(|&p| {
                    *index.entry(p).or_insert_with(|| {
                        points.push(self.kernel.approx(p).as_vec3());
                        u32_index(points.len() - 1)
                    })
                })
                .collect();
            if turned {
                face.reverse();
            }
            // A corner twice (only rounded points can do this) is no face.
            let mut sorted = face.clone();
            sorted.sort_unstable();
            if sorted.windows(2).any(|w| w[0] == w[1]) {
                continue;
            }
            let tri = &self.tris[us(self.pieces[piece].tri)];
            faces.push(face);
            origins.push(FaceOrigin {
                input: tri.input,
                face: tri.face,
            });
        }
        let mesh = PolyMesh::from_faces(points, faces).map_err(|_| BooleanError::TooLarge)?;
        Ok(BooleanResult {
            mesh,
            origins,
            stats: BooleanStats {
                pairs: self.pairs,
                pieces: self.pieces.len(),
                exact_calls: self.kernel.exact_calls,
                groups: self.groups,
                times: [Duration::ZERO; 6],
            },
        })
    }
}

/// The representative of `id`, without shortening paths.
fn root(parent: &[u32], mut id: u32) -> u32 {
    while parent[us(id)] != id {
        id = parent[us(id)];
    }
    id
}

/// The representative of `id` (path-halving union–find).
fn find(parent: &mut [u32], mut id: u32) -> u32 {
    while parent[us(id)] != id {
        let up = parent[us(parent[us(id)])];
        parent[us(id)] = up;
        id = up;
    }
    id
}

/// The generalised winding number of a triangle soup at `q`: about 1 inside
/// a closed, outward-facing mesh, 0 outside, in between near holes.
fn winding_number(triangles: &[[DVec3; 3]], q: DVec3) -> f64 {
    let mut total = 0.0;
    for [a, b, c] in triangles {
        let (a, b, c) = (*a - q, *b - q, *c - q);
        let (la, lb, lc) = (a.length(), b.length(), c.length());
        let numerator = a.dot(b.cross(c));
        let denominator = la * lb * lc + a.dot(b) * lc + b.dot(c) * la + c.dot(a) * lb;
        total += 2.0 * numerator.atan2(denominator);
    }
    total / (4.0 * std::f64::consts::PI)
}

/// The volume a closed, outward-facing mesh encloses (negative when it faces
/// inwards).
pub fn volume(mesh: &PolyMesh) -> f64 {
    mesh.faces()
        .map(|face| {
            let p = |i: usize| mesh.points()[us(face[i])].as_dvec3();
            (1..face.len() - 1).map(|i| p(0).dot(p(i).cross(p(i + 1)))).sum::<f64>()
        })
        .sum::<f64>()
        / 6.0
}

/// Whether every edge of `mesh` is used as often in one direction as in the
/// other (the surface is closed; it may touch itself).
pub fn is_closed(mesh: &PolyMesh) -> bool {
    let mut balance: HashMap<(u32, u32), i32> = HashMap::new();
    for face in mesh.faces() {
        for (i, &u) in face.iter().enumerate() {
            let v = face[(i + 1) % face.len()];
            *balance.entry((u.min(v), u.max(v))).or_default() += if u < v { 1 } else { -1 };
        }
    }
    balance.values().all(|&b| b == 0)
}
