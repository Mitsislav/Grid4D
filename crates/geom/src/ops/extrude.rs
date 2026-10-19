//! Extrude and Inset: existing point and face indices are kept, new ones appended.

use std::collections::{HashMap, hash_map::Entry};

use glam::Vec3;

use super::{corner_faces, next_corner};
use crate::{BitSet, MeshError, PolyMesh, Topology, u32_index, us};

/// Polygon Extrude options (see `docs/reference/modeling-tools.md`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtrudeOptions {
    /// Distance along the normals; negative extrudes inwards.
    pub offset: f32,
    /// Extra rows of points along the side walls.
    pub subdivision: u32,
    /// One n-gon per side wall instead of `subdivision + 1` quads.
    pub create_ngons: bool,
    /// Keep a reversed copy of the extruded polygons at the start.
    pub create_caps: bool,
    /// Connected polygons move as one piece.
    pub preserve_groups: bool,
    /// With `preserve_groups`: neighbours whose normals differ by more than this
    /// (radians) are separate pieces.
    pub max_angle: f32,
}

/// Edge Extrude options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeExtrudeOptions {
    pub offset: f32,
    /// Tilt of the new polygons around their edge, towards the polygon's normal (radians).
    pub rotation: f32,
    pub subdivision: u32,
    pub create_ngons: bool,
}

/// Inset (Extrude Inner) options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InsetOptions {
    /// Distance between the original outline and the inset one; negative grows outwards.
    pub offset: f32,
    /// Rings between the outlines (at least 1).
    pub subdivision: u32,
    /// One n-gon per outline edge instead of `subdivision` quads.
    pub create_ngons: bool,
    pub preserve_groups: bool,
    /// See [`ExtrudeOptions::max_angle`].
    pub max_angle: f32,
}

/// The result of [`extrude_edges`]: the mesh and the corners of the new outer
/// edges (what the edge selection becomes).
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeExtrusion {
    pub mesh: PolyMesh,
    pub outer_corners: Vec<u32>,
}

/// Longest a mitred offset may grow, as a multiple of the offset.
const MAX_MITRE: f32 = 4.0;

/// Groups `faces` into pieces: with `preserve_groups`, faces sharing an edge whose
/// normals are within `max_angle` of each other; otherwise one face per piece.
/// Returns each face's piece, `u32::MAX` for faces not in `faces`.
#[must_use]
pub fn pieces(mesh: &PolyMesh, topology: &Topology, faces: &BitSet, preserve_groups: bool, max_angle: f32) -> Vec<u32> {
    let mut piece = vec![u32::MAX; mesh.face_count()];
    let min_dot = max_angle.cos();
    let corner_faces = corner_faces(mesh);
    let mut next = 0u32;
    let mut stack = Vec::new();
    for start in faces.iter().filter(|&f| f < mesh.face_count()) {
        if piece[start] != u32::MAX {
            continue;
        }
        piece[start] = next;
        if preserve_groups {
            stack.push(start);
            while let Some(f) = stack.pop() {
                let normal = mesh.face_normal(f);
                for c in mesh.corner_range(f) {
                    let Some(edge) = topology.corner_edge(c) else { continue };
                    for &other in topology.edge_corners(us(edge)) {
                        let g = us(corner_faces[us(other)]);
                        if faces.contains(g)
                            && piece[g] == u32::MAX
                            && normal.dot(mesh.face_normal(g)) >= min_dot - 1e-6
                        {
                            piece[g] = next;
                            stack.push(g);
                        }
                    }
                }
            }
        }
        next += 1;
    }
    piece
}

