//! Modelling operations: pure functions from a mesh, its [`Topology`] and a
//! selection to a new mesh (see `docs/reference/modeling-tools.md`).
//!
//! * **Extrude and Inset** ([`extrude_polygons`], [`extrude_edges`], [`inset`]) keep
//!   every existing point and face index and append new ones, so selections, hidden
//!   flags and per-corner data stay valid as they are.
//! * **Operations that remove or split components** ([`delete`], [`melt`],
//!   [`dissolve`], [`connect`], [`loop_cut`], [`line_cut`]) return a [`Rebuilt`] mesh
//!   that records where each point and face came from; [`Rebuilt::carry`] moves a
//!   selection (or hidden flags) across.

mod bevel;
mod bridge;
mod cut;
mod extrude;
mod rebuild;

use std::collections::HashSet;

use glam::{Vec2, Vec3};

pub use bevel::{BevelOptions, bevel};
pub use bridge::{BridgeOptions, bridge_edges, bridge_polygons};
pub use cut::{LineCut, connect, line_cut, loop_cut};
pub use extrude::{
    EdgeExtrudeOptions, EdgeExtrusion, ExtrudeOptions, InsetOptions, extrude_edges, extrude_polygons, inset, pieces,
};
pub use rebuild::{OptimizeOptions, delete, dissolve, melt, optimize, weld};

use crate::{BitSet, ComponentSet, MeshError, PolyMesh, Topology, selection, u32_index, us};

/// [`Rebuilt::point_origin`] / [`Rebuilt::face_origin`] of something the operation added.
pub const NEW: u32 = u32::MAX;

/// A mesh rebuilt by an operation that removes or splits components, with where
/// each of its points and faces came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Rebuilt {
    pub mesh: PolyMesh,
    /// The source point of every point; [`NEW`] for an added one.
    pub point_origin: Vec<u32>,
    /// The source face of every face (the pieces of a cut face share it); [`NEW`]
    /// for an added one.
    pub face_origin: Vec<u32>,
    /// Edges the operation cut (point pairs of `mesh`): what "Select Cuts" selects.
    pub new_edges: Vec<[u32; 2]>,
}

impl Rebuilt {
    /// The source mesh's `set` (a selection, or hidden flags) on the rebuilt mesh:
    /// points and faces follow their origin; an edge stays flagged where both its
    /// ends come from the ends of a flagged edge. Nothing new is flagged.
    #[must_use]
    pub fn carry(&self, set: &ComponentSet, source_topology: &Topology) -> ComponentSet {
        let origin = |p: u32| self.point_origin.get(us(p)).copied().unwrap_or(NEW);
        let points = (0..self.mesh.point_count())
            .filter(|&p| self.point_origin[p] != NEW && set.points.contains(us(self.point_origin[p])))
            .collect();
        let polygons = (0..self.mesh.face_count())
            .filter(|&f| self.face_origin[f] != NEW && set.polygons.contains(us(self.face_origin[f])))
            .collect();
        let flagged: HashSet<[u32; 2]> = selection::edges_from_corners(&set.edges, source_topology)
            .iter()
            .map(|e| source_topology.edges()[e])
            .collect();
        let edges = self.corners_where(|a, b| {
            let (a, b) = (origin(a), origin(b));
            a != NEW && b != NEW && flagged.contains(&[a.min(b), a.max(b)])
        });
        ComponentSet {
            points,
            edges,
            polygons,
        }
    }

    /// The corners of the rebuilt mesh whose edge is one of [`Self::new_edges`].
    #[must_use]
    pub fn new_edge_corners(&self) -> BitSet {
        let new: HashSet<[u32; 2]> = self.new_edges.iter().map(|&[a, b]| [a.min(b), a.max(b)]).collect();
        self.corners_where(|a, b| new.contains(&[a.min(b), a.max(b)]))
    }

    /// Corners whose edge `a → b` passes `accept`.
    fn corners_where(&self, accept: impl Fn(u32, u32) -> bool) -> BitSet {
        let mesh = &self.mesh;
        let mut corners = BitSet::new();
        for f in 0..mesh.face_count() {
            for c in mesh.corner_range(f) {
                let (a, b) = (mesh.corner_points()[c], mesh.corner_points()[next_corner(mesh, f, c)]);
                if accept(a, b) {
                    corners.insert(c);
                }
            }
        }
        corners
    }

    /// `self` followed by `then` (built from `self.mesh`): origins go back to the
    /// first source.
    fn then(self, then: Self) -> Self {
        let back = |origins: &[u32], first: &[u32]| {
            origins
                .iter()
                .map(|&o| if o == NEW { NEW } else { first[us(o)] })
                .collect::<Vec<u32>>()
        };
        Self {
            point_origin: back(&then.point_origin, &self.point_origin),
            face_origin: back(&then.face_origin, &self.face_origin),
            new_edges: then.new_edges,
            mesh: then.mesh,
        }
    }
}

/// A corner of a face being built: a corner of the source mesh (its point and UV),
/// or a point (new or old) with its UV.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Corner {
    Old(usize),
    Point(u32, Vec2),
}

