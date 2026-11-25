//! Mirror, Array, Extrude along Spline and the Polygon Pen's polygons (spec
//! `docs/reference/modeling-tools-2.md`).

use std::collections::HashMap;

use glam::{EulerRot, IVec3, Mat4, Quat, Vec2, Vec3};

use super::{Builder, Corner, Rebuilt, corner_faces, next_corner, pieces};
use crate::{BitSet, MeshError, PolyMesh, Topology, spline::Polyline, u32_index, us};

/// The faces an operation works on: `faces`, or every face when `None`.
fn scope(mesh: &PolyMesh, faces: Option<&BitSet>) -> BitSet {
    match faces {
        Some(set) => set.iter().filter(|&f| f < mesh.face_count()).collect(),
        None => (0..mesh.face_count()).collect(),
    }
}

/// The points the faces `faces` use, in order.
fn points_of(mesh: &PolyMesh, faces: &BitSet) -> Vec<u32> {
    let mut seen = BitSet::new();
    let mut out = Vec::new();
    for f in faces.iter() {
        for &p in mesh.face(f) {
            if !seen.contains(us(p)) {
                seen.insert(us(p));
                out.push(p);
            }
        }
    }
    out
}

/// A Mirror: across the plane through `origin` with normal `normal` (the
/// mesh's space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mirror {
    pub origin: Vec3,
    pub normal: Vec3,
    /// Keep the original and add the mirror image; off: turn it over.
    pub duplicate: bool,
    /// Points of the image within `tolerance` of the plane are the
    /// original's.
    pub weld: bool,
    pub tolerance: f32,
}

/// Mirrors `faces` (every face when `None`): adds their image, wound so it
/// faces out, its UVs' U turned round so pictures do not read mirrored; or,
/// without Duplicate, moves them to their image. The image's faces are the
/// result's new faces.
pub fn mirror(mesh: &PolyMesh, faces: Option<&BitSet>, options: &Mirror) -> Result<Rebuilt, MeshError> {
    let mut builder = Builder::new(mesh);
    let normal = options.normal.normalize_or_zero();
    let faces = scope(mesh, faces);
    if normal == Vec3::ZERO {
        for f in 0..mesh.face_count() {
            builder.keep(f);
        }
        return builder.finish();
    }
    let reflect = |p: Vec3| p - normal * (2.0 * (p - options.origin).dot(normal));
    let on_plane = |p: Vec3| options.weld && (p - options.origin).dot(normal).abs() <= options.tolerance.max(0.0);
    let mut image: HashMap<u32, u32> = HashMap::new();
    for p in points_of(mesh, &faces) {
        let at = mesh.points()[us(p)];
        let q = if on_plane(at) {
            p
        } else if options.duplicate {
            builder.add_point(reflect(at))
        } else {
            builder.points[us(p)] = reflect(at);
            p
        };
        image.insert(p, q);
    }
    let uvs = mesh.uvs();
    let turned = |c: usize| {
        let uv = uvs.map_or(Vec2::ZERO, |uvs| uvs[c]);
        Vec2::new(1.0 - uv.x, uv.y)
    };
    let mut corners = Vec::new();
    for f in 0..mesh.face_count() {
        if !faces.contains(f) {
            builder.keep(f);
            continue;
        }
        corners.clear();
        corners.extend(
            mesh.corner_range(f)
                .rev()
                .map(|c| Corner::Point(image[&mesh.corner_points()[c]], turned(c))),
        );
        if options.duplicate {
            builder.keep(f);
            builder.push_new(&corners);
        } else {
            builder.push(&corners, f);
        }
    }
    builder.finish()
}

/// An Array: `copies` copies, each `offset` further, turned by `rotation`
/// (H, P, B radians) and scaled by `scale` more than the last, about the
/// copied faces' centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Array {
    pub copies: u32,
    pub offset: Vec3,
    pub rotation: Vec3,
    pub scale: f32,
    /// Points of neighbouring copies within 0.01 cm become one.
    pub weld: bool,
}