/// Extrudes `faces` (see [`ExtrudeOptions`]). Faces out of range are ignored.
pub fn extrude_polygons(
    mesh: &PolyMesh,
    topology: &Topology,
    faces: &BitSet,
    options: &ExtrudeOptions,
) -> Result<PolyMesh, MeshError> {
    let piece = pieces(mesh, topology, faces, options.preserve_groups, options.max_angle);
    let walk = Walk::new(mesh, topology, &piece);
    let selected = || (0..mesh.face_count()).filter(|&f| piece[f] != u32::MAX);

    // Direction of every (piece, point): the normalised sum of the piece's normals
    // there, lengthened so that each of those faces moves by the offset.
    let mut sums: HashMap<(u32, u32), Vec3> = HashMap::new();
    for f in selected() {
        let n = mesh.face_normal(f);
        for &p in mesh.face(f) {
            *sums.entry((piece[f], p)).or_default() += n;
        }
    }
    let mut cosines: HashMap<(u32, u32), f32> = HashMap::new();
    for f in selected() {
        let n = mesh.face_normal(f);
        for &p in mesh.face(f) {
            let dir = sums[&(piece[f], p)].normalize_or_zero();
            let cos = cosines.entry((piece[f], p)).or_insert(1.0);
            *cos = cos.min(dir.dot(n));
        }
    }
    let shift = |key: (u32, u32)| sums[&key].normalize_or_zero() * options.offset * mitre(cosines[&key]);

    let mut out = mesh.clone();
    // The moved copy of each outline point is per junction (the outline corner
    // leaving it: one per time the outline passes the point), so walls stay
    // manifold where a piece touches itself at a point; inside a piece one copy per
    // point. An inside point no other face uses moves itself (no stray points),
    // unless caps keep the original.
    let mut moved: HashMap<Moved, u32> = HashMap::new();
    for f in selected() {
        for c in mesh.corner_range(f) {
            let p = mesh.corner_points()[c];
            let key = walk.junction(c).map_or(Moved::Inside(piece[f], p), Moved::Junction);
            if let Some(&q) = moved.get(&key) {
                out.set_corner_point(c, q);
                continue;
            }
            let exclusive = !options.create_caps
                && matches!(key, Moved::Inside(..))
                && topology.point_faces(us(p)).iter().all(|&g| piece[us(g)] == piece[f]);
            let q = if exclusive {
                out.points_mut()[us(p)] += shift((piece[f], p));
                p
            } else {
                out.add_point(mesh.points()[us(p)] + shift((piece[f], p)))?
            };
            moved.insert(key, q);
            out.set_corner_point(c, q);
        }
    }

    // Side walls along every outline edge; the rows of points between the
    // original and the moved outline are shared by neighbouring walls.
    let mut sides = Sides::default();
    for f in selected() {
        for c in mesh.corner_range(f).filter(|&c| walk.is_outline(c)) {
            let end = walk.next(c);
            let Some(end_junction) = walk.junction(end) else {
                continue;
            };
            let (a, b) = (mesh.corner_points()[c], mesh.corner_points()[end]);
            let rows = options.subdivision + 1;
            let left = sides.side(&mut out, c, a, moved[&Moved::Junction(c)], rows)?;
            let right = sides.side(&mut out, end_junction, b, moved[&Moved::Junction(end_junction)], rows)?;
            add_strip(&mut out, &left, &right, options.create_ngons)?;
        }
    }

    if options.create_caps {
        for f in selected() {
            let reversed: Vec<u32> = mesh.face(f).iter().rev().copied().collect();
            out.add_face(&reversed)?;
        }
    }
    Ok(out)
}

