//! Booleans of polygon meshes: union, subtract, intersect and *without* of
//! two meshes taken as solids (spec `docs/reference/generator-boolean.md`;
//! the method — plane-based, exact — chosen by step 17.2's spike, D39).
//!
//! 1. Both meshes are triangulated. Only triangles whose boxes meet the other
//!    mesh are looked at more closely; each gets its own plane and three
//!    *edge planes* (through an edge, along the axis its normal leans on
//!    least), which bound it.
//! 2. Triangle pairs whose boxes touch are tested exactly; a triangle is cut
//!    by every plane of the other mesh that crosses it (by the other
//!    triangle's edge planes when the two lie in one plane) into convex
//!    pieces bounded by planes. Every triangle of a face that has a cut one
//!    becomes pieces; faces with no cut pass through whole.
//! 3. Equal points are joined, and every point lying on a piece's edge is
//!    inserted into it, so pieces of both meshes meet edge to edge; whole
//!    faces get the points their edges share with cut faces.
//! 4. Pieces and whole faces are grouped across edges that do not lie on the
//!    other mesh, and each group is inside or outside the other mesh by its
//!    generalised winding number (Jacobson et al. 2013), so slightly open
//!    meshes work; pieces lying on the other mesh's surface are kept by the
//!    mode's rule.
//! 5. The kept pieces of each face are merged back into polygons, with the
//!    face's UVs interpolated at every new corner; B's are turned over for
//!    Subtract. Each face of the result records its input and face; the
//!    edges along the cut are listed; points closer than a tolerance may be
//!    welded.
//!
//! Signs of points against planes are exact ([`crate::exact`]): nothing is
//! decided by a tolerance until the optional weld.

mod fast_hash;
mod kernel;
#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use glam::{DVec3, I64Vec3, Vec2};

use crate::{PolyMesh, Triangulator, u32_index, us};
use fast_hash::{FastMap, FastSet};
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

/// How the result is put together.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BooleanOptions {
    /// The pieces one face was cut into are merged back into polygons (off:
    /// triangles).
    pub merge_faces: bool,
    /// Points of the result closer than this are welded (0: none).
    pub weld: f32,
}

impl Default for BooleanOptions {
    fn default() -> Self {
        Self {
            merge_faces: true,
            weld: 0.01,
        }
    }
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
    /// With UVs when either input has them (zero where the other has none).
    pub mesh: PolyMesh,
    /// One per face of `mesh`.
    pub origins: Vec<FaceOrigin>,
    /// The edges along the cut, as point pairs of `mesh`.
    pub cut_edges: Vec<[u32; 2]>,
    pub stats: BooleanStats,
}

/// What the work took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BooleanStats {
    /// Time spent preparing and pairing, cutting, joining points,
    /// stitching, classifying and building the result.
    pub times: [Duration; 6],
    /// Triangle pairs whose boxes touch.
    pub pairs: usize,
    /// Convex pieces made of the faces that were cut.
    pub pieces: usize,
    /// Faces passed through whole.
    pub whole_faces: usize,
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

/// Combines `a` and `b` as solids (see the module documentation).
pub fn boolean(
    a: &PolyMesh,
    b: &PolyMesh,
    mode: BooleanMode,
    options: BooleanOptions,
) -> Result<BooleanResult, BooleanError> {
    let mut times = [Duration::ZERO; 6];
    let mut clock = Instant::now();
    let mut lap = |slot: usize| {
        times[slot] = clock.elapsed();
        clock = Instant::now();
    };
    let mut work = Work::new(a, b)?;
    work.pair();
    lap(0);
    work.cut();
    lap(1);
    work.join_points();
    lap(2);
    work.stitch();
    lap(3);
    work.classify(mode);
    lap(4);
    let mut result = work.output(options)?;
    lap(5);
    result.stats.times = times;
    Ok(result)
}