/// Collects the faces of a [`Rebuilt`] mesh over the source's points plus new ones.
struct Builder<'a> {
    source: &'a PolyMesh,
    points: Vec<Vec3>,
    corners: Vec<u32>,
    uvs: Vec<Vec2>,
    /// Face `i` is `corners[starts[i]..starts[i + 1]]`.
    starts: Vec<usize>,
    origins: Vec<u32>,
    new_edges: Vec<[u32; 2]>,
    /// Source points to drop even if a face still used them before.
    removed: BitSet,
}

impl<'a> Builder<'a> {
    fn new(source: &'a PolyMesh) -> Self {
        Self {
            source,
            points: source.points().to_vec(),
            corners: Vec::with_capacity(source.corner_count()),
            uvs: Vec::with_capacity(source.corner_count()),
            starts: vec![0],
            origins: Vec::with_capacity(source.face_count()),
            new_edges: Vec::new(),
            removed: BitSet::new(),
        }
    }

    fn add_point(&mut self, point: Vec3) -> u32 {
        self.points.push(point);
        u32_index(self.points.len() - 1)
    }

    fn position(&self, point: u32) -> Vec3 {
        self.points[us(point)]
    }

    /// The UV of source corner `corner` (zero without UVs).
    fn uv(&self, corner: usize) -> Vec2 {
        self.source.uvs().map_or(Vec2::ZERO, |uvs| uvs[corner])
    }

    fn resolve(&self, corner: Corner) -> (u32, Vec2) {
        match corner {
            Corner::Old(c) => (self.source.corner_points()[c], self.uv(c)),
            Corner::Point(p, uv) => (p, uv),
        }
    }

    fn point(&self, corner: Corner) -> u32 {
        self.resolve(corner).0
    }

    /// A point on source corner `corner`'s edge, `t` of the way to the next corner,
    /// with its UV interpolated.
    fn on_edge(&self, face: usize, corner: usize, point: u32, t: f32) -> Corner {
        let next = next_corner(self.source, face, corner);
        Corner::Point(point, self.uv(corner).lerp(self.uv(next), t))
    }

    fn push(&mut self, corners: &[Corner], origin: usize) {
        for &corner in corners {
            let (point, uv) = self.resolve(corner);
            self.corners.push(point);
            self.uvs.push(uv);
        }
        self.starts.push(self.corners.len());
        self.origins.push(u32_index(origin));
    }

    /// A face the operation adds (its origin is [`NEW`]).
    fn push_new(&mut self, corners: &[Corner]) {
        self.push(corners, 0);
        if let Some(origin) = self.origins.last_mut() {
            *origin = NEW;
        }
    }

    /// Source face `face`, unchanged.
    fn keep(&mut self, face: usize) {
        for c in self.source.corner_range(face) {
            self.corners.push(self.source.corner_points()[c]);
            self.uvs.push(self.uv(c));
        }
        self.starts.push(self.corners.len());
        self.origins.push(u32_index(face));
    }

    fn new_edge(&mut self, a: u32, b: u32) {
        self.new_edges.push([a, b]);
    }

    /// The rebuilt mesh. Points no face uses any more are dropped and indices close
    /// up; points the source never used stay, unless `removed`.
    fn finish(self) -> Result<Rebuilt, MeshError> {
        let old = self.source.point_count();
        let mut used = vec![false; self.points.len()];
        for &p in &self.corners {
            used[us(p)] = true;
        }
        let mut used_before = vec![false; old];
        for &p in self.source.corner_points() {
            used_before[us(p)] = true;
        }
        let mut map = vec![NEW; self.points.len()];
        let mut points = Vec::with_capacity(self.points.len());
        let mut point_origin = Vec::with_capacity(self.points.len());
        for (i, &position) in self.points.iter().enumerate() {
            let stray = i < old && !used_before[i] && !self.removed.contains(i);
            if used[i] || stray {
                map[i] = u32_index(points.len());
                points.push(position);
                point_origin.push(if i < old { u32_index(i) } else { NEW });
            }
        }
        let faces = self.starts.windows(2).map(|w| {
            self.corners[w[0]..w[1]]
                .iter()
                .map(|&p| map[us(p)])
                .collect::<Vec<u32>>()
        });
        let mut mesh = PolyMesh::from_faces(points, faces)?;
        if self.source.uvs().is_some() {
            mesh.set_uvs(Some(self.uvs))?;
        }
        let new_edges = self.new_edges.iter().map(|&[a, b]| [map[us(a)], map[us(b)]]).collect();
        Ok(Rebuilt {
            mesh,
            point_origin,
            face_origin: self.origins,
            new_edges,
        })
    }
}

/// The face of every corner.
fn corner_faces(mesh: &PolyMesh) -> Vec<u32> {
    let mut faces = Vec::with_capacity(mesh.corner_count());
    for f in 0..mesh.face_count() {
        faces.extend(mesh.corner_range(f).map(|_| u32_index(f)));
    }
    faces
}

fn next_corner(mesh: &PolyMesh, face: usize, corner: usize) -> usize {
    let range = mesh.corner_range(face);
    if corner + 1 == range.end {
        range.start
    } else {
        corner + 1
    }
}