/// Extrudes the border edges among `edges` (edge indices of `topology`) into new
/// polygons (see [`EdgeExtrudeOptions`]). Edges with other than one face are ignored.
pub fn extrude_edges(
    mesh: &PolyMesh,
    topology: &Topology,
    edges: &BitSet,
    options: &EdgeExtrudeOptions,
) -> Result<EdgeExtrusion, MeshError> {
    let corner_faces = corner_faces(mesh);
    // (a, b, direction) of every border edge to extrude, a → b as its face runs.
    let border: Vec<(u32, u32, Vec3)> = edges
        .iter()
        .filter(|&e| topology.edge_faces(e).len() == 1)
        .filter_map(|e| {
            let c = us(*topology.edge_corners(e).first()?);
            let f = us(corner_faces[c]);
            let (a, b) = (mesh.corner_points()[c], mesh.corner_points()[next_corner(mesh, f, c)]);
            let along = (mesh.points()[us(b)] - mesh.points()[us(a)]).normalize_or_zero();
            let normal = mesh.face_normal(f);
            let out = along.cross(normal).normalize_or_zero();
            let (sin, cos) = options.rotation.sin_cos();
            Some((a, b, out * cos + normal * sin))
        })
        .collect();

    let mut sums: HashMap<u32, Vec3> = HashMap::new();
    for &(a, b, dir) in &border {
        *sums.entry(a).or_default() += dir;
        *sums.entry(b).or_default() += dir;
    }
    let mut cosines: HashMap<u32, f32> = HashMap::new();
    for &(a, b, dir) in &border {
        for p in [a, b] {
            let cos = cosines.entry(p).or_insert(1.0);
            *cos = cos.min(sums[&p].normalize_or_zero().dot(dir));
        }
    }
    let shift = |p: u32| sums[&p].normalize_or_zero() * options.offset * mitre(cosines[&p]);

    let mut out = mesh.clone();
    // One new point per extruded point: neighbouring edges share it.
    let mut tops: HashMap<u32, u32> = HashMap::new();
    for &(a, b, _) in &border {
        for p in [a, b] {
            if let Entry::Vacant(slot) = tops.entry(p) {
                slot.insert(out.add_point(mesh.points()[us(p)] + shift(p))?);
            }
        }
    }
    let rows = options.subdivision + 1;
    let mut sides = Sides::default();
    let mut outer_corners = Vec::with_capacity(border.len());
    for &(a, b, _) in &border {
        // The new polygons run b → a along the old edge, continuing its face's winding.
        let left = sides.side(&mut out, b, b, tops[&b], rows)?;
        let right = sides.side(&mut out, a, a, tops[&a], rows)?;
        let first = add_strip(&mut out, &left, &right, options.create_ngons)?;
        // The outer edge a' → b': corner 2 of the last quad [b, a, a', b'], or where
        // the `a` side ends in the n-gon [b, a, …, a', b', …].
        let (face, corner) = if options.create_ngons {
            (first, 1 + rows)
        } else {
            (first + rows - 1, 2)
        };
        outer_corners.push(u32_index(out.corner_range(us(face)).start + us(corner)));
    }
    Ok(EdgeExtrusion {
        mesh: out,
        outer_corners,
    })
}

/// Insets `faces` (see [`InsetOptions`]). Faces out of range are ignored.
pub fn inset(
    mesh: &PolyMesh,
    topology: &Topology,
    faces: &BitSet,
    options: &InsetOptions,
) -> Result<PolyMesh, MeshError> {
    let piece = pieces(mesh, topology, faces, options.preserve_groups, options.max_angle);
    let walk = Walk::new(mesh, topology, &piece);
    let selected = || (0..mesh.face_count()).filter(|&f| piece[f] != u32::MAX);
    let rings = options.subdivision.max(1);

    // Inward perpendicular of an outline corner's edge, in its face's plane.
    let inward = |c: usize| {
        let f = walk.face(c);
        let (a, b) = (mesh.corner_points()[c], mesh.corner_points()[walk.next(c)]);
        let along = (mesh.points()[us(b)] - mesh.points()[us(a)]).normalize_or_zero();
        mesh.face_normal(f).cross(along).normalize_or_zero()
    };
    // The inner copy of an outline point is per junction: the outline corner that
    // leaves the point (one per time the outline passes through it).
    let mut inner: HashMap<usize, u32> = HashMap::new();
    let mut out = mesh.clone();
    for f in selected() {
        for c in mesh.corner_range(f).filter(|&c| walk.is_outline(c)) {
            let Some(before) = walk.previous_outline(c) else {
                continue;
            };
            let (d0, d1) = (inward(before), inward(c));
            let dir = (d0 + d1).normalize_or_zero();
            let length = options.offset * mitre(dir.dot(d0).min(dir.dot(d1)));
            let p = mesh.corner_points()[c];
            inner.insert(c, out.add_point(mesh.points()[us(p)] + dir * length)?);
        }
    }
    // Rewire every corner on the outline to its junction's inner copy.
    for f in selected() {
        for c in mesh.corner_range(f) {
            if let Some(&q) = walk.junction(c).and_then(|j| inner.get(&j)) {
                out.set_corner_point(c, q);
            }
        }
    }
    // Rings along every outline edge, from the original outline to the inner one.
    let mut sides = Sides::default();
    for f in selected() {
        for c in mesh.corner_range(f).filter(|&c| walk.is_outline(c)) {
            let end = walk.next(c);
            let Some(end_junction) = walk.junction(end) else {
                continue;
            };
            let (Some(&a_in), Some(&b_in)) = (inner.get(&c), inner.get(&end_junction)) else {
                continue;
            };
            let (a, b) = (mesh.corner_points()[c], mesh.corner_points()[end]);
            let left = sides.side(&mut out, c, a, a_in, rings)?;
            let right = sides.side(&mut out, end_junction, b, b_in, rings)?;
            add_strip(&mut out, &left, &right, options.create_ngons)?;
        }
    }
    Ok(out)
}

