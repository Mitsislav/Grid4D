//! Catmull-Clark subdivision on n-gons (see
//! `docs/reference/generator-subdivision-surface.md`): each level turns every
//! face with *k* corners into *k* quads around a new face point, moving the old
//! points towards the smooth limit surface. The bilinear variant splits the same
//! way without moving anything (Linear Pre-Subdivision).

use glam::{Vec2, Vec3};

use crate::{PolyMesh, Topology, u32_index, us};

/// How points on an open border are smoothed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Boundary {
    /// Border points follow the border curve, corners included.
    Edge,
    /// As `Edge`, but a border point used by a single face (a corner) stays put.
    #[default]
    EdgeAndCorner,
}

/// Subdivision options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SubdivisionOptions {
    /// Smoothing levels (0: the cage itself).
    pub levels: u32,
    /// Bilinear splits before smoothing: keeps the result closer to the cage.
    pub linear_pre: u32,
    pub boundary: Boundary,
}

/// The highest level the UI offers (each level multiplies faces by about 4).
pub const MAX_LEVEL: u32 = 6;

/// `mesh` subdivided `linear_pre` times without smoothing, then `levels` times
/// with Catmull-Clark (both clamped to [`MAX_LEVEL`]).
#[must_use]
pub fn subdivide(mesh: &PolyMesh, options: &SubdivisionOptions) -> PolyMesh {
    let mut current = mesh.clone();
    for _ in 0..options.linear_pre.min(MAX_LEVEL) {
        current = level(&current, false, options.boundary);
    }
    for _ in 0..options.levels.min(MAX_LEVEL) {
        current = level(&current, true, options.boundary);
    }
    current
}

/// One level: face points, edge points and (with `smooth`) moved old points; every
/// face with *k* corners becomes *k* quads (corner, edge point, face point,
/// previous edge point). New points follow: old points, edge points, face points.
fn level(mesh: &PolyMesh, smooth: bool, boundary: Boundary) -> PolyMesh {
    let topology = Topology::build(mesh);
    let points = mesh.points();
    let face_points: Vec<Vec3> = (0..mesh.face_count()).map(|f| mesh.face_center(f)).collect();

    let edge_points: Vec<Vec3> = topology
        .edges()
        .iter()
        .enumerate()
        .map(|(e, &[a, b])| {
            let middle = (points[us(a)] + points[us(b)]) * 0.5;
            match topology.edge_faces(e) {
                &[f, g] if smooth => (middle * 2.0 + face_points[us(f)] + face_points[us(g)]) * 0.25,
                _ => middle,
            }
        })
        .collect();

    let moved: Vec<Vec3> = if smooth {
        (0..mesh.point_count())
            .map(|p| smoothed_point(mesh, &topology, &face_points, p, boundary))
            .collect()
    } else {
        points.to_vec()
    };

    let n = mesh.point_count();
    let edge_index = |e: u32| u32_index(n + us(e));
    let face_index = |f: usize| u32_index(n + topology.edge_count() + f);
    let mut new_points = moved;
    new_points.extend_from_slice(&edge_points);
    new_points.extend_from_slice(&face_points);

    let mut faces: Vec<[u32; 4]> = Vec::with_capacity(mesh.corner_count());
    let mut uvs: Vec<Vec2> = Vec::new();
    let source_uvs = mesh.uvs();
    for f in 0..mesh.face_count() {
        let range = mesh.corner_range(f);
        let k = range.len();
        let face_uv =
            source_uvs.map(|uv| uv[range.clone()].iter().sum::<Vec2>() / f32::from(u16::try_from(k).unwrap_or(1)));
        for i in 0..k {
            let c = range.start + i;
            let prev = range.start + (i + k - 1) % k;
            let next = range.start + (i + 1) % k;
            let (Some(e_out), Some(e_in)) = (topology.corner_edge(c), topology.corner_edge(prev)) else {
                continue;
            };
            faces.push([
                mesh.corner_points()[c],
                edge_index(e_out),
                face_index(f),
                edge_index(e_in),
            ]);
            if let (Some(uv), Some(centre)) = (source_uvs, face_uv) {
                uvs.extend([uv[c], (uv[c] + uv[next]) * 0.5, centre, (uv[c] + uv[prev]) * 0.5]);
            }
        }
    }
    let Ok(mut result) = PolyMesh::from_faces(new_points, faces) else {
        return mesh.clone(); // cannot happen: every quad has four distinct points
    };
    if source_uvs.is_some() {
        let _ = result.set_uvs(Some(uvs));
    }
    result
}