/// Whether every edge of `mesh` is used as often in one direction as in the
/// other (the surface is closed; it may touch itself).
pub fn is_closed(mesh: &PolyMesh) -> bool {
    let mut balance: FastMap<(u32, u32), i32> = FastMap::default();
    for face in mesh.faces() {
        for (i, &u) in face.iter().enumerate() {
            let v = face[(i + 1) % face.len()];
            *balance.entry((u.min(v), u.max(v))).or_default() += if u < v { 1 } else { -1 };
        }
    }
    balance.values().all(|&b| b == 0)
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

/// A grid of ids by cell, for finding near things.
struct Grid {
    cell: f64,
    map: FastMap<I64Vec3, Vec<u32>>,
}

impl Grid {
    fn new(cell: f64) -> Self {
        Self {
            cell: cell.max(f64::MIN_POSITIVE),
            map: FastMap::default(),
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

/// One input, triangulated.
struct Source<'a> {
    mesh: &'a PolyMesh,
    input: Input,
    /// The kernel point of each mesh point (equal coordinates share one).
    points: Vec<u32>,
    /// Triangles as corner indices of `mesh` (a face's are consecutive).
    corners: Vec<[u32; 3]>,
    /// Each triangle's face.
    faces: Vec<u32>,
    /// Each face's first triangle (`face_start[f]..face_start[f + 1]`).
    face_start: Vec<u32>,
    lo: DVec3,
    hi: DVec3,
}

impl<'a> Source<'a> {
    fn new(mesh: &'a PolyMesh, input: Input, kernel: &mut Kernel) -> Result<Self, BooleanError> {
        if mesh.points().iter().any(|p| !p.is_finite()) {
            return Err(BooleanError::NotFinite);
        }
        let points = mesh.points().iter().map(|p| kernel.input_point(p.as_dvec3())).collect();
        let mut triangles = Vec::new();
        let mut triangulator = Triangulator::new();
        let mut corners = Vec::new();
        let mut faces = Vec::new();
        let mut face_start = Vec::with_capacity(mesh.face_count() + 1);
        for f in 0..mesh.face_count() {
            face_start.push(u32_index(corners.len()));
            triangles.clear();
            triangulator.face_triangles(mesh, f, &mut triangles);
            let range = mesh.corner_range(f);
            let corner_of = |p: u32| {
                range
                    .clone()
                    .find(|&c| mesh.corner_points()[c] == p)
                    .map_or(u32_index(range.start), u32_index)
            };
            for t in &triangles {
                corners.push(t.map(corner_of));
                faces.push(u32_index(f));
            }
        }
        face_start.push(u32_index(corners.len()));
        let (lo, hi) = mesh
            .bounds()
            .map_or((DVec3::ZERO, DVec3::ZERO), |(lo, hi)| (lo.as_dvec3(), hi.as_dvec3()));
        Ok(Self {
            mesh,
            input,
            points,
            corners,
            faces,
            face_start,
            lo,
            hi,
        })
    }

    fn corner_point(&self, corner: u32) -> u32 {
        self.mesh.corner_points()[us(corner)]
    }

    fn position(&self, triangle: usize, i: usize) -> DVec3 {
        self.mesh.points()[us(self.corner_point(self.corners[triangle][i]))].as_dvec3()
    }

    fn triangle(&self, triangle: usize) -> [DVec3; 3] {
        [0, 1, 2].map(|i| self.position(triangle, i))
    }

    fn uv(&self, corner: u32) -> Vec2 {
        self.mesh.uvs().map_or(Vec2::ZERO, |uvs| uvs[us(corner)])
    }

    fn bounds(&self, triangle: usize) -> (DVec3, DVec3) {
        let [a, b, c] = self.triangle(triangle);
        (a.min(b).min(c), a.max(b).max(c))
    }
}

/// A triangle near the other input: its planes.
#[derive(Clone, Debug)]
struct Tri {
    support: u32,
    /// Edge `i` runs from corner `i` to corner `i + 1`.
    edges: [u32; 3],
    /// The side of edge plane `i` the triangle lies on.
    inner: [i8; 3],
    /// Its corners' kernel points.
    points: [u32; 3],
    degenerate: bool,
}

/// A convex piece of a triangle: its bounding planes in order and its
/// corners (`verts[i]` lies on `edges[i - 1]` and `edges[i]`).
#[derive(Clone, Debug)]
struct Piece {
    /// The triangle (global id: A's, then B's).
    tri: u32,
    edges: Vec<u32>,
    verts: Vec<u32>,
}

/// Results of splitting a piece by a plane.
enum Split {
    Keep,
    Both(Piece, Piece),
}

/// What happens to a piece or a whole face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Fate {
    #[default]
    Dropped,
    Kept,
    Turned,
}

struct Work<'a> {
    kernel: Kernel,
    sources: [Source<'a>; 2],
    /// Triangles of A before those of B: global id = A's index, or A's count
    /// + B's index.
    a_count: usize,
    /// Planes of the triangles near the other input, by global id.
    tris: FastMap<u32, Tri>,
    /// Per triangle: the other input's triangles it meets, those lying in
    /// its plane, and the planes that cut it (each with the triangle whose
    /// box bounds the cut).
    others: FastMap<u32, Vec<u32>>,
    coplanar: FastMap<u32, Vec<u32>>,
    cuts: FastMap<u32, Vec<(u32, u32)>>,
    /// Faces with a cut triangle, per input.
    cut_faces: [Vec<u32>; 2],
    pieces: Vec<Piece>,
    /// Per kernel point: the point it was joined to.
    canon: Vec<u32>,
    /// Per piece: its corners after joining and stitching, and the plane of
    /// the edge leaving each.
    outlines: Vec<Vec<u32>>,
    outline_planes: Vec<Vec<u32>>,
    /// Points inside an input edge (kernel points of its ends, smaller
    /// first), in order from the smaller.
    along: FastMap<(u32, u32), Vec<u32>>,
    /// Edges lying on the other input's surface (point pairs, smaller first).
    on_surface: FastSet<(u32, u32)>,
    /// What happens to each piece, and to each face not cut (per input).
    piece_fates: Vec<Fate>,
    face_fates: [Vec<Fate>; 2],
    tolerance: f64,
    pairs: usize,
    groups: usize,
}

impl<'a> Work<'a> {
    fn new(a: &'a PolyMesh, b: &'a PolyMesh) -> Result<Self, BooleanError> {
        let mut kernel = Kernel::new();
        let sa = Source::new(a, Input::A, &mut kernel)?;
        let sb = Source::new(b, Input::B, &mut kernel)?;
        let a_count = sa.corners.len();
        let scale = (sa.hi - sa.lo).max_element().max((sb.hi - sb.lo).max_element());
        Ok(Self {
            kernel,
            sources: [sa, sb],
            a_count,
            tris: FastMap::default(),
            others: FastMap::default(),
            coplanar: FastMap::default(),
            cuts: FastMap::default(),
            cut_faces: [Vec::new(), Vec::new()],
            pieces: Vec::new(),
            canon: Vec::new(),
            outlines: Vec::new(),
            outline_planes: Vec::new(),
            along: FastMap::default(),
            on_surface: FastSet::default(),
            piece_fates: Vec::new(),
            face_fates: [Vec::new(), Vec::new()],
            tolerance: scale.max(f64::MIN_POSITIVE) * 1e-9,
            pairs: 0,
            groups: 0,
        })
    }

    /// The input and local index of a global triangle id.
    fn local(&self, tri: u32) -> (usize, usize) {
        let t = us(tri);
        if t < self.a_count {
            (0, t)
        } else {
            (1, t - self.a_count)
        }
    }

    fn global(&self, slot: usize, t: usize) -> u32 {
        u32_index(if slot == 0 { t } else { t + self.a_count })
    }

    fn input(&self, tri: u32) -> Input {
        if us(tri) < self.a_count { Input::A } else { Input::B }
    }

    /// The planes of a triangle, made the first time they are needed.
    fn tri(&mut self, tri: u32) -> &Tri {
        if !self.tris.contains_key(&tri) {
            let (slot, t) = self.local(tri);
            let source = &self.sources[slot];
            let p = source.triangle(t);
            let points = [0, 1, 2].map(|i| source.points[us(source.corner_point(source.corners[t][i]))]);
            let degenerate = {
                let probe = self.kernel.add_plane(PlaneDef::Support(p));
                self.kernel.is_degenerate(probe)
            };
            let support = if degenerate {
                self.kernel.add_plane(PlaneDef::Support(p))
            } else {
                self.kernel.support_plane(p, points)
            };
            let normal = self.kernel.normal(support).abs();
            let axis = if normal.x >= normal.y && normal.x >= normal.z {
                0
            } else if normal.y >= normal.z {
                1
            } else {
                2
            };
            let (edges, inner) = if degenerate {
                ([support; 3], [0; 3])
            } else {
                let edges = [0, 1, 2].map(|i| {
                    let j = (i + 1) % 3;
                    self.kernel
                        .edge_plane([points[i], points[j]], [p[i], p[j]], axis, support)
                });
                let inner = [0, 1, 2].map(|i| self.kernel.side(points[(i + 2) % 3], edges[i]));
                (edges, inner)
            };
            self.tris.insert(
                tri,
                Tri {
                    support,
                    edges,
                    inner,
                    points,
                    degenerate,
                },
            );
        }
        &self.tris[&tri]
    }

    fn bounds(&self, tri: u32) -> (DVec3, DVec3) {
        let (slot, t) = self.local(tri);
        self.sources[slot].bounds(t)
    }

    /// Finds triangle pairs whose boxes touch, tests them exactly and notes
    /// the planes that cut each triangle.
    fn pair(&mut self) {
        let tol = self.tolerance;
        let [sa, sb] = &self.sources;
        // Only triangles inside the other input's box can meet it.
        let near = |source: &Source<'_>, other: &Source<'_>| -> Vec<usize> {
            (0..source.corners.len())
                .filter(|&t| {
                    let (lo, hi) = source.bounds(t);
                    lo.cmple(other.hi + tol).all() && hi.cmpge(other.lo - tol).all()
                })
                .collect()
        };
        let (near_a, near_b) = (near(sa, sb), near(sb, sa));
        let size: f64 = near_b
            .iter()
            .map(|&t| {
                let (lo, hi) = sb.bounds(t);
                (hi - lo).max_element()
            })
            .sum();
        let cell = (size / f64::from(u32_index(near_b.len().max(1)))).max(tol * 1e3);
        let mut grid = Grid::new(cell);
        let mut large = Vec::new();
        for &t in &near_b {
            let (lo, hi) = sb.bounds(t);
            let (a, b) = (grid.key(lo), grid.key(hi));
            let span = b - a + I64Vec3::ONE;
            if span.x.saturating_mul(span.y).saturating_mul(span.z) > 64 {
                large.push(u32_index(t));
                continue;
            }
            for x in a.x..=b.x {
                for y in a.y..=b.y {
                    for z in a.z..=b.z {
                        grid.map.entry(I64Vec3::new(x, y, z)).or_default().push(u32_index(t));
                    }
                }
            }
        }
        let mut pairs = Vec::new();
        let mut found = Vec::new();
        for &ta in &near_a {
            let (alo, ahi) = sa.bounds(ta);
            grid.near(alo, ahi, &mut found);
            found.extend_from_slice(&large);
            found.sort_unstable();
            found.dedup();
            for &tb in &found {
                let (blo, bhi) = sb.bounds(us(tb));
                if alo.cmple(bhi).all() && blo.cmple(ahi).all() {
                    pairs.push((self.global(0, ta), self.global(1, us(tb))));
                }
            }
        }
        self.pairs = pairs.len();
        for (ia, ib) in pairs {
            let (ta, tb) = (self.tri(ia).clone(), self.tri(ib).clone());
            if ta.degenerate || tb.degenerate {
                continue;
            }
            let a_sides = ta.points.map(|p| self.kernel.side(p, tb.support));
            if a_sides.iter().all(|&s| s > 0) || a_sides.iter().all(|&s| s < 0) {
                continue;
            }
            let b_sides = tb.points.map(|p| self.kernel.side(p, ta.support));
            if b_sides.iter().all(|&s| s > 0) || b_sides.iter().all(|&s| s < 0) {
                continue;
            }
            self.others.entry(ia).or_default().push(ib);
            self.others.entry(ib).or_default().push(ia);
            if a_sides.iter().all(|&s| s == 0) {
                self.coplanar.entry(ia).or_default().push(ib);
                self.coplanar.entry(ib).or_default().push(ia);
                self.cuts.entry(ia).or_default().extend(tb.edges.map(|e| (e, ib)));
                self.cuts.entry(ib).or_default().extend(ta.edges.map(|e| (e, ia)));
            } else {
                self.cuts.entry(ia).or_default().push((tb.support, ib));
                self.cuts.entry(ib).or_default().push((ta.support, ia));
            }
        }
        let mut cut_faces: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
        for &tri in self.cuts.keys() {
            let (slot, t) = self.local(tri);
            cut_faces[slot].push(self.sources[slot].faces[t]);
        }
        for faces in &mut cut_faces {
            faces.sort_unstable();
            faces.dedup();
        }
        self.cut_faces = cut_faces;
    }

    /// Cuts every triangle of the faces that are cut by the other input's
    /// planes that cross it.
    fn cut(&mut self) {
        for slot in 0..2 {
            for index in 0..self.cut_faces[slot].len() {
                let face = us(self.cut_faces[slot][index]);
                let source = &self.sources[slot];
                let range = us(source.face_start[face])..us(source.face_start[face + 1]);
                for t in range {
                    let tri = self.global(slot, t);
                    let data = self.tri(tri).clone();
                    if data.degenerate {
                        continue;
                    }
                    let mut pieces = vec![Piece {
                        tri,
                        edges: data.edges.to_vec(),
                        verts: data.points.to_vec(),
                    }];
                    for (plane, by) in self.cuts.get(&tri).cloned().unwrap_or_default() {
                        let (blo, bhi) = self.bounds(by);
                        let (blo, bhi) = (blo - self.tolerance, bhi + self.tolerance);
                        let mut next = Vec::with_capacity(pieces.len() + 1);
                        for piece in pieces {
                            let (plo, phi) = self.piece_box(&piece);
                            if plo.cmpgt(bhi).any() || phi.cmplt(blo).any() {
                                next.push(piece);
                                continue;
                            }
                            match self.split(&piece, data.support, plane) {
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
    fn split(&mut self, piece: &Piece, support: u32, plane: u32) -> Split {
        let sides: Vec<i8> = piece.verts.iter().map(|&v| self.kernel.side(v, plane)).collect();
        if !(sides.iter().any(|&s| s > 0) && sides.iter().any(|&s| s < 0)) {
            return Split::Keep;
        }
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
            // An input edge's line is named by the same two planes from
            // every triangle along it, so its crossings are the same points.
            let x = crossing.then(|| {
                let partner = self.kernel.partner(e).unwrap_or(support);
                let x = self.kernel.meet(partner, e, plane);
                self.kernel.note_on(x, support);
                x
            });
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
        let mut seen = FastSet::default();
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
                if other == m
                    || find(&mut self.canon, other) == find(&mut self.canon, m)
                    || self.kernel.approx(other).distance(p) > self.tolerance
                {
                    continue;
                }
                if self.kernel.same(m, other) {
                    let (a, b) = (find(&mut self.canon, m), find(&mut self.canon, other));
                    // The smaller id (an input point before made ones) leads.
                    let (keep, drop) = if a < b { (a, b) } else { (b, a) };
                    self.canon[us(drop)] = keep;
                }
            }
        }
    }

    /// Inserts into every piece's edges the points lying on them, so pieces
    /// meet edge to edge, and notes the points inside input edges.
    fn stitch(&mut self) {
        let mut points = FastSet::default();
        for piece in &self.pieces {
            for &v in &piece.verts {
                points.insert(find(&mut self.canon, v));
            }
        }
        // Cells the size of a typical cut triangle (the median: a few long
        // ones must not make every cell hold everything).
        let mut sizes: Vec<f64> = self
            .cuts
            .keys()
            .map(|&t| (self.bounds(t).1 - self.bounds(t).0).max_element())
            .collect();
        sizes.sort_by(f64::total_cmp);
        let typical = sizes.get(sizes.len() / 2).copied().unwrap_or(1.0);
        let mut grid = Grid::new(typical.max(self.tolerance * 16.0));
        for &p in &points {
            grid.insert(p, self.kernel.approx(p));
        }
        let mut near = Vec::new();
        let mut found: Vec<u32> = Vec::new();
        self.outlines = Vec::with_capacity(self.pieces.len());
        self.outline_planes = Vec::with_capacity(self.pieces.len());
        let mut along: FastMap<(u32, u32), Vec<u32>> = FastMap::default();
        for index in 0..self.pieces.len() {
            let piece = self.pieces[index].clone();
            let tri = self.tris[&piece.tri].clone();
            let canon: Vec<u32> = piece.verts.iter().map(|&v| find(&mut self.canon, v)).collect();
            let n = canon.len();
            let mut outline = Vec::with_capacity(n);
            let mut planes = Vec::with_capacity(n);
            for i in 0..n {
                let (u, v) = (canon[i], canon[(i + 1) % n]);
                outline.push(u);
                planes.push(piece.edges[i]);
                if u == v {
                    continue;
                }
                let (pu, pv) = (self.kernel.approx(u), self.kernel.approx(v));
                grid.near(pu.min(pv) - self.tolerance, pu.max(pv) + self.tolerance, &mut near);
                found.clear();
                let line = pv - pu;
                let length2 = line.length_squared().max(f64::MIN_POSITIVE);
                // A quick test with room at both ends: the exact one decides.
                let slack = self.tolerance / length2.sqrt();
                for &w in &near {
                    if w == u || w == v {
                        continue;
                    }
                    let pw = self.kernel.approx(w);
                    let t = (pw - pu).dot(line) / length2;
                    if !(-slack..=1.0 + slack).contains(&t)
                        || (pu + line * t.clamp(0.0, 1.0)).distance(pw) > self.tolerance
                    {
                        continue;
                    }
                    if self.on_edge(&piece, tri.support, i, w) {
                        found.push(w);
                    }
                }
                self.order_along(&mut found, line);
                outline.extend_from_slice(&found);
                planes.extend(found.iter().map(|_| piece.edges[i]));
                // Points inside one of the triangle's own edges.
                if let Some(e) = tri.edges.iter().position(|&e| e == piece.edges[i]) {
                    let ends = (tri.points[e], tri.points[(e + 1) % 3]);
                    let key = (ends.0.min(ends.1), ends.0.max(ends.1));
                    let inside = along.entry(key).or_default();
                    for &p in [u, v].iter().chain(&found) {
                        if p != key.0 && p != key.1 {
                            inside.push(p);
                        }
                    }
                }
            }
            // Joined corners can repeat; drop the repeats.
            let mut k = 0;
            while outline.len() > 1 && k < outline.len() {
                let next = (k + 1) % outline.len();
                if outline[k] == outline[next] {
                    outline.remove(next);
                    planes.remove(next);
                } else {
                    k += 1;
                }
            }
            self.outlines.push(outline);
            self.outline_planes.push(planes);
        }
        for ((a, b), mut inside) in along {
            inside.sort_unstable();
            inside.dedup();
            let line = self.kernel.approx(b) - self.kernel.approx(a);
            self.order_along(&mut inside, line);
            self.along.insert((a, b), inside);
        }
    }

    /// Orders points lying on one line along `line`'s direction, exactly (by
    /// their coordinate along its main axis: approximate positions can swap
    /// points a hair apart).
    fn order_along(&mut self, points: &mut [u32], line: DVec3) {
        let axis = line.abs().max_position();
        let towards = if line[axis] > 0.0 { 1 } else { -1 };
        for k in 1..points.len() {
            let mut j = k;
            while j > 0 && self.kernel.compare(points[j - 1], points[j], axis) * towards > 0 {
                points.swap(j - 1, j);
                j -= 1;
            }
        }
    }

    /// Whether `w` lies strictly inside edge `i` of `piece`.
    fn on_edge(&mut self, piece: &Piece, support: u32, i: usize, w: u32) -> bool {
        let n = piece.verts.len();
        let (prev, next) = ((i + n - 1) % n, (i + 1) % n);
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

    /// Whether point `p` lies on triangle `t` (its plane, within its edges).
    fn on_tri(&mut self, p: u32, t: u32) -> bool {
        let Some(tri) = self.tris.get(&t) else { return false };
        let (support, edges, inner) = (tri.support, tri.edges, tri.inner);
        if self.kernel.side(p, support) != 0 {
            return false;
        }
        (0..3).all(|i| {
            let s = self.kernel.side(p, edges[i]);
            s == 0 || s == inner[i]
        })
    }

    /// Whether the edge `u`–`v` of a piece of `tri` lies on the other input.
    fn on_other(&mut self, tri: u32, u: u32, v: u32) -> bool {
        let count = self.others.get(&tri).map_or(0, Vec::len);
        let (pu, pv) = (self.kernel.approx(u), self.kernel.approx(v));
        let tolerance = self.tolerance;
        (0..count).any(|k| {
            let t = self.others[&tri][k];
            // Only a triangle whose box holds the edge can hold it.
            let (lo, hi) = self.bounds(t);
            let (lo, hi) = (lo - tolerance, hi + tolerance);
            let inside = |p: DVec3| p.cmpge(lo).all() && p.cmple(hi).all();
            inside(pu) && inside(pv) && self.on_tri(u, t) && self.on_tri(v, t)
        })
    }

    /// Decides what happens to every piece and whole face.
    fn classify(&mut self, mode: BooleanMode) {
        let count = self.pieces.len();
        // Pieces lying on the other input's surface: +1 facing the same way,
        // −1 facing the other way.
        let mut on = vec![0i8; count];
        for (index, slot) in on.iter_mut().enumerate() {
            let tri = self.pieces[index].tri;
            let Some(coplanar) = self.coplanar.get(&tri).cloned() else {
                continue;
            };
            let outline = self.outlines[index].clone();
            for t in coplanar {
                if outline.iter().all(|&p| self.on_tri(p, t)) {
                    let (a, b) = (self.tris[&tri].support, self.tris[&t].support);
                    *slot = self.kernel.normals_agree(a, b);
                    break;
                }
            }
        }
        // Units: pieces, then each input's faces (only whole ones are used).
        let face_base = [count, count + self.sources[0].mesh.face_count()];
        let total = face_base[1] + self.sources[1].mesh.face_count();
        let mut group: Vec<u32> = (0..u32_index(total)).collect();
        let mut cut = [
            vec![false; self.sources[0].mesh.face_count()],
            vec![false; self.sources[1].mesh.face_count()],
        ];
        for (flags, faces) in cut.iter_mut().zip(&self.cut_faces) {
            for &f in faces {
                flags[us(f)] = true;
            }
        }
        // Edges of pieces: (point pair, piece). Joined across edges not on
        // the other input.
        let mut edges: Vec<((u32, u32), u32)> = Vec::new();
        for (index, outline) in self.outlines.iter().enumerate() {
            if on[index] != 0 {
                continue;
            }
            let n = outline.len();
            for i in 0..n {
                let (u, v) = (outline[i], outline[(i + 1) % n]);
                edges.push(((u.min(v), u.max(v)), u32_index(index)));
            }
        }
        edges.sort_unstable();
        // Every piece edge is tried once for lying on the other input.
        let mut checked: FastMap<(u32, u32), bool> = FastMap::default();
        for index in 0..self.outlines.len() {
            let tri = self.pieces[index].tri;
            let n = self.outlines[index].len();
            for i in 0..n {
                let (u, v) = (self.outlines[index][i], self.outlines[index][(i + 1) % n]);
                let key = (u.min(v), u.max(v));
                let on_other = if let Some(&known) = checked.get(&key) {
                    known || self.on_other(tri, u, v)
                } else {
                    self.on_other(tri, u, v)
                };
                if on_other {
                    self.on_surface.insert(key);
                }
                checked.insert(key, on_other);
            }
        }
        for run in edges.chunk_by(|a, b| a.0 == b.0) {
            if self.on_surface.contains(&run[0].0) {
                continue;
            }
            for input in [Input::A, Input::B] {
                let mut same = run.iter().filter(|e| self.input(self.pieces[us(e.1)].tri) == input);
                if let Some(first) = same.next() {
                    for other in same {
                        union(&mut group, first.1, other.1);
                    }
                }
            }
        }
        // Whole faces: joined to whole faces and to pieces along shared
        // input edges (never on the other input: they were not cut).
        let mut by_piece_edge: FastMap<(u32, u32), Vec<u32>> = FastMap::default();
        for (index, (outline, planes)) in self.outlines.iter().zip(&self.outline_planes).enumerate() {
            let tri = &self.tris[&self.pieces[index].tri];
            let n = outline.len();
            for i in 0..n {
                if let Some(e) = tri.edges.iter().position(|&e| e == planes[i]) {
                    let (a, b) = (tri.points[e], tri.points[(e + 1) % 3]);
                    let key = (a.min(b), a.max(b));
                    let (u, v) = (outline[i], outline[(i + 1) % n]);
                    if on[index] == 0 && !self.on_surface.contains(&(u.min(v), u.max(v))) {
                        by_piece_edge.entry(key).or_default().push(u32_index(index));
                    }
                }
            }
        }
        for slot in 0..2 {
            let source = &self.sources[slot];
            let mut face_edges: Vec<((u32, u32), u32)> = Vec::new();
            for f in (0..source.mesh.face_count()).filter(|&f| !cut[slot][f]) {
                let face = source.mesh.face(f);
                let unit = u32_index(face_base[slot] + f);
                for (i, &c) in face.iter().enumerate() {
                    let (a, b) = (source.points[us(c)], source.points[us(face[(i + 1) % face.len()])]);
                    let key = (a.min(b), a.max(b));
                    face_edges.push((key, unit));
                    if let Some(pieces) = by_piece_edge.get(&key) {
                        for &p in pieces {
                            if self.input(self.pieces[us(p)].tri) == source.input {
                                union(&mut group, unit, p);
                            }
                        }
                    }
                }
            }
            face_edges.sort_unstable();
            for run in face_edges.chunk_by(|a, b| a.0 == b.0) {
                for pair in run.windows(2) {
                    union(&mut group, pair[0].1, pair[1].1);
                }
            }
        }
        // Each group's place: its largest member's winding number (the next
        // largest when that is too near the other surface to tell).
        let mut members: FastMap<u32, Vec<(f64, u32)>> = FastMap::default();
        for (index, &on) in on.iter().enumerate() {
            if on == 0 {
                let root = find(&mut group, u32_index(index));
                let area = self.piece_area(index);
                members.entry(root).or_default().push((area, u32_index(index)));
            }
        }
        for slot in 0..2 {
            for f in (0..self.sources[slot].mesh.face_count()).filter(|&f| !cut[slot][f]) {
                let unit = u32_index(face_base[slot] + f);
                let root = find(&mut group, unit);
                let area = f64::from(self.sources[slot].mesh.face_area(f));
                members.entry(root).or_default().push((area, unit));
            }
        }
        self.groups = members.len();
        let mut inside = vec![false; total];
        for list in members.values_mut() {
            list.sort_by(|a, b| b.0.total_cmp(&a.0));
            let mut best = 0.5f64;
            for &(_, unit) in list.iter().take(4) {
                if (best - 0.5).abs() >= 0.25 {
                    break;
                }
                let (input, point) = self.unit_point(us(unit), face_base);
                let other = &self.sources[usize::from(input == Input::A)];
                let w = winding_number(other, point);
                if (w - 0.5).abs() > (best - 0.5).abs() {
                    best = w;
                }
            }
            for &(_, unit) in list.iter() {
                inside[us(unit)] = best > 0.5;
            }
        }
        let fate = |from_a: bool, on: i8, inside: bool| -> Fate {
            let keep = |yes: bool| if yes { Fate::Kept } else { Fate::Dropped };
            match (from_a, on, inside) {
                (true, 1, _) => keep(matches!(mode, BooleanMode::Union | BooleanMode::Intersect)),
                (true, -1, _) => keep(matches!(mode, BooleanMode::Subtract | BooleanMode::Without)),
                (false, 1 | -1, _) => Fate::Dropped,
                (true, _, true) => keep(mode == BooleanMode::Intersect),
                (true, _, false) => keep(mode != BooleanMode::Intersect),
                (false, _, true) => match mode {
                    BooleanMode::Intersect => Fate::Kept,
                    BooleanMode::Subtract => Fate::Turned,
                    BooleanMode::Union | BooleanMode::Without => Fate::Dropped,
                },
                (false, _, false) => keep(mode == BooleanMode::Union),
            }
        };
        self.piece_fates = (0..count)
            .map(|index| fate(self.input(self.pieces[index].tri) == Input::A, on[index], inside[index]))
            .collect();
        for slot in 0..2 {
            self.face_fates[slot] = (0..self.sources[slot].mesh.face_count())
                .map(|f| {
                    if cut[slot][f] {
                        Fate::Dropped
                    } else {
                        fate(slot == 0, 0, inside[face_base[slot] + f])
                    }
                })
                .collect();
        }
    }

    fn piece_area(&self, piece: usize) -> f64 {
        let outline = &self.outlines[piece];
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

    /// A point inside a unit (a piece's corners' centre; a whole face's
    /// largest triangle's centre) and its input.
    fn unit_point(&self, unit: usize, face_base: [usize; 2]) -> (Input, DVec3) {
        if unit < face_base[0] {
            let outline = &self.outlines[unit];
            let sum: DVec3 = outline.iter().map(|&p| self.kernel.approx(p)).sum();
            let input = self.input(self.pieces[unit].tri);
            return (input, sum / f64::from(u32_index(outline.len().max(1))));
        }
        let (slot, f) = if unit < face_base[1] {
            (0, unit - face_base[0])
        } else {
            (1, unit - face_base[1])
        };
        let source = &self.sources[slot];
        let range = us(source.face_start[f])..us(source.face_start[f + 1]);
        let centre = range
            .map(|t| {
                let [a, b, c] = source.triangle(t);
                ((b - a).cross(c - a).length(), (a + b + c) / 3.0)
            })
            .max_by(|x, y| x.0.total_cmp(&y.0))
            .map_or(DVec3::ZERO, |(_, c)| c);
        (source.input, centre)
    }

    /// The UV of a point on a triangle, by its barycentric coordinates.
    fn uv_on(&self, tri: u32, point: DVec3) -> Vec2 {
        let (slot, t) = self.local(tri);
        let source = &self.sources[slot];
        if source.mesh.uvs().is_none() {
            return Vec2::ZERO;
        }
        let [a, b, c] = source.triangle(t);
        let normal = (b - a).cross(c - a);
        let area2 = normal.length_squared();
        if area2 == 0.0 {
            return source.uv(source.corners[t][0]);
        }
        let wa = (c - b).cross(point - b).dot(normal) / area2;
        let wb = (a - c).cross(point - c).dot(normal) / area2;
        let wc = 1.0 - wa - wb;
        let [ua, ub, uc] = source.corners[t].map(|c| source.uv(c).as_dvec2());
        (ua * wa + ub * wb + uc * wc).as_vec2()
    }

    fn output(&self, options: BooleanOptions) -> Result<BooleanResult, BooleanError> {
        let has_uvs = self.sources.iter().any(|s| s.mesh.uvs().is_some());
        let mut index = vec![u32::MAX; self.kernel.point_count()];
        let mut points = Vec::new();
        let mut faces: Vec<Vec<u32>> = Vec::new();
        let mut uvs: Vec<Vec2> = Vec::new();
        let mut origins = Vec::new();
        let mut emit = |work: &Self, corners: &[(u32, Vec2)], turned: bool, origin: FaceOrigin| {
            let mut face: Vec<u32> = corners
                .iter()
                .map(|&(p, _)| {
                    let slot = &mut index[us(p)];
                    if *slot == u32::MAX {
                        points.push(work.kernel.approx(p).as_vec3());
                        *slot = u32_index(points.len() - 1);
                    }
                    *slot
                })
                .collect();
            let mut face_uvs: Vec<Vec2> = corners.iter().map(|&(_, uv)| uv).collect();
            if turned {
                // Seen from the other side: wound the other way, and its
                // pictures mirrored back so they read the right way round.
                face.reverse();
                face_uvs.reverse();
                for uv in &mut face_uvs {
                    uv.x = 1.0 - uv.x;
                }
            }
            let mut sorted = face.clone();
            sorted.sort_unstable();
            if face.len() < 3 || sorted.windows(2).any(|w| w[0] == w[1]) {
                return;
            }
            faces.push(face);
            uvs.extend(face_uvs);
            origins.push(origin);
        };
        // Faces that were cut: their kept pieces, merged back per fate.
        let mut by_face: FastMap<(u32, u32, bool), Vec<Corners>> = FastMap::default();
        for (piece, fate) in self.piece_fates.iter().enumerate() {
            if *fate == Fate::Dropped || self.outlines[piece].len() < 3 {
                continue;
            }
            let tri = self.pieces[piece].tri;
            let (slot, t) = self.local(tri);
            let face = self.sources[slot].faces[t];
            let corners = self.outlines[piece]
                .iter()
                .map(|&p| (p, self.uv_on(tri, self.kernel.approx(p))))
                .collect();
            by_face
                .entry((u32_index(slot), face, *fate == Fate::Turned))
                .or_default()
                .push(corners);
        }
        let mut keys: Vec<_> = by_face.keys().copied().collect();
        keys.sort_unstable();
        for key in keys {
            let loops = by_face.remove(&key).unwrap_or_default();
            let (slot, face, turned) = key;
            let origin = FaceOrigin {
                input: self.sources[us(slot)].input,
                face,
            };
            let polygons = if options.merge_faces {
                merge_loops(loops)
            } else {
                loops.into_iter().flat_map(fan).collect()
            };
            for polygon in polygons {
                emit(self, &polygon, turned, origin);
            }
        }
        // Whole faces, with the points their edges share with cut faces.
        let ends: FastSet<u32> = self.along.keys().flat_map(|&(a, b)| [a, b]).collect();
        for slot in 0..2 {
            let source = &self.sources[slot];
            for f in 0..source.mesh.face_count() {
                let fate = self.face_fates[slot][f];
                if fate == Fate::Dropped {
                    continue;
                }
                let range = source.mesh.corner_range(f);
                let mut corners = Vec::with_capacity(range.len());
                for c in range.clone() {
                    let next = if c + 1 == range.end { range.start } else { c + 1 };
                    let (a, b) = (
                        source.points[us(source.mesh.corner_points()[c])],
                        source.points[us(source.mesh.corner_points()[next])],
                    );
                    let (ua, ub) = (source.uv(u32_index(c)), source.uv(u32_index(next)));
                    corners.push((a, ua));
                    if !(ends.contains(&a) && ends.contains(&b)) {
                        continue;
                    }
                    let Some(inside) = self.along.get(&(a.min(b), a.max(b))) else {
                        continue;
                    };
                    let (pa, pb) = (self.kernel.approx(a), self.kernel.approx(b));
                    let length2 = (pb - pa).length_squared().max(f64::MIN_POSITIVE);
                    let mut push = |p: u32| {
                        let t = ((self.kernel.approx(p) - pa).dot(pb - pa) / length2).clamp(0.0, 1.0);
                        corners.push((p, ua.lerp(ub, t as f32)));
                    };
                    if a < b {
                        inside.iter().copied().for_each(&mut push);
                    } else {
                        inside.iter().rev().copied().for_each(&mut push);
                    }
                }
                emit(
                    self,
                    &corners,
                    fate == Fate::Turned,
                    FaceOrigin {
                        input: source.input,
                        face: u32_index(f),
                    },
                );
            }
        }
        // The edges along the cut.
        let mut cut_edges = Vec::new();
        let mut seen = FastSet::default();
        let reverse: FastMap<u32, u32> = index
            .iter()
            .enumerate()
            .filter(|&(_, &i)| i != u32::MAX)
            .map(|(p, &i)| (i, u32_index(p)))
            .collect();
        for face in &faces {
            for (i, &u) in face.iter().enumerate() {
                let v = face[(i + 1) % face.len()];
                let key = (u.min(v), u.max(v));
                let (ku, kv) = (reverse[&key.0], reverse[&key.1]);
                if self.on_surface.contains(&(ku.min(kv), ku.max(kv))) && seen.insert(key) {
                    cut_edges.push([key.0, key.1]);
                }
            }
        }
        let stats = BooleanStats {
            times: [Duration::ZERO; 6],
            pairs: self.pairs,
            pieces: self.pieces.len(),
            whole_faces: self
                .face_fates
                .iter()
                .flatten()
                .filter(|&&f| f != Fate::Dropped)
                .count(),
            exact_calls: self.kernel.exact_calls,
            groups: self.groups,
        };
        let welded = Welded::new(points, faces, uvs, origins, cut_edges, options.weld);
        let mut mesh = PolyMesh::from_faces(welded.points, welded.faces).map_err(|_| BooleanError::TooLarge)?;
        if has_uvs {
            mesh.set_uvs(Some(welded.uvs)).map_err(|_| BooleanError::TooLarge)?;
        }
        Ok(BooleanResult {
            mesh,
            origins: welded.origins,
            cut_edges: welded.cut_edges,
            stats,
        })
    }
}

/// The result's parts with points closer than a tolerance made one: faces
/// that collapse are left out, unused points dropped.
struct Welded {
    points: Vec<glam::Vec3>,
    faces: Vec<Vec<u32>>,
    uvs: Vec<Vec2>,
    origins: Vec<FaceOrigin>,
    cut_edges: Vec<[u32; 2]>,
}

impl Welded {
    fn new(
        points: Vec<glam::Vec3>,
        faces: Vec<Vec<u32>>,
        uvs: Vec<Vec2>,
        origins: Vec<FaceOrigin>,
        cut_edges: Vec<[u32; 2]>,
        tolerance: f32,
    ) -> Self {
        // Each point onto the lowest-index point of its group.
        let mut onto: Vec<u32> = (0..u32_index(points.len())).collect();
        if tolerance > 0.0 {
            let tolerance = f64::from(tolerance);
            let mut grid = Grid::new(tolerance * 2.0);
            for (i, p) in points.iter().enumerate() {
                grid.insert(u32_index(i), p.as_dvec3());
            }
            let mut near = Vec::new();
            for (i, p) in points.iter().enumerate() {
                let p = p.as_dvec3();
                grid.near(p - tolerance, p + tolerance, &mut near);
                for &j in &near {
                    if points[us(j)].as_dvec3().distance(p) <= tolerance {
                        union(&mut onto, u32_index(i), j);
                    }
                }
            }
        }
        let mut new_index = vec![u32::MAX; points.len()];
        let mut out = Self {
            points: Vec::new(),
            faces: Vec::new(),
            uvs: Vec::new(),
            origins: Vec::new(),
            cut_edges: Vec::new(),
        };
        let mut start = 0;
        for (face, origin) in faces.iter().zip(origins) {
            let corner_uvs = uvs.get(start..start + face.len());
            start += face.len();
            let mut corners: Vec<(u32, Vec2)> = Vec::with_capacity(face.len());
            for (k, &p) in face.iter().enumerate() {
                let root = find(&mut onto, p);
                if corners.last().is_some_and(|c| c.0 == root) {
                    continue;
                }
                corners.push((root, corner_uvs.map_or(Vec2::ZERO, |u| u[k])));
            }
            while corners.len() > 1 && corners[0].0 == corners[corners.len() - 1].0 {
                corners.pop();
            }
            // A face pinched at a point becomes the loops on either side of
            // it; a loop of two corners has edges that cancel and is dropped.
            for corners in simple_loops(corners) {
                if corners.len() < 3 {
                    continue;
                }
                let face = corners
                    .iter()
                    .map(|&(root, _)| {
                        if new_index[us(root)] == u32::MAX {
                            new_index[us(root)] = u32_index(out.points.len());
                            out.points.push(points[us(root)]);
                        }
                        new_index[us(root)]
                    })
                    .collect();
                out.faces.push(face);
                out.uvs.extend(corners.iter().map(|c| c.1));
                out.origins.push(origin);
            }
        }
        let mut edges = FastSet::default();
        for face in &out.faces {
            for (i, &u) in face.iter().enumerate() {
                let v = face[(i + 1) % face.len()];
                edges.insert((u.min(v), u.max(v)));
            }
        }
        let mut seen = FastSet::default();
        for [u, v] in cut_edges {
            let (a, b) = (new_index[us(find(&mut onto, u))], new_index[us(find(&mut onto, v))]);
            let key = (a.min(b), a.max(b));
            if a != b && edges.contains(&key) && seen.insert(key) {
                out.cut_edges.push([key.0, key.1]);
            }
        }
        out
    }
}

/// A polygon's corners: points and their UVs.
type Corners = Vec<(u32, Vec2)>;

/// A closed walk of corners split into loops that visit each point once:
/// at a point met again, the stretch since its first visit is cut off.
fn simple_loops(corners: Vec<(u32, Vec2)>) -> Vec<Vec<(u32, Vec2)>> {
    let mut loops = Vec::new();
    let mut stack: Vec<(u32, Vec2)> = Vec::with_capacity(corners.len());
    for corner in corners {
        if let Some(at) = stack.iter().position(|c| c.0 == corner.0) {
            loops.push(stack.split_off(at));
        }
        stack.push(corner);
    }
    loops.push(stack);
    loops
}

/// A convex loop as triangles (a fan from its first corner).
fn fan(corners: Vec<(u32, Vec2)>) -> Vec<Vec<(u32, Vec2)>> {
    (1..corners.len().saturating_sub(1))
        .map(|i| vec![corners[0], corners[i], corners[i + 1]])
        .collect()
}

/// Merges loops of one face that share edges into as few simple polygons as
/// possible (a polygon never visits a point twice).
fn merge_loops(mut loops: Vec<Vec<(u32, Vec2)>>) -> Vec<Vec<(u32, Vec2)>> {
    loop {
        let mut merged = false;
        'search: for i in 0..loops.len() {
            for j in i + 1..loops.len() {
                if let Some(joined) = join_two(&loops[i], &loops[j]) {
                    loops[i] = joined;
                    loops.swap_remove(j);
                    merged = true;
                    break 'search;
                }
            }
        }
        if !merged {
            return loops;
        }
    }
}

/// Two loops joined along an edge they share in opposite directions, if
/// the result is a simple polygon.
fn join_two(x: &[(u32, Vec2)], y: &[(u32, Vec2)]) -> Option<Vec<(u32, Vec2)>> {
    let (n, m) = (x.len(), y.len());
    for i in 0..n {
        let (a, b) = (x[i].0, x[(i + 1) % n].0);
        let Some(j) = (0..m).find(|&j| y[j].0 == b && y[(j + 1) % m].0 == a) else {
            continue;
        };
        // x from b round to a, then y from a round to b (ends left out).
        let mut out: Vec<(u32, Vec2)> = (0..n).map(|k| x[(i + 1 + k) % n]).collect();
        out.extend((1..m - 1).map(|k| y[(j + 1 + k) % m]));
        // Spikes where more edges were shared: p, q, p → p.
        let mut changed = true;
        while changed && out.len() >= 3 {
            changed = false;
            let len = out.len();
            for k in 0..len {
                if out[k].0 == out[(k + 2) % len].0 {
                    let drop = [(k + 1) % len, (k + 2) % len];
                    let (first, second) = (drop[0].max(drop[1]), drop[0].min(drop[1]));
                    out.remove(first);
                    out.remove(second);
                    changed = true;
                    break;
                }
            }
        }
        let mut sorted: Vec<u32> = out.iter().map(|c| c.0).collect();
        sorted.sort_unstable();
        if out.len() >= 3 && sorted.windows(2).all(|w| w[0] != w[1]) {
            return Some(out);
        }
    }
    None
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

fn union(parent: &mut [u32], a: u32, b: u32) {
    let (a, b) = (find(parent, a), find(parent, b));
    if a != b {
        parent[us(a.max(b))] = a.min(b);
    }
}

/// The generalised winding number of an input's surface at `q`: about 1
/// inside a closed, outward-facing mesh, 0 outside, in between near holes.
fn winding_number(source: &Source<'_>, q: DVec3) -> f64 {
    let mut total = 0.0;
    for t in 0..source.corners.len() {
        let [a, b, c] = source.triangle(t).map(|p| p - q);
        let (la, lb, lc) = (a.length(), b.length(), c.length());
        let numerator = a.dot(b.cross(c));
        let denominator = la * lb * lc + a.dot(b) * lc + b.dot(c) * la + c.dot(a) * lb;
        total += 2.0 * numerator.atan2(denominator);
    }
    total / (4.0 * std::f64::consts::PI)
}