/// Adds the polygons between two sides of points, `left[k]` and `right[k]` from the
/// edge outwards: one quad per step `[left[k], right[k], right[k+1], left[k+1]]`, or
/// one n-gon. Returns the first new face's index.
fn add_strip(out: &mut PolyMesh, left: &[u32], right: &[u32], ngon: bool) -> Result<u32, MeshError> {
    let first = u32_index(out.face_count());
    if ngon {
        let face: Vec<u32> = left[..1]
            .iter()
            .chain(right)
            .chain(left[1..].iter().rev())
            .copied()
            .collect();
        out.add_face(&face)?;
    } else {
        for k in 0..left.len().saturating_sub(1) {
            out.add_face(&[left[k], right[k], right[k + 1], left[k + 1]])?;
        }
    }
    Ok(first)
}

/// Which moved copy of a point a corner of an extruded piece uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Moved {
    /// On the outline: the junction corner (see `Walk::junction`).
    Junction(usize),
    /// Inside piece `.0`: point `.1`.
    Inside(u32, u32),
}

/// Columns of points from a bottom point to a top point, created once per key and
/// shared by the strips on both sides of them.
struct Sides<K> {
    points: HashMap<(K, u32), u32>,
}

impl<K> Default for Sides<K> {
    fn default() -> Self {
        Self { points: HashMap::new() }
    }
}

impl<K: Copy + Eq + std::hash::Hash> Sides<K> {
    /// `steps + 1` points from `bottom` to `top`; the ones between are evenly spaced.
    fn side(&mut self, out: &mut PolyMesh, key: K, bottom: u32, top: u32, steps: u32) -> Result<Vec<u32>, MeshError> {
        let (from, to) = (out.points()[us(bottom)], out.points()[us(top)]);
        (0..=steps)
            .map(|k| {
                if k == 0 {
                    return Ok(bottom);
                }
                if k == steps {
                    return Ok(top);
                }
                if let Some(&q) = self.points.get(&(key, k)) {
                    return Ok(q);
                }
                let q = out.add_point(from.lerp(to, fraction(k, steps)))?;
                self.points.insert((key, k), q);
                Ok(q)
            })
            .collect()
    }
}

/// `1 / cos`, so that an offset along a mitred direction keeps its distance from
/// each plane it averages; capped at [`MAX_MITRE`].
fn mitre(cos: f32) -> f32 {
    if cos <= 1.0 / MAX_MITRE { MAX_MITRE } else { 1.0 / cos }
}

/// `k / n` as `f32` (`n > 0`).
fn fraction(k: u32, n: u32) -> f32 {
    let f = |v: u32| f32::from(u16::try_from(v).unwrap_or(u16::MAX));
    f(k) / f(n)
}

/// Walks the corners of a set of pieces: their outlines and the faces around a point.
struct Walk<'a> {
    mesh: &'a PolyMesh,
    topology: &'a Topology,
    piece: &'a [u32],
    corner_faces: Vec<u32>,
}