impl Array {
    pub const MAX_COPIES: u32 = 1000;
}

/// Copies `faces` (every face when `None`) as new pieces (the result's new
/// faces).
pub fn array(mesh: &PolyMesh, faces: Option<&BitSet>, options: &Array) -> Result<Rebuilt, MeshError> {
    const WELD: f32 = 0.01;
    let mut builder = Builder::new(mesh);
    let faces = scope(mesh, faces);
    for f in 0..mesh.face_count() {
        builder.keep(f);
    }
    let points = points_of(mesh, &faces);
    if points.is_empty() {
        return builder.finish();
    }
    let centre = points.iter().map(|&p| mesh.points()[us(p)]).sum::<Vec3>() / points.len() as f32;
    let turn = Quat::from_euler(
        EulerRot::YXZ,
        options.rotation.y,
        options.rotation.x,
        options.rotation.z,
    );
    let step = Mat4::from_translation(options.offset)
        * Mat4::from_translation(centre)
        * Mat4::from_scale_rotation_translation(Vec3::splat(options.scale.max(0.0)), turn, Vec3::ZERO)
        * Mat4::from_translation(-centre);
    let cell = |p: Vec3| (p / WELD).round().as_ivec3();
    // The previous copy's points by place (for welding) and by source point.
    let mut previous: HashMap<u32, u32> = points.iter().map(|&p| (p, p)).collect();
    let mut matrix = Mat4::IDENTITY;
    let mut corners = Vec::new();
    for _ in 0..options.copies.min(Array::MAX_COPIES) {
        matrix = step * matrix;
        let near: HashMap<IVec3, u32> = if options.weld {
            previous.values().map(|&q| (cell(builder.position(q)), q)).collect()
        } else {
            HashMap::new()
        };
        let mut current = HashMap::new();
        for &p in &points {
            let at = matrix.transform_point3(mesh.points()[us(p)]);
            let q = match near.get(&cell(at)) {
                Some(&q) if builder.position(q).distance(at) <= WELD => q,
                _ => builder.add_point(at),
            };
            current.insert(p, q);
        }
        for f in faces.iter() {
            corners.clear();
            corners.extend(
                mesh.corner_range(f)
                    .map(|c| Corner::Point(current[&mesh.corner_points()[c]], builder.uv(c))),
            );
            builder.push_new(&corners);
        }
        previous = current;
    }
    builder.finish()
}

/// Extrude along Spline's options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlongSpline {
    /// Segments along the path (1 – 1000).
    pub steps: u32,
    /// Turn over the whole path (radians).
    pub twist: f32,
    /// The profile's size at the end (1 = as it is).
    pub end_scale: f32,
    /// The profile turns with the path; off: it only moves.
    pub follow: bool,
}

impl AlongSpline {
    pub const MAX_STEPS: u32 = 1000;
}

impl Default for AlongSpline {
    fn default() -> Self {
        Self {
            steps: 10,
            twist: 0.0,
            end_scale: 1.0,
            follow: true,
        }
    }
}

/// `path` resampled at `steps + 1` equal lengths: the places and the
/// directions there. `None` if it has no length.
fn resample(path: &Polyline, steps: u32) -> Option<(Vec<Vec3>, Vec<Vec3>)> {
    let (points, closed) = path.runs().next()?;
    let mut line: Vec<Vec3> = points.iter().copied().filter(|p| p.is_finite()).collect();
    if closed && let Some(&first) = line.first() {
        line.push(first);
    }
    let lengths: Vec<f32> = line.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f32 = lengths.iter().sum();
    if line.len() < 2 || total <= 1e-6 {
        return None;
    }
    let mut places = Vec::new();
    let mut directions = Vec::new();
    let (mut segment, mut start) = (0usize, 0.0f32);
    for k in 0..=steps {
        let at = total * k as f32 / steps as f32;
        while segment + 1 < lengths.len() && start + lengths[segment] < at {
            start += lengths[segment];
            segment += 1;
        }
        let t = if lengths[segment] > 0.0 {
            ((at - start) / lengths[segment]).clamp(0.0, 1.0)
        } else {
            0.0
        };
        places.push(line[segment].lerp(line[segment + 1], t));
        directions.push((line[segment + 1] - line[segment]).normalize_or(Vec3::Z));
    }
    Some((places, directions))
}