/// Where old point `p` moves: the Catmull-Clark rule inside the surface, the
/// border-curve rule on borders and non-manifold edges, unchanged at corners
/// with [`Boundary::EdgeAndCorner`] and wherever the rules do not apply.
fn smoothed_point(mesh: &PolyMesh, topology: &Topology, face_points: &[Vec3], p: usize, boundary: Boundary) -> Vec3 {
    let points = mesh.points();
    let position = points[p];
    let edges = topology.point_edges(p);
    let faces = topology.point_faces(p);
    if edges.is_empty() {
        return position; // unused point
    }
    let other = |e: u32| {
        let [a, b] = topology.edges()[us(e)];
        points[us(if us(a) == p { b } else { a })]
    };
    // Border edges (one face) and non-manifold edges (more than two).
    let border: Vec<Vec3> = edges
        .iter()
        .filter(|&&e| topology.edge_faces(us(e)).len() != 2)
        .map(|&e| other(e))
        .collect();
    if border.is_empty() {
        let count = edges.len();
        let n = f32::from(u16::try_from(count).unwrap_or(u16::MAX));
        if count < 3 || faces.is_empty() {
            return position;
        }
        let q = faces.iter().map(|&f| face_points[us(f)]).sum::<Vec3>()
            / f32::from(u16::try_from(faces.len()).unwrap_or(1));
        let r = edges.iter().map(|&e| (position + other(e)) * 0.5).sum::<Vec3>() / n;
        return (q + r * 2.0 + position * (n - 3.0)) / n;
    }
    if boundary == Boundary::EdgeAndCorner && faces.len() == 1 {
        return position; // a corner
    }
    match border[..] {
        [previous, next] => (previous + position * 6.0 + next) / 8.0,
        _ => position,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;

    fn options(levels: u32) -> SubdivisionOptions {
        SubdivisionOptions {
            levels,
            linear_pre: 0,
            boundary: Boundary::EdgeAndCorner,
        }
    }

    #[test]
    fn level_zero_is_the_cage() {
        let cube = shapes::cube(2.0);
        assert_eq!(subdivide(&cube, &options(0)), cube);
    }

    #[test]
    fn a_cube_rounds_inside_its_cage() {
        let cube = shapes::cube(2.0);
        let once = subdivide(&cube, &options(1));
        assert_eq!((once.point_count(), once.face_count()), (26, 24));
        let topology = Topology::build(&once);
        assert!(topology.is_closed() && topology.is_manifold());
        for p in once.points() {
            assert!(p.abs().max_element() <= 1.0 + 1e-5, "inside the cage: {p}");
        }
        // The corners move in: a known Catmull-Clark value for a cube's corner.
        let corner = once.points()[0];
        assert!(corner.abs_diff_eq(Vec3::splat(-5.0 / 9.0), 1e-5), "{corner}");
    }

    #[test]
    fn faces_multiply_by_four_after_the_first_level() {
        for (name, mesh) in shapes::closed()
            .into_iter()
            .chain([("l_shape", shapes::l_shape()), ("star", shapes::star())])
        {
            let corners = mesh.corner_count();
            for levels in 1..=3u32 {
                let out = subdivide(&mesh, &options(levels));
                assert_eq!(
                    out.face_count(),
                    corners * 4usize.pow(levels - 1),
                    "{name} at level {levels}"
                );
                assert!(out.points().iter().all(|p| p.is_finite()), "{name}");
                assert!(out.faces().all(|f| f.len() == 4));
            }
        }
    }

    #[test]
    fn an_ngon_becomes_as_many_quads_as_it_has_corners() {
        let cylinder = shapes::cylinder(7);
        let once = subdivide(&cylinder, &options(1));
        assert_eq!(once.face_count(), 7 * 4 + 2 * 7);
    }

    #[test]
    fn unwelded_faces_smooth_into_separate_patches() {
        let cube = shapes::cube(2.0);
        let mut points = Vec::new();
        let mut faces = Vec::new();
        for face in cube.faces() {
            let start = u32_index(points.len());
            points.extend(face.iter().map(|&p| cube.points()[us(p)]));
            faces.push([start, start + 1, start + 2, start + 3]);
        }
        let separate = PolyMesh::from_faces(points, faces).unwrap();
        let out = subdivide(&separate, &options(1));
        let topology = Topology::build(&out);
        assert_eq!(topology.boundary_edge_count(), 6 * 8, "six open patches");
    }

    #[test]
    fn plane_corners_stay_with_edge_and_corner() {
        let plane = shapes::grid(2, 2);
        let corners = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(2.0, 0.0, 2.0),
        ];
        let kept = subdivide(&plane, &options(1));
        assert!(corners.iter().all(|c| kept.points().contains(c)), "corners stay");
        let edge = subdivide(
            &plane,
            &SubdivisionOptions {
                boundary: Boundary::Edge,
                ..options(1)
            },
        );
        assert!(corners.iter().all(|c| !edge.points().contains(c)), "corners move");
        assert!(edge.points().iter().all(|p| p.y.abs() < 1e-6), "a plane stays flat");
    }

    #[test]
    fn non_manifold_edges_give_finite_points() {
        // Three quads around one edge (0-1).
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let mesh = PolyMesh::from_faces(points, [[0u32, 1, 2, 3], [1, 0, 5, 4], [0, 1, 6, 7]]).unwrap();
        let out = subdivide(&mesh, &options(2));
        assert!(out.points().iter().all(|p| p.is_finite()));
        assert_eq!(out.face_count(), 12 * 4);
    }

    #[test]
    fn linear_pre_subdivision_splits_without_smoothing() {
        let cube = shapes::cube(2.0);
        let split = subdivide(
            &cube,
            &SubdivisionOptions {
                levels: 0,
                linear_pre: 1,
                boundary: Boundary::EdgeAndCorner,
            },
        );
        assert_eq!((split.point_count(), split.face_count()), (26, 24));
        assert!(
            split.points()[..8].iter().zip(cube.points()).all(|(a, b)| a == b),
            "old points stay"
        );
        for p in split.points() {
            assert!((p.abs().max_element() - 1.0).abs() < 1e-6, "on the cube's surface: {p}");
        }
    }

    #[test]
    fn uvs_follow_the_corners() {
        let mut quad = shapes::grid(1, 1);
        quad.set_uvs(Some(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 0.0),
        ]))
        .unwrap();
        let out = subdivide(&quad, &options(1));
        let uvs = out.uvs().unwrap();
        assert_eq!(uvs.len(), 16);
        assert!(uvs.contains(&Vec2::splat(0.5)), "the face point's UV is the average");
    }
}