impl<'a> Walk<'a> {
    fn new(mesh: &'a PolyMesh, topology: &'a Topology, piece: &'a [u32]) -> Self {
        Self {
            mesh,
            topology,
            piece,
            corner_faces: corner_faces(mesh),
        }
    }

    fn face(&self, corner: usize) -> usize {
        us(self.corner_faces[corner])
    }

    fn next(&self, corner: usize) -> usize {
        next_corner(self.mesh, self.face(corner), corner)
    }

    /// The corner of the same piece across `corner`'s edge (running the other way).
    fn twin(&self, corner: usize) -> Option<usize> {
        let piece = self.piece[self.face(corner)];
        let edge = self.topology.corner_edge(corner)?;
        self.topology
            .edge_corners(us(edge))
            .iter()
            .map(|&c| us(c))
            .find(|&c| c != corner && self.piece[self.face(c)] == piece)
    }

    /// `corner`'s edge has no neighbour in the corner's piece.
    fn is_outline(&self, corner: usize) -> bool {
        self.twin(corner).is_none()
    }

    /// The outline corner leaving `corner`'s point that `corner` belongs to: turn
    /// around the point (across each inner edge into the next face) until the edge
    /// is on the outline. `None` for a point inside the piece.
    fn junction(&self, corner: usize) -> Option<usize> {
        let mut c = corner;
        for _ in 0..=self.topology.point_faces(us(self.mesh.corner_points()[corner])).len() {
            if self.is_outline(c) {
                return Some(c);
            }
            c = self.next(self.twin(c)?);
        }
        None
    }