/// Extrudes each region of `faces` (faces sharing edges) along `path` (in
/// the mesh's space): the path starts at the region's centre, along its
/// normal; `steps` rings of side polygons; the region itself moves to the
/// end. Existing point and face numbers are kept, the new ones appended (so
/// a selection of the region selects the moved end). A path without length
/// changes nothing.
pub fn extrude_along(
    mesh: &PolyMesh,
    topology: &Topology,
    faces: &BitSet,
    path: &Polyline,
    options: &AlongSpline,
) -> Result<PolyMesh, MeshError> {
    let steps = options.steps.clamp(1, AlongSpline::MAX_STEPS);
    let Some((places, directions)) = resample(path, steps) else {
        return Ok(mesh.clone());
    };
    let groups = pieces(mesh, topology, faces, true, std::f32::consts::PI);
    let corner_faces = corner_faces(mesh);
    let mut points = mesh.points().to_vec();
    let mut new_faces: Vec<Vec<u32>> = mesh.faces().map(<[u32]>::to_vec).collect();
    let mut uvs = mesh.uvs().map(<[Vec2]>::to_vec);
    let regions = groups.iter().filter(|&&g| g != u32::MAX).max().map_or(0, |&g| g + 1);
    for region in 0..regions {
        let members: BitSet = (0..mesh.face_count()).filter(|&f| groups[f] == region).collect();
        let used = points_of(mesh, &members);
        let centre = used.iter().map(|&p| mesh.points()[us(p)]).sum::<Vec3>() / used.len().max(1) as f32;
        let normal = members
            .iter()
            .map(|f| mesh.face_vector(f))
            .sum::<Vec3>()
            .normalize_or(Vec3::Y);
        // The region's border, each edge a → b as its face runs.
        let mut border: Vec<(u32, u32)> = Vec::new();
        for f in members.iter() {
            for c in mesh.corner_range(f) {
                let Some(e) = topology.corner_edge(c) else { continue };
                let inside = topology
                    .edge_corners(us(e))
                    .iter()
                    .filter(|&&o| members.contains(us(corner_faces[us(o)])))
                    .count();
                if inside == 1 {
                    border.push((mesh.corner_points()[c], mesh.corner_points()[next_corner(mesh, f, c)]));
                }
            }
        }
        let start = Quat::from_rotation_arc(directions[0], normal);
        let mut frame = Quat::IDENTITY;
        let mut rings: HashMap<(u32, u32), u32> = HashMap::new();
        let ring_points: Vec<u32> = {
            let mut list: Vec<u32> = border.iter().flat_map(|&(a, b)| [a, b]).collect();
            list.sort_unstable();
            list.dedup();
            list
        };
        let place = |k: usize, frame: Quat, p: Vec3| {
            let share = k as f32 / steps as f32;
            let local = Quat::from_axis_angle(normal, options.twist * share)
                * ((p - centre) * (1.0 + (options.end_scale.max(0.0) - 1.0) * share));
            let turn = if options.follow {
                start * frame * start.inverse()
            } else {
                Quat::IDENTITY
            };
            centre + start * (places[k] - places[0]) + turn * local
        };
        for k in 1..=us(steps) {
            if options.follow {
                frame = (Quat::from_rotation_arc(directions[k - 1], directions[k]) * frame).normalize();
            }
            for &p in &ring_points {
                points.push(place(k, frame, mesh.points()[us(p)]));
                rings.insert((p, u32_index(k)), u32_index(points.len() - 1));
            }
            if k == us(steps) {
                // Inner points move with the end.
                for &p in &used {
                    if ring_points.binary_search(&p).is_err() {
                        points[us(p)] = place(k, frame, mesh.points()[us(p)]);
                    }
                }
            }
        }
        let ring = |p: u32, k: u32| if k == 0 { p } else { rings[&(p, k)] };
        // The region's faces become the end: their border points the last ring's.
        for f in members.iter() {
            for corner in &mut new_faces[f] {
                if ring_points.binary_search(corner).is_ok() {
                    *corner = ring(*corner, steps);
                }
            }
        }
        for &(a, b) in &border {
            for k in 1..=steps {
                new_faces.push(vec![ring(a, k - 1), ring(b, k - 1), ring(b, k), ring(a, k)]);
                if let Some(uvs) = &mut uvs {
                    let (v0, v1) = ((k - 1) as f32 / steps as f32, k as f32 / steps as f32);
                    uvs.extend([
                        Vec2::new(0.0, v0),
                        Vec2::new(1.0, v0),
                        Vec2::new(1.0, v1),
                        Vec2::new(0.0, v1),
                    ]);
                }
            }
        }
    }
    let mut out = PolyMesh::from_faces(points, new_faces)?;
    out.set_uvs(uvs)?;
    Ok(out)
}