    /// The outline corner arriving at outline corner `corner`'s point (its
    /// predecessor along the outline).
    fn previous_outline(&self, corner: usize) -> Option<usize> {
        let piece = self.piece[self.face(corner)];
        let point = self.mesh.corner_points()[corner];
        self.topology.point_faces(us(point)).iter().find_map(|&f| {
            let f = us(f);
            if self.piece[f] != piece {
                return None;
            }
            self.mesh.corner_range(f).find(|&c| {
                self.is_outline(c)
                    && self.mesh.corner_points()[self.next(c)] == point
                    && self.junction(self.next(c)) == Some(corner)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::PI;

    use super::*;
    use crate::shapes;

    const EXTRUDE: ExtrudeOptions = ExtrudeOptions {
        offset: 50.0,
        subdivision: 0,
        create_ngons: false,
        create_caps: false,
        preserve_groups: true,
        max_angle: 89.0 * PI / 180.0,
    };
    const INSET: InsetOptions = InsetOptions {
        offset: 0.25,
        subdivision: 1,
        create_ngons: false,
        preserve_groups: true,
        max_angle: 89.0 * PI / 180.0,
    };
    const EDGES: EdgeExtrudeOptions = EdgeExtrudeOptions {
        offset: 2.0,
        rotation: 0.0,
        subdivision: 0,
        create_ngons: false,
    };

    fn set(items: &[usize]) -> BitSet {
        items.iter().copied().collect()
    }

    fn face_with_normal(mesh: &PolyMesh, normal: Vec3) -> usize {
        (0..mesh.face_count())
            .find(|&f| mesh.face_normal(f).abs_diff_eq(normal, 1e-5))
            .unwrap()
    }

    /// Manifold and consistently wound, with no point left unused.
    fn assert_clean(mesh: &PolyMesh) -> Topology {
        let topology = Topology::build(mesh);
        assert!(topology.is_manifold(), "manifold and consistently oriented");
        for p in 0..mesh.point_count() {
            assert!(!topology.point_faces(p).is_empty(), "point {p} is unused");
        }
        topology
    }

    #[test]
    fn extruding_a_cube_face_raises_it_and_builds_four_walls() {
        let cube = shapes::cube(100.0);
        let top = face_with_normal(&cube, Vec3::Y);
        let out = extrude_polygons(&cube, &Topology::build(&cube), &set(&[top]), &EXTRUDE).unwrap();
        assert_eq!((out.point_count(), out.face_count()), (12, 10));
        let topology = assert_clean(&out);
        assert!(topology.is_closed());
        assert_eq!(topology.euler_characteristic(), 2);
        let raised = out.face_center(top) - cube.face_center(top);
        assert!(raised.abs_diff_eq(Vec3::Y * 50.0, 1e-4), "{raised}");
        assert!(
            out.face_normal(top).abs_diff_eq(Vec3::Y, 1e-5),
            "the moved face keeps its index"
        );
        for wall in 6..10 {
            let outward = (out.face_center(wall) * Vec3::new(1.0, 0.0, 1.0)).normalize();
            assert!(
                out.face_normal(wall).abs_diff_eq(outward, 1e-5),
                "wall {wall} faces out"
            );
        }
    }

    #[test]
    fn preserve_groups_extrudes_connected_faces_as_one_piece() {
        let plane = shapes::grid(2, 1);
        let topology = Topology::build(&plane);
        let both = set(&[0, 1]);
        let joined = extrude_polygons(&plane, &topology, &both, &EXTRUDE).unwrap();
        assert_eq!(joined.face_count(), 2 + 6, "walls only around the outline");
        assert_eq!(joined.point_count(), 6 + 6);
        assert_clean(&joined);
        let options = ExtrudeOptions {
            preserve_groups: false,
            ..EXTRUDE
        };
        let apart = extrude_polygons(&plane, &topology, &both, &options).unwrap();
        assert_eq!(apart.face_count(), 2 + 8, "every face walled on its own");
        assert_eq!(apart.point_count(), 6 + 8, "the shared edge is doubled");
        assert_clean(&apart);
    }

    #[test]
    fn maximum_angle_splits_pieces_and_mitres_corners() {
        let cube = shapes::cube(100.0);
        let topology = Topology::build(&cube);
        let (top, front) = (face_with_normal(&cube, Vec3::Y), face_with_normal(&cube, Vec3::Z));
        let faces = set(&[top, front]);
        let split_pieces = pieces(&cube, &topology, &faces, true, EXTRUDE.max_angle);
        assert_ne!(split_pieces[top], split_pieces[front]);
        let split = extrude_polygons(&cube, &topology, &faces, &EXTRUDE).unwrap();
        assert_eq!(split.face_count(), 6 + 8, "two pieces, four walls each");
        assert_clean(&split);

        let options = ExtrudeOptions {
            max_angle: PI,
            ..EXTRUDE
        };
        let one = extrude_polygons(&cube, &topology, &faces, &options).unwrap();
        assert_eq!(one.face_count(), 6 + 6, "one piece: the shared edge has no wall");
        assert_clean(&one);
        for (face, normal) in [(top, Vec3::Y), (front, Vec3::Z)] {
            let moved = (one.face_center(face) - cube.face_center(face)).dot(normal);
            assert!((moved - 50.0).abs() < 1e-3, "face moves the full offset, got {moved}");
        }
    }

    #[test]
    fn subdivision_adds_rows_and_ngons_merge_them() {
        let cube = shapes::cube(100.0);
        let topology = Topology::build(&cube);
        let top = set(&[face_with_normal(&cube, Vec3::Y)]);
        let options = ExtrudeOptions {
            subdivision: 2,
            ..EXTRUDE
        };
        let rows = extrude_polygons(&cube, &topology, &top, &options).unwrap();
        assert_eq!(rows.face_count(), 6 + 4 * 3);
        assert_eq!(rows.point_count(), 8 + 4 + 4 * 2);
        assert_clean(&rows);
        let options = ExtrudeOptions {
            subdivision: 2,
            create_ngons: true,
            ..EXTRUDE
        };
        let ngons = extrude_polygons(&cube, &topology, &top, &options).unwrap();
        assert_eq!(ngons.face_count(), 6 + 4);
        assert_eq!(ngons.point_count(), rows.point_count(), "the same points, fewer edges");
        assert!(ngons.faces().skip(6).all(|f| f.len() == 8));
        assert_clean(&ngons);
    }

    #[test]
    fn caps_close_an_extruded_plane() {
        let quad = shapes::grid(1, 1);
        let options = ExtrudeOptions {
            create_caps: true,
            ..EXTRUDE
        };
        let out = extrude_polygons(&quad, &Topology::build(&quad), &set(&[0]), &options).unwrap();
        assert_eq!((out.point_count(), out.face_count()), (8, 6));
        let topology = assert_clean(&out);
        assert!(topology.is_closed(), "a box");
        assert!(out.face_normal(5).abs_diff_eq(-Vec3::Y, 1e-5), "the cap faces down");
    }

    #[test]
    fn extruding_a_whole_closed_mesh_only_inflates_it() {
        let cube = shapes::cube(100.0);
        let all = set(&[0, 1, 2, 3, 4, 5]);
        let options = ExtrudeOptions {
            max_angle: PI,
            ..EXTRUDE
        };
        let out = extrude_polygons(&cube, &Topology::build(&cube), &all, &options).unwrap();
        assert_eq!((out.point_count(), out.face_count()), (8, 6), "no outline, no walls");
        let bounds = out.bounds().unwrap();
        assert!(
            bounds.0.abs_diff_eq(Vec3::splat(-100.0), 1e-3),
            "every face moved by 50: {bounds:?}"
        );
        assert!(bounds.1.abs_diff_eq(Vec3::splat(100.0), 1e-3));
    }

    #[test]
    fn negative_offsets_extrude_inwards() {
        let cube = shapes::cube(100.0);
        let top = face_with_normal(&cube, Vec3::Y);
        let options = ExtrudeOptions {
            offset: -20.0,
            ..EXTRUDE
        };
        let out = extrude_polygons(&cube, &Topology::build(&cube), &set(&[top]), &options).unwrap();
        assert!((out.face_center(top).y - 30.0).abs() < 1e-4);
        assert_clean(&out);
    }

    #[test]
    fn border_edges_grow_flaps_that_share_their_corner_points() {
        let quad = shapes::grid(1, 1);
        let topology = Topology::build(&quad);
        let edge = |a: u32, b: u32| {
            topology
                .edges()
                .iter()
                .position(|&e| e == [a.min(b), a.max(b)])
                .unwrap()
        };
        // Points: 0 (0,0) 1 (1,0) 2 (0,1) 3 (1,1) as (x, z); the face runs 0 → 2 → 3 → 1.
        let one = extrude_edges(&quad, &topology, &set(&[edge(0, 2)]), &EDGES).unwrap();
        assert_eq!((one.mesh.point_count(), one.mesh.face_count()), (6, 2));
        assert_clean(&one.mesh);
        let flap = one.mesh.face_center(1);
        assert!(
            flap.abs_diff_eq(Vec3::new(-1.0, 0.0, 0.5), 1e-5),
            "beside the edge, in the plane: {flap}"
        );
        assert!(
            one.mesh.face_normal(1).abs_diff_eq(Vec3::Y, 1e-5),
            "continues the face's winding"
        );
        let [outer] = one.outer_corners[..] else {
            panic!("one outer edge")
        };
        let range = one.mesh.corner_range(1);
        let (c, next) = (us(outer), range.start + (us(outer) - range.start + 1) % range.len());
        let ends = [c, next].map(|c| one.mesh.points()[us(one.mesh.corner_points()[c])]);
        assert!(
            ends.iter().all(|p| (p.x + 2.0).abs() < 1e-5),
            "the outer edge: {ends:?}"
        );

        let corner = extrude_edges(&quad, &topology, &set(&[edge(0, 2), edge(0, 1)]), &EDGES).unwrap();
        assert_eq!(corner.mesh.point_count(), 4 + 3, "the corner point is shared");
        assert_clean(&corner.mesh);
        let options = EdgeExtrudeOptions {
            rotation: PI / 2.0,
            ..EDGES
        };
        let tilted = extrude_edges(&quad, &topology, &set(&[edge(0, 2)]), &options).unwrap();
        assert!(
            tilted.mesh.points()[4].abs_diff_eq(Vec3::new(0.0, 2.0, 0.0), 1e-5),
            "90° points along the normal"
        );

        let plane = shapes::grid(2, 1);
        let all = set(&(0..Topology::build(&plane).edge_count()).collect::<Vec<_>>());
        let border = extrude_edges(&plane, &Topology::build(&plane), &all, &EDGES).unwrap();
        assert_eq!(border.mesh.face_count(), 2 + 6, "the inner edge has no free side");
        assert_clean(&border.mesh);
    }

    #[test]
    fn inset_shrinks_a_quad_inside_its_outline() {
        let quad = shapes::grid(1, 1);
        let topology = Topology::build(&quad);
        let out = inset(&quad, &topology, &set(&[0]), &INSET).unwrap();
        assert_eq!((out.point_count(), out.face_count()), (8, 5));
        assert_clean(&out);
        for &p in out.face(0) {
            let p = out.points()[us(p)];
            assert!(
                ((p.x - 0.5).abs() - 0.25).abs() < 1e-5 && ((p.z - 0.5).abs() - 0.25).abs() < 1e-5,
                "{p}"
            );
        }
        assert!((0..5).all(|f| out.face_normal(f).abs_diff_eq(Vec3::Y, 1e-5)));
        let options = InsetOptions {
            subdivision: 2,
            ..INSET
        };
        let rings = inset(&quad, &topology, &set(&[0]), &options).unwrap();
        assert_eq!((rings.point_count(), rings.face_count()), (12, 9));
        assert_clean(&rings);
        let options = InsetOptions {
            subdivision: 2,
            create_ngons: true,
            ..INSET
        };
        let ngons = inset(&quad, &topology, &set(&[0]), &options).unwrap();
        assert_eq!((ngons.point_count(), ngons.face_count()), (12, 5));
        assert_clean(&ngons);
    }

    #[test]
    fn inset_groups_keep_shared_edges_inside() {
        let plane = shapes::grid(2, 1);
        let topology = Topology::build(&plane);
        let joined = inset(&plane, &topology, &set(&[0, 1]), &INSET).unwrap();
        assert_eq!(joined.face_count(), 2 + 6, "a ring around the outline only");
        assert_clean(&joined);
        let options = InsetOptions {
            preserve_groups: false,
            ..INSET
        };
        let apart = inset(&plane, &topology, &set(&[0, 1]), &options).unwrap();
        assert_eq!(apart.face_count(), 2 + 8);
        assert_clean(&apart);
    }

    #[test]
    fn an_outline_through_one_point_twice_gets_two_copies() {
        // 4 × 4 grid: a path of faces from (1,1) round to (2,2); the two only touch at
        // their shared corner point, which the outline passes twice.
        let plane = shapes::grid(4, 4);
        let topology = Topology::build(&plane);
        let face = |i: usize, j: usize| j * 4 + i;
        let path = set(&[
            face(1, 1),
            face(1, 0),
            face(2, 0),
            face(3, 0),
            face(3, 1),
            face(3, 2),
            face(2, 2),
        ]);
        let out = inset(&plane, &topology, &path, &INSET).unwrap();
        assert_clean(&out);
        let outline = 7 * 4 - 2 * 6; // every face's edges minus both sides of the 6 shared ones
        assert_eq!(out.face_count(), 16 + outline);
        assert_eq!(out.point_count(), 25 + outline, "one inner point per outline corner");
        assert_clean(&extrude_polygons(&plane, &topology, &path, &EXTRUDE).unwrap());
    }

    #[test]
    fn faces_out_of_range_are_ignored() {
        let quad = shapes::grid(1, 1);
        let topology = Topology::build(&quad);
        assert_eq!(extrude_polygons(&quad, &topology, &set(&[7]), &EXTRUDE).unwrap(), quad);
        assert_eq!(inset(&quad, &topology, &set(&[7]), &INSET).unwrap(), quad);
        assert_eq!(extrude_edges(&quad, &topology, &set(&[99]), &EDGES).unwrap().mesh, quad);
    }
}