/// A corner of a polygon drawn with the Polygon Pen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PenCorner {
    /// An existing point.
    Point(u32),
    /// A new point on unique edge `edge`, `t` of the way from its first
    /// point; the faces on the edge gain it.
    Edge { edge: usize, t: f32 },
    /// A new point anywhere.
    New(Vec3),
}

/// Adds the polygon through `corners` (three or more), wound to agree with
/// a face it shares an edge with. Its face is the result's new face.
pub fn pen_polygon(mesh: &PolyMesh, topology: &Topology, corners: &[PenCorner]) -> Result<Rebuilt, MeshError> {
    let mut builder = Builder::new(mesh);
    let mut on_edge: HashMap<usize, (u32, f32)> = HashMap::new();
    let mut polygon: Vec<u32> = Vec::with_capacity(corners.len());
    for &corner in corners {
        let p = match corner {
            PenCorner::Point(p) if us(p) < mesh.point_count() => p,
            PenCorner::Point(_) => continue,
            PenCorner::Edge { edge, t } => {
                let Some(&[a, b]) = topology.edges().get(edge) else {
                    continue;
                };
                let at = mesh.points()[us(a)].lerp(mesh.points()[us(b)], t);
                let q = builder.add_point(at);
                on_edge.insert(edge, (q, t));
                q
            }
            PenCorner::New(at) if at.is_finite() => builder.add_point(at),
            PenCorner::New(_) => continue,
        };
        if polygon.last() != Some(&p) && polygon.first() != Some(&p) {
            polygon.push(p);
        }
    }
    let mut row = Vec::new();
    for f in 0..mesh.face_count() {
        row.clear();
        for c in mesh.corner_range(f) {
            row.push(Corner::Old(c));
            let Some(e) = topology.corner_edge(c).map(us) else {
                continue;
            };
            if let Some(&(q, t)) = on_edge.get(&e) {
                let forward = topology.edges()[e][0] == mesh.corner_points()[c];
                row.push(builder.on_edge(f, c, q, if forward { t } else { 1.0 - t }));
            }
        }
        builder.push(&row, f);
    }
    if polygon.len() >= 3 {
        // An edge it shares with a face must run the other way there.
        let runs = |a: u32, b: u32| {
            (0..mesh.face_count()).any(|f| {
                mesh.corner_range(f)
                    .any(|c| mesh.corner_points()[c] == a && mesh.corner_points()[next_corner(mesh, f, c)] == b)
            })
        };
        let n = polygon.len();
        if (0..n).any(|i| runs(polygon[i], polygon[(i + 1) % n])) {
            polygon.reverse();
        }
        let corners: Vec<Corner> = polygon.iter().map(|&p| Corner::Point(p, Vec2::ZERO)).collect();
        builder.push_new(&corners);
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{primitives, spline::Segment};

    fn half_cube() -> PolyMesh {
        let cube = primitives::cube(Vec3::splat(100.0), [2, 1, 1]);
        cube.with_faces_where(|f| cube.face_center(f).x > 0.0)
    }

    #[test]
    fn mirroring_half_a_cube_welded_closes_it() {
        let half = half_cube();
        let options = Mirror {
            origin: Vec3::ZERO,
            normal: Vec3::X,
            duplicate: true,
            weld: true,
            tolerance: 0.01,
        };
        let whole = mirror(&half, None, &options).unwrap();
        assert_eq!(whole.mesh.face_count(), half.face_count() * 2);
        assert!(crate::boolean::is_closed(&whole.mesh), "welded at the plane");
        assert!((crate::boolean::volume(&whole.mesh) - 1e6).abs() < 1.0, "facing out");
        assert!(crate::uv::flipped_faces(&whole.mesh) == 0, "no mirrored pictures");
        // Turned over instead: the same number of faces, on the other side.
        let flipped = mirror(
            &half,
            None,
            &Mirror {
                duplicate: false,
                ..options
            },
        )
        .unwrap();
        assert_eq!(flipped.mesh.face_count(), half.face_count());
        assert!(flipped.mesh.points().iter().all(|p| p.x <= 1e-3));
    }

    #[test]
    fn an_array_adds_copies_at_the_offsets() {
        let cube = primitives::cube(Vec3::splat(10.0), [1, 1, 1]);
        let options = Array {
            copies: 3,
            offset: Vec3::new(20.0, 0.0, 0.0),
            rotation: Vec3::ZERO,
            scale: 1.0,
            weld: false,
        };
        let out = array(&cube, None, &options).unwrap();
        assert_eq!(out.mesh.face_count(), 4 * 6);
        assert_eq!(out.face_origin.iter().filter(|&&o| o == super::super::NEW).count(), 18);
        let max_x = out.mesh.points().iter().map(|p| p.x).fold(f32::MIN, f32::max);
        assert!((max_x - 65.0).abs() < 1e-3, "{max_x}");
        // Touching copies welded: shared points once.
        let touching = Array {
            offset: Vec3::new(10.0, 0.0, 0.0),
            weld: true,
            ..options
        };
        let welded = array(&cube, None, &touching).unwrap();
        assert_eq!(welded.mesh.point_count(), 8 + 3 * 4);
    }

    #[test]
    fn a_quad_extruded_along_a_quarter_circle() {
        // The top of a box extruded along a quarter circle, 10 steps.
        let cube = primitives::cube(Vec3::splat(10.0), [1, 1, 1]);
        let top = (0..cube.face_count()).find(|&f| cube.face_normal(f).y > 0.9).unwrap();
        let topology = Topology::build(&cube);
        let arc: Vec<Vec3> = (0..=32)
            .map(|k| {
                let a = std::f32::consts::FRAC_PI_2 * k as f32 / 32.0;
                Vec3::new(100.0 * (1.0 - a.cos()), 100.0 * a.sin(), 0.0)
            })
            .collect();
        let path = Polyline {
            segments: vec![Segment {
                count: 33,
                closed: false,
            }],
            points: arc,
        };
        let faces: BitSet = [top].into_iter().collect();
        let out = extrude_along(&cube, &topology, &faces, &path, &AlongSpline::default()).unwrap();
        assert_eq!(out.face_count(), 6 + 40, "40 side quads");
        let end = out.face_normal(top);
        assert!(end.x > 0.99, "the end turned 90°: {end}");
        assert!(crate::boolean::is_closed(&out), "still closed");
        let flat = extrude_along(&cube, &topology, &faces, &Polyline::default(), &AlongSpline::default()).unwrap();
        assert_eq!(flat, cube, "no path: nothing");
    }

    #[test]
    fn the_pen_closes_a_quad_on_four_points_and_on_an_edge() {
        // Two separate quads side by side: a quad bridging their near edges.
        let a = primitives::plane(10.0, 10.0, 1, 1);
        let mut points = a.points().to_vec();
        points.extend(a.points().iter().map(|p| *p + Vec3::X * 20.0));
        let faces = vec![a.face(0).to_vec(), a.face(0).iter().map(|&c| c + 4).collect()];
        let mesh = PolyMesh::from_faces(points, faces).unwrap();
        let topology = Topology::build(&mesh);
        let near = |x: f32| {
            (0..mesh.point_count())
                .filter(|&p| (mesh.points()[p].x - x).abs() < 1e-3)
                .map(u32_index)
                .collect::<Vec<u32>>()
        };
        let (left, right) = (near(5.0), near(15.0));
        let corners = [
            PenCorner::Point(left[0]),
            PenCorner::Point(right[0]),
            PenCorner::Point(right[1]),
            PenCorner::Point(left[1]),
        ];
        let out = pen_polygon(&mesh, &topology, &corners).unwrap();
        assert_eq!(out.mesh.face_count(), 3);
        assert!(
            Topology::build(&out.mesh).is_consistently_oriented(),
            "wound with its neighbours"
        );
        // A point on an edge: both faces on it gain the point.
        let edge = (0..topology.edge_count())
            .find(|&e| topology.edge_faces(e).len() == 1)
            .unwrap();
        let on = pen_polygon(
            &mesh,
            &topology,
            &[
                PenCorner::Edge { edge, t: 0.5 },
                PenCorner::New(Vec3::new(0.0, 30.0, 0.0)),
                PenCorner::New(Vec3::new(5.0, 30.0, 0.0)),
            ],
        )
        .unwrap();
        assert_eq!(on.mesh.point_count(), 8 + 3);
        assert!(
            on.mesh.face(0).len() == 5 || on.mesh.face(1).len() == 5,
            "the edge's face gained it"
        );
    }

    #[test]
    fn carried_points_keep_their_places_on_random_shapes() {
        // Plane Cut, Mirror (duplicated) and Array keep every original
        // point where it was: selections carried by origin name the same
        // geometry, and the results stay manifold.
        let mut state = 99u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % 1000) as f32 / 1000.0
        };
        let shapes = [
            primitives::cube(Vec3::splat(100.0), [2, 3, 1]),
            primitives::sphere(60.0, 12),
            primitives::torus(80.0, 20.0, 12, 8),
        ];
        for shape in &shapes {
            let topology = Topology::build(shape);
            for _ in 0..10 {
                let normal = Vec3::new(next() - 0.5, next() - 0.5, next() - 0.5);
                let origin = Vec3::new(next() - 0.5, next() - 0.5, next() - 0.5) * 80.0;
                let cut = super::super::PlaneCut {
                    origin,
                    normal,
                    cuts: 1 + (next() * 3.0) as u32,
                    spacing: 7.0,
                    split: false,
                };
                let results = [
                    super::super::plane_cut(shape, &topology, &cut, None).unwrap(),
                    mirror(
                        shape,
                        None,
                        &Mirror {
                            origin,
                            normal,
                            duplicate: true,
                            weld: false,
                            tolerance: 0.01,
                        },
                    )
                    .unwrap(),
                    array(
                        shape,
                        None,
                        &Array {
                            copies: 2,
                            offset: origin,
                            rotation: normal,
                            scale: 0.9,
                            weld: false,
                        },
                    )
                    .unwrap(),
                ];
                for result in &results {
                    for (p, &o) in result.point_origin.iter().enumerate() {
                        if o != super::super::NEW {
                            assert_eq!(result.mesh.points()[p], shape.points()[us(o)]);
                        }
                    }
                    assert!(Topology::build(&result.mesh).is_manifold());
                }
            }
        }
    }
}
